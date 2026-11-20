//! Actual process-kill checks. Default builds contain neither these tests nor
//! the runtime fault-injection branches.
#![cfg(all(feature = "fault-injection", unix))]

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    state: PathBuf,
    manifest: PathBuf,
    input: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let manifest = directory.path().join("process.json");
        let input = directory.path().join("input.json");
        std::fs::write(&manifest, serde_json::to_vec(&process()).unwrap()).unwrap();
        std::fs::write(&input, br#"{"title":"Synthetic report","items":[1,2,3]}"#).unwrap();
        Self {
            directory,
            state,
            manifest,
            input,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_apr"));
        command
            .arg("--state-dir")
            .arg(&self.state)
            .env_remove("APR_TEST_CRASH_AT");
        command
    }

    fn start(&self) -> Command {
        let mut command = self.command();
        command
            .arg("run")
            .arg(&self.manifest)
            .arg("--input-file")
            .arg(&self.input)
            .arg("--request-key")
            .arg("crash-fixture-start");
        command
    }

    fn inspect(&self, run: &str) -> Value {
        let mut command = self.command();
        command.arg("inspect").arg(run);
        success(command)
    }

    fn advance(&self, run: &str) -> Value {
        let mut command = self.command();
        command.arg("continue").arg(run);
        success(command)
    }

    fn events(&self, run: &str) -> Value {
        let mut command = self.command();
        command.arg("events").arg(run);
        success(command)
    }

    fn submission(&self, waiting: &Value) -> PathBuf {
        let request = &waiting["request"];
        let result = json!({
            "protocol_version":"0.1", "action":"submit",
            "run_id":waiting["run_id"], "request_id":request["request_id"],
            "step_instance_id":request["step_instance_id"], "generation":request["generation"],
            "submission_key":"crash-fixture-result", "continuation_token":request["continuation_token"],
            "output":{"approved":true}, "provenance":{"source":"manual","client":"crash-test"}
        });
        let path = self.directory.path().join("result.json");
        std::fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
        path
    }

    fn resume(&self, run: &str, result: &Path) -> Command {
        let mut command = self.command();
        command
            .arg("resume")
            .arg(run)
            .arg("--result-file")
            .arg(result);
        command
    }

    fn assert_export(&self, reference: &Value, expected: &[u8]) {
        let export = tempfile::tempdir_in(self.directory.path()).unwrap();
        let reference_path = export.path().join("reference.json");
        let output_path = export.path().join("artifact.json");
        std::fs::write(&reference_path, serde_json::to_vec(reference).unwrap()).unwrap();
        let mut command = self.command();
        command
            .arg("artifact")
            .arg("read")
            .arg("--reference-file")
            .arg(reference_path)
            .arg("--output-file")
            .arg(&output_path);
        let result = success(command);
        assert_eq!(result["bytes_written"], expected.len());
        assert_eq!(std::fs::read(output_path).unwrap(), expected);
    }
}

fn process() -> Value {
    json!({
        "schema_version":"0.1", "name":"crash.recovery", "description":"Synthetic durable review",
        "input_schema":{"type":"object","required":["title","items"],"properties":{"title":{"type":"string"},"items":{"type":"array","items":{"type":"integer"}}},"additionalProperties":false},
        "output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"builtin"},{"kind":"artifacts"},{"kind":"agent","capability":"review"}],
        "limits":{"max_step_ms":5000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[
            {"id":"snapshot","kind":"builtin","operation":"json.write","needs":[],"effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"input":""}}},
            {"id":"prepare","kind":"builtin","operation":"json.write","needs":["snapshot"],"effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"literal":{"total":3}}}},
            {"id":"review","kind":"agent","needs":["prepare"],"effect":"agent","retry":{"max_attempts":1},"capability":"review","instructions":"Review the prepared synthetic report.","inputs":{"artifact":{"step":"prepare","pointer":"/artifact"}},"result_schema":{"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false}},
            {"id":"finish","kind":"builtin","operation":"json.write","needs":["review"],"effect":"pure","retry":{"max_attempts":2},"inputs":{"value":{"step":"review","pointer":""}}}
        ],
        "output":{"source":{"step":"snapshot","pointer":"/artifact"},"prepared":{"step":"prepare","pointer":"/artifact"},"approved":{"step":"review","pointer":"/approved"},"result":{"step":"finish","pointer":"/artifact"}}
    })
}

fn success(mut command: Command) -> Value {
    parse_success(command.output().unwrap())
}

fn parse_success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn kill_at(mut command: Command, point: &str) {
    let output = command.env("APR_TEST_CRASH_AT", point).output().unwrap();
    assert_eq!(
        output.status.signal(),
        Some(9),
        "fault point {point} did not SIGKILL: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "killed command acknowledged before its fault point"
    );
}

fn attempts(run: &Value, id: &str) -> u64 {
    run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == id)
        .unwrap()["attempts"]
        .as_u64()
        .unwrap()
}

fn event_count(events: &Value, kind: &str) -> usize {
    events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["kind"] == kind)
        .count()
}

fn assert_finished(run: &Value) {
    assert_eq!(run["state"], "succeeded");
    assert_eq!(run["completed_steps"], 4);
    assert_eq!(run["output"]["approved"], true);
    // Independent Python hashlib fixtures for the literal canonical JSON bytes.
    assert_eq!(
        run["output"]["prepared"]["sha256"],
        "9f827af5202050f264dfa05aeec4603d0fcec968de43ed7b039ce6a2d6231b08"
    );
    assert_eq!(
        run["output"]["result"]["sha256"],
        "20ec2f38fc96bc7d416a9af537493ae0bae26c5ad4c5a0f2d94f56f9c3e04958"
    );
}

#[test]
fn start_committed_before_sigkill_recovers_one_run_with_the_original_key() {
    let fixture = Fixture::new();
    kill_at(fixture.start(), "run_created");
    let waiting = success(fixture.start());
    assert_eq!(waiting["state"], "waiting_for_agent");
    let run = waiting["run_id"].as_str().unwrap();
    assert_eq!(success(fixture.start()), waiting);
    assert_eq!(event_count(&fixture.events(run), "run.created"), 1);
    assert_eq!(attempts(&waiting, "snapshot"), 1);
    assert_eq!(attempts(&waiting, "prepare"), 1);
}

#[test]
fn sigkill_before_or_after_step_commit_retries_only_uncommitted_work() {
    for (point, expected_attempts) in [
        ("step_started:prepare", 2),
        ("step_executed:prepare", 2),
        ("step_committed:prepare", 1),
    ] {
        let fixture = Fixture::new();
        kill_at(fixture.start(), point);
        let waiting = success(fixture.start());
        assert_eq!(
            waiting["state"], "waiting_for_agent",
            "recovery after {point}"
        );
        assert_eq!(waiting["completed_steps"], 2);
        assert_eq!(
            attempts(&waiting, "snapshot"),
            1,
            "committed snapshot reran after {point}"
        );
        assert_eq!(
            attempts(&waiting, "prepare"),
            expected_attempts,
            "attempt accounting after {point}"
        );
        let run = waiting["run_id"].as_str().unwrap();
        let result = fixture.submission(&waiting);
        let resumed = success(fixture.resume(run, &result));
        assert_finished(&resumed["run"]);
        assert_eq!(attempts(&resumed["run"], "snapshot"), 1);
        assert_eq!(attempts(&resumed["run"], "prepare"), expected_attempts);
        assert_eq!(attempts(&resumed["run"], "finish"), 1);
        assert_eq!(fixture.advance(run), resumed["run"]);
        fixture.assert_export(&resumed["run"]["output"]["prepared"], br#"{"total":3}"#);
        let events = fixture.events(run);
        let successful_prepares = events
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| {
                event["kind"] == "step.succeeded" && event["detail"]["step_id"] == "prepare"
            })
            .count();
        assert_eq!(successful_prepares, 1);
        assert_eq!(
            event_count(&events, "step.interrupted"),
            (expected_attempts - 1) as usize
        );
        assert_eq!(event_count(&events, "handoff.accepted"), 1);
    }
}

#[test]
fn handoff_committed_before_sigkill_reopens_the_same_request_without_regeneration() {
    let fixture = Fixture::new();
    kill_at(fixture.start(), "handoff_committed:review");
    let waiting = success(fixture.start());
    assert_eq!(waiting["state"], "waiting_for_agent");
    let run = waiting["run_id"].as_str().unwrap();
    assert_eq!(fixture.inspect(run), waiting);
    assert_eq!(fixture.advance(run), waiting);
    assert_eq!(success(fixture.start())["request"], waiting["request"]);
    assert_eq!(event_count(&fixture.events(run), "handoff.created"), 1);
    let result = fixture.submission(&waiting);
    let resumed = success(fixture.resume(run, &result));
    assert_finished(&resumed["run"]);
    assert_eq!(attempts(&resumed["run"], "snapshot"), 1);
    assert_eq!(attempts(&resumed["run"], "prepare"), 1);
}

#[test]
fn result_commit_survives_lost_acknowledgement_and_replays_its_original_receipt() {
    let fixture = Fixture::new();
    let waiting = success(fixture.start());
    let run = waiting["run_id"].as_str().unwrap();
    let result = fixture.submission(&waiting);
    kill_at(fixture.resume(run, &result), "result_committed");
    let saved = fixture.inspect(run);
    assert_eq!(saved["state"], "queued");
    assert_eq!(saved["completed_steps"], 3);
    assert!(saved.get("request").is_none());
    assert_eq!(attempts(&saved, "finish"), 0);
    assert_eq!(event_count(&fixture.events(run), "handoff.accepted"), 1);

    let first_retry = success(fixture.resume(run, &result));
    assert_finished(&first_retry["run"]);
    assert_eq!(first_retry["receipt"]["revision"], saved["revision"]);
    assert_eq!(
        first_retry["receipt"]["request_id"],
        waiting["request"]["request_id"]
    );
    assert_eq!(first_retry["receipt"]["status"], "accepted");
    assert_eq!(
        first_retry["receipt"]["output_digest"],
        "sha256:e56975d3b4ed2558b6ecf4695de6ba528dd46bd7e0d7f65eb4a5dc2adafe55ce"
    );

    let mut submission: Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    submission["continuation_token"] =
        json!("consumed transport token is not part of the semantic result");
    std::fs::write(&result, serde_json::to_vec(&submission).unwrap()).unwrap();
    let repeated = success(fixture.resume(run, &result));
    assert_eq!(repeated, first_retry);
    assert_eq!(attempts(&repeated["run"], "snapshot"), 1);
    assert_eq!(attempts(&repeated["run"], "prepare"), 1);
    assert_eq!(attempts(&repeated["run"], "finish"), 1);
    let events = fixture.events(run);
    assert_eq!(event_count(&events, "handoff.accepted"), 1);
    assert_eq!(event_count(&events, "run.succeeded"), 1);
    fixture.assert_export(
        &repeated["run"]["output"]["result"],
        br#"{"approved":true}"#,
    );
}

#[test]
fn simultaneous_processes_replaying_a_committed_result_receive_one_receipt() {
    use std::process::Stdio;
    let fixture = Fixture::new();
    let waiting = success(fixture.start());
    let run = waiting["run_id"].as_str().unwrap();
    let result = fixture.submission(&waiting);
    kill_at(fixture.resume(run, &result), "result_committed");
    let children = (0..4)
        .map(|_| {
            fixture
                .resume(run, &result)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let responses = children
        .into_iter()
        .map(|child| parse_success(child.wait_with_output().unwrap()))
        .collect::<Vec<_>>();
    assert!(responses.iter().all(|response| response == &responses[0]));
    assert_finished(&responses[0]["run"]);
    assert_eq!(attempts(&responses[0]["run"], "snapshot"), 1);
    assert_eq!(attempts(&responses[0]["run"], "prepare"), 1);
    assert_eq!(attempts(&responses[0]["run"], "finish"), 1);
    let events = fixture.events(run);
    assert_eq!(event_count(&events, "handoff.accepted"), 1);
    assert_eq!(event_count(&events, "run.succeeded"), 1);
}

#[test]
fn renewal_recovers_a_token_published_before_generation_commit() {
    let fixture = Fixture::new();
    let waiting = success(fixture.start());
    let run = waiting["run_id"].as_str().unwrap();
    let result = fixture.submission(&waiting);
    let stale_result = fixture.directory.path().join("old-generation.json");
    std::fs::rename(result, &stale_result).unwrap();
    let mut renew = fixture.command();
    renew.arg("renew").arg(run);
    kill_at(renew, "token_published");
    assert_eq!(fixture.inspect(run), waiting);

    let mut renew = fixture.command();
    renew.arg("renew").arg(run);
    let renewed = success(renew);
    assert_eq!(renewed["state"], "waiting_for_agent");
    assert_eq!(
        renewed["request"]["request_id"],
        waiting["request"]["request_id"]
    );
    assert_eq!(renewed["request"]["generation"], 2);
    assert_ne!(
        renewed["request"]["continuation_token"],
        waiting["request"]["continuation_token"]
    );
    let rejected = fixture.resume(run, &stale_result).output().unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    let error: Value = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(error["error"]["code"], "STALE_HANDOFF");
    assert_eq!(fixture.inspect(run), renewed);

    let result = fixture.submission(&renewed);
    let resumed = success(fixture.resume(run, &result));
    assert_finished(&resumed["run"]);
    let events = fixture.events(run);
    assert_eq!(event_count(&events, "handoff.created"), 1);
    assert_eq!(event_count(&events, "handoff.renewed"), 1);
    assert_eq!(event_count(&events, "handoff.accepted"), 1);
}
