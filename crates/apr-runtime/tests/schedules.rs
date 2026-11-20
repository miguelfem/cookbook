use apr_runtime::Runtime;
use rusqlite::Connection;
use serde_json::{Value, json};

fn setup(runtime: &mut Runtime, project: &std::path::Path, agent: bool) -> (Value, String, String) {
    std::fs::create_dir_all(project).unwrap();
    let project_id = runtime.attach_project(project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let assignment = runtime
        .create_assignment(&project_id, "Scheduled fixture")
        .unwrap();
    let attachment = runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let step = if agent {
        json!({"id":"work","kind":"agent","needs":[],"effect":"agent","retry":{"max_attempts":1},"capability":"review","instructions":"Review this scheduled item", "inputs":{},"result_schema":{"type":"object"}})
    } else {
        json!({"id":"work","kind":"builtin","operation":"json.write","needs":[],"effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"config":""}}})
    };
    let process = json!({"schema_version":"0.1","name":"schedule.fixture","description":"Scheduled work fixture",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","required":["locale"],"additionalProperties":false,"properties":{"locale":{"type":"string"}}},"defaults":{"locale":"en"},"run_override_keys":["locale"]},
        "capabilities":[{"kind":"builtin"},{"kind":"artifacts"},{"kind":"agent"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[step],"output":{"settings":{"config":""},"result":{"step":"work","pointer":""}}});
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&process).unwrap(),
    )
    .unwrap();
    let version = runtime
        .scoped_call(
            attachment["token"].as_str().unwrap(),
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )
        .unwrap()["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    (attachment, version, project_id)
}
fn create(runtime: &mut Runtime, token: &str, version: &str, key: &str, trigger: Value) -> Value {
    runtime.scoped_call(token,"manage",json!({"action":"schedule.create","name":"Recurring review","process_ref":version,"input":{},"request_key":key,"trigger":trigger})).unwrap()
}
fn inspect(runtime: &mut Runtime, token: &str, schedule: &Value) -> Value {
    runtime
        .scoped_call(
            token,
            "inspect",
            json!({"view":"schedule","schedule_id":schedule["schedule_id"]}),
        )
        .unwrap()
}

#[test]
fn once_is_atomic_idempotent_frozen_and_survives_restart_and_run_retention() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let (attachment, version, project) = setup(&mut runtime, &dir.path().join("project"), false);
    let token = attachment["token"].as_str().unwrap();
    runtime
        .set_project_config(&project, &version, json!({"locale":"es"}))
        .unwrap();
    let request = json!({"action":"schedule.create","name":"Saved settings","process_ref":version,"input":{"x":1},"request_key":"once","trigger":{"kind":"once","at_ms":1000}});
    let schedule = runtime
        .scoped_call(token, "manage", request.clone())
        .unwrap();
    runtime
        .set_project_config(&project, &version, json!({"locale":"fr"}))
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(token, "manage", request.clone())
            .unwrap()["schedule_id"],
        schedule["schedule_id"]
    );
    let mut conflict = request.clone();
    conflict["input"] = json!({"x":2});
    assert_eq!(
        runtime
            .scoped_call(token, "manage", conflict)
            .unwrap_err()
            .code(),
        "REQUEST_CONFLICT"
    );
    assert_eq!(runtime.tick_schedules(999).unwrap().started, 0);
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 1);
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 0);
    let run = inspect(&mut runtime, token, &schedule)["last_occurrence"]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(runtime);
    let mut runtime = Runtime::open(&state).unwrap();
    assert_eq!(runtime.tick_schedules(10_000).unwrap().started, 0);
    runtime.drive_pending().unwrap();
    assert_eq!(
        runtime.inspect(&run).unwrap().output.unwrap()["settings"],
        json!({"locale":"es"})
    );
    assert_eq!(
        inspect(&mut runtime, token, &schedule)["state"],
        "completed"
    );
    // Retention may remove the terminal run without erasing firing identity.
    let fixture = Connection::open(state.join("state.sqlite3")).unwrap();
    fixture
        .execute("UPDATE events SET created_at=1", [])
        .unwrap();
    fixture.execute("UPDATE runs SET created_at=1", []).unwrap();
    drop(fixture);
    let plan = runtime.retention_preview(10_000, 0).unwrap();
    assert!(plan.run_ids.contains(&run));
    runtime.retention_apply(&plan).unwrap();
    let retained = inspect(&mut runtime, token, &schedule);
    assert_eq!(retained["last_occurrence"]["run_id"], run);
    assert_eq!(runtime.tick_schedules(20_000).unwrap().started, 0);
    assert_eq!(
        runtime.scoped_call(token, "manage", request).unwrap()["state"],
        "completed"
    );
}

#[test]
fn interval_coalesces_missed_ticks_skips_overlap_and_obeys_controls() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), true);
    let token = attachment["token"].as_str().unwrap();
    let schedule = create(
        &mut runtime,
        token,
        &version,
        "interval",
        json!({"kind":"interval","every_seconds":60,"start_at_ms":1000}),
    );
    assert_eq!(runtime.tick_schedules(181_999).unwrap().started, 1);
    let current = inspect(&mut runtime, token, &schedule);
    assert_eq!(current["next_due_ms"], 241_000);
    assert_eq!(current["history"].as_array().unwrap().len(), 1);
    let run = current["last_occurrence"]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    runtime.drive_pending().unwrap();
    assert_eq!(runtime.inspect(&run).unwrap().state, "waiting_for_agent");
    assert_eq!(runtime.tick_schedules(500_000).unwrap().skipped_overlap, 1);
    assert_eq!(
        inspect(&mut runtime, token, &schedule)["next_due_ms"],
        541_000
    );
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"schedule.pause","schedule_id":schedule["schedule_id"]}),
        )
        .unwrap();
    assert_eq!(runtime.tick_schedules(1_000_000).unwrap().started, 0);
    assert_eq!(runtime.inspect(&run).unwrap().state, "waiting_for_agent");
    runtime
        .scoped_call(token, "resume", json!({"action":"cancel","run_id":run}))
        .unwrap();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"schedule.resume","schedule_id":schedule["schedule_id"]}),
        )
        .unwrap();
    assert_eq!(runtime.tick_schedules(1_000_000).unwrap().started, 1);
    let accepted = inspect(&mut runtime, token, &schedule)["last_occurrence"]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"schedule.delete","schedule_id":schedule["schedule_id"]}),
        )
        .unwrap();
    assert_eq!(runtime.inspect(&accepted).unwrap().state, "queued");
    assert_eq!(runtime.tick_schedules(2_000_000).unwrap().started, 0);
    assert_eq!(
        create(
            &mut runtime,
            token,
            &version,
            "interval",
            json!({"kind":"interval","every_seconds":60,"start_at_ms":1000})
        )["state"],
        "deleted"
    );
}

#[test]
fn authority_is_scoped_and_independent_of_attachment_expiry_but_assignment_closure_stops_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), false);
    let token = attachment["token"].as_str().unwrap();
    let assignment = attachment["assignment_id"].as_str().unwrap();
    let reader = runtime.create_attachment(assignment, "run").unwrap();
    let reader = reader["token"].as_str().unwrap();
    let schedule = create(
        &mut runtime,
        token,
        &version,
        "scope",
        json!({"kind":"interval","every_seconds":1,"start_at_ms":1000}),
    );
    let inspected = inspect(&mut runtime, reader, &schedule);
    assert_eq!(inspected["schedule_id"], schedule["schedule_id"]);
    assert_eq!(
        runtime
            .scoped_call(
                reader,
                "manage",
                json!({"action":"schedule.pause","schedule_id":schedule["schedule_id"]})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    let (other, _, _) = setup(&mut runtime, &dir.path().join("other"), false);
    assert_eq!(
        runtime
            .scoped_call(
                other["token"].as_str().unwrap(),
                "inspect",
                json!({"view":"schedule","schedule_id":schedule["schedule_id"]})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    runtime
        .revoke_attachment(attachment["attachment_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 1);
    runtime.drive_pending().unwrap();
    runtime.close_assignment(assignment).unwrap();
    assert_eq!(runtime.tick_schedules(2000).unwrap().started, 0);
    let fixture = Connection::open(state.join("state.sqlite3")).unwrap();
    assert_eq!(
        fixture
            .query_row("SELECT state FROM schedules", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "paused"
    );
}

#[test]
fn restore_pauses_future_jobs_until_explicit_author_resume() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), false);
    let token = attachment["token"].as_str().unwrap();
    let schedule = create(
        &mut runtime,
        token,
        &version,
        "restored",
        json!({"kind":"once","at_ms":1000}),
    );
    let backup = dir.path().join("backup");
    runtime.backup_to(&backup).unwrap();
    let destination = dir.path().join("restored");
    Runtime::restore_from(&backup, &destination).unwrap();
    let mut restored = Runtime::open(destination).unwrap();
    assert_eq!(restored.tick_schedules(1000).unwrap().started, 0);
    let fresh = restored
        .create_attachment(attachment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = fresh["token"].as_str().unwrap();
    assert_eq!(inspect(&mut restored, token, &schedule)["state"], "paused");
    restored
        .scoped_call(
            token,
            "manage",
            json!({"action":"schedule.resume","schedule_id":schedule["schedule_id"]}),
        )
        .unwrap();
    assert_eq!(restored.tick_schedules(1000).unwrap().started, 1);
}

#[test]
fn old_v8_backup_is_verified_migrated_and_authority_revoked() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), false);
    let token = attachment["token"].as_str().unwrap();
    let run = runtime
        .scoped_call(
            token,
            "run",
            json!({"process_ref":version,"input":{},"request_key":"old-run","mode":"start"}),
        )
        .unwrap();
    runtime.drive_pending().unwrap();
    let backup = dir.path().join("backup");
    runtime.backup_to(&backup).unwrap();
    // Deterministic previous-version fixture: strip the new v9 and v10 schema,
    // retaining the real persisted run, artifact inventory, and credentials policy.
    let fixture = Connection::open(backup.join("state.sqlite3")).unwrap();
    fixture.execute_batch("DROP TABLE worker_policy; DROP TABLE improvement_holdout_claims; DROP TABLE improvement_cycles; DROP TABLE improvement_selection; DROP TABLE saved_accounts; DROP TABLE connection_library_uses; DROP TABLE connection_library_blocks; DROP TABLE connection_library; DROP TABLE billing_invoice_lines; DROP TABLE billing_invoices; DROP TABLE billing_budgets; DROP TABLE billing_calls; DROP TABLE mcp_sampling_jobs;DROP TABLE mcp_input_receipts; DROP TABLE mcp_interactions; DROP TABLE price_reviews; DROP TABLE price_sources; DROP TABLE runtime_maintenance; DROP TABLE desktop_preferences; DROP TABLE connection_requests; DROP TABLE capacity_policy; DROP TABLE schedule_occurrences; DROP INDEX runs_by_schedule; ALTER TABLE runs DROP COLUMN schedule_id; ALTER TABLE runs DROP COLUMN scheduled_for_ms; DROP TABLE schedules; PRAGMA user_version=8;").unwrap();
    fixture.close().unwrap();
    let bytes = std::fs::read(backup.join("state.sqlite3")).unwrap();
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(backup.join("backup.json")).unwrap()).unwrap();
    manifest["database_schema"] = json!(8);
    manifest["database_bytes"] = json!(bytes.len());
    manifest["database_sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
    std::fs::write(
        backup.join("backup.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let destination = dir.path().join("restored");
    Runtime::restore_from(&backup, &destination).unwrap();
    let mut restored = Runtime::open(&destination).unwrap();
    assert_eq!(restored.storage_usage().unwrap().schema_version, 18);
    assert_eq!(
        restored
            .inspect(run["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "succeeded"
    );
    assert_eq!(
        restored
            .scoped_call(token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    let fresh = restored
        .create_attachment(attachment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = fresh["token"].as_str().unwrap();
    let schedule = create(
        &mut restored,
        token,
        &version,
        "after-upgrade",
        json!({"kind":"once","at_ms":1000}),
    );
    assert_eq!(restored.tick_schedules(1000).unwrap().started, 1);
    assert_eq!(
        inspect(&mut restored, token, &schedule)["state"],
        "completed"
    );
}

#[test]
fn skipped_history_is_bounded_and_clock_rollback_does_not_repeat_firings() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), true);
    let token = attachment["token"].as_str().unwrap();
    let schedule = create(
        &mut runtime,
        token,
        &version,
        "bounded",
        json!({"kind":"interval","every_seconds":1,"start_at_ms":1000}),
    );
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 1);
    runtime.drive_pending().unwrap();
    for second in 2..=105 {
        assert_eq!(
            runtime
                .tick_schedules(second * 1000)
                .unwrap()
                .skipped_overlap,
            1
        );
    }
    assert_eq!(runtime.tick_schedules(2_000).unwrap().skipped_overlap, 0);
    let fixture = Connection::open(state.join("state.sqlite3")).unwrap();
    assert_eq!(
        fixture
            .query_row("SELECT count(*) FROM schedule_occurrences", [], |row| row
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        100
    );
    assert_eq!(
        fixture
            .query_row("SELECT count(*) FROM runs", [], |row| row.get::<_, u64>(0))
            .unwrap(),
        1
    );
    let current = inspect(&mut runtime, token, &schedule);
    assert_eq!(current["history"].as_array().unwrap().len(), 20);
    assert_eq!(runtime.tick_schedules(106_000).unwrap().skipped_overlap, 1);
}

#[test]
fn composed_schedule_keeps_child_config_and_blocks_overlap_while_child_waits() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let (attachment, child, _) = setup(&mut runtime, &project, true);
    let token = attachment["token"].as_str().unwrap();
    let mut parent: Value =
        serde_json::from_slice(&std::fs::read(project.join("process.json")).unwrap()).unwrap();
    parent["schema_version"] = json!("0.2");
    parent["name"] = json!("scheduled.parent");
    parent["steps"] = json!([{"id":"child","kind":"call","effect":"control","needs":[],"retry":{"max_attempts":1},"inputs":{},"call":{"process_ref":child,"output_pointer":""}}]);
    parent["output"] = json!({"result":{"step":"child","pointer":""}});
    std::fs::write(
        project.join("parent.json"),
        serde_json::to_vec(&parent).unwrap(),
    )
    .unwrap();
    let version = runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"process.register","path":"parent.json"}),
        )
        .unwrap()["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"config.set","process_ref":child,"values":{"locale":"es"}}),
        )
        .unwrap();
    let schedule = create(
        &mut runtime,
        token,
        &version,
        "graph",
        json!({"kind":"interval","every_seconds":1,"start_at_ms":1000}),
    );
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"config.set","process_ref":child,"values":{"locale":"fr"}}),
        )
        .unwrap();
    runtime.tick_schedules(1000).unwrap();
    for _ in 0..3 {
        runtime.drive_pending().unwrap();
    }
    let runs = runtime
        .scoped_call(token, "inspect", json!({"view":"runs","limit":20}))
        .unwrap();
    assert_eq!(runs["runs"].as_array().unwrap().len(), 2);
    let child_run = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["parent_run_id"].is_string())
        .unwrap();
    assert_eq!(child_run["state"], "waiting_for_agent");
    assert_eq!(
        runtime
            .scoped_call(
                token,
                "inspect",
                json!({"view":"config","run_id":child_run["run_id"]})
            )
            .unwrap()["config"]["locale"],
        "es"
    );
    assert_eq!(runtime.tick_schedules(2000).unwrap().skipped_overlap, 1);
    assert_eq!(
        inspect(&mut runtime, token, &schedule)["last_occurrence"]["outcome"],
        "skipped_overlap"
    );
}

#[test]
fn saved_input_artifacts_survive_original_run_retention_and_remain_authorized() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), false);
    let token = attachment["token"].as_str().unwrap();
    let original=runtime.scoped_call(token,"run",json!({"mode":"start","process_ref":version,"input":{},"request_key":"artifact-source"})).unwrap();
    runtime.drive_pending().unwrap();
    let artifact = runtime
        .inspect(original["run_id"].as_str().unwrap())
        .unwrap()
        .output
        .unwrap()["result"]["artifact"]
        .clone();
    assert!(artifact["artifact_id"].is_string());
    let schedule=runtime.scoped_call(token,"manage",json!({"action":"schedule.create","name":"Artifact holder","process_ref":version,"input":{"artifact":artifact},"request_key":"retained","trigger":{"kind":"once","at_ms":1000}})).unwrap();
    let fixture = Connection::open(state.join("state.sqlite3")).unwrap();
    fixture
        .execute("UPDATE events SET created_at=1", [])
        .unwrap();
    fixture.execute("UPDATE runs SET created_at=1", []).unwrap();
    drop(fixture);
    std::fs::File::open(
        state
            .join("objects/sha256")
            .join(artifact["sha256"].as_str().unwrap()),
    )
    .unwrap()
    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
    .unwrap();
    let plan = runtime.retention_preview(10_000, 2_000).unwrap();
    assert!(
        plan.run_ids
            .contains(&original["run_id"].as_str().unwrap().to_owned())
    );
    assert!(plan.artifact_ids.is_empty());
    runtime.retention_apply(&plan).unwrap();
    runtime
        .scoped_call(
            token,
            "inspect",
            json!({"view":"artifact","reference":artifact}),
        )
        .unwrap();
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 1);
    assert_eq!(
        inspect(&mut runtime, token, &schedule)["state"],
        "completed"
    );
}

#[test]
fn concurrent_scheduler_instances_create_one_occurrence() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), false);
    let token = attachment["token"].as_str().unwrap();
    create(
        &mut runtime,
        token,
        &version,
        "concurrent",
        json!({"kind":"once","at_ms":1000}),
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let state = state.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut runtime = Runtime::open(state).unwrap();
                barrier.wait();
                runtime.tick_schedules(1000).unwrap().started
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .sum::<usize>(),
        1
    );
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn scheduler_crash_probe() {
    if let Some(state) = std::env::var_os("APR_SCHEDULE_TEST_STATE") {
        Runtime::open(state).unwrap().tick_schedules(1000).unwrap();
        panic!("scheduled crash point did not fire");
    }
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn killed_firing_transaction_never_loses_or_duplicates_an_occurrence() {
    use std::os::unix::process::ExitStatusExt;
    for point in ["schedule_firing_before_commit", "schedule_firing_committed"] {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut runtime = Runtime::open(&state).unwrap();
        let (attachment, version, _) = setup(&mut runtime, &dir.path().join("project"), false);
        let token = attachment["token"].as_str().unwrap();
        let schedule = create(
            &mut runtime,
            token,
            &version,
            "crash",
            json!({"kind":"once","at_ms":1000}),
        );
        drop(runtime);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "scheduler_crash_probe", "--nocapture"])
            .env("APR_SCHEDULE_TEST_STATE", &state)
            .env("APR_TEST_CRASH_AT", point)
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(9));
        let mut runtime = Runtime::open(&state).unwrap();
        runtime.tick_schedules(1000).unwrap();
        assert_eq!(runtime.tick_schedules(2000).unwrap().started, 0);
        assert_eq!(
            inspect(&mut runtime, token, &schedule)["history"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let runs = runtime
            .scoped_call(token, "inspect", json!({"view":"runs","limit":20}))
            .unwrap();
        assert_eq!(runs["runs"].as_array().unwrap().len(), 1);
        runtime.drive_pending().unwrap();
        assert_eq!(
            runtime
                .inspect(runs["runs"][0]["run_id"].as_str().unwrap())
                .unwrap()
                .state,
            "succeeded"
        );
    }
}

#[test]
fn changed_native_grants_pause_jobs_without_silently_rebinding() {
    let output = std::process::Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let python = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let (attachment, _, _) = setup(&mut runtime, &project, false);
    let token = attachment["token"].as_str().unwrap();
    let assignment = attachment["assignment_id"].as_str().unwrap();
    std::fs::write(
        project.join("main.py"),
        include_bytes!("../../../examples/native-normalize/main.py"),
    )
    .unwrap();
    std::fs::write(
        project.join("native.json"),
        include_bytes!("../../../examples/native-normalize/process.json"),
    )
    .unwrap();
    let version = runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"process.register","path":"native.json"}),
        )
        .unwrap()["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let first = runtime.grant_python(assignment, &python).unwrap();
    let same_pin = json!({"action":"schedule.create","name":"Same native grant","process_ref":version,"input":{"text":"example"},"request_key":"native-same","trigger":{"kind":"once","at_ms":1000}});
    let same_schedule = runtime.scoped_call(token, "manage", same_pin).unwrap();
    assert_eq!(runtime.grant_python(assignment, &python).unwrap(), first);
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 1);
    runtime.drive_pending().unwrap();
    assert_eq!(
        inspect(&mut runtime, token, &same_schedule)["state"],
        "completed"
    );

    let changed_pin = json!({"action":"schedule.create","name":"Changed native grant","process_ref":version,"input":{"text":"example"},"request_key":"native-changed","trigger":{"kind":"once","at_ms":2000}});
    let schedule = runtime.scoped_call(token, "manage", changed_pin).unwrap();
    let replacement = dir.path().join("replacement-python");
    std::os::unix::fs::symlink(&python, &replacement).unwrap();
    runtime.grant_python(assignment, &replacement).unwrap();
    assert_eq!(runtime.tick_schedules(2000).unwrap().paused, 1);
    let current = inspect(&mut runtime, token, &schedule);
    assert_eq!(current["state"], "paused");
    assert!(
        current["last_error"]
            .as_str()
            .unwrap()
            .contains("changed or was revoked")
    );
    assert_eq!(
        runtime
            .scoped_call(token, "inspect", json!({"view":"runs"}))
            .unwrap()["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        runtime
            .scoped_call(
                token,
                "manage",
                json!({"action":"schedule.resume","schedule_id":schedule["schedule_id"]})
            )
            .is_err()
    );
}

#[test]
fn schedule_creation_keeps_combined_detail_inside_the_transport_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let (attachment, version, project) = setup(&mut runtime, &dir.path().join("project"), false);
    let token = attachment["token"].as_str().unwrap();
    runtime
        .set_project_config(&project, &version, json!({"locale":"x".repeat(40*1024)}))
        .unwrap();
    let result=runtime.scoped_call(token,"manage",json!({"action":"schedule.create","name":"Budgeted detail","process_ref":version,"input":{"text":"x".repeat(30*1024)},"request_key":"detail","trigger":{"kind":"once","at_ms":1000}}));
    assert_eq!(result.unwrap_err().code(), "LIMIT_EXCEEDED");
    assert_eq!(
        runtime
            .scoped_call(token, "inspect", json!({"view":"schedules"}))
            .unwrap()["schedules"],
        json!([])
    );
}

#[test]
fn cancelling_an_uncertain_native_run_does_not_allow_the_next_scheduled_effect() {
    use sha2::{Digest, Sha256};
    let output = std::process::Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let python = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let (attachment, _, _) = setup(&mut runtime, &project, false);
    let token = attachment["token"].as_str().unwrap();
    let assignment = attachment["assignment_id"].as_str().unwrap();
    let source = b"raise RuntimeError('simulated worker interruption')\n";
    let mut process: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(source)));
    std::fs::write(project.join("main.py"), source).unwrap();
    std::fs::write(
        project.join("native.json"),
        serde_json::to_vec(&process).unwrap(),
    )
    .unwrap();
    let version = runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"process.register","path":"native.json"}),
        )
        .unwrap()["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    runtime.grant_python(assignment, &python).unwrap();
    let schedule=runtime.scoped_call(token,"manage",json!({"action":"schedule.create","name":"Uncertain native work","process_ref":version,"input":{"text":"example"},"request_key":"uncertain","trigger":{"kind":"interval","every_seconds":1,"start_at_ms":1000}})).unwrap();
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 1);
    runtime.drive_pending().unwrap();
    let run = inspect(&mut runtime, token, &schedule)["last_occurrence"]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(runtime.inspect(&run).unwrap().state, "needs_reconciliation");
    runtime
        .scoped_call(token, "resume", json!({"action":"cancel","run_id":run}))
        .unwrap();
    assert_eq!(runtime.inspect(&run).unwrap().state, "cancelled");
    assert_eq!(runtime.tick_schedules(2000).unwrap().skipped_overlap, 1);
    assert_eq!(
        runtime
            .scoped_call(token, "inspect", json!({"view":"runs"}))
            .unwrap()["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
