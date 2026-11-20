use std::path::{Path, PathBuf};
use std::process::Command;

use apr_runtime::computer::{ComputerOptions, DecisionSource, WORKER_SOURCE, manifest, scaffold};
use apr_runtime::{RunEnvelope, Runtime};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn options() -> ComputerOptions {
    ComputerOptions {
        allowed_apps: vec!["org.apr.Fixture".into()],
        max_steps: 2,
        ..ComputerOptions::default()
    }
}

fn python() -> PathBuf {
    let interpreter = std::env::var_os("APR_TEST_PYTHON").unwrap_or_else(|| "python3".into());
    let output = Command::new(interpreter)
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .expect("computer worker contract tests require Python 3");
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

#[test]
fn generated_graphs_admit_at_both_bounds_without_jumping_effect_boundaries() {
    for source in [DecisionSource::Caller, DecisionSource::Jev] {
        for max_steps in [1, 12] {
            let process = manifest(&ComputerOptions {
                decision_source: source,
                max_steps,
                ..options()
            })
            .unwrap();
            let admitted = apr_core::admit_value(&process).unwrap();
            assert_eq!(admitted.manifest.steps.len(), max_steps as usize * 5 + 4);
            assert!(
                admitted
                    .manifest
                    .steps
                    .iter()
                    .all(|step| step.retry.max_attempts == 1)
            );
            assert_eq!(
                process["steps"].as_array().unwrap().last().unwrap()["kind"],
                "join"
            );
            assert_eq!(
                process["limits"]["max_provider_calls"],
                if source == DecisionSource::Jev {
                    max_steps
                } else {
                    0
                }
            );
        }
    }
}

#[test]
fn scaffold_embeds_attribution_and_never_replaces_a_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("bundle");
    let result = scaffold(&destination, &options()).unwrap();
    assert_eq!(result["executed"], false);
    assert_eq!(result["granted"], false);
    assert_eq!(
        std::fs::read_to_string(destination.join("computer_worker.py")).unwrap(),
        WORKER_SOURCE
    );
    let license = std::fs::read_to_string(destination.join("LICENSE.upstream")).unwrap();
    assert!(license.contains("Copyright (c) 2026 Aaron Levin"));
    assert!(license.contains("Permission is hereby granted"));
    let notice = std::fs::read_to_string(destination.join("NOTICE.md")).unwrap();
    assert!(notice.contains(apr_runtime::computer::UPSTREAM_REVISION));
    assert!(scaffold(&destination, &options()).is_err());
}

#[test]
fn generator_requires_finite_policy_and_explicit_application_selection() {
    assert!(manifest(&ComputerOptions::default()).is_err());
    for max_steps in [0, 13] {
        assert!(
            manifest(&ComputerOptions {
                max_steps,
                ..options()
            })
            .is_err()
        );
    }
    for min_confidence in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert!(
            manifest(&ComputerOptions {
                min_confidence,
                ..options()
            })
            .is_err()
        );
    }
    assert!(
        manifest(&ComputerOptions {
            allowed_apps: vec!["org.apr.Fixture\"; do shell script".into()],
            ..options()
        })
        .is_err()
    );
}

#[test]
fn worker_contract_suite_uses_only_fake_desktops() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../integrations/computer-use/test_worker.py");
    let output = Command::new(python())
        .arg("-I")
        .arg(tests)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture_source(marker: &Path, fail_after_action: bool) -> String {
    let source = WORKER_SOURCE
        .rsplit_once("\nif __name__ == \"__main__\":")
        .unwrap()
        .0;
    format!(
        "{source}\nclass FixtureBackend:\n    def __init__(self): self.changed = False\n    def observe(self, allowed):\n        return {{'app':'org.apr.Fixture','pid':42,'window':{{'id':7,'bounds':[0,0,800,600]}},'focus':{{'role':'AXButton'}},'targets':[{{'label':'Changed' if self.changed else 'Continue','bounds':[20,20,100,30]}}]}}\n    def perform(self, action, screen, config):\n        with open({}, 'a') as file: file.write(action + '\\n')\n        self.changed = True\n        if {}: raise RuntimeError('effect completed but response lost')\njson.dump(handle(json.load(sys.stdin), FixtureBackend(), lambda: 100000), sys.stdout)\n",
        serde_json::to_string(&marker.to_string_lossy()).unwrap(),
        if fail_after_action { "True" } else { "False" }
    )
}

struct Fixture {
    _directory: tempfile::TempDir,
    state: PathBuf,
    marker: PathBuf,
    runtime: Runtime,
    token: String,
    run: String,
}

impl Fixture {
    fn new(fail_after_action: bool) -> Self {
        Self::configured(fail_after_action, options())
    }

    fn configured(fail_after_action: bool, options: ComputerOptions) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let marker = project.join("actions.log");
        let state = directory.path().join("state");
        let mut runtime = Runtime::open(&state).unwrap();
        let project_id = runtime.attach_project(&project).unwrap()["project_id"].clone();
        let assignment = runtime
            .create_assignment(project_id.as_str().unwrap(), "UI fixture")
            .unwrap()["assignment_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let token = runtime.create_attachment(&assignment, "author").unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        runtime.grant_python(&assignment, &python()).unwrap();
        let source = fixture_source(&marker, fail_after_action);
        let mut process = manifest(&options).unwrap();
        process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(source.as_bytes())));
        std::fs::write(project.join("computer_worker.py"), source).unwrap();
        std::fs::write(
            project.join("process.json"),
            serde_json::to_vec(&process).unwrap(),
        )
        .unwrap();
        let version = runtime
            .scoped_call(
                &token,
                "manage",
                json!({"action":"process.register","path":"process.json"}),
            )
            .unwrap()["process_ref"]
            .clone();
        let run = runtime
            .scoped_call(
                &token,
                "run",
                json!({"mode":"start","process_ref":version,
            "input":{"goal":"Complete a synthetic task"},"request_key":"computer-fixture"}),
            )
            .unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            _directory: directory,
            state,
            marker,
            runtime,
            token,
            run,
        }
    }

    fn advance(&mut self) -> RunEnvelope {
        for _ in 0..100 {
            self.runtime.drive_pending().unwrap();
            let run = self.runtime.inspect(&self.run).unwrap();
            if !matches!(run.state.as_str(), "queued" | "running") {
                return run;
            }
        }
        self.runtime.inspect(&self.run).unwrap()
    }

    fn submit(&mut self, run: &RunEnvelope, output: Value) -> Value {
        let request = run.request.as_ref().unwrap();
        self.runtime
            .scoped_call(
                &self.token,
                "resume",
                json!({
            "protocol_version":"0.1","action":"submit","run_id":self.run,
            "request_id":request.request_id,"step_instance_id":request.step_instance_id,
            "generation":request.generation,"submission_key":request.step_instance_id,
            "continuation_token":request.continuation_token,"output":output,
            "provenance":{"source":"synthetic fixture"}}),
            )
            .unwrap()
    }
}

#[test]
fn caller_workflow_pauses_restarts_and_requires_final_observed_verification() {
    let mut fixture = Fixture::new(false);
    let waiting = fixture.advance();
    assert_eq!(waiting.state, "waiting_for_agent");
    assert_eq!(
        waiting.request.as_ref().unwrap().capability,
        "computer_decision"
    );
    assert!(!fixture.marker.exists());
    // An acknowledged decision survives disconnect; the subsequent action gets
    // exactly one native intent under the same engine used by real desktops.
    fixture.submit(
        &waiting,
        json!({"choice":"click_target_0","confidence":0.95}),
    );
    let second = fixture.advance();
    assert_eq!(second.state, "waiting_for_agent", "{second:#?}");
    assert_eq!(
        second.request.as_ref().unwrap().step_instance_id,
        "decide_1"
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.marker)
            .unwrap()
            .lines()
            .count(),
        1
    );
    let request_id = second.request.as_ref().unwrap().request_id.clone();
    drop(fixture.runtime);
    fixture.runtime = Runtime::open(&fixture.state).unwrap();
    let after_restart = fixture.advance();
    assert_eq!(
        after_restart.request.as_ref().unwrap().request_id,
        request_id
    );
    fixture.submit(&after_restart, json!({"choice":"done","confidence":0.95}));
    let review = fixture.advance();
    assert_eq!(review.state, "waiting_for_agent");
    assert_eq!(
        review.request.as_ref().unwrap().capability,
        "computer_review"
    );
    assert_eq!(
        review.request.as_ref().unwrap().input["session"]["reason"],
        "verification_required"
    );
    assert!(review.output.is_none());
    fixture.submit(
        &review,
        json!({"outcome":"completed","summary":"Synthetic task verified",
        "evidence":"Fixture action marker and observed changed label"}),
    );
    let completed = fixture.advance();
    assert_eq!(completed.state, "succeeded");
    assert_eq!(
        completed.output.as_ref().unwrap()["review"]["outcome"],
        "completed"
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.marker)
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn lost_action_response_needs_reconciliation_and_does_not_replay_after_restart() {
    let mut fixture = Fixture::new(true);
    let waiting = fixture.advance();
    fixture.submit(
        &waiting,
        json!({"choice":"click_target_0","confidence":0.95}),
    );
    assert_eq!(fixture.advance().state, "needs_reconciliation");
    drop(fixture.runtime);
    fixture.runtime = Runtime::open(&fixture.state).unwrap();
    assert_eq!(fixture.advance().state, "needs_reconciliation");
    assert_eq!(
        std::fs::read_to_string(&fixture.marker)
            .unwrap()
            .lines()
            .count(),
        1
    );
    let effects = fixture
        .runtime
        .scoped_call(
            &fixture.token,
            "inspect",
            json!({"view":"effects","run_id":fixture.run}),
        )
        .unwrap();
    assert!(
        effects["intents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|intent| intent["status"] == "uncertain" && intent["step_instance_id"] == "act_0")
    );
}

#[test]
fn low_confidence_skips_remaining_decisions_and_hands_off_without_clicking() {
    let mut fixture = Fixture::new(false);
    let waiting = fixture.advance();
    fixture.submit(
        &waiting,
        json!({"choice":"click_target_0","confidence":0.1}),
    );
    let review = fixture.advance();
    assert_eq!(review.state, "waiting_for_agent", "{review:#?}");
    assert_eq!(
        review.request.as_ref().unwrap().capability,
        "computer_review"
    );
    assert_eq!(
        review.request.as_ref().unwrap().input["session"]["reason"],
        "insufficient_confidence"
    );
    assert!(!fixture.marker.exists());
    assert!(
        review
            .steps
            .iter()
            .any(|step| step.id == "decide_1" && step.state == "skipped")
    );
}

#[test]
fn visible_condition_already_met_completes_without_a_decision_or_click() {
    let mut fixture = Fixture::configured(
        false,
        ComputerOptions {
            max_steps: 12,
            success_text: Some("Continue".into()),
            ..options()
        },
    );
    let completed = fixture.advance();
    assert_eq!(completed.state, "succeeded", "{completed:#?}");
    assert!(completed.request.is_none());
    assert_eq!(
        completed.output.as_ref().unwrap()["review"]["outcome"],
        "completed"
    );
    assert_eq!(
        completed.output.as_ref().unwrap()["review"]["evidence"]["kind"],
        "exact_ocr_block"
    );
    assert!(!fixture.marker.exists());
}

#[test]
fn explicit_visible_condition_after_last_action_completes_without_final_handoff() {
    let mut fixture = Fixture::configured(
        false,
        ComputerOptions {
            max_steps: 1,
            success_text: Some("Changed".into()),
            ..options()
        },
    );
    let waiting = fixture.advance();
    fixture.submit(
        &waiting,
        json!({"choice":"click_target_0","confidence":0.95}),
    );
    let completed = fixture.advance();
    assert_eq!(completed.state, "succeeded", "{completed:#?}");
    assert_eq!(
        completed.output.as_ref().unwrap()["session"]["complete_verified"],
        true
    );
    assert_eq!(completed.output.as_ref().unwrap()["session"]["steps"], 1);
    assert_eq!(
        std::fs::read_to_string(&fixture.marker)
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[cfg(all(feature = "jev", feature = "fault-injection"))]
#[test]
fn protected_jev_decision_drives_one_native_action_without_worker_credentials() {
    use apr_connectors::{Auth, Connection, CredentialStore, DispatchError, HttpBroker, Secret};
    use apr_runtime::worker::{InterpreterPin, WorkerLimits, WorkerRequest, execute};
    use std::io::{Read, Write};
    use std::sync::atomic::AtomicBool;
    struct Vault;
    impl CredentialStore for Vault {
        fn get(&self, _: &str) -> Result<Secret, DispatchError> {
            Secret::new(b"computer-fixture-secret".to_vec())
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("actions.log");
    let code = fixture_source(&marker, false);
    let config = manifest(&ComputerOptions {
        decision_source: DecisionSource::Jev,
        ..options()
    })
    .unwrap()["config"]["defaults"]
        .clone();
    let pin = InterpreterPin::inspect(&python()).unwrap();
    let invoke = |input: &Value| {
        execute(
            WorkerRequest {
                run_id: None,
                interpreter: &pin,
                code: code.as_bytes(),
                input,
                config: &config,
                artifacts: &[],
                limits: WorkerLimits::default(),
            },
            &AtomicBool::new(false),
        )
        .unwrap()
        .output
    };
    let observation =
        invoke(&json!({"operation":"observe","goal":"Complete synthetic record","session":null}));
    let request = apr_provider_jev::prepare(
        json!({"model":observation["model"],"state":observation["state"],
        "questions":observation["questions"]}),
    )
    .unwrap();
    let probabilities: serde_json::Map<String, Value> =
        observation["questions"]["action"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .map(|key| {
                (
                    key.clone(),
                    json!(if key == "click_target_0" { 1.0 } else { 0.0 }),
                )
            })
            .collect();
    let response = json!({"model":"jev-fixture","answers":{"action":{"type":"choice",
        "choice":"click_target_0","probabilities":probabilities,"confidence":0.95}},
        "usage":{"input_tokens":42,"output_tokens":0}})
    .to_string();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut data = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buffer[..n]);
            if let Some(index) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..index]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                if data.len() >= index + 4 + length {
                    break;
                }
            }
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .unwrap();
        data
    });
    let connection = Connection {
        mcp: None,
        metering: None,
        label: "Computer fixture".into(),
        credential_label: None,
        endpoint,
        method: "POST".into(),
        auth: Auth::Bearer {
            credential_ref: "fixture".into(),
        },
        input_schema: json!({"type":"object"}),
        output_schema: json!({"type":"object"}),
        timeout_ms: 3000,
        max_response_bytes: 8192,
        loopback_test: true,
    };
    let response = HttpBroker::new(Vault)
        .dispatch(&connection, request.body())
        .unwrap();
    let decision =
        serde_json::to_value(apr_provider_jev::observe(&request, response.output).unwrap())
            .unwrap();
    let result = invoke(&json!({"operation":"act","observation":observation,"decision":decision}));
    assert_eq!(result["steps"], 1);
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "click_target_0\n"
    );
    assert!(!result.to_string().contains("computer-fixture-secret"));
    assert!(
        String::from_utf8(server.join().unwrap())
            .unwrap()
            .contains("Bearer computer-fixture-secret")
    );
}
