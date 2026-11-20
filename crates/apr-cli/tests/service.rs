//! Local IPC tests use the real CLI and private, disposable state directories.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use apr_runtime::service::{AttachmentFile, ServiceClient, socket_path};
use serde_json::{Value, json};
use tempfile::TempDir;

const DEFAULT_BUDGET: usize = 8 * 1024;
const DETAIL_BUDGET: usize = 64 * 1024;
// The 64-worker capacity fixture and latency-sensitive native cancellation
// fixtures each launch an independent supervisor. Keep their process startup
// measurements from competing for host CPU in this test binary.
static NATIVE_CAPACITY_FIXTURE: Mutex<()> = Mutex::new(());

struct Fixture {
    directory: TempDir,
    state: PathBuf,
    project: PathBuf,
    attachment_path: PathBuf,
    config: AttachmentFile,
    foreground: Option<Child>,
    detached_pid: Option<u32>,
}

impl Fixture {
    fn new(long_path: bool) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("apr-service-tests-")
            .tempdir()
            .unwrap();
        let base = if long_path {
            directory
                .path()
                .join("a deliberately long workspace name with spaces".repeat(3))
        } else {
            directory.path().join("fixture")
        };
        let state = base.join("state");
        let project = base.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let attachment_path = state.join("attachments/test.json");
        let mut attach = base_command(&state);
        attach.arg("project").arg("attach").arg(&project);
        let registered = success(attach);
        let mut assignment = base_command(&state);
        assignment
            .arg("assignment")
            .arg("create")
            .arg("--project")
            .arg(registered["project_id"].as_str().unwrap())
            .arg("--name")
            .arg("Service test");
        let assignment = success(assignment);
        let mut attachment = base_command(&state);
        attachment
            .arg("attachment")
            .arg("create")
            .arg("--assignment")
            .arg(assignment["assignment_id"].as_str().unwrap())
            .arg("--mode")
            .arg("author")
            .arg("--output-file")
            .arg(&attachment_path);
        let metadata = success(attachment);
        assert!(
            metadata.get("token").is_none(),
            "setup printed the attachment secret"
        );
        let config: AttachmentFile =
            serde_json::from_slice(&std::fs::read(&attachment_path).unwrap()).unwrap();
        assert!(
            !serde_json::to_string(&metadata)
                .unwrap()
                .contains(&config.token),
            "setup printed the attachment secret"
        );
        Self {
            directory,
            state,
            project,
            attachment_path,
            config,
            foreground: None,
            detached_pid: None,
        }
    }

    fn client(&self) -> ServiceClient {
        ServiceClient::with_executable(self.config.clone(), Path::new(env!("CARGO_BIN_EXE_apr")))
    }

    fn remember_detached(&mut self, info: &Value) {
        self.detached_pid = Some(u32::try_from(info["process_id"].as_u64().unwrap()).unwrap());
    }

    fn start_foreground(&mut self, gate: Option<&Path>) -> Value {
        assert!(self.foreground.is_none());
        let log_path = self.directory.path().join("service.stderr");
        let mut command = base_command(&self.state);
        command
            .arg("service")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log_path).unwrap());
        if let Some(gate) = gate {
            command.env("APR_TEST_WORKER_GATE", gate);
        }
        self.foreground = Some(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if UnixStream::connect(socket_path(&self.state).unwrap()).is_ok() {
                let (_, response) = raw_call(
                    &self.config,
                    "0.1",
                    "service.info",
                    json!({}),
                    DEFAULT_BUDGET,
                );
                if response.get("result").is_some() {
                    let info = response["result"].clone();
                    assert_eq!(info["process_id"], self.foreground.as_ref().unwrap().id());
                    return info;
                }
            }
            let status = self.foreground.as_mut().unwrap().try_wait().unwrap();
            assert!(
                status.is_none(),
                "service exited before readiness: {}",
                std::fs::read_to_string(&log_path).unwrap()
            );
            assert!(Instant::now() < deadline, "service did not become ready");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stop_foreground(&mut self, graceful: bool) {
        let child = self.foreground.as_mut().unwrap();
        if graceful {
            assert!(
                Command::new("/bin/kill")
                    .args(["-TERM", &child.id().to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
        } else {
            child.kill().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                if graceful {
                    assert!(status.success());
                }
                self.foreground = None;
                break;
            }
            assert!(Instant::now() < deadline, "owned service did not stop");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn register(&self, process: Value) -> String {
        std::fs::write(
            self.project.join("process.json"),
            serde_json::to_vec(&process).unwrap(),
        )
        .unwrap();
        self.client()
            .call(
                "manage",
                json!({"action":"process.register","path":"process.json"}),
                DEFAULT_BUDGET,
            )
            .unwrap()["process_ref"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn run(&self, process_ref: &str, input: Value) -> Value {
        self.client().call("run", json!({"mode":"start","process_ref":process_ref,"input":input,"request_key":"service-test"}), DEFAULT_BUDGET).unwrap()
    }

    fn expect_service_start_failure(&mut self) {
        let mut command = base_command(&self.state);
        command
            .arg("service")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        self.foreground = Some(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.foreground.as_mut().unwrap().try_wait().unwrap() {
                assert!(!status.success(), "unsafe endpoint was accepted");
                self.foreground = None;
                return;
            }
            assert!(
                Instant::now() < deadline,
                "service started despite an unsafe endpoint"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.foreground.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let expected = format!(
            "{} --state-dir {} service",
            env!("CARGO_BIN_EXE_apr"),
            self.config.state_dir.display()
        );
        let mut pids = self.detached_pid.take().into_iter().collect::<Vec<_>>();
        // A failed startup assertion may happen before service.info returns a PID.
        // Recover only processes matching this exact executable and unique fixture
        // state, never a broad process-name match.
        if let Ok(output) = Command::new("/bin/ps")
            .args(["-axo", "pid=,command="])
            .output()
        {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((pid, command)) = line.trim().split_once(char::is_whitespace)
                    && command.trim() == expected
                    && let Ok(pid) = pid.parse::<u32>()
                {
                    pids.push(pid);
                }
            }
        }
        pids.sort_unstable();
        pids.dedup();
        for pid in pids {
            if let Ok(output) = Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-ww", "-o", "command="])
                .output()
                && output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == expected
            {
                let _ = Command::new("/bin/kill")
                    .args(["-TERM", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while std::time::Instant::now() < deadline {
                    let alive = Command::new("/bin/ps")
                        .args(["-p", &pid.to_string(), "-o", "command="])
                        .output()
                        .is_ok_and(|output| {
                            output.status.success()
                                && String::from_utf8_lossy(&output.stdout).trim() == expected
                        });
                    if !alive {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
    }
}

fn base_command(state: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apr"));
    command
        .arg("--state-dir")
        .arg(state)
        .env_remove("APR_TEST_CRASH_AT")
        .env_remove("APR_TEST_WORKER_GATE");
    command
}

fn success(mut command: Command) -> Value {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "CLI {command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn raw_call(
    config: &AttachmentFile,
    protocol: &str,
    action: &str,
    args: Value,
    budget: usize,
) -> (Vec<u8>, Value) {
    let mut stream = UnixStream::connect(socket_path(&config.state_dir).unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = json!({"protocol_version":protocol,"request_id":"service-wire-test","attachment_token":config.token,
        "action":action,"args":args,"response_budget":budget});
    let mut request = serde_json::to_vec(&request).unwrap();
    request.push(b'\n');
    stream.write_all(&request).unwrap();
    let mut response = Vec::new();
    stream
        .take((DETAIL_BUDGET + 1) as u64)
        .read_to_end(&mut response)
        .unwrap();
    assert_eq!(response.last(), Some(&b'\n'));
    let value: Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(value["protocol_version"], "0.1");
    assert_eq!(value["request_id"], "service-wire-test");
    (response, value)
}

fn wait_for_state(client: &ServiceClient, run_id: &str, state: &str) -> Value {
    wait_for_state_with_timeout(client, run_id, state, Duration::from_secs(5))
}

fn wait_for_state_with_timeout(
    client: &ServiceClient,
    run_id: &str,
    state: &str,
    timeout: Duration,
) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let run = client
            .call(
                "inspect",
                json!({"view":"run","run_id":run_id}),
                DETAIL_BUDGET,
            )
            .unwrap();
        if run["state"] == state {
            return run;
        }
        assert!(
            Instant::now() < deadline,
            "expected {state}, observed {}",
            run["state"]
        );
        assert!(
            !matches!(
                run["state"].as_str(),
                Some("failed" | "blocked" | "cancelled")
            ),
            "run stopped: {run}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn billing_inspection_uses_the_service_read_snapshot() {
    let mut fixture = Fixture::new(false);
    let python = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim())
        .canonicalize()
        .unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mcp/server.py")
        .canonicalize()
        .unwrap();
    let input_schema = json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false});
    let tool_schema =
        json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}});
    let connection = serde_json::from_value(json!({
        "label":"Synthetic service billing fixture","endpoint":"stdio:local","method":"POST",
        "auth":{"kind":"none"},"input_schema":input_schema,"output_schema":{"type":"object"},
        "timeout_ms":2000,"max_response_bytes":16384,
        "mcp":{"protocol":"2026-07-28","transport":{"kind":"stdio","executable":python,
            "sha256":apr_connectors::mcp::pin_executable(&python).unwrap(),
            "args":["-I","-u",script,"ok",fixture.project.join("billing-calls")]},
            "tool":{"name":"normalize","input_schema":input_schema,"output_schema":tool_schema}},
        "metering":{"provider":"fixture","revision":"billing-v1","currency":"USD","meters":[
            {"name":"requested","unit":"items","add":["request:/value"],"rate":{"amount":"0.01","per_units":1}},
            {"name":"calls","unit":"requests","add":["response:/_meta/usage/requests"],"rate":{"amount":"0.10","per_units":1}}
        ]}
    }))
    .unwrap();
    let mut runtime = apr_runtime::Runtime::open(&fixture.state).unwrap();
    runtime
        .configure_connection(&fixture.config.assignment_id, "billing", connection, None)
        .unwrap();
    drop(runtime);
    fixture.start_foreground(None);
    let client = fixture.client();
    let args = json!({"view":"billing","starts_at_ms":0,"ends_at_ms":i64::MAX});
    let empty = client.call("inspect", args.clone(), DETAIL_BUDGET).unwrap();
    assert_eq!(empty["calls"], 0);
    assert_eq!(empty["groups"], json!([]));

    let mut process: Value =
        serde_json::from_str(include_str!("../../../examples/http-json/process.json")).unwrap();
    process["capabilities"][0]["name"] = json!("billing");
    process["steps"][0]["operation"] = json!("mcp.call");
    process["steps"][0]["connection"] = json!("billing");
    process["steps"][0]["inputs"] = json!({"value":{"input":"/value"}});
    let version = fixture.register(process);
    for number in 0..3 {
        let run = client.call("run",json!({"mode":"start","process_ref":version,"input":{"value":3},"request_key":format!("billed-{number}")}),DEFAULT_BUDGET).unwrap();
        wait_for_state(&client, run["run_id"].as_str().unwrap(), "succeeded");
    }
    let billed = client.call("inspect", args.clone(), DETAIL_BUDGET).unwrap();
    assert_eq!(billed["scope"], "whole_period");
    assert_eq!(billed["calls"], 3);
    assert_eq!(billed["groups"][0]["estimated_nanos"], "390000000");
    assert_eq!(billed["groups"][0]["unknown_or_partial"], 0);
    assert_eq!(client.call("inspect", args, DETAIL_BUDGET).unwrap(), billed);
    fixture.stop_foreground(true);
    let runtime = apr_runtime::Runtime::open(&fixture.state).unwrap();
    assert_eq!(
        runtime
            .billing_period(&fixture.config.assignment_id, 0, i64::MAX)
            .unwrap(),
        billed
    );
}

fn review_process() -> Value {
    json!({"schema_version":"0.1","name":"service.review","description":"Synthetic service review",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"agent","capability":"review"}],
        "limits":{"max_step_ms":5000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":1},
        "steps":[{"id":"review","kind":"agent","needs":[],"effect":"agent","retry":{"max_attempts":1},"capability":"review",
            "instructions":"Review the supplied record.","inputs":{"record":{"input":""}},
            "result_schema":{"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false}}],
        "output":{"approved":{"step":"review","pointer":"/approved"}}})
}

fn pure_process() -> Value {
    json!({"schema_version":"0.1","name":"service.snapshot","description":"Synthetic background snapshot",
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
        "capabilities":[{"kind":"builtin"},{"kind":"artifacts"}],
        "limits":{"max_step_ms":5000,"max_artifact_bytes":1048576,"max_provider_calls":0,"max_agent_requests":0},
        "steps":[{"id":"snapshot","kind":"builtin","operation":"json.write","needs":[],"effect":"pure","retry":{"max_attempts":2},
            "inputs":{"value":{"input":""}}}],
        "output":{"artifact":{"step":"snapshot","pointer":"/artifact"}}})
}

fn submission(waiting: &Value) -> Value {
    let request = &waiting["request"];
    json!({"protocol_version":"0.1","action":"submit","run_id":waiting["run_id"],
        "request_id":request["request_id"],"step_instance_id":request["step_instance_id"],"generation":request["generation"],
        "continuation_token":request["continuation_token"],"submission_key":"service-result",
        "output":{"approved":true},"provenance":{"source":"manual","client":"service-test"}})
}

// Run startup in a disposable launcher so a blocked Command::spawn cannot trap
// the test harness in a thread join. The parent can stop exactly its own group.
struct ServiceLauncher {
    child: Child,
    result_path: PathBuf,
}

impl ServiceLauncher {
    fn start(fixture: &Fixture, test: &str) -> Self {
        let result_path = fixture.directory.path().join("launcher-result.json");
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env("APR_TEST_SERVICE_ATTACHMENT", &fixture.attachment_path)
            .env("APR_TEST_SERVICE_RESULT", &result_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()
            .unwrap();
        Self { child, result_path }
    }

    fn result(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match std::fs::read(&self.result_path) {
                Ok(bytes) => return serde_json::from_slice(&bytes).unwrap(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("launcher result could not be read: {error}"),
            }
            if self.child.try_wait().unwrap().is_some() {
                return serde_json::from_slice(
                    &std::fs::read(&self.result_path)
                        .expect("launcher exited without reporting a service response"),
                )
                .unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "service startup did not return within the launcher deadline"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "owned launcher did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ServiceLauncher {
    fn drop(&mut self) {
        if self.child.try_wait().is_ok_and(|status| status.is_none()) {
            // SAFETY: this is the still-live child started in its own group
            // above. A negative ID targets only that owned launcher's group.
            unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
            let _ = self.child.wait();
        }
    }
}

fn launcher_attachment() -> Option<AttachmentFile> {
    std::env::var_os("APR_TEST_SERVICE_ATTACHMENT")
        .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
}

fn publish_launcher_result(result: &Value) {
    let path = PathBuf::from(std::env::var_os("APR_TEST_SERVICE_RESULT").unwrap());
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, serde_json::to_vec(result).unwrap()).unwrap();
    std::fs::rename(temporary, path).unwrap();
}

#[test]
fn concurrent_clients_start_one_authenticated_supervisor() {
    if let Some(config) = launcher_attachment() {
        let barrier = Arc::new(Barrier::new(8));
        let infos = std::thread::scope(|scope| {
            let jobs = (0..8)
                .map(|_| {
                    let config = config.clone();
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        let client = ServiceClient::with_executable(
                            config,
                            Path::new(env!("CARGO_BIN_EXE_apr")),
                        );
                        barrier.wait();
                        client.call("service.info", json!({}), DEFAULT_BUDGET)
                    })
                })
                .collect::<Vec<_>>();
            jobs.into_iter()
                .map(|job| job.join().unwrap().unwrap())
                .collect::<Vec<_>>()
        });
        publish_launcher_result(&json!(infos));
        return;
    }

    let mut fixture = Fixture::new(false);
    let mut launcher = ServiceLauncher::start(
        &fixture,
        "concurrent_clients_start_one_authenticated_supervisor",
    );
    let infos = launcher.result();
    let infos = infos.as_array().unwrap();
    fixture.remember_detached(&infos[0]);
    assert!(launcher.wait().success());
    assert_eq!(infos.len(), 8);
    assert!(!infos[0]["instance_id"].as_str().unwrap().is_empty());
    assert!(
        infos
            .iter()
            .all(|info| info["instance_id"] == infos[0]["instance_id"])
    );
    assert!(
        infos
            .iter()
            .all(|info| info["process_id"] == infos[0]["process_id"])
    );
    assert_eq!(
        fixture
            .client()
            .call("service.info", json!({}), DEFAULT_BUDGET)
            .unwrap(),
        infos[0]
    );
}

#[test]
fn supervisor_survives_launcher_process_group_teardown() {
    if let Some(config) = launcher_attachment() {
        let client = ServiceClient::with_executable(config, Path::new(env!("CARGO_BIN_EXE_apr")));
        publish_launcher_result(
            &client
                .call("service.info", json!({}), DEFAULT_BUDGET)
                .unwrap(),
        );
        // The parent terminates this group while the already-started supervisor
        // must remain usable. Bound the helper's lifetime if its parent exits.
        std::thread::sleep(Duration::from_secs(30));
        return;
    }

    let mut fixture = Fixture::new(false);
    let mut launcher = ServiceLauncher::start(
        &fixture,
        "supervisor_survives_launcher_process_group_teardown",
    );
    let before = launcher.result();
    fixture.remember_detached(&before);
    assert!(launcher.child.try_wait().unwrap().is_none());
    // SAFETY: only signal the still-live launcher group created by this test.
    assert_eq!(
        unsafe { libc::kill(-(launcher.child.id() as libc::pid_t), libc::SIGTERM) },
        0
    );
    assert_eq!(launcher.wait().signal(), Some(libc::SIGTERM));
    // Raw IPC deliberately cannot start a replacement supervisor.
    let (_, after) = raw_call(
        &fixture.config,
        "0.1",
        "service.info",
        json!({}),
        DEFAULT_BUDGET,
    );
    assert_eq!(after["result"], before);
}

#[test]
fn response_budgets_omit_whole_handoffs_and_explicit_detail_preserves_them() {
    let mut fixture = Fixture::new(false);
    fixture.start_foreground(None);
    let manifest = review_process();
    let process_ref = fixture.register(manifest.clone());
    // Multibyte text makes this a byte limit check rather than a character count.
    let input = json!({"text":"🦀".repeat(6000)});
    let queued = fixture.run(&process_ref, input.clone());
    let run_id = queued["run_id"].as_str().unwrap();
    let waiting = wait_for_state(&fixture.client(), run_id, "waiting_for_agent");
    let (compact_bytes, compact) = raw_call(
        &fixture.config,
        "0.1",
        "inspect",
        json!({"view":"run","run_id":run_id}),
        DEFAULT_BUDGET,
    );
    assert!(compact_bytes.len() <= DEFAULT_BUDGET);
    let compact = &compact["result"];
    for key in ["run_id", "state", "revision", "completed_steps"] {
        assert_eq!(compact[key], waiting[key]);
    }
    assert_eq!(compact["truncated"], true);
    assert!(
        compact["available_views"]
            .as_array()
            .is_some_and(|views| !views.is_empty())
    );
    assert!(
        compact.get("request").is_none(),
        "a partial handoff must not look actionable"
    );
    let token = waiting["request"]["continuation_token"].as_str().unwrap();
    assert!(!String::from_utf8(compact_bytes).unwrap().contains(token));

    let (detail_bytes, detailed) = raw_call(
        &fixture.config,
        "0.1",
        "inspect",
        json!({"view":"run","run_id":run_id}),
        DETAIL_BUDGET,
    );
    assert!(detail_bytes.len() > DEFAULT_BUDGET);
    assert!(detail_bytes.len() <= DETAIL_BUDGET);
    assert_eq!(detailed["result"], waiting);
    assert_eq!(detailed["result"]["request"]["input"]["record"], input);
    assert_eq!(
        detailed["result"]["request"]["result_schema"],
        manifest["steps"][0]["result_schema"]
    );
    assert_eq!(detailed["result"]["request"]["continuation_token"], token);
}

#[test]
fn compact_composed_runs_keep_family_identity_and_bounded_child_navigation() {
    let mut fixture = Fixture::new(false);
    fixture.start_foreground(None);
    let child_ref = fixture.register(review_process());
    let mut parent = review_process();
    parent["schema_version"] = json!("0.2");
    parent["name"] = json!("service.composed-review");
    parent["capabilities"] = json!([]);
    parent["limits"]["max_agent_requests"] = json!(40);
    parent["steps"] = Value::Array(
        (0..40)
            .map(|index| {
                json!({
                    "id":format!("review_{index:02}_{}", "detail".repeat(8)),
                    "kind":"call","needs":[],"effect":"control","retry":{"max_attempts":1},
                    "call":{"process_ref":child_ref},"inputs":{"record":{"input":""}}
                })
            })
            .collect(),
    );
    parent["output"] = json!({});
    let parent_ref = fixture.register(parent);
    let queued = fixture.run(&parent_ref, json!({"text":"🦀".repeat(6000)}));
    let parent_id = queued["run_id"].as_str().unwrap();
    // Forty durable child runs require more setup than the small fixtures,
    // especially on shared CI runners. This tests compaction, not throughput.
    let readiness_timeout = Duration::from_secs(20);
    let detailed = wait_for_state_with_timeout(
        &fixture.client(),
        parent_id,
        "waiting_for_child",
        readiness_timeout,
    );
    let steps = detailed["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 40);
    assert!(steps.iter().all(|step| step["child_run_id"].is_string()));

    for budget in [DEFAULT_BUDGET, 1024] {
        let (bytes, response) = raw_call(
            &fixture.config,
            "0.1",
            "inspect",
            json!({"view":"run","run_id":parent_id}),
            budget,
        );
        assert!(bytes.len() <= budget);
        let compact = &response["result"];
        assert_eq!(compact["run_id"], parent_id, "{response}");
        assert_eq!(compact["root_run_id"], parent_id);
        assert_eq!(compact["fixture"], detailed["fixture"]);
        assert_eq!(compact["child_count"], 40);
        assert_eq!(compact["children_truncated"], true);
        assert!(
            compact["available_views"]
                .as_array()
                .unwrap()
                .contains(&json!("inspect_runs"))
        );
        let children = compact["children"].as_array().unwrap();
        assert!(children.len() <= 10);
        if budget == DEFAULT_BUDGET {
            assert_eq!(children.len(), 10);
        }
        for child in children {
            assert!(
                steps
                    .iter()
                    .any(|step| step["id"] == child["parent_step_id"]
                        && step["child_run_id"] == child["child_run_id"])
            );
        }
    }

    let child_id = steps[0]["child_run_id"].as_str().unwrap();
    let child = wait_for_state_with_timeout(
        &fixture.client(),
        child_id,
        "waiting_for_agent",
        readiness_timeout,
    );
    let (_, compact) = raw_call(
        &fixture.config,
        "0.1",
        "inspect",
        json!({"view":"run","run_id":child_id}),
        DEFAULT_BUDGET,
    );
    for key in [
        "run_id",
        "root_run_id",
        "parent_run_id",
        "parent_step_id",
        "fixture",
    ] {
        assert_eq!(compact["result"][key], child[key], "lost {key}");
    }
    assert!(compact["result"].get("request").is_none());

    let (bytes, pending) = raw_call(
        &fixture.config,
        "0.1",
        "inspect",
        json!({"view":"pending","limit":20}),
        2048,
    );
    assert!(bytes.len() <= 2048);
    let pending_runs = pending["result"]["runs"].as_array().unwrap();
    assert!(!pending_runs.is_empty());
    for run in pending_runs {
        assert_eq!(run["root_run_id"], parent_id);
        assert_eq!(run["parent_run_id"], parent_id);
        assert_eq!(run["fixture"], detailed["fixture"]);
        assert!(run["parent_step_id"].is_string());
        assert!(run.get("request").is_none());
    }

    // The real paginated history is the recovery route for child references
    // omitted from a compact parent response, including under a tiny budget.
    let mut after = Value::Null;
    let mut children = std::collections::BTreeSet::new();
    for _ in 0..50 {
        let mut args = json!({"view":"runs","limit":20});
        if !after.is_null() {
            args["after"] = after.clone();
        }
        let (bytes, response) = raw_call(&fixture.config, "0.1", "inspect", args, 2048);
        assert!(bytes.len() <= 2048);
        let page = &response["result"];
        let runs = page["runs"]
            .as_array()
            .unwrap_or_else(|| panic!("{response}"));
        if runs.is_empty() {
            break;
        }
        for run in runs {
            assert_eq!(run["root_run_id"], parent_id);
            assert_eq!(run["fixture"], detailed["fixture"]);
            if run["run_id"] != parent_id {
                assert_eq!(run["parent_run_id"], parent_id);
                assert!(steps.iter().any(|step| step["id"] == run["parent_step_id"]
                    && step["child_run_id"] == run["run_id"]));
                children.insert(run["run_id"].as_str().unwrap().to_owned());
            }
        }
        assert_ne!(page["next_cursor"], after);
        after = page["next_cursor"].clone();
    }
    assert_eq!(children.len(), 40);
}

#[test]
fn long_state_paths_use_a_private_short_socket_and_preserve_the_lock_on_shutdown() {
    let mut fixture = Fixture::new(true);
    let info = fixture.start_foreground(None);
    assert!(fixture.state.as_os_str().len() > 104);
    let endpoint = socket_path(&fixture.state).unwrap();
    assert!(endpoint.as_os_str().len() < 104);
    let metadata = std::fs::symlink_metadata(&endpoint).unwrap();
    assert!(metadata.file_type().is_socket());
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert_eq!(
        std::fs::metadata(endpoint.parent().unwrap())
            .unwrap()
            .mode()
            & 0o777,
        0o700
    );
    let lock_path = endpoint.parent().unwrap().join("service.lock");
    let lock = std::fs::metadata(&lock_path).unwrap();
    assert_eq!(
        fixture
            .client()
            .call("service.info", json!({}), DEFAULT_BUDGET)
            .unwrap(),
        info
    );
    fixture.stop_foreground(true);
    assert!(!endpoint.exists());
    let after = std::fs::metadata(&lock_path).unwrap();
    assert_eq!((after.dev(), after.ino()), (lock.dev(), lock.ino()));
}

#[test]
fn attachment_files_must_be_private_owned_regular_files_inside_state() {
    let mut fixture = Fixture::new(false);
    let info = fixture.start_foreground(None);
    let client = ServiceClient::from_file(&fixture.attachment_path).unwrap();
    assert_eq!(client.mode(), "author");
    assert_eq!(
        client
            .call("service.info", json!({}), DEFAULT_BUDGET)
            .unwrap(),
        info
    );
    std::fs::set_permissions(
        &fixture.attachment_path,
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(ServiceClient::from_file(&fixture.attachment_path).is_err());
    std::fs::set_permissions(
        &fixture.attachment_path,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let linked = fixture.state.join("linked-attachment.json");
    symlink(&fixture.attachment_path, &linked).unwrap();
    assert!(ServiceClient::from_file(&linked).is_err());
    let copied = fixture.directory.path().join("outside-state.json");
    std::fs::copy(&fixture.attachment_path, &copied).unwrap();
    assert!(ServiceClient::from_file(&copied).is_err());
    let hardlinked = fixture.state.join("hardlinked-attachment.json");
    std::fs::hard_link(&fixture.attachment_path, &hardlinked).unwrap();
    assert!(ServiceClient::from_file(&hardlinked).is_err());
    assert!(ServiceClient::from_file(&fixture.attachment_path).is_err());
}

#[test]
fn protocol_and_authentication_errors_leave_the_same_service_available() {
    let mut fixture = Fixture::new(false);
    let info = fixture.start_foreground(None);
    let (_, incompatible) = raw_call(
        &fixture.config,
        "99.0",
        "service.info",
        json!({}),
        DEFAULT_BUDGET,
    );
    assert_eq!(incompatible["error"]["code"], "UNSUPPORTED_VERSION");
    assert!(incompatible.get("result").is_none());
    let mut unauthorized = fixture.config.clone();
    unauthorized.token = "0".repeat(64);
    let (_, denied) = raw_call(
        &unauthorized,
        "0.1",
        "service.info",
        json!({}),
        DEFAULT_BUDGET,
    );
    assert_eq!(denied["error"]["code"], "UNAUTHORIZED");
    assert!(denied.get("result").is_none());
    assert_eq!(
        fixture
            .client()
            .call("service.info", json!({}), DEFAULT_BUDGET)
            .unwrap(),
        info
    );

    let mut revoke = base_command(&fixture.state);
    revoke
        .arg("attachment")
        .arg("revoke")
        .arg(&fixture.config.attachment_id);
    success(revoke);
    let error = fixture
        .client()
        .call("service.info", json!({}), DEFAULT_BUDGET)
        .unwrap_err();
    assert_eq!(error.code(), "UNAUTHORIZED");
    assert!(
        fixture
            .foreground
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
}

#[test]
fn malformed_oversized_and_partial_frames_do_not_block_other_clients() {
    let mut fixture = Fixture::new(false);
    let info = fixture.start_foreground(None);
    let endpoint = socket_path(&fixture.state).unwrap();
    let mut partial = UnixStream::connect(&endpoint).unwrap();
    partial.write_all(b"{\"protocol_version\":").unwrap();
    let started = Instant::now();
    assert_eq!(
        fixture
            .client()
            .call("service.info", json!({}), DEFAULT_BUDGET)
            .unwrap(),
        info
    );
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "a partial frame serialized other clients behind its two-second deadline"
    );

    for (frame, code) in [
        (b"{broken}\n".to_vec(), "INVALID_INPUT"),
        (vec![b'x'; 1024 * 1024], "LIMIT_EXCEEDED"),
    ] {
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(&frame).unwrap();
        let mut bytes = Vec::new();
        stream
            .take((DEFAULT_BUDGET + 1) as u64)
            .read_to_end(&mut bytes)
            .unwrap();
        assert!(bytes.len() <= DEFAULT_BUDGET);
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["error"]["code"], code);
        assert!(response.get("result").is_none());
        assert_eq!(
            fixture
                .client()
                .call("service.info", json!({}), DEFAULT_BUDGET)
                .unwrap(),
            info
        );
    }
    drop(partial);
}

#[test]
fn stale_regular_files_and_symlinks_at_the_socket_path_are_preserved() {
    for use_symlink in [false, true] {
        let mut fixture = Fixture::new(false);
        let endpoint = socket_path(&fixture.state).unwrap();
        let sentinel = fixture.directory.path().join("sentinel");
        std::fs::write(&sentinel, b"keep this file").unwrap();
        if use_symlink {
            symlink(&sentinel, &endpoint).unwrap();
        } else {
            std::fs::write(&endpoint, b"keep this endpoint").unwrap();
        }
        fixture.expect_service_start_failure();
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep this file");
        if use_symlink {
            assert_eq!(std::fs::read_link(&endpoint).unwrap(), sentinel);
        } else {
            assert_eq!(std::fs::read(&endpoint).unwrap(), b"keep this endpoint");
        }
    }
}

#[test]
fn a_pending_handoff_survives_service_kill_and_resumes_once_after_restart() {
    let mut fixture = Fixture::new(false);
    let first = fixture.start_foreground(None);
    let process_ref = fixture.register(review_process());
    let queued = fixture.run(&process_ref, json!({"title":"Review after restart"}));
    let id = queued["run_id"].as_str().unwrap();
    let waiting = wait_for_state(&fixture.client(), id, "waiting_for_agent");
    fixture.stop_foreground(false);
    let second = fixture.start_foreground(None);
    assert_ne!(first["instance_id"], second["instance_id"]);
    assert_eq!(
        wait_for_state(&fixture.client(), id, "waiting_for_agent"),
        waiting
    );
    let result = submission(&waiting);
    let accepted = fixture
        .client()
        .call("resume", result.clone(), DEFAULT_BUDGET)
        .unwrap();
    assert_eq!(accepted["receipt"]["status"], "accepted");
    let completed = wait_for_state(&fixture.client(), id, "succeeded");
    assert_eq!(completed["output"], json!({"approved":true}));
    assert_eq!(
        completed["steps"][0]["attempts"],
        waiting["steps"][0]["attempts"]
    );
    let replay = fixture
        .client()
        .call("resume", result, DEFAULT_BUDGET)
        .unwrap();
    assert_eq!(replay["receipt"], accepted["receipt"]);
    let events = fixture
        .client()
        .call(
            "inspect",
            json!({"view":"events","run_id":id,"limit":100}),
            DETAIL_BUDGET,
        )
        .unwrap();
    assert_eq!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == "handoff.accepted")
            .count(),
        1
    );
}

#[test]
#[cfg(feature = "fault-injection")]
fn accepted_work_continues_after_the_client_disconnects() {
    let mut fixture = Fixture::new(false);
    let gate = fixture.directory.path().join("worker-ready");
    fixture.start_foreground(Some(&gate));
    let process_ref = fixture.register(pure_process());
    let client = fixture.client();
    let queued = client.call("run", json!({"mode":"start","process_ref":process_ref,"input":{"message":"durable"},"request_key":"disconnected"}), DEFAULT_BUDGET).unwrap();
    assert_eq!(queued["state"], "queued");
    assert_eq!(queued["steps"][0]["attempts"], 0);
    drop(client);
    std::fs::write(&gate, b"release worker").unwrap();
    let completed = wait_for_state(
        &fixture.client(),
        queued["run_id"].as_str().unwrap(),
        "succeeded",
    );
    assert_eq!(completed["steps"][0]["attempts"], 1);
    let artifact = fixture
        .client()
        .call(
            "inspect",
            json!({"view":"artifact","reference":completed["output"]["artifact"]}),
            DEFAULT_BUDGET,
        )
        .unwrap();
    assert_eq!(artifact["text"], "{\"message\":\"durable\"}");
}

#[test]
#[cfg(feature = "fault-injection")]
fn a_new_supervisor_automatically_drives_durably_queued_work() {
    let mut fixture = Fixture::new(false);
    let gate = fixture.directory.path().join("worker-ready");
    let first = fixture.start_foreground(Some(&gate));
    let process_ref = fixture.register(pure_process());
    let queued = fixture.run(&process_ref, json!({"message":"restart"}));
    assert_eq!(queued["state"], "queued");
    assert_eq!(queued["steps"][0]["attempts"], 0);
    fixture.stop_foreground(false);
    let second = fixture.start_foreground(None);
    assert_ne!(second["instance_id"], first["instance_id"]);
    let completed = wait_for_state(
        &fixture.client(),
        queued["run_id"].as_str().unwrap(),
        "succeeded",
    );
    assert_eq!(completed["steps"][0]["attempts"], 1);
    let replay = fixture.run(&process_ref, json!({"message":"restart"}));
    assert_eq!(replay, completed);
}

#[test]
fn rejected_oversized_request_ids_are_not_echoed_outside_the_response_budget() {
    let mut fixture = Fixture::new(false);
    let info = fixture.start_foreground(None);
    let mut stream = UnixStream::connect(socket_path(&fixture.state).unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = json!({"protocol_version":"0.1","request_id":"x".repeat(70_000),
        "attachment_token":fixture.config.token,"action":"service.info","args":{},"response_budget":DEFAULT_BUDGET});
    let mut bytes = serde_json::to_vec(&request).unwrap();
    bytes.push(b'\n');
    stream.write_all(&bytes).unwrap();
    let mut response = Vec::new();
    stream
        .take((DEFAULT_BUDGET + 1) as u64)
        .read_to_end(&mut response)
        .unwrap();
    assert!(
        response.len() <= DEFAULT_BUDGET,
        "a rejected correlation ID overflowed the response budget"
    );
    assert_eq!(response.last(), Some(&b'\n'));
    let response: Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(response["request_id"], "");
    assert_eq!(response["error"]["code"], "INVALID_INPUT");
    assert!(response.get("result").is_none());
    assert_eq!(
        fixture
            .client()
            .call("service.info", json!({}), DEFAULT_BUDGET)
            .unwrap(),
        info
    );
}

#[test]
fn an_escaped_artifact_preview_keeps_its_reference_when_detail_cannot_fit() {
    let mut fixture = Fixture::new(false);
    fixture.start_foreground(None);
    // NUL is valid UTF-8, but JSON requires six bytes for each encoded character.
    // The accepted 32 KiB text preview therefore exceeds even the 64 KiB response.
    std::fs::write(fixture.project.join("controls.txt"), vec![0_u8; 32 * 1024]).unwrap();
    let reference = fixture
        .client()
        .call(
            "manage",
            json!({"action":"artifact.import","path":"controls.txt","media_type":"text/plain"}),
            DEFAULT_BUDGET,
        )
        .unwrap();
    assert_eq!(reference["bytes"], 32 * 1024);
    let (bytes, response) = raw_call(
        &fixture.config,
        "0.1",
        "inspect",
        json!({"view":"artifact","reference":reference}),
        DETAIL_BUDGET,
    );
    assert!(bytes.len() <= DETAIL_BUDGET);
    assert!(response.get("error").is_none());
    let preview = &response["result"];
    assert_eq!(preview["artifact"], reference);
    assert_eq!(preview["truncated"], true);
    assert_eq!(preview["text_available"], false);
    assert!(preview.get("text").is_none());
    assert!(preview.get("available_views").is_none());
    assert!(
        preview["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
}

#[test]
fn an_output_that_exceeds_maximum_detail_returns_a_terminal_size_error() {
    let mut fixture = Fixture::new(false);
    fixture.start_foreground(None);
    let mut process = pure_process();
    process["name"] = json!("service.large-output");
    process["output"] = json!({"payload":{"literal":"x".repeat(65_500)}});
    let process_ref = fixture.register(process);
    let queued = fixture.run(&process_ref, json!({}));
    let id = queued["run_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let compact = fixture
            .client()
            .call("inspect", json!({"view":"run","run_id":id}), DEFAULT_BUDGET)
            .unwrap();
        if compact["state"] == "succeeded" {
            assert_eq!(compact["truncated"], true);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "expected completed large output, observed {}",
            compact["state"]
        );
        assert!(!matches!(
            compact["state"].as_str(),
            Some("failed" | "blocked" | "cancelled")
        ));
        std::thread::sleep(Duration::from_millis(20));
    }
    let (bytes, response) = raw_call(
        &fixture.config,
        "0.1",
        "inspect",
        json!({"view":"run","run_id":id}),
        DETAIL_BUDGET,
    );
    assert!(bytes.len() <= DETAIL_BUDGET);
    assert_eq!(response["error"]["code"], "LIMIT_EXCEEDED");
    assert!(response.get("result").is_none());
    assert!(response.get("available_views").is_none());
}

#[test]
fn inspection_and_authenticated_cancel_remain_responsive_during_parallel_native_work() {
    assert_parallel_cancellation(false);
}

#[test]
fn completed_worker_refills_a_slot_before_a_slow_sibling_finishes() {
    use sha2::{Digest, Sha256};
    let python = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
    let mut fixture = Fixture::new(false);
    let source = b"import json,sys,time\nr=json.load(sys.stdin)['input']\nstart=time.monotonic_ns()\nopen(r['marker'],'w').write('started')\ntime.sleep(r['delay'])\nprint(json.dumps({'protocol_version':'0.1','output':{'id':r['id'],'start':start,'end':time.monotonic_ns()}}))\n";
    std::fs::write(fixture.project.join("main.py"), source).unwrap();
    let mut runtime = apr_runtime::Runtime::open(&fixture.state).unwrap();
    runtime
        .grant_python(&fixture.config.assignment_id, &python)
        .unwrap();
    drop(runtime);
    fixture.start_foreground(None);
    let mut process: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(source)));
    process["input_schema"] = json!({"type":"object"});
    process["steps"][0]["inputs"] =
        json!({"id":{"item":"/id"},"delay":{"item":"/delay"},"marker":{"item":"/marker"}});
    process["steps"][0]["foreach"] =
        json!({"items":{"input":"/jobs"},"key":"/id","max_items":8,"concurrency":5});
    process["steps"][0]["result_schema"] = json!({"type":"object","required":["id","start","end"]});
    process["limits"]["max_step_ms"] = json!(5000);
    let version = fixture.register(process);
    let markers: Vec<_> = (0..5)
        .map(|index| fixture.project.join(format!("refill-{index}")))
        .collect();
    let run = fixture.run(
        &version,
        json!({"jobs":(0..5).map(|index| json!({
        "id":format!("job-{index}"),
        "delay":if index == 0 {1.5} else {0.15},
        "marker":markers[index]
    })).collect::<Vec<_>>()}),
    );
    let run_id = run["run_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while markers[..4].iter().any(|marker| !marker.exists()) {
        assert!(Instant::now() < deadline, "initial pool did not fill");
        std::thread::sleep(Duration::from_millis(10));
    }
    // A separate owner runtime may advance the same saved run while the
    // service's claim leases are held. It must not recover those live effects.
    let mut outside = apr_runtime::Runtime::open(&fixture.state).unwrap();
    let observed = outside.advance(run_id).unwrap();
    assert_ne!(observed.state, "needs_reconciliation");
    drop(outside);
    // The writer remains available for a new start while external work runs.
    let pure = fixture.register(pure_process());
    let started = Instant::now();
    let control = fixture.client().call("run", json!({"mode":"start","process_ref":pure,"input":{"value":7},"request_key":"refill-control"}), DEFAULT_BUDGET).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    wait_for_state(
        &fixture.client(),
        control["run_id"].as_str().unwrap(),
        "succeeded",
    );
    let result = wait_for_state_with_timeout(
        &fixture.client(),
        run_id,
        "succeeded",
        Duration::from_secs(10),
    );
    let items = result["output"]["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert!(
        items[4]["output"]["start"].as_u64().unwrap() < items[0]["output"]["end"].as_u64().unwrap(),
        "a free slot must refill before the slow sibling finishes: {items:?}"
    );
    fixture.stop_foreground(true);
}

#[test]
fn service_reaches_32_and_64_workers_and_drains_after_policy_decrease() {
    let _capacity_guard = NATIVE_CAPACITY_FIXTURE.lock().unwrap();
    use sha2::{Digest, Sha256};
    let python = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
    let mut fixture = Fixture::new(false);
    let gate = fixture.project.join("release-workers");
    let source = b"import json,os,sys,time\nr=json.load(sys.stdin)['input']\nopen(r['marker'],'w').write('started')\nwhile not os.path.exists(r['gate']): time.sleep(0.02)\nprint(json.dumps({'protocol_version':'0.1','output':{'id':r['id']}}))\n";
    std::fs::write(fixture.project.join("main.py"), source).unwrap();
    let mut runtime = apr_runtime::Runtime::open(&fixture.state).unwrap();
    runtime
        .grant_python(&fixture.config.assignment_id, &python)
        .unwrap();
    let mut policy = runtime.worker_policy().unwrap();
    policy.max_workers = 32;
    policy.max_local_workers = 32;
    policy = runtime
        .set_worker_policy(policy.clone(), policy.revision)
        .unwrap();
    drop(runtime);
    fixture.start_foreground(None);
    let mut process: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(source)));
    process["input_schema"] = json!({"type":"object"});
    process["steps"][0]["inputs"] =
        json!({"id":{"input":"/id"},"marker":{"input":"/marker"},"gate":{"input":"/gate"}});
    process["steps"][0]["result_schema"] = json!({"type":"object","required":["id"]});
    process["limits"]["max_step_ms"] = json!(60000);
    let version = fixture.register(process);
    let markers: Vec<_> = (0..65)
        .map(|index| fixture.project.join(format!("active-{index}")))
        .collect();
    let mut run_ids = Vec::new();
    let start = |index: usize| {
        fixture.client().call("run", json!({"mode":"start","process_ref":version,
        "input":{"id":index,"marker":markers[index],"gate":gate},"request_key":format!("active-{index}")}), DEFAULT_BUDGET).unwrap()
    };
    for index in 0..33 {
        run_ids.push(start(index)["run_id"].as_str().unwrap().to_owned());
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while markers[..32].iter().any(|marker| !marker.exists()) {
        assert!(Instant::now() < deadline, "32 workers did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !markers[32].exists(),
        "the 33rd worker exceeded the initial cap"
    );
    let mut owner = apr_runtime::Runtime::open(&fixture.state).unwrap();
    assert_eq!(owner.worker_status().unwrap()["running"], 32);
    policy.max_workers = 64;
    policy.max_local_workers = 64;
    policy = owner
        .set_worker_policy(policy.clone(), policy.revision)
        .unwrap();
    for index in 33..64 {
        run_ids.push(start(index)["run_id"].as_str().unwrap().to_owned());
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while markers[..64].iter().any(|marker| !marker.exists()) {
        assert!(Instant::now() < deadline, "64 workers did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(owner.worker_status().unwrap()["running"], 64);
    // A second process may author an independent root, but direct advance
    // cannot dispatch around the supervisor's full pool.
    let extra = owner
        .scoped_call(
            &fixture.config.token,
            "run",
            json!({"mode":"start","process_ref":version,
        "input":{"id":64,"marker":markers[64],"gate":gate},"request_key":"direct-extra"}),
        )
        .unwrap();
    run_ids.push(extra["run_id"].as_str().unwrap().to_owned());
    owner.advance(&run_ids[64]).unwrap();
    assert!(!markers[64].exists());
    assert_eq!(owner.worker_status().unwrap()["running"], 64);
    policy.max_workers = 1;
    policy.max_local_workers = 1;
    owner
        .set_worker_policy(policy.clone(), policy.revision)
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !markers[64].exists(),
        "lowering the cap must drain existing workers before another claim"
    );
    std::fs::write(&gate, b"release").unwrap();
    wait_for_state_with_timeout(
        &fixture.client(),
        &run_ids[64],
        "succeeded",
        Duration::from_secs(30),
    );
    let db = rusqlite::Connection::open(fixture.state.join("state.sqlite3")).unwrap();
    let finished: u64 = db
        .query_row(
            "SELECT count(*) FROM effect_intents WHERE status='succeeded'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(finished, 65);
    drop(owner);
    fixture.stop_foreground(true);
}

#[test]
fn direct_owner_closure_signals_a_service_owned_native_attempt() {
    use sha2::{Digest, Sha256};
    let python = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
    let mut fixture = Fixture::new(false);
    let marker = fixture.project.join("owner-close-started");
    let source = b"import json,sys,time\nr=json.load(sys.stdin)['input']\nopen(r['marker'],'w').write('started')\ntime.sleep(30)\nprint(json.dumps({'protocol_version':'0.1','output':{}}))\n";
    std::fs::write(fixture.project.join("main.py"), source).unwrap();
    let mut owner = apr_runtime::Runtime::open(&fixture.state).unwrap();
    owner
        .grant_python(&fixture.config.assignment_id, &python)
        .unwrap();
    drop(owner);
    fixture.start_foreground(None);
    let mut process: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(source)));
    process["input_schema"] = json!({"type":"object"});
    process["steps"][0]["inputs"] = json!({"marker":{"input":"/marker"}});
    process["steps"][0]["result_schema"] = json!({"type":"object"});
    process["limits"]["max_step_ms"] = json!(60000);
    let version = fixture.register(process);
    let run = fixture.run(&version, json!({"marker":marker}));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "native attempt did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut owner = apr_runtime::Runtime::open(&fixture.state).unwrap();
    owner
        .close_assignment(&fixture.config.assignment_id)
        .unwrap();
    assert_eq!(
        owner
            .inspect(run["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "cancelled"
    );
    let db = rusqlite::Connection::open(fixture.state.join("state.sqlite3")).unwrap();
    let intent: String = db
        .query_row("SELECT id FROM effect_intents", [], |row| row.get(0))
        .unwrap();
    let claim = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.state.join("execution-claims").join(intent))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let acquired = unsafe { libc::flock(claim.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if acquired == 0 {
            unsafe { libc::flock(claim.as_raw_fd(), libc::LOCK_UN) };
            break;
        }
        assert!(
            Instant::now() < deadline,
            "service worker ignored durable owner cancellation"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        db.query_row("SELECT status FROM effect_intents", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "uncertain"
    );
    fixture.stop_foreground(true);
}

#[test]
fn authenticated_cancel_interrupts_parallel_workers_in_child_recipes() {
    assert_parallel_cancellation(true);
}

fn assert_parallel_cancellation(nested: bool) {
    let _capacity_guard = NATIVE_CAPACITY_FIXTURE.lock().unwrap();
    use sha2::{Digest, Sha256};
    let python = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
    let mut fixture = Fixture::new(false);
    let markers: Vec<_> = (0..4)
        .map(|i| fixture.project.join(format!("started-{i}")))
        .collect();
    let code=b"import json,sys,time\nr=json.load(sys.stdin)\nopen(r['input']['marker'],'w').write('started')\ntime.sleep(30)\nprint(json.dumps({'protocol_version':'0.1','output':{}}))\n";
    let mut process: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(code)));
    process["input_schema"] = json!({"type":"object"});
    process["steps"][0]["inputs"] = json!({"marker":{"item":"/marker"}});
    process["steps"][0]["foreach"] =
        json!({"items":{"input":"/workers"},"key":"/id","max_items":4,"concurrency":4});
    process["steps"][0]["result_schema"] = json!({"type":"object"});
    process["limits"]["max_step_ms"] = json!(60000);
    std::fs::write(fixture.project.join("main.py"), code).unwrap();
    // The owner grants trusted execution before the service starts.
    let mut runtime = apr_runtime::Runtime::open(&fixture.state).unwrap();
    let db = rusqlite::Connection::open(fixture.state.join("state.sqlite3")).unwrap();
    let assignment: String = db
        .query_row("SELECT id FROM assignments", [], |r| r.get(0))
        .unwrap();
    drop(db);
    runtime.grant_python(&assignment, &python).unwrap();
    drop(runtime);
    fixture.start_foreground(None);
    if nested {
        let mut child = process.clone();
        child["steps"][0].as_object_mut().unwrap().remove("foreach");
        child["steps"][0]["inputs"] = json!({"marker":{"input":"/marker"}});
        let child_ref = fixture.register(child);
        process["schema_version"] = json!("0.2");
        process.as_object_mut().unwrap().remove("files");
        process["capabilities"] = json!([]);
        process["steps"] = json!([{
            "id":"normalize","kind":"call","effect":"control","needs":[],
            "retry":{"max_attempts":1},
            "foreach":{"items":{"input":"/workers"},"key":"/id","max_items":4,"concurrency":4},
            "inputs":{"marker":{"item":"/marker"}},
            "call":{"process_ref":child_ref,"output_pointer":""}
        }]);
    }
    let version = fixture.register(process);
    let run = fixture.run(&version, json!({"workers":markers.iter().enumerate().map(|(id,marker)|json!({"id":id.to_string(),"marker":marker})).collect::<Vec<_>>()}));
    let deadline = Instant::now() + Duration::from_secs(5);
    while markers.iter().any(|marker| !marker.exists()) {
        if Instant::now() >= deadline {
            let states: Vec<_> = markers.iter().map(|marker| marker.exists()).collect();
            let run_state = fixture.client().call(
                "inspect",
                json!({"view":"run","run_id":run["run_id"]}),
                DETAIL_BUDGET,
            );
            let log = std::fs::read_to_string(fixture.directory.path().join("service.stderr"));
            panic!(
                "parallel workers did not all start: markers={states:?}, run={run_state:?}, service_log={log:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    let state = fixture
        .client()
        .call(
            "inspect",
            json!({"view":"run","run_id":run["run_id"]}),
            DETAIL_BUDGET,
        )
        .unwrap();
    assert_eq!(
        state["state"],
        if nested {
            "waiting_for_child"
        } else {
            "running"
        }
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    // A different token must not interrupt another assignment's process.
    let mut invalid = fixture.config.clone();
    invalid.token = "f".repeat(64);
    assert!(
        ServiceClient::with_executable(invalid, Path::new(env!("CARGO_BIN_EXE_apr")))
            .call(
                "resume",
                json!({"action":"cancel","run_id":run["run_id"]}),
                DEFAULT_BUDGET
            )
            .is_err()
    );
    let started = Instant::now();
    let cancelled = fixture
        .client()
        .call(
            "resume",
            json!({"action":"cancel","run_id":run["run_id"]}),
            DEFAULT_BUDGET,
        )
        .unwrap();
    assert_eq!(cancelled["state"], "cancelled");
    assert!(started.elapsed() < Duration::from_secs(3));
    if nested {
        let db = rusqlite::Connection::open(fixture.state.join("state.sqlite3")).unwrap();
        let counts: (u32, u32) = db
            .query_row(
                "SELECT count(*),sum(status='uncertain') FROM effect_intents",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(counts, (4, 4));
        let children: u32 = db
            .query_row(
                "SELECT count(*) FROM runs WHERE parent_run_id=?1 AND state='cancelled'",
                [run["run_id"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(children, 4);
    } else {
        let effects = fixture
            .client()
            .call(
                "inspect",
                json!({"view":"effects","run_id":run["run_id"]}),
                DETAIL_BUDGET,
            )
            .unwrap();
        let intents = effects["intents"].as_array().unwrap();
        assert_eq!(intents.len(), 4);
        assert!(intents.iter().all(|intent| intent["status"] == "uncertain"));
    }
    fixture.stop_foreground(true);
}

#[test]
#[cfg(feature = "fault-injection")]
fn a_failed_wave_commit_does_not_strand_another_runs_dependents() {
    use sha2::{Digest, Sha256};
    let python = Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
    let mut fixture = Fixture::new(false);
    let code = b"import json,sys,time\nr=json.load(sys.stdin)['input']\ntime.sleep(0.1)\nprint(json.dumps({'protocol_version':'0.1','output':r}))\n";
    std::fs::write(fixture.project.join("main.py"), code).unwrap();
    let mut process: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(code)));
    process["input_schema"] = json!({"type":"object","required":["message"]});
    process["steps"][0]["inputs"] = json!({"message":{"input":"/message"}});
    process["steps"][0]["result_schema"] = json!({"type":"object","required":["message"]});
    process["steps"].as_array_mut().unwrap().push(json!({
        "id":"finish","kind":"builtin","operation":"json.select","effect":"pure",
        "needs":["normalize"],"retry":{"max_attempts":1},
        "inputs":{"value":{"step":"normalize","pointer":"/message"},"pointer":{"literal":""}}
    }));
    process["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"builtin"}));
    process["output"] = json!({"message":{"step":"finish","pointer":"/value"}});
    let mut runtime = apr_runtime::Runtime::open(&fixture.state).unwrap();
    runtime
        .grant_python(&fixture.config.assignment_id, &python)
        .unwrap();
    drop(runtime);
    let gate = fixture.directory.path().join("worker-ready");
    fixture.start_foreground(Some(&gate));
    let version = fixture.register(process);
    let failed = fixture.client().call("run", json!({"mode":"start","process_ref":version,"input":{"message":"failed commit"},"request_key":"failing-wave"}), DEFAULT_BUDGET).unwrap();
    let healthy = fixture.client().call("run", json!({"mode":"start","process_ref":version,"input":{"message":"complete unchanged"},"request_key":"healthy-wave"}), DEFAULT_BUDGET).unwrap();
    let failed_id = failed["run_id"].as_str().unwrap();
    let db = rusqlite::Connection::open(fixture.state.join("state.sqlite3")).unwrap();
    db.execute_batch(&format!(
        "CREATE TRIGGER reject_fixture_finish BEFORE UPDATE OF state ON attempts
         WHEN NEW.run_id='{}' AND NEW.state='succeeded'
         BEGIN SELECT RAISE(FAIL, 'synthetic result commit failure'); END;",
        failed_id.replace('\'', "''")
    ))
    .unwrap();
    std::fs::write(&gate, b"release both admitted runs").unwrap();
    // Read-only inspection must not be the wake that rescues healthy work.
    let completed = wait_for_state(
        &fixture.client(),
        healthy["run_id"].as_str().unwrap(),
        "succeeded",
    );
    assert_eq!(completed["output"], json!({"message":"complete unchanged"}));
    wait_for_state(&fixture.client(), failed_id, "needs_reconciliation");
    let effects: Vec<String> = db
        .prepare("SELECT status FROM effect_intents ORDER BY status")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(effects, vec!["succeeded", "uncertain"]);
    let diagnostic =
        std::fs::read_to_string(fixture.directory.path().join("service.stderr")).unwrap();
    assert!(diagnostic.contains("APR background advance deferred"));
}
