use std::collections::BTreeMap;

use apr_runtime::{Runtime, RuntimeError, worker_policy::WorkerPolicy};

#[test]
fn owner_worker_policy_is_durable_and_rejects_stale_or_invalid_edits() {
    let state = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(state.path()).unwrap();
    let initial = runtime.worker_policy().unwrap();
    assert_eq!(initial, WorkerPolicy::default());
    let status = runtime.worker_status().unwrap();
    assert_eq!(status["running"], 0);
    assert_eq!(status["queued"], 0);
    assert_eq!(status["policy"], serde_json::to_value(&initial).unwrap());

    let mut caps = BTreeMap::new();
    caps.insert("synthetic-service".to_owned(), 2);
    let proposed = WorkerPolicy {
        revision: initial.revision,
        max_workers: 8,
        max_local_workers: 3,
        per_connection: caps,
    };
    let saved = runtime.set_worker_policy(proposed.clone(), 0).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        Runtime::open(state.path())
            .unwrap()
            .worker_policy()
            .unwrap(),
        saved
    );

    assert!(matches!(
        runtime.set_worker_policy(proposed, 0),
        Err(RuntimeError::State(_))
    ));
    let mut invalid = saved.clone();
    invalid.max_local_workers = 9;
    assert!(matches!(
        runtime.set_worker_policy(invalid, 1),
        Err(RuntimeError::InvalidInput(_))
    ));
    assert_eq!(runtime.worker_policy().unwrap(), saved);

    let invalid_json = serde_json::json!({
        "revision": 1, "max_workers": 8, "max_local_workers": 3,
        "per_connection": {}, "ignored": true
    });
    assert!(serde_json::from_value::<WorkerPolicy>(invalid_json).is_err());
}

#[test]
fn migration_from_existing_schema_creates_default_worker_policy() {
    let state = tempfile::tempdir().unwrap();
    drop(Runtime::open(state.path()).unwrap());
    let db = rusqlite::Connection::open(state.path().join("state.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE worker_policy; PRAGMA user_version = 17;")
        .unwrap();
    drop(db);
    let runtime = Runtime::open(state.path()).unwrap();
    assert_eq!(runtime.worker_policy().unwrap(), WorkerPolicy::default());
}

#[test]
fn scoped_workers_hide_other_connection_limits_and_cannot_change_policy() {
    use serde_json::json;
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut runtime = Runtime::open(temp.path().join("runtime")).unwrap();
    let project_id = runtime.attach_project(&project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let assignment = runtime.create_assignment(&project_id, "scope").unwrap()["assignment_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let attachment = runtime.create_attachment(&assignment, "author").unwrap();
    let token = attachment["token"].as_str().unwrap();
    let connection = serde_json::from_value(json!({"label":"Synthetic scope", "endpoint":"https://example.com/read", "method":"POST", "auth":{"kind":"none"}, "input_schema":{"type":"object"}, "output_schema":{"type":"object"}, "timeout_ms":1000, "max_response_bytes":4096})).unwrap();
    runtime
        .configure_connection(&assignment, "mine", connection, None)
        .unwrap();
    let policy = WorkerPolicy {
        per_connection: BTreeMap::from([("mine".into(), 2), ("other-account-name".into(), 1)]),
        ..WorkerPolicy::default()
    };
    runtime.set_worker_policy(policy, 0).unwrap();
    let visible = runtime
        .scoped_call(token, "inspect", json!({"view":"workers"}))
        .unwrap();
    assert_eq!(visible["policy"]["per_connection"], json!({"mine":2}));
    assert_eq!(visible["scope"], "assignment");
    assert!(!visible.to_string().contains("other-account-name"));
    assert!(
        runtime
            .scoped_call(
                token,
                "manage",
                json!({"action":"worker_policy.set","max_workers":64})
            )
            .is_err()
    );
    runtime
        .revoke_execution(&assignment, "connection", "mine")
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(token, "inspect", json!({"view":"workers"}))
            .unwrap()["policy"]["per_connection"],
        json!({})
    );
    assert_eq!(runtime.worker_policy().unwrap().max_workers, 4);
}
