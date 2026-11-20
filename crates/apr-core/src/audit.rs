//! Advisory execution-boundary review. This sidecar is deliberately separate
//! from admission: a static report cannot prove that a judgment is automatable.

use crate::ProcessManifest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewInput {
    #[serde(default)]
    pub steps: BTreeMap<String, StepReview>,
    #[serde(default)]
    pub connections: BTreeMap<String, ConnectionReview>,
    #[serde(default)]
    pub baseline: Option<CallInventory>,
    #[serde(default)]
    pub expected: Option<CallInventory>,
    #[serde(default)]
    pub one_time_authoring: Option<CallInventory>,
    #[serde(default)]
    pub final_output: Option<OutputReview>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    Deterministic,
    Service,
    BoundedClassification,
    OpenJudgment,
    AgentExclusiveTool,
    OwnerDecision,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepReview {
    pub boundary: BoundaryKind,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub output: Option<OutputReview>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputReview {
    pub max_return_bytes: u64,
    #[serde(default)]
    pub raw_artifact: bool,
    #[serde(default)]
    pub paged_or_aggregated: bool,
    #[serde(default)]
    pub repeated_guidance: bool,
    #[serde(default)]
    pub revision_delta_includes_removals: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    Approved,
    Pending,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionReview {
    pub access: AccessState,
    #[serde(default)]
    pub caller_has_service_tool: bool,
    #[serde(default)]
    pub proposal_ref: Option<String>,
}

/// Counts model boundaries separately from calls already combined in scripts
/// and raw HTTP traffic. Values are author observations or estimates, not
/// runtime telemetry or evidence of token savings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallInventory {
    pub model_round_trips: u64,
    pub raw_http_calls: u64,
    pub retained_input_tokens: Option<u64>,
    pub retained_output_tokens: Option<u64>,
    #[serde(default)]
    pub reused_context_tokens: Option<u64>,
    #[serde(default)]
    pub cached_input_tokens: Option<u64>,
    #[serde(default)]
    pub classifier_calls: Option<u64>,
    #[serde(default)]
    pub provider_calls: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Review,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewReport {
    pub advisory_only: bool,
    pub agent_steps: usize,
    pub connector_steps: usize,
    pub findings: Vec<Finding>,
}

/// Inspect explicit topology and author-supplied boundary notes. No credential
/// lookup, provider call, text keyword ban, or semantic classification occurs.
pub fn audit_recipe(manifest: &ProcessManifest, review: &ReviewInput) -> ReviewReport {
    let mut findings = Vec::new();
    let mut agent_steps = 0;
    let mut connector_steps = 0;
    let ids: BTreeSet<_> = manifest.steps.iter().map(|step| step.id.as_str()).collect();
    for (index, step) in manifest.steps.iter().enumerate() {
        let path = format!("/steps/{index}");
        let note = review.steps.get(&step.id);
        if step.kind == "agent" {
            agent_steps += 1;
            match note {
                None => finding(
                    &mut findings,
                    "unreviewed_agent_boundary",
                    &path,
                    "Record the concrete judgment or exclusive tool boundary and its return contract.",
                ),
                Some(note) => {
                    if matches!(
                        note.boundary,
                        BoundaryKind::Deterministic | BoundaryKind::Service
                    ) {
                        finding(
                            &mut findings,
                            "routine_agent_boundary",
                            &path,
                            "Review whether this routine work belongs in a runtime step with an approved helper or connection.",
                        );
                    }
                    if note.reason.trim().is_empty() {
                        finding(
                            &mut findings,
                            "missing_boundary_reason",
                            &path,
                            "State why this boundary needs the caller and cannot use an approved runtime helper or connection.",
                        );
                    }
                    inspect_output(
                        &mut findings,
                        &format!("{path}/output"),
                        note.output.as_ref(),
                    );
                }
            }
        }
        if step.kind == "connector" {
            connector_steps += 1;
            if let Some(connection) = step.connection.as_ref() {
                match review.connections.get(connection) {
                    Some(ConnectionReview {
                        access: AccessState::Pending,
                        proposal_ref,
                        ..
                    }) => {
                        findings.push(Finding {
                            code: "pending_connection".into(),
                            severity: Severity::Info,
                            path: path.clone(),
                            message: format!("Connection {connection} awaits owner setup{}; keep the executable step and report blocked readiness.", proposal_ref.as_ref().map(|id| format!(" ({id})")).unwrap_or_default()),
                        });
                    }
                    None => finding(
                        &mut findings,
                        "unreviewed_connection",
                        &path,
                        "Record whether this named connection is approved or pending owner setup.",
                    ),
                    _ => {}
                }
            }
        }
        if let Some(note) = note
            && step.kind != "agent"
            && matches!(
                note.boundary,
                BoundaryKind::OpenJudgment
                    | BoundaryKind::AgentExclusiveTool
                    | BoundaryKind::OwnerDecision
            )
        {
            finding(
                &mut findings,
                "boundary_mismatch",
                &path,
                "Review this declared judgment boundary against the actual runtime step.",
            );
        }
    }
    for id in review.steps.keys().filter(|id| !ids.contains(id.as_str())) {
        finding(
            &mut findings,
            "unknown_review_step",
            &format!("/review/steps/{id}"),
            "Review metadata names no step in this manifest.",
        );
    }
    inspect_output(&mut findings, "/output", review.final_output.as_ref());
    if review.baseline.is_none() || review.expected.is_none() {
        finding(
            &mut findings,
            "missing_call_inventory",
            "/review",
            "Record current and expected model round trips separately from raw HTTP calls; leave token fields unknown until measured.",
        );
    }
    ReviewReport {
        advisory_only: true,
        agent_steps,
        connector_steps,
        findings,
    }
}

fn inspect_output(findings: &mut Vec<Finding>, path: &str, output: Option<&OutputReview>) {
    match output {
        None => finding(
            findings,
            "missing_output_budget",
            path,
            "Declare a compact normal return budget and how full raw data remains retrievable.",
        ),
        Some(output) if output.max_return_bytes == 0 => finding(
            findings,
            "unbounded_output",
            path,
            "Set a positive normal return byte budget; retain full data through a bounded artifact or page path.",
        ),
        Some(output) if output.max_return_bytes > 64 * 1024 => finding(
            findings,
            "large_output_budget",
            path,
            "Review whether the normal caller result can be aggregated, paged or moved to an artifact.",
        ),
        _ => {}
    }
    if let Some(output) = output
        && output.repeated_guidance
        && !output.revision_delta_includes_removals
    {
        finding(
            findings,
            "incomplete_context_revision",
            path,
            "Repeated guidance needs stable revisions and deltas that explicitly report removals.",
        );
    }
}

fn finding(findings: &mut Vec<Finding>, code: &str, path: &str, message: &str) {
    findings.push(Finding {
        code: code.into(),
        severity: Severity::Review,
        path: path.into(),
        message: message.into(),
    });
}
