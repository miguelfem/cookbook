use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Fixture {
    service: Child,
    _temporary: tempfile::TempDir,
    state: PathBuf,
    project: String,
    _assignment: String,
    author_file: PathBuf,
    run_file: PathBuf,
}

fn cli(state: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_apr"))
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

fn path(value: &Path) -> &str {
    value.to_str().unwrap()
}

fn process() -> Value {
    json!({
        "schema_version":"0.1","name":"mcp.local-review","description":"Review a local record through an explicit caller handoff",
        "input_schema":{"type":"object","required":["record"],"properties":{"record":{"type":"string"}},"additionalProperties":false},
        "output_schema":{"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"agent","capability":"review"}],
        "limits":{"max_step_ms":10000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[{"id":"review","kind":"agent","effect":"agent","needs":[],"retry":{"max_attempts":1},
            "capability":"review","instructions":"Read the record and return an observed review decision.",
            "inputs":{"record":{"input":"/record"}},
            "result_schema":{"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false}}],
        "output":{"approved":{"step":"review","pointer":"/approved"}}
    })
}

fn pure_child_process(name: &str) -> Value {
    json!({
        "schema_version":"0.2","name":name,"description":"Copy one synthetic value without external work",
        "input_schema":{"type":"object","required":["value"],"properties":{"value":{}},"additionalProperties":false},
        "output_schema":{"type":"object","required":["value"],"properties":{"value":{}},"additionalProperties":false},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"builtin"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0,
            "max_call_depth":4,"max_child_runs":4,"max_step_instances":8},
        "steps":[{"id":"copy","kind":"builtin","operation":"json.select","effect":"pure","needs":[],
            "retry":{"max_attempts":1},"inputs":{"value":{"input":"/value"},"pointer":{"literal":""}}}],
        "output":{"value":{"step":"copy","pointer":"/value"}}
    })
}

fn call_process(name: &str, child: &str) -> Value {
    json!({
        "schema_version":"0.2","name":name,"description":"Call one pinned synthetic child",
        "input_schema":{"type":"object","required":["value"],"properties":{"value":{}},"additionalProperties":false},
        "output_schema":{"type":"object","required":["value"],"properties":{"value":{}},"additionalProperties":false},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0,
            "max_call_depth":4,"max_child_runs":4,"max_step_instances":8},
        "steps":[{"id":"child","kind":"call","effect":"control","needs":[],"retry":{"max_attempts":1},
            "inputs":{"value":{"input":"/value"}},"call":{"process_ref":child,"output_pointer":""}}],
        "output":{"value":{"step":"child","pointer":"/value"}}
    })
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let project_path = temporary.path().join("project with spaces");
        fs::create_dir(&project_path).unwrap();
        fs::write(
            project_path.join("review.json"),
            serde_json::to_vec(&process()).unwrap(),
        )
        .unwrap();
        let attached = cli(&state, &["project", "attach", path(&project_path)]);
        let project = attached["project_id"].as_str().unwrap().to_owned();
        let created = cli(
            &state,
            &[
                "assignment",
                "create",
                "--project",
                &project,
                "--name",
                "MCP fixture",
            ],
        );
        let assignment = created["assignment_id"].as_str().unwrap().to_owned();
        let author_file = state.join("attachments/author attachment.json");
        let run_file = state.join("attachments/run attachment.json");
        for (mode, file) in [("author", &author_file), ("run", &run_file)] {
            let metadata = cli(
                &state,
                &[
                    "attachment",
                    "create",
                    "--assignment",
                    &assignment,
                    "--mode",
                    mode,
                    "--output-file",
                    path(file),
                ],
            );
            let saved: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
            assert!(
                !metadata
                    .to_string()
                    .contains(saved["token"].as_str().unwrap())
            );
            assert!(metadata.get("token").is_none());
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // Own the foreground service process so cleanup cannot affect unrelated
        // services. The separate service integration suite covers autostart.
        let mut service = Command::new(env!("CARGO_BIN_EXE_apr"))
            .arg("--state-dir")
            .arg(&state)
            .arg("service")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let socket = apr_runtime::service::socket_path(&state.canonicalize().unwrap()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while UnixStream::connect(&socket).is_err() {
            assert!(
                service.try_wait().unwrap().is_none(),
                "service exited during startup"
            );
            assert!(Instant::now() < deadline, "service never became ready");
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            service,
            _temporary: temporary,
            state,
            project,
            _assignment: assignment,
            author_file,
            run_file,
        }
    }

    fn register(&self, client: &mut Mcp) -> String {
        let result = client.tool(
            "apr_manage",
            json!({"action":"process.register","path":"review.json"}),
        );
        result["process_ref"].as_str().unwrap().to_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.service.kill();
        let _ = self.service.wait();
    }
}

struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<Value>,
    next_id: u64,
}

impl Mcp {
    fn start(attachment: &Path) -> Self {
        Self::launch(&["mcp", "--attachment-file", path(attachment)])
    }

    fn projects(state: &Path) -> Self {
        Self::launch(&["--state-dir", path(state), "mcp", "--projects"])
    }

    fn launch(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_apr"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                let value: Value = serde_json::from_str(&line)
                    .expect("MCP stdout must contain only JSON-RPC frames");
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            messages,
            next_id: 1,
        };
        let response = client.request(
            "initialize",
            json!({
                "protocolVersion":"2025-11-25","capabilities":{},
                "clientInfo":{"name":"apr-protocol-test","version":"0.1"}
            }),
        );
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response["result"]["serverInfo"]["name"], "apr");
        client.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        client
    }

    fn send(&mut self, value: &Value) {
        let input = self.input.as_mut().unwrap();
        serde_json::to_writer(&mut *input, value).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let message = self
                .messages
                .recv_timeout(Duration::from_secs(15))
                .expect("MCP response timed out or bridge exited");
            if message["id"] == id {
                return message;
            }
        }
    }

    fn tool_response(&mut self, name: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":args}))
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        let message = self.tool_response(name, args);
        assert!(message.get("error").is_none(), "{message}");
        let result = &message["result"];
        assert_ne!(result["isError"], true, "{result}");
        assert_eq!(
            result["content"],
            json!([]),
            "large structured output must not repeat as text"
        );
        result["structuredContent"].clone()
    }

    fn wait_for(&mut self, run_id: &str, state: &str, detail: bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let run = self.tool(
                "apr_inspect",
                json!({"view":"run","run_id":run_id,"detail":detail}),
            );
            if run["state"] == state {
                return run;
            }
            assert_ne!(run["state"], "failed", "{run}");
            assert!(Instant::now() < deadline, "expected {state}: {run}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn close(mut self) {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(
                Instant::now() < deadline,
                "bridge did not stop after stdin closed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn submission(waiting: &Value) -> Value {
    let request = &waiting["request"];
    json!({
        "protocol_version":"0.1","action":"submit","run_id":waiting["run_id"],
        "request_id":request["request_id"],"step_instance_id":request["step_instance_id"],
        "generation":request["generation"],"continuation_token":request["continuation_token"],
        "submission_key":"observed-review-1","output":{"approved":true},
        "provenance":{"source":"caller_tool","client":"mcp-protocol-test","verification":"synthetic_fixture"}
    })
}

#[test]
fn short_recipe_returns_output_in_the_start_call() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    fs::write(
        project.join("copy.json"),
        include_str!("../../../examples/recipe-improvement/candidate.json"),
    )
    .unwrap();
    let mut author = Mcp::start(&fixture.author_file);
    let registered = author.tool(
        "apr_manage",
        json!({"action":"process.register","path":"copy.json"}),
    );
    let mut client = Mcp::start(&fixture.run_file);
    let args = json!({"process_ref":registered["process_ref"],"mode":"start",
        "request_key":"one-call-result","input":{"answer":"A","evidence":"doc A"}});
    let completed = client.tool("apr_run", args.clone());
    assert_eq!(completed["state"], "succeeded", "{completed}");
    assert_eq!(
        completed["output"],
        json!({"answer":"A","evidence":"doc A"})
    );
    let replay = client.tool("apr_run", args.clone());
    assert_eq!(replay["run_id"], completed["run_id"]);
    assert_eq!(replay["output"], completed["output"]);
    let mut compact_args = args.clone();
    compact_args["result_view"] = json!("compact");
    let compact = client.tool("apr_run", compact_args);
    assert_eq!(compact["output"], completed["output"]);
    assert_eq!(compact["output_complete"], true);
    assert_eq!(compact["result_view"], "compact");
    assert!(compact.get("steps").is_none());
    assert!(compact.get("summary").is_none());
    for field in [
        "protocol_version",
        "run_id",
        "root_run_id",
        "process_version",
        "config_digest",
        "fixture",
        "revision",
        "state",
        "truncated",
    ] {
        assert_eq!(compact[field], completed[field], "receipt field {field}");
    }
    assert!(
        serde_json::to_vec(&compact).unwrap().len() < serde_json::to_vec(&completed).unwrap().len()
    );
    let audit = client.tool(
        "apr_inspect",
        json!({"view":"run","run_id":compact["run_id"],"detail":true}),
    );
    assert_eq!(
        audit, completed,
        "full inspection retains the original audit"
    );
    let mut conflict = args;
    conflict["input"]["answer"] = json!("different");
    conflict["result_view"] = json!("compact");
    let refused = client.tool_response("apr_run", conflict);
    assert_eq!(refused["result"]["isError"], true);

    // Completion must still honor the small result budget. Explicit detail
    // retrieves the intact output rather than returning a sliced success.
    let evidence = "large evidence ".repeat(1000);
    let large = client.tool(
        "apr_run",
        json!({"process_ref":registered["process_ref"],
        "mode":"start","request_key":"large-one-call-result",
        "result_view":"compact",
        "input":{"answer":"A","evidence":evidence}}),
    );
    assert_eq!(large["state"], "succeeded", "{large}");
    assert_eq!(large["truncated"], true);
    assert!(large.get("output").is_none());
    assert!(large.get("output_complete").is_none());
    let detailed = client.tool(
        "apr_inspect",
        json!({"view":"run",
        "run_id":large["run_id"],"detail":true}),
    );
    assert_eq!(detailed["output"]["evidence"], evidence);
}

#[test]
fn short_nested_children_return_complete_output_in_the_start_call() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    let mut client = Mcp::start(&fixture.author_file);
    fs::write(
        project.join("nested-leaf.json"),
        serde_json::to_vec(&pure_child_process("mcp.nested-leaf")).unwrap(),
    )
    .unwrap();
    let leaf = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"nested-leaf.json"}),
    )["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(
        project.join("nested-middle.json"),
        serde_json::to_vec(&call_process("mcp.nested-middle", &leaf)).unwrap(),
    )
    .unwrap();
    let middle = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"nested-middle.json"}),
    )["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(
        project.join("nested-root.json"),
        serde_json::to_vec(&call_process("mcp.nested-root", &middle)).unwrap(),
    )
    .unwrap();
    let root = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"nested-root.json"}),
    )["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();

    let completed = client.tool(
        "apr_run",
        json!({"process_ref":root,"mode":"start","wait_ms":5000,"result_view":"compact",
            "request_key":"nested-one-call-result","input":{"value":{"answer":42}}}),
    );
    assert_eq!(completed["state"], "succeeded", "{completed}");
    assert_eq!(completed["output"], json!({"value":{"answer":42}}));
    assert_eq!(completed["output_complete"], true);
    assert_eq!(completed["result_view"], "compact");
}

#[test]
fn sixty_four_composed_jobs_return_complete_output_without_tree_pagination() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    let mut client = Mcp::start(&fixture.author_file);
    fs::write(
        project.join("many-leaf.json"),
        serde_json::to_vec(&pure_child_process("mcp.many-leaf")).unwrap(),
    )
    .unwrap();
    let leaf = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"many-leaf.json"}),
    )["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut parent = call_process("mcp.many-parent", &leaf);
    parent["limits"]["max_child_runs"] = json!(64);
    parent["limits"]["max_step_instances"] = json!(256);
    parent["steps"][0]["foreach"] =
        json!({"items":{"input":"/value"},"key":"/id","max_items":64,"concurrency":64});
    parent["steps"][0]["inputs"] = json!({"value":{"item":""}});
    parent["output"] = json!({"value":{"step":"child","pointer":"/items"}});
    fs::write(
        project.join("many-parent.json"),
        serde_json::to_vec(&parent).unwrap(),
    )
    .unwrap();
    let root = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"many-parent.json"}),
    )["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let jobs: Vec<_> = (0..64)
        .map(|i| json!({"id":format!("job-{i}"),"text":format!("  evidence {i} 東京  ")}))
        .collect();
    let result = client.tool("apr_run", json!({"process_ref":root,"mode":"start","wait_ms":5000,"result_view":"compact","request_key":"many-composed","input":{"value":jobs}}));
    assert_eq!(result["state"], "succeeded", "{result}");
    let expected: Vec<_> = jobs
        .iter()
        .map(|item| json!({"key":item["id"],"output":{"value":item}}))
        .collect();
    assert_eq!(result["output"], json!({"value":expected}));
    let status = client.tool(
        "apr_inspect",
        json!({"view":"tree_status","run_id":result["run_id"]}),
    );
    assert_eq!(status["total_runs"], 65);
    assert_eq!(status["state_counts"]["succeeded"], 65);
    assert_eq!(status["complete"], true);
    assert!(serde_json::to_vec(&status).unwrap().len() < 1024);
}

#[test]
fn composed_run_returns_promptly_for_an_actionable_child_handoff() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    let mut client = Mcp::start(&fixture.author_file);
    let child = fixture.register(&mut client);
    let mut parent = call_process("mcp.review-parent", &child);
    parent["input_schema"] = process()["input_schema"].clone();
    parent["output_schema"] = process()["output_schema"].clone();
    parent["limits"]["max_agent_requests"] = json!(1);
    parent["steps"][0]["inputs"] = json!({"record":{"input":"/record"}});
    parent["output"] = json!({"approved":{"step":"child","pointer":"/approved"}});
    fs::write(
        project.join("review-parent.json"),
        serde_json::to_vec(&parent).unwrap(),
    )
    .unwrap();
    let parent_ref = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"review-parent.json"}),
    )["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();

    let started_at = Instant::now();
    let waiting = client.tool(
        "apr_run",
        json!({"process_ref":parent_ref,"mode":"start","wait_ms":5000,
            "request_key":"nested-caller-wait","input":{"record":"review this"}}),
    );
    assert_eq!(waiting["state"], "waiting_for_child", "{waiting}");
    assert!(
        started_at.elapsed() < Duration::from_secs(2),
        "actionable handoff consumed the wait budget: {waiting}"
    );
    let tree = client.tool(
        "apr_inspect",
        json!({"view":"tree","run_id":waiting["run_id"],"limit":10}),
    );
    let child_run_id = tree["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["actionable_waiting"] == true)
        .expect("child handoff must be actionable")["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let child_waiting = client.tool(
        "apr_inspect",
        json!({"view":"run","run_id":child_run_id,"detail":true}),
    );
    assert_eq!(child_waiting["state"], "waiting_for_agent");
    assert!(child_waiting["request"]["request_id"].is_string());
}

#[test]
fn compact_run_receipt_omits_successful_step_metadata_without_losing_output() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    let mut manifest: Value = serde_json::from_str(include_str!(
        "../../../examples/recipe-improvement/candidate.json"
    ))
    .unwrap();
    let step = manifest["steps"][0].clone();
    let mut steps = vec![step.clone()];
    for index in 1..16 {
        let mut extra = step.clone();
        extra["id"] = json!(format!("copy_{index}"));
        steps.push(extra);
    }
    manifest["steps"] = json!(steps);
    manifest["limits"]["max_step_instances"] = json!(16);
    fs::write(
        project.join("many.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut client = Mcp::start(&fixture.author_file);
    let registered = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"many.json"}),
    );
    let mut args = json!({"process_ref":registered["process_ref"],"mode":"start","wait_ms":5000,
        "request_key":"many-step-output","input":{"answer":"Every required field","evidence":"Exact evidence"}});
    let full = client.tool("apr_run", args.clone());
    assert_eq!(full["state"], "succeeded", "{full}");
    assert_eq!(full["steps"].as_array().unwrap().len(), 16);
    args["result_view"] = json!("compact");
    let compact = client.tool("apr_run", args.clone());
    assert_eq!(compact["run_id"], full["run_id"]);
    assert_eq!(compact["output"], full["output"]);
    assert_eq!(compact["output_complete"], true);
    let full_bytes = serde_json::to_vec(&full).unwrap().len();
    let compact_bytes = serde_json::to_vec(&compact).unwrap().len();
    assert!(
        compact_bytes * 2 < full_bytes,
        "{compact_bytes} vs {full_bytes}"
    );
    eprintln!(
        "compact receipt: {full_bytes} -> {compact_bytes} serialized bytes; 16 steps, same complete output"
    );
    args["result_view"] = json!("output");
    let output = client.tool("apr_run", args.clone());
    assert_eq!(
        output,
        json!({"run_id":full["run_id"],"state":"succeeded","result_view":"output",
               "output_complete":true,"output":full["output"]})
    );
    assert!(serde_json::to_vec(&output).unwrap().len() < compact_bytes);

    args["result_view"] = json!("receipt");
    let receipt = client.tool("apr_run", args);
    assert_eq!(
        receipt,
        json!({"run_id":full["run_id"],"state":"succeeded","result_view":"receipt",
               "output_available":true})
    );
    let audit = client.tool(
        "apr_inspect",
        json!({"view":"run","run_id":full["run_id"],"detail":true}),
    );
    assert_eq!(audit, full);
}

#[test]
fn output_view_returns_a_complete_result_beyond_the_default_reply_budget() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    fs::write(
        project.join("large.json"),
        include_bytes!("../../../examples/recipe-improvement/candidate.json"),
    )
    .unwrap();
    let mut client = Mcp::start(&fixture.author_file);
    let registered = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"large.json"}),
    );
    let input = json!({"answer":"Exact answer","evidence":"synthetic ".repeat(1100)});
    let mut args = json!({"process_ref":registered["process_ref"],"mode":"start","wait_ms":5000,
        "request_key":"large-output","input":input});
    let full = client.tool("apr_run", args.clone());
    assert_eq!(full["state"], "succeeded");
    assert_eq!(full["truncated"], true);
    args["result_view"] = json!("output");
    let result = client.tool("apr_run", args);
    assert_eq!(result["run_id"], full["run_id"]);
    assert_eq!(result["output_complete"], true);
    assert_eq!(result["output"], input);
}

#[test]
fn run_wait_can_be_disabled_and_returns_caller_handoffs() {
    let fixture = Fixture::new();
    let mut client = Mcp::start(&fixture.author_file);
    let reference = fixture.register(&mut client);
    let mut args = json!({"process_ref":reference,"mode":"start","wait_ms":0,
        "result_view":"compact",
        "request_key":"caller-wait","input":{"record":"review this"}});
    let queued = client.tool("apr_run", args.clone());
    assert_eq!(queued["state"], "queued");
    assert!(queued.get("result_view").is_none());
    args["wait_ms"] = json!(1000);
    let waiting = client.tool("apr_run", args.clone());
    assert_eq!(waiting["run_id"], queued["run_id"]);
    assert_eq!(waiting["state"], "waiting_for_agent", "{waiting}");
    assert!(waiting["request"]["request_id"].is_string());
    assert!(waiting.get("result_view").is_none());
    for view in ["output", "receipt"] {
        args["result_view"] = json!(view);
        let pending = client.tool("apr_run", args.clone());
        assert_eq!(pending["request"], waiting["request"]);
        assert_eq!(pending["state"], "waiting_for_agent");
        assert!(pending.get("result_view").is_none());
    }
    client.tool("apr_resume", submission(&waiting));
    client.wait_for(queued["run_id"].as_str().unwrap(), "succeeded", false);

    args["wait_ms"] = json!(5001);
    args["request_key"] = json!("invalid-wait");
    assert_eq!(
        client.tool_response("apr_run", args)["result"]["isError"],
        true
    );
}

#[test]
fn improvement_mcp_to_scoped_cli_report_and_adoption() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    let mut client = Mcp::start(&fixture.author_file);
    let guide = client.tool("apr_discover", json!({"guide":"recipe_improvement"}));
    fs::write(
        project.join("baseline.json"),
        serde_json::to_vec(&guide["content"]["baseline_manifest"]).unwrap(),
    )
    .unwrap();
    fs::write(
        project.join("candidate.json"),
        serde_json::to_vec(&guide["content"]["candidate_manifest"]).unwrap(),
    )
    .unwrap();
    let baseline = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"baseline.json"}),
    )["process_ref"]
        .as_str()
        .unwrap()
        .to_owned();
    let run=client.tool("apr_run",json!({"process_ref":baseline,"mode":"start","input":{"answer":"A","evidence":"doc A"},"request_key":"improvement-source"}));
    let run_id = run["run_id"].as_str().unwrap();
    client.wait_for(run_id, "succeeded", true);
    let cycle=client.tool("apr_manage",json!({"action":"improvement.open","request_key":"improve-mcp-1","process_ref":baseline,"run_ids":[run_id],"detail":true}));
    let id = cycle["cycle_id"].as_str().unwrap();
    client.tool("apr_manage",json!({"action":"improvement.propose","cycle_id":id,"candidate_path":"candidate.json",
        "proposal":{"hypothesis":"avoid redundant selection","change":"select directly","benefit":"one less step","risks":"omitted evidence","evidence_run_ids":[run_id]},"review":{},"detail":true}));
    client.tool("apr_manage",json!({"action":"improvement.freeze","cycle_id":id,"policy":{"evaluator_version":"pure-v1","split_method":"independent entities","min_step_reduction":1},
        "development":[{"case_id":"dev","source_id":"dev-src","input":{"answer":"D","evidence":"doc D"},"expected":{"answer":"D","evidence":"doc D"},"provenance":"human"}],
        "holdout":[{"case_id":"held","source_id":"held-src","input":{"answer":"H","evidence":"doc H"},"expected":{"answer":"H","evidence":"doc H"},"provenance":"human"}],"detail":true}));
    let evaluated = client.tool(
        "apr_manage",
        json!({"action":"improvement.evaluate","cycle_id":id,"detail":true}),
    );
    assert_eq!(evaluated["report"]["result"], "pass", "{evaluated}");
    let request = fixture._temporary.path().join("improvement-read.json");
    fs::write(
        &request,
        serde_json::to_vec(&json!({"view":"improvement","cycle_id":id})).unwrap(),
    )
    .unwrap();
    let from_cli = cli(
        &fixture.state,
        &[
            "improvement",
            "--attachment",
            path(&fixture.author_file),
            "--request-file",
            path(&request),
            "--inspect",
        ],
    );
    assert_eq!(from_cli["report_digest"], evaluated["report_digest"]);
    let digest = evaluated["report_digest"].as_str().unwrap();
    let adopted=client.tool("apr_manage",json!({"action":"improvement.decide","cycle_id":id,"decision":"adopt","candidate_path":"candidate.json","expected_report_digest":digest,"expected_selected_ref":null,"detail":true}));
    assert_eq!(adopted["state"], "adopted");
    let discovered = client.tool("apr_discover", json!({"query":"synthetic.copy-review"}));
    assert!(
        discovered["processes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["selected_for_future_work"] == true
                && p["process_ref"] == adopted["candidate_ref"])
    );
    client.close();
}

#[test]
fn bundled_guides_bootstrap_an_empty_catalog_without_a_source_checkout() {
    let fixture = Fixture::new();
    let mut client = Mcp::start(&fixture.author_file);
    let mut guides = serde_json::Map::new();
    for topic in [
        "overview",
        "workflow_design",
        "authoring",
        "execution_review",
        "schema",
        "schema_v0_1",
        "starter",
        "routing",
        "jev",
        "classifier_learning",
        "recipe_improvement",
        "connections",
        "adapters",
        "handoffs",
        "schedules",
    ] {
        let guide = client.tool("apr_discover", json!({"guide":topic}));
        assert_eq!(guide["guide"], topic);
        assert_eq!(guide["runtime"]["features"]["jev"], cfg!(feature = "jev"));
        assert_eq!(guide["runtime"]["features"]["classifier_learning"], true);
        assert_eq!(guide["runtime"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(serde_json::to_vec(&guide).unwrap().len() < 64 * 1024);
        guides.insert(topic.into(), guide);
    }
    // Capability routes must resolve through the advertised installed guide API.
    for route in guides["overview"]["content"]["possibilities"]
        .as_array()
        .unwrap()
    {
        let topic = route["guide"].as_str().unwrap();
        assert!(
            guides.contains_key(topic),
            "unreachable capability: {topic}"
        );
    }
    assert_eq!(guides["schema"]["content"]["schema"]["type"], "object");
    let adapter_cli = guides["adapters"]["content"]["cli_executable"]
        .as_str()
        .unwrap();
    assert!(std::path::Path::new(adapter_cli).is_absolute());
    assert_eq!(
        fs::canonicalize(adapter_cli).unwrap(),
        fs::canonicalize(env!("CARGO_BIN_EXE_apr")).unwrap()
    );
    assert_eq!(guides["adapters"]["content"]["grant_created"], false);
    let reference = &guides["authoring"]["content"]["reference_first_template"];
    assert!(
        reference["readme"]
            .as_str()
            .unwrap()
            .contains("trace compiler")
    );
    assert_eq!(
        reference["contract"]["status"],
        "template_requires_observed_evidence"
    );
    assert!(
        reference["evidence_note"]
            .as_str()
            .unwrap()
            .contains("not a completed run")
    );
    assert_eq!(
        guides["jev"]["content"]["sample_input"]["evidence_ref"],
        "fixture:ticket:jev-routing-1"
    );
    let invalid = client.tool_response("apr_discover", json!({"guide":"schema","query":"x"}));
    assert_eq!(
        invalid["result"]["structuredContent"]["error"]["code"],
        "INVALID_INPUT"
    );

    // Only the installed MCP's returned manifests are available in this project.
    // Jev is admitted but never called; ordinary examples run with no accounts.
    let project = fixture._temporary.path().join("project with spaces");
    let review = &guides["execution_review"]["content"]["service"];
    fs::write(
        project.join("reviewed.json"),
        serde_json::to_vec(&review["manifest"]).unwrap(),
    )
    .unwrap();
    let audit=client.tool("apr_manage",json!({"action":"process.audit","path":"reviewed.json","review":review["review"],"detail":true}));
    assert_eq!(audit["audit"]["advisory_only"], true);
    assert_eq!(audit["audit"]["agent_steps"], 0);
    assert_eq!(
        client.tool("apr_inspect", json!({"view":"connection_library"}))["connections"],
        json!([])
    );

    // Exercise the real project-file -> service IPC -> MCP error seam. The
    // bounded admission detail must help repair the script contract without
    // echoing private manifest values or device-specific paths.
    let mut unsafe_native: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    unsafe_native["description"] = json!("PRIVATE_MANIFEST_VALUE");
    unsafe_native["steps"][0]["effect"] = json!("pure");
    fs::write(
        project.join("unsafe-native.json"),
        serde_json::to_vec(&unsafe_native).unwrap(),
    )
    .unwrap();
    let failed = client.tool_response(
        "apr_manage",
        json!({"action":"process.validate","path":"unsafe-native.json"}),
    );
    let error = &failed["result"]["structuredContent"]["error"];
    assert_eq!(error["code"], "INVALID_MANIFEST");
    assert_eq!(error["diagnostic"]["field"], "steps[].script");
    assert_eq!(
        error["diagnostic"]["reason"],
        "invalid_python_step_contract"
    );
    assert!(!failed.to_string().contains("PRIVATE_MANIFEST_VALUE"));
    assert!(!failed.to_string().contains("unsafe-native.json"));
    let learning = &guides["classifier_learning"]["content"];
    assert_eq!(learning["learning_enabled_by_default"], false);
    for field in [
        "classifier_manifest",
        "proposal_manifest",
        "evaluation_manifest",
    ] {
        let path = format!("{field}.json");
        fs::write(
            project.join(&path),
            serde_json::to_vec(&learning[field]).unwrap(),
        )
        .unwrap();
        client.tool(
            "apr_manage",
            json!({"action":"process.validate","path":path}),
        );
        client.tool(
            "apr_manage",
            json!({"action":"process.register","path":path}),
        );
    }
    let proposed = client.tool("apr_manage", json!({"action":"connection.propose",
        "name":"classifier-demo","request_key":"classifier-proposal","rationale":"Synthetic installed guide contract validation; no grant or call",
        "connection":learning["connection_example"]}));
    assert_eq!(proposed["state"], "pending");
    for topic in ["starter", "routing", "jev"] {
        let content = &guides[topic]["content"];
        let relative = format!("{topic}.json");
        fs::write(
            project.join(&relative),
            serde_json::to_vec(&content["manifest"]).unwrap(),
        )
        .unwrap();
        client.tool(
            "apr_manage",
            json!({"action":"process.validate","path":relative}),
        );
        let registered = client.tool(
            "apr_manage",
            json!({"action":"process.register","path":relative}),
        );
        if topic == "jev" {
            assert_eq!(content["live_execution_authorized"], false);
            let proposed = client.tool("apr_manage", json!({"action":"connection.propose",
                "name":"typesafe","request_key":"first-jev-proposal","rationale":"Synthetic guide admission only; no execution or approval",
                "connection":content["connection_example"]}));
            assert_eq!(proposed["state"], "pending");
            continue;
        }
        let args = json!({"process_ref":registered["process_ref"],"input":content["sample_input"],
            "request_key":format!("first-{topic}"),"mode":"preflight"});
        let preflight = client.tool("apr_run", args.clone());
        assert!(preflight.get("run_id").is_none());
        let mut args = args;
        args["mode"] = json!("start");
        let started = client.tool("apr_run", args);
        let completed = client.wait_for(started["run_id"].as_str().unwrap(), "succeeded", true);
        if topic == "starter" {
            let artifact = client.tool("apr_inspect", json!({"view":"artifact","reference":completed["output"]["saved_note"],"detail":true}));
            assert!(
                artifact.to_string().contains("My first CookBook recipe"),
                "{artifact}"
            );
        } else {
            assert_eq!(completed["output"]["result"]["output"]["queue"], "standard");
        }
    }
    client.close();
    let mut reader = Mcp::start(&fixture.run_file);
    assert_eq!(
        reader.tool("apr_discover", json!({"guide":"overview"}))["tool_mode"],
        "run"
    );
    let tools = reader.request("tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 4);
    reader.close();
}

#[test]
fn scheduled_work_survives_client_disconnect_and_pauses_for_caller() {
    let fixture = Fixture::new();
    let mut author = Mcp::start(&fixture.author_file);
    let process_ref = fixture.register(&mut author);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    // Due shortly after the initial connection closes: the service owns firing.
    let create = json!({"action":"schedule.create","name":"Scheduled review","process_ref":process_ref,
        "input":{"record":"Scheduled record"},"request_key":"scheduled-review-1",
        "trigger":{"kind":"once","at_ms":now + 700},"detail":true});
    let saved = author.tool("apr_manage", create.clone());
    let schedule_id = saved["schedule_id"]
        .as_str()
        .expect("schedule identity")
        .to_owned();
    let duplicate = author.tool("apr_manage", create);
    assert_eq!(duplicate["schedule_id"], schedule_id);
    author.close();

    let mut reader = Mcp::start(&fixture.run_file);
    let deadline = Instant::now() + Duration::from_secs(8);
    let run_id = loop {
        let runs = reader.tool("apr_inspect", json!({"view":"runs","detail":true}));
        if let Some(run) = runs["runs"].as_array().unwrap().first() {
            break run["run_id"].as_str().unwrap().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "scheduled service did not fire: {runs}"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    let waiting = reader.wait_for(&run_id, "waiting_for_agent", true);
    assert_eq!(waiting["request"]["input"]["record"], "Scheduled record");
    let schedule = reader.tool(
        "apr_inspect",
        json!({"view":"schedule","schedule_id":schedule_id,"detail":true}),
    );
    assert_eq!(schedule["schedule_id"], schedule_id);
    reader.tool("apr_resume", submission(&waiting));
    reader.wait_for(&run_id, "succeeded", false);
    let runs = reader.tool("apr_inspect", json!({"view":"runs","detail":true}));
    assert_eq!(runs["runs"].as_array().unwrap().len(), 1);
    reader.close();
}

#[test]
fn schedule_cli_creates_and_controls_a_registered_process() {
    let fixture = Fixture::new();
    let mut author = Mcp::start(&fixture.author_file);
    let process_ref = fixture.register(&mut author);
    author.close();
    let input = fixture._temporary.path().join("scheduled-input.json");
    fs::write(
        &input,
        json!({"record":"A CLI scheduled record"}).to_string(),
    )
    .unwrap();
    let created = cli(
        &fixture.state,
        &[
            "schedule",
            "create",
            "--attachment",
            path(&fixture.author_file),
            "--name",
            "CLI scheduled review",
            "--process-ref",
            &process_ref,
            "--input-file",
            path(&input),
            "--request-key",
            "cli-schedule",
            "--every",
            "3600",
            "--start-at",
            "2030-01-02T09:00:00-06:00",
        ],
    );
    let id = created["schedule_id"].as_str().unwrap();
    assert_eq!(created["state"], "active");
    assert_eq!(created["trigger"]["every_seconds"], 3600);
    let detail = cli(
        &fixture.state,
        &[
            "schedule",
            "inspect",
            id,
            "--attachment",
            path(&fixture.author_file),
        ],
    );
    assert_eq!(detail["input"]["record"], "A CLI scheduled record");
    for (action, state) in [
        ("pause", "paused"),
        ("resume", "active"),
        ("delete", "deleted"),
    ] {
        assert_eq!(
            cli(
                &fixture.state,
                &[
                    "schedule",
                    action,
                    id,
                    "--attachment",
                    path(&fixture.author_file)
                ]
            )["state"],
            state
        );
    }
    let listed = cli(
        &fixture.state,
        &[
            "schedule",
            "list",
            "--attachment",
            path(&fixture.author_file),
        ],
    );
    assert!(listed["schedules"].as_array().unwrap().is_empty());
}

#[test]
fn stdio_tools_register_run_reconnect_and_resume_without_owning_execution() {
    let mut fixture = Fixture::new();
    let mut author = Mcp::start(&fixture.author_file);
    let listed = author.request("tools/list", json!({}));
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 5);
    let process_ref = fixture.register(&mut author);
    let discovered = author.tool("apr_discover", json!({"query":"review","limit":20}));
    assert!(discovered.to_string().contains(&process_ref));
    let schema = author.tool(
        "apr_discover",
        json!({"process_ref":process_ref,"detail":"schema"}),
    );
    assert!(schema.to_string().contains("input_schema"));
    let preflight = author.tool("apr_run", json!({"process_ref":process_ref,"input":{"record":"A local record"},"request_key":"preflight-1","mode":"preflight"}));
    assert!(
        preflight.get("run_id").is_none(),
        "preflight must not start a run"
    );
    let started = author.tool("apr_run", json!({"process_ref":process_ref,"input":{"record":"A local record"},"request_key":"start-1","mode":"start"}));
    let run_id = started["run_id"].as_str().unwrap().to_owned();
    author.close();
    assert!(
        fixture.service.try_wait().unwrap().is_none(),
        "closing MCP terminated its service"
    );

    let mut client = Mcp::start(&fixture.run_file);
    let listed = client.request("tools/list", json!({}));
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["apr_discover", "apr_inspect", "apr_resume", "apr_run"]
    );
    let hidden = client.tool_response(
        "apr_manage",
        json!({"action":"process.register","path":"review.json"}),
    );
    assert!(hidden.get("error").is_some());
    let waiting = client.wait_for(&run_id, "waiting_for_agent", true);
    let result = client.tool("apr_resume", submission(&waiting));
    assert_eq!(result["receipt"]["status"], "accepted");
    let completed = client.wait_for(&run_id, "succeeded", true);
    assert_eq!(completed["output"]["approved"], true);
    let replay = client.tool("apr_resume", submission(&waiting));
    assert_eq!(replay["receipt"], result["receipt"]);
    client.close();
}

#[test]
fn tool_schemas_keep_arbitrary_json_typed_and_raw_strings_unmodified() {
    fn assert_explicit_schema(schema: &Value) {
        assert!(
            schema.is_object(),
            "a boolean schema can become type:string in Codex: {schema}"
        );
        for key in ["properties", "$defs", "definitions"] {
            if let Some(children) = schema[key].as_object() {
                for child in children.values() {
                    assert_explicit_schema(child);
                }
            }
        }
        if let Some(items) = schema.get("items") {
            assert_explicit_schema(items);
        }
        if schema["additionalProperties"].is_object() {
            assert_explicit_schema(&schema["additionalProperties"]);
        }
        for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
            if let Some(children) = schema[key].as_array() {
                for child in children {
                    assert_explicit_schema(child);
                }
            }
        }
    }
    let fixture = Fixture::new();
    let mut client = Mcp::start(&fixture.author_file);
    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    for tool in tools {
        assert_explicit_schema(&tool["inputSchema"]);
    }
    let run_schema = &tools.iter().find(|tool| tool["name"] == "apr_run").unwrap()["inputSchema"];
    assert_eq!(run_schema["properties"]["wait_ms"]["maximum"], 5000);
    assert_eq!(run_schema["properties"]["wait_ms"]["default"], 1000);
    // Boolean `true` is valid JSON Schema but Codex's compatibility lowering
    // turns it into type:string. The public SDK schema must use explicit types
    // and local recursive references so clients retain every raw JSON value.
    let input_schema = &run_schema["properties"]["input"];
    assert!(
        input_schema.is_object(),
        "arbitrary input became a boolean schema: {input_schema}"
    );
    let mut input_schema = input_schema;
    if let Some(reference) = input_schema["$ref"].as_str() {
        input_schema = run_schema
            .pointer(reference.strip_prefix('#').unwrap())
            .unwrap();
    }
    let types = input_schema["type"]
        .as_array()
        .expect("raw JSON needs an explicit type union");
    for kind in ["null", "boolean", "number", "string", "array", "object"] {
        assert!(types.contains(&json!(kind)), "missing {kind}");
    }
    assert_eq!(input_schema["items"], input_schema["additionalProperties"]);
    assert!(input_schema["items"]["$ref"].is_string());

    let project = fixture._temporary.path().join("project with spaces");
    let mut manifest = process();
    manifest["name"] = json!("mcp.raw-json");
    manifest["input_schema"] = json!({});
    manifest["config"] = json!({"schema":{"type":"object","properties":{"payload":{}},"additionalProperties":false},
        "defaults":{"payload":{}},"run_override_keys":["payload"]});
    manifest["steps"][0]["inputs"] =
        json!({"record":{"input":""},"configuration":{"config":"/payload"}});
    manifest["steps"][0]["result_schema"] = json!({});
    manifest["output_schema"] = json!({"type":"object"});
    manifest["output"] = json!({"echo":{"step":"review","pointer":""}});
    fs::write(
        project.join("raw-json.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let registered = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"raw-json.json"}),
    );
    let saved_config = json!({"payload":[null,{"raw":"{\"text\":true}"}]});
    client.tool("apr_manage", json!({"action":"config.set","process_ref":registered["process_ref"],"values":saved_config}));
    let configured = client.tool(
        "apr_manage",
        json!({"action":"config.read","process_ref":registered["process_ref"],"detail":true}),
    );
    assert_eq!(configured["config"], saved_config);
    let values = [
        json!({"nested":[{"number":4,"enabled":true},null,["text",false]]}),
        json!("{\"looks_like\":\"JSON\"}"),
        json!([1,"text",null,{"inside":[false]}]),
        json!(true),
        json!(12.5),
        Value::Null,
    ];
    for (index, value) in values.into_iter().enumerate() {
        let queued = client.tool(
            "apr_run",
            json!({"process_ref":registered["process_ref"],"mode":"start",
            "request_key":format!("raw-json-{index}"),"input":value,"config_overrides":{"payload":value}}),
        );
        let id = queued["run_id"].as_str().unwrap();
        let waiting = client.wait_for(id, "waiting_for_agent", true);
        assert_eq!(waiting["request"]["input"]["record"], value);
        assert_eq!(waiting["request"]["input"]["configuration"], value);
        let mut answer = submission(&waiting);
        answer["output"] = value.clone();
        client.tool("apr_resume", answer);
        let completed = client.wait_for(id, "succeeded", true);
        assert_eq!(completed["output"]["echo"], value);
    }
    client.close();
}

#[test]
fn service_rejects_forged_author_mode_revoked_attachments_and_other_assignments() {
    let fixture = Fixture::new();
    let mut author = Mcp::start(&fixture.author_file);
    let process_ref = fixture.register(&mut author);
    let started = author.tool("apr_run", json!({"process_ref":process_ref,"input":{"record":"Private assignment input"},"request_key":"owned-run","mode":"start"}));
    let run_id = started["run_id"].as_str().unwrap();

    let mut forged: Value = serde_json::from_slice(&fs::read(&fixture.run_file).unwrap()).unwrap();
    forged["mode"] = json!("author");
    let forged_file = fixture.state.join("attachments/forged.json");
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&forged_file)
        .unwrap();
    serde_json::to_writer(file, &forged).unwrap();
    let mut forged_client = Mcp::start(&forged_file);
    let denied = forged_client.tool_response(
        "apr_manage",
        json!({"action":"process.register","path":"review.json"}),
    );
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        "FORBIDDEN"
    );
    forged_client.close();

    let other = cli(
        &fixture.state,
        &[
            "assignment",
            "create",
            "--project",
            &fixture.project,
            "--name",
            "Other task",
        ],
    );
    let other_file = fixture.state.join("attachments/other.json");
    cli(
        &fixture.state,
        &[
            "attachment",
            "create",
            "--assignment",
            other["assignment_id"].as_str().unwrap(),
            "--mode",
            "run",
            "--output-file",
            path(&other_file),
        ],
    );
    let mut other_client = Mcp::start(&other_file);
    let denied = other_client.tool_response(
        "apr_inspect",
        json!({"view":"run","run_id":run_id,"detail":true}),
    );
    assert_eq!(denied["result"]["isError"], true);
    assert!(!denied.to_string().contains("Private assignment input"));
    other_client.close();

    let descriptor: Value =
        serde_json::from_slice(&fs::read(&fixture.author_file).unwrap()).unwrap();
    cli(
        &fixture.state,
        &[
            "attachment",
            "revoke",
            descriptor["attachment_id"].as_str().unwrap(),
        ],
    );
    let verification = rusqlite::Connection::open(fixture.state.join("state.sqlite3")).unwrap();
    let revoked: bool = verification
        .query_row(
            "SELECT revoked_at IS NOT NULL FROM attachments WHERE id=?1",
            [descriptor["attachment_id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(revoked, "owner CLI did not persist revocation");
    drop(verification);
    let denied = author.tool_response("apr_discover", json!({}));
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"], "UNAUTHORIZED",
        "{denied}"
    );
    assert!(
        !denied
            .to_string()
            .contains(descriptor["token"].as_str().unwrap())
    );
    let denied = author.tool_response("apr_discover", json!({"guide":"schema"}));
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        "UNAUTHORIZED"
    );
    author.close();
}

#[test]
fn handoffs_are_complete_only_in_explicit_detail_and_action_payloads_are_bounded() {
    let fixture = Fixture::new();
    let mut client = Mcp::start(&fixture.author_file);
    let process_ref = fixture.register(&mut client);
    let started = client.tool("apr_run", json!({"process_ref":process_ref,"input":{"record":"x".repeat(12000)},"request_key":"large-handoff","mode":"start"}));
    let run_id = started["run_id"].as_str().unwrap();
    let compact = client.wait_for(run_id, "waiting_for_agent", false);
    assert!(serde_json::to_vec(&compact).unwrap().len() < 8192);
    assert!(
        compact.get("request").is_none(),
        "a partial executable request must not be returned"
    );
    assert_eq!(compact["truncated"], true);
    let full = client.wait_for(run_id, "waiting_for_agent", true);
    assert_eq!(
        full["request"]["input"]["record"].as_str().unwrap().len(),
        12000
    );
    assert!(full["request"]["continuation_token"].as_str().is_some());
    let denied = client.tool_response("apr_run", json!({"process_ref":process_ref,"input":{"record":"x".repeat(65536)},"request_key":"oversized","mode":"start"}));
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        "LIMIT_EXCEEDED"
    );
    assert!(serde_json::to_vec(&denied["result"]).unwrap().len() < 8192);
    client.close();
}

#[test]
fn oversized_wire_frames_close_the_bridge_before_parsing() {
    let fixture = Fixture::new();
    let mut client = Mcp::start(&fixture.run_file);
    let oversized = vec![b' '; 1024 * 1024 + 8192];
    let _ = client.input.as_mut().unwrap().write_all(&oversized);
    // Keep stdin open: shutdown must follow the size violation, not caller EOF.
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.child.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "oversized frame did not terminate the bridge"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        client.messages.try_recv().is_err(),
        "wire garbage produced a tool result"
    );
}

#[test]
fn fixture_work_wakes_an_idle_service_and_configuration_round_trips_through_mcp() {
    let fixture = Fixture::new();
    let project = fixture._temporary.path().join("project with spaces");
    let manifest = json!({"schema_version":"0.1","name":"mcp.configured-fixture","description":"Exercise saved configuration through real MCP",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","required":["note"],"additionalProperties":false,"properties":{"note":{"type":"string","maxLength":13000}}},"defaults":{"note":"x".repeat(12000)},"run_override_keys":["note"]},
        "capabilities":[{"kind":"builtin"},{"kind":"artifacts"}],
        "limits":{"max_step_ms":1000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[{"id":"save","kind":"builtin","operation":"json.write","effect":"pure","needs":[],"retry":{"max_attempts":1},"inputs":{"value":{"config":""}}}],
        "output":{"artifact":{"step":"save","pointer":"/artifact"}}});
    fs::write(
        project.join("configured.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut client = Mcp::start(&fixture.author_file);
    assert_eq!(
        client.tool("apr_discover", json!({}))["processes"],
        json!([])
    );
    std::thread::sleep(Duration::from_millis(60));
    let queued=client.tool("apr_manage",json!({"action":"process.test","path":"configured.json","input":{},"request_key":"fixture"}));
    assert_eq!(queued["fixture"], true);
    let run = queued["run_id"].as_str().unwrap();
    client.wait_for(run, "succeeded", false);
    assert_eq!(
        client.tool("apr_discover", json!({}))["processes"],
        json!([])
    );
    let registered = client.tool(
        "apr_manage",
        json!({"action":"process.register","path":"configured.json"}),
    );
    let version = &registered["process_ref"];
    let compact = client.tool(
        "apr_manage",
        json!({"action":"config.read","process_ref":version}),
    );
    assert_eq!(compact["truncated"], true);
    assert_eq!(compact["process_ref"], *version);
    let full = client.tool(
        "apr_manage",
        json!({"action":"config.read","process_ref":version,"detail":true}),
    );
    assert_eq!(full["config"]["note"].as_str().unwrap().len(), 12000);
    client.tool(
        "apr_manage",
        json!({"action":"config.set","process_ref":version,"values":{"note":"assignment"}}),
    );
    let run=client.tool("apr_run",json!({"process_ref":version,"input":{},"request_key":"override","mode":"start","config_overrides":{"note":"run"}}));
    let config = client.tool(
        "apr_inspect",
        json!({"view":"config","run_id":run["run_id"],"detail":true}),
    );
    assert_eq!(config["config"]["note"], "run");
    let history = client.tool("apr_inspect", json!({"view":"runs","limit":1}));
    assert_eq!(history["runs"].as_array().unwrap().len(), 1);
    client.close();
}

#[test]
fn attachment_output_stays_private_and_cannot_escape_or_clobber_files() {
    let temporary = tempfile::tempdir().unwrap();
    let state = temporary.path().join("state");
    let project_path = temporary.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let project = cli(&state, &["project", "attach", path(&project_path)]);
    let assignment = cli(
        &state,
        &[
            "assignment",
            "create",
            "--project",
            project["project_id"].as_str().unwrap(),
            "--name",
            "Private output",
        ],
    );
    let assignment_id = assignment["assignment_id"].as_str().unwrap();
    let create = |destination: &Path, assignment: &str| {
        Command::new(env!("CARGO_BIN_EXE_apr"))
            .arg("--state-dir")
            .arg(&state)
            .args([
                "attachment",
                "create",
                "--assignment",
                assignment,
                "--mode",
                "run",
                "--output-file",
            ])
            .arg(destination)
            .output()
            .unwrap()
    };
    let outside = project_path.join("must-not-exist.json");
    // Invalid assignment deliberately proves the path is rejected before the
    // capability creation API is invoked.
    let rejected = create(&outside, "unknown-assignment");
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    let error: Value = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_INPUT");
    assert!(!outside.exists());

    symlink(&project_path, state.join("escape")).unwrap();
    let rejected = create(&state.join("escape/must-not-exist.json"), assignment_id);
    assert!(!rejected.status.success());
    assert!(!outside.exists());

    let destination = state.join("attachments/nested/client.json");
    let created = create(&destination, assignment_id);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let bytes = fs::read(&destination).unwrap();
    let descriptor: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !String::from_utf8_lossy(&created.stdout).contains(descriptor["token"].as_str().unwrap())
    );
    assert_eq!(
        fs::metadata(destination.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!create(&destination, assignment_id).status.success());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    let link = state.join("attachments/existing-link.json");
    symlink(&outside, &link).unwrap();
    assert!(!create(&link, assignment_id).status.success());
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!outside.exists());
}

#[test]
fn global_connection_creates_projects_and_keeps_every_call_scoped() {
    let fixture = Fixture::new();
    let mut mcp = Mcp::projects(&fixture.state);
    let tools = mcp.request("tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);
    let missing = mcp.tool_response("apr_inspect", json!({"view":"runs"}));
    assert_eq!(
        missing["result"]["structuredContent"]["error"]["code"],
        "WORKSPACE_REQUIRED"
    );
    for topic in ["overview", "workflow_design"] {
        let guide = mcp.tool("apr_discover", json!({"guide":topic}));
        assert!(guide["connect"].is_string());
        assert!(guide["runtime"]["features"]["jev"].is_null());
    }
    let mut scopes = Vec::new();
    for name in ["agent-one", "agent-two"] {
        let root = fixture._temporary.path().join(name);
        fs::create_dir(&root).unwrap();
        let prepare = mcp.tool("apr_project", json!({"action":"prepare","root":root}));
        let missing_proof = mcp.tool_response(
            "apr_project",
            json!({"action":"connect","challenge_id":prepare["challenge_id"]}),
        );
        assert_eq!(missing_proof["result"]["isError"], true);
        fs::write(
            prepare["verification_file"].as_str().unwrap(),
            // Native file-edit tools may append a second newline to valid JSON.
            format!("{}\n", prepare["verification_contents"].as_str().unwrap()),
        )
        .unwrap();
        let connect_args = json!({"action":"connect","challenge_id":prepare["challenge_id"]});
        let connected = mcp.tool("apr_project", connect_args.clone());
        assert!(connected.get("token").is_none());
        assert!(connected.get("attachment_file").is_none());
        assert_eq!(mcp.tool("apr_project", connect_args), connected);
        fs::remove_file(connected["cleanup_file"].as_str().unwrap()).unwrap();
        scopes.push((root, connected));
    }
    assert_ne!(scopes[0].1["assignment_id"], scopes[1].1["assignment_id"]);
    let scope = &scopes[0].1["workspace"];
    let starter = mcp.tool("apr_discover", json!({"workspace":scope,"guide":"starter"}));
    fs::write(
        scopes[0].0.join("note.json"),
        serde_json::to_vec(&starter["content"]["manifest"]).unwrap(),
    )
    .unwrap();
    let registered = mcp.tool(
        "apr_manage",
        json!({"workspace":scope,"action":"process.register","path":"note.json"}),
    );
    let args = json!({"workspace":scope,"process_ref":registered["process_ref"],"input":{"note":"Created through agent project connection"},"request_key":"global-note","mode":"start"});
    let run = mcp.tool("apr_run", args);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let inspected = mcp.tool(
            "apr_inspect",
            json!({"workspace":scope,"view":"run","run_id":run["run_id"]}),
        );
        if inspected["state"] == "succeeded" {
            break;
        }
        assert!(Instant::now() < deadline, "{inspected}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let foreign = mcp.tool_response(
        "apr_inspect",
        json!({"workspace":scopes[1].1["workspace"],"view":"run","run_id":run["run_id"]}),
    );
    assert_eq!(foreign["result"]["isError"], true);
    let wrong = mcp.tool_response("apr_discover", json!({"workspace":"unknown"}));
    assert_eq!(wrong["result"]["isError"], true);
    let mut legacy = Mcp::start(&fixture.author_file);
    let denied = legacy.tool_response(
        "apr_project",
        json!({"action":"prepare","root":scopes[0].0}),
    );
    assert!(denied.get("error").is_some());
    let denied = legacy.tool_response("apr_inspect", json!({"workspace":scope,"view":"runs"}));
    assert_eq!(denied["result"]["isError"], true);
    mcp.close();
    let mut reconnected = Mcp::projects(&fixture.state);
    let proof = reconnected.tool(
        "apr_project",
        json!({"action":"prepare","root":scopes[0].0}),
    );
    fs::write(
        proof["verification_file"].as_str().unwrap(),
        proof["verification_contents"].as_str().unwrap(),
    )
    .unwrap();
    let connected = reconnected.tool(
        "apr_project",
        json!({"action":"connect","challenge_id":proof["challenge_id"]}),
    );
    assert_eq!(connected["assignment_id"], scopes[0].1["assignment_id"]);
    assert_ne!(connected["workspace"], scopes[0].1["workspace"]);
    let inspected = reconnected.tool(
        "apr_inspect",
        json!({"workspace":connected["workspace"],"view":"run","run_id":run["run_id"]}),
    );
    assert_eq!(inspected["state"], "succeeded");
}

#[test]
fn mcp_connection_contract_returns_approved_public_schema() {
    let fixture = Fixture::new();
    let mut author = Mcp::start(&fixture.author_file);
    let proposal = author.tool(
        "apr_manage",
        json!({
            "action":"connection.propose",
            "name":"synthetic.contract",
            "request_key":"contract-approval",
            "rationale":"Synthetic public contract inspection",
            "connection":{
                "label":"Synthetic contract",
                "endpoint":"https://example.test/SECRET-TOKEN",
                "method":"GET",
                "auth":{"kind":"none"},
                "input_schema":{"type":"object","properties":{"query":{"type":"string"}}},
                "output_schema":{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}},
                "timeout_ms":1000,
                "max_response_bytes":4096,
                "loopback_test":false
            }
        }),
    );
    let approved = cli(
        &fixture.state,
        &[
            "execution",
            "approve-connection",
            "--request",
            proposal["request_id"].as_str().unwrap(),
            "--digest",
            proposal["digest"].as_str().unwrap(),
        ],
    );
    assert_eq!(approved["granted"], true);

    let contract = author.tool(
        "apr_inspect",
        json!({"view":"connection_contract","name":"synthetic.contract"}),
    );
    assert_eq!(contract["name"], "synthetic.contract");
    assert_eq!(contract["revision"], 1);
    assert_eq!(contract["enabled"], true);
    assert_eq!(contract["label"], "Synthetic contract");
    assert_eq!(contract["kind"], "http.json");
    assert_eq!(contract["credentials"], "owner_managed");
    assert_eq!(
        contract["input_schema"]["properties"]["query"]["type"],
        "string"
    );
    assert_eq!(contract["output_schema"]["required"][0], "answer");
    assert!(contract.get("endpoint").is_none());
    assert!(!contract.to_string().contains("SECRET-TOKEN"));

    let unapproved = author.tool_response(
        "apr_inspect",
        json!({"view":"connection_contract","name":"synthetic.missing"}),
    );
    assert_eq!(unapproved["result"]["isError"], true);
    assert_eq!(
        unapproved["result"]["structuredContent"]["error"]["code"],
        "FORBIDDEN"
    );
    author.close();

    let other = cli(
        &fixture.state,
        &[
            "assignment",
            "create",
            "--project",
            &fixture.project,
            "--name",
            "Other contract scope",
        ],
    );
    let other_file = fixture.state.join("attachments/other-contract.json");
    cli(
        &fixture.state,
        &[
            "attachment",
            "create",
            "--assignment",
            other["assignment_id"].as_str().unwrap(),
            "--mode",
            "author",
            "--output-file",
            path(&other_file),
        ],
    );
    let mut foreign = Mcp::start(&other_file);
    let denied = foreign.tool_response(
        "apr_inspect",
        json!({"view":"connection_contract","name":"synthetic.contract"}),
    );
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        "FORBIDDEN"
    );
    foreign.close();
}
