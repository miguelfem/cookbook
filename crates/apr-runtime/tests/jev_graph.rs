#![cfg(feature = "jev")]

use apr_connectors::{Auth, Connection, Secret};
use apr_runtime::{RunEnvelope, Runtime};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;

fn manifest() -> Value {
    serde_json::from_str(include_str!("../../../examples/jev-routing/process.json")).unwrap()
}

struct Fixture {
    _root: tempfile::TempDir,
    runtime: Runtime,
    token: String,
    assignment: String,
    process_ref: String,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(
            project.join("process.json"),
            serde_json::to_vec(&manifest()).unwrap(),
        )
        .unwrap();
        let mut runtime = Runtime::open(root.path().join("state")).unwrap();
        let project_id = runtime.attach_project(&project).unwrap()["project_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let assignment = runtime
            .create_assignment(&project_id, "Synthetic Jev graph")
            .unwrap()["assignment_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let token = runtime.create_attachment(&assignment, "author").unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let process_ref = runtime
            .scoped_call(
                &token,
                "manage",
                json!({"action":"process.register","path":"process.json"}),
            )
            .unwrap()["process_ref"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            _root: root,
            runtime,
            token,
            assignment,
            process_ref,
        }
    }

    fn bind(&mut self, endpoint: String) {
        let mut connection: Connection = serde_json::from_str(include_str!(
            "../../../examples/jev-routing/connection.json"
        ))
        .unwrap();
        connection.endpoint = endpoint;
        connection.loopback_test = true;
        connection.auth = Auth::Bearer {
            credential_ref: "pending".into(),
        };
        self.runtime
            .configure_connection(
                &self.assignment,
                "typesafe",
                connection,
                Some(Secret::new(b"jev-graph-fixture-only".to_vec()).unwrap()),
            )
            .unwrap();
    }

    fn start(&mut self, key: &str, input: Value) -> String {
        self.runtime
            .scoped_call(
                &self.token,
                "run",
                json!({"mode":"start","process_ref":self.process_ref,"input":input,"request_key":key}),
            )
            .unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn settle(&mut self, id: &str) -> RunEnvelope {
        for _ in 0..20 {
            self.runtime.drive_pending().unwrap();
            let run = self.runtime.inspect(id).unwrap();
            if !matches!(run.state.as_str(), "queued" | "running") {
                return run;
            }
        }
        panic!("fixture run did not settle: {}", id)
    }
}

fn fake_jev(answer: Value) -> (String, std::thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let body_start = loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(split) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..split]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= split + 4 + length {
                    break (split + 4, length);
                }
            }
        };
        let request: Value =
            serde_json::from_slice(&bytes[body_start.0..body_start.0 + body_start.1]).unwrap();
        let body = answer.to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        request
    });
    (endpoint, handle)
}

fn response(choice: &str, confidence: f64) -> Value {
    let mut probabilities = json!({"operations":0.0,"engineering":0.0,"unclear":0.0});
    probabilities[choice] = json!(1.0);
    json!({"model":"jev-fixture","answers":{"team":{"type":"choice","choice":choice,
        "probabilities":probabilities,"confidence":confidence}},"usage":{"input_tokens":18,"output_tokens":4}})
}

fn assert_artifact(f: &Fixture, run: &RunEnvelope, expected: Value) {
    let output = run.output.as_ref().unwrap();
    assert_eq!(output["result"], expected);
    let reference = serde_json::from_value(output["artifact"].clone()).unwrap();
    let artifact: Value =
        serde_json::from_slice(&f.runtime.read_artifact(&reference, 65536).unwrap()).unwrap();
    assert_eq!(artifact, expected);
    assert_eq!(output["evidence_ref"], "fixture:ticket:1");
}

#[test]
fn confident_choice_executes_pinned_branch_and_writes_exact_result() {
    let mut f = Fixture::new();
    let (endpoint, server) = fake_jev(response("operations", 0.96));
    f.bind(endpoint);
    let id = f.start(
        "confident",
        json!({"ticket":"Reset a synthetic account", "evidence_ref":"fixture:ticket:1"}),
    );
    let run = f.settle(&id);
    assert_eq!(run.state, "succeeded", "{:?}", run.error);
    let request = server.join().unwrap();
    assert_eq!(request["state"], "Reset a synthetic account");
    assert!(request.to_string().find("fixture:ticket:1").is_none());
    assert_eq!(request["questions"].as_object().unwrap().len(), 1);
    assert_eq!(run.output.as_ref().unwrap()["route"]["route"], "operations");
    assert_eq!(
        run.output.as_ref().unwrap()["observation"]["answers"]["team"]["choice"],
        "operations"
    );
    assert_artifact(
        &f,
        &run,
        json!({"route":"operations","output":{"team":"operations"}}),
    );
    assert_eq!(
        run.steps
            .iter()
            .find(|step| step.id == "unclear")
            .unwrap()
            .state,
        "skipped"
    );
}

#[test]
fn low_confidence_waits_for_caller_and_preserves_raw_observation() {
    let mut f = Fixture::new();
    let (endpoint, server) = fake_jev(response("engineering", 0.48));
    f.bind(endpoint);
    let id = f.start(
        "review",
        json!({"ticket":"Integration is failing", "evidence_ref":"fixture:ticket:1"}),
    );
    let waiting = f.settle(&id);
    assert_eq!(waiting.state, "waiting_for_agent", "{:?}", waiting.error);
    server.join().unwrap();
    let request = waiting.request.unwrap();
    f.runtime.scoped_call(&f.token,"resume",json!({"protocol_version":"0.1","action":"submit",
        "run_id":id,"request_id":request.request_id,"step_instance_id":request.step_instance_id,
        "generation":request.generation,"submission_key":"reviewed","continuation_token":request.continuation_token,
        "output":{"team":"engineering"},"provenance":{"source":"fixture"}})).unwrap();
    let done = f.settle(&id);
    assert_eq!(done.state, "succeeded", "{:?}", done.error);
    let output = done.output.as_ref().unwrap();
    assert_eq!(
        output["observation"]["answers"]["team"]["choice"],
        "engineering"
    );
    assert_eq!(output["route"]["route"], "unclear");
    assert_artifact(
        &f,
        &done,
        json!({"route":"unclear","output":{"team":"engineering"}}),
    );
}

#[test]
fn missing_required_inputs_fail_before_provider_dispatch() {
    let mut f = Fixture::new();
    f.bind("http://127.0.0.1:9/v1/systemone".into());
    for (key, input) in [
        ("missing", json!({"ticket":"Synthetic request"})),
        (
            "empty",
            json!({"ticket":"","evidence_ref":"fixture:ticket:1"}),
        ),
    ] {
        assert!(
            f.runtime
                .scoped_call(
                    &f.token,
                    "run",
                    json!({"mode":"start",
            "process_ref":f.process_ref,"input":input,"request_key":key})
                )
                .is_err()
        );
    }
    let db = rusqlite::Connection::open(f._root.path().join("state/state.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM effect_intents", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn invalid_provider_choice_never_reaches_a_branch_or_final_artifact() {
    let mut f = Fixture::new();
    let (endpoint, server) = fake_jev(json!({"model":"jev-fixture","answers":{"team":{
        "type":"choice","choice":"invented","probabilities":{"operations":0.0,"engineering":0.0,"unclear":1.0},
        "confidence":0.99}},"usage":{"input_tokens":18,"output_tokens":4}}));
    f.bind(endpoint);
    let id = f.start(
        "invalid",
        json!({"ticket":"Synthetic request","evidence_ref":"fixture:ticket:1"}),
    );
    let run = f.settle(&id);
    server.join().unwrap();
    assert_ne!(run.state, "succeeded");
    assert!(run.output.is_none());
    assert_eq!(
        run.steps
            .iter()
            .find(|step| step.id == "route")
            .unwrap()
            .attempts,
        0
    );
    assert_eq!(
        run.steps
            .iter()
            .find(|step| step.id == "result_artifact")
            .unwrap()
            .attempts,
        0
    );
}
