use apr_runtime::Runtime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

struct Fixture {
    _root: tempfile::TempDir,
    project: std::path::PathBuf,
    state: std::path::PathBuf,
    runtime: Runtime,
    attachment: Value,
    process_ref: String,
    manifest: Value,
}

impl Fixture {
    fn new(native: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let state = root.path().join("state");
        std::fs::create_dir(&project).unwrap();
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../../examples/jev-intake/process.json"))
                .unwrap();
        manifest["schema_version"] = json!("0.2");
        manifest["steps"].as_array_mut().unwrap().push(json!({"id":"route","kind":"decision","effect":"pure","needs":["observe"],"inputs":{},"retry":{"max_attempts":1},"decision":{"source":"choice","routes":["operations","engineering","unclear"],"value":{"step":"observe","pointer":"/answers/team/choice"},"confidence":{"step":"observe","pointer":"/answers/team/confidence"},"min_confidence":0.75,"no_match":"unclear","needs_clarification":"unclear"}}));
        if native {
            let source = b"print('{}')\n";
            std::fs::write(project.join("main.py"), source).unwrap();
            manifest["files"] =
                json!([{"path":"main.py","sha256":format!("{:x}",Sha256::digest(source))}]);
            manifest["capabilities"]
                .as_array_mut()
                .unwrap()
                .push(json!({"kind":"script","language":"python"}));
            manifest["steps"].as_array_mut().unwrap().push(json!({"id":"native","kind":"script","entrypoint":"main.py","needs":[],"effect":"write","retry":{"max_attempts":1},"inputs":{},"result_schema":{"type":"object"}}));
        }
        std::fs::write(
            project.join("process.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut runtime = Runtime::open(&state).unwrap();
        let p = runtime.attach_project(&project).unwrap();
        let a = runtime
            .create_assignment(p["project_id"].as_str().unwrap(), "Tuning fixture")
            .unwrap();
        let attachment = runtime
            .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
            .unwrap();
        let registered = runtime
            .scoped_call(
                attachment["token"].as_str().unwrap(),
                "manage",
                json!({"action":"process.register","path":"process.json"}),
            )
            .unwrap();
        Self {
            _root: root,
            project,
            state,
            runtime,
            attachment,
            process_ref: registered["process_ref"].as_str().unwrap().into(),
            manifest,
        }
    }
    fn request(&self) -> Value {
        let mut question =
            self.manifest["steps"][0]["inputs"]["questions"]["literal"]["team"].clone();
        question["instructions"] =
            json!("Prefer an explicit team; use unclear when evidence is missing.");
        let mut policy = self.manifest["steps"][1]["decision"].clone();
        policy["min_confidence"] = json!(0.9);
        json!({"action":"process.tune_jev","process_ref":self.process_ref,"mode":"preview","changes":[{"kind":"question","step_id":"observe","question_id":"team","question":question},{"kind":"policy","step_id":"route","policy":policy}]})
    }
    fn call(&mut self, request: Value) -> apr_runtime::Result<Value> {
        self.runtime.scoped_call(
            self.attachment["token"].as_str().unwrap(),
            "manage",
            request,
        )
    }
}

#[test]
fn tuning_previews_then_publishes_immutable_idempotent_versions_without_provider() {
    let mut f = Fixture::new(false);
    let token = f.attachment["token"].as_str().unwrap().to_owned();
    let before = f
        .runtime
        .scoped_call(
            &token,
            "inspect",
            json!({"view":"jev_tuning","process_ref":f.process_ref}),
        )
        .unwrap();
    assert_eq!(before["questions"].as_array().unwrap().len(), 2);
    assert_eq!(before["policies"].as_array().unwrap().len(), 1);
    let mut request = f.request();
    let preview = f.call(request.clone()).unwrap();
    assert_eq!(preview["registered"], false);
    assert_eq!(preview["validation"]["provider_inference"], false);
    assert_ne!(preview["process_ref"], f.process_ref);
    assert!(
        f.runtime
            .scoped_call(
                &token,
                "inspect",
                json!({"view":"jev_tuning","process_ref":preview["process_ref"]})
            )
            .is_err()
    );
    request["mode"] = json!("publish");
    assert!(f.call(request.clone()).is_err());
    request["expected_process_ref"] = json!(f.process_ref);
    assert!(f.call(request.clone()).is_err());
    request["expected_process_ref"] = preview["process_ref"].clone();
    let published = f.call(request.clone()).unwrap();
    assert_eq!(published["registered"], true);
    assert_eq!(f.call(request).unwrap(), published);
    let old = f
        .runtime
        .scoped_call(
            &token,
            "inspect",
            json!({"view":"jev_tuning","process_ref":f.process_ref}),
        )
        .unwrap();
    assert_eq!(old, before);
    let new = f
        .runtime
        .scoped_call(
            &token,
            "inspect",
            json!({"view":"jev_tuning","process_ref":published["process_ref"]}),
        )
        .unwrap();
    assert_eq!(new["policies"][0]["policy"]["min_confidence"], 0.9);
    let db = rusqlite::Connection::open(f.state.join("state.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM catalog", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn tuning_rejects_authority_graph_and_provider_contract_changes() {
    let mut f = Fixture::new(false);
    let base = f.request();
    let mut invalids = Vec::new();
    let mut request = base.clone();
    request["capabilities"] = json!([]);
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][0]["question"]["type"] = json!("score");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][0]["question"]["criteria"]["new_route"] = json!("another route");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][0]["question"]["instructions"] = json!("");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][0]["question"]["url"] = json!("https://example.invalid");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][0]["step_id"] = json!("route");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][0]["question_id"] = json!("missing");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][1]["policy"]["min_confidence"] = json!(1.1);
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][1]["policy"]["value"]["pointer"] = json!("/answers/urgent/noul");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][1]["policy"]["routes"][0] = json!("different");
    invalids.push(request);
    let mut request = base.clone();
    request["changes"][1]["policy"]["no_match"] = json!("missing");
    invalids.push(request);
    for request in invalids {
        assert!(f.call(request.clone()).is_err(), "accepted {request}");
    }
    let run = f
        .runtime
        .create_attachment(f.attachment["assignment_id"].as_str().unwrap(), "run")
        .unwrap();
    for (action, request) in [
        ("manage", base.clone()),
        (
            "inspect",
            json!({"view":"jev_tuning","process_ref":f.process_ref}),
        ),
    ] {
        assert_eq!(
            f.runtime
                .scoped_call(run["token"].as_str().unwrap(), action, request)
                .unwrap_err()
                .code(),
            "FORBIDDEN"
        );
    }
    let p = f.runtime.attach_project(&f.project).unwrap();
    let a = f
        .runtime
        .create_assignment(p["project_id"].as_str().unwrap(), "Other")
        .unwrap();
    let other = f
        .runtime
        .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
        .unwrap();
    assert_eq!(
        f.runtime
            .scoped_call(other["token"].as_str().unwrap(), "manage", base)
            .unwrap_err()
            .code(),
        "FORBIDDEN"
    );
}

#[test]
fn tuning_retains_native_sources_and_rejects_missing_registered_snapshots() {
    let mut f = Fixture::new(true);
    std::fs::write(f.project.join("main.py"), b"different source\n").unwrap();
    let mut request = f.request();
    let preview = f.call(request.clone()).unwrap();
    request["mode"] = json!("publish");
    request["expected_process_ref"] = preview["process_ref"].clone();
    let published = f.call(request).unwrap();
    let db = rusqlite::Connection::open(f.state.join("state.sqlite3")).unwrap();
    let bytes: Vec<u8> = db
        .query_row(
            "SELECT source FROM process_sources WHERE version=?1",
            [published["process_ref"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bytes, b"print('{}')\n");
    db.execute(
        "DELETE FROM process_sources WHERE version=?1",
        [&f.process_ref],
    )
    .unwrap();
    assert_eq!(
        f.call(f.request()).unwrap_err().code(),
        "ARTIFACT_INTEGRITY"
    );
}

#[test]
fn score_noul_and_associated_rule_tuning_preserve_decision_structure() {
    let mut f = Fixture::new(false);
    f.manifest["steps"][0]["inputs"]["questions"]["literal"]["quality"] = json!({"type":"score","instructions":"Score the record quality","criteria":["Incomplete","Complete"]});
    f.manifest["steps"][1]["decision"] = json!({"source":"rules","routes":["operations","unclear"],"cases":[{"value":{"step":"observe","pointer":"/answers/urgent/noul"},"equals":1,"route":"operations"}],"default":"unclear"});
    std::fs::write(
        f.project.join("process.json"),
        serde_json::to_vec(&f.manifest).unwrap(),
    )
    .unwrap();
    let registered = f
        .call(json!({"action":"process.register","path":"process.json"}))
        .unwrap();
    f.process_ref = registered["process_ref"].as_str().unwrap().to_owned();
    let changes = json!([
        {"kind":"question","step_id":"observe","question_id":"quality","question":{"type":"score","instructions":"Assess completeness","criteria":[{"level":"Evidence missing"},{"level":"Evidence supplied"}]}},
        {"kind":"question","step_id":"observe","question_id":"urgent","question":{"type":"noul","instructions":"Is an outage underway?","criteria":{"true":"Explicit outage","false":"No outage stated"}}},
        {"kind":"policy","step_id":"route","policy":{"source":"rules","routes":["operations","unclear"],"cases":[{"value":{"step":"observe","pointer":"/answers/urgent/noul"},"equals":0.95,"route":"operations"}],"default":"unclear"}}
    ]);
    let request = json!({"action":"process.tune_jev","process_ref":f.process_ref,"mode":"preview","changes":changes});
    assert!(f.call(request.clone()).is_ok());
    let mut wrong = request.clone();
    wrong["changes"][0]["question"]["criteria"]
        .as_array_mut()
        .unwrap()
        .push(json!("Extra level"));
    assert!(f.call(wrong).is_err());
    let mut wrong = request.clone();
    wrong["changes"][1]["question"]["criteria"]["false"] = json!("");
    assert!(f.call(wrong).is_err());
    let mut wrong = request;
    wrong["changes"][2]["policy"]["cases"][0]["value"] = json!({"input":"/ticket"});
    assert!(f.call(wrong).is_err());
}
