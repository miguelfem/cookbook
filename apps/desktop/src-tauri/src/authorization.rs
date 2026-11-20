//! Native PKCE sessions. Codes and tokens never cross the WebView boundary.
use super::{Desktop, Reply, public_error, require};
use apr_connectors::{
    Auth, Connection,
    oauth::{OAuthConfig, PendingAuthorization},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
type PendingResult = Arc<Mutex<Option<Reply>>>;
type PreparedFlow = (Arc<AtomicBool>, PendingResult);
struct Flow {
    cancel: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Reply>>>,
}
struct Cached {
    config: OAuthConfig,
    account_id: String,
    revision: u64,
    expires: Instant,
}
#[derive(Default)]
pub struct Authorizations {
    flows: Mutex<BTreeMap<String, Flow>>,
    cache: Mutex<Vec<Cached>>,
}
impl Drop for Authorizations {
    fn drop(&mut self) {
        if let Ok(flows) = self.flows.get_mut() {
            for flow in flows.values() {
                flow.cancel.store(true, Ordering::Release);
            }
        }
    }
}
fn browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let program = "/usr/bin/open";
    #[cfg(target_os = "linux")]
    let program = "/usr/bin/xdg-open";
    let mut child = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "Cannot open the system browser".to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
fn prepare(flows: &Authorizations, id: &str) -> Result<PreparedFlow, String> {
    let mut entries = flows
        .flows
        .lock()
        .map_err(|_| "Authorization state unavailable")?;
    entries.retain(|_, f| f.result.lock().map(|r| r.is_none()).unwrap_or(false));
    if entries.len() >= 4 || entries.contains_key(id) {
        return Err("Complete or cancel pending sign-ins first".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let result = Arc::new(Mutex::new(None));
    entries.insert(
        id.into(),
        Flow {
            cancel: cancel.clone(),
            result: result.clone(),
        },
    );
    Ok((cancel, result))
}
fn cached(
    flows: &Authorizations,
    config: &OAuthConfig,
) -> Result<Option<apr_runtime::connection_library::SavedAccount>, String> {
    let mut cache = flows
        .cache
        .lock()
        .map_err(|_| "Authorization cache unavailable")?;
    cache.retain(|c| c.expires > Instant::now());
    Ok(cache.iter().find(|c| &c.config == config).map(|c| {
        apr_runtime::connection_library::SavedAccount {
            account_id: c.account_id.clone(),
            revision: c.revision,
        }
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    request_id: String,
    digest: String,
    consent: bool,
    #[serde(default)]
    all_projects: bool,
}
#[tauri::command]
pub async fn oauth_begin(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    desktop: tauri::State<'_, Desktop>,
    flows: tauri::State<'_, Authorizations>,
    request: Approval,
) -> Reply {
    require(&window, "setup")?;
    let Approval {
        request_id,
        digest,
        consent,
        all_projects,
    } = request;
    if !consent {
        return Err("Review and approve this exact connection before sign-in".into());
    }
    let config = desktop
        .runtime()?
        .oauth_request_config(&request_id, &digest)
        .map_err(public_error)?;
    let account = cached(&flows, &config)?;
    let authorization = if account.is_none() {
        Some(
            tauri::async_runtime::spawn_blocking(move || PendingAuthorization::begin(config))
                .await
                .map_err(|_| "Authorization worker stopped")?
                .map_err(|error| format!("Cannot prepare browser sign-in: {error}"))?,
        )
    } else {
        None
    };
    let (cancel, result) = prepare(&flows, &request_id)?;
    if let Some((_, url)) = &authorization
        && let Err(error) = browser(url)
    {
        if let Ok(mut r) = result.lock() {
            *r = Some(Err(error.clone()));
        }
        return Err(error);
    }
    let id = request_id.clone();
    let desktop = desktop.inner().clone();
    std::thread::spawn(move || {
        let value = (match account {
            Some(account) => Ok((None, Some(account))),
            None => authorization
                .ok_or("Missing authorization session".to_string())
                .and_then(|(p, _)| {
                    p.finish(&cancel)
                        .map(|token| (Some(token), None))
                        .map_err(|error| format!("Browser authorization stopped: {error}"))
                }),
        })
        .and_then(|(token, saved_account)| {
            if cancel.load(Ordering::Acquire) {
                return Err("Authorization cancelled".into());
            }
            desktop
                .runtime()?
                .approve_connection_with_options(
                    &id,
                    &digest,
                    token,
                    apr_runtime::connection_library::ApprovalOptions {
                        all_projects,
                        saved_credential: None,
                        saved_account,
                        reviewed_account_reuse: None,
                    },
                )
                .map_err(public_error)
        });
        if value.is_ok() {
            let _ = app.emit_to("main", "cookbook:changed", json!({}));
        }
        if let Ok(mut r) = result.lock() {
            *r = Some(value);
        }
    });
    Ok(json!({"flow_id":request_id,"state":"authorizing","expires_in_seconds":300}))
}
/// Authenticate only to discover metadata. No connection or tool grant is issued.
/// Account identity may subsequently be reused only after exact operation review.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataDiscovery {
    connection: Connection,
    kind: String,
    consent: bool,
    saved_account: Option<apr_runtime::connection_library::SavedAccount>,
    #[serde(default)]
    automatic_protocol: bool,
}
#[tauri::command]
pub async fn oauth_discover_begin(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    desktop: tauri::State<'_, Desktop>,
    flows: tauri::State<'_, Authorizations>,
    request: MetadataDiscovery,
) -> Reply {
    require(&window, "setup")?;
    let MetadataDiscovery {
        connection,
        kind,
        consent,
        saved_account,
        automatic_protocol,
    } = request;
    if !consent || !matches!(kind.as_str(), "tools" | "resources" | "prompts") {
        return Err("Review this metadata discovery before signing in".into());
    }
    connection
        .validate()
        .map_err(|_| "Invalid discovery connection")?;
    let Auth::Oauth {
        credential_ref,
        config,
    } = &connection.auth
    else {
        return Err("Select OAuth authorization first".into());
    };
    if credential_ref != "pending" || connection.mcp.as_ref().is_none_or(|m| m.is_stdio()) {
        return Err("OAuth discovery requires an unbound HTTPS MCP endpoint".into());
    }
    let config = config.clone();
    let account = saved_account;
    let authorization_is_cached = account.is_some();
    let authorization = if account.is_none() {
        let pending_config = config.clone();
        Some(
            tauri::async_runtime::spawn_blocking(move || {
                PendingAuthorization::begin(pending_config)
            })
            .await
            .map_err(|_| "Authorization worker stopped")?
            .map_err(|error| format!("Cannot prepare browser sign-in: {error}"))?,
        )
    } else {
        None
    };
    let id = format!("discovery-{}", uuid::Uuid::new_v4().simple());
    let (cancel, result) = prepare(&flows, &id)?;
    if let Some((_, url)) = &authorization
        && let Err(e) = browser(url)
    {
        if let Ok(mut r) = result.lock() {
            *r = Some(Err(e.clone()));
        }
        return Err(e);
    }
    let desktop = desktop.inner().clone();
    std::thread::spawn(move || {
        let value = (|| -> Reply {
            let account = if let Some(saved) = account {
                json!({"account_id":saved.account_id,"revision":saved.revision})
            } else {
                let (pending, _) = authorization.ok_or("Missing authorization session")?;
                let token = pending
                    .finish(&cancel)
                    .map_err(|error| format!("Browser authorization stopped: {error}"))?;
                if cancel.load(Ordering::Acquire) {
                    return Err("Authorization cancelled".into());
                }
                let account = desktop
                    .runtime()?
                    .connect_account(&connection, token)
                    .map_err(public_error)?;
                let auth = app.state::<Authorizations>();
                // Account persistence has succeeded. Cache failure cannot turn this
                // into a stopped flow that incorrectly implies the login was lost.
                if let Ok(mut cache) = auth.cache.lock() {
                    cache.retain(|c| c.expires > Instant::now() && c.config != config);
                    if cache.len() >= 4 {
                        cache.remove(0);
                    }
                    if let (Some(account_id), Some(revision)) =
                        (account["account_id"].as_str(), account["revision"].as_u64())
                    {
                        cache.push(Cached {
                            config,
                            account_id: account_id.to_owned(),
                            revision,
                            expires: Instant::now() + Duration::from_secs(300),
                        });
                    }
                }
                account
            };
            let account_id = account["account_id"]
                .as_str()
                .ok_or("Account receipt unavailable")?;
            let revision = account["revision"]
                .as_u64()
                .ok_or("Account revision unavailable")?;
            let discovery = if cancel.load(Ordering::Acquire) {
                Err("Discovery cancelled".to_string())
            } else {
                desktop.runtime().and_then(|runtime| {
                    runtime
                        .discover_mcp_with_account_protocol(
                            &connection,
                            account_id,
                            revision,
                            &kind,
                            automatic_protocol,
                        )
                        .map_err(public_error)
                })
            };
            match discovery {
                Ok(data) => Ok(
                    json!({"discovery":data,"status":"operation_approval_required","account":account,"grant_created":false}),
                ),
                Err(error) => Ok(
                    json!({"discovery":null,"discovery_error":error,"status":"account_connected_discovery_failed","account":account,"grant_created":false}),
                ),
            }
        })();
        if let Ok(mut r) = result.lock() {
            *r = Some(value);
        }
    });
    Ok(
        json!({"flow_id":id,"state":if authorization_is_cached {"discovering"} else {"browser_opened"},"grant_created":false}),
    )
}
#[tauri::command]
pub fn oauth_status(
    window: tauri::WebviewWindow,
    flows: tauri::State<'_, Authorizations>,
    flow_id: String,
) -> Reply {
    require(&window, "setup")?;
    let entries = flows
        .flows
        .lock()
        .map_err(|_| "Authorization state unavailable")?;
    let f = entries.get(&flow_id).ok_or("Sign-in is no longer active")?;
    let result = f
        .result
        .lock()
        .map_err(|_| "Authorization state unavailable")?;
    match &*result {
        None => Ok(json!({"state":"waiting"})),
        Some(Ok(receipt)) => Ok(json!({"state":"connected","receipt":receipt})),
        Some(Err(message)) => Ok(json!({"state":"stopped","message":message})),
    }
}
#[tauri::command]
pub fn oauth_cancel(
    window: tauri::WebviewWindow,
    flows: tauri::State<'_, Authorizations>,
    flow_id: String,
) -> Result<Value, String> {
    require(&window, "setup")?;
    let entries = flows
        .flows
        .lock()
        .map_err(|_| "Authorization state unavailable")?;
    let f = entries.get(&flow_id).ok_or("Sign-in is no longer active")?;
    f.cancel.store(true, Ordering::Release);
    Ok(json!({"cancel_requested":true}))
}
