use apr_runtime::Runtime;
use serde_json::{Value, json};

fn process() -> Value {
    json!({"schema_version":"0.1","name":"configured.snapshot","description":"Snapshot frozen ordinary settings",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","additionalProperties":false,"required":["locale","quality","layout"],
            "properties":{"locale":{"type":"string"},"quality":{"enum":["draft","standard","final"]},
                "layout":{"type":"object","additionalProperties":false,"required":["width","height"],
                    "properties":{"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1}}}}},
            "defaults":{"locale":"en","quality":"draft","layout":{"width":600,"height":800}},
            "run_override_keys":["locale","quality"]},
        "capabilities":[{"kind":"builtin"},{"kind":"artifacts"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[{"id":"save","kind":"builtin","operation":"json.write","needs":[],"effect":"pure",
            "retry":{"max_attempts":2},"inputs":{"value":{"config":""}}}],
        "output":{"settings":{"config":""},"artifact":{"step":"save","pointer":"/artifact"}}})
}

fn setup(runtime: &mut Runtime, project: &std::path::Path) -> (String, Value, String) {
    std::fs::create_dir_all(project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&process()).unwrap(),
    )
    .unwrap();
    let project_id = runtime.attach_project(project).unwrap()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let assignment = runtime
        .create_assignment(&project_id, "Config fixture")
        .unwrap();
    let attachment = runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
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
    (project_id, attachment, version)
}

#[test]
fn precedence_is_validated_and_frozen_before_work_or_later_configuration_changes() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let (project, attachment, version) = setup(&mut runtime, &directory.path().join("project"));
    let token = attachment["token"].as_str().unwrap();
    runtime
        .set_project_config(
            &project,
            &version,
            json!({"quality":"standard","layout":{"width":300,"height":400}}),
        )
        .unwrap();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"config.set","process_ref":version,
        "values":{"locale":"fr","layout":{"width":100,"height":200}}}),
        )
        .unwrap();
    let request = json!({"process_ref":version,"input":{},"request_key":"frozen","mode":"start",
        "config_overrides":{"locale":"es"}});
    let queued = runtime.scoped_call(token, "run", request.clone()).unwrap();
    assert_eq!(queued["state"], "queued");
    let expected = json!({"locale":"es","quality":"standard","layout":{"width":100,"height":200}});
    let frozen = runtime
        .scoped_call(
            token,
            "inspect",
            json!({"view":"config","run_id":queued["run_id"]}),
        )
        .unwrap();
    assert_eq!(frozen["config"], expected);
    assert_eq!(frozen["config_overrides"], json!({"locale":"es"}));
    runtime
        .set_project_config(&project, &version, json!({"quality":"final"}))
        .unwrap();
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"config.set","process_ref":version,"values":{}}),
        )
        .unwrap();
    assert_eq!(
        runtime.scoped_call(token, "run", request).unwrap()["run_id"],
        queued["run_id"]
    );
    drop(runtime);

    let mut runtime = Runtime::open(&state).unwrap();
    runtime.drive_pending().unwrap();
    let done = runtime.inspect(queued["run_id"].as_str().unwrap()).unwrap();
    assert_eq!(done.state, "succeeded");
    assert_eq!(done.output.unwrap()["settings"], expected);
    let current = runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"config.read","process_ref":version}),
        )
        .unwrap();
    assert_eq!(
        current["config"],
        json!({"locale":"en","quality":"final","layout":{"width":600,"height":800}})
    );
}

#[test]
fn a_draft_fixture_runs_without_publishing_and_reuses_its_frozen_start_identity() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (_, attachment, _) = setup(&mut runtime, &project);
    let token = attachment["token"].as_str().unwrap();
    let mut draft = process();
    draft["name"] = json!("draft.snapshot");
    std::fs::write(
        project.join("draft.json"),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    let args =
        json!({"action":"process.test","path":"draft.json","input":{},"request_key":"fixture"});
    let queued = runtime.scoped_call(token, "manage", args.clone()).unwrap();
    assert_eq!(queued["state"], "queued");
    assert_eq!(queued["fixture"], true);
    assert_eq!(
        runtime.scoped_call(token, "manage", args.clone()).unwrap()["run_id"],
        queued["run_id"]
    );
    assert_eq!(
        runtime
            .scoped_call(token, "discover", json!({"query":"draft.snapshot"}))
            .unwrap()["processes"],
        json!([])
    );
    runtime.drive_pending().unwrap();
    let done = runtime
        .scoped_call(
            token,
            "inspect",
            json!({"view":"run","run_id":queued["run_id"]}),
        )
        .unwrap();
    assert_eq!(done["state"], "succeeded");
    assert_eq!(done["fixture"], true);
    draft["description"] = json!("Changed after fixture submission");
    std::fs::write(
        project.join("draft.json"),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .scoped_call(token, "manage", args)
            .unwrap_err()
            .code(),
        "REQUEST_CONFLICT"
    );
}

#[test]
fn promotion_requires_an_owner_grant_and_copies_only_the_pinned_version() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (project_id, source, version) = setup(&mut runtime, &project);
    let source_assignment = source["assignment_id"].as_str().unwrap();
    let source_token = source["token"].as_str().unwrap();
    let target = runtime
        .create_assignment(&project_id, "Promotion destination")
        .unwrap();
    let target_id = target["assignment_id"].as_str().unwrap();
    let target_attachment = runtime.create_attachment(target_id, "run").unwrap();
    let target_token = target_attachment["token"].as_str().unwrap();
    let promotion =
        json!({"action":"process.promote","process_ref":version,"target_assignment_id":target_id});
    assert_eq!(
        runtime
            .scoped_call(source_token, "manage", promotion.clone())
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    runtime
        .set_promotion_grant(source_assignment, target_id, true)
        .unwrap();
    let mut changed = process();
    changed["config"]["defaults"]["locale"] = json!("changed-source");
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let copied = runtime
        .scoped_call(source_token, "manage", promotion.clone())
        .unwrap();
    assert_eq!(copied["process_ref"], version);
    assert_eq!(copied["promoted"], true);
    let queued = runtime
        .scoped_call(
            target_token,
            "run",
            json!({"process_ref":version,"input":{},"request_key":"promoted","mode":"start"}),
        )
        .unwrap();
    runtime.drive_pending().unwrap();
    assert_eq!(
        runtime
            .inspect(queued["run_id"].as_str().unwrap())
            .unwrap()
            .output
            .unwrap()["settings"]["locale"],
        "en"
    );
    runtime
        .set_promotion_grant(source_assignment, target_id, false)
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(source_token, "manage", promotion)
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    let (_, other, _) = setup(&mut runtime, &directory.path().join("other-project"));
    assert_eq!(
        runtime
            .set_promotion_grant(
                source_assignment,
                other["assignment_id"].as_str().unwrap(),
                true
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
}

#[test]
fn invalid_overrides_and_assignment_spoofing_cannot_change_settings_or_start_work() {
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let (project_id, attachment, version) = setup(&mut runtime, &directory.path().join("project"));
    let token = attachment["token"].as_str().unwrap();
    let initial = runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"config.read","process_ref":version}),
        )
        .unwrap();
    assert_eq!(runtime.scoped_call(token,"manage",json!({"action":"config.set","process_ref":version,"values":{"layout":{"width":10}}})).unwrap_err().code(),"INVALID_INPUT");
    assert_eq!(runtime.scoped_call(token,"manage",json!({"action":"config.set","process_ref":version,"values":{"locale":"spoof"},"scope":"project"})).unwrap_err().code(),"INVALID_INPUT");
    assert_eq!(
        runtime
            .scoped_call(
                token,
                "manage",
                json!({"action":"config.read","process_ref":version})
            )
            .unwrap(),
        initial
    );
    for mode in ["preflight", "start"] {
        for values in [
            json!({"layout":{"width":10,"height":20}}),
            json!({"locale":4}),
            json!({"quality":"unsafe"}),
        ] {
            assert_eq!(runtime.scoped_call(token,"run",json!({"mode":mode,"process_ref":version,"input":{},"request_key":"denied","config_overrides":values})).unwrap_err().code(),"INVALID_INPUT");
        }
    }
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    let other = runtime
        .create_assignment(&project_id, "Separate settings")
        .unwrap();
    let other = runtime
        .create_attachment(other["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    let other_token = other["token"].as_str().unwrap();
    assert_eq!(
        runtime
            .scoped_call(
                other_token,
                "manage",
                json!({"action":"config.read","process_ref":version})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    runtime
        .scoped_call(
            other_token,
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )
        .unwrap();
    runtime.scoped_call(token,"manage",json!({"action":"config.set","process_ref":version,"values":{"locale":"private-to-assignment"}})).unwrap();
    assert_eq!(
        runtime
            .scoped_call(
                other_token,
                "manage",
                json!({"action":"config.read","process_ref":version})
            )
            .unwrap()["config"]["locale"],
        "en"
    );
    let run_only = runtime
        .create_attachment(attachment["assignment_id"].as_str().unwrap(), "run")
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(
                run_only["token"].as_str().unwrap(),
                "manage",
                json!({"action":"config.set","process_ref":version,"values":{}})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    let request = json!({"mode":"start","process_ref":version,"input":{},"request_key":"dedupe","config_overrides":{"locale":"es"}});
    runtime.scoped_call(token, "run", request.clone()).unwrap();
    let mut changed = request;
    changed["config_overrides"]["locale"] = json!("fr");
    assert_eq!(
        runtime
            .scoped_call(token, "run", changed)
            .unwrap_err()
            .code(),
        "REQUEST_CONFLICT"
    );
}
