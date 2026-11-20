use apr_runtime::Runtime;
use serde_json::{Value, json};

fn attach(runtime: &mut Runtime, project: &std::path::Path, name: &str, mode: &str) -> Value {
    let project = runtime.attach_project(project).unwrap();
    let assignment = runtime
        .create_assignment(project["project_id"].as_str().unwrap(), name)
        .unwrap();
    runtime
        .create_attachment(assignment["assignment_id"].as_str().unwrap(), mode)
        .unwrap()
}

#[test]
fn authority_is_bound_to_an_assignment_and_is_checked_after_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let attachment = attach(&mut runtime, &project, "First assignment", "author");
    let token = attachment["token"].as_str().unwrap();
    let discovered = runtime.scoped_call(token, "discover", json!({})).unwrap();
    assert_eq!(discovered["processes"], json!([]));
    assert_eq!(
        runtime
            .scoped_call("invalid", "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    runtime
        .revoke_attachment(attachment["attachment_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
}

fn review_process() -> Value {
    json!({"schema_version":"0.1","name":"scoped.review","description":"Review an assignment record",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"agent"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[{"id":"review","kind":"agent","needs":[],"effect":"agent","retry":{"max_attempts":1},
            "capability":"review","instructions":"Review the record","inputs":{"record":{"input":""}},
            "result_schema":{"type":"object"}}],"output":{"result":{"step":"review","pointer":""}}})
}

#[test]
fn catalog_rejects_schemas_and_static_handoffs_that_cannot_fit_a_detail_response() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let attachment = attach(&mut runtime, &project, "Detail bounds", "author");
    let token = attachment["token"].as_str().unwrap();

    for field in ["input_schema", "output_schema", "capabilities", "handoff"] {
        let mut process = review_process();
        let large = json!("x".repeat(70_000));
        match field {
            "input_schema" | "output_schema" => process[field]["description"] = large,
            "capabilities" => process["capabilities"][0]["description"] = large,
            "handoff" => process["steps"][0]["capability"] = large,
            _ => unreachable!(),
        }
        std::fs::write(
            project.join("process.json"),
            serde_json::to_vec(&process).unwrap(),
        )
        .unwrap();
        for action in ["process.validate", "process.register"] {
            let error = runtime
                .scoped_call(
                    token,
                    "manage",
                    json!({"action":action,"path":"process.json"}),
                )
                .expect_err("an unusable agent descriptor must fail before registration");
            assert_eq!(error.code(), "LIMIT_EXCEEDED", "{field}: {error}");
        }
    }
    assert_eq!(
        runtime.scoped_call(token, "discover", json!({})).unwrap()["processes"],
        json!([])
    );
}

#[test]
fn resolved_handoffs_include_capability_and_input_in_the_same_byte_budget() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut process = review_process();
    process["steps"][0]["capability"] = json!("r".repeat(16 * 1024));
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&process).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let attachment = attach(&mut runtime, &project, "Handoff bounds", "author");
    let token = attachment["token"].as_str().unwrap();
    let version = register(&mut runtime, token);

    // The JSON input is below 64 KiB, and each variable field fits separately.
    // Together the capability and UTF-8 input exceed the handoff detail budget.
    let oversized = runtime
        .scoped_call(
            token,
            "run",
            json!({"mode":"start","process_ref":version,
            "input":{"text":"🦀".repeat(10 * 1024)},"request_key":"oversized"}),
        )
        .unwrap();
    runtime.drive_pending().unwrap();
    let failed = runtime
        .inspect(oversized["run_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(failed.state, "failed");
    assert!(failed.request.is_none());
    assert!(
        runtime
            .events(&failed.run_id, 0, 100)
            .unwrap()
            .iter()
            .all(|event| event.kind != "handoff.created")
    );

    let input = json!({"text":"🦀".repeat(6 * 1024)});
    let queued = runtime
        .scoped_call(
            token,
            "run",
            json!({"mode":"start","process_ref":version,
            "input":input,"request_key":"fits"}),
        )
        .unwrap();
    runtime.drive_pending().unwrap();
    let waiting = runtime
        .scoped_call(
            token,
            "inspect",
            json!({"view":"run","run_id":queued["run_id"]}),
        )
        .unwrap();
    assert_eq!(waiting["state"], "waiting_for_agent");
    assert_eq!(
        waiting["request"]["capability"],
        process["steps"][0]["capability"]
    );
    assert_eq!(waiting["request"]["input"]["record"], input);
    assert!(serde_json::to_vec(&waiting).unwrap().len() < 64 * 1024 - 1024);
}

#[test]
fn one_step_worker_quanta_rotate_between_pending_runs() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut process = review_process();
    process["capabilities"] = json!([{"kind":"builtin"},{"kind":"artifacts"}]);
    process["limits"]["max_agent_requests"] = json!(0);
    process["steps"] = json!([
        {"id":"first","kind":"builtin","operation":"json.write","needs":[],
         "effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"input":""}}},
        {"id":"second","kind":"builtin","operation":"json.write","needs":["first"],
         "effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"step":"first","pointer":""}}}
    ]);
    process["output"] = json!({"artifact":{"step":"second","pointer":"/artifact"}});
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&process).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let attachment = attach(&mut runtime, &project, "Fair worker", "author");
    let token = attachment["token"].as_str().unwrap();
    let version = register(&mut runtime, token);
    let a = start(&mut runtime, token, &version, "first-run");
    let b = start(&mut runtime, token, &version, "second-run");
    let ids = [a["run_id"].as_str().unwrap(), b["run_id"].as_str().unwrap()];
    let mut cursor = None;

    assert!(runtime.drive_next_pending(&mut cursor).unwrap());
    let after_first = ids.map(|id| runtime.inspect(id).unwrap().completed_steps);
    assert_eq!(after_first.iter().sum::<usize>(), 1);
    assert!(runtime.drive_next_pending(&mut cursor).unwrap());
    assert_eq!(
        ids.map(|id| runtime.inspect(id).unwrap().completed_steps),
        [1, 1]
    );
    assert!(runtime.drive_next_pending(&mut cursor).unwrap());
    assert!(runtime.drive_next_pending(&mut cursor).unwrap());
    assert!(
        ids.iter()
            .all(|id| runtime.inspect(id).unwrap().state == "succeeded")
    );
    assert!(!runtime.drive_next_pending(&mut cursor).unwrap());
}

#[test]
fn run_history_pages_completed_and_pending_runs_without_crossing_assignments() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "History A", "author");
    let b = attach(&mut runtime, &project, "History B", "author");
    let ta = a["token"].as_str().unwrap();
    let tb = b["token"].as_str().unwrap();
    let version = register(&mut runtime, ta);
    register(&mut runtime, tb);
    let first = start(&mut runtime, ta, &version, "first");
    let second = start(&mut runtime, ta, &version, "second");
    let foreign = start(&mut runtime, tb, &version, "foreign");
    runtime
        .scoped_call(
            ta,
            "resume",
            json!({"action":"cancel","run_id":first["run_id"]}),
        )
        .unwrap();
    let page = runtime
        .scoped_call(ta, "inspect", json!({"view":"runs","limit":1}))
        .unwrap();
    assert_eq!(page["runs"].as_array().unwrap().len(), 1);
    assert_eq!(page["truncated"], true);
    let next = runtime
        .scoped_call(
            ta,
            "inspect",
            json!({"view":"runs","limit":1,"after":page["next_cursor"]}),
        )
        .unwrap();
    assert_eq!(next["truncated"], false);
    let mut ids = vec![
        page["runs"][0]["run_id"].clone(),
        next["runs"][0]["run_id"].clone(),
    ];
    ids.sort_by_key(|id| id.as_str().unwrap().to_owned());
    let mut expected = vec![first["run_id"].clone(), second["run_id"].clone()];
    expected.sort_by_key(|id| id.as_str().unwrap().to_owned());
    assert_eq!(ids, expected);
    assert!(!ids.contains(&foreign["run_id"]));
    assert!(page["runs"][0].get("request").is_none());
    assert!(page["runs"][0].get("output").is_none());
}

fn register(runtime: &mut Runtime, token: &str) -> String {
    runtime
        .scoped_call(
            token,
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )
        .unwrap()["process_ref"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn start(runtime: &mut Runtime, token: &str, version: &str, key: &str) -> Value {
    runtime
        .scoped_call(
            token,
            "run",
            json!({"mode":"start","process_ref":version,"input":{},"request_key":key}),
        )
        .unwrap()
}

fn submit(waiting: &Value, output: Value) -> Value {
    let request = &waiting["request"];
    json!({"protocol_version":"0.1","action":"submit","run_id":waiting["run_id"],"request_id":request["request_id"],
        "step_instance_id":request["step_instance_id"],"generation":request["generation"],"submission_key":"result",
        "continuation_token":request["continuation_token"],"output":output,"provenance":{"source":"manual"}})
}

#[test]
fn catalog_versions_run_ids_and_receipts_stay_inside_the_authenticated_assignment() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    let other_project = directory.path().join("other-project");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&other_project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        other_project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let b = attach(&mut runtime, &project, "B", "author");
    let c = attach(&mut runtime, &other_project, "C", "author");
    let ta = a["token"].as_str().unwrap();
    let tb = b["token"].as_str().unwrap();
    let tc = c["token"].as_str().unwrap();
    let version = register(&mut runtime, ta);
    for token in [tb, tc] {
        assert_eq!(
            runtime.scoped_call(token, "discover", json!({})).unwrap()["processes"],
            json!([])
        );
        assert_eq!(
            runtime
                .scoped_call(
                    token,
                    "discover",
                    json!({"process_ref":version,"detail":"schema"})
                )
                .unwrap_err()
                .code(),
            "FORBIDDEN"
        );
        assert_eq!(
            runtime
                .scoped_call(
                    token,
                    "run",
                    json!({"mode":"start","process_ref":version,"input":{},"request_key":"shared"})
                )
                .unwrap_err()
                .code(),
            "FORBIDDEN"
        );
    }
    assert_eq!(
        runtime
            .scoped_call(
                ta,
                "run",
                json!({"mode":"preflight","process_ref":version,"input":{},"request_key":"shared"})
            )
            .unwrap()["dispatches"],
        0
    );
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    let queued = start(&mut runtime, ta, &version, "shared");
    assert_eq!(queued["state"], "queued");
    assert!(queued.get("request").is_none());
    let id = queued["run_id"].as_str().unwrap();
    for token in [tb, tc] {
        for args in [
            json!({"view":"run","run_id":id}),
            json!({"view":"events","run_id":id}),
            json!({"view":"pending","run_id":id}),
        ] {
            assert_eq!(
                runtime
                    .scoped_call(token, "inspect", args)
                    .unwrap_err()
                    .code(),
                "FORBIDDEN"
            );
        }
        for action in ["continue", "renew", "cancel"] {
            assert_eq!(
                runtime
                    .scoped_call(token, "resume", json!({"action":action,"run_id":id}))
                    .unwrap_err()
                    .code(),
                "FORBIDDEN"
            );
        }
    }
    assert_eq!(runtime.drive_pending().unwrap(), 1);
    let waiting = runtime
        .scoped_call(ta, "inspect", json!({"view":"run","run_id":id}))
        .unwrap();
    let submission = submit(&waiting, json!({"approved":true}));
    let accepted = runtime
        .scoped_call(ta, "resume", submission.clone())
        .unwrap();
    assert_eq!(accepted["receipt"]["status"], "accepted");
    for token in [tb, tc] {
        assert_eq!(
            runtime
                .scoped_call(token, "resume", submission.clone())
                .unwrap_err()
                .code(),
            "FORBIDDEN"
        );
    }
    assert_eq!(
        runtime
            .scoped_call(ta, "resume", submission.clone())
            .unwrap()["receipt"],
        accepted["receipt"]
    );
    runtime
        .revoke_attachment(a["attachment_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(ta, "resume", submission)
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    assert_eq!(register(&mut runtime, tb), version);
    let other = start(&mut runtime, tb, &version, "shared");
    assert_ne!(queued["run_id"], other["run_id"]);
}

#[test]
fn project_file_access_rejects_escapes_symlinks_runtime_state_and_run_only_authors() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("outside.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        directory.path().join("outside.json"),
        project.join("escape.json"),
    )
    .unwrap();
    let mut runtime = Runtime::open(project.join(".apr/local")).unwrap();
    std::fs::hard_link(
        project.join(".apr/local/state.sqlite3"),
        project.join("state-copy.sqlite3"),
    )
    .unwrap();
    let author = attach(&mut runtime, &project, "A", "author");
    let runner = runtime
        .create_attachment(author["assignment_id"].as_str().unwrap(), "run")
        .unwrap();
    let token = author["token"].as_str().unwrap();
    for path in [
        "../outside.json",
        "escape.json",
        ".apr/local/state.sqlite3",
        "state-copy.sqlite3",
    ] {
        assert_eq!(runtime.scoped_call(token,"manage",json!({"action":"artifact.import","path":path,"media_type":"application/octet-stream"})).unwrap_err().code(),"FORBIDDEN");
    }
    assert_eq!(
        runtime
            .scoped_call(
                runner["token"].as_str().unwrap(),
                "manage",
                json!({"action":"process.register","path":"process.json"})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
}

#[test]
fn an_artifact_digest_never_grants_reads_inputs_literals_or_result_submission() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    std::fs::write(project.join("record.txt"), "Assignment A data").unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let b = attach(&mut runtime, &project, "B", "author");
    let ta = a["token"].as_str().unwrap();
    let tb = b["token"].as_str().unwrap();
    let reference = runtime
        .scoped_call(
            ta,
            "manage",
            json!({"action":"artifact.import","path":"record.txt","media_type":"text/plain"}),
        )
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(
                ta,
                "inspect",
                json!({"view":"artifact","reference":reference})
            )
            .unwrap()["text"],
        "Assignment A data"
    );
    assert_eq!(
        runtime
            .scoped_call(
                tb,
                "inspect",
                json!({"view":"artifact","reference":reference})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    let version = register(&mut runtime, tb);
    for mode in ["start", "preflight"] {
        assert_eq!(runtime.scoped_call(tb,"run",json!({"process_ref":version,"input":{"source":reference},"mode":mode,"request_key":"input"})).unwrap_err().code(),"FORBIDDEN");
    }
    let mut literal_process = review_process();
    literal_process["steps"][0]["inputs"]["secret"] = json!({"literal":reference});
    std::fs::write(
        project.join("literal.json"),
        serde_json::to_vec(&literal_process).unwrap(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .scoped_call(
                tb,
                "manage",
                json!({"action":"process.register","path":"literal.json"})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    let queued = start(&mut runtime, tb, &version, "result");
    runtime.drive_pending().unwrap();
    let waiting = runtime
        .scoped_call(
            tb,
            "inspect",
            json!({"view":"run","run_id":queued["run_id"]}),
        )
        .unwrap();
    assert_eq!(
        runtime
            .scoped_call(tb, "resume", submit(&waiting, json!({"source":reference})))
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    assert_eq!(
        runtime
            .scoped_call(
                tb,
                "manage",
                json!({"action":"artifact.import","path":"record.txt","media_type":"text/plain"})
            )
            .unwrap(),
        reference
    );
    runtime
        .scoped_call(tb, "resume", submit(&waiting, json!({"source":reference})))
        .unwrap();
}

#[test]
fn registered_versions_survive_source_edits_and_assignment_closure_stops_dispatch() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let token = a["token"].as_str().unwrap();
    let version = register(&mut runtime, token);
    let queued = start(&mut runtime, token, &version, "frozen");
    let mut changed = review_process();
    changed["steps"][0]["instructions"] = json!("Mutated source");
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    runtime.drive_pending().unwrap();
    let waiting = runtime
        .scoped_call(
            token,
            "inspect",
            json!({"view":"run","run_id":queued["run_id"]}),
        )
        .unwrap();
    assert_eq!(waiting["request"]["instructions"], "Review the record");
    let another = start(&mut runtime, token, &version, "closed-before-start");
    runtime
        .close_assignment(a["assignment_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(runtime.drive_pending().unwrap(), 0);
    assert_eq!(
        runtime
            .inspect(another["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "cancelled"
    );
    assert_eq!(
        runtime
            .scoped_call(token, "resume", submit(&waiting, json!({})))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    assert_eq!(
        runtime
            .scoped_call(token, "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
}

#[test]
fn replacing_a_project_directory_requires_a_new_local_registration() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let before = runtime.attach_project(&project).unwrap();
    std::fs::rename(&project, directory.path().join("old-project")).unwrap();
    std::fs::create_dir(&project).unwrap();
    assert_eq!(
        runtime
            .scoped_call(a["token"].as_str().unwrap(), "discover", json!({}))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
    let after = runtime.attach_project(&project).unwrap();
    assert_ne!(before["project_id"], after["project_id"]);
}

#[test]
fn an_unavailable_workspace_blocks_then_resumes_without_replaying_completed_work() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut process = review_process();
    process["steps"][0]["needs"] = json!(["snapshot"]);
    process["steps"].as_array_mut().unwrap().insert(
        0,
        json!({"id":"snapshot","kind":"builtin","operation":"json.write","needs":[],
        "effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"input":""}}}),
    );
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&process).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let token = a["token"].as_str().unwrap();
    let version = register(&mut runtime, token);
    let queued = start(&mut runtime, token, &version, "restore");
    let run_id = queued["run_id"].as_str().unwrap();
    runtime.drive_pending().unwrap();
    assert_eq!(runtime.inspect(run_id).unwrap().completed_steps, 1);
    std::fs::rename(&project, directory.path().join("temporarily-unmounted")).unwrap();
    runtime.drive_pending().unwrap();
    assert_eq!(runtime.inspect(run_id).unwrap().state, "blocked");
    std::fs::rename(directory.path().join("temporarily-unmounted"), &project).unwrap();
    runtime
        .scoped_call(
            token,
            "resume",
            json!({"action":"continue","run_id":run_id}),
        )
        .unwrap();
    runtime.drive_pending().unwrap();
    let waiting = runtime
        .scoped_call(token, "inspect", json!({"view":"run","run_id":run_id}))
        .unwrap();
    assert_eq!(waiting["state"], "waiting_for_agent");
    assert_eq!(waiting["completed_steps"], 1);
    assert_eq!(waiting["steps"][0]["attempts"], 1);
}

#[test]
fn revoked_attachment_does_not_erase_authorized_work_and_preflight_rejects_bad_keys() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let token = a["token"].as_str().unwrap();
    let version = register(&mut runtime, token);
    assert_eq!(
        runtime
            .scoped_call(
                token,
                "run",
                json!({"mode":"preflight","process_ref":version,"input":{},"request_key":""})
            )
            .unwrap_err()
            .code(),
        "INVALID_INPUT"
    );
    let queued = start(&mut runtime, token, &version, "survives-revocation");
    runtime
        .revoke_attachment(a["attachment_id"].as_str().unwrap())
        .unwrap();
    runtime.drive_pending().unwrap();
    assert_eq!(
        runtime
            .inspect(queued["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "waiting_for_agent"
    );
    assert_eq!(
        runtime
            .scoped_call(
                token,
                "inspect",
                json!({"view":"run","run_id":queued["run_id"]})
            )
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
}

#[test]
fn an_independent_pending_handoff_can_be_renewed_and_submitted_while_siblings_are_queued() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut process = review_process();
    process["steps"].as_array_mut().unwrap().push(
        json!({"id":"snapshot","kind":"builtin","operation":"json.write","needs":[],
        "effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"input":""}}}),
    );
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&process).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let token = a["token"].as_str().unwrap();
    let version = register(&mut runtime, token);
    let queued = start(&mut runtime, token, &version, "independent");
    let run_id = queued["run_id"].as_str().unwrap();
    runtime.drive_pending().unwrap();
    let waiting = runtime
        .scoped_call(token, "inspect", json!({"view":"run","run_id":run_id}))
        .unwrap();
    assert_eq!(waiting["state"], "queued");
    assert_eq!(waiting["request"]["step_instance_id"], "review");
    let renewed = runtime
        .scoped_call(token, "resume", json!({"action":"renew","run_id":run_id}))
        .unwrap();
    assert_eq!(renewed["request"]["generation"], 2);
    assert_eq!(
        runtime
            .scoped_call(token, "resume", submit(&waiting, json!({})))
            .unwrap_err()
            .code(),
        "STALE_HANDOFF"
    );
    runtime
        .scoped_call(token, "resume", submit(&renewed, json!({"approved":true})))
        .unwrap();
    runtime.drive_pending().unwrap();
    assert_eq!(runtime.inspect(run_id).unwrap().state, "succeeded");
}

#[test]
fn scoped_arguments_cannot_override_assignment_or_smuggle_unrecognized_actions() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let a = attach(&mut runtime, &project, "A", "author");
    let token = a["token"].as_str().unwrap();
    let version = register(&mut runtime, token);
    for (action, args) in [
        ("discover", json!({"assignment_id":"other"})),
        (
            "run",
            json!({"process_ref":version,"input":{},"request_key":"key","mode":"start","assignment_id":"other"}),
        ),
        (
            "manage",
            json!({"action":"permission.approve","path":"process.json"}),
        ),
        (
            "manage",
            json!({"action":"process.validate","path":"process.json","approved":true}),
        ),
        ("inspect", json!({"view":"pending","limit":1000})),
    ] {
        assert_eq!(
            runtime.scoped_call(token, action, args).unwrap_err().code(),
            "INVALID_INPUT"
        );
    }
    assert_eq!(
        runtime
            .scoped_call("invalid", "run", json!("malformed"))
            .unwrap_err()
            .code(),
        "UNAUTHORIZED"
    );
}

#[test]
fn analytics_is_scoped_read_only_and_reports_pending_not_false_success() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("process.json"),
        serde_json::to_vec(&review_process()).unwrap(),
    )
    .unwrap();
    let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
    let first = attach(&mut runtime, &project, "First analytics", "author");
    let second = attach(&mut runtime, &project, "Second analytics", "run");
    let token = first["token"].as_str().unwrap();
    let other = second["token"].as_str().unwrap();
    let version = register(&mut runtime, token);
    let run = start(&mut runtime, token, &version, "analytics-review");
    runtime.drive_pending().unwrap();
    let view = runtime
        .scoped_call(token, "inspect", json!({"view":"analytics"}))
        .unwrap();
    assert_eq!(view["summary"]["pending_handoffs"], 1);
    assert!(view["summary"]["terminal_root_success_rate"].is_null());
    assert!(
        view["cost_page"]["currencies"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(view["telemetry"], "local_only");
    let empty = runtime
        .scoped_call(other, "inspect", json!({"view":"analytics"}))
        .unwrap();
    assert_eq!(empty["summary"]["attempts"], 0);
    assert!(
        runtime
            .scoped_call(
                other,
                "inspect",
                json!({"view":"analytics","run_id":run["run_id"]})
            )
            .is_err()
    );
    for limit in [0, 21] {
        assert!(
            runtime
                .scoped_call(token, "inspect", json!({"view":"analytics","limit":limit}))
                .is_err()
        );
    }
    assert!(
        runtime
            .scoped_call(
                token,
                "inspect",
                json!({"view":"analytics","after":"x".repeat(161)})
            )
            .is_err()
    );
}
