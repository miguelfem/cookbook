use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

fn command(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_apr"))
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn capacity_show_and_set_are_explicit_owner_commands() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let defaults = success(command(&state, &["capacity", "show"]));
    assert_eq!(
        defaults["policy"],
        json!({"max_call_depth":32,"max_child_runs":1024,"max_step_instances":10000,"max_graph_versions":4096})
    );
    assert_eq!(defaults["hard_ceilings"]["max_call_depth"], 128);
    assert_eq!(defaults["applies_to"], "new_roots");
    let set = success(command(
        &state,
        &[
            "capacity",
            "set",
            "--max-call-depth",
            "64",
            "--max-child-runs",
            "2048",
            "--max-step-instances",
            "30000",
            "--max-graph-versions",
            "2048",
        ],
    ));
    assert_eq!(set["policy"]["max_call_depth"], 64);
    assert_eq!(set["existing_runs_changed"], false);
    assert_eq!(success(command(&state, &["capacity", "show"])), set);
    for args in [
        vec!["capacity", "set", "--max-call-depth", "32"],
        vec![
            "capacity",
            "set",
            "--max-call-depth",
            "129",
            "--max-child-runs",
            "1024",
            "--max-step-instances",
            "10000",
            "--max-graph-versions",
            "4096",
        ],
    ] {
        let rejected = command(&state, &args);
        assert!(!rejected.status.success());
        assert!(rejected.stdout.is_empty());
        assert_eq!(success(command(&state, &["capacity", "show"])), set);
    }
    let help = command(&state, &["capacity", "set", "--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--max-call-depth"));
}
