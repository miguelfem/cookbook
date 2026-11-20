//! Durable MCP MRTR waits. Server requestState is stored privately and never
//! exposed to agents or editable UI. User input approval remains owner-only.
use crate::store::{StoreLock, event, now_ms};
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::mcp::interaction::{Continuation, InteractionPolicy};
use apr_core::content_digest;
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) const MIGRATION_V13: &str = r#"
CREATE TABLE mcp_interactions (
 id TEXT PRIMARY KEY,run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
 step_id TEXT NOT NULL,assignment_id TEXT NOT NULL,round INTEGER NOT NULL,
 request_digest TEXT NOT NULL,binding_snapshot TEXT NOT NULL,response TEXT NOT NULL,
 state TEXT NOT NULL,reply TEXT,reply_key TEXT,reply_digest TEXT,continued_by TEXT,created_at INTEGER NOT NULL
);
CREATE INDEX mcp_interaction_scope ON mcp_interactions(assignment_id,state,id);
CREATE TABLE mcp_input_receipts(interaction_id TEXT NOT NULL REFERENCES mcp_interactions(id) ON DELETE CASCADE, request_key TEXT NOT NULL,digest TEXT NOT NULL,owner INTEGER NOT NULL,PRIMARY KEY(interaction_id,request_key));
"#;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionResponse {
    pub interaction_id: String,
    pub request_key: String,
    pub responses: Value,
}
fn invalid() -> RuntimeError {
    RuntimeError::InvalidInput("MCP interaction response does not match the saved request".into())
}
fn policy(binding: &Value) -> Result<InteractionPolicy> {
    serde_json::from_value(binding["mcp"]["interaction"].clone()).map_err(|_| invalid())
}
fn compatible(left: &Value, right: &Value) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    for v in [&mut left, &mut right] {
        if v["auth"].get("credential_ref").is_some() {
            v["auth"]["credential_ref"] = json!("owner-reviewed-rebinding");
        }
    }
    left == right
}
impl Runtime {
    pub fn mcp_interactions(&self, assignment: &str, after: Option<&str>) -> Result<Value> {
        if after.is_some_and(|s| s.len() > 128) {
            return Err(invalid());
        }
        let mut query=self.db.prepare("SELECT i.id,i.run_id,i.step_id,i.round,i.response,r.state,i.reply,i.binding_snapshot FROM mcp_interactions i JOIN runs r ON r.id=i.run_id WHERE i.assignment_id=?1 AND i.state='waiting' AND r.state NOT IN ('cancelled','failed','succeeded') AND i.id>?2 ORDER BY i.id LIMIT 21")?;
        let mut items = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        for row in query.query_map(params![assignment, after.unwrap_or("")], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
            ))
        })? {
            let (id, run, step, round, response, state, reply, binding) = row?;
            let mut value: Value = serde_json::from_str(&response)?;
            let answered: Value = reply
                .map(|s| serde_json::from_str(&s))
                .transpose()?
                .unwrap_or(json!({}));
            if let Some(requests) = value["inputRequests"].as_object_mut() {
                requests.retain(|id, _| answered.get(id).is_none());
            }
            let binding: Value = serde_json::from_str(&binding)?;
            let worker = binding.pointer("/mcp/sampling_worker").map(|p| json!({"connection":p["connection"],"model":p["model"],"max_tokens":p["max_tokens"]}));
            let mut jobs = self.db.prepare("SELECT input_id,intent_id,state FROM mcp_sampling_jobs WHERE interaction_id=?1 ORDER BY input_id LIMIT 8")?;
            let sampling_jobs = jobs.query_map([&id], |r| Ok(json!({"input_id":r.get::<_,String>(0)?,"intent_id":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let item = json!({"interaction_id":id,"run_id":run,"step_id":step,"round":round,"run_state":state,"input_requests":value.get("inputRequests").unwrap_or(&json!({})),"requires_owner":value["inputRequests"].as_object().is_some_and(|m|m.values().any(|v|v["method"]=="elicitation/create")),"untrusted_server_data":true,"sampling_worker":worker,"sampling_jobs":sampling_jobs});
            let len = serde_json::to_vec(&item)?.len();
            if items.len() >= 20 || bytes + len > 40000 {
                more = true;
                break;
            }
            bytes += len;
            items.push(item);
        }
        Ok(
            json!({"view":"mcp_interactions","next_cursor":items.last().map(|v|v["interaction_id"].clone()),"items":items,"truncated":more,"request_state_is_private":true}),
        )
    }
    pub fn owner_respond_mcp(
        &mut self,
        assignment: &str,
        request: InteractionResponse,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.respond_mcp_inner(assignment, request, true)
    }
    pub(crate) fn respond_mcp_inner(
        &mut self,
        assignment: &str,
        request: InteractionResponse,
        owner: bool,
    ) -> Result<Value> {
        self.ensure_assignment_active(assignment)?;
        if request.interaction_id.len() > 128
            || request.request_key.is_empty()
            || request.request_key.len() > 128
            || serde_json::to_vec(&request.responses)?.len() > 32768
        {
            return Err(invalid());
        }
        type Row = (String, String, String, String, String, Option<String>, i64);
        let row:Option<Row>=self.db.query_row("SELECT run_id,step_id,binding_snapshot,response,state,reply,created_at FROM mcp_interactions WHERE id=?1 AND assignment_id=?2",params![request.interaction_id,assignment],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
        let (run, step, binding, response, state, prior, created) =
            row.ok_or(RuntimeError::Forbidden)?;
        let digest = content_digest(&request.responses)?;
        let receipt:Option<(String,bool)>=self.db.query_row("SELECT digest,owner FROM mcp_input_receipts WHERE interaction_id=?1 AND request_key=?2",params![request.interaction_id,request.request_key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((previous, was_owner)) = receipt {
            if previous != digest || (was_owner && !owner) {
                return Err(invalid());
            }
            return Ok(
                json!({"accepted":true,"duplicate":true,"run_id":run,"interaction_id":request.interaction_id,"complete":state!="waiting"}),
            );
        }
        if state != "waiting" || now_ms() - created > 7 * 86400 * 1000 {
            return Err(RuntimeError::State(
                "MCP request is no longer waiting or has expired".into(),
            ));
        }
        let binding: Value = serde_json::from_str(&binding)?;
        let response: Value = serde_json::from_str(&response)?;
        policy(&binding)?
            .validate_partial_response(&response, &request.responses, owner)
            .map_err(|_| invalid())?;
        let mut combined: Value = prior
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or(json!({}));
        for (id, answer) in request.responses.as_object().ok_or_else(invalid)? {
            if combined.get(id).is_some_and(|v| v != answer) {
                return Err(invalid());
            }
            combined[id] = answer.clone();
        }
        if serde_json::to_vec(&combined)?.len() > 32768 {
            return Err(invalid());
        }
        let complete = combined.as_object().map_or(0, |v| v.len())
            == response["inputRequests"].as_object().map_or(0, |v| v.len());
        let run_state = self.ensure_run(&run)?;
        if !matches!(run_state.as_str(), "blocked" | "paused" | "queued") {
            return Err(RuntimeError::State(
                "Recipe cannot accept MCP input in this state".into(),
            ));
        }
        let tx = self.db.transaction()?;
        let changed=tx.execute("UPDATE mcp_interactions SET state=?3,reply=?4,reply_key=?5,reply_digest=?6 WHERE id=?1 AND assignment_id=?2 AND state='waiting'",params![request.interaction_id,assignment,if complete {"ready"} else {"waiting"},serde_json::to_string(&combined)?,request.request_key,content_digest(&combined)?])?;
        if changed != 1 {
            return Err(invalid());
        }
        tx.execute("INSERT INTO mcp_input_receipts(interaction_id,request_key,digest,owner) VALUES(?1,?2,?3,?4)",params![request.interaction_id,request.request_key,digest,owner])?;
        if complete {
            tx.execute("UPDATE steps SET state='queued',active_attempt=NULL WHERE run_id=?1 AND id=?2 AND state='blocked'",params![run,step])?;
            if run_state != "paused" {
                tx.execute(
                    "UPDATE runs SET state='queued',error=NULL WHERE id=?1",
                    [&run],
                )?;
            }
        }
        event(
            &tx,
            &run,
            "mcp.input_supplied",
            json!({"interaction_id":request.interaction_id,"owner":owner,"response_digest":digest,"complete":complete}),
        )?;
        tx.commit()?;
        Ok(
            json!({"accepted":true,"duplicate":false,"run_id":run,"interaction_id":request.interaction_id,"complete":complete,"still_paused":run_state=="paused"}),
        )
    }
}
/// Claim a saved continuation atomically with the next effect intent. Confirmed
/// no-effect reconciliation can reuse it; uncertainty cannot cause replay.
pub(crate) fn claim(
    tx: &Transaction<'_>,
    run: &str,
    step: &str,
    id: &str,
    input: &Value,
    binding: &Value,
) -> Result<Option<(Continuation, u32)>> {
    type Row = (String, String, String, String, String, u32);
    let row:Option<Row>=tx.query_row("SELECT i.id,i.request_digest,i.binding_snapshot,i.response,i.reply,i.round FROM mcp_interactions i LEFT JOIN effect_intents e ON e.id=i.continued_by WHERE i.run_id=?1 AND i.step_id=?2 AND (i.state='ready' OR (i.state='consumed' AND e.status='reconciled_no_effect')) ORDER BY i.round DESC LIMIT 1",params![run,step],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let Some((previous, digest, snapshot, response, reply, round)) = row else {
        return Ok(None);
    };
    let snapshot: Value = serde_json::from_str(&snapshot)?;
    if digest != content_digest(input)?
        || !compatible(&snapshot, binding)
        || round >= policy(binding)?.max_rounds
    {
        return Err(RuntimeError::State(
            "MCP continuation contract changed or round limit reached".into(),
        ));
    }
    let response: Value = serde_json::from_str(&response)?;
    let reply: Value = serde_json::from_str(&reply)?;
    tx.execute(
        "UPDATE mcp_interactions SET state='consumed',continued_by=?2 WHERE id=?1",
        params![previous, id],
    )?;
    Ok(Some((
        Continuation {
            input_responses: reply,
            request_state: response["requestState"].as_str().map(str::to_string),
        },
        round + 1,
    )))
}
pub(crate) fn save_wait(
    tx: &Transaction<'_>,
    identity: (&str, &str, &str),
    input: &Value,
    binding: &Value,
    response: &Value,
    round: u32,
    observation: Option<&apr_connectors::metering::CostObservation>,
) -> Result<()> {
    let (run, step, id) = identity;
    let p = policy(binding)?;
    p.validate_request(response).map_err(|_| invalid())?;
    if round > p.max_rounds {
        return Err(RuntimeError::State("MCP round limit reached".into()));
    }
    let assignment: String =
        tx.query_row("SELECT assignment_id FROM runs WHERE id=?1", [run], |r| {
            r.get(0)
        })?;
    // A receipt exists for this round. It must not be reissued on restart.
    tx.execute("INSERT INTO mcp_interactions(id,run_id,step_id,assignment_id,round,request_digest,binding_snapshot,response,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'waiting',?9)",params![id,run,step,assignment,round,content_digest(input)?,serde_json::to_string(binding)?,serde_json::to_string(response)?,now_ms()])?;
    tx.execute(
        "UPDATE effect_intents SET status='input_required' WHERE id=?1",
        [id],
    )?;
    tx.execute(
        "UPDATE attempts SET state='input_required',finished_at=?2 WHERE id=?1",
        params![id, now_ms()],
    )?;
    tx.execute(
        "UPDATE steps SET state='blocked',active_attempt=NULL WHERE run_id=?1 AND id=?2",
        params![run, step],
    )?;
    tx.execute("UPDATE runs SET state='blocked',error='MCP input required; open the saved interaction' WHERE id=?1",[run])?;
    // An intermediate response is still an external attempt. Save its observed
    // usage atomically with the wait so restart never loses or repeats its cost.
    let observed_value = observation.map(serde_json::to_value).transpose()?;
    crate::billing::settle(
        tx,
        id,
        "input_required",
        observed_value.as_ref(),
        "broker_observed",
    )?;
    if let Some(observation) = observation {
        event(
            tx,
            run,
            "usage.observed",
            json!({"intent_id":id,"provenance":"broker_observed","observation":observation}),
        )?;
    }
    event(
        tx,
        run,
        "mcp.input_required",
        json!({"interaction_id":id,"step_id":step,"round":round}),
    )?;
    Ok(())
}
