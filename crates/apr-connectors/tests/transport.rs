use apr_connectors::{Auth, Connection, CredentialStore, DispatchError, HttpBroker, Secret};
use serde_json::json;

struct Vault;
impl CredentialStore for Vault {
    fn get(&self, _: &str) -> Result<Secret, DispatchError> {
        Secret::new(b"apr-private-test-secret-123".to_vec())
    }
}

fn connection(endpoint: String) -> Connection {
    Connection {
        mcp: None,
        metering: None,
        credential_label: None,
        label: "fixture".into(),
        endpoint,
        method: "POST".into(),
        auth: Auth::Bearer {
            credential_ref: "fixture-key".into(),
        },
        input_schema: json!({"type":"object","required":["value"],"additionalProperties":false,"properties":{"value":{"type":"integer"}}}),
        output_schema: json!({"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}},"additionalProperties":false}),
        timeout_ms: 250,
        max_response_bytes: 4096,
        loopback_test: false,
    }
}

#[test]
fn configuration_rejects_cleartext_and_userinfo() {
    assert!(
        connection("http://example.com/op".into())
            .validate()
            .is_err()
    );
    assert!(
        connection("https://user:password@example.com/op".into())
            .validate()
            .is_err()
    );
    assert!(
        connection("https://127.0.0.1/op".into())
            .validate()
            .is_err()
    );
    assert!(
        connection("https://example.com/op".into())
            .validate()
            .is_ok()
    );
    let result = HttpBroker::new(Vault).dispatch(
        &connection("https://example.com/op".into()),
        &json!({"url":"https://elsewhere.example"}),
    );
    assert!(matches!(result, Err(DispatchError::InvalidInput)));
}

#[test]
fn credential_label_is_optional_nonsecret_metadata_with_legacy_serialization() {
    let mut labeled = connection("https://example.com/op".into());
    labeled.credential_label = Some("generic project API key".into());
    assert!(labeled.validate().is_ok());

    let mut unauthenticated = labeled.clone();
    unauthenticated.auth = Auth::None;
    assert!(unauthenticated.validate().is_err());

    let mut invalid = labeled.clone();
    invalid.credential_label = Some(" \t".into());
    assert!(invalid.validate().is_err());
    invalid.credential_label = Some("x".repeat(161));
    assert!(invalid.validate().is_err());

    let legacy = serde_json::to_value(connection("https://example.com/op".into())).unwrap();
    assert!(legacy.get("credential_label").is_none());
    let decoded: Connection = serde_json::from_value(legacy).unwrap();
    assert!(decoded.credential_label.is_none());
}

#[cfg(feature = "test-http")]
mod http {
    use super::*;
    use std::io::{self, Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    const FIXTURE_TIMEOUT: Duration = Duration::from_secs(2);
    const MAX_HEADER_BYTES: usize = 16 * 1024;
    const MAX_BODY_BYTES: usize = 64 * 1024;

    fn remaining(deadline: Instant) -> io::Result<Duration> {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "fixture deadline expired"))
    }

    fn accept(listener: &TcpListener, deadline: Instant) -> io::Result<TcpStream> {
        listener.set_nonblocking(true)?;
        loop {
            let left = remaining(deadline)?;
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    return Ok(stream);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(left.min(Duration::from_millis(1)));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn read_request(
        stream: &mut TcpStream,
        deadline: Instant,
        mut headers_observed: Option<mpsc::Sender<()>>,
    ) -> io::Result<String> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid fixture request");
        let mut data = Vec::new();
        let mut expected = None;
        let mut buf = [0; 1024];
        loop {
            stream.set_read_timeout(Some(remaining(deadline)?))?;
            let n = match stream.read(&mut buf) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete fixture request",
                ));
            }
            data.extend_from_slice(&buf[..n]);
            if expected.is_none() {
                if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let end = end + 4;
                    if end > MAX_HEADER_BYTES {
                        return Err(invalid());
                    }
                    let headers = std::str::from_utf8(&data[..end]).map_err(|_| invalid())?;
                    let mut length = None;
                    for (name, value) in headers.lines().skip(1).filter_map(|l| l.split_once(':')) {
                        if name.eq_ignore_ascii_case("transfer-encoding") {
                            return Err(invalid());
                        }
                        if name.eq_ignore_ascii_case("content-length") {
                            let parsed = value.trim().parse::<usize>().map_err(|_| invalid())?;
                            if parsed > MAX_BODY_BYTES || length.replace(parsed).is_some() {
                                return Err(invalid());
                            }
                        }
                    }
                    expected = Some(end + length.unwrap_or(0));
                    if let Some(observer) = headers_observed.take() {
                        observer.send(()).map_err(|_| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "fixture observer closed")
                        })?;
                    }
                } else if data.len() >= MAX_HEADER_BYTES {
                    return Err(invalid());
                }
            }
            if let Some(expected) = expected {
                if data.len() > expected {
                    return Err(invalid());
                }
                if data.len() == expected {
                    return String::from_utf8(data).map_err(|_| invalid());
                }
            }
        }
    }

    fn serve(response: Option<String>) -> (String, thread::JoinHandle<String>) {
        serve_with_header_observer(response, None)
    }

    fn serve_with_header_observer(
        response: Option<String>,
        headers_observed: Option<mpsc::Sender<()>>,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/op", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + FIXTURE_TIMEOUT;
            let mut stream = accept(&listener, deadline).unwrap();
            let request = read_request(&mut stream, deadline, headers_observed).unwrap();
            stream
                .set_write_timeout(Some(remaining(deadline).unwrap()))
                .unwrap();
            if let Some(response) = response {
                let _ = stream.write_all(response.as_bytes());
            } else {
                thread::sleep(Duration::from_millis(450));
            }
            request
        });
        (url, handle)
    }

    #[test]
    fn fixture_consumes_a_post_body_sent_after_its_headers() {
        let (observed, headers_read) = mpsc::channel();
        let (url, server) =
            serve_with_header_observer(Some(reply(r#"{"ok":true}"#)), Some(observed));
        let address = url
            .strip_prefix("http://")
            .unwrap()
            .strip_suffix("/op")
            .unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let body = r#"{"value":7}"#;
        write!(
            stream,
            "POST /op HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        // The server has consumed only headers, so packet coalescing cannot
        // accidentally make a headers-only request reader pass this test.
        headers_read.recv_timeout(Duration::from_secs(2)).unwrap();
        let sent = stream.write_all(body.as_bytes());
        let request = server.join().unwrap();
        assert_eq!(request.split_once("\r\n\r\n").unwrap().1, body);
        sent.unwrap();
    }

    #[test]
    fn fixture_rejects_eof_during_headers_or_body() {
        for partial in [
            "POST /op HTTP/1.1\r\nContent-Length: 10",
            "POST /op HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let deadline = Instant::now() + FIXTURE_TIMEOUT;
                let mut stream = accept(&listener, deadline).unwrap();
                read_request(&mut stream, deadline, None)
            });
            let mut stream = TcpStream::connect(address).unwrap();
            stream.set_write_timeout(Some(FIXTURE_TIMEOUT)).unwrap();
            stream.write_all(partial.as_bytes()).unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            assert_eq!(
                server.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
    }

    fn reply(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[test]
    fn broker_injects_secret_and_returns_validated_json() {
        let (url, server) = serve(Some(reply(r#"{"ok":true}"#)));
        let mut c = connection(url);
        c.loopback_test = true;
        let output = HttpBroker::new(Vault)
            .dispatch(&c, &json!({"value":7}))
            .unwrap();
        assert_eq!(output.output, json!({"ok":true}));
        assert_eq!(output.status, 200);
        let request = server.join().unwrap();
        assert!(request.contains("Bearer apr-private-test-secret-123"));
        assert!(
            !serde_json::to_string(&output)
                .unwrap()
                .contains("private-test-secret")
        );
    }

    #[test]
    fn fixed_get_has_no_body_and_cannot_accept_query_substitutions() {
        let (url, server) = serve(Some(reply(r#"{"ok":true}"#)));
        let mut c = connection(url);
        c.loopback_test = true;
        c.method = "GET".into();
        c.input_schema = json!({"type":"object"});
        assert!(matches!(
            HttpBroker::new(Vault).dispatch(&c, &json!({"query":"anything"})),
            Err(DispatchError::InvalidInput)
        ));
        assert_eq!(
            HttpBroker::new(Vault)
                .dispatch(&c, &json!({}))
                .unwrap()
                .output,
            json!({"ok":true})
        );
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /op HTTP/1.1"));
        assert!(!request.contains("query"));
    }

    #[test]
    fn response_loss_redirect_invalid_json_and_echo_require_reconciliation() {
        for response in [None, Some("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/leak\r\nContent-Length: 0\r\n\r\n".into()), Some(reply("not-json")), Some(reply(r#"{"ok":true,"leaked":"apr-private-test-secret-123"}"#)), Some(reply(&"x".repeat(5000)))] {
            let (url, server) = serve(response);
            let mut c = connection(url); c.loopback_test = true;
            let error = HttpBroker::new(Vault).dispatch(&c, &json!({"value":7})).unwrap_err();
            assert!(error.may_have_executed());
            assert!(!error.to_string().contains("private-test-secret"));
            server.join().unwrap();
        }
    }
}
