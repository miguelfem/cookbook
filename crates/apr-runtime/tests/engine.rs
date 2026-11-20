use apr_core::admit_value;
use apr_runtime::{RunEnvelope, Runtime, Submission};
use serde_json::{Value, json};

fn review_process() -> Value {
    json!({
        "schema_version":"0.1", "name":"local.review", "description":"Review a local record",
        "input_schema":{"type":"object","required":["title"],"properties":{"title":{"type":"string"}},"additionalProperties":false},
        "output_schema":{"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"agent","capability":"review"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[{
            "id":"review","kind":"agent","needs":[],"effect":"agent","retry":{"max_attempts":1},
            "capability":"review","instructions":"Review the record and return your decision.",
            "inputs":{"record":{"input":""}},
            "result_schema":{"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false}
        }],
        "output":{"approved":{"step":"review","pointer":"/approved"}}
    })
}

#[test]
fn accepted_process_waits_durably_and_reuses_its_start_key() {
    let directory = tempfile::tempdir().unwrap();
    let process = admit_value(&review_process()).unwrap();
    let run_id;
    let request_id;
    {
        let mut runtime = Runtime::open(directory.path()).unwrap();
        let run = runtime
            .start(&process, json!({"title":"A record"}), "request-1")
            .unwrap();
        assert_eq!(run.state, "waiting_for_agent");
        assert_eq!(
            run.request.as_ref().unwrap().input,
            json!({"record":{"title":"A record"}})
        );
        run_id = run.run_id;
        request_id = run.request.unwrap().request_id;
    }
    let mut runtime = Runtime::open(directory.path()).unwrap();
    let run = runtime
        .start(&process, json!({"title":"A record"}), "request-1")
        .unwrap();
    assert_eq!(run.run_id, run_id);
    assert_eq!(run.request.unwrap().request_id, request_id);
    assert!(
        runtime
            .start(&process, json!({"title":"Changed"}), "request-1")
            .is_err()
    );
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
        submission_key: "review-result-1".into(),
        continuation_token: request.continuation_token.clone(),
        output: json!({"approved":true}),
        provenance: json!({"source":"manual","client":"test"}),
    }
}

#[test]
fn schema_errors_preserve_the_handoff_and_duplicate_results_replay_the_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let process = admit_value(&review_process()).unwrap();
    let mut runtime = Runtime::open(directory.path()).unwrap();
    let run = runtime
        .start(&process, json!({"title":"A record"}), "schema-test")
        .unwrap();
    let mut submission = answer(&run);
    submission.output = json!({"approved":"not a boolean"});
    assert_eq!(
        runtime.resume(&submission).unwrap_err().code(),
        "INVALID_INPUT"
    );
    assert_eq!(runtime.inspect(&run.run_id).unwrap(), run);
    submission.output = json!({"approved":true});
    let receipt = runtime.resume(&submission).unwrap();
    assert_eq!(
        runtime.inspect(&run.run_id).unwrap().output,
        Some(json!({"approved":true}))
    );
    drop(runtime);
    let mut runtime = Runtime::open(directory.path()).unwrap();
    submission.continuation_token = "token was consumed; semantics are unchanged".into();
    assert_eq!(runtime.resume(&submission).unwrap(), receipt);
    submission.output = json!({"approved":false});
    assert_eq!(
        runtime.resume(&submission).unwrap_err().code(),
        "SUBMISSION_CONFLICT"
    );
    submission.submission_key = "different-key".into();
    assert_eq!(
        runtime.resume(&submission).unwrap_err().code(),
        "STALE_HANDOFF"
    );
}

#[test]
fn concurrent_duplicate_submissions_commit_one_result() {
    let directory = tempfile::tempdir().unwrap();
    let process = admit_value(&review_process()).unwrap();
    let mut runtime = Runtime::open(directory.path()).unwrap();
    let run = runtime
        .start(&process, json!({"title":"Concurrent record"}), "concurrent")
        .unwrap();
    let submission = answer(&run);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let root = directory.path().to_path_buf();
            let barrier = barrier.clone();
            let submission = submission.clone();
            std::thread::spawn(move || {
                let mut runtime = Runtime::open(root).unwrap();
                barrier.wait();
                runtime.resume(&submission).unwrap()
            })
        })
        .collect();
    let receipts: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(receipts.iter().all(|receipt| receipt == &receipts[0]));
    let events = runtime.events(&run.run_id, 0, 100).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "handoff.accepted")
            .count(),
        1
    );
    assert_eq!(
        events.iter().filter(|e| e.kind == "run.succeeded").count(),
        1
    );
}

#[test]
fn renewing_a_saved_handoff_rotates_authority_without_repeating_work() {
    let directory = tempfile::tempdir().unwrap();
    let process = admit_value(&review_process()).unwrap();
    let mut runtime = Runtime::open(directory.path()).unwrap();
    let run = runtime
        .start(&process, json!({"title":"Paused record"}), "renew")
        .unwrap();
    let old = answer(&run);
    let renewed = runtime.renew_handoff(&run.run_id).unwrap();
    let request = renewed.request.as_ref().unwrap();
    assert_eq!(request.request_id, old.request_id);
    assert_eq!(request.generation, 2);
    assert_ne!(request.continuation_token, old.continuation_token);
    assert_eq!(runtime.resume(&old).unwrap_err().code(), "STALE_HANDOFF");
    drop(runtime);
    let mut runtime = Runtime::open(directory.path()).unwrap();
    assert_eq!(runtime.inspect(&run.run_id).unwrap(), renewed);
    runtime.resume(&answer(&renewed)).unwrap();
    assert_eq!(runtime.inspect(&run.run_id).unwrap().state, "succeeded");
    assert!(runtime.renew_handoff(&run.run_id).is_err());
}
