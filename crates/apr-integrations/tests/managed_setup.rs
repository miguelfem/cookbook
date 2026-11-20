use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use apr_integrations::{Client, Error, InstallRequest, Manager};

struct Fixture {
    _temp: tempfile::TempDir,
    state: PathBuf,
    project: PathBuf,
    request: InstallRequest,
}

impl Fixture {
    fn new(client: Client) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("private state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let project = temp.path().join("project with spaces");
        fs::create_dir(&project).unwrap();
        let executable = temp.path().join("apr executable");
        fs::write(
            &executable,
            "synthetic executable; setup must not invoke it",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let attachment_file = state.join("attachment.json");
        fs::write(
            &attachment_file,
            "PRIVATE_ATTACHMENT_CONTENT_MUST_NOT_BE_READ",
        )
        .unwrap();
        fs::set_permissions(&attachment_file, fs::Permissions::from_mode(0o600)).unwrap();
        let request = InstallRequest {
            project_dir: project.clone(),
            attachment_file,
            executable,
            client,
        };
        Self {
            _temp: temp,
            state,
            project,
            request,
        }
    }
}

#[test]
fn preview_then_apply_codex_preserves_unrelated_configuration_and_uses_local_paths() {
    let fixture = Fixture::new(Client::Codex);
    fs::create_dir(fixture.project.join(".codex")).unwrap();
    let config = fixture.project.join(".codex/config.toml");
    let original = "# Keep this comment\nmodel = \"custom-model\"\n[mcp_servers.notes]\ncommand = \"notes-server\"\n";
    fs::write(&config, original).unwrap();
    let manager = Manager::open(&fixture.state).unwrap();
    let plan = manager.preview(&fixture.request).unwrap();
    assert_eq!(fs::read_to_string(&config).unwrap(), original);
    assert!(!fixture.project.join(".agents/skills/apr/SKILL.md").exists());
    assert!(
        !serde_json::to_string(&plan)
            .unwrap()
            .contains("PRIVATE_ATTACHMENT_CONTENT")
    );
    let result = manager.apply(&plan).unwrap();
    assert!(result.changed);
    assert_eq!(result.state, "installed");
    let saved = fs::read_to_string(&config).unwrap();
    assert!(saved.contains("# Keep this comment"));
    let document = saved.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(document["model"].as_str(), Some("custom-model"));
    assert_eq!(
        document["mcp_servers"]["notes"]["command"].as_str(),
        Some("notes-server")
    );
    let executable = fixture.request.executable.canonicalize().unwrap();
    assert_eq!(
        document["mcp_servers"]["apr"]["command"].as_str(),
        executable.to_str()
    );
    let args = document["mcp_servers"]["apr"]["args"].as_array().unwrap();
    assert_eq!(args.get(0).unwrap().as_str(), Some("mcp"));
    assert_eq!(args.get(1).unwrap().as_str(), Some("--attachment-file"));
    let attachment = fixture.request.attachment_file.canonicalize().unwrap();
    assert_eq!(args.get(2).unwrap().as_str(), attachment.to_str());
    let skill = fs::read_to_string(fixture.project.join(".agents/skills/apr/SKILL.md")).unwrap();
    assert!(skill.contains("name: apr"));
}

#[test]
fn project_setup_rejects_nested_changes_to_its_owned_codex_entry() {
    let fixture = Fixture::new(Client::Codex);
    let manager = Manager::open(&fixture.state).unwrap();
    let plan = manager.preview(&fixture.request).unwrap();
    manager.apply(&plan).unwrap();
    let config = fixture.project.join(".codex/config.toml");
    let mut edited = fs::read_to_string(&config).unwrap();
    edited.push_str("\n[mcp_servers.apr.tools.apr_manage]\napproval_mode = \"approve\"\n");
    fs::write(&config, &edited).unwrap();
    assert_eq!(
        manager.preview(&fixture.request).unwrap_err().reason(),
        "owned_entry_edited"
    );
    assert_eq!(
        manager
            .uninstall(&fixture.project, Client::Codex)
            .unwrap_err()
            .reason(),
        "owned_entry_edited"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), edited);
}

#[test]
fn claude_project_setup_preserves_existing_servers_and_policy_values() {
    let fixture = Fixture::new(Client::Claude);
    let config = fixture.project.join(".mcp.json");
    fs::write(&config, r#"{"mcpServers":{"notes":{"type":"stdio","command":"notes-server","env":{"TOKEN":"PRIVATE_EXISTING_VALUE"}}},"customPolicy":{"keep":true}}"#).unwrap();
    let manager = Manager::open(&fixture.state).unwrap();
    let plan = manager.preview(&fixture.request).unwrap();
    assert!(
        !serde_json::to_string(&plan)
            .unwrap()
            .contains("PRIVATE_EXISTING_VALUE")
    );
    manager.apply(&plan).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        saved["mcpServers"]["notes"]["env"]["TOKEN"],
        "PRIVATE_EXISTING_VALUE"
    );
    assert_eq!(saved["customPolicy"]["keep"], true);
    assert_eq!(saved["mcpServers"]["apr"]["type"], "stdio");
    assert!(saved.get("enableAllProjectMcpServers").is_none());
    assert!(fixture.project.join(".claude/skills/apr/SKILL.md").exists());
}

#[test]
fn active_owner_blocks_update_without_changing_installed_files() {
    for client in [Client::Codex, Client::Claude] {
        let fixture = Fixture::new(client);
        let manager = Manager::open(&fixture.state).unwrap();
        let plan = manager.preview(&fixture.request).unwrap();
        manager.apply(&plan).unwrap();
        let config_path = fixture.project.join(client.config_path());
        let original = fs::read(&config_path).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(
                fixture
                    .state
                    .join("integrations")
                    .join(&plan.integration_id)
                    .join("lock"),
            )
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&lock).unwrap();

        assert!(matches!(manager.update(&fixture.request), Err(Error::Busy)));
        assert_eq!(fs::read(&config_path).unwrap(), original);

        fs2::FileExt::unlock(&lock).unwrap();
        assert!(!manager.update(&fixture.request).unwrap().changed);
        assert_eq!(fs::read(&config_path).unwrap(), original);
    }
}

#[test]
fn retrying_an_applied_plan_and_reapplying_unchanged_setup_are_noops() {
    for client in [Client::Codex, Client::Claude] {
        let fixture = Fixture::new(client);
        let manager = Manager::open(&fixture.state).unwrap();
        let plan = manager.preview(&fixture.request).unwrap();
        manager.apply(&plan).unwrap();
        let config = fs::read(fixture.project.join(client.config_path())).unwrap();
        assert!(!manager.apply(&plan).unwrap().changed);
        let unchanged = manager.preview(&fixture.request).unwrap();
        assert!(unchanged.changes.is_empty());
        assert!(!manager.apply(&unchanged).unwrap().changed);
        assert_eq!(
            fs::read(fixture.project.join(client.config_path())).unwrap(),
            config
        );
    }
}

#[test]
fn updating_the_attachment_preserves_unrelated_edits_made_after_installation() {
    let fixture = Fixture::new(Client::Claude);
    let manager = Manager::open(&fixture.state).unwrap();
    manager
        .apply(&manager.preview(&fixture.request).unwrap())
        .unwrap();
    let config = fixture.project.join(".mcp.json");
    let mut saved: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    saved["mcpServers"]["later"] = serde_json::json!({"command":"added-by-user"});
    saved["userSetting"] = serde_json::json!(17);
    fs::write(&config, serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut request = fixture.request.clone();
    request.attachment_file = fixture.state.join("another attachment.json");
    fs::write(&request.attachment_file, "ANOTHER_PRIVATE_VALUE").unwrap();
    fs::set_permissions(&request.attachment_file, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(manager.update(&request).unwrap().changed);
    let updated: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(updated["userSetting"], 17);
    assert_eq!(updated["mcpServers"]["later"]["command"], "added-by-user");
    assert_eq!(
        updated["mcpServers"]["apr"]["args"][2],
        request
            .attachment_file
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
}

#[test]
fn uninstall_removes_only_owned_material_and_preserves_later_user_settings() {
    for client in [Client::Codex, Client::Claude] {
        let fixture = Fixture::new(client);
        let manager = Manager::open(&fixture.state).unwrap();
        manager
            .apply(&manager.preview(&fixture.request).unwrap())
            .unwrap();
        let path = fixture.project.join(client.config_path());
        if client == Client::Codex {
            let mut bytes = fs::read_to_string(&path).unwrap();
            bytes.push_str(
                "\n# Added after setup\n[mcp_servers.later]\ncommand = \"user-command\"\n",
            );
            fs::write(&path, bytes).unwrap();
        } else {
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            value["mcpServers"]["later"] = serde_json::json!({"command":"user-command"});
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let result = manager.uninstall(&fixture.project, client).unwrap();
        assert_eq!(result.state, "uninstalled");
        assert!(result.changed);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("user-command"));
        if client == Client::Codex {
            assert!(saved.contains("# Added after setup"));
            let document = saved.parse::<toml_edit::DocumentMut>().unwrap();
            assert!(
                document["mcp_servers"]
                    .as_table_like()
                    .unwrap()
                    .get("apr")
                    .is_none()
            );
        } else {
            let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
            assert!(value["mcpServers"].get("apr").is_none());
        }
        assert!(!fixture.project.join(client.skill_path()).exists());
        assert!(!manager.uninstall(&fixture.project, client).unwrap().changed);
        assert!(fixture.request.attachment_file.exists());
    }
}

#[test]
fn retry_after_an_interrupted_multifile_apply_completes_the_saved_operation() {
    let fixture = Fixture::new(Client::Codex);
    let skill_parent = fixture.project.join(".agents/skills/apr");
    fs::create_dir_all(&skill_parent).unwrap();
    let manager = Manager::open(&fixture.state).unwrap();
    let plan = manager.preview(&fixture.request).unwrap();
    fs::set_permissions(&skill_parent, fs::Permissions::from_mode(0o500)).unwrap();
    let interrupted = manager.apply(&plan);
    fs::set_permissions(&skill_parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        interrupted.is_err(),
        "a read-only target must interrupt publication"
    );
    assert!(fixture.project.join(Client::Codex.config_path()).exists());
    let recovered = manager.apply(&plan).unwrap();
    assert_eq!(recovered.state, "installed");
    assert!(fixture.project.join(Client::Codex.skill_path()).exists());
    assert!(!manager.apply(&plan).unwrap().changed);
}

#[test]
fn stale_previews_and_user_edits_are_preserved_instead_of_overwritten() {
    let fixture = Fixture::new(Client::Codex);
    let manager = Manager::open(&fixture.state).unwrap();
    let stale = manager.preview(&fixture.request).unwrap();
    fs::create_dir(fixture.project.join(".codex")).unwrap();
    let config = fixture.project.join(Client::Codex.config_path());
    fs::write(&config, "model = \"added-after-preview\"\n").unwrap();
    assert_eq!(
        manager.apply(&stale).unwrap_err().code(),
        "INTEGRATION_CONFLICT"
    );
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "model = \"added-after-preview\"\n"
    );
    manager
        .apply(&manager.preview(&fixture.request).unwrap())
        .unwrap();
    let skill = fixture.project.join(Client::Codex.skill_path());
    fs::write(&skill, "User edited skill; preserve this body").unwrap();
    let before = fs::read(&config).unwrap();
    assert_eq!(
        manager.update(&fixture.request).unwrap_err().reason(),
        "owned_skill_edited"
    );
    assert_eq!(
        manager
            .uninstall(&fixture.project, Client::Codex)
            .unwrap_err()
            .reason(),
        "owned_skill_edited"
    );
    assert_eq!(fs::read(&config).unwrap(), before);
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        "User edited skill; preserve this body"
    );
}

#[test]
fn interrupted_recovery_refuses_to_replace_a_later_user_edit() {
    let fixture = Fixture::new(Client::Claude);
    let parent = fixture.project.join(".claude/skills/apr");
    fs::create_dir_all(&parent).unwrap();
    let manager = Manager::open(&fixture.state).unwrap();
    let plan = manager.preview(&fixture.request).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    let result = manager.apply(&plan);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    let path = fixture.project.join(".mcp.json");
    fs::write(&path, "{\"userChanged\":true}").unwrap();
    assert_eq!(
        manager.apply(&plan).unwrap_err().reason(),
        "recovery_target_edited"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"userChanged\":true}");
    assert!(!fixture.project.join(Client::Claude.skill_path()).exists());
}

#[test]
fn doctor_distinguishes_saved_configuration_missing_client_and_invalid_attachment() {
    let fixture = Fixture::new(Client::Claude);
    let manager = Manager::open(&fixture.state).unwrap();
    manager
        .apply(&manager.preview(&fixture.request).unwrap())
        .unwrap();
    let report = manager
        .doctor_with_client(
            &fixture.request,
            Some(&fixture.project.join("missing-client")),
        )
        .unwrap();
    assert_eq!(report.configuration, "configured");
    assert_eq!(report.client.status, "missing");
    assert_eq!(report.service, "unavailable");
    assert_eq!(report.client_trust, "unknown");
    assert_eq!(report.client_connection, "unverified");
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("PRIVATE_ATTACHMENT_CONTENT")
    );
}

#[test]
fn a_retried_update_recovers_its_journal_without_a_hidden_plan_file() {
    let fixture = Fixture::new(Client::Claude);
    let manager = Manager::open(&fixture.state).unwrap();
    manager
        .apply(&manager.preview(&fixture.request).unwrap())
        .unwrap();
    let mut updated = fixture.request.clone();
    updated.attachment_file = fixture.state.join("replacement.json");
    fs::write(&updated.attachment_file, "PRIVATE_REPLACEMENT").unwrap();
    fs::set_permissions(&updated.attachment_file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&fixture.project, fs::Permissions::from_mode(0o500)).unwrap();
    let result = manager.update(&updated);
    fs::set_permissions(&fixture.project, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert!(manager.update(&updated).unwrap().changed);
    assert!(!manager.update(&updated).unwrap().changed);
}

#[test]
fn tracked_project_config_is_left_unchanged_to_avoid_committing_device_paths() {
    let fixture = Fixture::new(Client::Claude);
    let config = fixture.project.join(".mcp.json");
    fs::write(&config, "{\"mcpServers\":{}}").unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(&fixture.project)
        .status()
        .unwrap();
    assert!(status.success());
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&fixture.project)
        .args(["add", "--", ".mcp.json"])
        .status()
        .unwrap();
    assert!(status.success());
    let manager = Manager::open(&fixture.state).unwrap();
    assert_eq!(
        manager.preview(&fixture.request).unwrap_err().reason(),
        "config_is_tracked"
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), "{\"mcpServers\":{}}");
}

#[test]
fn interrupted_apply_rechecks_config_tracking_before_finishing() {
    let fixture = Fixture::new(Client::Claude);
    let parent = fixture.project.join(".claude/skills/apr");
    fs::create_dir_all(&parent).unwrap();
    let manager = Manager::open(&fixture.state).unwrap();
    let plan = manager.preview(&fixture.request).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    let result = manager.apply(&plan);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err());
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(&fixture.project)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&fixture.project)
            .args(["add", "--", ".mcp.json"])
            .status()
            .unwrap()
            .success()
    );
    let config = fs::read(fixture.project.join(".mcp.json")).unwrap();
    assert_eq!(
        manager.apply(&plan).unwrap_err().reason(),
        "config_is_tracked"
    );
    assert_eq!(fs::read(fixture.project.join(".mcp.json")).unwrap(), config);
    assert!(!fixture.project.join(Client::Claude.skill_path()).exists());
}

#[test]
fn setup_refuses_unmanaged_entries_and_unsafe_files_without_changing_them() {
    use std::os::unix::fs::symlink;
    for kind in [
        "entry",
        "skill",
        "symlink",
        "parent_symlink",
        "hardlink",
        "writable_parent",
    ] {
        let fixture = Fixture::new(Client::Claude);
        let external = fixture.state.join("external-config");
        fs::write(&external, "{}").unwrap();
        let config = fixture.project.join(".mcp.json");
        match kind {
            "entry" => fs::write(
                &config,
                "{\"mcpServers\":{\"apr\":{\"command\":\"owned-by-user\"}}}",
            )
            .unwrap(),
            "skill" => {
                let parent = fixture.project.join(".claude/skills/apr");
                fs::create_dir_all(&parent).unwrap();
                fs::write(parent.join("SKILL.md"), "user skill").unwrap();
            }
            "symlink" => symlink(&external, &config).unwrap(),
            "parent_symlink" => {
                fs::create_dir(fixture.state.join("external-dir")).unwrap();
                symlink(
                    fixture.state.join("external-dir"),
                    fixture.project.join(".claude"),
                )
                .unwrap();
            }
            "hardlink" => fs::hard_link(&external, &config).unwrap(),
            "writable_parent" => {
                fs::create_dir(fixture.project.join(".claude")).unwrap();
                fs::set_permissions(
                    fixture.project.join(".claude"),
                    fs::Permissions::from_mode(0o777),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            Manager::open(&fixture.state)
                .unwrap()
                .preview(&fixture.request)
                .is_err(),
            "{kind}"
        );
        assert_eq!(fs::read_to_string(external).unwrap(), "{}");
    }
}
