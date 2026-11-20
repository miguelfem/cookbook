use apr_runtime::{Runtime, capacity::CapacityPolicy};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

struct Fixture {
    dir: tempfile::TempDir,
    runtime: Runtime,
    token: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut runtime = Runtime::open(dir.path().join("state")).unwrap();
        let p = runtime.attach_project(&project).unwrap();
        let a = runtime
            .create_assignment(p["project_id"].as_str().unwrap(), "capacity")
            .unwrap();
        let token = runtime
            .create_attachment(a["assignment_id"].as_str().unwrap(), "author")
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            dir,
            runtime,
            token,
        }
    }
    fn call(&mut self, action: &str, args: Value) -> Value {
        self.runtime.scoped_call(&self.token, action, args).unwrap()
    }
    fn register(&mut self, process: &Value) -> String {
        std::fs::write(
            self.dir.path().join("project/process.json"),
            serde_json::to_vec(process).unwrap(),
        )
        .unwrap();
        self.call(
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )["process_ref"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn start(&mut self, version: &str, key: &str) -> String {
        self.call(
            "run",
            json!({"mode":"start","process_ref":version,"input":{},"request_key":key}),
        )["run_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn family(&mut self, root: &str) -> Vec<Value> {
        let mut runs = Vec::new();
        let mut args = json!({"view":"tree","run_id":root,"limit":100});
        loop {
            let page = self.call("inspect", args.clone());
            runs.extend(page["runs"].as_array().unwrap().iter().cloned());
            if page["truncated"] == false {
                return runs;
            }
            args["after"] = page["next_cursor"].clone();
        }
    }
    fn wait_for_leaf(&mut self, root: &str, depth: usize) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.runtime.drive_pending().unwrap();
            let runs = self.family(root);
            if let Some(leaf) = runs.iter().find(|r| r["actionable_waiting"] == true) {
                assert_eq!(runs.len(), depth + 1);
                assert_eq!(leaf["depth"], depth);
                return leaf["run_id"].as_str().unwrap().to_owned();
            }
            assert!(
                Instant::now() < deadline,
                "deep workflow did not reach review: {runs:?}"
            );
        }
    }
    fn settle(&mut self, root: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.runtime.inspect(root).unwrap().state != "succeeded" {
            self.runtime.drive_pending().unwrap();
            let run = self.runtime.inspect(root).unwrap();
            assert!(
                !matches!(run.state.as_str(), "failed" | "needs_reconciliation"),
                "{run:?}"
            );
            assert!(Instant::now() < deadline, "root did not complete: {run:?}");
        }
    }
    fn chain(&mut self, depth: usize) -> String {
        let mut target = self.register(&process("deep.review", true));
        for level in 1..=depth {
            let mut wrapper = process(&format!("deep.level{level}"), false);
            wrapper["limits"]["max_call_depth"] = json!(32.max(level));
            wrapper["steps"] = json!([{"id":"invoke","kind":"call","effect":"control","needs":[],"retry":{"max_attempts":1},"inputs":{},"call":{"process_ref":target,"output_pointer":"/result"}}]);
            wrapper["output"] = json!({"result":{"step":"invoke","pointer":""}});
            target = self.register(&wrapper);
        }
        target
    }
}
fn process(name: &str, review: bool) -> Value {
    let step = if review {
        json!({"id":"work","kind":"agent","effect":"agent","needs":[],"retry":{"max_attempts":1},"inputs":{},"capability":"review","instructions":"Synthetic capacity review","result_schema":{"type":"object"}})
    } else {
        json!({"id":"work","kind":"builtin","operation":"json.select","effect":"pure","needs":[],"retry":{"max_attempts":1},"inputs":{"value":{"literal":{"approved":true}},"pointer":{"literal":""}}})
    };
    json!({"schema_version":"0.2","name":name,"description":"Synthetic nesting capacity test","input_schema":{"type":"object"},"output_schema":{"type":"object"},"config":{"schema":{"type":"object"},"defaults":{},"run_override_keys":[]},"capabilities":[{"kind":"builtin"}],"limits":{"max_step_ms":1000,"max_artifact_bytes":1000000,"max_provider_calls":0,"max_agent_requests":1,"max_call_depth":32,"max_child_runs":1024,"max_step_instances":10000},"steps":[step],"output":{"result":{"step":"work","pointer":if review {""} else {"/value"}}}})
}

#[test]
fn depth32_and_owner_enabled128_preserve_pause_restart_handoffs_and_identity() {
    for depth in [32, 128] {
        let mut f = Fixture::new();
        let version = f.chain(depth);
        if depth > 32 {
            let error = f
                .runtime
                .scoped_call(
                    &f.token,
                    "run",
                    json!({"mode":"start","process_ref":version,"input":{},"request_key":"denied"}),
                )
                .unwrap_err();
            assert_eq!(error.code(), "LIMIT_EXCEEDED");
            f.runtime
                .set_capacity_policy(CapacityPolicy {
                    max_call_depth: 128,
                    ..CapacityPolicy::default()
                })
                .unwrap();
        }
        let root = f.start(&version, "root");
        let leaf = f.wait_for_leaf(&root, depth);
        let ids: Vec<_> = f
            .family(&root)
            .iter()
            .map(|r| r["run_id"].clone())
            .collect();
        let request = f.runtime.inspect(&leaf).unwrap().request.unwrap();
        let answer = json!({"protocol_version":"0.1","action":"submit","run_id":leaf,"request_id":request.request_id,"step_instance_id":request.step_instance_id,"generation":request.generation,"submission_key":"observed","continuation_token":request.continuation_token,"output":{"approved":true},"provenance":{"source":"synthetic_test"}});
        f.call("resume", json!({"action":"pause","run_id":root}));
        assert!(
            f.family(&root)
                .iter()
                .all(|r| r["actionable_waiting"] == false)
        );
        f.runtime.drive_pending().unwrap();
        assert_eq!(f.runtime.inspect(&root).unwrap().state, "paused");
        assert_eq!(f.runtime.inspect(&leaf).unwrap().state, "waiting_for_agent");
        // Lowering policy affects new work, not this accepted graph or its replay.
        f.runtime
            .set_capacity_policy(CapacityPolicy {
                max_call_depth: 8,
                ..CapacityPolicy::default()
            })
            .unwrap();
        drop(f.runtime);
        f.runtime = Runtime::open(f.dir.path().join("state")).unwrap();
        assert_eq!(f.start(&version, "root"), root);
        f.call("resume", json!({"action":"continue","run_id":root}));
        let receipt = f.call("resume", answer.clone());
        assert_eq!(f.call("resume", answer), receipt);
        f.settle(&root);
        assert_eq!(
            f.runtime.inspect(&root).unwrap().output.unwrap(),
            json!({"result":{"approved":true}})
        );
        assert_eq!(
            f.family(&root)
                .iter()
                .map(|r| r["run_id"].clone())
                .collect::<Vec<_>>(),
            ids
        );
    }
}

#[test]
fn deep_cancel_stops_descendants_and_rejects_the_saved_leaf_submission() {
    let mut f = Fixture::new();
    let version = f.chain(32);
    let root = f.start(&version, "cancel");
    let leaf = f.wait_for_leaf(&root, 32);
    let request = f.runtime.inspect(&leaf).unwrap().request.unwrap();
    f.call("resume", json!({"action":"cancel","run_id":root}));
    drop(f.runtime);
    f.runtime = Runtime::open(f.dir.path().join("state")).unwrap();
    f.runtime.drive_pending().unwrap();
    let family = f.family(&root);
    assert_eq!(family.len(), 33);
    assert!(family.iter().all(|r| r["state"] == "cancelled"));
    assert!(f.runtime.scoped_call(&f.token,"resume",json!({"protocol_version":"0.1","action":"submit","run_id":leaf,"request_id":request.request_id,"step_instance_id":request.step_instance_id,"generation":request.generation,"submission_key":"too-late","continuation_token":request.continuation_token,"output":{"approved":true},"provenance":{}})).is_err());
}

#[test]
fn exclusive_routes_share_execution_budget_but_all_versions_count_toward_graph_capacity() {
    let mut f = Fixture::new();
    let routes: Vec<_> = (0..32).map(|i| format!("r{i}")).collect();
    let mut root = process("routing.capacity", false);
    let mut steps = vec![
        json!({"id":"gate","kind":"decision","effect":"pure","needs":[],"retry":{"max_attempts":1},"inputs":{},"decision":{"source":"rules","routes":routes,"cases":[{"value":{"literal":true},"equals":true,"route":"r0"}],"default":"r1"}}),
    ];
    let mut branches = serde_json::Map::new();
    let mut join_needs = vec!["gate".to_owned()];
    for i in 0..32 {
        let route = format!("r{i}");
        let child = f.register(&process(&format!("route.leaf{i}"), false));
        steps.push(json!({"id":route,"kind":"call","effect":"control","needs":["gate"],"when":{"decision":"gate","route":route},"retry":{"max_attempts":1},"inputs":{},"call":{"process_ref":child}}));
        branches.insert(route.clone(), json!({"step":route,"pointer":"/result"}));
        join_needs.push(route);
    }
    steps.push(json!({"id":"merge","kind":"join","effect":"pure","needs":join_needs,"retry":{"max_attempts":1},"inputs":{},"join":{"decision":"gate","branches":branches}}));
    root["steps"] = json!(steps);
    root["limits"]["max_child_runs"] = json!(1);
    root["limits"]["max_step_instances"] = json!(35);
    root["output"] = json!({"result":{"step":"merge","pointer":"/output"}});
    let version = f.register(&root);
    f.runtime
        .set_capacity_policy(CapacityPolicy {
            max_graph_versions: 32,
            ..CapacityPolicy::default()
        })
        .unwrap();
    assert!(f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":version,"input":{},"request_key":"graph-denied"})).is_err());
    f.runtime
        .set_capacity_policy(CapacityPolicy {
            max_graph_versions: 33,
            ..CapacityPolicy::default()
        })
        .unwrap();
    let run = f.start(&version, "one-route");
    f.settle(&run);
    assert_eq!(f.family(&run).len(), 2);
    let inspected = f.runtime.inspect(&run).unwrap();
    assert_eq!(
        inspected
            .steps
            .iter()
            .filter(|s| s.state == "skipped")
            .count(),
        31
    );
    assert_eq!(
        inspected.output.unwrap(),
        json!({"result":{"approved":true}})
    );
    root["limits"]["max_step_instances"] = json!(34);
    let too_small = f.register(&root);
    assert!(f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":too_small,"input":{},"request_key":"rows-count"})).is_err());
}
