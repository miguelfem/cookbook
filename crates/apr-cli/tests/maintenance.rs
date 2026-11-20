use std::fs;
use std::path::Path;
use std::process::Command;

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
fn cli_backups_restore_without_overwrite_and_retention_requires_a_saved_plan() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let mut runtime = apr_runtime::Runtime::open(&state).unwrap();
    let project_id = runtime.attach_project(&project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let source = directory.path().join("source.txt");
    fs::write(&source, "Test evidence").unwrap();
    let artifact = runtime
        .import_artifact(&source, "text/plain", 1024)
        .unwrap();
    let process = apr_core::admit_json(include_bytes!(
        "../../../examples/local-review/process.json"
    ))
    .unwrap();
    let run = runtime
        .start(
            &process,
            json!({"record":{"title":"Saved work"},"sources":[artifact]}),
            "backup-test",
        )
        .unwrap();
    drop(runtime);
    assert!(
        success(command(&state, &["storage", "usage"]))["stored_object_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    success(command(
        &state,
        &["storage", "pin-run", &run.run_id, "--pinned", "true"],
    ));
    let backup = directory.path().join("backup");
    let mut make_backup = command(&state, &["backup", "--output"]);
    make_backup.arg(&backup);
    assert_eq!(success(make_backup)["runs"], 1);
    let restored = directory.path().join("restored");
    let mut restore = command(&restored, &["restore"]);
    restore.arg(&backup);
    assert_eq!(success(restore)["runs"], 1);
    assert_eq!(
        success(command(&restored, &["inspect", &run.run_id]))["state"],
        "waiting_for_agent"
    );
    let mut duplicate = command(&restored, &["restore"]);
    duplicate.arg(&backup);
    let duplicate = duplicate.output().unwrap();
    assert!(!duplicate.status.success());
    assert!(duplicate.stdout.is_empty());
    let new_root = directory.path().join("new project");
    fs::create_dir(&new_root).unwrap();
    let mut relocate = command(&restored, &["project", "relocate", &project_id]);
    relocate.arg(&new_root);
    assert_eq!(success(relocate)["relocated"], true);
    let plan = state.join("plans/retention.json");
    let mut preview = command(
        &state,
        &[
            "storage",
            "retention-preview",
            "--terminal-before-ms",
            "0",
            "--orphan-before-ms",
            "0",
            "--output-file",
        ],
    );
    preview.arg(&plan);
    success(preview);
    let mut apply = command(&state, &["storage", "retention-apply", "--plan-file"]);
    apply.arg(&plan);
    assert_eq!(success(apply)["removed_runs"], 0);

    success(command(&state, &["storage", "quota", "--bytes", "1"]));
    assert_eq!(
        success(command(&state, &["storage", "usage"]))["quota_bytes"],
        1
    );
    let extra = directory.path().join("extra.txt");
    fs::write(&extra, "New object growth").unwrap();
    let mut import = command(&state, &["artifact", "import"]);
    import.arg(&extra).args(["--media-type", "text/plain"]);
    let refused = import.output().unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&refused.stderr).unwrap()["error"]["code"],
        "LIMIT_EXCEEDED"
    );
    success(command(&state, &["storage", "quota", "--unlimited"]));
    let mut import = command(&state, &["artifact", "import"]);
    import.arg(&extra).args(["--media-type", "text/plain"]);
    assert!(
        success(import)["artifact_id"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}
