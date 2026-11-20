use apr_connectors::{Auth, Connection};
use apr_runtime::{Runtime, setup::ConnectionProposal};
use serde_json::json;

#[test]
fn pending_owner_approval_waits_for_explicit_maintenance_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    let project = temporary.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let state = temporary.path().join("state");
    let mut runtime = Runtime::open(&state).unwrap();
    let assignment = runtime
        .owner_project_setup(&project, "Synthetic approval")
        .unwrap();
    let assignment = assignment["assignment_id"].as_str().unwrap();
    let attachment = runtime.create_attachment(assignment, "author").unwrap();
    let token = attachment["token"].as_str().unwrap();
    let contract = json!({"view":"connection_contract","name":"synthetic.read"});
    let proposal = runtime
        .propose_connection(
            assignment,
            ConnectionProposal {
                name: "synthetic.read".into(),
                request_key: "before-maintenance".into(),
                rationale: "Synthetic public-data approval; no request is dispatched".into(),
                connection: Connection {
                    label: "Synthetic public read".into(),
                    credential_label: None,
                    endpoint: "https://example.com/synthetic".into(),
                    method: "GET".into(),
                    auth: Auth::None,
                    input_schema: json!({"type":"object"}),
                    output_schema: json!({"type":"object"}),
                    timeout_ms: 1000,
                    max_response_bytes: 4096,
                    loopback_test: false,
                    metering: None,
                    mcp: None,
                },
            },
        )
        .unwrap();
    let request = proposal["request_id"].as_str().unwrap();
    let digest = proposal["digest"].as_str().unwrap();

    // An OAuth callback or owner CLI can hold a Runtime opened before the updater
    // closes its gate. Approval must recheck durable state under its own lock.
    let mut owner = Runtime::open(&state).unwrap();
    runtime.prepare_desktop_update("0.2.1").unwrap();
    let error = owner.approve_connection(request, digest, None).unwrap_err();
    assert!(error.to_string().contains("paused for a desktop update"));
    assert_eq!(
        runtime.connection_requests(assignment).unwrap()["requests"][0]["state"],
        "pending"
    );
    assert!(
        runtime
            .scoped_call(token, "inspect", contract.clone())
            .is_err()
    );

    drop(runtime);
    let mut runtime = Runtime::open(&state).unwrap();
    assert_eq!(
        runtime.maintenance_status().unwrap()["paused_for_update"],
        true
    );
    assert!(runtime.approve_connection(request, digest, None).is_err());
    assert_eq!(
        runtime.connection_requests(assignment).unwrap()["requests"][0]["state"],
        "pending"
    );

    runtime.finish_desktop_update().unwrap();
    let approved = owner.approve_connection(request, digest, None).unwrap();
    assert_eq!(approved["granted"], true);
    assert_eq!(approved["revision"], 1);
    assert_eq!(
        runtime.approve_connection(request, digest, None).unwrap(),
        approved
    );
    assert_eq!(
        runtime.connection_requests(assignment).unwrap()["requests"][0]["state"],
        "approved"
    );
    let contract = runtime.scoped_call(token, "inspect", contract).unwrap();
    assert_eq!(contract["enabled"], true);
    assert_eq!(contract["revision"], 1);
}
