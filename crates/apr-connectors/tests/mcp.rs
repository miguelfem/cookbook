use apr_connectors::mcp::{McpBinding, ToolContract, Transport, pin_executable};
use apr_connectors::{Auth, Connection, CredentialStore, DispatchError, HttpBroker, Secret};
#[cfg(feature = "test-http")]
use serde_json::Value;
use serde_json::json;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

struct Vault;
impl CredentialStore for Vault {
    fn get(&self, _: &str) -> Result<Secret, DispatchError> {
        Secret::new(b"fixture-mcp-key".to_vec())
    }
}
fn contract() -> ToolContract {
    ToolContract {
        ui_resource_uri: None,
        name: "normalize".into(),
        input_schema: json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}),
        output_schema: Some(
            json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}}),
        ),
    }
}
fn connection(transport: Transport, protocol: &str) -> Connection {
    Connection {
        label: "fixture MCP".into(),
        credential_label: None,
        endpoint: "stdio:local".into(),
        method: "POST".into(),
        auth: Auth::None,
        input_schema: contract().input_schema,
        output_schema: json!({"type":"object"}),
        timeout_ms: 2000,
        max_response_bytes: 16384,
        loopback_test: false,
        metering: None,
        mcp: Some(McpBinding {
            sampling_worker: None,
            feature: None,
            interaction: None,
            protocol: protocol.into(),
            transport,
            tool: Some(contract()),
        }),
    }
}
fn native(temp: &tempfile::TempDir, mode: &str, protocol: &str) -> Connection {
    let python = std::env::var_os("APR_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let output = std::process::Command::new("python3")
                .args(["-I", "-c", "import sys; print(sys.executable)"])
                .output()
                .expect("Python required for MCP fixtures");
            PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        })
        .canonicalize()
        .unwrap();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mcp/server.py")
        .canonicalize()
        .unwrap();
    connection(
        Transport::Stdio {
            executable: python.to_string_lossy().into(),
            sha256: pin_executable(&python).unwrap(),
            args: vec![
                "-I".into(),
                "-u".into(),
                script.to_string_lossy().into(),
                mode.into(),
                temp.path().join("calls").to_string_lossy().into(),
            ],
            credential_env: None,
        },
        protocol,
    )
}
fn calls(temp: &tempfile::TempDir) -> String {
    std::fs::read_to_string(temp.path().join("calls")).unwrap_or_default()
}

#[test]
fn native_protocols_discover_validate_and_call_exact_tool() {
    for protocol in ["2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"] {
        let temp = tempfile::tempdir().unwrap();
        let cfg = native(&temp, "ok", protocol);
        let listed = apr_connectors::mcp::discover(&cfg, &Vault).unwrap();
        assert_eq!(listed["tools"][0]["name"], "normalize");
        assert!(!calls(&temp).contains("tools/call"));
        let output = HttpBroker::new(Vault)
            .dispatch(&cfg, &json!({"value":7}))
            .unwrap();
        assert_eq!(output.output["structuredContent"]["value"], 7);
        assert_eq!(calls(&temp).matches("tools/call").count(), 1);
    }
}
#[test]
fn draft7_tools_preserve_advertised_contracts_and_validate_inputs_and_outputs() {
    for mode in ["draft7", "draft7_bad_output"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cfg = native(&temp, mode, "2026-07-28");
        let draft = json!("http://json-schema.org/draft-07/schema#");
        let tool = cfg.mcp.as_mut().unwrap().tool.as_mut().unwrap();
        tool.input_schema["$schema"] = draft.clone();
        tool.output_schema.as_mut().unwrap()["$schema"] = draft.clone();
        cfg.input_schema = tool.input_schema.clone();
        cfg.output_schema["$schema"] = draft.clone();
        let listed = apr_connectors::mcp::discover(&cfg, &Vault).unwrap();
        assert_eq!(listed["tools"][0]["input_schema"], cfg.input_schema);
        assert_eq!(listed["tools"][0]["output_schema"]["$schema"], draft);
        assert!(!calls(&temp).contains("tools/call"));
        let broker = HttpBroker::new(Vault);
        assert!(matches!(
            broker.dispatch(&cfg, &json!({"value":"wrong type"})),
            Err(DispatchError::InvalidInput)
        ));
        assert!(!calls(&temp).contains("tools/call"));
        let result = broker.dispatch(&cfg, &json!({"value":7}));
        if mode == "draft7" {
            assert_eq!(result.unwrap().output["structuredContent"]["value"], 7);
        } else {
            assert!(matches!(result, Err(DispatchError::Uncertain)));
        }
        assert_eq!(calls(&temp).matches("tools/call").count(), 1);
    }
}
#[cfg(unix)]
#[test]
fn native_python_venv_symlink_imports_installed_module_and_detects_retargeting() {
    let temp = tempfile::tempdir().unwrap();
    let base = std::env::var_os("APR_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let output = std::process::Command::new("python3")
                .args(["-I", "-c", "import sys; print(sys.executable)"])
                .output()
                .expect("Python required for MCP fixtures");
            assert!(output.status.success());
            PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
        });
    let venv = temp.path().join("env");
    assert!(
        std::process::Command::new(&base)
            .args(["-m", "venv", "--without-pip", "--symlinks"])
            .arg(&venv)
            .status()
            .unwrap()
            .success()
    );
    let python = venv.join("bin/python3");
    assert!(python.symlink_metadata().unwrap().file_type().is_symlink());
    let purelib = std::process::Command::new(&python)
        .args([
            "-I",
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .unwrap();
    assert!(purelib.status.success());
    let packages = PathBuf::from(String::from_utf8(purelib.stdout).unwrap().trim());
    std::fs::write(packages.join("apr_venv_probe.py"), "VALUE = 'venv-only'\n").unwrap();
    let script = temp.path().join("server.py");
    std::fs::write(
        &script,
        format!(
            "import apr_venv_probe\nassert apr_venv_probe.VALUE == 'venv-only'\n{}",
            include_str!("../../../tests/fixtures/mcp/server.py")
        ),
    )
    .unwrap();
    let digest = pin_executable(&python).unwrap();
    let cfg = connection(
        Transport::Stdio {
            executable: python.to_string_lossy().into_owned(),
            sha256: digest,
            args: vec![
                "-I".into(),
                "-u".into(),
                script.to_string_lossy().into_owned(),
                "ok".into(),
                temp.path().join("calls").to_string_lossy().into_owned(),
            ],
            credential_env: None,
        },
        "2025-11-25",
    );
    let result = HttpBroker::new(Vault)
        .dispatch(&cfg, &json!({"value": 7}))
        .unwrap();
    assert_eq!(result.output["structuredContent"]["value"], 7);

    std::fs::remove_file(&python).unwrap();
    symlink("/bin/sh", &python).unwrap();
    std::fs::remove_file(temp.path().join("calls")).unwrap();
    assert!(matches!(
        HttpBroker::new(Vault).dispatch(&cfg, &json!({"value": 7})),
        Err(DispatchError::InvalidConnection)
    ));
    assert!(!temp.path().join("calls").exists());
}
#[test]
fn native_input_pin_and_drift_checks_never_invoke_tool() {
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = native(&temp, "drift", "2025-11-25");
    assert!(matches!(
        HttpBroker::new(Vault).dispatch(&cfg, &json!({"value":"wrong"})),
        Err(DispatchError::InvalidInput)
    ));
    assert!(calls(&temp).is_empty());
    assert!(matches!(
        HttpBroker::new(Vault).dispatch(&cfg, &json!({"value":2})),
        Err(DispatchError::Uncertain)
    ));
    assert!(!calls(&temp).contains("tools/call")); // native startup itself can have effects
    if let Transport::Stdio { sha256, .. } = &mut cfg.mcp.as_mut().unwrap().transport {
        *sha256 = "0".repeat(64);
    }
    std::fs::remove_file(temp.path().join("calls")).unwrap();
    assert!(
        HttpBroker::new(Vault)
            .dispatch(&cfg, &json!({"value":1}))
            .is_err()
    );
    assert!(calls(&temp).is_empty());
}
#[test]
fn native_secret_is_scoped_and_echoes_are_rejected() {
    for mode in ["env", "echo"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cfg = native(&temp, mode, "2026-07-28");
        cfg.auth = Auth::Bearer {
            credential_ref: "fixture".into(),
        };
        if let Transport::Stdio { credential_env, .. } = &mut cfg.mcp.as_mut().unwrap().transport {
            *credential_env = Some("SERVICE_API_KEY".into());
        }
        let result = HttpBroker::new(Vault).dispatch(&cfg, &json!({"value":3}));
        if mode == "env" {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(DispatchError::Uncertain)));
        }
    }
}
#[test]
fn native_protocol_errors_timeouts_and_interactions_are_uncertain() {
    for mode in ["timeout", "flood", "error", "interaction", "input_required"] {
        let temp = tempfile::tempdir().unwrap();
        let mut cfg = native(&temp, mode, "2026-07-28");
        cfg.timeout_ms = 200;
        let start = std::time::Instant::now();
        assert!(
            matches!(
                HttpBroker::new(Vault).dispatch(&cfg, &json!({"value":1})),
                Err(DispatchError::Uncertain)
            ),
            "{mode}"
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
    }
}
#[test]
fn native_process_group_is_cleaned_after_a_successful_response() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = native(&temp, "child", "2025-11-25");
    HttpBroker::new(Vault)
        .dispatch(&cfg, &json!({"value":1}))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(450));
    assert!(!temp.path().join("calls.child").exists());
}
#[test]
fn cancelled_dispatch_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = native(&temp, "timeout", "2026-07-28");
    let flag = AtomicBool::new(true);
    assert!(
        HttpBroker::new(Vault)
            .dispatch_cancellable(&cfg, &json!({"value":1}), Some(&flag))
            .is_err()
    );
    assert!(flag.load(Ordering::Acquire));
}

#[cfg(feature = "test-http")]
mod http {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    struct Server {
        endpoint: String,
        stop: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<Value>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn new(mode: &'static str, protocol: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let flag = stop.clone();
            let observed = requests.clone();
            let thread = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !flag.load(Ordering::Acquire) && Instant::now() < deadline {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            prepare_stream(&stream);
                            let Some((headers, msg)) = read(&mut stream) else {
                                continue;
                            };
                            let method = msg["method"].as_str().unwrap_or("");
                            if headers.starts_with("DELETE") {
                                respond(&mut stream, 204, "application/json", "");
                                continue;
                            }
                            if protocol == "2026-07-28" {
                                assert_eq!(
                                    msg.pointer(
                                        "/params/_meta/io.modelcontextprotocol~1protocolVersion"
                                    )
                                    .unwrap(),
                                    protocol
                                );
                                assert_eq!(
                                    msg.pointer(
                                        "/params/_meta/io.modelcontextprotocol~1clientCapabilities"
                                    )
                                    .unwrap(),
                                    &json!({})
                                );
                                assert!(
                                    headers
                                        .to_ascii_lowercase()
                                        .contains(&format!("mcp-method: {method}"))
                                );
                            }
                            observed.lock().unwrap().push(msg.clone());
                            if mode == "reject-metadata"
                                || (mode == "reject-call" && method == "tools/call")
                            {
                                respond(
                                    &mut stream,
                                    400,
                                    "application/json",
                                    r#"{"error":"fixture-mcp-key must never appear in diagnostics"}"#,
                                );
                                continue;
                            }
                            if method == "notifications/initialized" {
                                respond(&mut stream, 202, "application/json", "");
                                continue;
                            }
                            let mut result = if protocol == "2026-07-28" {
                                json!({"resultType":"complete"})
                            } else {
                                json!({})
                            };
                            if method == "initialize" {
                                result["protocolVersion"] = json!(protocol);
                                result["capabilities"] = json!({"tools":{}});
                            } else if method == "tools/list" {
                                let mut c = contract();
                                if mode == "drift" {
                                    c.input_schema["properties"]["value"]["type"] = json!("string");
                                }
                                result["tools"] = json!([{"name":c.name,"inputSchema":c.input_schema,"outputSchema":c.output_schema}]);
                            } else if method == "tools/call" {
                                assert_eq!(msg["params"]["name"], "normalize");
                                if mode == "drop" {
                                    continue;
                                }
                                result["content"] = json!([]);
                                result["structuredContent"] =
                                    json!({"value":msg["params"]["arguments"]["value"]});
                                if mode == "error" {
                                    result["isError"] = json!(true);
                                }
                                if mode == "echo" {
                                    result["content"] =
                                        json!([{"type":"text","text":"fixture-mcp-key"}]);
                                }
                            }
                            let body =
                                json!({"jsonrpc":"2.0","id":msg["id"],"result":result}).to_string();
                            if mode == "sse" {
                                respond(
                                    &mut stream,
                                    200,
                                    "text/event-stream",
                                    &format!("event: message\ndata: {body}\n\n"),
                                );
                            } else {
                                respond(&mut stream, 200, "application/json", &body);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2))
                        }
                        Err(e) => panic!("{e}"),
                    }
                }
            });
            Self {
                endpoint,
                stop,
                requests,
                thread: Some(thread),
            }
        }
        fn connection(&self, protocol: &str) -> Connection {
            let mut cfg = connection(Transport::Http, protocol);
            cfg.endpoint = self.endpoint.clone();
            cfg.loopback_test = true;
            cfg.auth = Auth::Bearer {
                credential_ref: "fixture".into(),
            };
            cfg
        }
        fn tool_calls(&self) -> usize {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|v| v["method"] == "tools/call")
                .count()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    fn prepare_stream(stream: &TcpStream) {
        // macOS inherits O_NONBLOCK from the polling listener. Read timeouts do
        // not clear it: request bytes can arrive after accept and must be read
        // with the same bounded blocking semantics as on Linux.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
    }
    fn read(stream: &mut TcpStream) -> Option<(String, Value)> {
        let mut data = Vec::new();
        let mut byte = [0];
        while !data.ends_with(b"\r\n\r\n") {
            if data.len() > 16384 || stream.read(&mut byte).ok()? == 0 {
                return None;
            }
            data.push(byte[0]);
        }
        let headers = String::from_utf8(data).ok()?;
        let len = headers
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if len > 65536 {
            return None;
        }
        let mut body = vec![0; len];
        stream.read_exact(&mut body).ok()?;
        Some((
            headers,
            if body.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&body).ok()?
            },
        ))
    }
    fn respond(stream: &mut TcpStream, status: u16, kind: &str, body: &str) {
        let text = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(text.as_bytes());
    }
    #[test]
    fn metadata_http_failure_is_actionable_without_exposing_body_or_retrying_effects() {
        let server = Server::new("reject-metadata", "2025-06-18");
        let cfg = server.connection("2025-06-18");
        let error = apr_connectors::mcp::discover(&cfg, &Vault).unwrap_err();
        assert!(error.to_string().contains("HTTP 400"), "{error}");
        assert!(!error.may_have_executed());
        assert!(!error.to_string().contains("fixture-mcp-key"));
        assert_eq!(server.tool_calls(), 0);

        let server = Server::new("reject-call", "2025-06-18");
        let cfg = server.connection("2025-06-18");
        assert!(matches!(
            HttpBroker::new(Vault).dispatch(&cfg, &json!({"value":7})),
            Err(DispatchError::Uncertain)
        ));
        assert_eq!(server.tool_calls(), 1);
    }
    #[test]
    fn accepted_fixture_stream_waits_for_delayed_request_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("fixture did not accept the connected client: {error}"),
            }
        };
        prepare_stream(&stream);
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            std::thread::sleep(Duration::from_millis(30));
            client.write_all(b"POST /mcp HTTP/1.1\r\nContent-Length: 11\r\n\r\n")?;
            std::thread::sleep(Duration::from_millis(30));
            client.write_all(br#"{"value":7}"#)
        });
        let received = read(&mut stream);
        writer.join().unwrap().unwrap();
        assert_eq!(
            received
                .expect("an accepted fixture socket must wait for delayed headers and body")
                .1,
            json!({"value":7})
        );
    }
    #[test]
    fn remote_json_and_sse_support_both_protocol_eras() {
        for protocol in ["2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"] {
            for mode in ["json", "sse"] {
                let server = Server::new(mode, protocol);
                let cfg = server.connection(protocol);
                let result = HttpBroker::new(Vault)
                    .dispatch(&cfg, &json!({"value":9}))
                    .unwrap();
                assert_eq!(result.output["structuredContent"]["value"], 9);
                assert_eq!(server.tool_calls(), 1);
            }
        }
    }
    #[test]
    fn remote_schema_drift_is_proven_before_tool_dispatch() {
        let server = Server::new("drift", "2026-07-28");
        assert!(matches!(
            HttpBroker::new(Vault).dispatch(&server.connection("2026-07-28"), &json!({"value":1})),
            Err(DispatchError::InvalidConnection)
        ));
        assert_eq!(server.tool_calls(), 0);
    }
    #[test]
    fn remote_response_loss_errors_and_secret_echoes_never_retry() {
        for mode in ["drop", "error", "echo"] {
            let server = Server::new(mode, "2026-07-28");
            assert!(matches!(
                HttpBroker::new(Vault)
                    .dispatch(&server.connection("2026-07-28"), &json!({"value":1})),
                Err(DispatchError::Uncertain)
            ));
            assert_eq!(server.tool_calls(), 1);
        }
    }
}

#[test]
fn approved_resources_prompts_and_subscription_windows_use_bounded_transport() {
    use apr_connectors::mcp::{FeatureContract, subscription::Filter};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = native(&temp, "features", "2026-07-28");
    cfg.mcp.as_mut().unwrap().tool = None;
    cfg.input_schema = json!({"type":"object"});
    let resources = apr_connectors::mcp::discover_features(&cfg, &Vault, "resources").unwrap();
    assert_eq!(resources["items"][0]["uri"], "fixture://record");
    cfg.mcp.as_mut().unwrap().feature = Some(FeatureContract::Resource {
        uri: "fixture://record".into(),
    });
    let result = HttpBroker::new(Vault).dispatch(&cfg, &json!({})).unwrap();
    assert_eq!(result.output["contents"][0]["text"], "Synthetic resource");
    cfg.mcp.as_mut().unwrap().feature = Some(FeatureContract::Prompt {
        name: "summarize".into(),
        arguments: json!([{"name":"topic","required":true}]),
    });
    let result = HttpBroker::new(Vault)
        .dispatch(&cfg, &json!({"topic":"synthetic"}))
        .unwrap();
    assert_eq!(result.output["messages"][0]["role"], "user");
    cfg.mcp.as_mut().unwrap().feature = Some(FeatureContract::Subscription {
        notifications: serde_json::from_value::<Filter>(json!({"toolsListChanged":true})).unwrap(),
    });
    let result = HttpBroker::new(Vault)
        .dispatch(&cfg, &json!({"max_notifications":8}))
        .unwrap();
    assert_eq!(result.output["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(result.output["server_closed_gracefully"], true);
    assert_eq!(result.output["continuous_delivery_guaranteed"], false);
    let mut drift = native(&temp, "features", "2026-07-28");
    drift.mcp.as_mut().unwrap().tool = None;
    drift.input_schema = json!({"type":"object"});
    drift.mcp.as_mut().unwrap().feature = Some(FeatureContract::Prompt {
        name: "summarize".into(),
        arguments: json!([]),
    });
    assert!(HttpBroker::new(Vault).dispatch(&drift, &json!({})).is_err());
    assert!(!calls(&temp).contains("tools/call"));
}
