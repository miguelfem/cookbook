#![cfg(feature = "fault-injection")]

use apr_connectors::{Auth, Connection, Secret};
use apr_runtime::{Runtime, connection_library::SavedAccount};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;

#[test]
fn saved_account_survives_failed_discovery_and_retries_without_granting_tools() {
    for header in ["bearer", "x-api-key", "api-key"] {
        account_discovery_case(header, false);
    }
}

#[test]
fn automatic_discovery_reuses_account_after_restart_without_widening_authority() {
    account_discovery_case("bearer", true);
}

fn account_discovery_case(header: &str, automatic_protocol: bool) {
    let expected_header = if header == "bearer" {
        "authorization: bearer account-discovery-fixture".to_owned()
    } else {
        format!("{header}: account-discovery-fixture")
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for request_index in 0..if automatic_protocol { 4 } else { 2 } {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
                assert!(headers.len() < 8192);
            }
            let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();
            assert!(headers.contains(&expected_header));
            let size: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let mut body = vec![0; size];
            stream.read_exact(&mut body).unwrap();
            let request: Value = serde_json::from_slice(&body).unwrap();
            let probe = automatic_protocol && request_index % 2 == 0;
            assert_eq!(
                request["method"],
                if probe {
                    "server/discover"
                } else {
                    "tools/list"
                }
            );
            let attempt = if automatic_protocol {
                request_index / 2
            } else {
                request_index
            };
            let (status, body) = if probe {
                (
                    200,
                    json!({"jsonrpc":"2.0","id":request["id"],"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}}}),
                )
            } else if attempt == 0 {
                (
                    400,
                    json!({"error":"account-discovery-fixture must not leak"}),
                )
            } else {
                (
                    200,
                    json!({"jsonrpc":"2.0","id":request["id"],"result":{"resultType":"complete","tools":[{"name":"read_summary","inputSchema":{"type":"object"}}]}}),
                )
            };
            let body = body.to_string();
            write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::open(temp.path().join("state")).unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let assignment = runtime.owner_project_setup(&project, "synthetic").unwrap();
    let connection: Connection = serde_json::from_value(json!({
        "label":"synthetic account","endpoint":endpoint,"method":"POST",
        "auth":if header=="bearer" {json!({"kind":"bearer","credential_ref":"pending"})} else {json!({"kind":"header","name":header,"credential_ref":"pending"})},
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},
        "timeout_ms":2000,"max_response_bytes":4096,"loopback_test":true,
        "mcp":{"protocol":"2026-07-28","transport":{"kind":"http"}}
    }))
    .unwrap();
    assert!(
        runtime
            .connect_mcp_account(
                &connection,
                Some(Secret::new(b"account-discovery-fixture".to_vec()).unwrap()),
                None,
                "invalid"
            )
            .is_err()
    );
    assert_eq!(runtime.saved_accounts(None).unwrap()["accounts"], json!([]));
    let first = runtime
        .connect_mcp_account_protocol(
            &connection,
            Some(Secret::new(b"account-discovery-fixture".to_vec()).unwrap()),
            None,
            "tools",
            automatic_protocol,
        )
        .unwrap();
    let id = first["account"]["account_id"].as_str().unwrap();
    assert_eq!(first["grant_created"], false);
    assert!(
        first["discovery_error"]
            .as_str()
            .unwrap()
            .contains("HTTP 400")
    );
    assert!(!first.to_string().contains("account-discovery-fixture"));
    assert_eq!(
        runtime.saved_accounts(None).unwrap()["accounts"][0]["enabled"],
        true
    );
    assert!(
        runtime
            .connect_mcp_account(
                &connection,
                Some(Secret::new(b"account-discovery-fixture".to_vec()).unwrap()),
                Some(SavedAccount {
                    account_id: id.into(),
                    revision: 1
                }),
                "tools"
            )
            .is_err()
    );
    // Restarting the runtime must not require re-entering the key.
    drop(runtime);
    let mut runtime = Runtime::open(temp.path().join("state")).unwrap();
    let retry = runtime
        .connect_mcp_account_protocol(
            &connection,
            None,
            Some(SavedAccount {
                account_id: id.into(),
                revision: 1,
            }),
            "tools",
            automatic_protocol,
        )
        .unwrap();
    let result = &retry["discovery"];
    assert_eq!(result["tools"][0]["name"], "read_summary");
    assert_eq!(result["protocol_detected"], automatic_protocol);
    assert_eq!(result["protocol"], "2026-07-28");
    assert_eq!(retry["grant_created"], false);
    assert_eq!(
        runtime.saved_accounts(None).unwrap()["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!result.to_string().contains("account-discovery-fixture"));
    server.join().unwrap();
    assert_eq!(
        runtime
            .execution_bindings(assignment["assignment_id"].as_str().unwrap())
            .unwrap()["bindings"],
        json!([])
    );
    assert_eq!(
        runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
    assert!(
        matches!(&connection.auth, Auth::Bearer {credential_ref} | Auth::Header {credential_ref,..} if credential_ref == "pending")
    );
    let mut changed = connection.clone();
    changed.endpoint.push_str("?another=true");
    assert!(
        runtime
            .discover_mcp_with_account(&changed, id, 1, "tools")
            .unwrap_err()
            .to_string()
            .contains("does not match")
    );
    assert!(
        runtime
            .discover_mcp_with_account(&connection, id, 2, "tools")
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
    assert!(
        runtime
            .connect_mcp_account(
                &changed,
                None,
                Some(SavedAccount {
                    account_id: id.into(),
                    revision: 1
                }),
                "tools"
            )
            .unwrap_err()
            .to_string()
            .contains("does not match")
    );
    runtime.revoke_saved_account(id, 1).unwrap();
    assert!(
        runtime
            .connect_mcp_account(
                &connection,
                None,
                Some(SavedAccount {
                    account_id: id.into(),
                    revision: 1
                }),
                "tools"
            )
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
    assert!(
        runtime
            .discover_mcp_with_account(&connection, id, 1, "tools")
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
}
