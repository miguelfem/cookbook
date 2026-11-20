//! Read-only App windows have no Tauri capability entry and no runtime client.
//! A one-use private protocol serves only the selected, already authorized data.
use super::{Desktop, Reply, require};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::http::Response;

const APP_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'";
struct Documents {
    at: Instant,
    host: Option<Vec<u8>>,
    child: Option<Vec<u8>>,
}
type Pending = BTreeMap<String, Documents>;
#[derive(Clone, Default)]
pub struct Previews(Arc<Mutex<Pending>>);
impl Previews {
    pub fn response(&self, path: &str) -> Response<Vec<u8>> {
        let route = preview_route(path);
        let payload = route.and_then(|(id, child)| {
            self.0.lock().ok().and_then(|mut pending| {
                pending.retain(|_, documents| documents.at.elapsed() < Duration::from_secs(60));
                let documents = pending.get_mut(id)?;
                let bytes = if child {
                    documents.child.take()
                } else {
                    documents.host.take()
                };
                if documents.host.is_none() && documents.child.is_none() {
                    pending.remove(id);
                }
                bytes
            })
        });
        let csp = route.map_or_else(
            || APP_CSP.to_string(),
            |(id, child)| {
                if child {
                    APP_CSP.to_string()
                } else {
                    host_csp(id)
                }
            },
        );
        let code = if payload.is_some() { 200 } else { 404 };
        Response::builder()
            .status(code)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Content-Security-Policy", csp)
            .header("Cache-Control", "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .header("Referrer-Policy", "no-referrer")
            .body(
                payload.unwrap_or_else(|| {
                    b"Preview expired. Reopen it from the recipe window.".to_vec()
                }),
            )
            .expect("static preview response")
    }
}

fn preview_route(path: &str) -> Option<(&str, bool)> {
    let path = path.strip_prefix('/')?;
    let (id, child) = match path.split_once('/') {
        None => (path, false),
        Some((id, "document")) => (id, true),
        _ => return None,
    };
    (id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some((id, child))
}

fn host_address(id: &str) -> String {
    format!("cookbook-app://localhost/{id}")
}

fn child_address(id: &str) -> String {
    format!("{}/document", host_address(id))
}

fn host_csp(id: &str) -> String {
    format!(
        "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none'; frame-src {}; object-src 'none'; base-uri 'none'; form-action 'none'",
        child_address(id)
    )
}

fn navigation_allowed(id: &str, url: &str) -> bool {
    // Wry invokes this callback for child navigations as well as the host.
    // Permit only this preview's two one-use documents, never a blob wildcard.
    url == host_address(id) || url == child_address(id)
}

fn documents(id: &str, value: &Value) -> Result<Documents, String> {
    let html = value["html"].as_str().ok_or("Invalid App document")?;
    let encoded = serde_json::to_vec(value).map_err(|_| "Invalid App result")?;
    if encoded.len() > 60000 {
        return Err("App preview exceeds its limit".into());
    }
    let data = serde_json::to_string(&json!({
        "document_url": child_address(id),
        "tool_result": value["tool_result"],
    }))
    .map_err(|_| "Invalid App result")?;
    // Escaping < prevents untrusted tool data closing the inert JSON node.
    let data = data
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let host = format!(
        "<!doctype html><meta charset=utf-8><title>CookBook read-only App</title><h2>Read-only MCP App</h2><p>No runtime permissions, external network or host tools.</p><p id=status>Loading…</p><main></main><script type=application/json id=payload>{data}</script><script>{}</script>",
        include_str!("../../ui/preview-host.js")
    );
    // The header enforces this policy before any untrusted bytes are parsed.
    // The prepended meta also retains the restriction inside the document.
    // This narrow bootstrap proves document execution for static Apps; it does
    // not claim the App completed the Apps protocol handshake.
    let child = format!(
        "<!doctype html><meta charset=utf-8><meta http-equiv=Content-Security-Policy content=\"{APP_CSP}\"><script>window.addEventListener('DOMContentLoaded',()=>parent.postMessage({{type:'cookbook:document-loaded'}},'*'));</script>{html}"
    );
    Ok(Documents {
        at: Instant::now(),
        host: Some(host.into_bytes()),
        child: Some(child.into_bytes()),
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewArgs {
    run_id: String,
    resource_step_id: String,
    tool_step_id: String,
}
#[tauri::command]
pub async fn app_preview_open(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    desktop: tauri::State<'_, Desktop>,
    state: tauri::State<'_, Previews>,
    args: PreviewArgs,
    consent: bool,
) -> Reply {
    require(&window, "main")?;
    if !consent {
        return Err("Review the selected App and data before opening".into());
    }
    let isolated = desktop.isolated;
    let desktop = desktop.inner().clone();
    let value = tauri::async_runtime::spawn_blocking(move || desktop.client()?.call("inspect", json!({"view":"mcp_app","run_id":args.run_id,"resource_step_id":args.resource_step_id,"tool_step_id":args.tool_step_id}), 65536).map_err(super::public_error))
        .await.map_err(|_| "App lookup failed")??;
    if value["read_only"] != true || value["host_tools"] != false {
        return Err("Unsupported App profile".into());
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let documents = documents(&id, &value)?;
    {
        let mut pending = state.0.lock().map_err(|_| "Preview state unavailable")?;
        pending.retain(|_, documents| documents.at.elapsed() < Duration::from_secs(60));
        if pending.len() >= 4 {
            return Err("Too many previews are opening".into());
        }
        pending.insert(id.clone(), documents);
    }
    let address = host_address(&id);
    let allowed = id.clone();
    let preview = tauri::WebviewWindowBuilder::new(
        &app,
        format!("app-preview-{id}"),
        tauri::WebviewUrl::External(address.parse().map_err(|_| "Preview address invalid")?),
    )
    .incognito(isolated)
    .title("CookBook · Read-only App")
    .inner_size(900.0, 700.0)
    .on_navigation(move |url| navigation_allowed(&allowed, url.as_str()))
    .build();
    if preview.is_err() {
        if let Ok(mut pending) = state.0.lock() {
            pending.remove(&id);
        }
        return Err("Could not open isolated App window".into());
    }
    let cleanup = state.inner().clone();
    let cleanup_id = id.clone();
    preview.unwrap().on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed)
            && let Ok(mut pending) = cleanup.0.lock()
        {
            pending.remove(&cleanup_id);
        }
    });
    Ok(json!({"opened":true,"read_only":true,"host_tools":false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef";
    const OTHER_ID: &str = "fedcba9876543210fedcba9876543210";

    fn fixture(html: &str) -> Value {
        json!({"html":html,"tool_result":{"text":"</script><script>untrusted()</script>"},"read_only":true,"host_tools":false})
    }

    #[test]
    fn navigation_permits_only_this_previews_host_and_child_document() {
        assert!(navigation_allowed(ID, &host_address(ID)));
        assert!(navigation_allowed(ID, &child_address(ID)));
        for url in [
            "https://example.org/".to_string(),
            "http://127.0.0.1/".to_string(),
            "file:///private/example".to_string(),
            "data:text/html,untrusted".to_string(),
            "javascript:untrusted()".to_string(),
            "blob:null/example".to_string(),
            format!("blob:{}/example", host_address(ID)),
            "about:blank".to_string(),
            child_address(OTHER_ID),
            host_address(OTHER_ID),
            format!("{}/other", host_address(ID)),
            format!("{}/", child_address(ID)),
            format!("{}?other", child_address(ID)),
            format!("{}#other", child_address(ID)),
        ] {
            assert!(!navigation_allowed(ID, &url), "must deny {url}");
        }
    }

    #[test]
    fn preview_documents_have_distinct_policies_and_each_load_only_once() {
        let previews = Previews::default();
        previews.0.lock().unwrap().insert(
            ID.to_string(),
            documents(ID, &fixture("<h1>Synthetic App</h1>")).unwrap(),
        );
        assert_eq!(previews.response(&format!("/{ID}/other")).status(), 404);
        let host = previews.response(&format!("/{ID}"));
        assert_eq!(host.status(), 200);
        assert_eq!(host.headers()["Cache-Control"], "no-store");
        assert_eq!(host.headers()["Content-Security-Policy"], host_csp(ID));
        let host_html = std::str::from_utf8(host.body()).unwrap();
        assert!(host_html.contains(&child_address(ID)));
        assert!(!host_html.contains("<script>untrusted()</script>"));
        assert!(!host_html.contains("<h1>Synthetic App</h1>"));
        assert_eq!(previews.response(&format!("/{ID}")).status(), 404);
        let child = previews.response(&format!("/{ID}/document"));
        assert_eq!(child.status(), 200);
        assert_eq!(child.headers()["Cache-Control"], "no-store");
        assert_eq!(child.headers()["Content-Security-Policy"], APP_CSP);
        let child_html = std::str::from_utf8(child.body()).unwrap();
        assert!(child_html.contains("<h1>Synthetic App</h1>"));
        assert!(child_html.contains("cookbook:document-loaded"));
        assert!(!child_html.contains("untrusted()"));
        assert_eq!(previews.response(&format!("/{ID}/document")).status(), 404);
        assert!(previews.0.lock().unwrap().is_empty());
    }

    #[test]
    fn expired_and_foreign_preview_paths_cannot_consume_current_documents() {
        let previews = Previews::default();
        let mut expired = documents(ID, &fixture("Expired synthetic App")).unwrap();
        expired.at = Instant::now() - Duration::from_secs(61);
        {
            let mut pending = previews.0.lock().unwrap();
            pending.insert(ID.to_string(), expired);
            pending.insert(
                OTHER_ID.to_string(),
                documents(OTHER_ID, &fixture("Current synthetic App")).unwrap(),
            );
        }
        for path in [
            format!("/{ID}"),
            format!("/{ID}/document"),
            format!("//{OTHER_ID}/document"),
            format!("/{OTHER_ID}/document/extra"),
            format!("/{OTHER_ID}/../document"),
        ] {
            let response = previews.response(&path);
            assert_eq!(response.status(), 404);
            assert!(
                !std::str::from_utf8(response.body())
                    .unwrap()
                    .contains("synthetic App")
            );
        }
        assert_eq!(previews.response(&format!("/{OTHER_ID}")).status(), 200);
        assert_eq!(
            previews.response(&format!("/{OTHER_ID}/document")).status(),
            200
        );
    }
}
