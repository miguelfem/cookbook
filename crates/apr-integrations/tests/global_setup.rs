use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

use apr_integrations::global::{GlobalInstallOptions, GlobalManager};
use apr_integrations::{Client, Error};

struct Fixture {
    _temp: tempfile::TempDir,
    state: PathBuf,
    home: PathBuf,
    executable: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("private state with spaces");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let home = temp.path().join("home with spaces");
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        let executable = temp.path().join("apr executable");
        fs::write(&executable, "synthetic executable").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _temp: temp,
            state,
            home,
            executable,
        }
    }
}

#[test]
fn installs_global_entries_without_attachment_and_preserves_codex_comments() {
    let f = Fixture::new();
    fs::create_dir(f.home.join(".codex")).unwrap();
    fs::write(
        f.home.join(".codex/config.toml"),
        "# keep me\nmodel = \"custom\"\n[mcp_servers.other]\ncommand = \"other\"\n",
    )
    .unwrap();
    let manager = GlobalManager::open(&f.state).unwrap();
    let plan = manager
        .preview(&f.home, &f.executable, Client::Codex)
        .unwrap();
    assert_eq!(
        plan.server_entry["args"],
        serde_json::json!([
            "--state-dir",
            f.state.canonicalize().unwrap(),
            "mcp",
            "--projects"
        ])
    );
    manager.apply(&plan).unwrap();
    let saved = fs::read_to_string(f.home.join(".codex/config.toml")).unwrap();
    assert!(saved.contains("# keep me"));
    assert!(saved.contains("other"));
    assert!(!saved.contains("attachment"));
    assert!(!saved.contains("approval_mode"));
    assert!(f.home.join(".agents/skills/apr/SKILL.md").exists());
}

#[test]
fn opted_in_codex_run_trust_is_narrow_and_preserves_unrelated_settings() {
    let f = Fixture::new();
    fs::create_dir(f.home.join(".codex")).unwrap();
    fs::write(
        f.home.join(".codex/config.toml"),
        "# owner setting\nmodel = \"custom\"\n[mcp_servers.other]\ncommand = \"other\"\n",
    )
    .unwrap();
    let manager = GlobalManager::open(&f.state).unwrap();
    let options = GlobalInstallOptions {
        trust_apr_run: true,
    };
    let plan = manager
        .preview_with_options(&f.home, &f.executable, Client::Codex, options)
        .unwrap();
    assert!(plan.trust_apr_run);
    assert_eq!(
        plan.server_entry["tools"]["apr_run"]["approval_mode"],
        "approve"
    );
    assert!(
        plan.server_entry
            .get("default_tools_approval_mode")
            .is_none()
    );
    manager.apply(&plan).unwrap();
    assert!(!manager.apply(&plan).unwrap().changed);
    let saved = fs::read_to_string(f.home.join(".codex/config.toml")).unwrap();
    let doc: toml_edit::DocumentMut = saved.parse().unwrap();
    assert_eq!(doc["model"].as_str(), Some("custom"));
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("other")
    );
    assert_eq!(
        doc["mcp_servers"]["apr"]["tools"]["apr_run"]["approval_mode"].as_str(),
        Some("approve")
    );
    assert!(
        doc["mcp_servers"]["apr"]
            .get("default_tools_approval_mode")
            .is_none()
    );
    assert!(saved.contains("# owner setting"));
    manager.uninstall(&f.home, Client::Codex).unwrap();
    let saved = fs::read_to_string(f.home.join(".codex/config.toml")).unwrap();
    assert!(saved.contains("# owner setting"));
    assert!(!saved.contains("apr_run"));
    assert!(saved.contains("[mcp_servers.other]"));
}

#[test]
fn trust_change_requires_fresh_preview_and_rejects_edited_owned_entry() {
    let f = Fixture::new();
    let manager = GlobalManager::open(&f.state).unwrap();
    let default_plan = manager
        .preview(&f.home, &f.executable, Client::Codex)
        .unwrap();
    manager.apply(&default_plan).unwrap();
    let options = GlobalInstallOptions {
        trust_apr_run: true,
    };
    let trust_plan = manager
        .preview_with_options(&f.home, &f.executable, Client::Codex, options)
        .unwrap();
    assert_ne!(trust_plan.plan_id, default_plan.plan_id);
    // A later settings edit cannot be covered by the earlier owner review.
    let path = f.home.join(".codex/config.toml");
    let mut saved = fs::read_to_string(&path).unwrap();
    saved.push_str("\n# later owner edit\n");
    fs::write(&path, saved).unwrap();
    assert_eq!(
        manager.apply(&trust_plan).unwrap_err().reason(),
        "stale_preview"
    );
    let refreshed = manager
        .preview_with_options(&f.home, &f.executable, Client::Codex, options)
        .unwrap();
    manager.apply(&refreshed).unwrap();
    let mut saved = fs::read_to_string(&path).unwrap();
    saved.push_str("\n[mcp_servers.apr.tools.apr_manage]\napproval_mode = \"approve\"\n");
    fs::write(&path, saved).unwrap();
    assert_eq!(
        manager
            .preview_with_options(&f.home, &f.executable, Client::Codex, options)
            .unwrap_err()
            .reason(),
        "owned_entry_edited"
    );
}

#[test]
fn codex_trust_option_rejects_other_clients() {
    let f = Fixture::new();
    let manager = GlobalManager::open(&f.state).unwrap();
    assert_eq!(
        manager
            .preview_with_options(
                &f.home,
                &f.executable,
                Client::Claude,
                GlobalInstallOptions {
                    trust_apr_run: true,
                },
            )
            .unwrap_err()
            .reason(),
        "codex_trust_requires_codex"
    );
}

#[test]
fn stale_and_unowned_global_material_is_never_replaced() {
    let f = Fixture::new();
    let manager = GlobalManager::open(&f.state).unwrap();
    let stale = manager
        .preview(&f.home, &f.executable, Client::Claude)
        .unwrap();
    fs::write(f.home.join(".claude.json"), r#"{"later":true}"#).unwrap();
    assert_eq!(manager.apply(&stale).unwrap_err().reason(), "stale_preview");
    assert_eq!(
        fs::read_to_string(f.home.join(".claude.json")).unwrap(),
        r#"{"later":true}"#
    );
    fs::remove_file(f.home.join(".claude.json")).unwrap();
    fs::write(
        f.home.join(".claude.json"),
        r#"{"mcpServers":{"apr":{"command":"user"}}}"#,
    )
    .unwrap();
    assert_eq!(manager.apply(&stale).unwrap_err().reason(), "unowned_entry");
    fs::remove_file(f.home.join(".claude.json")).unwrap();
    fs::create_dir_all(f.home.join(".claude/skills/apr")).unwrap();
    fs::write(f.home.join(".claude/skills/apr/SKILL.md"), "user skill").unwrap();
    assert_eq!(
        manager
            .preview(&f.home, &f.executable, Client::Claude)
            .unwrap_err()
            .reason(),
        "unowned_skill"
    );
}

#[test]
fn uninstall_preserves_later_settings_and_refuses_changed_owned_skill() {
    let f = Fixture::new();
    let manager = GlobalManager::open(&f.state).unwrap();
    let plan = manager
        .preview(&f.home, &f.executable, Client::Claude)
        .unwrap();
    manager.apply(&plan).unwrap();
    let config = f.home.join(".claude.json");
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    json["later"] = serde_json::json!(true);
    fs::write(&config, serde_json::to_vec(&json).unwrap()).unwrap();
    manager.uninstall(&f.home, Client::Claude).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(saved["later"], true);
    assert!(saved["mcpServers"].get("apr").is_none());
    let plan = manager
        .preview(&f.home, &f.executable, Client::Claude)
        .unwrap();
    manager.apply(&plan).unwrap();
    fs::write(f.home.join(".claude/skills/apr/SKILL.md"), "changed").unwrap();
    assert_eq!(
        manager
            .uninstall(&f.home, Client::Claude)
            .unwrap_err()
            .reason(),
        "owned_skill_edited"
    );
}

#[test]
fn symlinked_settings_are_refused() {
    let f = Fixture::new();
    fs::create_dir(f.home.join(".codex")).unwrap();
    let outside = f.home.join("outside");
    fs::write(&outside, "").unwrap();
    symlink(&outside, f.home.join(".codex/config.toml")).unwrap();
    let manager = GlobalManager::open(&f.state).unwrap();
    assert!(matches!(
        manager.preview(&f.home, &f.executable, Client::Codex),
        Err(Error::UnsafePath)
    ));
}

#[test]
fn retry_completes_an_interrupted_global_install() {
    let f = Fixture::new();
    fs::create_dir_all(f.home.join(".agents/skills/apr")).unwrap();
    let manager = GlobalManager::open(&f.state).unwrap();
    let plan = manager
        .preview(&f.home, &f.executable, Client::Codex)
        .unwrap();
    let parent = f.home.join(".agents/skills/apr");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(manager.apply(&plan).is_err());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(manager.apply(&plan).unwrap().state, "installed");
    assert!(f.home.join(".agents/skills/apr/SKILL.md").exists());
}
