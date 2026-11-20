//! Explicit local runtime selection. An invalid override never falls back to
//! the user's normal store, and selecting a store grants no new authority.
use apr_runtime::service::ServiceClient;
use serde::Deserialize;
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub const USAGE: &str = "CookBook APR [--state-dir /absolute/private/runtime]\n\
    Omit --state-dir to use the normal application store. An explicit store\n\
    must be a private owned directory, or a new directory under a private\n\
    owned parent. Reopen with the same option to recover its saved work.";

pub fn parse_args(args: &[OsString]) -> Result<Option<PathBuf>, &'static str> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--state-dir" => {
            let path = PathBuf::from(path);
            if !path.is_absolute()
                || path.file_name().is_none()
                || path.components().any(|part| part == Component::ParentDir)
            {
                return Err("--state-dir requires an absolute directory without '..'");
            }
            Ok(Some(path))
        }
        _ => Err("Expected only --state-dir followed by one absolute directory"),
    }
}

pub fn runtime_root(
    explicit: Option<&Path>,
    default: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    let Some(path) = explicit else {
        return default();
    };
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            require_private_directory(&metadata)?;
            path.canonicalize()
                .map_err(|_| "Explicit state directory is unavailable".into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .ok_or("The explicit state directory needs an existing private parent")?;
            let metadata = std::fs::metadata(&parent)
                .map_err(|_| "Explicit state directory parent is unavailable")?;
            require_private_directory(&metadata)?;
            Ok(parent.join(path.file_name().ok_or("Invalid state directory")?))
        }
        Err(_) => Err("Explicit state directory is unavailable".into()),
    }
}

fn require_private_directory(metadata: &std::fs::Metadata) -> Result<(), String> {
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("Explicit state directories must be private, owned directories without symlinks; no permissions were changed".into());
    }
    Ok(())
}

/// A saved path can select authority only from this exact runtime store.
pub fn selected_attachment(root: &Path) -> Option<PathBuf> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Selection {
        attachment_file: PathBuf,
    }
    let bytes = std::fs::read(root.join("desktop-selection.json")).ok()?;
    let selection: Selection = serde_json::from_slice(&bytes).ok()?;
    let path = selection.attachment_file;
    let actual = path.canonicalize().ok()?;
    if !actual.starts_with(root.join("attachments")) {
        return None;
    }
    let client = ServiceClient::from_file(&path).ok()?;
    if client.state_dir().canonicalize().ok()? != root {
        return None;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Desktop, OwnerAction, owner_dispatch};
    use apr_runtime::Runtime;
    use serde_json::json;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
    use std::sync::{Arc, Mutex};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("cookbook-launch-{}", uuid::Uuid::new_v4()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn explicit(path: &Path) -> PathBuf {
        let parsed = parse_args(&["--state-dir".into(), path.as_os_str().to_owned()]).unwrap();
        runtime_root(parsed.as_deref(), || {
            panic!("must not inspect the default store")
        })
        .unwrap()
    }
    fn desktop(root: PathBuf) -> Desktop {
        Desktop {
            root,
            executable: PathBuf::from("/unused/apr"),
            selected: Arc::new(Mutex::new(None)),
            install_plan: Arc::new(Mutex::new(None)),
            agent_plan: Arc::new(Mutex::new(None)),
            isolated: true,
        }
    }

    #[test]
    fn malformed_override_never_selects_the_default() {
        for args in [
            vec!["--state-dir"],
            vec!["--state-dir", ""],
            vec!["--state-dir", "relative"],
            vec!["--state-dir", "/"],
            vec!["--state-dir", "/private/../runtime"],
            vec!["--state-dir", "/private/runtime", "--state-dir", "/other"],
            vec!["--unknown"],
        ] {
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert!(parse_args(&args).is_err());
        }
        assert_eq!(parse_args(&[]).unwrap(), None);
        assert_eq!(
            runtime_root(None, || Ok(PathBuf::from("/normal/store"))).unwrap(),
            PathBuf::from("/normal/store")
        );
    }

    #[test]
    fn explicit_root_rejects_unsafe_locations_without_modifying_them() {
        let scratch = Scratch::new();
        let store = scratch.0.join("runtime");
        assert_eq!(explicit(&store), store);
        assert!(!store.exists());
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&store)
            .unwrap();
        assert_eq!(explicit(&store), store);
        let link = scratch.0.join("link");
        symlink(&store, &link).unwrap();
        let file = scratch.0.join("file");
        std::fs::write(&file, b"preserve").unwrap();
        let public = scratch.0.join("public");
        std::fs::create_dir(&public).unwrap();
        std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
        for invalid in [link, file.clone(), public.clone(), public.join("child")] {
            assert!(runtime_root(Some(&invalid), || panic!("no fallback")).is_err());
        }
        assert_eq!(std::fs::read(&file).unwrap(), b"preserve");
        assert_eq!(std::fs::metadata(public).unwrap().mode() & 0o777, 0o755);
        assert!(!scratch.0.join("public/child").exists());
    }

    #[test]
    fn stores_keep_selections_and_update_preferences_separate_across_reopen() {
        let scratch = Scratch::new();
        let a = desktop(explicit(&scratch.0.join("a")));
        let b = desktop(explicit(&scratch.0.join("b")));
        let project = scratch.0.join("project");
        std::fs::create_dir(&project).unwrap();
        for (state, name) in [(&a, "First"), (&b, "Second")] {
            let assignment = Runtime::open(&state.root)
                .unwrap()
                .owner_project_setup(&project, name)
                .unwrap();
            state
                .select(assignment["assignment_id"].as_str().unwrap())
                .unwrap();
        }
        owner_dispatch(&a, OwnerAction::SetUpdatePreferences { enabled: true }).unwrap();
        let a_selection = selected_attachment(&a.root).unwrap();
        let b_selection = selected_attachment(&b.root).unwrap();
        assert_ne!(a_selection, b_selection);
        let reopened = desktop(explicit(&a.root));
        *reopened.selected.lock().unwrap() = selected_attachment(&reopened.root);
        assert_eq!(
            reopened.client().unwrap().assignment_id(),
            a.client().unwrap().assignment_id()
        );
        assert_eq!(
            owner_dispatch(&reopened, OwnerAction::UpdatePreferences).unwrap()["automatic_checks"],
            true
        );
        assert_eq!(
            owner_dispatch(&b, OwnerAction::UpdatePreferences).unwrap()["automatic_checks"],
            false
        );
        std::fs::write(
            b.root.join("desktop-selection.json"),
            serde_json::to_vec(&json!({"attachment_file": a_selection})).unwrap(),
        )
        .unwrap();
        assert!(selected_attachment(&b.root).is_none());
        // A copied file retains its original store identity and cannot redirect
        // this desktop to a different store even when its path looks local.
        let copied = b.root.join("attachments/copied.json");
        std::fs::copy(a_selection, &copied).unwrap();
        std::fs::write(
            b.root.join("desktop-selection.json"),
            serde_json::to_vec(&json!({"attachment_file": copied})).unwrap(),
        )
        .unwrap();
        assert!(selected_attachment(&b.root).is_none());
        assert_eq!(
            selected_attachment(&a.root).unwrap(),
            *a.selected.lock().unwrap().as_ref().unwrap()
        );
    }
}
