//! Synthetic local MCP acceptance for the authoring execution boundary.
//! The server is a checked-in fixture and makes no network or account call.

use apr_connectors::{
    Auth, Connection,
    mcp::{McpBinding, ToolContract, Transport, pin_executable},
};
use apr_runtime::Runtime;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    temp: tempfile::TempDir,
    runtime: Runtime,
    project: PathBuf,
    assignment: String,
    token: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut runtime = Runtime::open(temp.path().join("state")).unwrap();
        let assignment = runtime
            .owner_project_setup(&project, "boundary fixture")
            .unwrap()["assignment_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let token = runtime.create_attachment(&assignment, "author").unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            temp,
            runtime,
            project,
            assignment,
            token,
        }
    }

    fn connection(&self) -> Connection {
        let output = std::process::Command::new("python3")
            .args(["-I", "-c", "import sys; print(sys.executable)"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let python = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/mcp/server.py")
            .canonicalize()
            .unwrap();
        let input = json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false});
        Connection {
            label: "synthetic local service".into(),
            credential_label: None,
            endpoint: "stdio:local".into(),
            method: "POST".into(),
            auth: Auth::None,
            input_schema: input.clone(),
            output_schema: json!({"type":"object"}),
            metering: None,
            timeout_ms: 2000,
            max_response_bytes: 16384,
            loopback_test: false,
            mcp: Some(McpBinding {
                sampling_worker: None,
                feature: None,
                interaction: None,
                protocol: "2026-07-28".into(),
                transport: Transport::Stdio {
                    executable: python.to_string_lossy().into(),
                    sha256: pin_executable(&python).unwrap(),
                    args: vec![
                        "-I".into(),
                        "-u".into(),
                        script.to_string_lossy().into(),
                        "ok".into(),
                        self.temp.path().join("calls").to_string_lossy().into(),
                    ],
                    credential_env: None,
                },
                tool: Some(ToolContract {
                    ui_resource_uri: None,
                    name: "normalize".into(),
                    input_schema: input,
                    output_schema: Some(
                        json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}}),
                    ),
                }),
            }),
        }
    }

    fn propose(&mut self, name: &str) -> Value {
        self.runtime.scoped_call(&self.token, "manage", json!({
            "action":"connection.propose", "name":name, "request_key":format!("proposal-{name}"),
            "rationale":"Use one explicit synthetic service contract", "connection":self.connection()
        })).unwrap()
    }

    fn approve(&mut self, proposal: &Value) {
        self.runtime
            .approve_connection(
                proposal["request_id"].as_str().unwrap(),
                proposal["digest"].as_str().unwrap(),
                None,
            )
            .unwrap();
    }

    fn register(&mut self, mut process: Value) -> String {
        process["input_schema"] = json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false});
        process["output_schema"] = json!({"type":"object"});
        for step in process["steps"].as_array_mut().unwrap() {
            if step["kind"] == "connector" {
                step["inputs"] = json!({"value":{"input":"/value"}});
                step["result_schema"] = json!({"type":"object"});
            }
        }
        if process["name"] == "synthetic.reviewed-update" {
            process["steps"][2]["inputs"] = json!({
                "record":{"step":"fetch_record","pointer":"/structuredContent/value"},
                "policy":{"step":"fetch_policy","pointer":"/structuredContent/value"}
            });
            process["steps"][2]["result_schema"] = json!({
                "type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false
            });
            process["steps"][3]["inputs"] = json!({"value":{"step":"judge","pointer":"/value"}});
            process["output"] =
                json!({"receipt":{"step":"write_decision","pointer":"/structuredContent"}});
        } else {
            process["output"] = json!({
                "record":{"step":"fetch_record","pointer":"/structuredContent"},
                "policy":{"step":"fetch_policy","pointer":"/structuredContent"}
            });
        }
        std::fs::write(
            self.project.join("boundary.json"),
            serde_json::to_vec(&process).unwrap(),
        )
        .unwrap();
        self.runtime
            .scoped_call(
                &self.token,
                "manage",
                json!({
                    "action":"process.register", "path":"boundary.json"
                }),
            )
            .unwrap()["process_ref"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn register_native(&mut self) -> String {
        let source =
            include_bytes!("../../../examples/execution-boundary-review/context-helper.py");
        std::fs::write(self.project.join("context-helper.py"), source).unwrap();
        let process: Value = serde_json::from_str(include_str!(
            "../../../examples/execution-boundary-review/context-process.json"
        ))
        .unwrap();
        std::fs::write(
            self.project.join("context-process.json"),
            serde_json::to_vec(&process).unwrap(),
        )
        .unwrap();
        let version = self
            .runtime
            .scoped_call(
                &self.token,
                "manage",
                json!({
                    "action":"process.register", "path":"context-process.json"
                }),
            )
            .unwrap()["process_ref"]
            .as_str()
            .unwrap()
            .to_owned();
        let output = std::process::Command::new("python3")
            .args(["-I", "-c", "import sys; print(sys.executable)"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let python = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        self.runtime
            .grant_python(&self.assignment, &python)
            .unwrap();
        version
    }

    fn run(&mut self, version: &str, key: &str) -> Value {
        self.runtime
            .scoped_call(
                &self.token,
                "run",
                json!({
                    "process_ref":version, "input":{"value":7}, "request_key":key, "mode":"start"
                }),
            )
            .unwrap()
    }

    fn settle(&mut self, run_id: &str) -> apr_runtime::RunEnvelope {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            self.runtime.drive_pending().unwrap();
            let run = self.runtime.inspect(run_id).unwrap();
            if !matches!(run.state.as_str(), "queued" | "running") {
                return run;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "run did not settle: {run:?}"
            );
        }
    }

    fn tool_calls(&self) -> usize {
        std::fs::read_to_string(self.temp.path().join("calls"))
            .unwrap_or_default()
            .matches("tools/call")
            .count()
    }
}

fn service_process() -> Value {
    serde_json::from_str(include_str!(
        "../../../examples/execution-boundary-review/service-process.json"
    ))
    .unwrap()
}

fn mixed_process() -> Value {
    serde_json::from_str(include_str!(
        "../../../examples/execution-boundary-review/mixed-process.json"
    ))
    .unwrap()
}

#[test]
fn pending_multi_service_access_blocks_without_a_routine_caller_handoff() {
    let review: Value = serde_json::from_str(include_str!(
        "../../../examples/execution-boundary-review/service-review.json"
    ))
    .unwrap();
    assert_eq!(
        review["connections"]["records"]["caller_has_service_tool"],
        true
    );
    assert_eq!(review["expected"]["model_round_trips"], 0);
    let mut f = Fixture::new();
    let _records = f.propose("records");
    let _policy = f.propose("policy");
    let version = f.register(service_process());
    let ready = f
        .runtime
        .scoped_call(
            &f.token,
            "inspect",
            json!({
                "view":"readiness","process_ref":version
            }),
        )
        .unwrap();
    assert_eq!(ready["can_finish_unattended"], false);
    assert!(
        f.runtime
            .scoped_call(
                &f.token,
                "run",
                json!({
                    "process_ref":version,"input":{"value":7},"request_key":"blocked","mode":"start"
                })
            )
            .is_err()
    );
    assert_eq!(f.tool_calls(), 0);
}

#[test]
fn approved_multi_service_recipe_finishes_with_zero_caller_handoffs() {
    let mut f = Fixture::new();
    let records = f.propose("records");
    let policy = f.propose("policy");
    f.approve(&records);
    f.approve(&policy);
    let version = f.register(service_process());
    let start = f.run(&version, "service-brief");
    let run = f.settle(start["run_id"].as_str().unwrap());
    assert_eq!(run.state, "succeeded", "{run:?}");
    assert!(run.request.is_none());
    assert_eq!(f.tool_calls(), 2);
    assert_eq!(
        run.output.unwrap(),
        json!({"record":{"value":7},"policy":{"value":7}})
    );
    let replay = f.run(&version, "service-brief");
    assert_eq!(replay["run_id"], start["run_id"]);
    assert_eq!(f.tool_calls(), 2);
}

#[test]
fn mixed_recipe_waits_for_judgment_then_writes_once_after_replayed_resume() {
    let mut f = Fixture::new();
    for name in ["records", "policy", "updates"] {
        let proposal = f.propose(name);
        f.approve(&proposal);
    }
    let version = f.register(mixed_process());
    let start = f.run(&version, "mixed-run");
    let run_id = start["run_id"].as_str().unwrap();
    let waiting = f.settle(run_id);
    assert_eq!(waiting.state, "waiting_for_agent", "{waiting:?}");
    assert_eq!(f.tool_calls(), 2, "downstream write ran before judgment");
    let request = waiting.request.unwrap();
    assert_eq!(request.step_instance_id, "judge");
    let submission = json!({
        "protocol_version":"0.1", "action":"submit", "run_id":run_id,
        "request_id":request.request_id, "step_instance_id":request.step_instance_id,
        "generation":request.generation, "submission_key":"judgment-1",
        "continuation_token":request.continuation_token,
        "output":{"value":9}, "provenance":{"source":"synthetic-observed-judgment"}
    });
    let receipt = f
        .runtime
        .scoped_call(&f.token, "resume", submission.clone())
        .unwrap();
    let done = f.settle(run_id);
    assert_eq!(done.state, "succeeded", "{done:?}");
    assert_eq!(f.tool_calls(), 3);
    assert_eq!(done.output.unwrap(), json!({"receipt":{"value":9}}));
    let mut conflicting = submission.clone();
    conflicting["output"]["value"] = json!(10);
    assert_eq!(
        f.runtime
            .scoped_call(&f.token, "resume", conflicting)
            .unwrap_err()
            .code(),
        "SUBMISSION_CONFLICT"
    );
    let replay = f
        .runtime
        .scoped_call(&f.token, "resume", submission)
        .unwrap();
    assert_eq!(replay["receipt"], receipt["receipt"]);
    assert_eq!(f.run(&version, "mixed-run")["run_id"], start["run_id"]);
    f.runtime.drive_pending().unwrap();
    assert_eq!(f.tool_calls(), 3);
}

#[test]
fn frozen_helper_keeps_raw_pages_but_returns_bounded_revision_deltas() {
    use apr_core::ArtifactRef;
    use apr_runtime::artifacts::ArtifactStore;
    use sha2::{Digest, Sha256};

    let mut f = Fixture::new();
    let version = f.register_native();
    std::fs::write(
        f.project.join("context-helper.py"),
        b"raise RuntimeError('post-registration edit must not run')",
    )
    .unwrap();
    let initial_guidance = json!([
        {"id":"keep","text":"Original guidance"},
        {"id":"remove","text":"Old guidance"}
    ]);
    let base = json!({
        "item_count":1200, "page_size":75, "guidance":initial_guidance,
        "previous_guidance":[], "previous_revision":null, "missing_page":null
    });
    let first = f
        .runtime
        .scoped_call(
            &f.token,
            "run",
            json!({
                "process_ref":version, "input":base, "request_key":"first", "mode":"start"
            }),
        )
        .unwrap();
    let first = f.settle(first["run_id"].as_str().unwrap());
    assert_eq!(first.state, "succeeded", "{first:?}");
    let first_result = &first.output.as_ref().unwrap()["result"];
    assert_eq!(first_result["status"], "complete");
    assert_eq!(first_result["item_count"], 1200);
    assert_eq!(first_result["delta"]["added"], json!(["keep", "remove"]));
    let first_revision = first_result["revision"].as_str().unwrap().to_owned();
    let first_bytes = serde_json::to_vec(first_result).unwrap();
    assert!(first_bytes.len() <= 2048, "normal result exceeded budget");
    let raw: ArtifactRef = serde_json::from_value(first_result["raw"].clone()).unwrap();
    let store = ArtifactStore::open(f.temp.path().join("state")).unwrap();
    let full = store.read(&raw, 1_048_576).unwrap();
    assert!(full.len() > 64 * 1024, "dataset was not oversized");
    assert_eq!(full.iter().filter(|&&byte| byte == b'\n').count(), 1200);
    assert_eq!(
        format!("{:x}", Sha256::digest(&full)),
        first_result["raw_sha256"]
    );

    let unchanged = f.runtime.scoped_call(&f.token, "run", json!({
        "process_ref":version,
        "input":{"item_count":1200,"page_size":75,"guidance":initial_guidance,
            "previous_guidance":initial_guidance,"previous_revision":first_revision,"missing_page":null},
        "request_key":"unchanged", "mode":"start"
    })).unwrap();
    let unchanged = f.settle(unchanged["run_id"].as_str().unwrap());
    assert_eq!(unchanged.state, "succeeded", "{unchanged:?}");
    let unchanged_result = &unchanged.output.as_ref().unwrap()["result"];
    assert_eq!(unchanged_result["status"], "unchanged");
    assert_eq!(unchanged_result.as_object().unwrap().len(), 2);
    assert!(serde_json::to_vec(unchanged_result).unwrap().len() < 128);

    let changed_guidance = json!([
        {"id":"keep","text":"Revised guidance"},
        {"id":"add","text":"New guidance"}
    ]);
    let changed = f.runtime.scoped_call(&f.token, "run", json!({
        "process_ref":version,
        "input":{"item_count":1200,"page_size":75,"guidance":changed_guidance,
            "previous_guidance":initial_guidance,"previous_revision":first_revision,"missing_page":null},
        "request_key":"changed", "mode":"start"
    })).unwrap();
    let changed = f.settle(changed["run_id"].as_str().unwrap());
    assert_eq!(changed.state, "succeeded", "{changed:?}");
    let delta = &changed.output.as_ref().unwrap()["result"]["delta"];
    assert_eq!(delta["added"], json!(["add"]));
    assert_eq!(delta["changed"], json!(["keep"]));
    assert_eq!(delta["removed"], json!(["remove"]));

    let missing = f.runtime.scoped_call(&f.token, "run", json!({
        "process_ref":version,
        "input":{"item_count":1200,"page_size":75,"guidance":changed_guidance,
            "previous_guidance":initial_guidance,"previous_revision":first_revision,"missing_page":3},
        "request_key":"missing", "mode":"start"
    })).unwrap();
    let missing = f.settle(missing["run_id"].as_str().unwrap());
    assert_eq!(missing.state, "succeeded", "{missing:?}");
    let exception = &missing.output.as_ref().unwrap()["result"];
    assert_eq!(exception["status"], "incomplete");
    assert_eq!(exception["missing_page"], 3);
    assert!(exception.get("raw").is_none());
}
