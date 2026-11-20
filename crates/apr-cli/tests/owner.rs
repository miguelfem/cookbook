use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn command(state: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apr"));
    command.arg("--state-dir").arg(state).args(args);
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

#[test]
fn owner_controls_configure_project_values_promotion_and_nonsecret_connections() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let state = directory.path().join("state");
    fs::write(
        project.join("process.json"),
        include_bytes!("../../../examples/local-review/process.json"),
    )
    .unwrap();
    let mut runtime = apr_runtime::Runtime::open(&state).unwrap();
    let project = runtime.attach_project(&project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let source = runtime.create_assignment(&project, "source").unwrap()["assignment_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let target = runtime.create_assignment(&project, "target").unwrap()["assignment_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = runtime.create_attachment(&source, "author").unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let registered = runtime
        .scoped_call(
            &token,
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )
        .unwrap();
    let reference = registered["process_ref"].as_str().unwrap();
    let values_file = directory.path().join("values.json");
    fs::write(&values_file, "{}").unwrap();
    let mut set = command(
        &state,
        &[
            "config",
            "project-set",
            "--project",
            &project,
            "--process-ref",
            reference,
            "--values-file",
        ],
    );
    set.arg(&values_file);
    assert_eq!(success(set)["project_revision"], 1);
    assert_eq!(
        success(command(
            &state,
            &[
                "permission",
                "promote",
                "--source",
                &source,
                "--target",
                &target,
                "--enabled",
                "true"
            ]
        ))["enabled"],
        true
    );
    assert_eq!(
        success(command(
            &state,
            &[
                "permission",
                "promote",
                "--source",
                &source,
                "--target",
                &target,
                "--enabled",
                "false"
            ]
        ))["enabled"],
        false
    );

    let connection_file = directory.path().join("connection.json");
    fs::write(&connection_file, json!({"label":"Test configuration only","endpoint":"https://example.invalid/v1","method":"POST","auth":{"kind":"none"},"input_schema":{"type":"object"},"output_schema":{"type":"object"},"timeout_ms":1000,"max_response_bytes":1024}).to_string()).unwrap();
    let mut configure = command(
        &state,
        &[
            "execution",
            "configure-connection",
            "--assignment",
            &source,
            "--name",
            "example",
            "--connection-file",
        ],
    );
    configure.arg(&connection_file);
    assert_eq!(success(configure)["name"], "example");
    assert_eq!(
        success(command(
            &state,
            &[
                "execution",
                "revoke",
                "--assignment",
                &source,
                "--kind",
                "connection",
                "--name",
                "example"
            ]
        ))["revoked"],
        true
    );

    let mut invalid_secret = command(
        &state,
        &[
            "execution",
            "configure-connection",
            "--assignment",
            &source,
            "--name",
            "example",
            "--connection-file",
        ],
    );
    invalid_secret
        .arg(&connection_file)
        .arg("--secret-stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = invalid_secret.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"DO_NOT_PRINT_THIS_SECRET\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(!error.contains("DO_NOT_PRINT_THIS_SECRET"));
    assert_eq!(
        serde_json::from_str::<Value>(&error).unwrap()["error"]["code"],
        "INVALID_INPUT"
    );
}
