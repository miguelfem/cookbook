use apr_connectors::{Auth, Connection, CredentialStore, DispatchError, HttpBroker, Secret};
use apr_provider_jev::{observe, prepare};
use serde_json::json;
use std::io::{Read, Write};
struct Vault;
impl CredentialStore for Vault {
    fn get(&self, _: &str) -> Result<Secret, DispatchError> {
        Secret::new(b"synthetic-fixture-secret".to_vec())
    }
}
#[test]
fn protected_broker_roundtrip_validates_the_jev_wire_contract_without_a_provider_account() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut data = Vec::new();
        let mut buf = [0; 1024];
        loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buf[..n]);
            if let Some(i) = data.windows(4).position(|p| p == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..i]);
                let len: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                if data.len() >= i + 4 + len {
                    break;
                }
            }
        }
        let response=json!({"model":"jev-fixture","answers":{"route":{"type":"choice","choice":"review","probabilities":{"review":0.75,"publish":0.25}}},"usage":{"input_tokens":24,"output_tokens":3}}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .unwrap();
        data
    });
    let request=prepare(json!({"model":"jev-fixture","state":{"text":"A document"},"questions":{"route":{"type":"choice","instructions":"Choose next step","criteria":{"review":"Needs review","publish":"Ready"}}}})).unwrap();
    let connection = Connection {
        mcp: None,
        metering: None,
        label: "Contract fixture".into(),
        credential_label: None,
        endpoint,
        method: "POST".into(),
        auth: Auth::Bearer {
            credential_ref: "fixture".into(),
        },
        input_schema: json!({"type":"object"}),
        output_schema: json!({"type":"object"}),
        timeout_ms: 2000,
        max_response_bytes: 4096,
        loopback_test: true,
    };
    let response = HttpBroker::new(Vault)
        .dispatch(&connection, request.body())
        .unwrap();
    let observation = serde_json::to_value(observe(&request, response.output).unwrap()).unwrap();
    assert_eq!(observation["answers"]["route"]["choice"], "review");
    assert!(observation["answers"]["route"].get("confidence").is_none());
    assert_eq!(observation["usage"]["input_tokens"], 24);
    let request = String::from_utf8(server.join().unwrap()).unwrap();
    assert!(request.contains("Bearer synthetic-fixture-secret"));
    assert!(!observation.to_string().contains("synthetic-fixture-secret"));
}
