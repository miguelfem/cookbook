//! Optional, manually driven improvement records. Evaluation has a deliberately
//! small allowlist: real admitted pure recipes execute in a fresh disposable
//! runtime, with no inherited grants, secrets, native worker or network access.
use std::collections::{BTreeMap, BTreeSet};

use apr_core::{
    AdmittedProcess, admit_json, content_digest, routing::DecisionPolicy, validate_instance,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::{Result, Runtime, RuntimeError, engine::bounded_detail, store::now_ms};

pub(crate) const MIGRATION_V16: &str = r#"
CREATE TABLE improvement_cycles (
  id TEXT PRIMARY KEY,
  assignment_id TEXT NOT NULL REFERENCES assignments(id),
  request_key TEXT NOT NULL,
  open_digest TEXT NOT NULL,
  record TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(assignment_id,request_key)
);
CREATE TABLE improvement_selection (
  assignment_id TEXT NOT NULL REFERENCES assignments(id),
  name TEXT NOT NULL,
  version TEXT NOT NULL REFERENCES process_versions(version),
  revision INTEGER NOT NULL,
  PRIMARY KEY(assignment_id,name)
);
CREATE INDEX improvement_by_assignment ON improvement_cycles(assignment_id,created_at);
CREATE TABLE improvement_holdout_claims (
  assignment_id TEXT NOT NULL REFERENCES assignments(id),
  identity TEXT NOT NULL,
  cycle_id TEXT NOT NULL REFERENCES improvement_cycles(id),
  PRIMARY KEY(assignment_id,identity)
);
"#;

fn normalized(value: &Value) -> Value {
    match value {
        Value::String(s) => json!(s.trim().to_lowercase()),
        Value::Array(a) => Value::Array(a.iter().map(normalized).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), normalized(v))).collect())
        }
        _ => value.clone(),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 500)
        .ok_or_else(|| {
            RuntimeError::InvalidInput(format!("{key} must be a nonempty bounded string"))
        })
}

fn cases(value: &Value) -> Result<Vec<Value>> {
    let rows = value
        .as_array()
        .ok_or_else(|| RuntimeError::InvalidInput("cases must be an array".into()))?;
    if rows.is_empty() || rows.len() > 12 {
        return Err(RuntimeError::LimitExceeded(
            "each split needs 1–12 cases".into(),
        ));
    }
    for row in rows {
        field(row, "case_id")?;
        field(row, "source_id")?;
        if !row.get("input").is_some_and(Value::is_object) {
            return Err(RuntimeError::InvalidInput(
                "case input must be an object".into(),
            ));
        }
        if row.get("provenance").and_then(Value::as_str) != Some("human") {
            return Err(RuntimeError::InvalidInput(
                "held-out truth requires human provenance".into(),
            ));
        }
        if serde_json::to_vec(row)?.len() > 2048 {
            return Err(RuntimeError::LimitExceeded("case exceeds 2 KiB".into()));
        }
    }
    Ok(rows.clone())
}

fn pure(process: &AdmittedProcess) -> Result<()> {
    if process.manifest.steps.len() > 24
        || process
            .manifest
            .files
            .as_ref()
            .is_some_and(|v| !v.is_empty())
        || process.manifest.limits.max_provider_calls != 0
        || process.manifest.limits.max_agent_requests != 0
        || process.manifest.steps.iter().any(|s| {
            s.effect != "pure"
                || s.foreach.is_some()
                || !match s.kind.as_str() {
                    "builtin" => s.operation.as_deref() == Some("json.select"),
                    "decision" => matches!(s.decision, Some(DecisionPolicy::Rules { .. })),
                    "join" => s.join.is_some(),
                    _ => false,
                }
        })
    {
        return Err(RuntimeError::State("additional_evidence_required: controlled evaluation supports at most 24 pure json.select/rules/guard/join steps; no script, provider, connector, artifact, caller or child dispatch".into()));
    }
    Ok(())
}

fn eval_one(process: &AdmittedProcess, input: Value) -> Result<(Value, usize)> {
    let disposable = tempfile::tempdir()?;
    let mut runtime = Runtime::open(disposable.path())?;
    let mut result = runtime.start(process, input, "frozen-case")?;
    for _ in 0..64 {
        if result.state != "queued" && result.state != "running" {
            break;
        }
        result = runtime.advance(&result.run_id)?;
    }
    if result.state != "succeeded" {
        return Err(RuntimeError::State(format!(
            "controlled fixture did not complete: {} ({})",
            result.state,
            result.error.unwrap_or_else(|| "no error recorded".into())
        )));
    }
    Ok((
        result
            .output
            .ok_or_else(|| RuntimeError::Integrity("missing fixture output".into()))?,
        result
            .steps
            .iter()
            .filter(|s| s.state == "succeeded")
            .count(),
    ))
}

impl Runtime {
    pub(crate) fn improvement_evidence(
        &self,
        assignment: &str,
        version: &str,
        ids: &[String],
    ) -> Result<Value> {
        self.catalog_process(assignment, version)?;
        if ids.is_empty()
            || ids.len() > 12
            || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
        {
            return Err(RuntimeError::LimitExceeded(
                "select 1–12 unique retained run IDs".into(),
            ));
        }
        let mut runs = Vec::new();
        for id in ids {
            let row: Option<(String,String,String,Option<String>,i64)> = self.db.query_row(
                "SELECT state,config,input,output,created_at FROM runs WHERE id=?1 AND assignment_id=?2 AND version=?3",
                params![id,assignment,version],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
            let Some((state, config, input, output, created)) = row else {
                return Err(RuntimeError::Forbidden);
            };
            let mut step_counts = BTreeMap::<String, u64>::new();
            let mut statement = self
                .db
                .prepare("SELECT state,COUNT(*) FROM steps WHERE run_id=?1 GROUP BY state")?;
            for entry in
                statement.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
            {
                let (state, count) = entry?;
                step_counts.insert(state, count);
            }
            let (attempts,retries,execution_ms): (u64,u64,Option<i64>)=self.db.query_row("SELECT COUNT(*),COALESCE(SUM(number>1),0),SUM(CASE WHEN finished_at IS NOT NULL THEN MAX(0,finished_at-started_at) END) FROM attempts WHERE run_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            let (handoffs, pending): (u64, u64) = self.db.query_row(
                "SELECT COUNT(*),COALESCE(SUM(state='pending'),0) FROM handoffs WHERE run_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let effects: u64 = self.db.query_row(
                "SELECT COUNT(*) FROM effect_intents WHERE run_id=?1",
                [id],
                |r| r.get(0),
            )?;
            let latest: Option<i64> = self.db.query_row(
                "SELECT MAX(created_at) FROM events WHERE run_id=?1",
                [id],
                |r| r.get(0),
            )?;
            let config: Value = serde_json::from_str(&config)?;
            let analytics = self.analytics_view(assignment, Some(id), Some(20), None)?;
            runs.push(json!({"run_id":id,"state":state,"created_at_ms":created,"elapsed_observed_ms":latest.map(|n|(n-created).max(0)),
                "config_digest":content_digest(&config)?,"input_bytes":input.len(),"output_bytes":output.as_ref().map(String::len),
                "step_counts":step_counts,"attempts":attempts,"retries":retries,"caller_handoffs":handoffs,"pending_handoffs":pending,
                "external_effect_intents":effects,"execution_attempt_duration_sum_ms":execution_ms,"human_wait_ms":null,
                "cost_page":analytics["cost_page"],"cost_page_truncated":analytics["truncated"],
                "quality":"unverified"}));
        }
        let summary = json!({"process_ref":version,"runs":runs,"retained_only":true,"scope":"selected exact version and attached assignment",
            "limitations":["Success does not prove answer correctness.","Caller-client usage and human correction effort are not captured here.","Attempt durations sum across execution and may overlap; human waiting remains unknown.","Cost subtotals cover at most the first 20 intents per run; reported and estimated are separate. Inspect analytics for paging."]});
        bounded_detail(&summary)?;
        Ok(summary)
    }

    fn cycle(&self, assignment: &str, id: &str) -> Result<Value> {
        let record: String = self
            .db
            .query_row(
                "SELECT record FROM improvement_cycles WHERE id=?1 AND assignment_id=?2",
                params![id, assignment],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(RuntimeError::Forbidden)?;
        Ok(serde_json::from_str(&record)?)
    }
    fn save_cycle(&self, assignment: &str, id: &str, value: &Value) -> Result<()> {
        self.db.execute(
            "UPDATE improvement_cycles SET record=?3 WHERE id=?1 AND assignment_id=?2",
            params![id, assignment, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub(crate) fn improvement_view(&self, assignment: &str, id: &str) -> Result<Value> {
        let mut value = self.cycle(assignment, id)?;
        if let Some(object) = value.as_object_mut() {
            object.remove("candidate_manifest");
            object.remove("development");
            object.remove("holdout");
        }
        bounded_detail(&value)?;
        Ok(value)
    }
    pub(crate) fn improvements_view(&self, assignment: &str, version: &str) -> Result<Value> {
        self.catalog_process(assignment, version)?;
        let mut query=self.db.prepare("SELECT id,record FROM improvement_cycles WHERE assignment_id=?1 AND json_extract(record,'$.baseline_ref')=?2 ORDER BY created_at DESC,id DESC LIMIT 10")?;
        let rows = query.query_map(params![assignment, version], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut cycles = Vec::new();
        for row in rows {
            let (id, record) = row?;
            let v: Value = serde_json::from_str(&record)?;
            cycles.push(json!({"cycle_id":id,"state":v["state"],"baseline_ref":v["baseline_ref"],
                "candidate_ref":v["candidate_ref"],"report_digest":v["report_digest"],"result":v["report"]["result"],
                "outcome":v["outcome"],"reason":v["reason"]}));
        }
        Ok(json!({"process_ref":version,"cycles":cycles,"limit":10}))
    }
    pub(crate) fn improvement_open(
        &self,
        assignment: &str,
        request_key: &str,
        version: &str,
        ids: Vec<String>,
    ) -> Result<Value> {
        crate::engine::check_key(request_key)?;
        let digest = content_digest(&json!({"process_ref":version,"run_ids":ids}))?;
        let existing: Option<(String,String)>=self.db.query_row("SELECT id,open_digest FROM improvement_cycles WHERE assignment_id=?1 AND request_key=?2",params![assignment,request_key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((id, prior)) = existing {
            if prior != digest {
                return Err(RuntimeError::RequestConflict);
            }
            return self.improvement_view(assignment, &id);
        }
        let evidence = self.improvement_evidence(assignment, version, &ids)?;
        let process = self.catalog_process(assignment, version)?;
        let default_config_digest = content_digest(&process.manifest.config.defaults)?;
        if evidence["runs"].as_array().is_some_and(|runs| {
            runs.iter()
                .any(|r| r["config_digest"] != default_config_digest)
        }) {
            return Err(RuntimeError::State("controlled pure-v1 evaluation requires selected retained runs to use manifest default configuration; a frozen non-default configuration is not supported".into()));
        }
        let (selected, revision) = self.selection(assignment, &process.manifest.name)?;
        let id = uuid::Uuid::new_v4().to_string();
        let value = json!({"cycle_id":id,"state":"opened","baseline_ref":version,"process_name":process.manifest.name,
            "expected_selected_ref":selected,"expected_selection_revision":revision,
            "evidence_digest":content_digest(&evidence)?,"evidence":evidence,
            "baseline_config_digest":default_config_digest,
            "limits":{"runs":12,"cases_per_split":12,"evaluations":1,"provider_calls":0}});
        self.db.execute("INSERT INTO improvement_cycles(id,assignment_id,request_key,open_digest,record,created_at) VALUES (?1,?2,?3,?4,?5,?6)",params![id,assignment,request_key,digest,serde_json::to_string(&value)?,now_ms()])?;
        self.improvement_view(assignment, &id)
    }
    pub(crate) fn selection(&self, assignment: &str, name: &str) -> Result<(Option<String>, u64)> {
        Ok(self.db.query_row("SELECT version,revision FROM improvement_selection WHERE assignment_id=?1 AND name=?2",
            params![assignment,name],|r|Ok((Some(r.get(0)?),r.get(1)?))).optional()?.unwrap_or((None,0)))
    }
    pub(crate) fn select_registered(
        &mut self,
        assignment: &str,
        version: &str,
        expected: Option<&str>,
        revision: u64,
    ) -> Result<Value> {
        let process = self.catalog_process(assignment, version)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current:(Option<String>,u64)=tx.query_row("SELECT version,revision FROM improvement_selection WHERE assignment_id=?1 AND name=?2",
            params![assignment,process.manifest.name],|r|Ok((Some(r.get(0)?),r.get(1)?))).optional()?.unwrap_or((None,0));
        if current.0.as_deref() != expected || current.1 != revision {
            return Err(RuntimeError::RequestConflict);
        }
        tx.execute("INSERT INTO improvement_selection(assignment_id,name,version,revision) VALUES (?1,?2,?3,1) ON CONFLICT(assignment_id,name) DO UPDATE SET version=excluded.version,revision=revision+1",
            params![assignment,process.manifest.name,version])?;
        tx.commit()?;
        Ok(
            json!({"process_name":process.manifest.name,"selected_process_ref":version,"selection_revision":revision+1,
            "scope":"future discovery in this assignment; existing runs and schedules remain pinned"}),
        )
    }
    pub(crate) fn improvement_propose(
        &self,
        assignment: &str,
        id: &str,
        proposal: Value,
        candidate: &AdmittedProcess,
        review: apr_core::audit::ReviewInput,
    ) -> Result<Value> {
        let mut cycle = self.cycle(assignment, id)?;
        let request_digest = content_digest(
            &json!({"proposal":proposal,"candidate_ref":candidate.version,"review":review}),
        )?;
        if cycle["state"] != "opened" && cycle["proposal_request_digest"] == request_digest {
            return self.improvement_view(assignment, id);
        }
        if cycle["state"] != "opened" {
            return Err(RuntimeError::State(
                "proposal already frozen; open a new cycle for changes".into(),
            ));
        }
        for key in ["hypothesis", "change", "benefit", "risks"] {
            field(&proposal, key)?;
        }
        let evidence_ids = proposal["evidence_run_ids"]
            .as_array()
            .ok_or_else(|| RuntimeError::InvalidInput("proposal needs evidence_run_ids".into()))?;
        if evidence_ids.is_empty()
            || evidence_ids.iter().any(|id| {
                !cycle["evidence"]["runs"]
                    .as_array()
                    .is_some_and(|runs| runs.iter().any(|r| r["run_id"] == *id))
            })
        {
            return Err(RuntimeError::InvalidInput(
                "proposal evidence must refer to selected runs".into(),
            ));
        }
        if candidate.manifest.name != cycle["process_name"] {
            return Err(RuntimeError::InvalidInput(
                "candidate must preserve the recipe name".into(),
            ));
        }
        let baseline = self.catalog_process(assignment, field(&cycle, "baseline_ref")?)?;
        if candidate.manifest.input_schema != baseline.manifest.input_schema
            || candidate.manifest.output_schema != baseline.manifest.output_schema
        {
            return Err(RuntimeError::InvalidInput(
                "candidate must preserve public input/output schemas".into(),
            ));
        }
        if serde_json::to_value(&candidate.manifest.config)?
            != serde_json::to_value(&baseline.manifest.config)?
        {
            return Err(RuntimeError::InvalidInput(
                "candidate must preserve baseline configuration".into(),
            ));
        }
        let audit = apr_core::audit::audit_recipe(&candidate.manifest, &review);
        cycle["state"] = json!("proposed");
        cycle["proposal"] = proposal;
        cycle["candidate_ref"] = json!(candidate.version);
        cycle["proposal_request_digest"] = json!(request_digest);
        cycle["candidate_manifest"] = serde_json::from_slice(&candidate.canonical_bytes)?;
        cycle["audit"] = serde_json::to_value(audit)?;
        self.save_cycle(assignment, id, &cycle)?;
        self.improvement_view(assignment, id)
    }
    pub(crate) fn improvement_freeze(
        &self,
        assignment: &str,
        id: &str,
        policy: Value,
        development: Value,
        holdout: Value,
    ) -> Result<Value> {
        let mut cycle = self.cycle(assignment, id)?;
        let request_digest =
            content_digest(&json!({"policy":policy,"development":development,"holdout":holdout}))?;
        if cycle["state"] != "proposed" && cycle["freeze_request_digest"] == request_digest {
            return self.improvement_view(assignment, id);
        }
        if cycle["state"] != "proposed" {
            return Err(RuntimeError::State(
                "freeze requires a single proposed candidate".into(),
            ));
        }
        if policy.as_object().is_none_or(|v| {
            v.len() != 3
                || !v.contains_key("evaluator_version")
                || !v.contains_key("split_method")
                || !v.contains_key("min_step_reduction")
        }) || policy["evaluator_version"] != "pure-v1"
            || policy["split_method"].as_str().is_none_or(str::is_empty)
            || policy["min_step_reduction"]
                .as_u64()
                .is_none_or(|v| v == 0 || v > 24)
        {
            return Err(RuntimeError::InvalidInput("policy requires evaluator_version pure-v1, split_method and min_step_reduction 1–24".into()));
        }
        let dev = cases(&development)?;
        let held = cases(&holdout)?;
        let mut ids = BTreeSet::new();
        let mut sources = BTreeSet::new();
        let mut inputs = BTreeSet::new();
        for case in dev.iter().chain(held.iter()) {
            if !ids.insert(field(case, "case_id")?.trim().to_lowercase())
                || !sources.insert(field(case, "source_id")?.trim().to_lowercase())
                || !inputs.insert(content_digest(&normalized(&case["input"]))?)
            {
                return Err(RuntimeError::InvalidInput(
                    "duplicate case, source entity or input across frozen splits".into(),
                ));
            }
        }
        let frozen = json!({"policy":policy,"development":dev,"holdout":held,"candidate_ref":cycle["candidate_ref"],"baseline_ref":cycle["baseline_ref"]});
        cycle["policy_digest"] = json!(content_digest(&frozen)?);
        cycle["freeze_request_digest"] = json!(request_digest);
        cycle["policy"] = policy;
        cycle["development"] = development;
        cycle["holdout"] = holdout;
        cycle["state"] = json!("frozen");
        let tx = self.db.unchecked_transaction()?;
        for case in dev.iter().chain(held.iter()) {
            for identity in [
                format!("source:{}", field(case, "source_id")?.trim().to_lowercase()),
                format!("input:{}", content_digest(&normalized(&case["input"]))?),
            ] {
                let owner:Option<String>=tx.query_row("SELECT cycle_id FROM improvement_holdout_claims WHERE assignment_id=?1 AND identity=?2",params![assignment,identity],|r|r.get(0)).optional()?;
                if owner.is_some_and(|owner| owner != id) {
                    return Err(RuntimeError::State("held-out entity/input was already used in this assignment; refresh independent cases".into()));
                }
                if held.iter().any(|h| h["case_id"] == case["case_id"]) {
                    tx.execute("INSERT OR IGNORE INTO improvement_holdout_claims(assignment_id,identity,cycle_id) VALUES (?1,?2,?3)",params![assignment,identity,id])?;
                }
            }
        }
        tx.execute(
            "UPDATE improvement_cycles SET record=?3 WHERE id=?1 AND assignment_id=?2",
            params![id, assignment, serde_json::to_string(&cycle)?],
        )?;
        tx.commit()?;
        self.improvement_view(assignment, id)
    }
    pub(crate) fn improvement_evaluate(&self, assignment: &str, id: &str) -> Result<Value> {
        let mut cycle = self.cycle(assignment, id)?;
        if cycle["state"] == "evaluated"
            || cycle["state"] == "adopted"
            || cycle["state"] == "rejected"
        {
            return self.improvement_view(assignment, id);
        }
        if cycle["state"] != "frozen" {
            return Err(RuntimeError::State(
                "freeze the evaluation contract first".into(),
            ));
        }
        let baseline = self.catalog_process(assignment, field(&cycle, "baseline_ref")?)?;
        let candidate = admit_json(&serde_json::to_vec(&cycle["candidate_manifest"])?)?;
        if candidate.version != field(&cycle, "candidate_ref")? {
            return Err(RuntimeError::Integrity("candidate digest changed".into()));
        }
        let held = cycle["holdout"]
            .as_array()
            .ok_or_else(|| RuntimeError::Integrity("missing frozen holdout".into()))?;
        let mut outcomes = Vec::new();
        let mut baseline_correct = 0;
        let mut candidate_correct = 0;
        let mut baseline_steps = 0usize;
        let mut candidate_steps = 0usize;
        let mut required_complete = 0usize;
        let supported = pure(&baseline).and_then(|_| pure(&candidate));
        let mut reason = supported.err().map(|e| e.to_string());
        if reason.is_none() {
            for case in held {
                if case.get("expected").is_none() {
                    reason = Some("missing human expected output".into());
                    break;
                }
                let input = case["input"].clone();
                if validate_instance(&baseline.manifest.input_schema, &input).is_err() {
                    reason = Some("frozen input fails baseline schema".into());
                    break;
                }
                let b = eval_one(&baseline, input.clone());
                let c = eval_one(&candidate, input);
                match (b, c) {
                    (Ok((b, bs)), Ok((c, cs))) => {
                        let expected = &case["expected"];
                        let bc = b == *expected;
                        let cc = c == *expected;
                        baseline_correct += usize::from(bc);
                        candidate_correct += usize::from(cc);
                        baseline_steps += bs;
                        candidate_steps += cs;
                        required_complete += usize::from(
                            validate_instance(&candidate.manifest.output_schema, &c).is_ok(),
                        );
                        outcomes.push(json!({"case_id":case["case_id"]}));
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        reason = Some(format!("controlled run incomplete: {e}"));
                        break;
                    }
                }
            }
        }
        let threshold = cycle["policy"]["min_step_reduction"]
            .as_u64()
            .unwrap_or(u64::MAX);
        let result = if reason.is_some() {
            "inconclusive"
        } else if candidate_correct != held.len()
            || required_complete != held.len()
            || baseline_steps.saturating_sub(candidate_steps) < (threshold as usize) * held.len()
            || candidate_steps > baseline_steps
        {
            "fail"
        } else {
            "pass"
        };
        let eligible = result == "pass";
        let report = json!({"mode":"controlled_end_to_end","evaluator_version":"pure-v1","result":result,"eligible":eligible,
            "reason":reason,"baseline_ref":cycle["baseline_ref"],"candidate_ref":cycle["candidate_ref"],
            "policy_digest":cycle["policy_digest"],"baseline_config_digest":cycle["baseline_config_digest"],
            "heldout_cases":held.len(),"evaluated_cases":outcomes.len(),
            "baseline_correct":baseline_correct,"candidate_correct":candidate_correct,"required_output_complete":required_complete,
            "baseline_steps_total":baseline_steps,"candidate_steps_total":candidate_steps,
            "step_reduction_total":baseline_steps as i64-candidate_steps as i64,
            "cases":outcomes,"cost":{"reported":null,"estimated":null,"coverage":"unknown; synthetic pure execution, no provider billing"},
            "latency":{"live_ms":null,"replay_ms":null,"coverage":"not measured"},
            "caller_usage":"not observed","quality_scope":"declared human-provenance cases; collection and truth independence remain author responsibilities. No production quality or savings claim",
            "effects":"no network, native, artifact or model execution permitted","evaluations_used":1});
        cycle["report_digest"] = json!(content_digest(&report)?);
        cycle["report"] = report;
        cycle["state"] = json!("evaluated");
        crate::store::fault_point("improvement_before_report_commit");
        self.save_cycle(assignment, id, &cycle)?;
        crate::store::fault_point("improvement_after_report_commit");
        self.improvement_view(assignment, id)
    }
    pub(crate) fn improvement_close(
        &self,
        assignment: &str,
        id: &str,
        outcome: &str,
        reason: &str,
    ) -> Result<Value> {
        if !matches!(
            outcome,
            "no_supported_improvement" | "more_evidence_required"
        ) || reason.trim().is_empty()
            || reason.len() > 1000
        {
            return Err(RuntimeError::InvalidInput(
                "close requires a supported outcome and bounded reason".into(),
            ));
        }
        let mut cycle = self.cycle(assignment, id)?;
        if cycle["state"] == "closed" {
            if cycle["outcome"] == outcome && cycle["reason"] == reason {
                return self.improvement_view(assignment, id);
            }
            return Err(RuntimeError::RequestConflict);
        }
        if !matches!(cycle["state"].as_str(), Some("opened" | "proposed")) {
            return Err(RuntimeError::State(
                "only a cycle without frozen evaluation can close without a report".into(),
            ));
        }
        cycle["state"] = json!("closed");
        cycle["outcome"] = json!(outcome);
        cycle["reason"] = json!(reason);
        self.save_cycle(assignment, id, &cycle)?;
        self.improvement_view(assignment, id)
    }
    pub(crate) fn improvement_decide(
        &mut self,
        assignment: &str,
        id: &str,
        decision: &str,
        candidate: &AdmittedProcess,
        report_digest: &str,
        expected_selected: Option<&str>,
    ) -> Result<Value> {
        let mut cycle = self.cycle(assignment, id)?;
        if !matches!(decision, "adopt" | "reject") {
            return Err(RuntimeError::InvalidInput(
                "decision must be adopt or reject".into(),
            ));
        }
        if cycle["report_digest"] != report_digest
            || cycle["candidate_ref"] != candidate.version
            || cycle["candidate_manifest"]
                != serde_json::from_slice::<Value>(&candidate.canonical_bytes)?
        {
            return Err(RuntimeError::RequestConflict);
        }
        if cycle["state"]
            == decision
                .replace("adopt", "adopted")
                .replace("reject", "rejected")
        {
            return self.improvement_view(assignment, id);
        }
        if cycle["state"] != "evaluated" {
            return Err(RuntimeError::State(
                "decision requires a frozen completed report".into(),
            ));
        }
        if cycle["expected_selected_ref"].as_str() != expected_selected {
            return Err(RuntimeError::RequestConflict);
        }
        if decision == "adopt" && cycle["report"]["eligible"] != true {
            return Err(RuntimeError::State(
                "candidate did not pass its frozen acceptance policy".into(),
            ));
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let selected:Option<(String,u64)>=tx.query_row("SELECT version,revision FROM improvement_selection WHERE assignment_id=?1 AND name=?2",
            params![assignment,field(&cycle,"process_name")?],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if selected.as_ref().map(|s| s.0.as_str()) != expected_selected
            || selected.as_ref().map_or(0, |s| s.1)
                != cycle["expected_selection_revision"]
                    .as_u64()
                    .unwrap_or(u64::MAX)
        {
            return Err(RuntimeError::RequestConflict);
        }
        if decision == "adopt" {
            tx.execute("INSERT OR IGNORE INTO process_versions(version,manifest,canonical_format) VALUES (?1,?2,?3)",params![candidate.version,String::from_utf8_lossy(&candidate.canonical_bytes),apr_core::CANONICAL_FORMAT])?;
            tx.execute("INSERT OR IGNORE INTO catalog(assignment_id,version,name,registered_at) VALUES (?1,?2,?3,?4)",params![assignment,candidate.version,candidate.manifest.name,now_ms()])?;
            tx.execute("INSERT INTO improvement_selection(assignment_id,name,version,revision) VALUES (?1,?2,?3,1) ON CONFLICT(assignment_id,name) DO UPDATE SET version=excluded.version,revision=revision+1",params![assignment,candidate.manifest.name,candidate.version])?;
            cycle["selection_revision"] =
                json!(cycle["expected_selection_revision"].as_u64().unwrap_or(0) + 1);
        }
        cycle["state"] = json!(if decision == "adopt" {
            "adopted"
        } else {
            "rejected"
        });
        tx.execute(
            "UPDATE improvement_cycles SET record=?3 WHERE id=?1 AND assignment_id=?2",
            params![id, assignment, serde_json::to_string(&cycle)?],
        )?;
        tx.commit()?;
        self.improvement_view(assignment, id)
    }
}
