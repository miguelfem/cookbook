use apr_core::{
    admit_json,
    audit::{ReviewInput, audit_recipe},
};
use serde_json::json;

#[test]
fn pending_connector_is_reviewed_without_inventing_caller_work() {
    let process = admit_json(include_bytes!("../../../examples/http-json/process.json")).unwrap();
    let review: ReviewInput = serde_json::from_value(json!({
        "connections": {"records": {"access": "pending", "proposal_ref": "proposal-1"}},
        "baseline": {"model_round_trips": 2, "raw_http_calls": 5, "retained_input_tokens": null, "retained_output_tokens": null},
        "expected": {"model_round_trips": 0, "raw_http_calls": 5, "retained_input_tokens": null, "retained_output_tokens": null},
        "final_output": {"max_return_bytes": 4096, "raw_artifact": true, "paged_or_aggregated": true, "revision_delta_includes_removals": true}
    })).unwrap();
    let report = audit_recipe(&process.manifest, &review);
    assert_eq!(report.agent_steps, 0);
    assert_eq!(report.connector_steps, 1);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "pending_connection")
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == "unreviewed_agent_boundary")
    );
}

#[test]
fn routine_handoff_and_unbounded_output_are_advisory_findings() {
    let process = admit_json(include_bytes!(
        "../../../examples/software-verification/process.json"
    ))
    .unwrap();
    let review: ReviewInput = serde_json::from_value(json!({
        "steps": {"investigate": {
            "boundary": "service",
            "reason": "",
            "output": {"max_return_bytes": 0, "raw_artifact": false, "paged_or_aggregated": false, "revision_delta_includes_removals": false}
        }},
        "final_output": {"max_return_bytes": 4096, "raw_artifact": true, "repeated_guidance": true}
    })).unwrap();
    let report = audit_recipe(&process.manifest, &review);
    let codes: Vec<_> = report.findings.iter().map(|f| f.code.as_str()).collect();
    assert!(codes.contains(&"routine_agent_boundary"));
    assert!(codes.contains(&"missing_boundary_reason"));
    assert!(codes.contains(&"unbounded_output"));
    assert!(codes.contains(&"missing_call_inventory"));
    assert!(codes.contains(&"incomplete_context_revision"));
    assert!(report.advisory_only);
}

#[test]
fn justified_judgment_has_no_boundary_warning() {
    let process = admit_json(include_bytes!(
        "../../../examples/software-verification/process.json"
    ))
    .unwrap();
    let review: ReviewInput = serde_json::from_value(json!({
        "steps": {"investigate": {
            "boundary": "open_judgment",
            "reason": "The caller must weigh novel evidence against the user's acceptance criteria.",
            "output": {"max_return_bytes": 2048, "raw_artifact": true}
        }},
        "baseline": {"model_round_trips": 1, "raw_http_calls": 0, "retained_input_tokens": null, "retained_output_tokens": null},
        "expected": {"model_round_trips": 1, "raw_http_calls": 0, "retained_input_tokens": null, "retained_output_tokens": null},
        "final_output": {"max_return_bytes": 4096, "raw_artifact": true}
    })).unwrap();
    let report = audit_recipe(&process.manifest, &review);
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
}

#[test]
fn synthetic_connector_and_mixed_review_fixtures_admit() {
    let cases = [
        (
            include_bytes!("../../../examples/execution-boundary-review/service-process.json")
                .as_slice(),
            include_str!("../../../examples/execution-boundary-review/service-review.json"),
            0,
            2,
        ),
        (
            include_bytes!("../../../examples/execution-boundary-review/mixed-process.json")
                .as_slice(),
            include_str!("../../../examples/execution-boundary-review/mixed-review.json"),
            1,
            3,
        ),
        (
            include_bytes!("../../../examples/execution-boundary-review/context-process.json")
                .as_slice(),
            include_str!("../../../examples/execution-boundary-review/context-review.json"),
            0,
            0,
        ),
    ];
    for (bytes, review_json, agents, connectors) in cases {
        let process = admit_json(bytes).unwrap();
        let review: ReviewInput = serde_json::from_str(review_json).unwrap();
        let report = audit_recipe(&process.manifest, &review);
        assert_eq!(report.agent_steps, agents);
        assert_eq!(report.connector_steps, connectors);
        assert!(
            report
                .findings
                .iter()
                .all(|f| f.code == "pending_connection"),
            "{:#?}",
            report.findings
        );
    }
}
