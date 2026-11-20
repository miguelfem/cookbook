use apr_runtime::Runtime;
use serde_json::{Value, json};

fn routed_manifest() -> Value {
    json!({"schema_version":"0.2","name":"route.fixture","description":"Choose a declared branch",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},"capabilities":[{"kind":"builtin"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1000000,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[
            {"id":"gate","kind":"decision","effect":"pure","needs":[],"inputs":{},"retry":{"max_attempts":1},
                "decision":{"source":"rules","routes":["yes","no"],"cases":[{"value":{"input":"/accept"},"equals":true,"route":"yes"}],"default":"no"}},
            {"id":"yes","kind":"builtin","operation":"json.select","effect":"pure","needs":["gate"],"retry":{"max_attempts":1},"when":{"decision":"gate","route":"yes"},"inputs":{"value":{"literal":{"approved":true}},"pointer":{"literal":""}}},
            {"id":"no","kind":"builtin","operation":"json.select","effect":"pure","needs":["gate"],"retry":{"max_attempts":1},"when":{"decision":"gate","route":"no"},"inputs":{"value":{"literal":{"approved":false}},"pointer":{"literal":""}}},
            {"id":"merge","kind":"join","effect":"pure","needs":["gate","yes","no"],"retry":{"max_attempts":1},"inputs":{},"join":{"decision":"gate","branches":{"yes":{"step":"yes","pointer":"/value"},"no":{"step":"no","pointer":"/value"}}}}
        ],"output":{"result":{"step":"merge","pointer":""}}})
}

#[test]
fn committed_decisions_skip_unselected_work_and_join_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = apr_core::admit_value(&routed_manifest()).unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("process.json"), &manifest.canonical_bytes).unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let p = runtime.attach_project(&project).unwrap();
    let a = runtime
        .create_assignment(p["project_id"].as_str().unwrap(), "Routing")
        .unwrap();
    let attachment = runtime
        .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = attachment["token"].as_str().unwrap();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )
        .unwrap();
    let run=runtime.scoped_call(token,"run",json!({"mode":"start","process_ref":manifest.version,"input":{"accept":true},"request_key":"route"})).unwrap();
    let id = run["run_id"].as_str().unwrap();
    runtime.drive_pending().unwrap();
    let first = runtime.inspect(id).unwrap();
    assert_ne!(first.state, "failed", "{:?}", first.error);
    assert_eq!(
        first
            .steps
            .iter()
            .find(|step| step.id == "gate")
            .unwrap()
            .state,
        "succeeded"
    );
    drop(runtime);
    let mut runtime = Runtime::open(&state).unwrap();
    for _ in 0..8 {
        runtime.drive_pending().unwrap();
    }
    let done = runtime.inspect(id).unwrap();
    assert_eq!(done.state, "succeeded", "{:?}", done.error);
    assert_eq!(
        done.output.unwrap(),
        json!({"result":{"route":"yes","output":{"approved":true}}})
    );
    let skipped = done.steps.iter().find(|step| step.id == "no").unwrap();
    assert_eq!(skipped.state, "skipped");
    assert_eq!(skipped.attempts, 0);
    assert_eq!(
        runtime
            .events(id, 0, 100)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "decision.committed")
            .count(),
        1
    );
}

fn child_manifest() -> Value {
    let mut child = routed_manifest();
    child["schema_version"] = json!("0.1");
    child["name"] = json!("child.fixture");
    child["config"] = json!({"schema":{"type":"object","required":["label"],"properties":{"label":{"type":"string"}},"additionalProperties":false},"defaults":{"label":"default"},"run_override_keys":[]});
    child["steps"] = json!([{"id":"select","kind":"builtin","operation":"json.select","effect":"pure","needs":[],"retry":{"max_attempts":1},"inputs":{"value":{"config":"/label"},"pointer":{"literal":""}}}]);
    child["output"] = json!({"value":{"step":"select","pointer":"/value"}});
    child
}

fn parent_manifest(child_ref: &str) -> Value {
    let mut parent = routed_manifest();
    parent["name"] = json!("parent.fixture");
    parent["steps"] = json!([{"id":"invoke","kind":"call","effect":"control","needs":[],"retry":{"max_attempts":1},"inputs":{},"call":{"process_ref":child_ref,"output_pointer":""}}]);
    parent["output"] = json!({"child":{"step":"invoke","pointer":""}});
    parent
}

#[test]
fn child_creation_pins_configuration_and_reuses_its_identity_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let child = apr_core::admit_value(&child_manifest()).unwrap();
    let parent = apr_core::admit_value(&parent_manifest(&child.version)).unwrap();
    std::fs::write(project.join("child.json"), &child.canonical_bytes).unwrap();
    std::fs::write(project.join("parent.json"), &parent.canonical_bytes).unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let p = runtime.attach_project(&project).unwrap();
    let a = runtime
        .create_assignment(p["project_id"].as_str().unwrap(), "Children")
        .unwrap();
    let attachment = runtime
        .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
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
    runtime.scoped_call(token,"manage",json!({"action":"config.set","process_ref":child.version,"values":{"label":"accepted"}})).unwrap();
    let start =
        json!({"mode":"start","process_ref":parent.version,"input":{},"request_key":"root"});
    let queued = runtime.scoped_call(token, "run", start.clone()).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    runtime.scoped_call(token,"manage",json!({"action":"config.set","process_ref":child.version,"values":{"label":"changed later"}})).unwrap();
    runtime.drive_pending().unwrap();
    let waiting = runtime
        .scoped_call(token, "inspect", json!({"view":"run","run_id":id}))
        .unwrap();
    assert_ne!(waiting["state"], "failed", "{waiting}");
    let child_id = waiting["steps"][0]["child_run_id"]
        .as_str()
        .expect("parent must expose its recorded child identity")
        .to_owned();
    drop(runtime);
    let mut runtime = Runtime::open(&state).unwrap();
    for _ in 0..10 {
        runtime.drive_pending().unwrap();
    }
    let done = runtime.inspect(id).unwrap();
    assert_eq!(done.state, "succeeded", "{:?}", done.error);
    assert_eq!(done.output.unwrap(), json!({"child":{"value":"accepted"}}));
    assert_eq!(
        runtime
            .scoped_call(token, "inspect", json!({"view":"config","run_id":child_id}))
            .unwrap()["config"],
        json!({"label":"accepted"})
    );
    let replay = runtime.scoped_call(token, "run", start).unwrap();
    assert_eq!(replay["steps"][0]["child_run_id"], child_id);
    assert_eq!(
        runtime
            .events(id, 0, 100)
            .unwrap()
            .iter()
            .filter(|event| event.kind == "child.created")
            .count(),
        1
    );
}

fn setup_pair(
    directory: &std::path::Path,
    child: &Value,
    parent: &Value,
) -> (Runtime, String, String) {
    let project = directory.join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("child.json"),
        serde_json::to_vec(child).unwrap(),
    )
    .unwrap();
    std::fs::write(
        project.join("parent.json"),
        serde_json::to_vec(parent).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.join("state")).unwrap();
    let p = runtime.attach_project(&project).unwrap();
    let a = runtime
        .create_assignment(p["project_id"].as_str().unwrap(), "Tree")
        .unwrap();
    let attachment = runtime
        .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let token = attachment["token"].as_str().unwrap().to_owned();
    for path in ["child.json", "parent.json"] {
        runtime
            .scoped_call(
                &token,
                "manage",
                json!({"action":"process.register","path":path}),
            )
            .unwrap();
    }
    (
        runtime,
        token,
        apr_core::admit_value(parent).unwrap().version,
    )
}

fn agent_child() -> Value {
    let mut child = child_manifest();
    child["limits"]["max_agent_requests"] = json!(1);
    child["steps"] = json!([{"id":"ask","kind":"agent","effect":"agent","needs":[],"retry":{"max_attempts":1},"inputs":{},"capability":"review","instructions":"Return a reviewed result","result_schema":{"type":"object"}}]);
    child["output"] = json!({"result":{"step":"ask","pointer":""}});
    child
}

#[test]
fn parent_pause_holds_children_across_restart_and_cancel_invalidates_their_handoffs() {
    let directory = tempfile::tempdir().unwrap();
    let child = agent_child();
    let mut parent = parent_manifest(&apr_core::admit_value(&child).unwrap().version);
    parent["limits"]["max_agent_requests"] = json!(1);
    let (mut runtime, token, version) = setup_pair(directory.path(), &child, &parent);
    let queued = runtime
        .scoped_call(
            &token,
            "run",
            json!({"mode":"start","process_ref":version,"input":{},"request_key":"controls"}),
        )
        .unwrap();
    let id = queued["run_id"].as_str().unwrap();
    runtime.drive_pending().unwrap();
    let child_id = runtime.inspect(id).unwrap().steps[0]
        .child_run_id
        .clone()
        .unwrap();
    runtime
        .scoped_call(&token, "resume", json!({"action":"pause","run_id":id}))
        .unwrap();
    drop(runtime);
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    assert_eq!(runtime.advance(&child_id).unwrap().completed_steps, 0);
    assert_eq!(runtime.inspect(&child_id).unwrap().steps[0].attempts, 0);
    runtime
        .scoped_call(&token, "resume", json!({"action":"continue","run_id":id}))
        .unwrap();
    for _ in 0..4 {
        runtime.drive_pending().unwrap();
    }
    let waiting = runtime.inspect(&child_id).unwrap();
    let request = waiting.request.unwrap();
    assert_eq!(waiting.root_run_id.as_deref(), Some(id));
    assert_eq!(waiting.parent_run_id.as_deref(), Some(id));
    assert_eq!(waiting.parent_step_id.as_deref(), Some("invoke"));
    runtime
        .scoped_call(&token, "resume", json!({"action":"cancel","run_id":id}))
        .unwrap();
    assert_eq!(runtime.inspect(&child_id).unwrap().state, "cancelled");
    let stale=runtime.scoped_call(&token,"resume",json!({"protocol_version":"0.1","action":"submit","run_id":child_id,"request_id":request.request_id,"step_instance_id":request.step_instance_id,"generation":request.generation,"submission_key":"observed","continuation_token":request.continuation_token,"output":{},"provenance":{"source":"fixture"}})).unwrap_err();
    assert_eq!(stale.code(), "STALE_HANDOFF");
}

#[test]
fn nested_artifact_publication_cannot_exceed_the_root_resource_ceiling() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = child_manifest();
    child["steps"][0]["operation"] = json!("json.write");
    child["steps"][0]["inputs"] = json!({"value":{"literal":"x".repeat(64)}});
    child["output"] = json!({"artifact":{"step":"select","pointer":"/artifact"}});
    let mut parent = parent_manifest(&apr_core::admit_value(&child).unwrap().version);
    parent["limits"]["max_artifact_bytes"] = json!(16);
    let (mut runtime, token, version) = setup_pair(directory.path(), &child, &parent);
    let queued = runtime
        .scoped_call(
            &token,
            "run",
            json!({"mode":"start","process_ref":version,"input":{},"request_key":"caps"}),
        )
        .unwrap();
    let id = queued["run_id"].as_str().unwrap();
    for _ in 0..8 {
        runtime.drive_pending().unwrap();
    }
    let root = runtime.inspect(id).unwrap();
    let child = runtime
        .inspect(root.steps[0].child_run_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        child.state, "failed",
        "child must enforce the inherited cap before publishing"
    );
    assert!(
        child
            .error
            .as_ref()
            .is_some_and(|error| error.contains("16")),
        "{:?}",
        child.error
    );
    assert_eq!(root.state, "failed");
}

fn foreach_parent(child_ref: &str) -> Value {
    let mut parent = parent_manifest(child_ref);
    parent["steps"][0]["foreach"] =
        json!({"items":{"input":"/items"},"key":"/id","max_items":2,"concurrency":2});
    parent
}

#[test]
fn shared_agent_and_static_step_budgets_include_foreach_children() {
    let directory = tempfile::tempdir().unwrap();
    let child = agent_child();
    let child_ref = apr_core::admit_value(&child).unwrap().version;
    let mut parent = foreach_parent(&child_ref);
    parent["limits"]["max_agent_requests"] = json!(1);
    let (mut runtime, token, version) = setup_pair(directory.path(), &child, &parent);
    let queued=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"items":[{"id":"one"},{"id":"two"}]},"request_key":"shared"})).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    for _ in 0..12 {
        runtime.drive_pending().unwrap();
    }
    let root = runtime.inspect(id).unwrap();
    assert_eq!(root.state, "failed");
    let children: Vec<_> = root
        .steps
        .iter()
        .filter_map(|step| step.child_run_id.as_ref())
        .collect();
    assert_eq!(children.len(), 2);
    let requests: usize = children
        .iter()
        .map(|child| {
            runtime
                .events(child, 0, 100)
                .unwrap()
                .iter()
                .filter(|event| event.kind == "handoff.created")
                .count()
        })
        .sum();
    assert_eq!(
        requests, 1,
        "a child must not reset its root's caller budget"
    );
    runtime
        .scoped_call(&token, "resume", json!({"action":"cancel","run_id":id}))
        .unwrap();
    assert!(
        children
            .iter()
            .all(|child| runtime.inspect(child).unwrap().request.is_none())
    );

    let bounded = tempfile::tempdir().unwrap();
    parent["limits"]["max_step_instances"] = json!(4);
    let (mut runtime, token, version) = setup_pair(bounded.path(), &child, &parent);
    let error=runtime.scoped_call(&token,"run",json!({"mode":"preflight","process_ref":version,"input":{"items":[{"id":"one"}]},"request_key":"too-many"})).unwrap_err();
    assert_eq!(
        error.code(),
        "LIMIT_EXCEEDED",
        "static declared expansion must include all child steps"
    );
    assert!(
        runtime
            .scoped_call(&token, "inspect", json!({"view":"runs"}))
            .unwrap()["runs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn explicit_child_retry_reopens_only_its_failed_calls_and_cannot_revive_cancellation() {
    for cancelled in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = child_manifest();
        child["steps"][0]["operation"] = json!("json.write");
        child["steps"][0]["inputs"] = json!({"value":{"literal":"retry"}});
        child["output"] = json!({"artifact":{"step":"select","pointer":"/artifact"}});
        let mut parent = parent_manifest(&apr_core::admit_value(&child).unwrap().version);
        parent["steps"][0]["needs"] = json!(["before"]);
        parent["steps"].as_array_mut().unwrap().insert(0,json!({"id":"before","kind":"builtin","operation":"json.select","effect":"pure","needs":[],"retry":{"max_attempts":1},"inputs":{"value":{"literal":true},"pointer":{"literal":""}}}));
        let (mut runtime, token, version) = setup_pair(directory.path(), &child, &parent);
        runtime.set_artifact_quota(Some(1)).unwrap();
        let queued = runtime
            .scoped_call(
                &token,
                "run",
                json!({"mode":"start","process_ref":version,"input":{},"request_key":"retry-tree"}),
            )
            .unwrap();
        let id = queued["run_id"].as_str().unwrap();
        for _ in 0..8 {
            runtime.drive_pending().unwrap();
        }
        let root = runtime.inspect(id).unwrap();
        assert_eq!(root.state, "failed");
        let child_id = root.steps[1].child_run_id.clone().unwrap();
        if cancelled {
            runtime
                .scoped_call(&token, "resume", json!({"action":"cancel","run_id":id}))
                .unwrap();
        }
        drop(runtime);
        let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
        runtime.set_artifact_quota(None).unwrap();
        let retry = runtime.scoped_call(
            &token,
            "resume",
            json!({"action":"retry","run_id":child_id,"step_instance_id":"select"}),
        );
        if cancelled {
            assert_eq!(retry.unwrap_err().code(), "INVALID_STATE");
            assert_eq!(runtime.drive_pending().unwrap(), 0);
            continue;
        }
        retry.unwrap();
        for _ in 0..8 {
            runtime.drive_pending().unwrap();
        }
        let root = runtime.inspect(id).unwrap();
        assert_eq!(root.state, "succeeded", "{:?}", root.error);
        assert_eq!(root.steps[0].attempts, 1);
        assert_eq!(root.steps[1].attempts, 1);
        assert_eq!(root.steps[1].child_run_id.as_ref(), Some(&child_id));
        assert_eq!(runtime.inspect(&child_id).unwrap().steps[0].attempts, 2);
    }
}

#[test]
fn nested_handoff_renewal_keeps_one_attempt_and_observed_results_wait_for_parent_continue() {
    let directory = tempfile::tempdir().unwrap();
    let child = agent_child();
    let mut parent = parent_manifest(&apr_core::admit_value(&child).unwrap().version);
    parent["limits"]["max_agent_requests"] = json!(1);
    let (mut runtime, token, version) = setup_pair(directory.path(), &child, &parent);
    let queued = runtime
        .scoped_call(
            &token,
            "run",
            json!({"mode":"start","process_ref":version,"input":{},"request_key":"handoff"}),
        )
        .unwrap();
    let id = queued["run_id"].as_str().unwrap();
    for _ in 0..5 {
        runtime.drive_pending().unwrap();
    }
    let child_id = runtime.inspect(id).unwrap().steps[0]
        .child_run_id
        .clone()
        .unwrap();
    let waiting = runtime.inspect(&child_id).unwrap();
    assert_eq!(waiting.steps[0].attempts, 1);
    let renewed = runtime.renew_handoff(&child_id).unwrap();
    assert_eq!(renewed.steps[0].attempts, 1);
    let request = renewed.request.unwrap();
    assert_eq!(request.generation, 2);
    let history = runtime
        .scoped_call(&token, "inspect", json!({"view":"runs"}))
        .unwrap();
    let child_history = history["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["run_id"] == child_id)
        .unwrap();
    assert_eq!(child_history["parent_run_id"], id);
    assert_eq!(child_history["root_run_id"], id);
    runtime
        .scoped_call(&token, "resume", json!({"action":"pause","run_id":id}))
        .unwrap();
    assert!(runtime.inspect(&child_id).unwrap().request.is_none());
    let active = runtime
        .scoped_call(
            &token,
            "run",
            json!({"mode":"start","process_ref":version,"input":{},"request_key":"other-handoff"}),
        )
        .unwrap();
    for _ in 0..5 {
        runtime.drive_pending().unwrap();
    }
    let visible = runtime
        .scoped_call(&token, "inspect", json!({"view":"pending","limit":1}))
        .unwrap();
    assert_eq!(
        visible["runs"].as_array().unwrap().len(),
        1,
        "a hidden paused child must not consume the pending page"
    );
    assert_eq!(visible["runs"][0]["root_run_id"], active["run_id"]);
    let submission = json!({"protocol_version":"0.1","action":"submit","run_id":child_id,"request_id":request.request_id,"step_instance_id":request.step_instance_id,"generation":request.generation,"submission_key":"done","continuation_token":request.continuation_token,"output":{"reviewed":true},"provenance":{"source":"fixture"}});
    let receipt = runtime
        .scoped_call(&token, "resume", submission.clone())
        .unwrap()["receipt"]
        .clone();
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    drop(runtime);
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    runtime
        .scoped_call(&token, "resume", json!({"action":"continue","run_id":id}))
        .unwrap();
    for _ in 0..8 {
        runtime.drive_pending().unwrap();
    }
    assert_eq!(runtime.inspect(id).unwrap().state, "succeeded");
    assert_eq!(runtime.inspect(&child_id).unwrap().steps[0].attempts, 1);
    assert_eq!(
        runtime.scoped_call(&token, "resume", submission).unwrap()["receipt"],
        receipt
    );
}

#[test]
fn child_catalog_and_run_artifacts_never_inherit_another_assignments_authority() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = child_manifest();
    child["steps"][0]["operation"] = json!("json.write");
    child["steps"][0]["inputs"] = json!({"value":{"literal":{"private":"source"}}});
    child["output"] = json!({"artifact":{"step":"select","pointer":"/artifact"}});
    let parent = parent_manifest(&apr_core::admit_value(&child).unwrap().version);
    let (mut runtime, token, version) = setup_pair(directory.path(), &child, &parent);
    let p = runtime
        .attach_project(&directory.path().join("project"))
        .unwrap();
    let a = runtime
        .create_assignment(p["project_id"].as_str().unwrap(), "Other")
        .unwrap();
    let attachment = runtime
        .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let other = attachment["token"].as_str().unwrap();
    runtime
        .scoped_call(
            other,
            "manage",
            json!({"action":"process.register","path":"parent.json"}),
        )
        .unwrap();
    let args = json!({"mode":"start","process_ref":version,"input":{},"request_key":"same"});
    assert_eq!(
        runtime
            .scoped_call(other, "run", args.clone())
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    let queued = runtime.scoped_call(&token, "run", args).unwrap();
    let id = queued["run_id"].as_str().unwrap();
    for _ in 0..8 {
        runtime.drive_pending().unwrap();
    }
    let root = runtime.inspect(id).unwrap();
    assert_eq!(root.state, "succeeded");
    let child_id = root.steps[0].child_run_id.clone().unwrap();
    for args in [
        json!({"view":"run","run_id":child_id}),
        json!({"view":"events","run_id":child_id}),
        json!({"view":"config","run_id":child_id}),
    ] {
        assert_eq!(
            runtime
                .scoped_call(other, "inspect", args)
                .unwrap_err()
                .code(),
            "FORBIDDEN"
        );
    }
    assert_eq!(
        runtime
            .scoped_call(
                other,
                "resume",
                json!({"action":"cancel","run_id":child_id})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    assert_eq!(
        runtime
            .scoped_call(
                other,
                "inspect",
                json!({"view":"artifact","reference":root.output.unwrap()["child"]["artifact"]})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    assert!(
        runtime
            .scoped_call(other, "inspect", json!({"view":"runs"}))
            .unwrap()["runs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn child_creation_and_join_survive_sigkill_with_one_recorded_identity() {
    use std::os::unix::process::ExitStatusExt;
    for fault in [
        "child_created:invoke",
        "child_joined:invoke",
        "routing_committed:gate",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let child = child_manifest();
        let parent = if fault.starts_with("routing_") {
            routed_manifest()
        } else {
            parent_manifest(&apr_core::admit_value(&child).unwrap().version)
        };
        let (mut runtime, token, version) = setup_pair(directory.path(), &child, &parent);
        let queued=runtime.scoped_call(&token,"run",json!({"mode":"start","process_ref":version,"input":{"accept":true},"request_key":"crash"})).unwrap();
        let id = queued["run_id"].as_str().unwrap();
        drop(runtime);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "routing_crash_probe", "--nocapture"])
            .env("APR_ROUTING_TEST_STATE", directory.path().join("state"))
            .env("APR_TEST_CRASH_AT", fault)
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(9), "{fault}");
        let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
        for _ in 0..12 {
            runtime.drive_pending().unwrap();
        }
        let root = runtime.inspect(id).unwrap();
        assert_eq!(root.state, "succeeded", "{fault}: {:?}", root.error);
        let event = if fault.starts_with("routing_") {
            "decision.committed"
        } else {
            "child.created"
        };
        assert_eq!(
            runtime
                .events(id, 0, 100)
                .unwrap()
                .iter()
                .filter(|entry| entry.kind == event)
                .count(),
            1,
            "{fault}"
        );
        assert_eq!(root.steps[0].attempts, 1, "{fault}");
        if let Some(child) = &root.steps[0].child_run_id {
            assert_eq!(runtime.inspect(child).unwrap().steps[0].attempts, 1);
        }
    }
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn routing_crash_probe() {
    if let Some(root) = std::env::var_os("APR_ROUTING_TEST_STATE") {
        let mut runtime = Runtime::open(root).unwrap();
        for _ in 0..50 {
            runtime.drive_pending().unwrap();
        }
        panic!("routing crash point did not fire");
    }
}
