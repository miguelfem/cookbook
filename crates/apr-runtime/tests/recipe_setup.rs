use apr_connectors::{
    Auth, Connection,
    mcp::{McpBinding, ToolContract, Transport, pin_executable},
};
use apr_core::content_digest;
use apr_runtime::{Runtime, setup::ConnectionProposal};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    temp: tempfile::TempDir,
    runtime: Runtime,
    assignment: String,
    token: String,
    project: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut runtime = Runtime::open(temp.path().join("state")).unwrap();
        let value = runtime
            .owner_project_setup(&project, "recipe fixture")
            .unwrap();
        let assignment = value["assignment_id"].as_str().unwrap().to_string();
        let token = runtime.create_attachment(&assignment, "author").unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string();
        Self {
            temp,
            runtime,
            assignment,
            token,
            project,
        }
    }
    fn proposal(&mut self, connection: Connection, key: &str) -> Value {
        self.runtime.scoped_call(&self.token,"manage",json!({"action":"connection.propose","name":"normalize","request_key":key,"rationale":"Normalize an approved synthetic ingredient","connection":connection})).unwrap()
    }
    fn approve(&mut self, value: &Value) -> Value {
        self.runtime
            .approve_connection(
                value["request_id"].as_str().unwrap(),
                value["digest"].as_str().unwrap(),
                None,
            )
            .unwrap()
    }
    fn register(&mut self) -> String {
        self.register_with_calls(1)
    }
    fn register_with_calls(&mut self, calls: u32) -> String {
        let mut process: Value =
            serde_json::from_str(include_str!("../../../examples/http-json/process.json")).unwrap();
        process["name"] = json!("recipes.normalize");
        process["limits"]["max_provider_calls"] = json!(calls);
        process["capabilities"][0]["name"] = json!("normalize");
        process["steps"][0]["operation"] = json!("mcp.call");
        process["steps"][0]["connection"] = json!("normalize");
        process["steps"][0]["inputs"] = json!({"value":{"input":"/value"}});
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
            .to_string()
    }
    fn connection(&self, mode: &str) -> Connection {
        let output = std::process::Command::new("python3")
            .args(["-I", "-c", "import sys; print(sys.executable)"])
            .output()
            .unwrap();
        let python = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
            .canonicalize()
            .unwrap();
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/mcp/server.py")
            .canonicalize()
            .unwrap();
        let input = json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false});
        let output = json!({"type":"object"});
        let metering=serde_json::from_value(json!({"provider":"fixture","revision":"recipe-v1","currency":"USD","analysis":{"recipe":"recipes.normalize","request_schema_digest":content_digest(&input).unwrap(),"response_schema_digest":content_digest(&output).unwrap(),"pricing_sources":["https://example.org/synthetic-pricing"],"checked_at":"2026-09-17","limitations":["Synthetic rates, not live pricing"]},"meters":[{"name":"planned_units","unit":"items","add":["request:/value"],"rate":{"amount":"0.01","per_units":1}},{"name":"calls","unit":"requests","add":["response:/_meta/usage/requests"],"rate":{"amount":"0.10","per_units":1}}]})).unwrap();
        Connection {
            label: "synthetic normalization".into(),
            credential_label: None,
            endpoint: "stdio:local".into(),
            method: "POST".into(),
            auth: Auth::None,
            input_schema: input.clone(),
            output_schema: output,
            metering: Some(metering),
            timeout_ms: 2000,
            max_response_bytes: 16384,
            loopback_test: false,
            mcp: Some(McpBinding {
                sampling_worker: None,
                feature: None,
                interaction: None,
                protocol: "2026-07-28".into(),
                transport: Transport::Stdio {
                    executable: python.to_string_lossy().into(),
                    sha256: pin_executable(&python).unwrap(),
                    args: vec![
                        "-I".into(),
                        "-u".into(),
                        script.to_string_lossy().into(),
                        mode.into(),
                        self.temp.path().join("calls").to_string_lossy().into(),
                    ],
                    credential_env: None,
                },
                tool: Some(ToolContract {
                    ui_resource_uri: None,
                    name: "normalize".into(),
                    input_schema: input,
                    output_schema: Some(
                        json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}}),
                    ),
                }),
            }),
        }
    }
}
#[test]
fn recipe_authoring_proposes_costs_without_granting_access() {
    let mut f = Fixture::new();
    let cfg = f.connection("ok");
    let preview=f.runtime.scoped_call(&f.token,"manage",json!({"action":"cost.preview","connection":cfg,"request":{"value":3},"response":{"_meta":{"usage":{"requests":1}}}})).unwrap();
    assert_eq!(preview["observation"]["estimated_nanos"], "130000000");
    assert_eq!(preview["external_calls"], 0);
    let proposal = f.proposal(cfg.clone(), "request-1");
    assert_eq!(proposal["granted"], false);
    assert!(!f.temp.path().join("calls").exists());
    let replay = f.proposal(cfg, "request-1");
    assert_eq!(proposal, replay);
    assert!(
        f.runtime
            .approve_connection(proposal["request_id"].as_str().unwrap(), "wrong", None)
            .is_err()
    );
    let view = f
        .runtime
        .scoped_call(&f.token, "inspect", json!({"view":"approvals"}))
        .unwrap();
    assert_eq!(view["requests"][0]["digest"], proposal["digest"]);
    for args in [
        json!({"action":"connection.approve","request_id":proposal["request_id"]}),
        json!({"action":"connection.propose","approved":true}),
    ] {
        assert!(f.runtime.scoped_call(&f.token, "manage", args).is_err());
    }
    let version = f.register();
    let readiness = f
        .runtime
        .scoped_call(
            &f.token,
            "inspect",
            json!({"view":"readiness","process_ref":version}),
        )
        .unwrap();
    assert_eq!(readiness["can_finish_unattended"], false);
    assert!(f.runtime.scoped_call(&f.token,"run",json!({"process_ref":version,"input":{"value":3},"mode":"start","request_key":"denied"})).is_err());
}
#[test]
fn owner_approval_runs_one_mcp_effect_with_frozen_recipe_cost_and_replay() {
    let mut f = Fixture::new();
    let cfg = f.connection("ok");
    let request = f.proposal(cfg, "first");
    let receipt = f.approve(&request);
    assert_eq!(f.approve(&request), receipt);
    let version = f.register();
    let ready = f
        .runtime
        .scoped_call(
            &f.token,
            "inspect",
            json!({"view":"readiness","process_ref":version}),
        )
        .unwrap();
    assert_eq!(ready["can_finish_unattended"], true);
    let args =
        json!({"process_ref":version,"input":{"value":3},"mode":"start","request_key":"one-call"});
    let run = f
        .runtime
        .scoped_call(&f.token, "run", args.clone())
        .unwrap();
    f.runtime.drive_pending().unwrap();
    let id = run["run_id"].as_str().unwrap();
    assert_eq!(
        f.runtime.inspect(id).unwrap().state,
        "succeeded",
        "{:?}",
        f.runtime.inspect(id).unwrap()
    );
    let stats = f
        .runtime
        .scoped_call(&f.token, "inspect", json!({"view":"analytics","run_id":id}))
        .unwrap();
    assert_eq!(stats["calls"].as_array().unwrap().len(), 1);
    assert_eq!(
        stats["calls"][0]["usage"]["observation"]["estimated_nanos"],
        "130000000"
    );
    assert_eq!(
        std::fs::read_to_string(f.temp.path().join("calls"))
            .unwrap()
            .matches("tools/call")
            .count(),
        1
    );
    drop(f.runtime);
    f.runtime = Runtime::open(f.temp.path().join("state")).unwrap();
    let replay = f.runtime.scoped_call(&f.token, "run", args).unwrap();
    assert_eq!(replay["run_id"], run["run_id"]);
    f.runtime.drive_pending().unwrap();
    assert_eq!(
        std::fs::read_to_string(f.temp.path().join("calls"))
            .unwrap()
            .matches("tools/call")
            .count(),
        1
    );
}
#[test]
fn scoped_proposals_and_stale_approvals_cannot_expand_authority() {
    let mut f = Fixture::new();
    let request = f.proposal(f.connection("ok"), "one");
    let second = f.proposal(f.connection("ok"), "two");
    let other = f.runtime.owner_project_setup(&f.project, "other").unwrap()["assignment_id"]
        .as_str()
        .unwrap()
        .to_string();
    let token = f.runtime.create_attachment(&other, "author").unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        f.runtime
            .scoped_call(&token, "inspect", json!({"view":"approvals"}))
            .unwrap()["requests"],
        json!([])
    );
    let run_only = f.runtime.create_attachment(&f.assignment, "run").unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(f.runtime.scoped_call(&run_only,"manage",json!({"action":"connection.propose","name":"bad","request_key":"bad","rationale":"bad","connection":f.connection("ok")})).is_err());
    f.approve(&request);
    assert!(
        f.runtime
            .approve_connection(
                second["request_id"].as_str().unwrap(),
                second["digest"].as_str().unwrap(),
                None
            )
            .is_err()
    );
    let mut cfg = f.connection("ok");
    cfg.auth = Auth::Bearer {
        credential_ref: "not-a-placeholder".into(),
    };
    let proposal = ConnectionProposal {
        name: "unsafe".into(),
        request_key: "unsafe".into(),
        rationale: "unsafe fixture".into(),
        connection: cfg,
    };
    assert!(
        f.runtime
            .propose_connection(&f.assignment, proposal)
            .is_err()
    );
}
#[test]
fn changed_payload_schema_requires_reanalysis_and_uncertain_tool_is_not_replayed() {
    let mut f = Fixture::new();
    let mut cfg = f.connection("error");
    cfg.input_schema["properties"]["new"] = json!({"type":"string"});
    assert!(cfg.validate().is_err());
    let request = f.proposal(f.connection("error"), "uncertain");
    f.approve(&request);
    let version = f.register();
    let run=f.runtime.scoped_call(&f.token,"run",json!({"process_ref":version,"input":{"value":1},"mode":"start","request_key":"uncertain"})).unwrap();
    f.runtime.drive_pending().unwrap();
    let id = run["run_id"].as_str().unwrap();
    assert_eq!(f.runtime.inspect(id).unwrap().state, "needs_reconciliation");
    f.runtime.drive_pending().unwrap();
    assert!(f.runtime.rebind_execution(id).is_err());
    assert_eq!(
        std::fs::read_to_string(f.temp.path().join("calls"))
            .unwrap()
            .matches("tools/call")
            .count(),
        1
    );
}

#[test]
fn mcp_reconciliation_rejects_wrong_structured_content() {
    mcp_reconciliation_case("structured_content");
}

#[test]
fn mcp_reconciliation_rejects_tool_errors() {
    mcp_reconciliation_case("tool_error");
}

#[test]
fn mcp_reconciliation_rejects_input_required_as_completion() {
    mcp_reconciliation_case("input_required");
}

#[test]
fn mcp_reconciliation_rejects_wrong_resource_uri() {
    mcp_reconciliation_case("resource_uri");
}

fn mcp_reconciliation_case(invalid_kind: &str) {
    let mut f = Fixture::new();
    let mut cfg = f.connection("error");
    if invalid_kind == "resource_uri" {
        cfg.mcp.as_mut().unwrap().tool = None;
        cfg.mcp.as_mut().unwrap().feature = Some(apr_connectors::mcp::FeatureContract::Resource {
            uri: "fixture://approved".into(),
        });
        cfg.input_schema = json!({"type":"object","additionalProperties":false});
        cfg.metering.as_mut().unwrap().analysis = None;
    }
    let proposal = f.proposal(cfg, "reconciliation-contract");
    f.approve(&proposal);
    let mut version = f.register();
    if invalid_kind == "resource_uri" {
        let mut recipe: Value =
            serde_json::from_slice(&std::fs::read(f.project.join("recipe.json")).unwrap()).unwrap();
        recipe["steps"][0]["inputs"] = json!({});
        std::fs::write(
            f.project.join("resource.json"),
            serde_json::to_vec(&recipe).unwrap(),
        )
        .unwrap();
        version = f
            .runtime
            .scoped_call(
                &f.token,
                "manage",
                json!({"action":"process.register","path":"resource.json"}),
            )
            .unwrap()["process_ref"]
            .as_str()
            .unwrap()
            .into();
    }
    let run=f.runtime.scoped_call(&f.token,"run",json!({"process_ref":version,"input":{"value":1},"mode":"start","request_key":"reconcile"})).unwrap();
    f.runtime.drive_pending().unwrap();
    let run = run["run_id"].as_str().unwrap();
    assert_eq!(
        f.runtime.inspect(run).unwrap().state,
        "needs_reconciliation"
    );
    let effects = f
        .runtime
        .scoped_call(&f.token, "inspect", json!({"view":"effects","run_id":run}))
        .unwrap();
    let complete = if invalid_kind == "resource_uri" {
        json!({"resultType":"complete","contents":[{"uri":"fixture://approved","text":"Observed synthetic record"}],"_meta":{"usage":{"requests":1}}})
    } else {
        json!({"resultType":"complete","structuredContent":{"value":1},"content":[{"type":"text","text":"Observed synthetic result"}],"_meta":{"usage":{"requests":1}}})
    };
    let mut invalid = complete.clone();
    match invalid_kind {
        "structured_content" => invalid["structuredContent"]["value"] = json!("wrong-type"),
        "tool_error" => invalid["isError"] = json!(true),
        "input_required" => invalid["resultType"] = json!("input_required"),
        "resource_uri" => invalid["contents"][0]["uri"] = json!("fixture://unapproved"),
        _ => unreachable!(),
    }
    let mut request = json!({"action":"reconcile","run_id":run,"step_instance_id":"request","intent_id":effects["intents"][0]["intent_id"],"outcome":"observed_result","evidence":"Synthetic provider record for this exact attempt","output":invalid});
    assert!(
        f.runtime
            .scoped_call(&f.token, "resume", request.clone())
            .is_err(),
        "invalid {invalid_kind} was accepted as completed MCP work"
    );
    assert_eq!(
        f.runtime.inspect(run).unwrap().state,
        "needs_reconciliation"
    );
    request["output"] = complete;
    f.runtime.scoped_call(&f.token, "resume", request).unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(f.runtime.inspect(run).unwrap().state, "succeeded");
    let analytics = f
        .runtime
        .scoped_call(
            &f.token,
            "inspect",
            json!({"view":"analytics","run_id":run}),
        )
        .unwrap();
    assert_eq!(analytics["summary"]["external_attempts"], 1);
    assert_eq!(
        analytics["calls"][0]["usage"]["provenance"],
        "caller_reported"
    );
    assert_eq!(
        analytics["calls"][0]["usage"]["observation"]["estimated_nanos"],
        if invalid_kind == "resource_uri" {
            "100000000"
        } else {
            "110000000"
        }
    );
}

#[test]
fn owner_attachment_is_private_and_survives_reopen() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    let path = f.runtime.owner_attachment_file(&f.assignment).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
        0
    );
    let client = apr_runtime::service::ServiceClient::from_file(&path).unwrap();
    assert_eq!(client.assignment_id(), f.assignment);
}

#[test]
fn durable_mcp_form_requires_owner_and_resumes_once_after_restart() {
    let mut f = Fixture::new();
    let mut cfg = f.connection("mrtr");
    cfg.mcp.as_mut().unwrap().interaction =
        Some(apr_connectors::mcp::interaction::InteractionPolicy {
            sampling: false,
            form: true,
            url: false,
            max_rounds: 4,
        });
    let proposal = f.proposal(cfg, "interactive");
    f.approve(&proposal);
    let recipe = f.register_with_calls(4);
    let run=f.runtime.scoped_call(&f.token,"run",json!({"process_ref":recipe,"input":{"value":3},"request_key":"interactive-run","mode":"start"})).unwrap();
    f.runtime.drive_pending().unwrap();
    let id = run["run_id"].as_str().unwrap();
    assert_eq!(f.runtime.inspect(id).unwrap().state, "blocked");
    let first_round = f
        .runtime
        .scoped_call(&f.token, "inspect", json!({"view":"analytics","run_id":id}))
        .unwrap();
    let usage = &first_round["calls"][0]["usage"];
    assert_eq!(usage["provenance"], "broker_observed");
    assert_eq!(usage["observation"]["estimated_nanos"], "30000000");
    assert_eq!(usage["observation"]["estimate_status"], "partial");
    assert_eq!(usage["observation"]["meters"][0]["basis"], "request");
    let first_period = f
        .runtime
        .billing_period(&f.assignment, 0, i64::MAX)
        .unwrap();
    assert_eq!(first_period["calls"], 1);
    assert_eq!(first_period["groups"][0]["estimated_nanos"], "30000000");
    assert_eq!(first_period["groups"][0]["unknown_or_partial"], 1);
    let pending = f
        .runtime
        .scoped_call(&f.token, "inspect", json!({"view":"mcp_interactions"}))
        .unwrap();
    assert!(
        !pending
            .to_string()
            .contains("opaque-state-private-sentinel")
    );
    let interaction = pending["items"][0]["interaction_id"]
        .as_str()
        .unwrap()
        .to_string();
    let data = json!({"interaction_id":interaction,"request_key":"respond-once","responses":{"review":{"action":"accept","content":{"factor":2}}}});
    let mut model = data.clone();
    model["action"] = json!("mcp.respond");
    assert!(f.runtime.scoped_call(&f.token, "resume", model).is_err());
    f.runtime
        .owner_respond_mcp(&f.assignment, serde_json::from_value(data.clone()).unwrap())
        .unwrap();
    drop(f.runtime);
    f.runtime = Runtime::open(f.temp.path().join("state")).unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(
        f.runtime.inspect(id).unwrap().state,
        "succeeded",
        "{:?}",
        f.runtime.inspect(id).unwrap()
    );
    f.runtime
        .owner_respond_mcp(&f.assignment, serde_json::from_value(data).unwrap())
        .unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(
        std::fs::read_to_string(f.temp.path().join("calls"))
            .unwrap()
            .matches("tools/call")
            .count(),
        2
    );
    let period = f
        .runtime
        .billing_period(&f.assignment, 0, i64::MAX)
        .unwrap();
    assert_eq!(period["calls"], 2);
    assert_eq!(period["groups"][0]["estimated_calls"], 2);
    assert_eq!(period["groups"][0]["estimated_nanos"], "160000000");
    assert_eq!(period["groups"][0]["unknown_or_partial"], 1);
}
#[test]
fn budget_reservation_is_durable_and_blocks_a_second_effect() {
    use apr_runtime::billing::BudgetPolicy;
    let mut f = Fixture::new();
    let req = f.proposal(f.connection("ok"), "metered");
    f.approve(&req);
    let recipe = f.register();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    f.runtime
        .set_billing_budget(
            &f.assignment,
            BudgetPolicy {
                currency: "USD".into(),
                starts_at_ms: now - 60000,
                ends_at_ms: now + 60000,
                limit_nanos: "200000000".into(),
                per_connection_max_nanos: [("normalize".into(), "150000000".into())].into(),
            },
        )
        .unwrap();
    let first=f.runtime.scoped_call(&f.token,"run",json!({"process_ref":recipe,"input":{"value":3},"request_key":"first-budget","mode":"start"})).unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(
        f.runtime
            .inspect(first["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "succeeded"
    );
    drop(f.runtime);
    f.runtime = Runtime::open(f.temp.path().join("state")).unwrap();
    let second=f.runtime.scoped_call(&f.token,"run",json!({"process_ref":recipe,"input":{"value":3},"request_key":"second-budget","mode":"start"})).unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(
        f.runtime
            .inspect(second["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "blocked"
    );
    assert_eq!(
        std::fs::read_to_string(f.temp.path().join("calls"))
            .unwrap()
            .matches("tools/call")
            .count(),
        1
    );
    let totals = f
        .runtime
        .scoped_call(
            &f.token,
            "inspect",
            json!({"view":"billing","starts_at_ms":now-60000,"ends_at_ms":now+60000}),
        )
        .unwrap();
    assert_eq!(totals["scope"], "whole_period");
    assert_eq!(totals["groups"][0]["estimated_nanos"], "130000000");
    let ledger = f
        .runtime
        .scoped_call(
            &f.token,
            "inspect",
            json!({"view":"analytics","run_id":first["run_id"]}),
        )
        .unwrap();
    let intent = ledger["calls"][0]["intent_id"].clone();
    let invoice = json!({"invoice_id":"invoice-one","account_label":"synthetic-account","connection_name":"normalize","currency":"USD","starts_at_ms":now-60000,"ends_at_ms":now+60000,"lines":[{"line_id":"one","intent_id":intent,"amount_nanos":"130000000"}]});
    f.runtime
        .import_invoice(
            &f.assignment,
            serde_json::from_value(invoice.clone()).unwrap(),
        )
        .unwrap();
    f.runtime
        .import_invoice(
            &f.assignment,
            serde_json::from_value(invoice.clone()).unwrap(),
        )
        .unwrap();
    let mut changed = invoice;
    changed["lines"][0]["amount_nanos"] = json!("1");
    assert!(
        f.runtime
            .import_invoice(&f.assignment, serde_json::from_value(changed).unwrap())
            .is_err()
    );
    let totals = f
        .runtime
        .billing_period(&f.assignment, now - 60000, now + 60000)
        .unwrap();
    assert_eq!(totals["groups"][0]["invoice_matched_nanos"], "130000000");
    assert_eq!(totals["calls"], 1);
}
#[test]
fn mixed_mcp_inputs_keep_human_consent_separate_from_agent_sampling() {
    let mut f = Fixture::new();
    let mut cfg = f.connection("mixed");
    cfg.mcp.as_mut().unwrap().interaction =
        Some(apr_connectors::mcp::interaction::InteractionPolicy {
            sampling: true,
            form: true,
            url: false,
            max_rounds: 4,
        });
    let p = f.proposal(cfg, "mixed");
    f.approve(&p);
    let recipe = f.register_with_calls(4);
    let run=f.runtime.scoped_call(&f.token,"run",json!({"process_ref":recipe,"input":{"value":3},"request_key":"mixed-run","mode":"start"})).unwrap();
    f.runtime.drive_pending().unwrap();
    let waiting = f.runtime.mcp_interactions(&f.assignment, None).unwrap();
    let id = waiting["items"][0]["interaction_id"].as_str().unwrap();
    let request = json!({"action":"mcp.respond","interaction_id":id,"request_key":"sample-once","responses":{"model":{"role":"assistant","content":{"type":"text","text":"Observed synthetic summary"},"model":"fixture","stopReason":"endTurn"}}});
    let answer = f
        .runtime
        .scoped_call(&f.token, "resume", request.clone())
        .unwrap();
    assert_eq!(answer["complete"], false);
    f.runtime.scoped_call(&f.token, "resume", request).unwrap();
    let pending = f.runtime.mcp_interactions(&f.assignment, None).unwrap();
    assert!(pending["items"][0]["input_requests"].get("model").is_none());
    assert_eq!(
        f.runtime
            .inspect(run["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "blocked"
    );
    f.runtime.owner_respond_mcp(&f.assignment,serde_json::from_value(json!({"interaction_id":id,"request_key":"human-once","responses":{"review":{"action":"accept","content":{"factor":2}}}})).unwrap()).unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(
        f.runtime
            .inspect(run["run_id"].as_str().unwrap())
            .unwrap()
            .state,
        "succeeded"
    );
    let stats = f
        .runtime
        .scoped_call(&f.token, "inspect", json!({"view":"analytics"}))
        .unwrap();
    assert_eq!(stats["summary"]["retries"], 0);
}
#[test]
fn whole_period_billing_survives_retention_and_update_gate_survives_restart() {
    let mut f = Fixture::new();
    let p = f.proposal(f.connection("ok"), "billing-retention");
    f.approve(&p);
    let recipe = f.register();
    f.runtime
        .scoped_call(
            &f.token,
            "run",
            json!({"process_ref":recipe,"input":{"value":1},"request_key":"retain","mode":"start"}),
        )
        .unwrap();
    f.runtime.drive_pending().unwrap();
    let before = f
        .runtime
        .billing_period(&f.assignment, 0, i64::MAX)
        .unwrap();
    assert_eq!(before["calls"], 1);
    std::thread::sleep(std::time::Duration::from_millis(3));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let plan = f.runtime.retention_preview(now, now - 60001).unwrap();
    let removed = f.runtime.retention_apply(&plan).unwrap();
    assert_eq!(removed.removed_runs, 1);
    assert_eq!(
        before,
        f.runtime
            .billing_period(&f.assignment, 0, i64::MAX)
            .unwrap()
    );
    f.runtime.prepare_desktop_update("0.2.0").unwrap();
    drop(f.runtime);
    f.runtime = Runtime::open(f.temp.path().join("state")).unwrap();
    assert_eq!(
        f.runtime.maintenance_status().unwrap()["paused_for_update"],
        true
    );
    assert!(f.runtime.scoped_call(&f.token,"run",json!({"process_ref":recipe,"input":{"value":1},"request_key":"held","mode":"start"})).is_err());
    f.runtime
        .backup_to(&f.temp.path().join("update-backup"))
        .unwrap();
    f.runtime.finish_desktop_update().unwrap();
    assert_eq!(
        f.runtime.maintenance_status().unwrap()["paused_for_update"],
        false
    );
    let mut another = Fixture::new();
    assert_eq!(
        another
            .runtime
            .scoped_call(
                &another.token,
                "inspect",
                json!({"view":"billing","starts_at_ms":0,"ends_at_ms":i64::MAX})
            )
            .unwrap()["calls"],
        0
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn explicit_api_worker_fulfills_sampling_once_across_restart_and_shares_call_limits() {
    sampling_worker_case(false, 0);
}

#[test]
fn lost_sampling_worker_response_requires_evidence_and_never_repeats_inference() {
    sampling_worker_case(true, 0);
}

#[cfg(feature = "fault-injection")]
#[test]
fn sampling_worker_advances_past_a_full_page_of_owner_waits() {
    sampling_worker_case(false, 25);
}

fn sampling_worker_case(lose_reply: bool, prior_owner_waits: usize) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/model", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut b = [0u8; 4096];
        loop {
            let n = stream.read(&mut b).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&b[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let len = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + len {
                    let body: Value =
                        serde_json::from_slice(&bytes[end + 4..end + 4 + len]).unwrap();
                    assert_eq!(body["model"], "fixture-model");
                    assert_eq!(body["max_tokens"], 100);
                    break;
                }
            }
        }
        if lose_reply {
            return;
        }
        let body = r#"{"model":"fixture-model","text":"Observed synthetic reasoning","finish":"stop","usage":{"tokens":4}}"#;
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let mut f = Fixture::new();
    let mut owner_runs = Vec::new();
    if prior_owner_waits > 0 {
        let mut cfg = f.connection("mrtr");
        cfg.mcp.as_mut().unwrap().interaction =
            Some(serde_json::from_value(json!({"form":true,"max_rounds":3})).unwrap());
        let proposal = f.proposal(cfg, "earlier-owner-waits");
        f.approve(&proposal);
        let recipe = f.register_with_calls(2);
        for number in 0..prior_owner_waits {
            let run=f.runtime.scoped_call(&f.token,"run",json!({"process_ref":recipe,"input":{"value":1},"request_key":format!("owner-wait-{number}"),"mode":"start"})).unwrap();
            f.runtime.drive_pending().unwrap();
            let run = run["run_id"].as_str().unwrap().to_string();
            assert_eq!(f.runtime.inspect(&run).unwrap().state, "blocked");
            owner_runs.push(run);
        }
    }
    let model:Connection=serde_json::from_value(json!({"label":"approved synthetic model","endpoint":endpoint,"method":"POST","auth":{"kind":"none"},"input_schema":{"type":"object"},"output_schema":{"type":"object"},"timeout_ms":3000,"max_response_bytes":16000,"loopback_test":true})).unwrap();
    f.runtime
        .configure_connection(&f.assignment, "model", model, None)
        .unwrap();
    let mut cfg = f.connection("sampling");
    let m = cfg.mcp.as_mut().unwrap();
    m.interaction = Some(serde_json::from_value(json!({"sampling":true,"max_rounds":3})).unwrap());
    m.sampling_worker=Some(serde_json::from_value(json!({"connection":"model","model":"fixture-model","max_tokens":128,"request_template":{"store":false},"messages_key":"messages","max_tokens_key":"max_tokens","text_pointer":"/text","model_pointer":"/model","stop_reason_pointer":"/finish","stop_reason_map":{"stop":"endTurn","length":"maxTokens"}})).unwrap());
    let request = f.proposal(cfg, "automatic-worker");
    f.approve(&request);
    f.register_with_calls(3);
    let mut recipe: Value =
        serde_json::from_slice(&std::fs::read(f.project.join("recipe.json")).unwrap()).unwrap();
    recipe["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"connection","name":"model"}));
    std::fs::write(
        f.project.join("worker.json"),
        serde_json::to_vec(&recipe).unwrap(),
    )
    .unwrap();
    let version = f
        .runtime
        .scoped_call(
            &f.token,
            "manage",
            json!({"action":"process.register","path":"worker.json"}),
        )
        .unwrap()["process_ref"]
        .clone();
    let start=f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":version,"input":{"value":2},"request_key":"worker-run"})).unwrap();
    let run = start["run_id"].as_str().unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(f.runtime.inspect(run).unwrap().state, "blocked");
    // Older human waits must remain waiting while bounded worker ticks reach
    // independent authorized work beyond the first page.
    let tick_worker = |runtime: &mut Runtime| {
        (0..=prior_owner_waits / 20).any(|_| runtime.tick_sampling_workers().unwrap())
    };
    assert!(
        tick_worker(&mut f.runtime),
        "eligible sampling worker was starved by owner waits"
    );
    server.join().unwrap();
    drop(f.runtime);
    f.runtime = Runtime::open(f.temp.path().join("state")).unwrap();
    if lose_reply {
        assert!(!f.runtime.tick_sampling_workers().unwrap());
        let waiting = f.runtime.mcp_interactions(&f.assignment, None).unwrap();
        let job = &waiting["items"][0]["sampling_jobs"][0];
        assert_eq!(job["state"], "uncertain");
        assert_eq!(
            waiting["items"][0]["sampling_worker"]["connection"],
            "model"
        );
        let result = json!({"model":"fixture-model","text":"Observed synthetic reasoning","finish":"stop","usage":{"tokens":4}});
        let receipt: apr_runtime::execution::Reconciliation = serde_json::from_value(json!({"run_id":run,"step_instance_id":"request","intent_id":job["intent_id"],"outcome":"observed_result","evidence":"Synthetic provider journal verifies this exact attempt","output":result})).unwrap();
        f.runtime.reconcile(&receipt).unwrap();
    }
    assert!(tick_worker(&mut f.runtime));
    f.runtime.drive_pending().unwrap();
    assert_eq!(f.runtime.inspect(run).unwrap().state, "succeeded");
    assert!(!f.runtime.tick_sampling_workers().unwrap());
    let ledger = f
        .runtime
        .billing_period(&f.assignment, 0, i64::MAX)
        .unwrap();
    assert_eq!(ledger["calls"], 3 + prior_owner_waits);
    for run in owner_runs {
        assert_eq!(f.runtime.inspect(&run).unwrap().state, "blocked");
    }
}

#[test]
fn app_view_requires_completed_matching_tool_and_ui_resource_in_same_scope() {
    let mut f = Fixture::new();
    let mut cfg = f.connection("app");
    cfg.mcp
        .as_mut()
        .unwrap()
        .tool
        .as_mut()
        .unwrap()
        .ui_resource_uri = Some("ui://fixture/dashboard".into());
    let req = f.proposal(cfg.clone(), "app-tool");
    f.approve(&req);
    cfg.mcp.as_mut().unwrap().tool = None;
    cfg.mcp.as_mut().unwrap().feature = Some(apr_connectors::mcp::FeatureContract::Resource {
        uri: "ui://fixture/dashboard".into(),
    });
    cfg.input_schema = json!({"type":"object","additionalProperties":false});
    cfg.metering = None;
    f.runtime
        .configure_connection(&f.assignment, "app-ui", cfg, None)
        .unwrap();
    f.register_with_calls(2);
    let mut recipe: Value =
        serde_json::from_slice(&std::fs::read(f.project.join("recipe.json")).unwrap()).unwrap();
    recipe["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"connection","name":"app-ui"}));
    let mut resource = recipe["steps"][0].clone();
    resource["id"] = json!("app_resource");
    resource["connection"] = json!("app-ui");
    resource["inputs"] = json!({});
    resource["needs"] = json!(["request"]);
    recipe["steps"].as_array_mut().unwrap().push(resource);
    std::fs::write(
        f.project.join("app.json"),
        serde_json::to_vec(&recipe).unwrap(),
    )
    .unwrap();
    let version = f
        .runtime
        .scoped_call(
            &f.token,
            "manage",
            json!({"action":"process.register","path":"app.json"}),
        )
        .unwrap()["process_ref"]
        .clone();
    let start=f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":version,"input":{"value":2},"request_key":"app-run"})).unwrap();
    let run = start["run_id"].as_str().unwrap();
    f.runtime.drive_pending().unwrap();
    f.runtime.drive_pending().unwrap();
    assert_eq!(
        f.runtime.inspect(run).unwrap().state,
        "succeeded",
        "{:?}",
        f.runtime.inspect(run).unwrap()
    );
    let view=f.runtime.scoped_call(&f.token,"inspect",json!({"view":"mcp_app","run_id":run,"resource_step_id":"app_resource","tool_step_id":"request"})).unwrap();
    assert_eq!(view["read_only"], true);
    assert_eq!(view["host_tools"], false);
    assert!(view["html"].as_str().unwrap().contains("Synthetic App"));
    assert!(f.runtime.scoped_call(&f.token,"inspect",json!({"view":"mcp_app","run_id":run,"resource_step_id":"request","tool_step_id":"app_resource"})).is_err());
    let mut other = Fixture::new();
    assert!(other.runtime.scoped_call(&other.token,"inspect",json!({"view":"mcp_app","run_id":run,"resource_step_id":"app_resource","tool_step_id":"request"})).is_err());
}
#[test]
fn owner_invoice_can_reconcile_an_unpriced_call_without_inventing_an_estimate() {
    let mut f = Fixture::new();
    let mut cfg = f.connection("ok");
    cfg.metering = None;
    let request = f.proposal(cfg, "no-price");
    f.approve(&request);
    let version = f.register();
    f.runtime.scoped_call(&f.token,"run",json!({"mode":"start","process_ref":version,"input":{"value":1},"request_key":"unpriced"})).unwrap();
    f.runtime.drive_pending().unwrap();
    let analytics = f
        .runtime
        .scoped_call(&f.token, "inspect", json!({"view":"analytics"}))
        .unwrap();
    let intent = analytics["calls"][0]["intent_id"].clone();
    let invoice = json!({"invoice_id":"priced-after","account_label":"test-account","connection_name":"normalize","currency":"USD","starts_at_ms":0,"ends_at_ms":i64::MAX,"lines":[{"line_id":"line-1","intent_id":intent,"amount_nanos":"200000000"}]});
    f.runtime
        .import_invoice(&f.assignment, serde_json::from_value(invoice).unwrap())
        .unwrap();
    let period = f
        .runtime
        .billing_period(&f.assignment, 0, i64::MAX)
        .unwrap();
    assert!(period["groups"][0]["estimated_nanos"].is_null());
    assert_eq!(period["groups"][0]["invoice_matched_nanos"], "200000000");
    assert_eq!(period["calls"], 1);
}
