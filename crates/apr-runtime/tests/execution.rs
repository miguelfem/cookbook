use apr_runtime::Runtime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

struct Fixture {
    _temp: tempfile::TempDir,
    runtime: Runtime,
    project: PathBuf,
    token: String,
    assignment: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut runtime = Runtime::open(temp.path().join("state")).unwrap();
        let attached = runtime.attach_project(&project).unwrap();
        let assignment = runtime
            .create_assignment(
                attached["project_id"].as_str().unwrap(),
                "execution fixture",
            )
            .unwrap()["assignment_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let token = runtime.create_attachment(&assignment, "author").unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            _temp: temp,
            runtime,
            project,
            token,
            assignment,
        }
    }
    fn register(&mut self, process: &Value) -> String {
        std::fs::write(
            self.project.join("process.json"),
            serde_json::to_vec(process).unwrap(),
        )
        .unwrap();
        self.runtime
            .scoped_call(
                &self.token,
                "manage",
                json!({"action":"process.register","path":"process.json"}),
            )
            .unwrap()["process_ref"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn start(&mut self, version: &str, input: Value, key: &str) -> Value {
        self.runtime
            .scoped_call(
                &self.token,
                "run",
                json!({"process_ref":version,"input":input,"request_key":key,"mode":"start"}),
            )
            .unwrap()
    }
    fn finish(&mut self, run: &str) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            self.runtime.drive_pending().unwrap();
            let value = serde_json::to_value(self.runtime.inspect(run).unwrap()).unwrap();
            if !matches!(
                value["state"].as_str(),
                Some("queued" | "running" | "waiting_for_child")
            ) {
                return value;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "run did not settle: {value}"
            );
            // Child scheduling order is not a fixed number of drive passes.
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}
fn python() -> Option<PathBuf> {
    let output = std::process::Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    path.is_absolute().then_some(path)
}
fn native_process() -> Value {
    serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap()
}

#[test]
fn native_registration_freezes_source_and_requires_assignment_grant() {
    let Some(python) = python() else {
        eprintln!("Python not installed: native execution fixture skipped");
        return;
    };
    let mut fixture = Fixture::new();
    std::fs::write(
        fixture.project.join("main.py"),
        include_bytes!("../../../examples/native-normalize/main.py"),
    )
    .unwrap();
    let version = fixture.register(&native_process());
    assert!(fixture.runtime.scoped_call(&fixture.token,"run",json!({"process_ref":version,"input":{"text":"hello  world"},"request_key":"denied","mode":"start"})).is_err());
    fixture
        .runtime
        .grant_python(&fixture.assignment, &python)
        .unwrap();
    let queued = fixture.start(&version, json!({"text":"hello  world"}), "saved");
    std::fs::write(
        fixture.project.join("main.py"),
        b"raise RuntimeError('must not run edited source')",
    )
    .unwrap();
    let done = fixture.finish(queued["run_id"].as_str().unwrap());
    assert_eq!(done["state"], "succeeded");
    assert_eq!(
        done["output"]["result"],
        json!({"normalized":"hello world","word_count":2})
    );
    let effects = fixture
        .runtime
        .scoped_call(
            &fixture.token,
            "inspect",
            json!({"view":"effects","run_id":queued["run_id"]}),
        )
        .unwrap();
    assert_eq!(effects["intents"].as_array().unwrap().len(), 1);
    assert_eq!(effects["intents"][0]["status"], "succeeded");
}

#[test]
fn failed_native_effect_keeps_safe_diagnostic_after_reopen_without_replay() {
    let Some(python) = python() else {
        return;
    };
    let mut fixture = Fixture::new();
    let marker = fixture.project.join("native-attempts.txt");
    let secret = "PRIVATE_CHILD_STDERR_DO_NOT_PERSIST";
    let code = b"import json,sys\nr=json.load(sys.stdin)['input']\nwith open(r['marker'],'a') as f: f.write('attempt\\n')\nsys.stderr.write(r['secret'])\nsys.exit(37)\n";
    let mut process = native_process();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(code)));
    process["input_schema"] = json!({"type":"object","required":["marker","secret"],
        "properties":{"marker":{"type":"string"},"secret":{"type":"string"}},
        "additionalProperties":false});
    process["steps"][0]["inputs"] = json!({"marker":{"input":"/marker"},
        "secret":{"input":"/secret"}});
    std::fs::write(fixture.project.join("main.py"), code).unwrap();
    let version = fixture.register(&process);
    fixture
        .runtime
        .grant_python(&fixture.assignment, &python)
        .unwrap();
    let run = fixture.start(
        &version,
        json!({"marker":marker,"secret":secret}),
        "native-exit",
    )["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let done = fixture.finish(&run);
    assert_eq!(done["state"], "needs_reconciliation", "{done}");
    assert_eq!(done["steps"][0]["state"], "needs_reconciliation");
    assert_eq!(done["steps"][0]["attempts"], 1);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "attempt\n");

    fixture.runtime = Runtime::open(fixture._temp.path().join("state")).unwrap();
    assert_eq!(fixture.finish(&run)["state"], "needs_reconciliation");
    assert_eq!(
        fixture.start(
            &version,
            json!({"marker":marker,"secret":secret}),
            "native-exit"
        )["run_id"],
        run
    );
    let reopened = fixture.finish(&run);
    assert_eq!(reopened["steps"][0]["attempts"], 1);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "attempt\n");
    let effects = fixture
        .runtime
        .scoped_call(
            &fixture.token,
            "inspect",
            json!({"view":"effects","run_id":run}),
        )
        .unwrap();
    assert_eq!(effects["intents"].as_array().unwrap().len(), 1);
    assert_eq!(effects["intents"][0]["status"], "uncertain");
    let events = fixture
        .runtime
        .scoped_call(
            &fixture.token,
            "inspect",
            json!({"view":"events","run_id":run,"limit":100}),
        )
        .unwrap();
    let unfinished: Vec<_> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["kind"] == "effect.unfinished")
        .collect();
    assert_eq!(unfinished.len(), 1);
    assert_eq!(
        unfinished[0]["detail"]["diagnostic"],
        json!({"stage":"native_worker","code":"worker_exit","exit_code":37})
    );
    for visible in [&done, &reopened, &effects, &events] {
        assert!(!visible.to_string().contains(secret));
    }
}

#[test]
fn invalid_native_result_records_only_fixed_validation_diagnostic() {
    let Some(python) = python() else {
        return;
    };
    let mut fixture = Fixture::new();
    let secret = "PRIVATE_INVALID_OUTPUT_DO_NOT_PERSIST";
    let code = b"import json,sys\nprint(json.dumps({'protocol_version':'0.1','output':{'normalized':json.load(sys.stdin)['input']['text']}}))\n";
    let mut process = native_process();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(code)));
    std::fs::write(fixture.project.join("main.py"), code).unwrap();
    let version = fixture.register(&process);
    fixture
        .runtime
        .grant_python(&fixture.assignment, &python)
        .unwrap();
    let run = fixture.start(&version, json!({"text":secret}), "invalid-native-result")["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let done = fixture.finish(&run);
    assert_eq!(done["state"], "needs_reconciliation", "{done}");
    assert_eq!(done["steps"][0]["attempts"], 1);
    let events = fixture
        .runtime
        .scoped_call(
            &fixture.token,
            "inspect",
            json!({"view":"events","run_id":run,"limit":100}),
        )
        .unwrap();
    let unfinished = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["kind"] == "effect.unfinished")
        .unwrap();
    assert_eq!(
        unfinished["detail"]["diagnostic"],
        json!({"stage":"result_validation","code":"result_contract_failed"})
    );
    assert!(!events.to_string().contains(secret));
}

#[test]
fn changed_binding_requires_owner_rebind_and_changed_source_digest_is_rejected() {
    let Some(python) = python() else {
        return;
    };
    let mut fixture = Fixture::new();
    std::fs::write(
        fixture.project.join("main.py"),
        include_bytes!("../../../examples/native-normalize/main.py"),
    )
    .unwrap();
    let version = fixture.register(&native_process());
    fixture
        .runtime
        .grant_python(&fixture.assignment, &python)
        .unwrap();
    let queued = fixture.start(&version, json!({"text":"still pinned"}), "revision");
    let replacement = fixture._temp.path().join("replacement-python");
    std::os::unix::fs::symlink(&python, &replacement).unwrap();
    fixture
        .runtime
        .grant_python(&fixture.assignment, &replacement)
        .unwrap();
    assert_eq!(
        fixture.finish(queued["run_id"].as_str().unwrap())["state"],
        "blocked"
    );
    fixture
        .runtime
        .rebind_execution(queued["run_id"].as_str().unwrap())
        .unwrap();
    fixture
        .runtime
        .scoped_call(
            &fixture.token,
            "resume",
            json!({"action":"continue","run_id":queued["run_id"]}),
        )
        .unwrap();
    assert_eq!(
        fixture.finish(queued["run_id"].as_str().unwrap())["state"],
        "succeeded"
    );
    std::fs::write(fixture.project.join("main.py"), "print('changed')").unwrap();
    assert!(
        fixture
            .runtime
            .scoped_call(
                &fixture.token,
                "manage",
                json!({"action":"process.register","path":"process.json"})
            )
            .is_err()
    );
}

#[test]
fn identical_python_reapproval_preserves_queued_run_binding() {
    let Some(python) = python() else {
        return;
    };
    let mut fixture = Fixture::new();
    std::fs::write(
        fixture.project.join("main.py"),
        include_bytes!("../../../examples/native-normalize/main.py"),
    )
    .unwrap();
    let version = fixture.register(&native_process());
    let first = fixture
        .runtime
        .grant_python(&fixture.assignment, &python)
        .unwrap();
    let queued = fixture.start(&version, json!({"text":"still pinned"}), "same-pin");
    let again = fixture
        .runtime
        .grant_python(&fixture.assignment, &python)
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(
        fixture.finish(queued["run_id"].as_str().unwrap())["state"],
        "succeeded"
    );
}

#[test]
fn reviewed_python_grant_rejects_stale_digest_without_changing_revision() {
    let Some(python) = python() else {
        return;
    };
    let mut fixture = Fixture::new();
    let digest = format!("{:x}", Sha256::digest(std::fs::read(&python).unwrap()));
    let stale_digest = "0".repeat(64);
    assert_ne!(digest, stale_digest);
    assert!(
        fixture
            .runtime
            .grant_python_reviewed(&fixture.assignment, &python, &stale_digest)
            .is_err()
    );
    let first = fixture
        .runtime
        .grant_python_reviewed(&fixture.assignment, &python, &digest)
        .unwrap();
    assert_eq!(first["revision"], 1);
    assert!(
        fixture
            .runtime
            .grant_python_reviewed(&fixture.assignment, &python, &stale_digest)
            .is_err()
    );
    assert_eq!(
        fixture
            .runtime
            .grant_python_reviewed(&fixture.assignment, &python, &digest)
            .unwrap(),
        first
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn effect_crash_child() {
    let Some(state) = std::env::var_os("APR_EFFECT_CHILD_STATE") else {
        return;
    };
    let run = std::env::var("APR_EFFECT_CHILD_RUN").unwrap();
    Runtime::open(state).unwrap().advance(&run).unwrap();
}

#[cfg(feature = "fault-injection")]
#[test]
fn process_death_after_native_effect_does_not_replay_the_write() {
    use sha2::{Digest, Sha256};
    let Some(python) = python() else {
        return;
    };
    for boundary in [
        "effect_dispatched:normalize",
        "effect_observed:normalize",
        "effect_committed:normalize",
    ] {
        let mut fixture = Fixture::new();
        let marker = fixture.project.join("effect-count.txt");
        let code=b"import json,sys\nr=json.load(sys.stdin)\nwith open(r['input']['marker'],'a') as f: f.write('one\\n')\nprint(json.dumps({'protocol_version':'0.1','output':{'appended':True}}))\n";
        let mut process = native_process();
        process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(code)));
        process["input_schema"] = json!({"type":"object","required":["marker"],"properties":{"marker":{"type":"string"}},"additionalProperties":false});
        process["steps"][0]["inputs"] = json!({"marker":{"input":"/marker"}});
        process["steps"][0]["result_schema"] = json!({"type":"object","required":["appended"],"properties":{"appended":{"const":true}},"additionalProperties":false});
        std::fs::write(fixture.project.join("main.py"), code).unwrap();
        let version = fixture.register(&process);
        fixture
            .runtime
            .grant_python(&fixture.assignment, &python)
            .unwrap();
        let queued = fixture.start(&version, json!({"marker":marker}), "crash");
        let run = queued["run_id"].as_str().unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "effect_crash_child", "--nocapture"])
            .env("APR_EFFECT_CHILD_STATE", fixture._temp.path().join("state"))
            .env("APR_EFFECT_CHILD_RUN", run)
            .env("APR_TEST_CRASH_AT", boundary)
            .output()
            .unwrap();
        assert!(
            !child.status.success(),
            "child did not reach crash boundary {boundary}"
        );
        let done = fixture.finish(run);
        let count = std::fs::read_to_string(&marker)
            .unwrap_or_default()
            .lines()
            .count();
        if boundary.starts_with("effect_dispatched") {
            assert_eq!(count, 0);
            assert_eq!(done["state"], "needs_reconciliation");
        } else if boundary.starts_with("effect_observed") {
            assert_eq!(count, 1);
            assert_eq!(done["state"], "needs_reconciliation");
        } else {
            assert_eq!(count, 1);
            assert_eq!(done["state"], "succeeded");
        }
    }
}

#[cfg(feature = "fault-injection")]
mod http {
    use super::*;
    use apr_connectors::{Auth, Connection};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    fn setup(response: Option<&str>) -> (Fixture, String, std::thread::JoinHandle<usize>) {
        setup_metered(response, None)
    }
    fn setup_metered(
        response: Option<&str>,
        metering: Option<apr_connectors::metering::MeteringConfig>,
    ) -> (Fixture, String, std::thread::JoinHandle<usize>) {
        let mut fixture = Fixture::new();
        let server = configure(&mut fixture, response, metering);
        let process =
            serde_json::from_str(include_str!("../../../examples/http-json/process.json")).unwrap();
        let version = fixture.register(&process);
        (fixture, version, server)
    }
    fn configure(
        fixture: &mut Fixture,
        response: Option<&str>,
        metering: Option<apr_connectors::metering::MeteringConfig>,
    ) -> std::thread::JoinHandle<usize> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/operation", listener.local_addr().unwrap());
        let response = response.map(str::to_owned);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0; 4096];
            let n = stream.read(&mut bytes).unwrap();
            if let Some(body) = response {
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(reply.as_bytes()).unwrap();
            }
            n
        });
        let connection = Connection {
            mcp: None,
            metering,
            label: "Local fixture".into(),
            credential_label: None,
            endpoint,
            method: "POST".into(),
            auth: Auth::None,
            input_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
            timeout_ms: 1000,
            max_response_bytes: 4096,
            loopback_test: true,
        };
        fixture
            .runtime
            .configure_connection(&fixture.assignment, "records", connection, None)
            .unwrap();
        server
    }
    fn pricing(currency: &str) -> apr_connectors::metering::MeteringConfig {
        // Synthetic rate, not any provider's published price.
        serde_json::from_value(
            json!({"provider":"fixture","revision":"synthetic-v1","currency":currency,
            "meters":[{"name":"requests","unit":"requests","add":["/usage/requests"],
                "rate":{"amount":"0.001","per_units":1}}]}),
        )
        .unwrap()
    }
    fn analytics(f: &mut Fixture, args: Value) -> Value {
        f.runtime.scoped_call(&f.token, "inspect", args).unwrap()
    }
    #[test]
    fn lost_reply_is_not_replayed_and_observed_result_can_complete_the_step() {
        let (mut fixture, version, server) = setup(None);
        let run = fixture.start(&version, json!({"id":"record1"}), "one");
        let id = run["run_id"].as_str().unwrap();
        assert_eq!(fixture.finish(id)["state"], "needs_reconciliation");
        assert!(server.join().unwrap() > 0);
        // Reconnect/start replay does not dispatch again; the listening socket is gone.
        assert_eq!(
            fixture.start(&version, json!({"id":"record1"}), "one")["run_id"],
            id
        );
        assert_eq!(fixture.finish(id)["state"], "needs_reconciliation");
        let effects = fixture
            .runtime
            .scoped_call(
                &fixture.token,
                "inspect",
                json!({"view":"effects","run_id":id}),
            )
            .unwrap();
        assert_eq!(effects["intents"].as_array().unwrap().len(), 1);
        fixture.runtime.scoped_call(&fixture.token,"resume",json!({"action":"reconcile","run_id":id,"step_instance_id":"request","intent_id":effects["intents"][0]["intent_id"],"outcome":"observed_result","evidence":"Read back record1 through the controlled fixture","output":{"saved":true}})).unwrap();
        assert_eq!(
            fixture.finish(id)["output"]["result"],
            json!({"saved":true})
        );
    }
    #[test]
    fn cancelling_an_uncertain_effect_closes_remaining_work() {
        let (mut fixture, _, server) = setup(None);
        let mut process: Value =
            serde_json::from_str(include_str!("../../../examples/http-json/process.json")).unwrap();
        process["capabilities"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind":"builtin"}));
        process["steps"].as_array_mut().unwrap().push(json!({"id":"after","kind":"builtin","operation":"json.select","effect":"pure","needs":["request"],"retry":{"max_attempts":1},"inputs":{"value":{"step":"request","pointer":""},"pointer":{"literal":""}}}));
        let version = fixture.register(&process);
        let started = fixture.start(&version, json!({"id":"cancel"}), "cancel-uncertain");
        let run = started["run_id"].as_str().unwrap();
        assert_eq!(fixture.finish(run)["state"], "needs_reconciliation");
        server.join().unwrap();
        let effects = fixture
            .runtime
            .scoped_call(
                &fixture.token,
                "inspect",
                json!({"view":"effects","run_id":run}),
            )
            .unwrap();
        fixture.runtime.scoped_call(&fixture.token,"resume",json!({"action":"reconcile","run_id":run,"step_instance_id":"request","intent_id":effects["intents"][0]["intent_id"],"outcome":"cancel_uncertain","evidence":"Cancelled after controlled endpoint closed without a response"})).unwrap();
        let done = fixture.finish(run);
        assert_eq!(done["state"], "cancelled");
        assert!(
            done["steps"]
                .as_array()
                .unwrap()
                .iter()
                .all(|step| step["state"] == "cancelled"),
            "{done}"
        );
    }

    #[test]
    fn child_processes_share_the_roots_provider_budget() {
        let (mut fixture, child, server) = setup(Some(r#"{"saved":true}"#));
        let mut parent: Value =
            serde_json::from_str(include_str!("../../../examples/http-json/process.json")).unwrap();
        parent["schema_version"] = json!("0.2");
        parent["name"] = json!("http.parent");
        parent["capabilities"] = json!([]);
        parent["steps"] = json!([
         {"id":"one","kind":"call","effect":"control","needs":[],"retry":{"max_attempts":1},"inputs":{"id":{"input":"/id"}},"call":{"process_ref":child}},
         {"id":"two","kind":"call","effect":"control","needs":["one"],"retry":{"max_attempts":1},"inputs":{"id":{"input":"/id"}},"call":{"process_ref":child}}
        ]);
        parent["output"] = json!({"result":{"step":"two","pointer":""}});
        let version = fixture.register(&parent);
        let run = fixture.start(&version, json!({"id":"bounded"}), "tree-budget");
        let id = run["run_id"].as_str().unwrap();
        let done = fixture.finish(id);
        assert_eq!(done["state"], "failed", "{done}");
        assert!(server.join().unwrap() > 0);
        let children: Vec<_> = fixture
            .runtime
            .events(id, 0, 100)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "child.created")
            .map(|e| e.detail["child_run_id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(children.len(), 2);
        let mut intents = 0;
        for child in children {
            let effects = fixture
                .runtime
                .scoped_call(
                    &fixture.token,
                    "inspect",
                    json!({"view":"effects","run_id":child}),
                )
                .unwrap();
            intents += effects["intents"].as_array().unwrap().len();
        }
        assert_eq!(intents, 1, "a child reset the root provider allowance");
        let aggregate = analytics(&mut fixture, json!({"view":"analytics","run_id":id}));
        assert_eq!(aggregate["summary"]["external_attempts"], 1);
        assert_eq!(aggregate["summary"]["child_runs"], 2);
        assert_eq!(aggregate["calls"].as_array().unwrap().len(), 1);
        let child_id = aggregate["calls"][0]["run_id"].clone();
        let from_child = analytics(&mut fixture, json!({"view":"analytics","run_id":child_id}));
        assert_eq!(from_child["root_run_id"], id);
        assert_eq!(from_child["summary"], aggregate["summary"]);
    }

    #[test]
    fn validated_network_result_is_committed_once() {
        let (mut fixture, version, server) = setup(Some(r#"{"saved":true}"#));
        let run = fixture.start(&version, json!({"id":"record2"}), "two");
        let id = run["run_id"].as_str().unwrap();
        assert_eq!(fixture.finish(id)["state"], "succeeded");
        assert!(server.join().unwrap() > 0);
        assert_eq!(fixture.finish(id)["state"], "succeeded");
    }

    #[test]
    fn metered_calls_survive_reopen_and_duplicate_start_without_duplicate_costs() {
        let (mut f, version, server) = setup_metered(
            Some(r#"{"saved":true,"usage":{"requests":3},"private_payload":"DO_NOT_EXPORT"}"#),
            Some(pricing("USD")),
        );
        let run = f.start(&version, json!({"id":"metered"}), "metered")["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(f.finish(&run)["state"], "succeeded");
        server.join().unwrap();
        f.runtime = Runtime::open(f._temp.path().join("state")).unwrap();
        assert_eq!(
            f.start(&version, json!({"id":"metered"}), "metered")["run_id"],
            run
        );
        assert_eq!(f.finish(&run)["state"], "succeeded");
        let view = analytics(&mut f, json!({"view":"analytics"}));
        assert_eq!(view["summary"]["external_attempts"], 1);
        assert_eq!(
            view["cost_page"]["currencies"][0]["estimated_nanos"],
            "3000000"
        );
        assert_eq!(view["calls"][0]["usage"]["provenance"], "broker_observed");
        assert!(!view.to_string().contains("DO_NOT_EXPORT"));
        assert_eq!(
            f.runtime
                .events(&run, 0, 100)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "usage.observed")
                .count(),
            1
        );
    }

    #[test]
    fn failed_output_keeps_usage_and_reconciliation_replaces_instead_of_adding() {
        let (mut f, _, server) = setup_metered(
            Some(r#"{"saved":"invalid","usage":{"requests":2}}"#),
            Some(pricing("USD")),
        );
        let mut process: Value =
            serde_json::from_str(include_str!("../../../examples/http-json/process.json")).unwrap();
        process["steps"][0]["result_schema"] =
            json!({"type":"object","required":["saved"],"properties":{"saved":{"type":"boolean"}}});
        let version = f.register(&process);
        let run = f.start(&version, json!({}), "invalid-output")["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(f.finish(&run)["state"], "needs_reconciliation");
        server.join().unwrap();
        let before = analytics(&mut f, json!({"view":"analytics","run_id":run}));
        assert_eq!(
            before["cost_page"]["currencies"][0]["estimated_nanos"],
            "2000000"
        );
        let request = json!({"action":"reconcile","run_id":run,"step_instance_id":"request",
            "intent_id":before["calls"][0]["intent_id"],"outcome":"observed_result",
            "evidence":"Read back the synthetic saved record and usage", "output":{"saved":true,"usage":{"requests":2}}});
        f.runtime
            .scoped_call(&f.token, "resume", request.clone())
            .unwrap();
        assert_eq!(f.finish(&run)["state"], "succeeded");
        assert!(f.runtime.scoped_call(&f.token, "resume", request).is_err());
        let after = analytics(&mut f, json!({"view":"analytics","run_id":run}));
        assert_eq!(after["summary"]["external_attempts"], 1);
        assert_eq!(
            after["cost_page"]["currencies"][0]["estimated_nanos"],
            "2000000"
        );
        assert_eq!(after["calls"][0]["usage"]["provenance"], "caller_reported");
        assert_eq!(
            f.runtime
                .events(&run, 0, 100)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "usage.observed")
                .count(),
            2
        );
    }

    #[test]
    fn costs_page_independently_and_currencies_are_never_combined() {
        let (mut f, version, server) =
            setup_metered(Some(r#"{"usage":{"requests":1}}"#), Some(pricing("USD")));
        let one = f.start(&version, json!({}), "usd")["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(f.finish(&one)["state"], "succeeded");
        server.join().unwrap();
        let second = configure(
            &mut f,
            Some(r#"{"usage":{"requests":1}}"#),
            Some(pricing("MXN")),
        );
        let two = f.start(&version, json!({}), "mxn")["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(f.finish(&two)["state"], "succeeded");
        second.join().unwrap();
        let all = analytics(&mut f, json!({"view":"analytics"}));
        assert_eq!(all["cost_page"]["currencies"].as_array().unwrap().len(), 2);
        assert_eq!(all["cost_page"]["scope"], "this_page_only");
        let page = analytics(&mut f, json!({"view":"analytics","limit":1}));
        assert_eq!(page["summary"]["external_attempts"], 2);
        assert_eq!(page["calls"].as_array().unwrap().len(), 1);
        assert_eq!(page["truncated"], true);
        let next = analytics(
            &mut f,
            json!({"view":"analytics","limit":1,"after":page["next_cursor"]}),
        );
        assert_eq!(next["truncated"], false);
        assert_ne!(next["calls"][0]["intent_id"], page["calls"][0]["intent_id"]);
        assert_eq!(
            next["cost_page"]["currencies"][0]["estimated_nanos"],
            "1000000"
        );
        let one_tree = analytics(&mut f, json!({"view":"analytics","run_id":one}));
        assert_eq!(one_tree["cost_page"]["currencies"][0]["currency"], "USD");
    }

    #[test]
    fn missing_usage_and_unobserved_responses_are_unknown_not_zero() {
        for response in [None, Some(r#"{"saved":true}"#)] {
            let (mut f, version, server) = setup_metered(response, Some(pricing("USD")));
            let run = f.start(&version, json!({}), "unknown")["run_id"]
                .as_str()
                .unwrap()
                .to_owned();
            f.finish(&run);
            server.join().unwrap();
            let view = analytics(&mut f, json!({"view":"analytics"}));
            assert_eq!(view["cost_page"]["unknown_or_partial_calls"], 1);
            assert!(view["calls"][0]["usage"]["observation"]["estimated_nanos"].is_null());
        }
    }
}

#[test]
fn native_artifact_exchange_publishes_references_and_reuses_input_snapshots() {
    use sha2::{Digest, Sha256};
    let Some(python) = python() else {
        return;
    };
    let mut fixture = Fixture::new();
    let code=br#"import json,sys
r=json.load(sys.stdin)
source=r['artifacts'][r['input']['source']['artifact_id']]['path']
with open(source,'rb') as f: data=f.read()
with open('result.bin','wb') as f: f.write(data[::-1])
json.dump({'protocol_version':'0.1','output':{'result':{'worker_artifact':'reversed'}},'artifacts':[{'name':'reversed','path':'result.bin','media_type':'application/octet-stream'}]},sys.stdout)
"#;
    let mut process = native_process();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(code)));
    process["input_schema"] = json!({"type":"object"});
    process["steps"][0]["inputs"] = json!({"source":{"input":"/source"}});
    process["steps"][0]["result_schema"] = json!({"type":"object"});
    std::fs::write(fixture.project.join("main.py"), code).unwrap();
    std::fs::write(fixture.project.join("source.bin"), [0_u8, 1, 2, 255]).unwrap();
    let imported=fixture.runtime.scoped_call(&fixture.token,"manage",json!({"action":"artifact.import","path":"source.bin","media_type":"application/octet-stream"})).unwrap();
    let version = fixture.register(&process);
    fixture
        .runtime
        .grant_python(&fixture.assignment, &python)
        .unwrap();
    let queued = fixture.start(&version, json!({"source":imported}), "binary");
    let done = fixture.finish(queued["run_id"].as_str().unwrap());
    assert_eq!(done["state"], "succeeded", "{done}");
    let reference: apr_core::ArtifactRef =
        serde_json::from_value(done["output"]["result"]["result"].clone()).unwrap();
    assert_eq!(reference.bytes, 4);
    let store =
        apr_runtime::artifacts::ArtifactStore::open(fixture._temp.path().join("state")).unwrap();
    assert_eq!(store.read(&reference, 4).unwrap(), [255, 2, 1, 0]);
    let inspected = fixture
        .runtime
        .scoped_call(
            &fixture.token,
            "inspect",
            json!({"view":"artifact","reference":reference}),
        )
        .unwrap();
    assert_eq!(inspected["text_available"], false);
}
