use apr_runtime::Runtime;
use serde_json::{Value, json};

struct Fixture {
    dir: tempfile::TempDir,
    runtime: Runtime,
    token: String,
    other_token: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
        let p = runtime.attach_project(&project).unwrap();
        let a = runtime
            .create_assignment(p["project_id"].as_str().unwrap(), "first")
            .unwrap();
        let b = runtime
            .create_assignment(p["project_id"].as_str().unwrap(), "second")
            .unwrap();
        let token = runtime
            .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .into();
        let other_token = runtime
            .create_attachment(b["assignment_id"].as_str().unwrap(), "author")
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .into();
        Self {
            dir,
            runtime,
            token,
            other_token,
        }
    }
    fn file(&self, name: &str, value: &Value) {
        std::fs::write(
            self.dir.path().join("project").join(name),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    fn call(&mut self, action: &str, value: Value) -> Value {
        self.runtime
            .scoped_call(&self.token, action, value)
            .unwrap()
    }
    fn baseline(&mut self) -> (String, String) {
        let value: Value = serde_json::from_str(include_str!(
            "../../../examples/recipe-improvement/baseline.json"
        ))
        .unwrap();
        self.file("baseline.json", &value);
        let version = self.call(
            "manage",
            json!({"action":"process.register","path":"baseline.json"}),
        )["process_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let id=self.call("run",json!({"process_ref":version,"mode":"start","request_key":"one","input":{"answer":"A","evidence":"source A"}}))["run_id"].as_str().unwrap().to_string();
        for _ in 0..8 {
            self.runtime.drive_pending().unwrap();
            if self.runtime.inspect(&id).unwrap().state == "succeeded" {
                break;
            }
        }
        (version, id)
    }
    fn proposed(&mut self, version: &str, run: &str, candidate: Value, key: &str) -> String {
        self.file("candidate.json", &candidate);
        let opened=self.call("manage",json!({"action":"improvement.open","request_key":key,"process_ref":version,"run_ids":[run]}));
        let id = opened["cycle_id"].as_str().unwrap().to_string();
        self.call("manage",json!({"action":"improvement.propose","cycle_id":id,"candidate_path":"candidate.json",
            "proposal":{"hypothesis":"remove redundant selection","change":"direct selection","benefit":"one step","risks":"missing evidence","evidence_run_ids":[run]},
            "review":{}}));
        id
    }
    fn freeze(&mut self, id: &str, heldout: &str, expected: Value) {
        self.call("manage",json!({"action":"improvement.freeze","cycle_id":id,"policy":{"evaluator_version":"pure-v1","split_method":"separate source entities","min_step_reduction":1},
            "development":[{"case_id":"dev","source_id":"src-dev","input":{"answer":"D","evidence":"doc D"},"expected":{"answer":"D","evidence":"doc D"},"provenance":"human"}],
            "holdout":[{"case_id":heldout,"source_id":heldout,"input":{"answer":"H","evidence":"doc H"},"expected":expected,"provenance":"human"}]}));
    }
}

fn candidate() -> Value {
    serde_json::from_str(include_str!(
        "../../../examples/recipe-improvement/candidate.json"
    ))
    .unwrap()
}

#[test]
fn evidence_to_explicit_adoption_preserves_old_run_and_scoped_selection() {
    let mut f = Fixture::new();
    let (base, run) = f.baseline();
    let evidence = f.call(
        "inspect",
        json!({"view":"improvement_evidence","process_ref":base,"run_ids":[run]}),
    );
    assert_eq!(evidence["runs"][0]["state"], "succeeded");
    assert_eq!(evidence["runs"][0]["quality"], "unverified");
    let id = f.proposed(&base, &run, candidate(), "adopt-cycle");
    let schedule=f.call("manage",json!({"action":"schedule.create","name":"Pinned synthetic","process_ref":base,
        "input":{"answer":"future","evidence":"future source"},"request_key":"pinned-schedule","trigger":{"kind":"once","at_ms":2000000000000_i64}}));
    let schedule_id = schedule["schedule_id"].as_str().unwrap().to_string();
    f.freeze(&id, "held-a", json!({"answer":"H","evidence":"doc H"}));
    let evaluated = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    assert_eq!(evaluated["report"]["result"], "pass", "{evaluated}");
    assert_eq!(evaluated["report"]["step_reduction_total"], 1);
    assert!(!evaluated.to_string().contains("doc H"));
    let digest = evaluated["report_digest"].as_str().unwrap();
    let adopted=f.call("manage",json!({"action":"improvement.decide","cycle_id":id,"decision":"adopt","candidate_path":"candidate.json","expected_report_digest":digest,"expected_selected_ref":null}));
    assert_eq!(adopted["state"], "adopted");
    let found = f.call("discover", json!({"query":"synthetic.copy-review"}));
    assert!(
        found["processes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["selected_for_future_work"] == true
                && p["process_ref"] == adopted["candidate_ref"])
    );
    let old = f.call("inspect", json!({"view":"run","run_id":run}));
    assert_eq!(old["process_version"], base);
    let pinned = f.call(
        "inspect",
        json!({"view":"schedule","schedule_id":schedule_id}),
    );
    assert_eq!(pinned["process_ref"], base, "{pinned}");
    let rollback = f.call(
        "manage",
        json!({"action":"process.select","process_ref":base,
        "expected_selected_ref":adopted["candidate_ref"],"expected_selection_revision":1}),
    );
    assert_eq!(rollback["selected_process_ref"], base);
    assert_eq!(rollback["selection_revision"], 2);
    assert_eq!(
        f.runtime
            .scoped_call(
                &f.token,
                "manage",
                json!({"action":"process.select","process_ref":adopted["candidate_ref"],
        "expected_selected_ref":base,"expected_selection_revision":1})
            )
            .unwrap_err()
            .code(),
        "REQUEST_CONFLICT"
    );
    assert_eq!(
        f.call("inspect", json!({"view":"improvements","process_ref":base}))["cycles"][0]["state"],
        "adopted"
    );
    assert_eq!(
        f.runtime
            .scoped_call(
                &f.other_token,
                "inspect",
                json!({"view":"improvement","cycle_id":id})
            )
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
    f.runtime = Runtime::open(f.dir.path().join("state")).unwrap();
    let retry = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    assert_eq!(retry["report_digest"], digest);
}

#[test]
fn conflicting_open_bad_quality_missing_evidence_and_holdout_reuse_fail_closed() {
    let mut f = Fixture::new();
    let (base, run) = f.baseline();
    let id = f.proposed(&base, &run, candidate(), "cycle-one");
    assert_eq!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.open","request_key":"cycle-one","process_ref":base,"run_ids":[]})).unwrap_err().code(),"REQUEST_CONFLICT");
    f.freeze(
        &id,
        "held-one",
        json!({"answer":"WRONG","evidence":"doc H"}),
    );
    let failed = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    assert_eq!(failed["report"]["result"], "fail");
    assert_eq!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.decide","cycle_id":id,"decision":"adopt","candidate_path":"candidate.json","expected_report_digest":failed["report_digest"],"expected_selected_ref":null})).unwrap_err().code(),"INVALID_STATE");
    let second = f.proposed(&base, &run, candidate(), "cycle-two");
    assert_eq!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.freeze","cycle_id":second,"policy":{"evaluator_version":"pure-v1","split_method":"separate","min_step_reduction":1},
        "development":[{"case_id":"d2","source_id":"new-dev","input":{"answer":"X","evidence":"src"},"expected":{"answer":"X","evidence":"src"},"provenance":"human"}],
        "holdout":[{"case_id":"h2","source_id":"HELD-ONE","input":{"answer":"Q","evidence":"src"},"expected":{"answer":"Q","evidence":"src"},"provenance":"human"}]})).unwrap_err().code(),"INVALID_STATE");
}

#[test]
fn unsupported_external_effect_and_stale_candidate_cannot_adopt() {
    let mut f = Fixture::new();
    let (base, run) = f.baseline();
    let id = f.proposed(&base, &run, candidate(), "cycle-stale");
    f.freeze(&id, "held-stale", json!({"answer":"H","evidence":"doc H"}));
    let report = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    let mut changed = candidate();
    changed["description"] = json!("altered after report");
    f.file("candidate.json", &changed);
    assert_eq!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.decide","cycle_id":id,"decision":"adopt","candidate_path":"candidate.json","expected_report_digest":report["report_digest"],"expected_selected_ref":null})).unwrap_err().code(),"REQUEST_CONFLICT");
    // A valid native/connector candidate is not needed to prove this gate: even
    // an admitted non-allowlisted builtin must be refused before evaluation.
    let mut artifact = candidate();
    artifact["steps"][0]["operation"] = json!("json.write");
    let other = f.proposed(&base, &run, artifact, "cycle-unsafe");
    let freeze=f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.freeze","cycle_id":other,"policy":{"evaluator_version":"pure-v1","split_method":"separate","min_step_reduction":1},
        "development":[{"case_id":"dev-unsafe","source_id":"src-dev-unsafe","input":{"answer":"D2","evidence":"doc D2"},"expected":{"answer":"D2","evidence":"doc D2"},"provenance":"human"}],
        "holdout":[{"case_id":"held-unsafe","source_id":"held-unsafe","input":{"answer":"H2","evidence":"doc H2"},"expected":{"answer":"H2","evidence":"doc H2"},"provenance":"human"}]})).unwrap();
    assert_eq!(freeze["state"], "frozen");
    let result = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":other}),
    );
    assert_eq!(result["report"]["result"], "inconclusive");
    assert_eq!(result["report"]["evaluated_cases"], 0);

    let mut external = candidate();
    external["capabilities"] = json!([{"kind":"connection","name":"unapproved-write"}]);
    external["limits"]["max_provider_calls"] = json!(1);
    external["steps"][0] = json!({"id":"answer","kind":"connector","effect":"write","operation":"http.json",
        "connection":"unapproved-write","needs":[],"retry":{"max_attempts":1},
        "inputs":{"record":{"input":""}},"result_schema":{"type":"object"}});
    let effect = f.proposed(&base, &run, external, "cycle-external-write");
    f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.freeze","cycle_id":effect,
        "policy":{"evaluator_version":"pure-v1","split_method":"third source","min_step_reduction":1},
        "development":[{"case_id":"d-ext","source_id":"d-ext","input":{"answer":"DE","evidence":"doc DE"},"expected":{"answer":"DE","evidence":"doc DE"},"provenance":"human"}],
        "holdout":[{"case_id":"h-ext","source_id":"h-ext","input":{"answer":"HE","evidence":"doc HE"},"expected":{"answer":"HE","evidence":"doc HE"},"provenance":"human"}]})).unwrap();
    let blocked = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":effect}),
    );
    assert_eq!(blocked["report"]["result"], "inconclusive");
    assert_eq!(blocked["report"]["evaluated_cases"], 0);
    assert!(
        blocked["report"]["reason"]
            .as_str()
            .unwrap()
            .contains("additional_evidence_required")
    );
}

#[test]
fn pure_route_and_early_stop_keep_complete_evidence() {
    let mut f = Fixture::new();
    let (base, run) = f.baseline();
    let mut routed = candidate();
    routed["steps"] = json!([
        {"id":"route","kind":"decision","effect":"pure","needs":[],"inputs":{},"retry":{"max_attempts":1},
          "decision":{"source":"rules","routes":["fast","normal"],"cases":[{"value":{"input":"/answer"},"equals":"H","route":"fast"}],"default":"normal"}},
        {"id":"fast","kind":"builtin","effect":"pure","needs":["route"],"when":{"decision":"route","route":"fast"},
          "retry":{"max_attempts":1},"operation":"json.select","inputs":{"value":{"input":""},"pointer":{"literal":"/answer"}}},
        {"id":"normal","kind":"builtin","effect":"pure","needs":["route"],"when":{"decision":"route","route":"normal"},
          "retry":{"max_attempts":1},"operation":"json.select","inputs":{"value":{"input":""},"pointer":{"literal":"/answer"}}},
        {"id":"joined","kind":"join","effect":"pure","needs":["route","fast","normal"],"inputs":{},"retry":{"max_attempts":1},
          "join":{"decision":"route","branches":{"fast":{"step":"fast","pointer":"/value"},"normal":{"step":"normal","pointer":"/value"}}}}
    ]);
    routed["output"]["answer"] = json!({"step":"joined","pointer":"/output"});
    let id = f.proposed(&base, &run, routed, "routed");
    f.freeze(&id, "route-held", json!({"answer":"H","evidence":"doc H"}));
    // Three executed routing steps exceed the two-step baseline. The candidate
    // preserves the output, but fails the predeclared benefit threshold.
    let report = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    assert_eq!(report["report"]["result"], "fail", "{report}");
    assert_eq!(report["report"]["candidate_correct"], 1);
    assert_eq!(report["report"]["required_output_complete"], 1);
    assert!(report["report"]["step_reduction_total"].as_i64().unwrap() < 0);

    let mut omitted = candidate();
    omitted["output"]
        .as_object_mut()
        .unwrap()
        .remove("evidence");
    f.file("candidate.json", &omitted);
    let opened=f.call("manage",json!({"action":"improvement.open","request_key":"omitted","process_ref":base,"run_ids":[run]}));
    let attempt=f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.propose","cycle_id":opened["cycle_id"],"candidate_path":"candidate.json",
        "proposal":{"hypothesis":"stop early","change":"omit evidence","benefit":"fewer bytes","risks":"missing source","evidence_run_ids":[run]},"review":{}}));
    if let Ok(proposed) = attempt {
        let id = proposed["cycle_id"].as_str().unwrap();
        f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.freeze","cycle_id":id,
            "policy":{"evaluator_version":"pure-v1","split_method":"separate sources","min_step_reduction":1},
            "development":[{"case_id":"dev-omitted","source_id":"dev-omitted","input":{"answer":"D-O","evidence":"doc D-O"},"expected":{"answer":"D-O","evidence":"doc D-O"},"provenance":"human"}],
            "holdout":[{"case_id":"held-omitted","source_id":"held-omitted","input":{"answer":"H-O","evidence":"doc H-O"},"expected":{"answer":"H-O","evidence":"doc H-O"},"provenance":"human"}]})).unwrap();
        let result = f.call(
            "manage",
            json!({"action":"improvement.evaluate","cycle_id":id}),
        );
        assert_ne!(result["report"]["result"], "pass");
    }
}

#[test]
fn no_change_is_durable_and_newer_selection_blocks_old_adoption() {
    let mut f = Fixture::new();
    let (base, run) = f.baseline();
    let opened=f.call("manage",json!({"action":"improvement.open","request_key":"no-change","process_ref":base,"run_ids":[run]}));
    let id = opened["cycle_id"].as_str().unwrap();
    let closed=f.call("manage",json!({"action":"improvement.close","cycle_id":id,"outcome":"no_supported_improvement","reason":"No supported pure change in inspected cases."}));
    assert_eq!(closed["state"], "closed");
    assert_eq!(
        f.call("inspect", json!({"view":"improvement","cycle_id":id}))["outcome"],
        "no_supported_improvement"
    );
    assert_eq!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.close","cycle_id":id,"outcome":"more_evidence_required","reason":"changed"})).unwrap_err().code(),"REQUEST_CONFLICT");

    let a = f.proposed(&base, &run, candidate(), "selection-a");
    f.freeze(&a, "select-a", json!({"answer":"H","evidence":"doc H"}));
    let report_a = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":a}),
    );
    let b = f.proposed(&base, &run, candidate(), "selection-b");
    let second=f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.freeze","cycle_id":b,"policy":{"evaluator_version":"pure-v1","split_method":"new source","min_step_reduction":1},
        "development":[{"case_id":"d-b","source_id":"d-b","input":{"answer":"D-B","evidence":"d-B"},"expected":{"answer":"D-B","evidence":"d-B"},"provenance":"human"}],
        "holdout":[{"case_id":"h-b","source_id":"h-b","input":{"answer":"H-B","evidence":"h-B"},"expected":{"answer":"H-B","evidence":"h-B"},"provenance":"human"}]})).unwrap();
    assert_eq!(second["state"], "frozen");
    let report_b = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":b}),
    );
    f.call("manage",json!({"action":"improvement.decide","cycle_id":b,"decision":"adopt","candidate_path":"candidate.json","expected_report_digest":report_b["report_digest"],"expected_selected_ref":null}));
    assert_eq!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.decide","cycle_id":a,"decision":"adopt","candidate_path":"candidate.json","expected_report_digest":report_a["report_digest"],"expected_selected_ref":null})).unwrap_err().code(),"REQUEST_CONFLICT");
}

#[test]
fn missing_human_label_is_inconclusive_and_cannot_adopt() {
    let mut f = Fixture::new();
    let (base, run) = f.baseline();
    let id = f.proposed(&base, &run, candidate(), "missing-label");
    f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.freeze","cycle_id":id,
        "policy":{"evaluator_version":"pure-v1","split_method":"distinct source","min_step_reduction":1},
        "development":[{"case_id":"d","source_id":"dev-label","input":{"answer":"D","evidence":"doc D"},"expected":{"answer":"D","evidence":"doc D"},"provenance":"human"}],
        "holdout":[{"case_id":"h","source_id":"held-label","input":{"answer":"H","evidence":"doc H"},"provenance":"human"}]})).unwrap();
    let report = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    assert_eq!(report["report"]["result"], "inconclusive");
    assert_eq!(report["report"]["evaluated_cases"], 0);
    assert_eq!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"improvement.decide","cycle_id":id,"decision":"adopt","candidate_path":"candidate.json","expected_report_digest":report["report_digest"],"expected_selected_ref":null})).unwrap_err().code(),"INVALID_STATE");
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn improvement_child_evaluate() {
    let Ok(state) = std::env::var("APR_IMPROVEMENT_CHILD_STATE") else {
        return;
    };
    let token = std::env::var("APR_IMPROVEMENT_CHILD_TOKEN").unwrap();
    let cycle = std::env::var("APR_IMPROVEMENT_CHILD_CYCLE").unwrap();
    let mut runtime = Runtime::open(state).unwrap();
    runtime
        .scoped_call(
            &token,
            "manage",
            json!({"action":"improvement.evaluate","cycle_id":cycle}),
        )
        .unwrap();
}

#[cfg(all(feature = "fault-injection", unix))]
#[test]
fn interrupted_evaluation_resumes_without_duplicate_dispatch() {
    use std::os::unix::process::ExitStatusExt;
    let mut f = Fixture::new();
    let (base, run) = f.baseline();
    let id = f.proposed(&base, &run, candidate(), "crash-evaluate");
    f.freeze(&id, "crash-held", json!({"answer":"H","evidence":"doc H"}));
    for (point, state) in [
        ("improvement_before_report_commit", "frozen"),
        ("improvement_after_report_commit", "evaluated"),
    ] {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "improvement_child_evaluate", "--nocapture"])
            .env("APR_IMPROVEMENT_CHILD_STATE", f.dir.path().join("state"))
            .env("APR_IMPROVEMENT_CHILD_TOKEN", &f.token)
            .env("APR_IMPROVEMENT_CHILD_CYCLE", &id)
            .env("APR_TEST_CRASH_AT", point)
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        f.runtime = Runtime::open(f.dir.path().join("state")).unwrap();
        assert_eq!(
            f.call("inspect", json!({"view":"improvement","cycle_id":id}))["state"],
            state
        );
    }
    let first = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    let second = f.call(
        "manage",
        json!({"action":"improvement.evaluate","cycle_id":id}),
    );
    assert_eq!(first["report_digest"], second["report_digest"]);
    assert_eq!(first["report"]["evaluations_used"], 1);
    assert_eq!(
        first["report"]["effects"],
        "no network, native, artifact or model execution permitted"
    );
}
