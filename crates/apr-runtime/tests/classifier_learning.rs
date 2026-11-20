use apr_core::admit_value;
use apr_runtime::{RunEnvelope, Runtime, Submission};
use serde_json::{Value, json};

fn template(name: &str) -> Value {
    let source = match name {
        "classify" => include_str!("../../../examples/adaptive-classifier/classify/process.json"),
        "propose" => include_str!("../../../examples/adaptive-classifier/propose/process.json"),
        _ => include_str!("../../../examples/adaptive-classifier/evaluate/process.json"),
    };
    serde_json::from_str(source).unwrap()
}
fn discovery() -> Value {
    json!([{"id":"d1","text":"Refund an accidental double charge."},{"id":"d2","text":"Invoice for a cancelled order."}])
}
struct Fixture {
    root: tempfile::TempDir,
    runtime: Runtime,
    token: String,
    assignment: String,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut runtime = Runtime::open(root.path().join("state")).unwrap();
        let p = runtime.attach_project(&project).unwrap();
        let a = runtime
            .create_assignment(p["project_id"].as_str().unwrap(), "Classifier test")
            .unwrap();
        let assignment = a["assignment_id"].as_str().unwrap().to_owned();
        let token = runtime.create_attachment(&assignment, "author").unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            root,
            runtime,
            token,
            assignment,
        }
    }
    fn call(&mut self, command: &str, args: Value) -> Value {
        self.runtime
            .scoped_call(&self.token, command, args)
            .unwrap()
    }
    fn register(&mut self, manifest: &Value) -> String {
        std::fs::write(
            self.root.path().join("project/process.json"),
            serde_json::to_vec(manifest).unwrap(),
        )
        .unwrap();
        self.call(
            "manage",
            json!({"action":"process.register","path":"process.json"}),
        )["process_ref"]
            .as_str()
            .unwrap()
            .into()
    }
    fn start(&mut self, version: &str, input: Value, key: &str) -> String {
        self.call(
            "run",
            json!({"mode":"start","process_ref":version,"input":input,"request_key":key}),
        )["run_id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn finish(&mut self, run_id: &str) -> RunEnvelope {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            self.runtime.drive_pending().unwrap();
            let run = self.runtime.inspect(run_id).unwrap();
            if !matches!(run.state.as_str(), "queued" | "running") {
                return run;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "did not settle: {run:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn answer(&mut self, run: &RunEnvelope) -> Value {
        let request = run.request.as_ref().unwrap();
        let submission = Submission {
            protocol_version: "0.1".into(),
            action: "submit".into(),
            run_id: run.run_id.clone(),
            request_id: request.request_id.clone(),
            step_instance_id: request.step_instance_id.clone(),
            generation: request.generation,
            submission_key: "proposal".into(),
            continuation_token: request.continuation_token.clone(),
            output: json!({"additions":[{"id":"billing","description":"Invoices and refunds.","example_ids":["d1","d2"],"rationale":"Two distinct billing issues."}]}),
            provenance: json!({"source":"synthetic_fixture"}),
        };
        self.call("resume", serde_json::to_value(submission).unwrap());
        let done = self.finish(&run.run_id);
        assert_eq!(done.state, "succeeded", "{:?}", done.error);
        done.output.unwrap()["candidate"].clone()
    }
}

#[test]
fn proposal_handoff_survives_restart_and_does_not_publish_or_receive_heldout() {
    let mut f = Fixture::new();
    let base = template("classify");
    let base_ref = f.register(&base);
    let propose = f.register(&template("propose"));
    let run = f.start(
        &propose,
        json!({"base_manifest":base,"base_process_ref":base_ref,"cases":discovery()}),
        "propose",
    );
    let waiting = f.finish(&run);
    assert_eq!(waiting.state, "waiting_for_agent");
    assert!(
        waiting
            .request
            .as_ref()
            .unwrap()
            .input
            .to_string()
            .contains("Refund")
    );
    assert!(
        !waiting
            .request
            .as_ref()
            .unwrap()
            .input
            .to_string()
            .contains("heldout")
    );
    f.runtime = Runtime::open(f.root.path().join("state")).unwrap();
    let candidate = f.answer(&waiting);
    assert_eq!(candidate["status"], "needs_evaluation");
    assert_eq!(
        candidate["candidate_process_ref"],
        admit_value(&candidate["candidate_manifest"])
            .unwrap()
            .version
    );
    // A proposal is not registered or executed as a side effect.
    assert!(
        f.runtime
            .scoped_call(
                &f.token,
                "discover",
                json!({"process_ref":candidate["candidate_process_ref"],"detail":"schema"})
            )
            .is_err()
    );
    let reader = f.runtime.create_attachment(&f.assignment, "run").unwrap();
    assert!(f.runtime.scoped_call(reader["token"].as_str().unwrap(),"manage",json!({"action":"config.set","process_ref":base_ref,"values":{"learning_enabled":true}})).is_err());
}

#[cfg(all(feature = "jev", feature = "fault-injection"))]
mod live_contract {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    struct Server {
        stop: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let result = self.thread.take().unwrap().join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
    fn configure(f: &mut Fixture) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut connection: apr_connectors::Connection = serde_json::from_str(include_str!(
            "../../../examples/adaptive-classifier/connection.json"
        ))
        .unwrap();
        connection.endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        connection.loopback_test = true;
        f.runtime
            .configure_connection(
                &f.assignment,
                "typesafe",
                connection,
                Some(apr_connectors::Secret::new(b"classifier-test-only".to_vec()).unwrap()),
            )
            .unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let flag = stop.clone();
        let count = calls.clone();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let request: Value = loop {
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        assert!(headers.contains("Bearer classifier-test-only"));
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break serde_json::from_slice(&bytes[end + 4..end + 4 + length])
                                .unwrap();
                        }
                    }
                };
                count.fetch_add(1, Ordering::SeqCst);
                let text = request["state"].as_str().unwrap();
                let criteria = request["questions"]["category"]["criteria"]
                    .as_object()
                    .unwrap();
                let choice = if text.starts_with("Account") {
                    "account"
                } else if (text.starts_with("Refund") || text.starts_with("Invoice"))
                    && criteria.contains_key("billing")
                {
                    "billing"
                } else {
                    "other"
                };
                let probabilities: serde_json::Map<String, Value> = criteria
                    .keys()
                    .map(|key| (key.clone(), json!(if key == choice { 1.0 } else { 0.0 })))
                    .collect();
                let body=json!({"model":"jev-fixture","answers":{"category":{"type":"choice","choice":choice,"confidence":0.95,"probabilities":probabilities}},"usage":{"input_tokens":10,"output_tokens":0}}).to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        Server {
            stop,
            calls,
            thread: Some(thread),
        }
    }

    #[test]
    fn optional_collection_scoped_history_proposal_evaluation_and_version_registration() {
        let mut f = Fixture::new();
        let server = configure(&mut f);
        let base = template("classify");
        let base_ref = f.register(&base);
        let input = json!({"records":discovery()});
        assert!(f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":base_ref,"input":input,"request_key":"forged-opt-in","config_overrides":{"learning_enabled":true}})).is_err());
        let off = f.start(&base_ref, input.clone(), "off");
        let done = f.finish(&off);
        assert_eq!(done.state, "succeeded", "{done:?}");
        assert_eq!(done.output.unwrap()["learning"]["other_count"], 0);
        f.call("manage",json!({"action":"config.set","process_ref":base_ref,"values":{"learning_enabled":true}}));
        let one = f.start(&base_ref, input.clone(), "one");
        assert_eq!(f.finish(&one).output.unwrap()["learning"]["other_count"], 2);
        let two = f.start(&base_ref, input, "two");
        assert_eq!(f.finish(&two).state, "succeeded");
        assert_eq!(server.calls.load(Ordering::SeqCst), 6);
        f.runtime = Runtime::open(f.root.path().join("state")).unwrap();
        let page = f.call(
            "inspect",
            json!({"view":"learning_cases","process_ref":base_ref,"limit":1}),
        );
        assert_eq!(page["batches"].as_array().unwrap().len(), 1);
        assert_eq!(page["batches"][0]["run_id"], one);
        assert_eq!(page["truncated"], true);
        let end = f.call(
            "inspect",
            json!({"view":"learning_cases","process_ref":base_ref,"after":page["next_cursor"]}),
        );
        assert_eq!(end["batches"][0]["run_id"], two);
        assert_eq!(end["truncated"], false);
        let project = f
            .runtime
            .attach_project(&f.root.path().join("project"))
            .unwrap();
        let other = f
            .runtime
            .create_assignment(project["project_id"].as_str().unwrap(), "other")
            .unwrap();
        let other = f
            .runtime
            .create_attachment(other["assignment_id"].as_str().unwrap(), "author")
            .unwrap();
        assert_eq!(
            f.runtime
                .scoped_call(
                    other["token"].as_str().unwrap(),
                    "inspect",
                    json!({"view":"learning_cases","process_ref":base_ref})
                )
                .unwrap_err()
                .code(),
            "FORBIDDEN"
        );
        // Even with its own registration of the same public template, another
        // assignment cannot reuse our cursor or see our collected records.
        f.runtime
            .scoped_call(
                other["token"].as_str().unwrap(),
                "manage",
                json!({"action":"process.register","path":"process.json"}),
            )
            .unwrap();
        let empty = f
            .runtime
            .scoped_call(
                other["token"].as_str().unwrap(),
                "inspect",
                json!({"view":"learning_cases","process_ref":base_ref}),
            )
            .unwrap();
        assert_eq!(empty["batches"], json!([]));
        assert_eq!(
            f.runtime
                .scoped_call(
                    other["token"].as_str().unwrap(),
                    "inspect",
                    json!({"view":"learning_cases","process_ref":base_ref,"after":one})
                )
                .unwrap_err()
                .code(),
            "FORBIDDEN"
        );
        let propose = f.register(&template("propose"));
        let proposal_run = f.start(
            &propose,
            json!({"base_manifest":base,"base_process_ref":base_ref,"cases":discovery()}),
            "propose",
        );
        let waiting = f.finish(&proposal_run);
        let candidate = f.answer(&waiting);
        let evaluate = f.register(&template("evaluate"));
        let heldout = json!([
            {"id":"a1","text":"Account password reset","expected":"account"},{"id":"a2","text":"Account profile change","expected":"account"},
            {"id":"b1","text":"Refund a mistaken purchase","expected":"billing"},{"id":"b2","text":"Invoice amount looks wrong","expected":"billing"},
            {"id":"x1","text":"Weather tomorrow","expected":"other"},{"id":"x2","text":"Suggest a novel","expected":"other"}]);
        let eval = f.start(
            &evaluate,
            json!({"candidate":candidate,"heldout":heldout}),
            "evaluate",
        );
        let result = f.finish(&eval);
        assert_eq!(result.state, "succeeded", "{:?}", result.error);
        let report = result.output.unwrap()["report"].clone();
        assert_eq!(report["passed"], true);
        assert_eq!(server.calls.load(Ordering::SeqCst), 18);
        let next = f.register(&report["candidate_manifest"]);
        assert_eq!(next, report["candidate_process_ref"]);
        assert_ne!(next, base_ref);
        // The original config/history and accepted versions remain unchanged.
        assert_eq!(f.runtime.inspect(&one).unwrap().process_version, base_ref);
        let next_run = f.start(&next, json!({"records":discovery()}), "new-version");
        let done = f.finish(&next_run);
        assert_eq!(done.state, "succeeded", "{:?}", done.error);
        let out = done.output.unwrap();
        assert_eq!(
            out["observations"]["items"][0]["output"]["answers"]["category"]["choice"],
            "billing"
        );
        assert_eq!(out["learning"]["enabled"], false);
        assert_eq!(server.calls.load(Ordering::SeqCst), 20);
    }
}
