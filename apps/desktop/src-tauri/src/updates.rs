//! Only the trusted setup window can use the updater. Publisher key/endpoint
//! are compiled into a release, never supplied by a recipe or remote content.
use super::{Desktop, Reply, public_error, require};
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
    installing: AtomicBool,
}
struct Installing<'a>(&'a AtomicBool);
impl Drop for Installing<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
#[tauri::command]
pub async fn update_check(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
) -> Reply {
    require(&window, "setup")?;
    check(&app, &state).await
}
async fn check(app: &tauri::AppHandle, state: &UpdateState) -> Reply {
    if state.installing.load(Ordering::Acquire) {
        return Err("An update is already being installed".into());
    }
    let (Some(key), Some(endpoint)) = (
        option_env!("COOKBOOK_UPDATE_PUBLIC_KEY").filter(|v| !v.is_empty()),
        option_env!("COOKBOOK_UPDATE_ENDPOINT").filter(|v| !v.is_empty()),
    ) else {
        return Ok(
            json!({"configured":false,"reason":"This development build has no publisher update key or endpoint"}),
        );
    };
    let endpoint = apr_connectors::oauth::public_url(endpoint, false)
        .map_err(|_| "Release update endpoint is invalid")?;
    let updater = app
        .updater_builder()
        .pubkey(key)
        .endpoints(vec![endpoint])
        .map_err(|_| "Update endpoint unavailable")?
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Updater configuration invalid")?;
    let found = updater
        .check()
        .await
        .map_err(|_| "Could not check the publisher's update feed")?;
    let metadata = found
        .as_ref()
        .map(|u| json!({"version":u.version,"current_version":u.current_version,"notes":u.body}));
    *state
        .pending
        .lock()
        .map_err(|_| "Update state unavailable")? = found;
    Ok(json!({"configured":true,"available":metadata,"signature_required":true}))
}
#[tauri::command]
pub async fn update_install(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
    desktop: tauri::State<'_, Desktop>,
    version: String,
    consent: bool,
) -> Reply {
    require(&window, "setup")?;
    if !consent {
        return Err("Approve this update before installation".into());
    }
    if state
        .installing
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("An update is already being installed".into());
    }
    let _installing = Installing(&state.installing);
    let update = {
        let mut pending = state
            .pending
            .lock()
            .map_err(|_| "Update state unavailable")?;
        if pending.as_ref().is_none_or(|u| u.version != version) {
            return Err("Check and review the update again".into());
        }
        pending.take().unwrap()
    };
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        return Err("This installation uses the system package manager. Install the signed release package instead of an in-app replacement".into());
    }
    let desktop = desktop.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut runtime = desktop.runtime()?;
        runtime
            .prepare_desktop_update(&version)
            .map_err(public_error)?;
        let parent = desktop
            .root
            .parent()
            .ok_or("Backup location unavailable")?
            .join("backups");
        std::fs::create_dir_all(&parent).map_err(|_| "Backup location unavailable")?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Local clock invalid")?
            .as_nanos();
        runtime
            .backup_to(&parent.join(format!("before-update-{stamp}")))
            .map_err(public_error)?;
        if let Ok(client) = desktop.client() {
            client.request_owner_stop().map_err(public_error)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "Update preparation failed")??;
    // Tauri verifies the artifact signature before installation. Failures leave
    // the persisted maintenance gate closed for explicit owner recovery.
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|_| "Update failed. Runtime remains paused; review maintenance controls")?;
    app.restart();
}

/// Opt-in metadata checks only. No updater URL, authority or install action comes
/// from a recipe; unsigned previews make no network request.
pub fn start_checks(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut next = std::time::Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(30));
            if std::time::Instant::now() < next {
                continue;
            }
            let desktop = app.state::<Desktop>();
            let enabled = desktop
                .runtime()
                .ok()
                .and_then(|r| r.update_preferences().ok())
                .is_some_and(|v| v["automatic_checks"] == true);
            if !enabled {
                continue;
            }
            next = std::time::Instant::now() + Duration::from_secs(86400);
            let state = app.state::<UpdateState>();
            if let Ok(result) = tauri::async_runtime::block_on(check(&app, &state))
                && result["available"].is_object()
            {
                let _ = app.emit("cookbook:update-available", result);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn preview_configuration_initializes_the_real_updater_without_a_feed() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        // Tauri deserializes plugin configuration before running app setup,
        // even when the preview never checks a publisher feed.
        let updater: tauri_plugin_updater::Config =
            serde_json::from_value(config["plugins"]["updater"].clone()).unwrap();
        assert!(updater.pubkey.is_empty());
        assert!(updater.endpoints.is_empty());
        assert!(!updater.dangerous_insecure_transport_protocol);
        assert!(!updater.dangerous_accept_invalid_certs);
        assert!(!updater.dangerous_accept_invalid_hostnames);
    }
}
