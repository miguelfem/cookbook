use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn apr(state: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apr"));
    command.arg("--state-dir").arg(state);
    command
}
fn success(mut command: Command) -> Value {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn setup(state: &Path, project: &Path) -> PathBuf {
    let mut command = apr(state);
    command.args(["project", "attach"]).arg(project);
    let project = success(command);
    let mut command = apr(state);
    command.args([
        "assignment",
        "create",
        "--project",
        project["project_id"].as_str().unwrap(),
        "--name",
        "integration-test",
    ]);
    let assignment = success(command);
    let path = state.join("attachments/client.json");
    let mut command = apr(state);
    command
        .args([
            "attachment",
            "create",
            "--assignment",
            assignment["assignment_id"].as_str().unwrap(),
            "--mode",
            "author",
            "--output-file",
        ])
        .arg(&path);
    success(command);
    path
}

#[test]
fn cli_previews_applies_diagnoses_and_uninstalls_using_only_temporary_project_settings() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project with spaces");
    let state = directory.path().join("private state");
    fs::create_dir(&project).unwrap();
    fs::write(project.join(".mcp.json"), "{\"customSetting\":true}").unwrap();
    let attachment = setup(&state, &project);
    let plan_path = state.join("plans/claude.json");
    let mut command = apr(&state);
    command
        .args(["integrate", "preview", "--client", "claude", "--project"])
        .arg(&project)
        .arg("--attachment-file")
        .arg(&attachment)
        .arg("--output-file")
        .arg(&plan_path);
    let plan = success(command);
    assert_eq!(plan["request"]["client"], "claude");
    assert!(plan_path.exists());
    assert_eq!(
        fs::metadata(&plan_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::read_to_string(project.join(".mcp.json")).unwrap(),
        "{\"customSetting\":true}"
    );
    let mut command = apr(&state);
    command
        .args(["integrate", "apply", "--plan-file"])
        .arg(&plan_path);
    assert_eq!(success(command)["state"], "installed");

    let mut command = apr(&state);
    command
        .arg("service")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _service = Service(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while UnixStream::connect(apr_runtime::service::socket_path(&state).unwrap()).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let client_probe = directory.path().join("missing-client");
    let mut command = apr(&state);
    command
        .args(["integrate", "doctor", "--client", "claude", "--project"])
        .arg(&project)
        .arg("--client-executable")
        .arg(client_probe);
    let doctor = success(command);
    assert_eq!(doctor["configuration"], "configured");
    assert_eq!(doctor["service"], "ready");
    assert_eq!(doctor["client"]["status"], "missing");
    assert_eq!(doctor["client_trust"], "unknown");
    assert_eq!(doctor["client_connection"], "unverified");
    let mut command = apr(&state);
    command
        .args(["integrate", "uninstall", "--client", "claude", "--project"])
        .arg(&project);
    assert_eq!(success(command)["state"], "uninstalled");
    let saved: Value =
        serde_json::from_slice(&fs::read(project.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(saved, json!({"customSetting":true,"mcpServers":{}}));
    assert!(attachment.exists());
}

#[test]
fn cli_manages_one_time_agent_setup_only_in_an_explicit_temporary_home() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("private state");
    let home = directory.path().join("temporary home with spaces");
    assert!(!state.exists());
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    let plan_path = state.join("plans/global.json");
    let mut command = apr(&state);
    command
        .args(["integrate", "agent-preview", "--client", "codex", "--home"])
        .arg(&home)
        .arg("--executable")
        .arg(env!("CARGO_BIN_EXE_apr"))
        .arg("--output-file")
        .arg(&plan_path);
    let plan = success(command);
    assert_eq!(plan["server_entry"]["args"][0], "--state-dir");
    assert_eq!(plan["server_entry"]["args"][2], "mcp");
    assert_eq!(plan["server_entry"]["args"][3], "--projects");
    let mut command = apr(&state);
    command.args(["integrate", "agent-apply", "--plan-file"]);
    command.arg(&plan_path);
    assert_eq!(success(command)["state"], "installed");
    assert!(home.join(".codex/config.toml").exists());
    assert!(home.join(".agents/skills/apr/SKILL.md").exists());
    let mut command = apr(&state);
    command
        .args([
            "integrate",
            "agent-uninstall",
            "--client",
            "codex",
            "--home",
        ])
        .arg(&home);
    assert_eq!(success(command)["state"], "uninstalled");
}
