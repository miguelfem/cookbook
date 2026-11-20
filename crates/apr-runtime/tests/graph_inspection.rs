use apr_runtime::Runtime;
use serde_json::{Value, json};

fn leaf() -> Value {
    json!({"schema_version":"0.2","name":"review.leaf","description":"Synthetic caller review",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object"},"defaults":{"private":"DO_NOT_PROJECT_CONFIG"},"run_override_keys":[]},"capabilities":[{"kind":"builtin"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1000000,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[{"id":"ask","kind":"agent","effect":"agent","needs":[],"retry":{"max_attempts":1},"inputs":{"private":{"literal":"DO_NOT_PROJECT_INPUT"}},"instructions":"DO_NOT_PROJECT_INSTRUCTIONS","capability":"review","result_schema":{"type":"object"}}],
        "output":{"result":{"step":"ask","pointer":""}}})
}
fn parent(name: &str, child: &str) -> Value {
    let mut value = leaf();
    value["name"] = json!(name);
    value["steps"] = json!([{"id":"invoke","kind":"call","effect":"control","needs":[],"retry":{"max_attempts":1},"inputs":{},"call":{"process_ref":child,"output_pointer":""}}]);
    value["output"] = json!({"child":{"step":"invoke","pointer":""}});
    value
}
struct Fixture {
    directory: tempfile::TempDir,
    runtime: Runtime,
    token: String,
    other: String,
    leaf: String,
    middle: String,
    root: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let leaf = apr_core::admit_value(&leaf()).unwrap();
        let middle = apr_core::admit_value(&parent("review.middle", &leaf.version)).unwrap();
        let root = apr_core::admit_value(&parent("review.root", &middle.version)).unwrap();
        let mut runtime = Runtime::open(directory.path().join("state")).unwrap();
        let project_info = runtime.attach_project(&project).unwrap();
        let project_id = project_info["project_id"].as_str().unwrap();
        let assignment = runtime.create_assignment(project_id, "graph").unwrap();
        let other = runtime.create_assignment(project_id, "other").unwrap();
        let token = runtime
            .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let other = runtime
            .create_attachment(other["assignment_id"].as_str().unwrap(), "author")
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        for (name, process) in [("leaf", &leaf), ("middle", &middle), ("root", &root)] {
            let path = format!("{name}.json");
            std::fs::write(project.join(&path), &process.canonical_bytes).unwrap();
            runtime
                .scoped_call(
                    &token,
                    "manage",
                    json!({"action":"process.register","path":path}),
                )
                .unwrap();
        }
        Self {
            directory,
            runtime,
            token,
            other,
            leaf: leaf.version,
            middle: middle.version,
            root: root.version,
        }
    }
    fn inspect(&mut self, args: Value) -> Value {
        self.runtime
            .scoped_call(&self.token, "inspect", args)
            .unwrap()
    }
    fn start(&mut self) -> String {
        self.runtime
            .scoped_call(
                &self.token,
                "run",
                json!({"mode":"start","process_ref":self.root,"input":{},"request_key":"nested"}),
            )
            .unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn drive(&mut self) {
        for _ in 0..12 {
            self.runtime.drive_pending().unwrap();
        }
    }
}

#[test]
fn grandchild_handoff_is_discoverable_and_resumes_to_root_after_restart() {
    let mut f = Fixture::new();
    let catalog = f.inspect(json!({"view":"graph","process_ref":f.root}));
    assert_eq!(catalog["nodes"][0]["call"]["process_ref"], f.middle);
    assert!(catalog["run_id"].is_null());
    let root = f.start();
    f.drive();
    let tree = f.inspect(json!({"view":"tree","run_id":root}));
    assert_eq!(tree["runs"].as_array().unwrap().len(), 3);
    let child = tree["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["actionable_waiting"] == true)
        .unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let graph = f.inspect(json!({"view":"graph","run_id":&child}));
    assert_eq!(graph["process_ref"], f.leaf);
    assert_eq!(graph["root_run_id"], root);
    assert_eq!(graph["nodes"][0]["state"], "waiting_for_agent");
    for projected in [graph, tree] {
        let encoded = serde_json::to_string(&projected).unwrap();
        assert!(!encoded.contains("DO_NOT_PROJECT"));
        assert!(!encoded.contains("continuation_token"));
    }
    // A parent pause must suppress, rather than fabricate, an actionable child.
    f.runtime
        .scoped_call(&f.token, "resume", json!({"action":"pause","run_id":root}))
        .unwrap();
    let paused = f.inspect(json!({"view":"tree","run_id":child}));
    assert!(
        paused["runs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["actionable_waiting"] == false)
    );
    f.runtime
        .scoped_call(
            &f.token,
            "resume",
            json!({"action":"continue","run_id":root}),
        )
        .unwrap();
    drop(f.runtime);
    f.runtime = Runtime::open(f.directory.path().join("state")).unwrap();
    f.drive();
    let request = f.runtime.inspect(&child).unwrap().request.unwrap();
    f.runtime.scoped_call(&f.token,"resume",json!({"protocol_version":"0.1","action":"submit","run_id":child,"request_id":request.request_id,"step_instance_id":request.step_instance_id,"generation":request.generation,"submission_key":"reviewed","continuation_token":request.continuation_token,"output":{"approved":true},"provenance":{"source":"fixture"}})).unwrap();
    f.drive();
    assert_eq!(f.runtime.inspect(&root).unwrap().state, "succeeded");
    assert_eq!(
        f.runtime.inspect(&root).unwrap().output.unwrap(),
        json!({"child":{"child":{"result":{"approved":true}}}})
    );
    let graph = f.inspect(json!({"view":"graph","run_id":root}));
    assert_eq!(graph["nodes"][0]["instances"][0]["state"], "succeeded");
    assert!(graph["nodes"][0]["instances"][0]["child_run_id"].is_string());
}

#[test]
fn graph_and_tree_enforce_scope_cursors_caps_and_immutable_versions() {
    let mut f = Fixture::new();
    let root = f.start();
    f.drive();
    for args in [
        json!({"view":"graph","process_ref":f.root}),
        json!({"view":"graph","run_id":root}),
        json!({"view":"tree","run_id":root}),
        json!({"view":"tree_status","run_id":root}),
    ] {
        assert!(f.runtime.scoped_call(&f.other, "inspect", args).is_err());
    }
    for args in [
        json!({"view":"graph"}),
        json!({"view":"graph","process_ref":f.root,"run_id":root}),
        json!({"view":"graph","process_ref":f.root,"limit":101}),
        json!({"view":"graph","process_ref":f.root,"after":"unknown"}),
        json!({"view":"tree","run_id":root,"limit":0}),
        json!({"view":"tree","run_id":root,"after":"unknown"}),
    ] {
        assert!(f.runtime.scoped_call(&f.token, "inspect", args).is_err());
    }
    let mut ids = Vec::new();
    let summary = f.inspect(json!({"view":"tree_status","run_id":root}));
    assert_eq!(summary["total_runs"], 3);
    assert_eq!(summary["complete"], true);
    assert_eq!(summary["runnable_without_caller"], false);
    assert_eq!(summary["state_counts"]["waiting_for_agent"], 1);
    assert!(!summary.to_string().contains("DO_NOT_PROJECT"));
    let mut after: Option<String> = None;
    loop {
        let mut args = json!({"view":"tree","run_id":root,"limit":1});
        if let Some(cursor) = &after {
            args["after"] = json!(cursor);
        }
        let page = f.inspect(args);
        ids.push(page["runs"][0]["run_id"].as_str().unwrap().to_owned());
        if page["truncated"] == false {
            break;
        }
        after = Some(page["next_cursor"].as_str().unwrap().to_owned());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3);
    // Re-register a new same-name process: existing graph must remain frozen.
    let replacement = parent("review.root", &f.leaf);
    std::fs::write(
        f.directory.path().join("project/root.json"),
        serde_json::to_vec(&replacement).unwrap(),
    )
    .unwrap();
    f.runtime
        .scoped_call(
            &f.token,
            "manage",
            json!({"action":"process.register","path":"root.json"}),
        )
        .unwrap();
    let frozen = f.inspect(json!({"view":"graph","run_id":root}));
    assert_eq!(frozen["process_ref"], f.root);
    assert_eq!(frozen["nodes"][0]["call"]["process_ref"], f.middle);
}

#[test]
fn graph_pages_preserve_branch_edges_and_only_declared_selected_routes() {
    let mut f = Fixture::new();
    let mut process = leaf();
    process["name"] = json!("branch.fixture");
    process["steps"] = json!([
        {"id":"a","kind":"decision","effect":"pure","needs":[],"inputs":{},"retry":{"max_attempts":1},"decision":{"source":"rules","routes":["yes","no"],"cases":[{"value":{"literal":"PRIVATE_CASE"},"equals":"PRIVATE_CASE","route":"yes"}],"default":"no"}},
        {"id":"b","kind":"builtin","operation":"json.select","effect":"pure","needs":["a"],"retry":{"max_attempts":1},"when":{"decision":"a","route":"yes"},"inputs":{"value":{"literal":true},"pointer":{"literal":""}}}
    ]);
    process["output"] = json!({"route":{"step":"a","pointer":"/route"}});
    let admitted = apr_core::admit_value(&process).unwrap();
    std::fs::write(
        f.directory.path().join("project/branch.json"),
        &admitted.canonical_bytes,
    )
    .unwrap();
    f.runtime
        .scoped_call(
            &f.token,
            "manage",
            json!({"action":"process.register","path":"branch.json"}),
        )
        .unwrap();
    let page = f.inspect(json!({"view":"graph","process_ref":admitted.version,"limit":1}));
    assert_eq!(page["next_cursor"], "a");
    assert_eq!(page["truncated"], true);
    assert!(!page.to_string().contains("PRIVATE_CASE"));
    let page =
        f.inspect(json!({"view":"graph","process_ref":admitted.version,"limit":1,"after":"a"}));
    assert_eq!(page["edges"], json!([{"from":"a","to":"b","route":"yes"}]));
    assert_eq!(page["truncated"], false);
    let run=f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":admitted.version,"input":{},"request_key":"branch"})).unwrap();
    f.drive();
    let graph = f.inspect(json!({"view":"graph","run_id":run["run_id"]}));
    assert_eq!(graph["nodes"][0]["selected_route"], "yes");
}

#[test]
fn legacy_omitted_depth_still_limits_calls_to_eight() {
    let mut f = Fixture::new();
    let mut target = f.leaf.clone();
    for depth in 1..=9 {
        let process = apr_core::admit_value(&parent(&format!("depth.{depth}"), &target)).unwrap();
        let path = format!("depth-{depth}.json");
        std::fs::write(
            f.directory.path().join("project").join(&path),
            &process.canonical_bytes,
        )
        .unwrap();
        f.runtime
            .scoped_call(
                &f.token,
                "manage",
                json!({"action":"process.register","path":path}),
            )
            .unwrap();
        target = process.version;
        if depth == 8 {
            f.root = target.clone();
        }
    }
    assert!(
        f.runtime
            .scoped_call(
                &f.token,
                "run",
                json!({"mode":"start","process_ref":target,"input":{},"request_key":"too-deep"})
            )
            .is_err()
    );
    let root = f.start();
    for _ in 0..25 {
        f.runtime.drive_pending().unwrap();
    }
    let tree = f.inspect(json!({"view":"tree","run_id":root,"limit":100}));
    assert_eq!(tree["runs"].as_array().unwrap().len(), 9);
    assert_eq!(
        tree["runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["actionable_waiting"] == true)
            .count(),
        1
    );
    let mut run = root;
    for depth in 0..=8 {
        let graph = f.inspect(json!({"view":"graph","run_id":run}));
        assert_eq!(
            graph["nodes"].as_array().unwrap().len(),
            1,
            "child graph must not be eagerly embedded"
        );
        if depth < 8 {
            run = graph["nodes"][0]["instances"][0]["child_run_id"]
                .as_str()
                .unwrap()
                .to_owned();
        }
    }
}

#[test]
fn foreach_child_previews_are_bounded_and_tree_keeps_all_created_children() {
    let mut f = Fixture::new();
    let mut process = parent("many.children", &f.leaf);
    process["limits"]["max_agent_requests"] = json!(25);
    process["steps"][0]["foreach"] =
        json!({"items":{"input":"/items"},"key":"/id","max_items":25,"concurrency":25});
    let admitted = apr_core::admit_value(&process).unwrap();
    std::fs::write(
        f.directory.path().join("project/many.json"),
        &admitted.canonical_bytes,
    )
    .unwrap();
    f.runtime
        .scoped_call(
            &f.token,
            "manage",
            json!({"action":"process.register","path":"many.json"}),
        )
        .unwrap();
    let items: Vec<_> = (0..25).map(|i| json!({"id":format!("item-{i}")})).collect();
    let run = f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":admitted.version,"input":{"items":items},"request_key":"many"})).unwrap()["run_id"].as_str().unwrap().to_owned();
    for _ in 0..35 {
        f.runtime.drive_pending().unwrap();
    }
    let graph = f.inspect(json!({"view":"graph","run_id":run}));
    assert_eq!(graph["nodes"][0]["instance_count"], 25);
    assert_eq!(graph["nodes"][0]["instances"].as_array().unwrap().len(), 20);
    assert_eq!(graph["nodes"][0]["instances_truncated"], true);
    let tree = f.inspect(json!({"view":"tree","run_id":run,"limit":100}));
    assert_eq!(tree["runs"].as_array().unwrap().len(), 26);
}
