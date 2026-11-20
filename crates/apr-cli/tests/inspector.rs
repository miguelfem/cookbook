use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Inspector {
    _child: ChildGuard,
    _service: ChildGuard,
    authority: String,
    token: String,
    foreign_run: String,
    _directory: tempfile::TempDir,
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Inspector {
    fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        fs::create_dir(&project).unwrap();
        let state = directory.path().join("state");
        let mut runtime = apr_runtime::Runtime::open(&state).unwrap();
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../../examples/local-review/process.json"))
                .unwrap();
        let mut review = manifest["steps"][4].clone();
        review["needs"] = json!([]);
        review["inputs"] = json!({"note":{"literal":"<script>not executable content</script>"}});
        manifest["steps"] = json!([review]);
        manifest["input_schema"] = json!({"type":"object","additionalProperties":false});
        manifest["output_schema"] = json!({"type":"object","required":["approved"],"properties":{"approved":{"type":"boolean"}},"additionalProperties":false});
        manifest["output"] = json!({"approved":{"step":"review","pointer":"/approved"}});
        fs::write(
            project.join("review.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let process = apr_core::admit_json(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        let foreign_run = runtime
            .start(&process, json!({}), "foreign-owner-run")
            .unwrap()
            .run_id;
        let project = runtime.attach_project(&project).unwrap();
        let assignment = runtime
            .create_assignment(project["project_id"].as_str().unwrap(), "inspector")
            .unwrap();
        let attachment = runtime
            .create_attachment(assignment["assignment_id"].as_str().unwrap(), "author")
            .unwrap();
        let attachment_file = state.join("inspector.json");
        fs::write(&attachment_file, serde_json::to_vec(&attachment).unwrap()).unwrap();
        fs::set_permissions(&attachment_file, fs::Permissions::from_mode(0o600)).unwrap();
        drop(runtime);
        let service = ChildGuard(
            Command::new(env!("CARGO_BIN_EXE_apr"))
                .arg("--state-dir")
                .arg(&state)
                .arg("service")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while UnixStream::connect(apr_runtime::service::socket_path(&state).unwrap()).is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut child = ChildGuard(
            Command::new(env!("CARGO_BIN_EXE_apr"))
                .args(["inspector", "--attachment"])
                .arg(&attachment_file)
                .args(["--port", "0"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let output = child.0.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(output).read_line(&mut line);
            let _ = tx.send(line);
        });
        let line = rx.recv_timeout(Duration::from_secs(8)).unwrap();
        let startup: Value =
            serde_json::from_str(&line).expect("inspector must print its local URL");
        assert!(!line.contains(attachment["token"].as_str().unwrap()));
        let url = startup["url"].as_str().unwrap();
        let (base, fragment) = url.split_once('#').unwrap();
        let authority = base
            .strip_prefix("http://")
            .unwrap()
            .trim_end_matches('/')
            .to_owned();
        Self {
            _child: child,
            _service: service,
            authority,
            token: fragment.strip_prefix("session=").unwrap().into(),
            foreign_run,
            _directory: directory,
        }
    }
    fn request(&self, path: &str, method: &str, headers: &[(&str, &str)], body: &str) -> String {
        let mut connection = TcpStream::connect(&self.authority).unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        write!(
            connection,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n",
            self.authority,
            body.len()
        )
        .unwrap();
        for (name, value) in headers {
            write!(connection, "{name}: {value}\r\n").unwrap();
        }
        write!(connection, "\r\n{body}").unwrap();
        let mut response = String::new();
        connection.read_to_string(&mut response).unwrap();
        response
    }
    fn api(&self, action: &str, args: Value) -> Value {
        let origin = format!("http://{}", self.authority);
        let response = self.request(
            "/api",
            "POST",
            &[
                ("Content-Type", "application/json"),
                ("Origin", &origin),
                ("X-APR-Session", &self.token),
            ],
            &json!({"action":action,"args":args,"detail":true}).to_string(),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap()["result"]
            .clone()
    }
}

#[test]
fn inspector_serves_bundled_ui_and_requires_its_own_token_and_same_origin() {
    let inspector = Inspector::start();
    let page = inspector.request("/", "GET", &[], "");
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("Content-Security-Policy:"));
    assert!(!page.contains(&inspector.token));
    let body = json!({"action":"inspect","args":{"view":"runs","limit":10}}).to_string();
    let unauthenticated = inspector.request(
        "/api",
        "POST",
        &[("Content-Type", "application/json")],
        &body,
    );
    assert!(unauthenticated.starts_with("HTTP/1.1 403"));
    let wrong_origin = inspector.request(
        "/api",
        "POST",
        &[
            ("Content-Type", "application/json"),
            ("Origin", "https://example.com"),
            ("X-APR-Session", &inspector.token),
        ],
        &body,
    );
    assert!(wrong_origin.starts_with("HTTP/1.1 403"));
    let origin = format!("http://{}", inspector.authority);
    let forbidden_action = inspector.request(
        "/api",
        "POST",
        &[
            ("Content-Type", "application/json"),
            ("Origin", &origin),
            ("X-APR-Session", &inspector.token),
        ],
        &json!({"action":"attachment.create","args":{}}).to_string(),
    );
    assert!(forbidden_action.starts_with("HTTP/1.1 403"));
}

#[test]
fn inspector_manages_schedules_through_real_scoped_service() {
    let inspector = Inspector::start();
    let page = inspector.request("/", "GET", &[], "");
    assert!(page.contains("href=\"#schedules\" data-nav=\"schedules\">Schedules</a>"));
    assert!(page.contains("data-page=\"schedules\""));
    assert!(page.contains("id=\"schedule-create\""));
    let registered = inspector.api(
        "manage",
        json!({"action":"process.register","path":"review.json"}),
    );
    let created = inspector.api(
        "manage",
        json!({"action":"schedule.create","name":"UI review",
        "process_ref":registered["process_ref"],"input":{},"request_key":"ui-schedule",
        "trigger":{"kind":"interval","every_seconds":3600}}),
    );
    let schedule = created["schedule_id"].as_str().unwrap();
    let listed = inspector.api("inspect", json!({"view":"schedules"}));
    assert_eq!(listed["schedules"][0]["schedule_id"], schedule);
    assert_eq!(
        inspector.api(
            "manage",
            json!({"action":"schedule.pause","schedule_id":schedule})
        )["state"],
        "paused"
    );
    assert_eq!(
        inspector.api(
            "manage",
            json!({"action":"schedule.resume","schedule_id":schedule})
        )["state"],
        "active"
    );
    let detail = inspector.api("inspect", json!({"view":"schedule","schedule_id":schedule}));
    assert_eq!(detail["trigger"]["every_seconds"], 3600);
    assert_eq!(
        inspector.api(
            "manage",
            json!({"action":"schedule.delete","schedule_id":schedule})
        )["state"],
        "deleted"
    );
    assert!(
        inspector.api("inspect", json!({"view":"schedules"}))["schedules"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn inspector_controls_real_scoped_runs_and_cannot_read_another_scope() {
    let inspector = Inspector::start();
    assert!(
        inspector.api("inspect", json!({"view":"runs"}))["runs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let registered = inspector.api(
        "manage",
        json!({"action":"process.register","path":"review.json"}),
    );
    let catalog = inspector.api("discover", json!({"limit":10}));
    assert_eq!(
        catalog["processes"][0]["process_ref"],
        registered["process_ref"]
    );
    let started = inspector.api("run", json!({"process_ref":registered["process_ref"],"input":{},"mode":"start","request_key":"ui-run"}));
    let run = started["run_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let saved = inspector.api("inspect", json!({"view":"run","run_id":run}));
        if saved["state"] == "waiting_for_agent" {
            assert!(
                saved["request"]["input"]["note"]
                    .as_str()
                    .unwrap()
                    .contains("<script>")
            );
            break;
        }
        assert!(Instant::now() < deadline, "{saved}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        inspector.api("inspect", json!({"view":"runs"}))["runs"][0]["run_id"],
        run
    );
    let config = inspector.api("inspect", json!({"view":"config","run_id":run}));
    assert!(config.is_object());
    let events = inspector.api("inspect", json!({"view":"events","run_id":run,"limit":20}));
    assert!(events.to_string().contains("run.created"));
    inspector.api("resume", json!({"action":"pause","run_id":run}));
    inspector.api("resume", json!({"action":"continue","run_id":run}));
    let cancelled = inspector.api("resume", json!({"action":"cancel","run_id":run}));
    assert_eq!(cancelled["state"], "cancelled");
    let origin = format!("http://{}", inspector.authority);
    let forbidden = inspector.request(
        "/api",
        "POST",
        &[
            ("Content-Type", "application/json"),
            ("Origin", &origin),
            ("X-APR-Session", &inspector.token),
        ],
        &json!({"action":"inspect","args":{"view":"run","run_id":inspector.foreign_run}})
            .to_string(),
    );
    assert!(forbidden.starts_with("HTTP/1.1 403"));
}

#[test]
fn inspector_rejects_oversized_frames_duplicate_headers_and_slow_headers() {
    let inspector = Inspector::start();
    for wire in [
        format!(
            "POST /api HTTP/1.1\r\nHost: {}\r\nContent-Length: 65537\r\n\r\n",
            inspector.authority
        ),
        format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nHost: evil.example\r\n\r\n",
            inspector.authority
        ),
    ] {
        let mut stream = TcpStream::connect(&inspector.authority).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(wire.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 413") || response.starts_with("HTTP/1.1 400"));
    }
    let mut stream = TcpStream::connect(&inspector.authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(b"GET / HTTP/1.1\r\n").unwrap();
    let started = Instant::now();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 408"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn inspector_analytics_and_connections_use_real_service_without_expanding_access() {
    let inspector = Inspector::start();
    let html = inspector.request("/", "GET", &[], "");
    assert!(html.contains("href=\"#costs\" data-nav=\"costs\">Costs</a>"));
    assert!(html.contains("data-page=\"costs\""));
    assert!(html.contains("data-page=\"activity\""));
    assert!(html.contains("id=\"analytics\""));
    assert!(html.contains("id=\"connections\""));
    let view = inspector.api("inspect", json!({"view":"analytics","limit":2}));
    assert_eq!(view["view"], "analytics");
    assert_eq!(view["telemetry"], "local_only");
    assert_eq!(view["summary"]["external_attempts"], 0);
    assert_eq!(view["cost_page"]["scope"], "this_page_only");
    let origin = format!("http://{}", inspector.authority);
    let denied = inspector.request(
        "/api",
        "POST",
        &[
            ("Content-Type", "application/json"),
            ("Origin", &origin),
            ("X-APR-Session", &inspector.token),
        ],
        &json!({"action":"inspect","args":{"view":"analytics","run_id":inspector.foreign_run}})
            .to_string(),
    );
    assert!(denied.starts_with("HTTP/1.1 403"));
    let bindings = inspector.api("inspect", json!({"view":"bindings"}));
    assert!(bindings["bindings"].as_array().unwrap().is_empty());
}
