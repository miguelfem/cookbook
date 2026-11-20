use apr_core::admit_value;
use apr_runtime::Runtime;
use serde_json::json;

fn artifact_check() -> apr_core::AdmittedProcess {
    admit_value(&json!({
        "schema_version": "0.1", "name": "check.artifacts", "description": "Check registered artifacts",
        "input_schema": {"type": "object"}, "output_schema": {"type": "object"},
        "config": {"schema": {"type": "object"}, "defaults": {}, "run_override_keys": []},
        "capabilities": [{"kind": "artifacts"}],
        "limits": {"max_step_ms": 1000, "max_artifact_bytes": 1048576, "max_provider_calls": 0, "max_agent_requests": 0},
        "steps": [{"id": "check", "kind": "builtin", "operation": "artifact.validate_set", "needs": [],
            "effect": "pure", "retry": {"max_attempts": 1}, "inputs": {
                "items": {"input": "/sources"}, "media_type_prefix": {"literal": "image/"}
            }}],
        "output": {"artifacts": {"step": "check", "pointer": "/artifacts"}}
    })).unwrap()
}

#[test]
fn caller_cannot_relabel_a_registered_artifact_to_pass_a_type_constraint() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "A plain text record").unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let original = runtime
        .import_artifact(&source, "text/plain", 1048576)
        .unwrap();
    let mut relabelled = original.clone();
    relabelled.media_type = "image/png".into();
    let error = runtime
        .start(
            &artifact_check(),
            json!({"sources": [relabelled]}),
            "forged-label",
        )
        .unwrap_err();
    assert_eq!(error.code(), "ARTIFACT_INTEGRITY");
    assert_eq!(
        runtime.read_artifact(&original, 1048576).unwrap(),
        b"A plain text record"
    );
}

#[test]
fn importing_identical_bytes_cannot_change_previously_registered_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "A plain text record").unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let original = runtime
        .import_artifact(&source, "text/plain", 1048576)
        .unwrap();
    assert_eq!(
        runtime
            .import_artifact(&source, "text/plain", 1048576)
            .unwrap(),
        original
    );
    assert_eq!(
        runtime
            .import_artifact(&source, "image/png", 1048576)
            .unwrap_err()
            .code(),
        "ARTIFACT_INTEGRITY"
    );
}

#[test]
fn a_failed_sibling_closes_the_visible_handoff_and_cannot_be_renewed() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest = serde_json::to_value(artifact_check().manifest).unwrap();
    manifest["limits"]["max_agent_requests"] = json!(1);
    manifest["steps"] = json!([
        {"id": "review", "kind": "agent", "needs": [], "effect": "agent", "retry": {"max_attempts": 1},
            "capability": "review", "instructions": "Review data", "inputs": {}, "result_schema": {"type": "object"}},
        {"id": "check", "kind": "builtin", "operation": "json.select", "needs": [], "effect": "pure",
            "retry": {"max_attempts": 1}, "inputs": {"value": {"literal": {}}, "pointer": {"literal": "/missing"}}}
    ]);
    manifest["output"] = json!({"result": {"step": "check", "pointer": "/value"}});
    let mut runtime = Runtime::open(directory.path()).unwrap();
    let run = runtime
        .start(
            &admit_value(&manifest).unwrap(),
            json!({}),
            "sibling-failure",
        )
        .unwrap();
    assert_eq!(run.state, "failed");
    assert!(
        run.request.is_none(),
        "failed run exposed an actionable pending request"
    );
    assert_eq!(
        runtime
            .events(&run.run_id, 0, 100)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "handoff.created")
            .count(),
        1
    );
    assert!(runtime.renew_handoff(&run.run_id).is_err());
    assert_eq!(runtime.advance(&run.run_id).unwrap().state, "failed");
}

#[test]
fn a_restored_handoff_requires_explicit_renewal_before_becoming_actionable() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest = serde_json::to_value(artifact_check().manifest).unwrap();
    manifest["limits"]["max_agent_requests"] = json!(1);
    manifest["steps"] = json!([{ "id":"review","kind":"agent","needs":[],"effect":"agent","retry":{"max_attempts":1},
        "capability":"review","instructions":"Review restored data","inputs":{},"result_schema":{"type":"object"}}]);
    manifest["output"] = json!({"result":{"step":"review","pointer":""}});
    let mut runtime = Runtime::open(directory.path()).unwrap();
    let waiting = runtime
        .start(&admit_value(&manifest).unwrap(), json!({}), "restore")
        .unwrap();
    let original = waiting.request.unwrap();
    // Controlled restore fixture: backups retain pending work but omit the
    // issuing machine's bearer token and mark the claim as needing renewal.
    let fixture = rusqlite::Connection::open(directory.path().join("state.sqlite3")).unwrap();
    fixture
        .execute(
            "UPDATE handoffs SET token_hash='',expires_at=0 WHERE run_id=?1",
            [&waiting.run_id],
        )
        .unwrap();
    drop(fixture);
    for entry in std::fs::read_dir(directory.path().join("handoffs")).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let restored = runtime.inspect(&waiting.run_id).unwrap();
    assert!(restored.request.is_none());
    assert_eq!(
        restored.error.as_deref(),
        Some("caller handoff must be renewed after restore")
    );
    let renewed = runtime.renew_handoff(&waiting.run_id).unwrap();
    let request = renewed.request.unwrap();
    assert_eq!(request.generation, original.generation + 1);
    assert_ne!(request.continuation_token, original.continuation_token);
    assert!(renewed.error.is_none());
    let mut submission = apr_runtime::Submission {
        protocol_version: "0.1".into(),
        action: "submit".into(),
        run_id: waiting.run_id.clone(),
        request_id: request.request_id,
        step_instance_id: request.step_instance_id,
        generation: original.generation,
        submission_key: "restored-result".into(),
        continuation_token: original.continuation_token,
        output: json!({"approved":true}),
        provenance: json!({"source":"manual"}),
    };
    assert_eq!(
        runtime.resume(&submission).unwrap_err().code(),
        "STALE_HANDOFF"
    );
    submission.generation = request.generation;
    submission.continuation_token = request.continuation_token;
    runtime.resume(&submission).unwrap();
    assert_eq!(runtime.inspect(&waiting.run_id).unwrap().state, "succeeded");
}
