use apr_runtime::{Runtime, capacity::CapacityPolicy};
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn owner_capacity_is_persisted_atomic_and_has_hard_ceilings() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    assert_eq!(
        runtime.capacity_policy().unwrap(),
        CapacityPolicy::default()
    );
    let policy = CapacityPolicy {
        max_call_depth: 64,
        max_child_runs: 2048,
        max_step_instances: 30000,
        max_graph_versions: 2048,
    };
    assert_eq!(runtime.set_capacity_policy(policy).unwrap(), policy);
    let mut another = Runtime::open(&state).unwrap();
    assert_eq!(another.capacity_policy().unwrap(), policy);
    for invalid in [
        CapacityPolicy {
            max_call_depth: 129,
            ..policy
        },
        CapacityPolicy {
            max_child_runs: 4097,
            ..policy
        },
        CapacityPolicy {
            max_step_instances: 100001,
            ..policy
        },
        CapacityPolicy {
            max_graph_versions: 4097,
            ..policy
        },
        CapacityPolicy {
            max_call_depth: 0,
            ..policy
        },
        CapacityPolicy {
            max_child_runs: 0,
            ..policy
        },
        CapacityPolicy {
            max_step_instances: 0,
            ..policy
        },
        CapacityPolicy {
            max_graph_versions: 0,
            ..policy
        },
    ] {
        assert!(another.set_capacity_policy(invalid).is_err());
        assert_eq!(runtime.capacity_policy().unwrap(), policy);
    }
    another
        .set_capacity_policy(CapacityPolicy::HARD_CEILINGS)
        .unwrap();
    assert_eq!(
        runtime.capacity_policy().unwrap(),
        CapacityPolicy::HARD_CEILINGS
    );
}

#[test]
fn author_attachments_cannot_mutate_device_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let p = runtime.attach_project(&project).unwrap();
    let assignment = runtime
        .create_assignment(p["project_id"].as_str().unwrap(), "capacity isolation")
        .unwrap();
    let attachment = runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = attachment["token"].as_str().unwrap();
    for action in ["capacity.set", "capacity.update", "capacity.policy"] {
        assert!(
            runtime
                .scoped_call(
                    token,
                    "manage",
                    json!({"action":action,"max_call_depth":128})
                )
                .is_err()
        );
    }
    assert_eq!(
        runtime.capacity_policy().unwrap(),
        CapacityPolicy::default()
    );
}

#[test]
fn version_nine_database_upgrades_and_capacity_backup_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    drop(Runtime::open(&state).unwrap());
    let db = Connection::open(state.join("state.sqlite3")).unwrap();
    db.execute_batch(
        "DROP TABLE worker_policy; DROP TABLE improvement_holdout_claims; DROP TABLE improvement_cycles; DROP TABLE improvement_selection; DROP TABLE saved_accounts; DROP TABLE connection_library_uses; DROP TABLE connection_library_blocks; DROP TABLE connection_library; DROP TABLE billing_invoice_lines; DROP TABLE billing_invoices; DROP TABLE billing_budgets; DROP TABLE billing_calls; DROP TABLE mcp_sampling_jobs;DROP TABLE mcp_input_receipts; DROP TABLE mcp_interactions; DROP TABLE price_reviews; DROP TABLE price_sources; DROP TABLE runtime_maintenance; DROP TABLE desktop_preferences; DROP TABLE connection_requests; DROP TABLE capacity_policy; PRAGMA user_version=9;",
    )
    .unwrap();
    db.close().unwrap();
    let mut runtime = Runtime::open(&state).unwrap();
    assert_eq!(runtime.storage_usage().unwrap().schema_version, 18);
    assert_eq!(
        runtime.capacity_policy().unwrap(),
        CapacityPolicy::default()
    );
    let policy = CapacityPolicy {
        max_call_depth: 60,
        ..CapacityPolicy::default()
    };
    runtime.set_capacity_policy(policy).unwrap();
    let backup = dir.path().join("backup");
    runtime.backup_to(&backup).unwrap();
    let restored = dir.path().join("restored");
    Runtime::restore_from(&backup, &restored).unwrap();
    assert_eq!(
        Runtime::open(restored).unwrap().capacity_policy().unwrap(),
        policy
    );
}

#[test]
fn version_nine_backup_is_verified_before_capacity_migration() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(dir.path().join("state")).unwrap();
    let backup = dir.path().join("backup-v9");
    runtime.backup_to(&backup).unwrap();
    let db = Connection::open(backup.join("state.sqlite3")).unwrap();
    db.execute_batch(
        "DROP TABLE worker_policy; DROP TABLE improvement_holdout_claims; DROP TABLE improvement_cycles; DROP TABLE improvement_selection; DROP TABLE saved_accounts; DROP TABLE connection_library_uses; DROP TABLE connection_library_blocks; DROP TABLE connection_library; DROP TABLE billing_invoice_lines; DROP TABLE billing_invoices; DROP TABLE billing_budgets; DROP TABLE billing_calls; DROP TABLE mcp_sampling_jobs;DROP TABLE mcp_input_receipts; DROP TABLE mcp_interactions; DROP TABLE price_reviews; DROP TABLE price_sources; DROP TABLE runtime_maintenance; DROP TABLE desktop_preferences; DROP TABLE connection_requests; DROP TABLE capacity_policy; PRAGMA user_version=9;",
    )
    .unwrap();
    db.close().unwrap();
    let bytes = std::fs::read(backup.join("state.sqlite3")).unwrap();
    let mut inventory: Value =
        serde_json::from_slice(&std::fs::read(backup.join("backup.json")).unwrap()).unwrap();
    inventory["database_schema"] = json!(9);
    inventory["database_bytes"] = json!(bytes.len());
    inventory["database_sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
    std::fs::write(
        backup.join("backup.json"),
        serde_json::to_vec(&inventory).unwrap(),
    )
    .unwrap();
    let restored = dir.path().join("restored");
    Runtime::restore_from(&backup, &restored).unwrap();
    let restored = Runtime::open(restored).unwrap();
    assert_eq!(restored.storage_usage().unwrap().schema_version, 18);
    assert_eq!(
        restored.capacity_policy().unwrap(),
        CapacityPolicy::default()
    );
}

#[test]
fn lower_policy_applies_to_new_roots_and_occurrences_but_not_replay_or_resume() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let process = apr_core::admit_value(&json!({"schema_version":"0.2","name":"capacity.review","description":"Synthetic admission fixture",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},"capabilities":[{"kind":"agent"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1000000,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[{"id":"review","kind":"agent","effect":"agent","needs":[],"retry":{"max_attempts":1},"inputs":{},"instructions":"Review synthetic input","capability":"review","result_schema":{"type":"object"}}],
        "output":{"result":{"step":"review","pointer":""}}})).unwrap();
    // Historical omitted manifest budgets remain independent of new owner defaults.
    assert_eq!(process.manifest.limits.max_call_depth, 8);
    assert_eq!(process.manifest.limits.max_child_runs, 256);
    std::fs::write(project.join("process.json"), &process.canonical_bytes).unwrap();
    let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
    let p = runtime.attach_project(&project).unwrap();
    let assignment = runtime
        .create_assignment(p["project_id"].as_str().unwrap(), "new-root capacity")
        .unwrap();
    let attachment = runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = attachment["token"].as_str().unwrap();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )
        .unwrap();
    let start =
        json!({"mode":"start","process_ref":process.version,"input":{},"request_key":"accepted"});
    let run = runtime.scoped_call(token, "run", start.clone()).unwrap();
    runtime.drive_pending().unwrap();
    let schedule=runtime.scoped_call(token,"manage",json!({"action":"schedule.create","name":"Future occurrence","process_ref":process.version,"input":{},"request_key":"future","trigger":{"kind":"once","at_ms":1000}})).unwrap();
    runtime
        .set_capacity_policy(CapacityPolicy {
            max_call_depth: 1,
            max_child_runs: 1,
            ..CapacityPolicy::default()
        })
        .unwrap();
    let capacity = runtime
        .scoped_call(token, "inspect", json!({"view":"capacity"}))
        .unwrap();
    assert!(capacity.to_string().contains("max_call_depth"));
    assert_eq!(
        runtime.scoped_call(token, "run", start).unwrap()["run_id"],
        run["run_id"]
    );
    assert_eq!(runtime.scoped_call(token,"run",json!({"mode":"start","process_ref":process.version,"input":{},"request_key":"new-root"})).unwrap_err().code(),"LIMIT_EXCEEDED");
    let tick = runtime.tick_schedules(1000).unwrap();
    assert_eq!(tick.started, 0);
    assert_eq!(tick.paused, 1);
    let job = runtime
        .scoped_call(
            token,
            "inspect",
            json!({"view":"schedule","schedule_id":schedule["schedule_id"]}),
        )
        .unwrap();
    assert_eq!(job["state"], "paused");
    let waiting = runtime.inspect(run["run_id"].as_str().unwrap()).unwrap();
    let request = waiting.request.unwrap();
    runtime.scoped_call(token,"resume",json!({"protocol_version":"0.1","action":"submit","run_id":run["run_id"],"request_id":request.request_id,"step_instance_id":request.step_instance_id,"generation":request.generation,"submission_key":"observed","continuation_token":request.continuation_token,"output":{"approved":true},"provenance":{"source":"fixture"}})).unwrap();
    for _ in 0..3 {
        runtime.drive_pending().unwrap();
    }
    assert_eq!(
        runtime
            .inspect(run["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "succeeded"
    );
    runtime
        .set_capacity_policy(CapacityPolicy::default())
        .unwrap();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"schedule.resume","schedule_id":schedule["schedule_id"]}),
        )
        .unwrap();
    assert_eq!(runtime.tick_schedules(1000).unwrap().started, 1);
}
