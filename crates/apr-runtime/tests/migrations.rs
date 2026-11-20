use apr_core::{AdmittedProcess, admit_value, content_digest};
use apr_runtime::{Runtime, Submission, SubmissionReceipt};
use rusqlite::{Connection, params};
use serde_json::json;
use std::path::Path;

fn legacy_process() -> AdmittedProcess {
    admit_value(&json!({"schema_version":"0.1","name":"legacy.review","description":"Legacy process",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{"mode":"legacy-default"},"run_override_keys":[]},"capabilities":[{"kind":"agent"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1000000,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[{"id":"review","kind":"agent","needs":[],"effect":"agent","retry":{"max_attempts":1},
            "capability":"review","instructions":"Review legacy data","inputs":{},"result_schema":{"type":"object"}}],
        "output":{"result":{"step":"review","pointer":""}}})).unwrap()
}

fn seed_v1(root: &Path) -> (AdmittedProcess, Submission, SubmissionReceipt) {
    std::fs::create_dir_all(root).unwrap();
    let db = Connection::open(root.join("state.sqlite3")).unwrap();
    db.execute_batch(include_str!("fixtures/state-v1.sql"))
        .unwrap();
    let process = legacy_process();
    db.execute("INSERT INTO process_versions(version,manifest,canonical_format) VALUES (?1,?2,'apr-json-v1')",
        params![process.version,String::from_utf8_lossy(&process.canonical_bytes)]).unwrap();
    let digest = content_digest(&json!({"version":process.version,"input":{}})).unwrap();
    for (index, key) in ["legacy-key", r#"[null,"legacy-key"]"#].iter().enumerate() {
        let id = format!("legacy-run-{index}");
        db.execute("INSERT INTO runs(id,request_key,request_digest,version,input,state,revision,output,created_at) VALUES (?1,?2,?3,?4,'{}','succeeded',7,?5,1000)",
            params![id,key,digest,process.version,r#"{"result":{"approved":true}}"#]).unwrap();
        db.execute("INSERT INTO steps(run_id,id,position,state,output) VALUES (?1,'review',0,'succeeded',?2)",params![id,r#"{"approved":true}"#]).unwrap();
        db.execute("INSERT INTO events(sequence,run_id,kind,detail,created_at) VALUES (?1,?2,'run.succeeded','{}',1000)",params![10+index,id]).unwrap();
    }
    let submission = Submission {
        protocol_version: "0.1".into(),
        action: "submit".into(),
        run_id: "legacy-run-0".into(),
        request_id: "legacy-request".into(),
        step_instance_id: "review".into(),
        generation: 1,
        submission_key: "legacy-result".into(),
        continuation_token: "consumed-transport-token".into(),
        output: json!({"approved":true}),
        provenance: json!({"source":"manual"}),
    };
    let semantic = content_digest(&json!({"run_id":submission.run_id,"request_id":submission.request_id,
        "step_instance_id":submission.step_instance_id,"generation":submission.generation,"output":submission.output,"provenance":submission.provenance})).unwrap();
    let receipt = SubmissionReceipt {
        protocol_version: "0.1".into(),
        run_id: submission.run_id.clone(),
        request_id: submission.request_id.clone(),
        submission_key: submission.submission_key.clone(),
        output_digest: content_digest(&submission.output).unwrap(),
        status: "accepted".into(),
        revision: 6,
    };
    db.execute(
        "INSERT INTO receipts(run_id,submission_key,semantic_digest,receipt) VALUES (?1,?2,?3,?4)",
        params![
            submission.run_id,
            submission.submission_key,
            semantic,
            serde_json::to_string(&receipt).unwrap()
        ],
    )
    .unwrap();
    (process, submission, receipt)
}

fn verify_history(
    root: &Path,
    process: &AdmittedProcess,
    submission: &Submission,
    receipt: &SubmissionReceipt,
) {
    let mut runtime = Runtime::open(root).unwrap();
    let run = runtime.start(process, json!({}), "legacy-key").unwrap();
    assert_eq!(run.run_id, "legacy-run-0");
    assert_eq!(run.state, "succeeded");
    assert_eq!(run.revision, 7);
    assert_eq!(
        run.config_digest,
        content_digest(&json!({"mode":"legacy-default"})).unwrap()
    );
    assert_eq!(run.output.unwrap(), json!({"result":{"approved":true}}));
    assert_eq!(
        runtime
            .start(process, json!({}), r#"[null,"legacy-key"]"#)
            .unwrap()
            .run_id,
        "legacy-run-1"
    );
    let events = runtime.events("legacy-run-0", 0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence, 10);
    assert_eq!(runtime.resume(submission).unwrap(), *receipt);
}

#[test]
fn v1_upgrade_preserves_legacy_keys_outputs_events_and_consumed_result_receipts() {
    let directory = tempfile::tempdir().unwrap();
    let (process, submission, receipt) = seed_v1(directory.path());
    verify_history(directory.path(), &process, &submission, &receipt);
    verify_history(directory.path(), &process, &submission, &receipt);
}

#[test]
fn an_expired_persisted_grant_is_rechecked_without_restarting_the_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    let state = directory.path().join("state");
    std::fs::create_dir(&project).unwrap();
    let mut runtime = Runtime::open(&state).unwrap();
    let project = runtime.attach_project(&project).unwrap();
    let assignment = runtime
        .create_assignment(project["project_id"].as_str().unwrap(), "Expiry fixture")
        .unwrap();
    let attachment = runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = attachment["token"].as_str().unwrap();
    runtime.scoped_call(token, "discover", json!({})).unwrap();
    // Controlled persisted-state fixture models elapsed wall time. Assertions
    // stay at the public authorization boundary, never inspect private rows.
    let fixture = Connection::open(state.join("state.sqlite3")).unwrap();
    fixture
        .execute(
            "UPDATE attachments SET expires_at=0 WHERE id=?1",
            [attachment["attachment_id"].as_str().unwrap()],
        )
        .unwrap();
    drop(fixture);
    assert_eq!(
        runtime
            .scoped_call(token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn interrupted_v2_upgrade_rolls_back_and_can_be_retried() {
    use std::os::unix::process::ExitStatusExt;
    let directory = tempfile::tempdir().unwrap();
    let (process, submission, receipt) = seed_v1(directory.path());
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "migration_crash_probe", "--nocapture"])
        .env("APR_MIGRATION_TEST_STATE", directory.path())
        .env("APR_TEST_CRASH_AT", "migration_v2_before_commit")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9));
    verify_history(directory.path(), &process, &submission, &receipt);
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn interrupted_configuration_upgrade_preserves_history_and_frozen_defaults() {
    use std::os::unix::process::ExitStatusExt;
    let directory = tempfile::tempdir().unwrap();
    let (process, submission, receipt) = seed_v1(directory.path());
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "migration_crash_probe", "--nocapture"])
        .env("APR_MIGRATION_TEST_STATE", directory.path())
        .env("APR_TEST_CRASH_AT", "migration_v3_before_commit")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9));
    verify_history(directory.path(), &process, &submission, &receipt);
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn interrupted_foreach_upgrade_preserves_existing_run_history() {
    use std::os::unix::process::ExitStatusExt;
    let directory = tempfile::tempdir().unwrap();
    let (process, submission, receipt) = seed_v1(directory.path());
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "migration_crash_probe", "--nocapture"])
        .env("APR_MIGRATION_TEST_STATE", directory.path())
        .env("APR_TEST_CRASH_AT", "migration_v4_before_commit")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9));
    verify_history(directory.path(), &process, &submission, &receipt);
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn migration_crash_probe() {
    if let Some(root) = std::env::var_os("APR_MIGRATION_TEST_STATE") {
        Runtime::open(Path::new(&root)).unwrap();
        panic!("migration crash point did not fire");
    }
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn interrupted_composition_and_quota_upgrades_preserve_legacy_receipts() {
    use std::os::unix::process::ExitStatusExt;
    for fault in [
        "migration_v7_before_commit",
        "migration_v8_before_commit",
        "migration_v9_before_commit",
        "migration_v10_before_commit",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (process, submission, receipt) = seed_v1(directory.path());
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "migration_crash_probe", "--nocapture"])
            .env("APR_MIGRATION_TEST_STATE", directory.path())
            .env("APR_TEST_CRASH_AT", fault)
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(9), "{fault}");
        verify_history(directory.path(), &process, &submission, &receipt);
    }
}
