use apr_runtime::Runtime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WORKER: &str = r#"import json,sys,time
r=json.load(sys.stdin)['input']
if r.get('marker'):
    with open(r['marker'],'a') as marker: marker.write('started\n')
start=time.monotonic_ns()
time.sleep(0.3)
if r.get('fail',False):
    raise RuntimeError('synthetic failure after dispatch')
print(json.dumps({'protocol_version':'0.1','output':{'id':r['id'],'start':start,'end':time.monotonic_ns()}}))
"#;

struct Fixture {
    temp: tempfile::TempDir,
    project: PathBuf,
    runtime: Runtime,
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
            .create_assignment(attached["project_id"].as_str().unwrap(), "parallel fixture")
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
            project,
            runtime,
            token,
            assignment,
        }
    }

    fn grant_python(&mut self) {
        let output = std::process::Command::new("python3")
            .args(["-I", "-c", "import sys; print(sys.executable)"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Python is required for this native check"
        );
        let path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        self.runtime.grant_python(&self.assignment, &path).unwrap();
        std::fs::write(self.project.join("main.py"), WORKER).unwrap();
    }

    fn register(&mut self, process: Value) -> String {
        std::fs::write(
            self.project.join("recipe.json"),
            serde_json::to_vec(&process).unwrap(),
        )
        .unwrap();
        self.runtime
            .scoped_call(
                &self.token,
                "manage",
                json!({"action":"process.register","path":"recipe.json"}),
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
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut cursor = None;
        loop {
            self.runtime.drive_pending_batch(&mut cursor).unwrap();
            let result = serde_json::to_value(self.runtime.inspect(run).unwrap()).unwrap();
            if !matches!(
                result["state"].as_str(),
                Some("queued" | "running" | "waiting_for_child")
            ) {
                return result;
            }
            assert!(Instant::now() < deadline, "run did not settle: {result}");
        }
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.temp.path().join("state/state.sqlite3")).unwrap()
    }
}

fn native() -> Value {
    let mut process: Value = serde_json::from_str(include_str!(
        "../../../examples/native-normalize/process.json"
    ))
    .unwrap();
    process["schema_version"] = json!("0.2");
    process["name"] = json!("parallel.native");
    process["files"][0]["sha256"] = json!(format!("{:x}", Sha256::digest(WORKER.as_bytes())));
    process["input_schema"] = json!({"type":"object"});
    process["output_schema"] = json!({"type":"object"});
    process["steps"][0]["id"] = json!("work");
    process["steps"][0]["inputs"] = json!({"id":{"input":"/id"},"fail":{"input":"/fail"}});
    process["steps"][0]["result_schema"] = json!({"type":"object","required":["id","start","end"]});
    process["output"] = json!({"result":{"step":"work","pointer":""}});
    process
}

fn foreach(mut process: Value, concurrency: u32) -> Value {
    process["steps"][0]["foreach"] =
        json!({"items":{"input":"/jobs"},"key":"/id","max_items":8,"concurrency":concurrency});
    process["steps"][0]["inputs"] = json!({"id":{"item":"/id"},"fail":{"item":"/fail"}});
    process
}

fn jobs(count: usize) -> Value {
    json!({"jobs":(0..count).map(|id|json!({"id":format!("job-{id}"),"fail":false})).collect::<Vec<_>>()})
}

fn peak(outputs: &[Value]) -> usize {
    let mut events = Vec::new();
    for output in outputs {
        let start = output["start"].as_u64().unwrap();
        let end = output["end"].as_u64().unwrap();
        assert!(end > start);
        events.push((start, 1_i32));
        events.push((end, -1_i32));
    }
    events.sort_unstable();
    let (mut active, mut maximum) = (0, 0);
    for (_, change) in events {
        active += change;
        maximum = maximum.max(active);
    }
    assert_eq!(active, 0);
    maximum as usize
}

#[test]
fn native_foreach_overlaps_with_global_and_item_bounds_without_reordering_or_replay() {
    for (concurrency, expected_peak) in [(8, 4), (2, 2)] {
        let mut f = Fixture::new();
        f.grant_python();
        let version = f.register(foreach(native(), concurrency));
        let input = jobs(6);
        let run = f.start(&version, input.clone(), "batch");
        // Published source, not this edited project file, must execute.
        std::fs::write(
            f.project.join("main.py"),
            "raise RuntimeError('edited source must not execute')",
        )
        .unwrap();
        let result = f.finish(run["run_id"].as_str().unwrap());
        assert_eq!(result["state"], "succeeded", "{result}");
        let items = result["output"]["result"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        for (id, item) in items.iter().enumerate() {
            assert_eq!(item["key"], format!("job-{id}"));
            assert_eq!(item["output"]["id"], format!("job-{id}"));
        }
        assert_eq!(
            peak(
                &items
                    .iter()
                    .map(|i| i["output"].clone())
                    .collect::<Vec<_>>()
            ),
            expected_peak
        );
        assert_eq!(f.start(&version, input, "batch")["run_id"], run["run_id"]);
        f.runtime.drive_pending_batch(&mut None).unwrap();
        let count: u64 = f
            .db()
            .query_row("SELECT count(*) FROM effect_intents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 6);
    }
}

#[test]
fn local_worker_policy_caps_native_dispatch_below_global_limit() {
    let mut f = Fixture::new();
    f.grant_python();
    let mut policy = f.runtime.worker_policy().unwrap();
    policy.max_local_workers = 2;
    f.runtime
        .set_worker_policy(policy.clone(), policy.revision)
        .unwrap();
    let version = f.register(foreach(native(), 8));
    let run = f.start(&version, jobs(6), "local-cap");
    let result = f.finish(run["run_id"].as_str().unwrap());
    assert_eq!(result["state"], "succeeded", "{result}");
    let outputs: Vec<Value> = result["output"]["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["output"].clone())
        .collect();
    assert_eq!(peak(&outputs), 2);
}

#[test]
fn pinned_child_recipes_execute_together_and_preserve_parent_foreach_limit() {
    let mut f = Fixture::new();
    f.grant_python();
    let child = f.register(native());
    let mut parent = native();
    parent["name"] = json!("parallel.parent");
    parent.as_object_mut().unwrap().remove("files");
    parent["capabilities"] = json!([]);
    parent["steps"] = json!([{"id":"work","kind":"call","effect":"control","needs":[],"retry":{"max_attempts":1},"inputs":{"id":{"item":"/id"},"fail":{"item":"/fail"}},"foreach":{"items":{"input":"/jobs"},"key":"/id","max_items":8,"concurrency":2},"call":{"process_ref":child,"output_pointer":"/result"}}]);
    let version = f.register(parent);
    let run = f.start(&version, jobs(4), "children");
    let result = f.finish(run["run_id"].as_str().unwrap());
    assert_eq!(result["state"], "succeeded", "{result}");
    let items = result["output"]["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(
        peak(
            &items
                .iter()
                .map(|i| i["output"].clone())
                .collect::<Vec<_>>()
        ),
        2
    );
    let child_count: u64 = f
        .db()
        .query_row("SELECT count(*) FROM child_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(child_count, 4);
}

#[test]
fn a_failed_worker_preserves_successful_siblings_and_never_retries_uncertainty() {
    let mut f = Fixture::new();
    f.grant_python();
    let version = f.register(foreach(native(), 4));
    let input = json!({"jobs":[{"id":"a-fails","fail":true},{"id":"b-completes","fail":false}]});
    let run = f.start(&version, input.clone(), "partial");
    let run_id = run["run_id"].as_str().unwrap();
    assert_eq!(f.finish(run_id)["state"], "needs_reconciliation");
    let db = f.db();
    let states: Vec<String> = db
        .prepare("SELECT status FROM effect_intents ORDER BY step_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        states.iter().filter(|s| s.as_str() == "uncertain").count(),
        1
    );
    assert_eq!(
        states.iter().filter(|s| s.as_str() == "succeeded").count(),
        1
    );
    let saved: String = db
        .query_row(
            "SELECT output FROM steps WHERE state='succeeded' AND active_attempt IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved).unwrap()["id"],
        "b-completes"
    );
    assert_eq!(f.start(&version, input, "partial")["run_id"], run_id);
    f.runtime.drive_pending_batch(&mut None).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM effect_intents", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        2
    );
}

#[test]
fn revoked_grant_blocks_later_waves_without_replaying_completed_items() {
    let mut f = Fixture::new();
    f.grant_python();
    let version = f.register(foreach(native(), 8));
    let run = f.start(&version, jobs(6), "revoke");
    f.runtime.drive_pending_batch(&mut None).unwrap();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM effect_intents WHERE status='succeeded'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        4
    );
    f.runtime
        .revoke_execution(&f.assignment, "python", "python")
        .unwrap();
    let result = f.finish(run["run_id"].as_str().unwrap());
    assert_eq!(result["state"], "blocked");
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM effect_intents", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        4
    );
}

#[test]
#[cfg(feature = "fault-injection")]
fn crash_after_wave_reservations_recovers_every_claim_without_dispatch() {
    const STATE_ENV: &str = "APR_PARALLEL_RECOVERY_TEST_STATE";
    if let Some(state) = std::env::var_os(STATE_ENV) {
        let mut runtime = Runtime::open(PathBuf::from(state)).unwrap();
        runtime.drive_pending_batch(&mut None).unwrap();
        panic!("expected injected process death after wave reservation");
    }
    let mut f = Fixture::new();
    f.grant_python();
    let mut process = foreach(native(), 4);
    process["steps"][0]["inputs"]["marker"] = json!({"item":"/marker"});
    let version = f.register(process);
    let markers: Vec<_> = (0..4)
        .map(|i| f.project.join(format!("dispatch-{i}")))
        .collect();
    let input = json!({"jobs":markers.iter().enumerate().map(|(id,path)|json!({"id":id.to_string(),"fail":false,"marker":path})).collect::<Vec<_>>()});
    let run = f.start(&version, input.clone(), "crash-wave");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_after_wave_reservations_recovers_every_claim_without_dispatch",
        ])
        .env(STATE_ENV, f.temp.path().join("state"))
        .env("APR_TEST_CRASH_AT", "external_wave_prepared")
        .status()
        .unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    let db = f.db();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM effect_intents WHERE status='dispatched'",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        4
    );
    assert_eq!(
        f.finish(run["run_id"].as_str().unwrap())["state"],
        "needs_reconciliation"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM effect_intents WHERE status='uncertain'",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        4
    );
    assert_eq!(
        f.start(&version, input, "crash-wave")["run_id"],
        run["run_id"]
    );
    f.runtime.drive_pending_batch(&mut None).unwrap();
    assert!(
        markers.iter().all(|path| !path.exists()),
        "recovery must not dispatch any reserved effect"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM effect_intents", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        4
    );
}

#[cfg(feature = "fault-injection")]
mod http {
    use super::*;
    use apr_connectors::{Auth, Connection};
    use base64::Engine;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn request(stream: &mut TcpStream) -> Value {
        // Accepted sockets can inherit the listener's nonblocking mode on macOS.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 2048];
        loop {
            let n = stream.read(&mut buffer).unwrap();
            assert!(n > 0, "request ended early");
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                }
            }
            assert!(bytes.len() < 8192);
        }
    }

    fn configure(
        f: &mut Fixture,
        count: usize,
        lose_reply: bool,
        input_schema: Value,
    ) -> std::thread::JoinHandle<Vec<Value>> {
        configure_with_auth(f, count, lose_reply, input_schema, false)
    }

    fn configure_with_auth(
        f: &mut Fixture,
        count: usize,
        lose_reply: bool,
        input_schema: Value,
        oauth: bool,
    ) -> std::thread::JoinHandle<Vec<Value>> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/fixture", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let started = Instant::now();
            let mut threads = Vec::new();
            while threads.len() < count {
                match listener.accept() {
                    Ok((mut stream,_)) => threads.push(std::thread::spawn(move|| {
                        let input = request(&mut stream);
                        let start = started.elapsed().as_nanos() as u64;
                        std::thread::sleep(Duration::from_millis(200));
                        let output = json!({"id":input["id"],"start":start,"end":started.elapsed().as_nanos() as u64,"usage":{"requests":1}});
                        if !lose_reply {
                            let body = serde_json::to_string(&output).unwrap();
                            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
                        }
                        output
                    })),
                    Err(error) if error.kind()==std::io::ErrorKind::WouldBlock => {
                        assert!(started.elapsed()<Duration::from_secs(8),"expected requests were not dispatched");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            }
            threads.into_iter().map(|t| t.join().unwrap()).collect()
        });
        let metering = serde_json::from_value(json!({"provider":"fixture","revision":"v1","currency":"USD","meters":[{"name":"requests","unit":"requests","add":["/usage/requests"],"rate":{"amount":"0.000001","per_units":1}}]})).unwrap();
        let (auth, secret) = if oauth {
            let config: apr_connectors::oauth::OAuthConfig = serde_json::from_value(json!({
                "issuer":endpoint,"authorization_endpoint":endpoint,"token_endpoint":endpoint,
                "client_id":"synthetic-fixture","resource":endpoint,"scopes":["read"],
                "require_issuer_response":false,"loopback_test":true
            }))
            .unwrap();
            let tokens = json!({"access":"synthetic-fixture-token","refresh":null,
                "expires":u64::MAX,"scope":["read"],
                "config_digest":apr_core::content_digest(&serde_json::to_value(&config).unwrap()).unwrap()});
            (
                Auth::Oauth {
                    credential_ref: "pending".into(),
                    config,
                },
                Some(
                    apr_connectors::Secret::new(
                        base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .encode(serde_json::to_vec(&tokens).unwrap())
                            .into_bytes(),
                    )
                    .unwrap(),
                ),
            )
        } else {
            (Auth::None, None)
        };
        f.runtime
            .configure_connection(
                &f.assignment,
                "records",
                Connection {
                    mcp: None,
                    metering: Some(metering),
                    label: "Synthetic parallel fixture".into(),
                    credential_label: None,
                    endpoint,
                    method: "POST".into(),
                    auth,
                    input_schema,
                    output_schema: json!({"type":"object"}),
                    timeout_ms: 2000,
                    max_response_bytes: 4096,
                    loopback_test: true,
                },
                secret,
            )
            .unwrap();
        server
    }

    fn process() -> Value {
        let mut p = native();
        p["name"] = json!("parallel.http");
        p.as_object_mut().unwrap().remove("files");
        p["capabilities"] = json!([{"kind":"connection","name":"records"}]);
        p["limits"]["max_provider_calls"] = json!(8);
        p["steps"] = json!([{"id":"work","kind":"connector","effect":"write","operation":"http.json","connection":"records","needs":[],"retry":{"max_attempts":1},"inputs":{"id":{"input":"/id"},"fail":{"input":"/fail"}},"result_schema":{"type":"object"}}]);
        p
    }

    #[test]
    fn jobs_sharing_a_valid_oauth_credential_overlap() {
        let mut f = Fixture::new();
        let server = configure_with_auth(&mut f, 4, false, json!({"type":"object"}), true);
        let version = f.register(foreach(process(), 4));
        let run = f.start(&version, jobs(4), "oauth-overlap");
        let result = f.finish(run["run_id"].as_str().unwrap());
        assert_eq!(result["state"], "succeeded", "{result}");
        assert_eq!(peak(&server.join().unwrap()), 4);
        let items = result["output"]["result"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 4);
        for (index, item) in items.iter().enumerate() {
            assert_eq!(item["key"], format!("job-{index}"));
            assert_eq!(item["output"]["id"], format!("job-{index}"));
        }
    }

    #[test]
    fn http_workers_overlap_and_reservations_cannot_overspend() {
        for cap in ["none", "calls", "money"] {
            let mut f = Fixture::new();
            let expected = if cap == "none" { 4 } else { 2 };
            let server = configure(&mut f, expected, false, json!({"type":"object"}));
            let mut p = foreach(process(), 4);
            if cap == "calls" {
                p["limits"]["max_provider_calls"] = json!(2);
            }
            if cap == "money" {
                f.runtime.set_billing_budget(&f.assignment,serde_json::from_value(json!({"currency":"USD","starts_at_ms":0,"ends_at_ms":i64::MAX,"limit_nanos":"2000","per_connection_max_nanos":{"records":"1000"}})).unwrap()).unwrap();
            }
            let version = f.register(p);
            let run = f.start(&version, jobs(4), "http-batch");
            let result = f.finish(run["run_id"].as_str().unwrap());
            assert_eq!(
                result["state"],
                match cap {
                    "calls" => "failed",
                    "money" => "blocked",
                    _ => "succeeded",
                },
                "{result}"
            );
            let outputs = server.join().unwrap();
            assert_eq!(peak(&outputs), expected);
            let db = f.db();
            assert_eq!(
                db.query_row("SELECT count(*) FROM effect_intents", [], |r| r
                    .get::<_, usize>(0))
                    .unwrap(),
                expected
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM billing_calls", [], |r| r
                    .get::<_, usize>(0))
                    .unwrap(),
                expected
            );
            assert_eq!(
                f.start(&version, jobs(4), "http-batch")["run_id"],
                run["run_id"]
            );
            f.runtime.drive_pending_batch(&mut None).unwrap();
            assert_eq!(
                db.query_row("SELECT count(*) FROM effect_intents", [], |r| r
                    .get::<_, usize>(0))
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn named_connection_policy_caps_provider_dispatch() {
        let mut f = Fixture::new();
        let server = configure(&mut f, 4, false, json!({"type":"object"}));
        let mut policy = f.runtime.worker_policy().unwrap();
        policy.per_connection.insert("records".into(), 2);
        f.runtime
            .set_worker_policy(policy.clone(), policy.revision)
            .unwrap();
        let version = f.register(foreach(process(), 4));
        let run = f.start(&version, jobs(4), "connection-cap");
        let result = f.finish(run["run_id"].as_str().unwrap());
        assert_eq!(result["state"], "succeeded", "{result}");
        assert_eq!(peak(&server.join().unwrap()), 2);
        assert_eq!(
            result["output"]["result"]["items"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn later_not_sent_result_does_not_hide_an_uncertain_sibling() {
        let mut f = Fixture::new();
        let server = configure(
            &mut f,
            1,
            true,
            json!({"type":"object","properties":{"id":{"const":"allowed"}},"required":["id"]}),
        );
        let version = f.register(foreach(process(), 4));
        let run = f.start(
            &version,
            json!({"jobs":[{"id":"allowed","fail":false},{"id":"refused","fail":false}]}),
            "mixed-http",
        );
        assert_eq!(
            f.finish(run["run_id"].as_str().unwrap())["state"],
            "needs_reconciliation"
        );
        assert_eq!(server.join().unwrap().len(), 1);
        let db = f.db();
        for status in ["uncertain", "not_sent"] {
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM effect_intents WHERE status=?1",
                    [status],
                    |r| r.get::<_, u32>(0)
                )
                .unwrap(),
                1
            );
        }
    }
}
