use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn invoke(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_apr"))
        .current_dir(directory)
        .args(["--state-dir", "state"])
        .args(args)
        .output()
        .unwrap()
}

fn success(directory: &Path, args: &[&str]) -> Value {
    let output = invoke(directory, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "unexpected stderr for {args:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn failure(directory: &Path, args: &[&str], code: &str) -> Value {
    let output = invoke(directory, args);
    assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
    assert!(
        output.stdout.is_empty(),
        "failure wrote to stdout: {args:?}"
    );
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], code, "{error}");
    error
}

fn write_json(directory: &Path, name: &str, value: &Value) {
    fs::write(
        directory.join(name),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .unwrap();
}

fn process() -> Value {
    serde_json::from_str(include_str!("../../../examples/local-review/process.json")).unwrap()
}

fn setup(directory: &Path) {
    write_json(directory, "process.json", &process());
    fs::write(directory.join("source.txt"), b"Evidence for the review.\n").unwrap();
    let artifact = success(
        directory,
        &[
            "artifact",
            "import",
            "source.txt",
            "--media-type",
            "text/plain",
        ],
    );
    write_json(
        directory,
        "input.json",
        &json!({"record":{"title":"Local record"},"sources":[artifact]}),
    );
}

fn start(directory: &Path, key: &str) -> Value {
    success(
        directory,
        &[
            "run",
            "process.json",
            "--input-file",
            "input.json",
            "--request-key",
            key,
        ],
    )
}

fn submission(waiting: &Value) -> Value {
    let request = &waiting["request"];
    json!({
        "protocol_version":"0.1","action":"submit","run_id":waiting["run_id"],
        "request_id":request["request_id"],"step_instance_id":request["step_instance_id"],
        "generation":request["generation"],"submission_key":"review-result-1",
        "continuation_token":request["continuation_token"],"output":{"approved":true},
        "provenance":{"source":"caller_tool","client":"cli-integration-test","verification":"synthetic_fixture"}
    })
}

#[test]
fn process_validation_is_stateless_and_file_limits_precede_parsing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    write_json(root, "process.json", &process());
    let validated = success(root, &["process", "validate", "process.json"]);
    assert_eq!(validated["valid"], true);
    assert_eq!(validated["steps"], 6);
    assert!(
        validated["process_version"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(!root.join("state").exists());

    fs::write(root.join("oversized.json"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    failure(
        root,
        &["process", "validate", "oversized.json"],
        "LIMIT_EXCEEDED",
    );
    fs::write(
        root.join("invalid.json"),
        br#"{"private":"DO_NOT_ECHO_SECRET","schema_version":false}"#,
    )
    .unwrap();
    let error = failure(
        root,
        &["process", "validate", "invalid.json"],
        "INVALID_MANIFEST",
    );
    assert!(!error.to_string().contains("DO_NOT_ECHO_SECRET"));

    fs::write(root.join("input.json"), vec![b' '; 64 * 1024 + 1]).unwrap();
    failure(
        root,
        &[
            "run",
            "process.json",
            "--input-file",
            "input.json",
            "--request-key",
            "too-large",
        ],
        "LIMIT_EXCEEDED",
    );
    assert!(!root.join("state").exists());
}

#[test]
fn all_builtins_resume_across_cli_restarts_without_repeating_committed_work() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    setup(root);
    let waiting = start(root, "run-1");
    let run_id = waiting["run_id"].as_str().unwrap();
    assert_eq!(waiting["state"], "waiting_for_agent", "{waiting}");
    assert_eq!(waiting["completed_steps"], 4);
    assert_eq!(
        waiting["request"]["input"]["record"]["title"],
        "Local record"
    );
    assert_eq!(success(root, &["inspect", run_id]), waiting);
    assert_eq!(success(root, &["continue", run_id]), waiting);
    assert_eq!(start(root, "run-1"), waiting);

    write_json(root, "submission.json", &submission(&waiting));
    let accepted = success(
        root,
        &["resume", run_id, "--result-file", "submission.json"],
    );
    assert_eq!(accepted["receipt"]["status"], "accepted");
    assert_eq!(accepted["run"]["state"], "succeeded", "{accepted}");
    assert_eq!(accepted["run"]["completed_steps"], 6);
    assert_eq!(accepted["run"]["output"]["approved"], true);
    assert_eq!(
        success(
            root,
            &["resume", run_id, "--result-file", "submission.json"]
        ),
        accepted
    );
    assert_eq!(success(root, &["continue", run_id]), accepted["run"]);

    let first = success(root, &["events", run_id, "--limit", "2"]);
    let events = first.as_array().unwrap();
    assert_eq!(events.len(), 2);
    let cursor = events[1]["sequence"].as_u64().unwrap();
    let next = success(
        root,
        &[
            "events",
            run_id,
            "--after",
            &cursor.to_string(),
            "--limit",
            "2",
        ],
    );
    assert!(
        next.as_array()
            .unwrap()
            .iter()
            .all(|event| event["sequence"].as_u64().unwrap() > cursor)
    );
    let all = success(root, &["events", run_id]);
    let token = waiting["request"]["continuation_token"].as_str().unwrap();
    assert!(!all.to_string().contains(token));
    assert_eq!(
        all.as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == "handoff.accepted")
            .count(),
        1
    );

    // A distinct run with the same accepted review produces the same ZIP bytes.
    let second = start(root, "run-2");
    write_json(root, "submission-2.json", &submission(&second));
    let repeated = success(
        root,
        &[
            "resume",
            second["run_id"].as_str().unwrap(),
            "--result-file",
            "submission-2.json",
        ],
    );
    assert_eq!(
        repeated["run"]["output"]["package"],
        accepted["run"]["output"]["package"]
    );

    let reference = &accepted["run"]["output"]["package"];
    write_json(root, "package-ref.json", reference);
    let exported = success(
        root,
        &[
            "artifact",
            "read",
            "--reference-file",
            "package-ref.json",
            "--output-file",
            "package.zip",
        ],
    );
    let bytes = fs::read(root.join("package.zip")).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"));
    assert_eq!(exported["bytes_written"], bytes.len());
    assert_eq!(reference["bytes"], bytes.len());

    let mut different: Value =
        serde_json::from_slice(&fs::read(root.join("input.json")).unwrap()).unwrap();
    different["record"]["title"] = json!("A different record");
    write_json(root, "different.json", &different);
    failure(
        root,
        &[
            "run",
            "process.json",
            "--input-file",
            "different.json",
            "--request-key",
            "run-1",
        ],
        "REQUEST_CONFLICT",
    );
}

#[test]
fn renewed_handoffs_reject_stale_tokens_and_accepted_receipts_replay() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    setup(root);
    let waiting = start(root, "renew-test");
    let run_id = waiting["run_id"].as_str().unwrap();
    write_json(root, "old.json", &submission(&waiting));
    let renewed = success(root, &["renew", run_id]);
    assert_eq!(renewed["state"], "waiting_for_agent");
    assert!(
        renewed["request"]["generation"].as_u64().unwrap()
            > waiting["request"]["generation"].as_u64().unwrap()
    );
    assert_ne!(
        renewed["request"]["continuation_token"],
        waiting["request"]["continuation_token"]
    );
    failure(
        root,
        &["resume", run_id, "--result-file", "old.json"],
        "STALE_HANDOFF",
    );
    write_json(root, "renewed.json", &submission(&renewed));
    let accepted = success(root, &["resume", run_id, "--result-file", "renewed.json"]);
    assert_eq!(accepted["run"]["state"], "succeeded");
    assert_eq!(
        success(root, &["resume", run_id, "--result-file", "renewed.json"]),
        accepted
    );
}

#[test]
fn bad_submissions_and_argument_values_never_echo_file_contents_or_mutate_state() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let fake = json!({"run_id":"another-run","request":{"request_id":"request","step_instance_id":"review","generation":1,"continuation_token":"PRIVATE_TOKEN"}});
    write_json(root, "submission.json", &submission(&fake));
    let error = failure(
        root,
        &[
            "resume",
            "different-run",
            "--result-file",
            "submission.json",
        ],
        "INVALID_INPUT",
    );
    assert!(!error.to_string().contains("PRIVATE_TOKEN"));
    fs::write(root.join("large.json"), vec![b'x'; 64 * 1024 + 1]).unwrap();
    failure(
        root,
        &["resume", "run", "--result-file", "large.json"],
        "LIMIT_EXCEEDED",
    );
    fs::write(
        root.join("bad.json"),
        br#"{"output":"PRIVATE_PAYLOAD","unexpected":true}"#,
    )
    .unwrap();
    let error = failure(
        root,
        &["resume", "run", "--result-file", "bad.json"],
        "INVALID_INPUT",
    );
    assert!(!error.to_string().contains("PRIVATE_PAYLOAD"));
    let error = failure(root, &["--PRIVATE_ARGUMENT"], "INVALID_INPUT");
    assert!(!error.to_string().contains("PRIVATE_ARGUMENT"));
    assert!(!root.join("state").exists());
}

#[test]
fn artifact_export_never_overwrites_existing_files_and_checks_integrity_first() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    fs::write(root.join("source.txt"), b"immutable source").unwrap();
    let reference = success(
        root,
        &[
            "artifact",
            "import",
            "source.txt",
            "--media-type",
            "text/plain",
        ],
    );
    write_json(root, "reference.json", &reference);
    fs::write(root.join("existing.txt"), b"keep this file").unwrap();
    failure(
        root,
        &[
            "artifact",
            "read",
            "--reference-file",
            "reference.json",
            "--output-file",
            "existing.txt",
        ],
        "INTERNAL_ERROR",
    );
    assert_eq!(
        fs::read(root.join("existing.txt")).unwrap(),
        b"keep this file"
    );
    std::os::unix::fs::symlink("missing-target.txt", root.join("existing-link.txt")).unwrap();
    failure(
        root,
        &[
            "artifact",
            "read",
            "--reference-file",
            "reference.json",
            "--output-file",
            "existing-link.txt",
        ],
        "INTERNAL_ERROR",
    );
    assert!(!root.join("missing-target.txt").exists());
    assert!(root.join("existing-link.txt").is_symlink());
    let object = root
        .join("state/objects/sha256")
        .join(reference["sha256"].as_str().unwrap());
    fs::write(object, b"corrupted source").unwrap();
    failure(
        root,
        &[
            "artifact",
            "read",
            "--reference-file",
            "reference.json",
            "--output-file",
            "new.txt",
        ],
        "ARTIFACT_INTEGRITY",
    );
    assert!(!root.join("new.txt").exists());
}
