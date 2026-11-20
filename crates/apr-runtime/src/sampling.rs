//! Durable, explicitly approved sampling workers. Worker calls share their
//! recipe's attempts, provider limits, monetary reservations and private ledger.
//! A saved output is reused after restart; ambiguous calls require reconciliation.
use crate::execution::Reconciliation;
use crate::store::{StoreLock, event, now_ms};
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::{Connection, HttpBroker, mcp::sampling::SamplingWorker};
use apr_core::{content_digest, validate_instance};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
pub(crate) const TABLES: &str = r#"
CREATE TABLE mcp_sampling_jobs (interaction_id TEXT NOT NULL REFERENCES mcp_interactions(id) ON DELETE CASCADE, input_id TEXT NOT NULL, intent_id TEXT NOT NULL UNIQUE, profile TEXT NOT NULL, state TEXT NOT NULL, result TEXT,PRIMARY KEY(interaction_id,input_id));
"#;
fn invalid() -> RuntimeError {
    RuntimeError::State(
        "Sampling worker needs explicit review; no automatic retry or result was supplied".into(),
    )
}
impl Runtime {
    /// Runs at most one bounded worker request per service tick; no configured
    /// profile means no inference, including in ordinary default installation.
    pub fn tick_sampling_workers(&mut self) -> Result<bool> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        let rows = self.mcp_sampling_pending()?;
        for (interaction, run, step, assignment, binding, request, reply, created_at) in rows {
            // Advance before validating this candidate. A revoked grant or an
            // unresolved request cannot trap every later job behind this page.
            self.sampling_cursor = Some((created_at, interaction.clone()));
            self.ensure_assignment_active(&assignment)?;
            let binding: Value = serde_json::from_str(&binding)?;
            let Some(profile) = binding.pointer("/mcp/sampling_worker") else {
                continue;
            };
            let profile: SamplingWorker = serde_json::from_value(profile.clone())?;
            profile.validate().map_err(|_| invalid())?;
            let original: Value = serde_json::from_str(&request)?;
            let reply: Value = reply
                .map(|v| serde_json::from_str(&v))
                .transpose()?
                .unwrap_or(json!({}));
            for (id, input) in original["inputRequests"].as_object().ok_or_else(invalid)? {
                if input["method"] != "sampling/createMessage" || reply.get(id).is_some() {
                    continue;
                }
                let job:Option<(String,String,Option<String>)>=self.db.query_row("SELECT intent_id,state,result FROM mcp_sampling_jobs WHERE interaction_id=?1 AND input_id=?2",params![interaction,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                if let Some((intent, state, result)) = &job {
                    if state == "complete" {
                        let answer: Value =
                            serde_json::from_str(result.as_ref().ok_or_else(invalid)?)?;
                        self.respond_mcp_inner(
                            &assignment,
                            crate::interactions::InteractionResponse {
                                interaction_id: interaction.clone(),
                                request_key: format!("sampling-{intent}"),
                                responses: json!({id:answer}),
                            },
                            false,
                        )?;
                        return Ok(true);
                    }
                    if state != "retry_authorized" {
                        if state == "dispatched" {
                            self.mark_sampling_uncertain(&run, intent)?;
                        }
                        continue;
                    }
                }
                let original_name: String = self.db.query_row(
                    "SELECT binding_name FROM effect_intents WHERE id=?1",
                    [&interaction],
                    |r| r.get(0),
                )?;
                let (_, original_binding) =
                    self.current_binding(&run, "connection", &original_name)?;
                if original_binding.pointer("/mcp/sampling_worker")
                    != binding.pointer("/mcp/sampling_worker")
                {
                    return Err(invalid());
                }
                let (revision, cfg) =
                    self.current_binding(&run, "connection", &profile.connection)?;
                let connection: Connection = serde_json::from_value(cfg.clone())?;
                if connection.mcp.is_some() {
                    return Err(invalid());
                }
                let body = profile.request(input).map_err(|_| invalid())?;
                validate_instance(&connection.input_schema, &body)?;
                let process = self.load_run(&run)?.0;
                let used: u32 = self.db.query_row(
                    "SELECT count(*) FROM effect_intents WHERE run_id=?1 AND kind='connection'",
                    [&run],
                    |r| r.get(0),
                )?;
                if used >= process.manifest.limits.max_provider_calls
                    || !self.shared_provider_budget_available(&run)?
                {
                    return Err(RuntimeError::LimitExceeded(
                        "Sampling worker provider budget exhausted".into(),
                    ));
                }
                let intent = format!("attempt_{}", uuid::Uuid::new_v4().simple());
                let active = self.begin_interruptible(&run, &intent)?;
                if active.flag.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(false);
                }
                let tx = self.db.transaction()?;
                crate::billing::reserve(&tx, &run, &intent, &profile.connection, revision, &cfg)?;
                tx.execute("UPDATE steps SET attempt_count=attempt_count+1 WHERE run_id=?1 AND id=?2 AND state='blocked'",params![run,step])?;
                let (number, fence): (u32, i64) = tx.query_row(
                    "SELECT attempt_count,fence FROM steps WHERE run_id=?1 AND id=?2",
                    params![run, step],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                tx.execute("INSERT INTO attempts(id,run_id,step_id,number,fence,state,started_at) VALUES(?1,?2,?3,?4,?5,'running',?6)",params![intent,run,step,number,fence,now_ms()])?;
                tx.execute("INSERT INTO effect_intents(id,run_id,step_id,kind,binding_name,binding_revision,binding_snapshot,request_digest,request_snapshot,status,created_at) VALUES(?1,?2,?3,'connection',?4,?5,?6,?7,?8,'dispatched',?9)",params![intent,run,step,profile.connection,revision,serde_json::to_string(&cfg)?,content_digest(&body)?,serde_json::to_string(&body)?,now_ms()])?;
                tx.execute("INSERT INTO mcp_sampling_jobs(interaction_id,input_id,intent_id,profile,state) VALUES(?1,?2,?3,?4,'dispatched') ON CONFLICT(interaction_id,input_id) DO UPDATE SET intent_id=excluded.intent_id,state='dispatched',result=NULL",params![interaction,id,intent,serde_json::to_string(&profile)?])?;
                event(
                    &tx,
                    &run,
                    "sampling.dispatched",
                    json!({"intent_id":intent,"interaction_id":interaction,"input_id":id}),
                )?;
                tx.commit()?;
                let result = if active.flag.load(std::sync::atomic::Ordering::Acquire) {
                    None
                } else {
                    Some(HttpBroker::new(self.vault()?).dispatch_cancellable(
                        &connection,
                        &body,
                        Some(&active.flag),
                    ))
                };
                match result {
                    Some(Ok(output)) => self.save_sampling_result(
                        (&run, &intent),
                        &profile,
                        &connection,
                        &body,
                        &output.output,
                        "broker_observed",
                    )?,
                    result => {
                        let state = if result.as_ref().is_some_and(|result| {
                            result.as_ref().is_err_and(|e| e.may_have_executed())
                        }) {
                            "uncertain"
                        } else {
                            "not_sent"
                        };
                        let tx = self.db.transaction()?;
                        tx.execute(
                            "UPDATE effect_intents SET status=?2 WHERE id=?1",
                            params![intent, state],
                        )?;
                        tx.execute(
                            "UPDATE attempts SET state=?2,finished_at=?3 WHERE id=?1",
                            params![intent, state, now_ms()],
                        )?;
                        tx.execute(
                            "UPDATE mcp_sampling_jobs SET state='uncertain' WHERE intent_id=?1",
                            [&intent],
                        )?;
                        crate::billing::settle(&tx, &intent, state, None, "broker_observed")?;
                        event(
                            &tx,
                            &run,
                            "sampling.needs_reconciliation",
                            json!({"intent_id":intent,"status":state}),
                        )?;
                        tx.commit()?;
                    }
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn mcp_sampling_pending(&mut self) -> Result<Vec<PendingRow>> {
        let rows = self.mcp_sampling_page()?;
        if rows.is_empty() && self.sampling_cursor.take().is_some() {
            // Revisit older waits after reaching the end, including requests
            // that became eligible while another page was being inspected.
            return self.mcp_sampling_page();
        }
        Ok(rows)
    }
    fn mcp_sampling_page(&self) -> Result<Vec<PendingRow>> {
        let (after_ms, after_id) = self
            .sampling_cursor
            .as_ref()
            .map(|(time, id)| (*time, id.as_str()))
            .unwrap_or((-1, ""));
        let mut q=self.db.prepare("SELECT i.id,i.run_id,i.step_id,i.assignment_id,i.binding_snapshot,i.response,i.reply,i.created_at FROM mcp_interactions i JOIN runs r ON r.id=i.run_id WHERE i.state='waiting' AND r.state='blocked' AND (i.created_at>?1 OR (i.created_at=?1 AND i.id>?2)) ORDER BY i.created_at,i.id LIMIT 20")?;
        Ok(q.query_map(params![after_ms, after_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    fn mark_sampling_uncertain(&mut self, run: &str, intent: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE mcp_sampling_jobs SET state='uncertain' WHERE intent_id=?1",
            [intent],
        )?;
        tx.execute(
            "UPDATE effect_intents SET status='uncertain' WHERE id=?1 AND status='dispatched'",
            [intent],
        )?;
        tx.execute(
            "UPDATE attempts SET state='uncertain',finished_at=?2 WHERE id=?1",
            params![intent, now_ms()],
        )?;
        crate::billing::settle(&tx, intent, "uncertain", None, "broker_observed")?;
        event(
            &tx,
            run,
            "sampling.needs_reconciliation",
            json!({"intent_id":intent}),
        )?;
        tx.commit()?;
        Ok(())
    }
    fn save_sampling_result(
        &mut self,
        identity: (&str, &str),
        profile: &SamplingWorker,
        connection: &Connection,
        input: &Value,
        output: &Value,
        provenance: &str,
    ) -> Result<()> {
        let (run, intent) = identity;
        let observed = connection
            .metering
            .as_ref()
            .map(|m| serde_json::to_value(m.observe_exchange(input, output)))
            .transpose()?;
        let normalized = profile.result(output).ok();
        let status = if normalized.is_some() {
            "succeeded"
        } else {
            "uncertain"
        };
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE mcp_sampling_jobs SET state=?2,result=?3 WHERE intent_id=?1",
            params![
                intent,
                if normalized.is_some() {
                    "complete"
                } else {
                    "uncertain"
                },
                normalized.as_ref().map(serde_json::to_string).transpose()?
            ],
        )?;
        tx.execute(
            "UPDATE effect_intents SET status=?2 WHERE id=?1",
            params![intent, status],
        )?;
        tx.execute(
            "UPDATE attempts SET state=?2,finished_at=?3 WHERE id=?1",
            params![intent, status, now_ms()],
        )?;
        crate::billing::settle(&tx, intent, status, observed.as_ref(), provenance)?;
        if let Some(observation) = observed {
            event(
                &tx,
                run,
                "usage.observed",
                json!({"intent_id":intent,"provenance":provenance,"observation":observation}),
            )?;
        }
        event(
            &tx,
            run,
            "sampling.result",
            json!({"intent_id":intent,"valid":normalized.is_some(),"provenance":provenance}),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn reconcile_sampling(&mut self, request: &Reconciliation) -> Result<Option<Value>> {
        let profile:Option<String>=self.db.query_row("SELECT j.profile FROM mcp_sampling_jobs j JOIN effect_intents e ON e.id=j.intent_id WHERE j.intent_id=?1 AND e.run_id=?2 AND e.step_id=?3",params![request.intent_id,request.run_id,request.step_instance_id],|r|r.get(0)).optional()?;
        let Some(profile) = profile else {
            return Ok(None);
        };
        let profile: SamplingWorker = serde_json::from_str(&profile)?;
        let (status, cfg, input): (String, String, String) = self.db.query_row(
            "SELECT status,binding_snapshot,request_snapshot FROM effect_intents WHERE id=?1",
            [&request.intent_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if !matches!(status.as_str(), "uncertain" | "not_sent") {
            return Err(invalid());
        }
        if request.outcome == "observed_result" {
            let output = request.output.as_ref().ok_or_else(invalid)?;
            let cfg: Connection = serde_json::from_str(&cfg)?;
            validate_instance(&cfg.output_schema, output)?;
            profile.result(output).map_err(|_| invalid())?;
            self.save_sampling_result(
                (&request.run_id, &request.intent_id),
                &profile,
                &cfg,
                &serde_json::from_str(&input)?,
                output,
                "caller_reported",
            )?;
        } else if request.outcome == "confirmed_no_effect" {
            let tx = self.db.transaction()?;
            tx.execute(
                "UPDATE mcp_sampling_jobs SET state='retry_authorized' WHERE intent_id=?1",
                [&request.intent_id],
            )?;
            tx.execute(
                "UPDATE effect_intents SET status='reconciled_no_effect',evidence=?2 WHERE id=?1",
                params![request.intent_id, request.evidence],
            )?;
            crate::billing::settle(
                &tx,
                &request.intent_id,
                "reconciled_no_effect",
                None,
                "caller_reported",
            )?;
            tx.commit()?;
        } else if request.outcome == "cancel_uncertain" {
            let tx = self.db.transaction()?;
            crate::scope::cancel_in_transaction(&tx, &request.run_id, "sampling.cancel_uncertain")?;
            tx.execute(
                "UPDATE effect_intents SET evidence=?2 WHERE id=?1",
                params![request.intent_id, request.evidence],
            )?;
            tx.commit()?;
        } else {
            return Err(invalid());
        }
        Ok(Some(
            json!({"intent_id":request.intent_id,"sampling_reconciled":true,"run":self.envelope(&request.run_id)?}),
        ))
    }
}
type PendingRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
);
