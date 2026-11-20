#![cfg(feature = "test-http")]

use apr_connectors::mcp::{McpBinding, Transport, negotiate_protocol, pin_executable};
use apr_connectors::{Auth, Connection, CredentialStore, DispatchError, Secret};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Vault;
impl CredentialStore for Vault {
    fn get(&self, _: &str) -> Result<Secret, DispatchError> {
        Secret::new(b"fixture-secret".to_vec())
    }
}

struct Server {
    endpoint: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(mode: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let seen = requests.clone();
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let Some((headers, request)) = read_request(&mut stream) else {
                    continue;
                };
                seen.lock().unwrap().push((headers, request.clone()));
                let method = request["method"].as_str().unwrap_or("");
                let id = request["id"].clone();
                let (status, body) = if method == "server/discover" {
                    match mode {
                        "modern" => (200, json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"complete","supportedVersions":["2026-07-28","2025-11-25"],"capabilities":{"tools":{}}}}).to_string()),
                        "malformed" => (200, json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"complete","supportedVersions":["2025-11-25"],"capabilities":{}}}).to_string()),
                        "unsupported" => (400, json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2099-01-01"]}}}).to_string()),
                        "legacy-404-method" => (404, json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}).to_string()),
                        "legacy-400-invalid" => (400, json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Invalid request"}}).to_string()),
                        "legacy-400-null" => (400, json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid request"}}).to_string()),
                        "legacy-200-params" => (200, json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Invalid params"}}).to_string()),
                        "legacy-only-version" => (400, json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2025-11-25"]}}}).to_string()),
                        "auth" => (401, json!({"error":"unauthorized"}).to_string()),
                        "timeout" => (408, json!({"error":"timeout"}).to_string()),
                        "rate_limited" => (429, json!({"error":"rate limited"}).to_string()),
                        "failure" => (503, json!({"error":"unavailable"}).to_string()),
                        "legacy" => (404, String::new()),
                        _ => unreachable!(),
                    }
                } else if method == "initialize"
                    && (mode == "legacy" || mode.starts_with("legacy-"))
                {
                    (200, json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}}).to_string())
                } else if method == "notifications/initialized"
                    && (mode == "legacy" || mode.starts_with("legacy-"))
                {
                    (202, String::new())
                } else {
                    (500, String::new())
                };
                write_response(&mut stream, status, &body);
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn connection(&self) -> Connection {
        Connection {
            label: "selection fixture".into(),
            credential_label: None,
            endpoint: self.endpoint.clone(),
            method: "POST".into(),
            auth: Auth::Bearer {
                credential_ref: "fixture".into(),
            },
            input_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
            timeout_ms: 2000,
            max_response_bytes: 16384,
            loopback_test: true,
            metering: None,
            mcp: Some(McpBinding {
                protocol: "2025-11-25".into(),
                transport: Transport::Http,
                tool: None,
                feature: None,
                interaction: None,
                sampling_worker: None,
            }),
        }
    }
    fn methods(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, v)| v["method"].as_str().unwrap_or("").to_owned())
            .collect()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn read_request(stream: &mut TcpStream) -> Option<(String, Value)> {
    let mut header = Vec::new();
    let mut byte = [0];
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() > 16384 || stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        header.push(byte[0]);
    }
    let headers = String::from_utf8(header).ok()?;
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    if length > 65536 {
        return None;
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body).ok()?;
    Some((headers, serde_json::from_slice(&body).ok()?))
}
fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

#[test]
fn modern_advertisement_selects_exact_supported_revision_without_effects() {
    let server = Server::new("modern");
    let connection = server.connection();
    assert_eq!(
        negotiate_protocol(&connection, &Vault).unwrap(),
        "2026-07-28"
    );
    assert_eq!(server.methods(), ["server/discover"]);
    let requests = server.requests.lock().unwrap();
    assert!(
        requests[0]
            .0
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-secret")
    );
    assert!(
        requests[0]
            .0
            .to_ascii_lowercase()
            .contains("mcp-protocol-version: 2026-07-28")
    );
    assert_eq!(
        requests[0].1["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2026-07-28"
    );
    assert_eq!(connection.mcp.unwrap().protocol, "2025-11-25");
}

#[test]
fn legacy_probe_uses_fresh_initialize_and_accepts_supported_counteroffer() {
    for mode in [
        "legacy",
        "legacy-404-method",
        "legacy-400-invalid",
        "legacy-400-null",
        "legacy-200-params",
        "legacy-only-version",
    ] {
        let server = Server::new(mode);
        assert_eq!(
            negotiate_protocol(&server.connection(), &Vault).unwrap(),
            "2025-03-26",
            "{mode}"
        );
        assert_eq!(
            server.methods(),
            ["server/discover", "initialize", "notifications/initialized"],
            "{mode}"
        );
        let requests = server.requests.lock().unwrap();
        assert!(requests.iter().all(|(headers, _)| {
            headers
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-secret")
        }));
        assert_eq!(requests[1].1["params"]["protocolVersion"], "2025-11-25");
        assert!(
            requests[2]
                .0
                .to_ascii_lowercase()
                .contains("mcp-protocol-version: 2025-03-26")
        );
    }
}

#[test]
fn invalid_metadata_unsupported_version_and_auth_never_fall_back_or_call_effect() {
    for mode in [
        "malformed",
        "unsupported",
        "auth",
        "timeout",
        "rate_limited",
        "failure",
    ] {
        let server = Server::new(mode);
        let error = negotiate_protocol(&server.connection(), &Vault).unwrap_err();
        if mode == "auth" {
            assert!(matches!(error, DispatchError::McpHttpStatus(401)));
        }
        if mode == "timeout" {
            assert!(matches!(error, DispatchError::McpHttpStatus(408)));
        }
        if mode == "rate_limited" {
            assert!(matches!(error, DispatchError::McpHttpStatus(429)));
        }
        if mode == "failure" {
            assert!(matches!(error, DispatchError::McpHttpStatus(503)));
        }
        assert_eq!(server.methods(), ["server/discover"], "{mode}");
    }
}

#[cfg(unix)]
#[test]
fn stdio_method_not_found_restarts_before_legacy_initialize() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("probe.log");
    let output = std::process::Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    let python = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .unwrap();
    let script = r#"import json,sys,os
with open(sys.argv[1], 'a') as f: f.write('spawn '+str(os.getpid())+'\n')
for line in sys.stdin:
 m=json.loads(line); method=m.get('method')
 with open(sys.argv[1], 'a') as f: f.write(method+'\n')
 if 'id' not in m: continue
 if method=='server/discover': out={'error':{'code':-32601,'message':'Method not found'}}
 elif method=='initialize': out={'result':{'protocolVersion':'2025-06-18','capabilities':{},'serverInfo':{'name':'fixture','version':'1'}}}
 else: raise RuntimeError(method)
 print(json.dumps({'jsonrpc':'2.0','id':m['id'],**out}),flush=True)
"#;
    let mut connection = Server::new("legacy").connection();
    connection.endpoint = "stdio:local".into();
    connection.loopback_test = false;
    connection.auth = Auth::None;
    connection.mcp.as_mut().unwrap().transport = Transport::Stdio {
        executable: python.to_string_lossy().into(),
        sha256: pin_executable(&python).unwrap(),
        args: vec![
            "-I".into(),
            "-u".into(),
            "-c".into(),
            script.into(),
            log.to_string_lossy().into(),
        ],
        credential_env: None,
    };
    assert_eq!(
        negotiate_protocol(&connection, &Vault).unwrap(),
        "2025-06-18"
    );
    let log = std::fs::read_to_string(log).unwrap();
    assert_eq!(log.matches("spawn ").count(), 2);
    assert_eq!(log.matches("server/discover").count(), 1);
    assert_eq!(log.matches("initialize\n").count(), 1);
    assert!(!log.contains("tools/call"));
}

#[cfg(unix)]
#[test]
fn stdio_invalid_params_with_null_probe_id_uses_fresh_legacy_process() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("probe.log");
    let output = std::process::Command::new("python3")
        .args(["-I", "-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    let python = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        .canonicalize()
        .unwrap();
    let script = r#"import json,sys,os
with open(sys.argv[1], 'a') as f: f.write('spawn '+str(os.getpid())+'\n')
for line in sys.stdin:
 m=json.loads(line); method=m.get('method')
 with open(sys.argv[1], 'a') as f: f.write(method+'\n')
 if 'id' not in m: continue
 if method=='server/discover': out={'id':None,'error':{'code':-32602,'message':'Invalid params'}}
 elif method=='initialize': out={'id':m['id'],'result':{'protocolVersion':'2025-06-18','capabilities':{},'serverInfo':{'name':'fixture','version':'1'}}}
 else: raise RuntimeError(method)
 print(json.dumps({'jsonrpc':'2.0',**out}),flush=True)
"#;
    let mut connection = Server::new("legacy").connection();
    connection.endpoint = "stdio:local".into();
    connection.loopback_test = false;
    connection.auth = Auth::None;
    connection.mcp.as_mut().unwrap().transport = Transport::Stdio {
        executable: python.to_string_lossy().into(),
        sha256: pin_executable(&python).unwrap(),
        args: vec![
            "-I".into(),
            "-u".into(),
            "-c".into(),
            script.into(),
            log.to_string_lossy().into(),
        ],
        credential_env: None,
    };
    assert_eq!(
        negotiate_protocol(&connection, &Vault).unwrap(),
        "2025-06-18"
    );
    let log = std::fs::read_to_string(log).unwrap();
    assert_eq!(log.matches("spawn ").count(), 2);
    assert_eq!(log.matches("server/discover").count(), 1);
    assert_eq!(log.matches("initialize\n").count(), 1);
    assert!(!log.contains("tools/call"));
}
