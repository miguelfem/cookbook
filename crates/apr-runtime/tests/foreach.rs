use apr_runtime::Runtime;
use serde_json::{Value, json};

fn process() -> Value {
    json!({"schema_version":"0.1","name":"foreach.snapshot","description":"Checkpoint each selected value",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"builtin"}],"limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[{"id":"each","kind":"builtin","operation":"json.select","effect":"pure","needs":[],"retry":{"max_attempts":2},
            "foreach":{"items":{"input":"/items"},"key":"/id","max_items":3,"concurrency":2},
            "inputs":{"value":{"item":""},"pointer":{"literal":"/value"}}}],
        "output":{"results":{"step":"each","pointer":""}}})
}

fn setup(root: &std::path::Path, manifest: &Value) -> (Runtime, String, String) {
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(root.join("state")).unwrap();
    let registered = runtime.attach_project(&project).unwrap();
    let assignment = runtime
        .create_assignment(registered["project_id"].as_str().unwrap(), "Foreach")
        .unwrap();
    let attachment = runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = attachment["token"].as_str().unwrap().to_owned();
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
    (runtime, token, version)
}

#[test]
fn expansion_keeps_item_order_and_checkpoints_across_a_restart() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, token, version) = setup(directory.path(), &process());
    let queued=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"second","value":2},{"id":"first","value":1}]},"request_key":"items"})).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    let mut cursor = None;
    for _ in 0..5 {
        runtime.drive_next_pending(&mut cursor).unwrap();
        let run = runtime.inspect(id).unwrap();
        assert_ne!(run.state, "failed", "{:?}", run.error);
        if run.completed_steps > 0 {
            break;
        }
    }
    let halfway = runtime.inspect(id).unwrap();
    let completed = halfway
        .steps
        .iter()
        .find(|step| step.state == "succeeded")
        .expect("first item must be committed")
        .id
        .clone();
    let expansion_events = runtime
        .events(id, 0, 100)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "foreach.expanded")
        .count();
    assert_eq!(expansion_events, 1);
    std::fs::write(
        directory.path().join("project/process.json"),
        "changed source after registration",
    )
    .unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    for _ in 0..5 {
        runtime.drive_pending().unwrap();
    }
    let done = runtime.inspect(id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(
        done.output.unwrap()["results"],
        json!({"items":[{"key":"second","output":{"value":2}},{"key":"first","output":{"value":1}}]})
    );
    assert_eq!(
        done.steps
            .iter()
            .find(|step| step.id == completed)
            .unwrap()
            .attempts,
        1
    );
    assert_eq!(
        runtime
            .events(id, 0, 100)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "foreach.expanded")
            .count(),
        1
    );
}

#[test]
fn pause_survives_restart_and_continue_preserves_completed_items() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, token, version) = setup(directory.path(), &process());
    let run = runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"one","value":1},{"id":"two","value":2}]},"request_key":"pause"})).unwrap();
    let id = run["run_id"].as_str().unwrap();
    runtime.drive_pending().unwrap();
    runtime.drive_pending().unwrap();
    let completed = runtime
        .inspect(id)
        .unwrap()
        .steps
        .into_iter()
        .find(|step| step.state == "succeeded")
        .unwrap()
        .id;
    assert_eq!(
        runtime
            .scoped_call(&token, "resume", json!({"action":"pause","run_id":id}))
            .unwrap()["state"],
        "paused"
    );
    drop(runtime);
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    assert_eq!(runtime.advance(id).unwrap().state, "paused");
    assert_eq!(
        runtime
            .scoped_call(&token, "resume", json!({"action":"continue","run_id":id}))
            .unwrap()["state"],
        "queued"
    );
    for _ in 0..5 {
        runtime.drive_pending().unwrap();
    }
    let done = runtime.inspect(id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(
        done.steps
            .iter()
            .find(|step| step.id == completed)
            .unwrap()
            .attempts,
        1
    );
}

#[test]
fn explicit_retry_is_bounded_and_never_replays_completed_siblings() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, token, version) = setup(directory.path(), &process());
    let run = runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"good","value":1},{"id":"bad"}]},"request_key":"retry"})).unwrap();
    let id = run["run_id"].as_str().unwrap();
    for _ in 0..6 {
        runtime.drive_pending().unwrap();
    }
    let failed = runtime.inspect(id).unwrap();
    assert_eq!(failed.state, "failed");
    let bad = failed
        .steps
        .iter()
        .find(|step| step.item_key == Some(json!("bad")))
        .unwrap()
        .id
        .clone();
    let good = failed
        .steps
        .iter()
        .find(|step| step.item_key == Some(json!("good")))
        .unwrap()
        .id
        .clone();
    assert_eq!(
        failed
            .steps
            .iter()
            .find(|step| step.id == bad)
            .unwrap()
            .attempts,
        2
    );
    assert_eq!(
        runtime
            .scoped_call(
                &token,
                "resume",
                json!({"action":"retry","run_id":id,"step_instance_id":good})
            )
            .unwrap_err()
            .code(),
        "INVALID_STATE"
    );
    for expected_attempts in 3..=5 {
        assert_eq!(
            runtime
                .scoped_call(
                    &token,
                    "resume",
                    json!({"action":"retry","run_id":id,"step_instance_id":bad})
                )
                .unwrap()["state"],
            "queued"
        );
        for _ in 0..3 {
            runtime.drive_pending().unwrap();
        }
        let run = runtime.inspect(id).unwrap();
        assert_eq!(run.state, "failed");
        assert_eq!(
            run.steps
                .iter()
                .find(|step| step.id == bad)
                .unwrap()
                .attempts,
            expected_attempts
        );
        assert_eq!(
            run.steps
                .iter()
                .find(|step| step.id == good)
                .unwrap()
                .attempts,
            1
        );
    }
    assert_eq!(
        runtime
            .scoped_call(
                &token,
                "resume",
                json!({"action":"retry","run_id":id,"step_instance_id":bad})
            )
            .unwrap_err()
            .code(),
        "LIMIT_EXCEEDED"
    );
}

#[test]
fn bounded_agent_items_accept_observed_results_while_paused_and_resume_in_order() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest = process();
    manifest["steps"][0]["kind"] = json!("agent");
    manifest["steps"][0]
        .as_object_mut()
        .unwrap()
        .remove("operation");
    manifest["steps"][0]["effect"] = json!("agent");
    manifest["steps"][0]["retry"] = json!({"max_attempts":1});
    manifest["steps"][0]["capability"] = json!("review");
    manifest["steps"][0]["instructions"] = json!("Return the item value after review");
    manifest["steps"][0]["result_schema"] = json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false});
    manifest["limits"]["max_agent_requests"] = json!(3);
    manifest["capabilities"] = json!([{"kind":"agent"}]);
    let (mut runtime, token, version) = setup(directory.path(), &manifest);
    let run=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"a","value":1},{"id":"b","value":2},{"id":"c","value":3}]},"request_key":"agents"})).unwrap();
    let id = run["run_id"].as_str().unwrap();
    for _ in 0..5 {
        runtime.drive_pending().unwrap();
    }
    let waiting = runtime.inspect(id).unwrap();
    assert_eq!(waiting.state, "waiting_for_agent");
    assert_eq!(
        waiting
            .steps
            .iter()
            .filter(|step| step.state == "waiting_for_agent")
            .count(),
        2
    );
    let request = waiting.request.unwrap();
    runtime
        .scoped_call(&token, "resume", json!({"action":"pause","run_id":id}))
        .unwrap();
    let submission = |request: &apr_runtime::AgentRequest| {
        json!({"protocol_version":"0.1","action":"submit","run_id":id,
        "request_id":request.request_id,"step_instance_id":request.step_instance_id,"generation":request.generation,
        "continuation_token":request.continuation_token,"submission_key":request.step_instance_id,
        "output":{"value":request.input["value"]["value"]},"provenance":{"source":"fixture"}})
    };
    let accepted = runtime
        .scoped_call(&token, "resume", submission(&request))
        .unwrap();
    assert_eq!(accepted["run"]["state"], "paused");
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    drop(runtime);
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    runtime
        .scoped_call(&token, "resume", json!({"action":"continue","run_id":id}))
        .unwrap();
    for _ in 0..10 {
        runtime.drive_pending().unwrap();
        let run = runtime.inspect(id).unwrap();
        if run.state == "succeeded" {
            break;
        }
        assert_ne!(run.state, "failed", "{:?}", run.error);
        if let Some(request) = run.request {
            runtime
                .scoped_call(&token, "resume", submission(&request))
                .unwrap();
        }
    }
    let done = runtime.inspect(id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(
        done.output.unwrap()["results"],
        json!({"items":[{"key":"a","output":{"value":1}},{"key":"b","output":{"value":2}},{"key":"c","output":{"value":3}}]})
    );
    assert_eq!(
        runtime
            .events(id, 0, 100)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "handoff.created")
            .count(),
        3
    );
}

#[test]
fn invalid_expansions_never_dispatch_and_empty_expansion_is_complete() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, token, version) = setup(directory.path(), &process());
    for (index, items) in [
        json!([{"id":"same","value":1},{"id":"same","value":2}]),
        json!([{"id":1,"value":1},{"id":2,"value":2},{"id":3,"value":3},{"id":4,"value":4}]),
        json!([{"value":1}]),
        json!([{"id":false,"value":1}]),
        json!({"id":"object"}),
    ]
    .into_iter()
    .enumerate()
    {
        let run=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":items},"request_key":format!("invalid-{index}")})).unwrap();
        let id = run["run_id"].as_str().unwrap();
        runtime.drive_pending().unwrap();
        let failed = runtime.inspect(id).unwrap();
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.steps.len(), 1);
        assert!(
            !runtime
                .events(id, 0, 100)
                .unwrap()
                .iter()
                .any(|event| event.kind == "step.started" || event.kind == "foreach.expanded")
        );
    }
    let run=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[]},"request_key":"empty"})).unwrap();
    let id = run["run_id"].as_str().unwrap();
    for _ in 0..3 {
        runtime.drive_pending().unwrap();
    }
    let done = runtime.inspect(id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(done.output.unwrap()["results"], json!({"items":[]}));
}

#[test]
fn foreach_literal_artifacts_require_the_assignment_grant() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, token, _) = setup(directory.path(), &process());
    let secret_file = directory.path().join("local-only.txt");
    std::fs::write(&secret_file, "private local artifact").unwrap();
    let reference = runtime
        .import_artifact(&secret_file, "text/plain", 1024)
        .unwrap();
    let mut manifest = process();
    manifest["steps"][0]["foreach"]["items"] =
        json!({"literal":[{"id":"foreign","value":reference}]});
    std::fs::write(
        directory.path().join("project/foreign.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .scoped_call(
                &token,
                "manage",
                json!({"action":"process.register","path":"foreign.json"})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    assert_eq!(runtime.scoped_call(&token,"manage",json!({"action":"process.test","path":"foreign.json","input":{},"request_key":"foreign"})).unwrap_err().code(),"FORBIDDEN");
}

#[test]
fn cancelled_expansion_cannot_continue_or_retry() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, token, version) = setup(directory.path(), &process());
    let run=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"one","value":1},{"id":"two","value":2}]},"request_key":"cancel"})).unwrap();
    let id = run["run_id"].as_str().unwrap();
    runtime.drive_pending().unwrap();
    let run = runtime
        .scoped_call(&token, "resume", json!({"action":"cancel","run_id":id}))
        .unwrap();
    assert_eq!(run["state"], "cancelled");
    assert_eq!(
        runtime
            .scoped_call(&token, "resume", json!({"action":"continue","run_id":id}))
            .unwrap()["state"],
        "cancelled"
    );
    assert_eq!(
        runtime
            .scoped_call(
                &token,
                "resume",
                json!({"action":"retry","run_id":id,"step_instance_id":"each"})
            )
            .unwrap_err()
            .code(),
        "INVALID_STATE"
    );
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    assert_eq!(
        runtime
            .inspect(id)
            .unwrap()
            .steps
            .iter()
            .map(|step| step.attempts)
            .sum::<u32>(),
        0
    );
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn committed_expansion_recovers_after_sigkill_without_duplicate_items() {
    use std::os::unix::process::ExitStatusExt;
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, token, version) = setup(directory.path(), &process());
    let queued=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"one","value":1},{"id":"two","value":2}]},"request_key":"kill"})).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    drop(runtime);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "expansion_crash_probe", "--nocapture"])
        .env("APR_FOREACH_TEST_STATE", directory.path().join("state"))
        .env("APR_TEST_CRASH_AT", "foreach_expanded:each")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9));
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    for _ in 0..5 {
        runtime.drive_pending().unwrap();
    }
    let done = runtime.inspect(id).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(done.steps.len(), 3);
    assert_eq!(
        runtime
            .events(id, 0, 100)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "foreach.expanded")
            .count(),
        1
    );
    assert_eq!(done.steps.iter().map(|step| step.attempts).sum::<u32>(), 2);
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn expansion_crash_probe() {
    if let Some(root) = std::env::var_os("APR_FOREACH_TEST_STATE") {
        Runtime::open(root).unwrap().drive_pending().unwrap();
        panic!("expansion crash point did not fire");
    }
}
