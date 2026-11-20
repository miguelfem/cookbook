use std::fs::{self, File, FileTimes};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apr_core::{ArtifactRef, admit_value};
use apr_runtime::artifacts::ArtifactStore;
use apr_runtime::{RunEnvelope, Runtime, Submission};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn process(review: bool) -> Value {
    let mut value = json!({
        "schema_version":"0.1", "name":"maintenance.fixture", "description":"Snapshot and retention fixture",
        "input_schema":{"type":"object"}, "output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"builtin"},{"kind":"artifacts"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[{"id":"save","kind":"builtin","operation":"json.write","needs":[],"effect":"pure",
            "retry":{"max_attempts":2},"inputs":{"value":{"input":""}}}],
        "output":{"artifact":{"step":"save","pointer":"/artifact"}}
    });
    if review {
        value["capabilities"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"agent","capability":"review"}));
        value["limits"]["max_agent_requests"] = json!(1);
        value["steps"].as_array_mut().unwrap().push(json!({
            "id":"review","kind":"agent","needs":["save"],"effect":"agent","retry":{"max_attempts":1},
            "capability":"review","instructions":"Review the saved record.",
            "inputs":{"artifact":{"step":"save","pointer":"/artifact"}},
            "result_schema":{"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false}
        }));
        value["output"]["approved"] = json!({"step":"review","pointer":"/approved"});
    }
    value
}

fn answer(run: &RunEnvelope) -> Submission {
    let request = run.request.as_ref().unwrap();
    Submission {
        protocol_version: "0.1".into(),
        action: "submit".into(),
        run_id: run.run_id.clone(),
        request_id: request.request_id.clone(),
        step_instance_id: request.step_instance_id.clone(),
        generation: request.generation,
        submission_key: "maintenance-result".into(),
        continuation_token: request.continuation_token.clone(),
        output: json!({"approved":true}),
        provenance: json!({"source":"manual","client":"maintenance-test"}),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn age(path: &Path) {
    File::open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3600)))
        .unwrap();
}

fn artifact(run: &RunEnvelope) -> ArtifactRef {
    serde_json::from_value(run.output.as_ref().unwrap()["artifact"].clone()).unwrap()
}

#[test]
fn managed_object_quota_rejects_growth_atomically_allows_verified_dedup_and_recovers_after_retention()
 {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    runtime.set_artifact_quota(Some(5)).unwrap();
    let one = directory.path().join("one.txt");
    let two = directory.path().join("two.txt");
    fs::write(&one, b"first").unwrap();
    fs::write(&two, b"next").unwrap();
    let reference = runtime.import_artifact(&one, "text/plain", 1024).unwrap();
    assert_eq!(runtime.storage_usage().unwrap().quota_used_bytes, 5);
    assert_eq!(
        runtime
            .import_artifact(&two, "text/plain", 1024)
            .unwrap_err()
            .code(),
        "LIMIT_EXCEEDED"
    );
    assert_eq!(
        fs::read_dir(state.join("objects/sha256")).unwrap().count(),
        1
    );
    runtime.set_artifact_quota(Some(1)).unwrap();
    assert_eq!(
        runtime.import_artifact(&one, "text/plain", 1024).unwrap(),
        reference
    );
    assert_eq!(runtime.read_artifact(&reference, 1024).unwrap(), b"first");
    let usage = runtime.storage_usage().unwrap();
    assert_eq!(usage.quota_mode, "managed_artifacts");
    assert_eq!(usage.quota_bytes, Some(1));
    assert_eq!(usage.quota_used_bytes, 5);
    let object = state.join("objects/sha256").join(&reference.sha256);
    fs::write(&object, b"wrong").unwrap();
    assert!(
        runtime.import_artifact(&one, "text/plain", 1024).is_err(),
        "dedup must verify even while over quota"
    );
    assert_eq!(
        fs::read(&object).unwrap(),
        b"wrong",
        "corrupt cached content must not be replaced"
    );
    fs::write(&object, b"first").unwrap();
    age(&object);
    let plan = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    runtime.retention_apply(&plan).unwrap();
    assert_eq!(runtime.storage_usage().unwrap().quota_used_bytes, 0);
    runtime.set_artifact_quota(Some(5)).unwrap();
    assert_eq!(
        runtime
            .import_artifact(&two, "text/plain", 1024)
            .unwrap()
            .bytes,
        4
    );
    runtime.set_artifact_quota(None).unwrap();
    assert_eq!(runtime.storage_usage().unwrap().quota_mode, "unlimited");
}

#[test]
fn concurrent_runtime_imports_share_the_current_quota_without_overcommitting() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    runtime.set_artifact_quota(Some(4)).unwrap();
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    fs::write(&first, b"aaaa").unwrap();
    fs::write(&second, b"bbbb").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = [first, second]
        .into_iter()
        .map(|source| {
            let barrier = barrier.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                let mut runtime = Runtime::open(state).unwrap();
                barrier.wait();
                runtime.import_artifact(&source, "text/plain", 1024)
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .next()
            .unwrap()
            .code(),
        "LIMIT_EXCEEDED"
    );
    assert_eq!(runtime.storage_usage().unwrap().quota_used_bytes, 4);
    assert_eq!(
        fs::read_dir(state.join("objects/sha256")).unwrap().count(),
        1
    );
}

#[test]
fn builtin_publications_use_the_managed_quota_too() {
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(directory.path()).unwrap();
    runtime.set_artifact_quota(Some(0)).unwrap();
    let run = runtime
        .start(
            &admit_value(&process(false)).unwrap(),
            json!({"record":"blocked by quota"}),
            "quota",
        )
        .unwrap();
    assert_eq!(run.state, "failed");
    assert_eq!(runtime.storage_usage().unwrap().quota_used_bytes, 0);
    assert_eq!(
        fs::read_dir(directory.path().join("objects/sha256"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn storage_diagnostics_preserve_other_sqlite_connections_locks_and_attachment_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut first = Runtime::open(&state).unwrap();
    let (_, token, version) = scoped_project(
        &mut first,
        &directory.path().join("project"),
        &process(false),
    );
    let queued=first.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"saved":true},"request_key":"locks"})).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    let done = first.advance(id).unwrap();
    let reference = artifact(&done);
    let mut second = Runtime::open(&state).unwrap();
    let mut database = rusqlite::Connection::open(state.join("state.sqlite3")).unwrap();
    let transaction = database
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute("UPDATE attachments SET revoked_at=1", [])
        .unwrap();
    first.storage_usage().unwrap();
    second.storage_usage().unwrap();
    let probe = std::process::Command::new(python())
        .args([
            "-I",
            "-c",
            r#"import sqlite3,sys
db=sqlite3.connect(sys.argv[1],timeout=0.02)
try:
    db.execute('BEGIN IMMEDIATE')
except sqlite3.OperationalError:
    print('locked')
else:
    db.rollback()
    print('lock-lost')
"#,
        ])
        .arg(state.join("state.sqlite3"))
        .output()
        .unwrap();
    assert!(probe.status.success());
    assert_eq!(
        probe.stdout, b"locked\n",
        "diagnostics released another connection's SQLite advisory lock"
    );
    transaction.commit().unwrap();
    assert_eq!(
        first
            .scoped_call(&token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    assert_eq!(
        second
            .scoped_call(&token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    assert_eq!(
        first.read_artifact(&reference, 1024).unwrap(),
        second.read_artifact(&reference, 1024).unwrap()
    );
}

fn scoped_project(
    runtime: &mut Runtime,
    project: &Path,
    manifest: &Value,
) -> (String, String, String) {
    fs::create_dir_all(project).unwrap();
    fs::write(
        project.join("process.json"),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
    let project_id = runtime.attach_project(project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let assignment = runtime
        .create_assignment(&project_id, "Maintenance fixture")
        .unwrap();
    let assignment_id = assignment["assignment_id"].as_str().unwrap().to_owned();
    let token = runtime.create_attachment(&assignment_id, "author").unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let version = runtime
        .scoped_call(
            &token,
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )
        .unwrap()["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    (assignment_id, token, version)
}

#[test]
fn restored_queued_work_waits_for_explicit_continuation() {
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (_, token, version) = scoped_project(
        &mut runtime,
        &directory.path().join("project"),
        &process(false),
    );
    let queued = runtime.scoped_call(&token, "run", json!({"mode":"start","process_ref":version,"input":{"record":"queued"},"request_key":"queued"})).unwrap();
    let run_id = queued["run_id"].as_str().unwrap();
    let backup = directory.path().join("backup");
    let state = directory.path().join("restored");
    runtime.backup_to(&backup).unwrap();
    Runtime::restore_from(&backup, &state).unwrap();
    let mut restored = Runtime::open(&state).unwrap();
    restored.drive_pending().unwrap();
    let blocked = restored.inspect(run_id).unwrap();
    assert_eq!(blocked.state, "blocked");
    assert_eq!(blocked.completed_steps, 0);
    assert_eq!(
        blocked.error.as_deref(),
        Some("restored work requires explicit continuation")
    );
    assert_eq!(restored.advance(run_id).unwrap().state, "succeeded");
    assert_eq!(runtime.inspect(run_id).unwrap().state, "queued");
}

#[test]
fn snapshot_preserves_foreach_progress_and_retention_removes_its_expansion() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest = process(false);
    manifest["steps"][0]["foreach"] =
        json!({"items":{"input":"/items"},"key":"/id","max_items":3,"concurrency":1});
    manifest["steps"][0]["inputs"]["value"] = json!({"item":""});
    manifest["output"] = json!({"items":{"step":"save","pointer":"/items"}});
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (_, token, version) =
        scoped_project(&mut runtime, &directory.path().join("project"), &manifest);
    let queued = runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"second","value":2},{"id":"first","value":1}]},"request_key":"items"})).unwrap();
    let run_id = queued["run_id"].as_str().unwrap();
    let mut cursor = None;
    for _ in 0..5 {
        runtime.drive_next_pending(&mut cursor).unwrap();
        if runtime.inspect(run_id).unwrap().completed_steps > 0 {
            break;
        }
    }
    let partial = runtime.inspect(run_id).unwrap();
    assert_eq!(partial.completed_steps, 1);
    let saved_step = partial
        .steps
        .iter()
        .find(|step| step.state == "succeeded")
        .unwrap()
        .id
        .clone();
    let backup = directory.path().join("backup");
    let state = directory.path().join("restored");
    runtime.backup_to(&backup).unwrap();
    Runtime::restore_from(&backup, &state).unwrap();
    let mut restored = Runtime::open(&state).unwrap();
    let done = restored.advance(run_id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(
        done.steps
            .iter()
            .find(|step| step.id == saved_step)
            .unwrap()
            .attempts,
        1
    );
    assert_eq!(
        restored
            .events(run_id, 0, 100)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "foreach.expanded")
            .count(),
        1
    );
    let items = done.output.as_ref().unwrap()["items"].as_array().unwrap();
    for (item, expected) in items.iter().zip(["second", "first"]) {
        let reference: ArtifactRef =
            serde_json::from_value(item["output"]["artifact"].clone()).unwrap();
        let value: Value =
            serde_json::from_slice(&restored.read_artifact(&reference, 1024).unwrap()).unwrap();
        assert_eq!(value["id"], expected);
        age(&state.join("objects/sha256").join(reference.sha256));
    }
    std::thread::sleep(Duration::from_millis(2));
    let plan = restored
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert_eq!(plan.run_ids, vec![run_id.to_owned()]);
    assert_eq!(plan.artifact_ids.len(), 2);
    restored.retention_apply(&plan).unwrap();
    assert_eq!(restored.inspect(run_id).unwrap_err().code(), "NOT_FOUND");
}

#[test]
fn snapshot_preserves_completed_work_and_requires_new_authority_after_restore() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let backup = directory.path().join("backup");
    let restored = directory.path().join("restored");
    let mut runtime = Runtime::open(&state).unwrap();
    let run = runtime
        .start(
            &admit_value(&process(true)).unwrap(),
            json!({"title":"Private source record"}),
            "saved-review",
        )
        .unwrap();
    let old = answer(&run);
    let reference: ArtifactRef =
        serde_json::from_value(run.request.as_ref().unwrap().input["artifact"].clone()).unwrap();
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let project_id = runtime.attach_project(&project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let assignment = runtime
        .create_assignment(&project_id, "Owner restore fixture")
        .unwrap();
    let assignment_id = assignment["assignment_id"].as_str().unwrap();
    let attachment = runtime.create_attachment(assignment_id, "author").unwrap();
    let report = runtime.backup_to(&backup).unwrap();
    assert_eq!(report.runs, 1);
    assert_eq!(report.handoffs_requiring_renewal, 1);
    assert_eq!(report.artifacts, 1);
    assert_eq!(
        runtime.inspect(&run.run_id).unwrap(),
        run,
        "snapshot must not revoke source authority"
    );
    assert!(!backup.join("handoffs").exists());
    assert!(!backup.join("attachments").exists());
    assert_eq!(
        fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let database = fs::read(backup.join("state.sqlite3")).unwrap();
    for token in [
        &old.continuation_token,
        attachment["token"].as_str().unwrap(),
    ] {
        assert!(
            !database
                .windows(token.len())
                .any(|bytes| bytes == token.as_bytes())
        );
    }
    Runtime::restore_from(&backup, &restored).unwrap();
    let mut recovered = Runtime::open(&restored).unwrap();
    let waiting = recovered.inspect(&run.run_id).unwrap();
    assert_eq!(waiting.state, "waiting_for_agent");
    assert_eq!(waiting.completed_steps, 1);
    assert!(waiting.request.is_none());
    assert_eq!(
        waiting.error.as_deref(),
        Some("caller handoff must be renewed after restore")
    );
    assert_eq!(
        recovered.read_artifact(&reference, 1024).unwrap(),
        runtime.read_artifact(&reference, 1024).unwrap()
    );
    assert_eq!(
        recovered
            .scoped_call(attachment["token"].as_str().unwrap(), "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    assert_eq!(recovered.resume(&old).unwrap_err().code(), "STALE_HANDOFF");
    let renewed = recovered.renew_handoff(&run.run_id).unwrap();
    assert_eq!(
        renewed.request.as_ref().unwrap().generation,
        old.generation + 1
    );
    recovered.resume(&answer(&renewed)).unwrap();
    let done = recovered.inspect(&run.run_id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(done.steps[0].attempts, 1);
    assert_eq!(done.output.as_ref().unwrap()["approved"], true);
    assert_eq!(runtime.inspect(&run.run_id).unwrap(), run);
}

#[test]
fn retention_preserves_whole_active_and_pinned_child_trees_then_removes_them_together() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let mut child_value = process(false);
    child_value["name"] = json!("maintenance.child");
    let child = admit_value(&child_value).unwrap();
    let mut parent_value = process(true);
    parent_value["schema_version"] = json!("0.2");
    parent_value["name"] = json!("maintenance.parent");
    parent_value["steps"][0] = json!({
        "id":"save", "kind":"call", "effect":"control", "needs":[],
        "retry":{"max_attempts":1}, "inputs":{"record":{"input":"/record"}},
        "call":{"process_ref":child.version,"output_pointer":""}
    });
    let parent = admit_value(&parent_value).unwrap();
    fs::write(project.join("child.json"), &child.canonical_bytes).unwrap();
    fs::write(project.join("parent.json"), &parent.canonical_bytes).unwrap();
    let mut runtime = Runtime::open(&state).unwrap();
    let trusted = runtime.attach_project(&project).unwrap();
    let assignment = runtime
        .create_assignment(trusted["project_id"].as_str().unwrap(), "Retention tree")
        .unwrap();
    let attachment = runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = attachment["token"].as_str().unwrap();
    for path in ["child.json", "parent.json"] {
        runtime
            .scoped_call(
                token,
                "manage",
                json!({"action":"process.register","path":path}),
            )
            .unwrap();
    }
    let queued = runtime.scoped_call(token, "run", json!({"mode":"start","process_ref":parent.version,"input":{"record":"retained child"},"request_key":"tree"})).unwrap();
    let root_id = queued["run_id"].as_str().unwrap();
    for _ in 0..16 {
        runtime.drive_pending().unwrap();
        if runtime.inspect(root_id).unwrap().state == "waiting_for_agent" {
            break;
        }
    }
    let waiting = runtime.inspect(root_id).unwrap();
    assert_eq!(waiting.state, "waiting_for_agent", "{:?}", waiting.error);
    let child_id = waiting
        .steps
        .iter()
        .find_map(|step| step.child_run_id.clone())
        .unwrap();
    let completed_child = runtime.inspect(&child_id).unwrap();
    assert_eq!(completed_child.state, "succeeded");
    let reference = artifact(&completed_child);
    age(&state.join("objects/sha256").join(&reference.sha256));
    std::thread::sleep(Duration::from_millis(2));
    let active = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert!(active.run_ids.is_empty());
    assert!(active.artifact_ids.is_empty());
    runtime.resume(&answer(&waiting)).unwrap();
    for _ in 0..8 {
        runtime.drive_pending().unwrap();
    }
    assert_eq!(runtime.inspect(root_id).unwrap().state, "succeeded");
    runtime.pin_run(&child_id, true).unwrap();
    std::thread::sleep(Duration::from_millis(2));
    let pinned = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert!(pinned.run_ids.is_empty());
    runtime.pin_run(&child_id, false).unwrap();
    let removable = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    let mut expected = vec![root_id.to_owned(), child_id.clone()];
    expected.sort();
    assert_eq!(removable.run_ids, expected);
    runtime.retention_apply(&removable).unwrap();
    assert_eq!(runtime.inspect(root_id).unwrap_err().code(), "NOT_FOUND");
    assert_eq!(runtime.inspect(&child_id).unwrap_err().code(), "NOT_FOUND");
    assert!(runtime.read_artifact(&reference, 1024).is_err());
}

#[test]
fn corrupted_backups_and_existing_destinations_never_publish_partial_restore() {
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let run = runtime
        .start(
            &admit_value(&process(false)).unwrap(),
            json!({"record":"immutable"}),
            "one",
        )
        .unwrap();
    let reference = artifact(&run);
    let backup = directory.path().join("backup");
    runtime.backup_to(&backup).unwrap();
    let destination = directory.path().join("restored");
    let object = backup.join("objects/sha256").join(&reference.sha256);
    let original = fs::read(&object).unwrap();
    fs::write(&object, b"corrupted").unwrap();
    assert!(Runtime::restore_from(&backup, &destination).is_err());
    assert!(!destination.exists());
    fs::write(&object, original).unwrap();
    let db = backup.join("state.sqlite3");
    let original = fs::read(&db).unwrap();
    fs::write(&db, b"damaged database").unwrap();
    assert!(Runtime::restore_from(&backup, &destination).is_err());
    assert!(!destination.exists());
    fs::write(&db, original).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("user.txt"), "preserved").unwrap();
    assert!(Runtime::restore_from(&backup, &destination).is_err());
    assert_eq!(
        fs::read_to_string(destination.join("user.txt")).unwrap(),
        "preserved"
    );
    let link = directory.path().join("linked-backup");
    symlink(&backup, &link).unwrap();
    assert!(Runtime::restore_from(&link, &directory.path().join("another")).is_err());
    let target = directory.path().join("linked-target");
    symlink(&destination, &target).unwrap();
    assert!(runtime.backup_to(&target).is_err());
    assert!(
        fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!fs::read_dir(directory.path()).unwrap().any(|item| {
        item.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".apr-snapshot-")
    }));
}

#[test]
fn retention_preserves_waiting_work_pins_and_new_or_unknown_objects() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let pure = admit_value(&process(false)).unwrap();
    let removed = runtime
        .start(&pure, json!({"record":"remove"}), "remove")
        .unwrap();
    let pinned = runtime
        .start(&pure, json!({"record":"pin"}), "pin")
        .unwrap();
    runtime.pin_run(&pinned.run_id, true).unwrap();
    let waiting = runtime
        .start(
            &admit_value(&process(true)).unwrap(),
            json!({"record":"waiting"}),
            "waiting",
        )
        .unwrap();
    let objects = ArtifactStore::open(&state).unwrap();
    let orphan = objects
        .put(b"orphan from interrupted publication", "text/plain", 1024)
        .unwrap();
    let fresh = objects
        .put(b"new uncommitted object", "text/plain", 1024)
        .unwrap();
    let object_dir = state.join("objects/sha256");
    for reference in [artifact(&removed), artifact(&pinned), orphan.clone()] {
        age(&object_dir.join(&reference.sha256));
    }
    fs::write(object_dir.join("user-note.txt"), "do not delete").unwrap();
    std::thread::sleep(Duration::from_millis(2));
    let plan = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert_eq!(plan.run_ids, vec![removed.run_id.clone()]);
    assert_eq!(plan.artifact_ids, vec![artifact(&removed).artifact_id]);
    assert_eq!(plan.orphan_object_names, vec![orphan.sha256.clone()]);
    let roundtrip = serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    let report = runtime.retention_apply(&roundtrip).unwrap();
    assert_eq!(report.removed_runs, 1);
    assert_eq!(report.removed_artifacts, 1);
    assert_eq!(report.removed_objects, 2);
    assert_eq!(
        runtime.inspect(&removed.run_id).unwrap_err().code(),
        "NOT_FOUND"
    );
    assert_eq!(runtime.inspect(&pinned.run_id).unwrap(), pinned);
    assert_eq!(runtime.inspect(&waiting.run_id).unwrap(), waiting);
    assert!(!object_dir.join(orphan.sha256).exists());
    assert!(object_dir.join(fresh.sha256).exists());
    assert_eq!(
        fs::read_to_string(object_dir.join("user-note.txt")).unwrap(),
        "do not delete"
    );
}

#[test]
fn retention_rechecks_pins_and_preserves_dependencies_of_pinned_json_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let source = directory.path().join("source.txt");
    fs::write(&source, "leaf payload").unwrap();
    let leaf = runtime
        .import_artifact(&source, "text/plain", 1024)
        .unwrap();
    let manifest = directory.path().join("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({"artifacts":[leaf]})).unwrap(),
    )
    .unwrap();
    let parent = runtime
        .import_artifact(&manifest, "application/json", 1024)
        .unwrap();
    for reference in [&leaf, &parent] {
        age(&state.join("objects/sha256").join(&reference.sha256));
    }
    let plan = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert_eq!(plan.artifact_ids.len(), 2);
    runtime.pin_artifact(&parent.artifact_id, true).unwrap();
    assert_eq!(
        runtime.retention_apply(&plan).unwrap_err().code(),
        "INVALID_STATE"
    );
    let protected = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert!(protected.artifact_ids.is_empty());
    runtime.retention_apply(&protected).unwrap();
    assert_eq!(runtime.read_artifact(&leaf, 1024).unwrap(), b"leaf payload");
    runtime.pin_artifact(&parent.artifact_id, false).unwrap();
    let plan = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    let mut changed = plan.clone();
    changed.bytes_reclaimable += 1;
    assert!(runtime.retention_apply(&changed).is_err());
    runtime.retention_apply(&plan).unwrap();
    assert!(runtime.read_artifact(&leaf, 1024).is_err());
}

#[test]
fn storage_report_contains_only_bounded_metadata_and_honestly_reports_no_total_quota() {
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(directory.path()).unwrap();
    runtime
        .start(
            &admit_value(&process(false)).unwrap(),
            json!({"private":"not in diagnostics"}),
            "secret-request-key",
        )
        .unwrap();
    let usage = runtime.storage_usage().unwrap();
    assert_eq!(usage.runs, 1);
    assert_eq!(usage.registered_artifacts, 1);
    assert_eq!(usage.orphan_object_bytes, 0);
    assert!(usage.registered_artifact_bytes > 0);
    assert_eq!(usage.registered_artifact_bytes, usage.stored_object_bytes);
    assert_eq!(usage.quota_mode, "unlimited");
    assert!(usage.quota_bytes.is_none());
    let encoded = serde_json::to_string(&usage).unwrap();
    assert!(encoded.len() < 1024);
    for forbidden in [
        "not in diagnostics",
        "secret-request-key",
        &directory.path().display().to_string(),
    ] {
        assert!(!encoded.contains(forbidden));
    }
}

#[test]
fn retention_remains_bound_to_the_open_object_directory_after_path_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let objects = ArtifactStore::open(&state).unwrap();
    let orphan = objects.put(b"owned orphan", "text/plain", 1024).unwrap();
    let path = state.join("objects/sha256");
    age(&path.join(&orphan.sha256));
    let original = state.join("objects/original");
    fs::rename(&path, &original).unwrap();
    let external = directory.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join(&orphan.sha256), "unrelated user file").unwrap();
    symlink(&external, &path).unwrap();
    let plan = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert_eq!(plan.orphan_object_names, vec![orphan.sha256.clone()]);
    runtime.retention_apply(&plan).unwrap();
    assert!(!original.join(&orphan.sha256).exists());
    assert_eq!(
        fs::read_to_string(external.join(&orphan.sha256)).unwrap(),
        "unrelated user file"
    );
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn retention_rejects_object_changes_between_preview_and_apply() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let orphan = ArtifactStore::open(&state)
        .unwrap()
        .put(b"unlinked", "text/plain", 1024)
        .unwrap();
    let path = state.join("objects/sha256").join(&orphan.sha256);
    age(&path);
    let plan = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    fs::write(&path, "changed after review").unwrap();
    assert!(runtime.retention_apply(&plan).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "changed after review");
}

#[test]
fn restored_project_can_move_without_reusing_old_attachments_or_rewriting_saved_work() {
    let directory = tempfile::tempdir().unwrap();
    let old_project = directory.path().join("old-project");
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (assignment, token, version) = scoped_project(&mut runtime, &old_project, &process(true));
    let project_id = runtime.attach_project(&old_project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let queued = runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"unchanged_path":"/source/device/input.txt"},"request_key":"move"})).unwrap();
    for _ in 0..3 {
        runtime.drive_pending().unwrap();
    }
    let id = queued["run_id"].as_str().unwrap();
    assert_eq!(runtime.inspect(id).unwrap().state, "waiting_for_agent");
    let backup = directory.path().join("backup");
    let restored_path = directory.path().join("restored");
    runtime.backup_to(&backup).unwrap();
    Runtime::restore_from(&backup, &restored_path).unwrap();
    let new_project = directory.path().join("new-project");
    fs::rename(&old_project, &new_project).unwrap();
    let mut restored = Runtime::open(&restored_path).unwrap();
    let result = restored
        .relocate_project(&project_id, &new_project)
        .unwrap();
    assert_eq!(
        result["root"],
        new_project.canonicalize().unwrap().to_str().unwrap()
    );
    let waiting = restored.inspect(id).unwrap();
    assert_eq!(waiting.state, "waiting_for_agent");
    assert!(waiting.request.is_none());
    assert_eq!(
        restored
            .scoped_call(&token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    let new_token = restored.create_attachment(&assignment, "author").unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let discovery = restored
        .scoped_call(&new_token, "discover", json!({}))
        .unwrap();
    assert_eq!(discovery["processes"].as_array().unwrap().len(), 1);
    let renewed = restored.renew_handoff(id).unwrap();
    let reference: ArtifactRef =
        serde_json::from_value(renewed.request.as_ref().unwrap().input["artifact"].clone())
            .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&restored.read_artifact(&reference, 1024).unwrap())
            .unwrap()["unchanged_path"],
        "/source/device/input.txt"
    );
    restored.resume(&answer(&renewed)).unwrap();
    assert_eq!(restored.inspect(id).unwrap().state, "succeeded");
    let conflicting = directory.path().join("another-project");
    fs::create_dir(&conflicting).unwrap();
    restored.attach_project(&conflicting).unwrap();
    assert!(
        restored
            .relocate_project(&project_id, &conflicting)
            .is_err()
    );
    assert!(
        restored
            .scoped_call(&new_token, "discover", json!({}))
            .is_ok()
    );
    restored
        .relocate_project(&project_id, &new_project)
        .unwrap();
    assert_eq!(
        restored
            .scoped_call(&new_token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
}

fn python() -> PathBuf {
    std::env::var_os("APR_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            [
                "/Library/Frameworks/Python.framework/Versions/3.11/bin/python3",
                "/usr/bin/python3",
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect("maintenance native tests require an existing Python 3; use APR_TEST_PYTHON")
        })
}

fn native_process(code: &[u8]) -> Value {
    json!({
        "schema_version":"0.1","name":"maintenance.native","description":"Frozen native maintenance fixture",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"script","language":"python"}],
        "limits":{"max_step_ms":2000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "files":[{"path":"main.py","sha256":format!("{:x}",Sha256::digest(code))}],
        "steps":[{"id":"native","kind":"script","entrypoint":"main.py","effect":"write","needs":[],
            "retry":{"max_attempts":1},"inputs":{"payload":{"input":""}},"result_schema":{"type":"object"}}],
        "output":{"result":{"step":"native","pointer":""}}
    })
}

#[test]
fn restored_native_work_requires_owner_rebinding_and_uses_the_frozen_source() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let code = b"import json,sys\nr=json.load(sys.stdin)\njson.dump({'protocol_version':'0.1','output':r['input']['payload']},sys.stdout)\n";
    fs::write(project.join("main.py"), code).unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (assignment, token, version) =
        scoped_project(&mut runtime, &project, &native_process(code));
    runtime.grant_python(&assignment, &python()).unwrap();
    let queued = runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"frozen":true},"request_key":"native"})).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    let backup = directory.path().join("backup");
    let restored_path = directory.path().join("restored");
    runtime.backup_to(&backup).unwrap();
    Runtime::restore_from(&backup, &restored_path).unwrap();
    fs::write(
        project.join("main.py"),
        "raise RuntimeError('edited source must never execute')",
    )
    .unwrap();
    let mut restored = Runtime::open(&restored_path).unwrap();
    let token = restored.create_attachment(&assignment, "author").unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        restored
            .scoped_call(
                &token,
                "run",
                json!({"mode":"start","process_ref":version,"input":{},"request_key":"denied"})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    assert_eq!(restored.advance(id).unwrap().state, "blocked");
    restored.grant_python(&assignment, &python()).unwrap();
    assert_eq!(
        restored.advance(id).unwrap().state,
        "blocked",
        "new grants must not silently replace frozen run bindings"
    );
    restored.rebind_execution(id).unwrap();
    let done = restored.advance(id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(done.output.unwrap()["result"], json!({"frozen":true}));
    assert_eq!(runtime.inspect(id).unwrap().state, "queued");
}

#[test]
fn uncertain_native_effect_stays_stopped_after_restore_and_observation_finishes_without_replay() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    fs::create_dir(&project).unwrap();
    let code = b"import json,sys\nr=json.load(sys.stdin)\nwith open(r['input']['payload']['path'],'a') as f: f.write('effect\\n')\nraise RuntimeError('private native diagnostic')\n";
    fs::write(project.join("main.py"), code).unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (assignment, token, version) =
        scoped_project(&mut runtime, &project, &native_process(code));
    runtime.grant_python(&assignment, &python()).unwrap();
    let marker = directory.path().join("effect.txt");
    let queued = runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"path":marker},"request_key":"effect"})).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    runtime.drive_pending().unwrap();
    assert_eq!(runtime.inspect(id).unwrap().state, "needs_reconciliation");
    assert_eq!(fs::read_to_string(&marker).unwrap(), "effect\n");
    let backup = directory.path().join("backup");
    let restored_path = directory.path().join("restored");
    let report = runtime.backup_to(&backup).unwrap();
    assert_eq!(report.runs_requiring_reconciliation, 1);
    Runtime::restore_from(&backup, &restored_path).unwrap();
    let mut restored = Runtime::open(&restored_path).unwrap();
    restored.drive_pending().unwrap();
    assert_eq!(restored.advance(id).unwrap().state, "needs_reconciliation");
    assert!(restored.rebind_execution(id).is_err());
    assert_eq!(fs::read_to_string(&marker).unwrap(), "effect\n");
    let token = restored.create_attachment(&assignment, "author").unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let effects = restored
        .scoped_call(&token, "inspect", json!({"view":"effects","run_id":id}))
        .unwrap();
    assert_eq!(effects["intents"].as_array().unwrap().len(), 1);
    assert_eq!(effects["intents"][0]["status"], "uncertain");
    restored.scoped_call(&token,"resume",json!({"action":"reconcile","run_id":id,"step_instance_id":"native","intent_id":effects["intents"][0]["intent_id"],"outcome":"observed_result","evidence":"Read the unique marker once through the controlled fixture","output":{"saved":true}})).unwrap();
    assert_eq!(restored.advance(id).unwrap().state, "succeeded");
    assert_eq!(fs::read_to_string(&marker).unwrap(), "effect\n");
}

#[cfg(feature = "fault-injection")]
#[test]
fn retention_fault_fixture() {
    let Some(state) = std::env::var_os("APR_RETENTION_FIXTURE_STATE") else {
        return;
    };
    let mut runtime = Runtime::open(state).unwrap();
    let plan: apr_runtime::maintenance::RetentionPlan = serde_json::from_slice(
        &fs::read(std::env::var_os("APR_RETENTION_FIXTURE_PLAN").unwrap()).unwrap(),
    )
    .unwrap();
    runtime.retention_apply(&plan).unwrap();
    panic!("retention fault did not terminate the child");
}

#[cfg(feature = "fault-injection")]
#[test]
fn killed_retention_leaves_only_safe_collectible_orphans_after_database_commit() {
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let run = runtime
        .start(
            &admit_value(&process(false)).unwrap(),
            json!({"remove":"after committed references"}),
            "crash",
        )
        .unwrap();
    let reference = artifact(&run);
    let object = state.join("objects/sha256").join(&reference.sha256);
    age(&object);
    std::thread::sleep(Duration::from_millis(2));
    let plan = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    let plan_path = directory.path().join("plan.json");
    fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    drop(runtime);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "retention_fault_fixture", "--nocapture"])
        .env("APR_RETENTION_FIXTURE_STATE", &state)
        .env("APR_RETENTION_FIXTURE_PLAN", &plan_path)
        .env("APR_TEST_CRASH_AT", "retention_committed")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("owned retention fixture did not terminate");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    let mut runtime = Runtime::open(&state).unwrap();
    assert_eq!(
        runtime.inspect(&run.run_id).unwrap_err().code(),
        "NOT_FOUND"
    );
    assert!(object.exists(), "kill must occur before object unlink");
    let recovery = runtime
        .retention_preview(now_ms(), now_ms() - 60_001)
        .unwrap();
    assert!(recovery.artifact_ids.is_empty());
    assert_eq!(recovery.orphan_object_names, vec![reference.sha256]);
    runtime.retention_apply(&recovery).unwrap();
    assert!(!object.exists());
}
