use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use apr_core::{
    AdmittedProcess, ArtifactRef, ProcessManifest, Step, admit_json, canonical_bytes,
    content_digest, resolve_bindings_with_context, validate_instance,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::artifacts::ArtifactStore;
use crate::store::{self, StoreLock, event, fault_point, now_ms};
use crate::*;

// Reserve 16 KiB for run/claim identity and transport wrappers. Detail payloads
// must remain usable through the 64 KiB service and MCP response contracts.
const MAX_DETAIL_PAYLOAD_BYTES: usize = 48 * 1024;

/// Single-user executor. A process-wide file lock serializes state changes;
/// bounded prepared external workers may overlap while one coordinator commits.
pub struct Runtime {
    pub(crate) root: PathBuf,
    pub(crate) db: Connection,
    pub(crate) artifacts: ArtifactStore,
    pub(crate) interrupt: crate::execution::ExecutionInterrupt,
    pub(crate) sampling_cursor: Option<(i64, String)>,
    pub(crate) external_queue: Option<Vec<crate::execution::PreparedExternal>>,
    pub(crate) external_pool: Option<crate::execution::ExternalPool>,
    pub(crate) dispatch_policy: Option<crate::worker_policy::WorkerPolicy>,
}

struct StepRecord {
    id: String,
    definition_id: String,
    parent_step_id: Option<String>,
    position: usize,
    item_key: Option<Value>,
    item_index: Option<usize>,
    item_value: Option<Value>,
    state: String,
    output: Option<Value>,
    attempt_count: u32,
    active_attempt: Option<String>,
    retry_limit: u32,
}

struct BindingContext<'a> {
    input: &'a Value,
    outputs: &'a BTreeMap<String, Value>,
    config: &'a Value,
}

pub(crate) struct StartOptions<'a> {
    pub assignment_id: Option<&'a str>,
    pub drive: bool,
    pub config_overrides: &'a Value,
    pub fixture: bool,
}

impl Runtime {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        store::private_dir(root.as_ref())?;
        let root = root.as_ref().canonicalize()?;
        let _lock = StoreLock::acquire(&root)?;
        store::private_dir(&root.join("handoffs"))?;
        let db = store::connect(&root)?;
        let artifacts = ArtifactStore::open(&root)?;
        Ok(Self {
            root,
            db,
            artifacts,
            interrupt: Default::default(),
            sampling_cursor: None,
            external_queue: None,
            external_pool: None,
            dispatch_policy: None,
        })
    }

    pub fn start(
        &mut self,
        process: &AdmittedProcess,
        input: Value,
        request_key: &str,
    ) -> Result<RunEnvelope> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.start_inner(process, input, request_key, None, true)
    }

    pub(crate) fn start_inner(
        &mut self,
        process: &AdmittedProcess,
        input: Value,
        request_key: &str,
        assignment_id: Option<&str>,
        drive: bool,
    ) -> Result<RunEnvelope> {
        self.start_with_config_inner(
            process,
            input,
            request_key,
            StartOptions {
                assignment_id,
                drive,
                config_overrides: &json!({}),
                fixture: false,
            },
        )
    }

    pub(crate) fn start_with_config_inner(
        &mut self,
        process: &AdmittedProcess,
        input: Value,
        request_key: &str,
        options: StartOptions<'_>,
    ) -> Result<RunEnvelope> {
        let StartOptions {
            assignment_id,
            drive,
            config_overrides,
            fixture,
        } = options;
        self.ensure_not_maintaining()?;
        check_key(request_key)?;
        bounded(&input)?;
        bounded(config_overrides)?;
        if !config_overrides.is_object() {
            return Err(RuntimeError::InvalidInput(
                "config overrides must be an object".into(),
            ));
        }
        // Admission is repeated at this public boundary, so callers cannot forge a
        // mutable AdmittedProcess or substitute its digest.
        let frozen = admit_json(&process.canonical_bytes)?;
        if frozen.version != process.version {
            return Err(RuntimeError::Integrity(
                "process version does not match its frozen manifest".into(),
            ));
        }
        validate_instance(&frozen.manifest.input_schema, &input)
            .map_err(|e| RuntimeError::InvalidInput(e.to_string()))?;
        if let Some(assignment) = assignment_id {
            self.ensure_assignment_active(assignment)?;
            self.authorize_artifacts(assignment, &input)?;
            self.authorize_literals(assignment, &frozen.manifest)?;
        }
        let request_key = serde_json::to_string(&json!([assignment_id, request_key]))?;
        let existing: Option<(String, String, String)> = self
            .db
            .query_row(
                "SELECT id,request_digest,config FROM runs WHERE request_key=?1",
                [&request_key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((id, digest, config)) = existing {
            let config: Value = serde_json::from_str(&config)?;
            let request_digest = content_digest(
                &json!({"version":frozen.version,"input":input,"config":config,"config_overrides":config_overrides,"fixture":fixture}),
            )?;
            if digest != request_digest {
                return Err(RuntimeError::RequestConflict);
            }
            if drive {
                self.drive(&id)?;
            }
            return self.envelope(&id);
        }
        self.execution_preflight(&frozen, assignment_id)?;
        let config = self.resolve_configuration(&frozen, assignment_id, config_overrides)?;
        if let Some(assignment) = assignment_id {
            self.authorize_artifacts(assignment, &config.value)?;
        }
        let config_refs =
            self.checked_refs(&config.value, frozen.manifest.limits.max_artifact_bytes)?;
        let refs = self.checked_refs(&input, frozen.manifest.limits.max_artifact_bytes)?;
        let calls = self.prepare_call_snapshots(&frozen, assignment_id, fixture)?;
        let id = new_id("run");
        let tx = self.db.transaction()?;
        insert_run_record(
            &tx,
            &id,
            RunRecord {
                process: &frozen,
                input: &input,
                request_key: &request_key,
                assignment_id,
                config: &config,
                config_overrides,
                fixture,
            },
        )?;
        crate::execution::freeze_execution_bindings(&tx, &id, &frozen.manifest, assignment_id)?;
        crate::composition::freeze_call_snapshots(&tx, &id, assignment_id, &calls)?;
        link_artifacts(&tx, &id, "$input", &refs)?;
        link_artifacts(&tx, &id, "$config", &config_refs)?;
        event(
            &tx,
            &id,
            "run.created",
            json!({"process_version":frozen.version}),
        )?;
        tx.commit()?;
        fault_point("run_created");
        if drive {
            self.drive(&id)?;
        }
        self.envelope(&id)
    }

    pub fn inspect(&self, run_id: &str) -> Result<RunEnvelope> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.envelope(run_id)
    }

    pub fn advance(&mut self, run_id: &str) -> Result<RunEnvelope> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.drive(run_id)?;
        self.envelope(run_id)
    }

    pub(crate) fn retry_inner(&mut self, run_id: &str, step_id: &str) -> Result<Value> {
        if self.ensure_run(run_id)? != "failed" {
            return Err(RuntimeError::State("retry requires a failed run".into()));
        }
        let record = self
            .step_records(run_id)?
            .into_iter()
            .find(|record| record.id == step_id && record.state == "failed")
            .ok_or_else(|| RuntimeError::State("retry requires a failed step instance".into()))?;
        let (process, _, _) = self.load_run(run_id)?;
        let step = process
            .manifest
            .steps
            .iter()
            .find(|step| step.id == record.definition_id)
            .ok_or_else(|| RuntimeError::State("step definition is missing".into()))?;
        if step.kind != "builtin"
            || step.effect != "pure"
            || (step.foreach.is_some() && record.parent_step_id.is_none())
        {
            return Err(RuntimeError::State(
                "only failed pure builtin instances support explicit retry".into(),
            ));
        }
        if record.retry_limit >= 5 {
            return Err(RuntimeError::LimitExceeded(
                "step reached its five-attempt retry ceiling".into(),
            ));
        }
        let parents = self.prepare_child_retry(run_id)?;
        let tx = self.db.transaction()?;
        tx.execute("UPDATE steps SET state='queued',retry_limit=retry_limit+1,active_attempt=NULL,fence=fence+1 WHERE run_id=?1 AND id=?2 AND state='failed'",params![run_id,step_id])?;
        tx.execute(
            "UPDATE runs SET state='queued',error=NULL WHERE id=?1",
            [run_id],
        )?;
        event(
            &tx,
            run_id,
            "step.retry_requested",
            json!({"step_id":step_id,"retry_limit":record.retry_limit+1}),
        )?;
        crate::composition::restore_parent_calls(&tx, &parents)?;
        tx.commit()?;
        Ok(serde_json::to_value(self.envelope(run_id)?)?)
    }

    /// Explicitly reclaim the next pending handoff. This rotates its generation
    /// and token without issuing a new agent request or spending another budget unit.
    pub fn renew_handoff(&mut self, run_id: &str) -> Result<RunEnvelope> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.renew_inner(run_id)
    }

    pub(crate) fn renew_inner(&mut self, run_id: &str) -> Result<RunEnvelope> {
        if self.ancestors_hold(run_id, true)? {
            return Err(RuntimeError::StaleHandoff);
        }
        if !matches!(
            self.ensure_run(run_id)?.as_str(),
            "waiting_for_agent" | "queued" | "running"
        ) {
            return Err(RuntimeError::StaleHandoff);
        }
        let pending: Option<(String,u32)> = self.db.query_row("SELECT id,generation FROM handoffs WHERE run_id=?1 AND state='pending' ORDER BY rowid LIMIT 1",[run_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (id, generation) = pending.ok_or(RuntimeError::StaleHandoff)?;
        let next = generation
            .checked_add(1)
            .ok_or_else(|| RuntimeError::LimitExceeded("handoff generation exhausted".into()))?;
        let token = self.persist_token(&id, next)?;
        let tx = self.db.transaction()?;
        let changed = tx.execute("UPDATE handoffs SET generation=?2,token_hash=?3,expires_at=?4 WHERE id=?1 AND generation=?5 AND state='pending'",params![id,next,hash_token(&token),now_ms()+86_400_000,generation])?;
        if changed != 1 {
            return Err(RuntimeError::StaleHandoff);
        }
        event(
            &tx,
            run_id,
            "handoff.renewed",
            json!({"request_id":id,"generation":next}),
        )?;
        tx.commit()?;
        self.envelope(run_id)
    }

    pub fn resume(&mut self, submission: &Submission) -> Result<SubmissionReceipt> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.resume_inner(submission, true)
    }

    pub(crate) fn resume_inner(
        &mut self,
        submission: &Submission,
        drive: bool,
    ) -> Result<SubmissionReceipt> {
        check_key(&submission.submission_key)?;
        bounded(&serde_json::to_value(submission)?)?;
        if submission.protocol_version != "0.1" || submission.action != "submit" {
            return Err(RuntimeError::InvalidInput(
                "expected protocol_version 0.1 and action submit".into(),
            ));
        }
        let semantic_digest = content_digest(&json!({
            "run_id":submission.run_id,"request_id":submission.request_id,
            "step_instance_id":submission.step_instance_id,"generation":submission.generation,
            "output":submission.output,"provenance":submission.provenance
        }))?;
        // Local CLI authority is the private state directory. M1 must authenticate
        // attachments before this lookup. Transport credentials are not semantic input.
        let saved: Option<(String,String)> = self.db.query_row(
            "SELECT semantic_digest,receipt FROM receipts WHERE run_id=?1 AND submission_key=?2",
            params![submission.run_id,submission.submission_key], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((digest, receipt)) = saved {
            if digest != semantic_digest {
                return Err(RuntimeError::SubmissionConflict);
            }
            if drive {
                self.drive(&submission.run_id)?;
            }
            return Ok(serde_json::from_str(&receipt)?);
        }
        let request: Option<(String,String,u32,String,i64,String,String)> = self.db.query_row(
            "SELECT run_id,step_id,generation,token_hash,expires_at,state,result_schema FROM handoffs WHERE id=?1",
            [&submission.request_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
        let Some((run_id, step_id, generation, token_hash, expires, state, schema)) = request
        else {
            return Err(RuntimeError::StaleHandoff);
        };
        if run_id != submission.run_id
            || step_id != submission.step_instance_id
            || generation != submission.generation
            || state != "pending"
            || expires <= now_ms()
            || hash_token(&submission.continuation_token) != token_hash
        {
            return Err(RuntimeError::StaleHandoff);
        }
        if !matches!(
            self.ensure_run(&run_id)?.as_str(),
            "waiting_for_agent" | "queued" | "running" | "paused"
        ) || self.ancestors_hold(&run_id, false)?
        {
            return Err(RuntimeError::StaleHandoff);
        }
        if !submission.provenance.is_object()
            || submission
                .provenance
                .get("source")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return Err(RuntimeError::InvalidInput(
                "provenance.source must identify how the result was obtained".into(),
            ));
        }
        let result_schema: Value = serde_json::from_str(&schema)?;
        validate_instance(&result_schema, &submission.output)
            .map_err(|e| RuntimeError::InvalidInput(e.to_string()))?;
        let (process, _, _) = self.load_run(&run_id)?;
        let limits = self.effective_limits(&run_id, &process.manifest.limits)?;
        self.verify_committed(&run_id, limits.max_artifact_bytes)?;
        let refs = self.checked_refs(&submission.output, limits.max_artifact_bytes)?;
        let output_digest = content_digest(&submission.output)?;
        let tx = self.db.transaction()?;
        let changed = tx.execute("UPDATE handoffs SET state='accepted' WHERE id=?1 AND state='pending' AND generation=?2", params![submission.request_id,generation])?;
        if changed != 1 {
            return Err(RuntimeError::StaleHandoff);
        }
        let changed = tx.execute("UPDATE steps SET state='succeeded',output=?3,active_attempt=NULL WHERE run_id=?1 AND id=?2 AND state='waiting_for_agent'", params![run_id,step_id,serde_json::to_string(&submission.output)?])?;
        if changed != 1 {
            return Err(RuntimeError::StaleHandoff);
        }
        tx.execute("UPDATE attempts SET state='succeeded',finished_at=?3 WHERE run_id=?1 AND step_id=?2 AND state='waiting_for_agent'",params![run_id,step_id,now_ms()])?;
        link_artifacts(&tx, &run_id, &step_id, &refs)?;
        tx.execute(
            "UPDATE runs SET state=CASE WHEN state='paused' THEN 'paused' ELSE 'queued' END,error=NULL WHERE id=?1",
            [&run_id],
        )?;
        event(
            &tx,
            &run_id,
            "handoff.accepted",
            json!({"request_id":submission.request_id,"step_id":step_id,"output_digest":output_digest,"provenance":submission.provenance}),
        )?;
        let revision = tx.query_row("SELECT revision FROM runs WHERE id=?1", [&run_id], |r| {
            r.get(0)
        })?;
        let receipt = SubmissionReceipt {
            protocol_version: "0.1".into(),
            run_id: run_id.clone(),
            request_id: submission.request_id.clone(),
            submission_key: submission.submission_key.clone(),
            output_digest,
            status: "accepted".into(),
            revision,
        };
        tx.execute("INSERT INTO receipts(run_id,submission_key,semantic_digest,receipt) VALUES (?1,?2,?3,?4)", params![run_id,submission.submission_key,semantic_digest,serde_json::to_string(&receipt)?])?;
        tx.commit()?;
        fault_point("result_committed");
        if drive {
            self.drive(&run_id)?;
        }
        Ok(receipt)
    }

    pub fn events(&self, run_id: &str, after: u64, limit: usize) -> Result<Vec<Event>> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.events_inner(run_id, after, limit)
    }

    pub(crate) fn events_inner(
        &self,
        run_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<Event>> {
        self.ensure_run(run_id)?;
        let mut query = self.db.prepare("SELECT sequence,kind,detail FROM events WHERE run_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
        let rows = query.query_map(params![run_id, after, limit.clamp(1, 100)], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (sequence, kind, detail) = row?;
            Ok(Event {
                sequence,
                kind,
                detail: serde_json::from_str(&detail)?,
            })
        })
        .collect()
    }

    pub fn import_artifact(
        &mut self,
        path: &Path,
        media_type: &str,
        max_bytes: u64,
    ) -> Result<ArtifactRef> {
        let _lock = StoreLock::acquire(&self.root)?;
        let reference = self.artifacts.import_with_quota(
            path,
            media_type,
            max_bytes,
            self.artifact_quota()?,
        )?;
        verify_registered_reference(&self.db, &reference)?;
        self.db.execute(
            "INSERT OR IGNORE INTO artifacts(id,reference) VALUES (?1,?2)",
            params![reference.artifact_id, serde_json::to_string(&reference)?],
        )?;
        Ok(reference)
    }

    pub fn read_artifact(&self, reference: &ArtifactRef, max_bytes: u64) -> Result<Vec<u8>> {
        let _lock = StoreLock::acquire(&self.root)?;
        verify_registered_reference(&self.db, reference)?;
        Ok(self.artifacts.read(reference, max_bytes)?)
    }

    pub(crate) fn ensure_run(&self, run_id: &str) -> Result<String> {
        self.db
            .query_row("SELECT state FROM runs WHERE id=?1", [run_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| {
                RuntimeError::NotFound("run does not exist in this state directory".into())
            })
    }

    pub(crate) fn root_limits(&self, run_id: &str) -> Result<(String, apr_core::Limits)> {
        let root: String = self.db.query_row(
            "SELECT COALESCE(root_run_id,id) FROM runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )?;
        let (process, _, _) = self.load_run(&root)?;
        Ok((root, process.manifest.limits))
    }

    pub(crate) fn effective_limits(
        &self,
        run_id: &str,
        own: &apr_core::Limits,
    ) -> Result<apr_core::Limits> {
        let (_, root) = self.root_limits(run_id)?;
        let mut limits = own.clone();
        limits.max_artifact_bytes = limits.max_artifact_bytes.min(root.max_artifact_bytes);
        limits.max_step_ms = limits.max_step_ms.min(root.max_step_ms);
        Ok(limits)
    }

    /// Called under the writer lock before an intent reservation. Pending and
    /// uncertain attempts remain charged across every descendant of the root.
    pub(crate) fn shared_provider_budget_available(&self, run_id: &str) -> Result<bool> {
        let (root, limits) = self.root_limits(run_id)?;
        let used:u32=self.db.query_row("SELECT COUNT(*) FROM effect_intents e JOIN runs r ON r.id=e.run_id WHERE COALESCE(r.root_run_id,r.id)=?1 AND e.kind='connection'",[root],|row|row.get(0))?;
        Ok(used < limits.max_provider_calls)
    }

    pub(crate) fn load_run(&self, run_id: &str) -> Result<(AdmittedProcess, Value, Value)> {
        let row: Option<(String,String,String,String)> = self.db.query_row("SELECT v.manifest,r.version,r.input,r.config FROM runs r JOIN process_versions v ON v.version=r.version WHERE r.id=?1", [run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (manifest, version, input, config) = row.ok_or_else(|| {
            RuntimeError::NotFound("run does not exist in this state directory".into())
        })?;
        let process = admit_json(manifest.as_bytes())?;
        if process.version != version {
            return Err(RuntimeError::Integrity(
                "saved process snapshot digest mismatch".into(),
            ));
        }
        Ok((
            process,
            serde_json::from_str(&input)?,
            serde_json::from_str(&config)?,
        ))
    }

    pub(crate) fn run_config_view(&self, run_id: &str) -> Result<Value> {
        let (config,overrides,project_revision,assignment_revision):(String,String,u64,u64)=self.db.query_row(
            "SELECT config,config_overrides,config_project_revision,config_assignment_revision FROM runs WHERE id=?1",[run_id],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
        let config: Value = serde_json::from_str(&config)?;
        let overrides: Value = serde_json::from_str(&overrides)?;
        let result = json!({"run_id":run_id,"config":config,"config_digest":content_digest(&config)?,"config_overrides":overrides,
            "project_revision":project_revision,"assignment_revision":assignment_revision});
        bounded(&result)?;
        Ok(result)
    }

    fn step_records(&self, run_id: &str) -> Result<Vec<StepRecord>> {
        let mut query = self.db.prepare("SELECT id,state,output,attempt_count,active_attempt,COALESCE(definition_id,id),parent_step_id,position,item_key,item_index,item_value,retry_limit FROM steps WHERE run_id=?1 ORDER BY position")?;
        let rows = query.query_map([run_id], |r| {
            Ok(StepRecord {
                id: r.get(0)?,
                state: r.get(1)?,
                output: json_column(r, 2)?,
                attempt_count: r.get(3)?,
                active_attempt: r.get(4)?,
                definition_id: r.get(5)?,
                parent_step_id: r.get(6)?,
                position: r.get(7)?,
                item_key: json_column(r, 8)?,
                item_index: r.get(9)?,
                item_value: json_column(r, 10)?,
                retry_limit: r.get(11)?,
            })
        })?;
        rows.map(|row| row.map_err(RuntimeError::from)).collect()
    }

    fn drive(&mut self, run_id: &str) -> Result<()> {
        self.drive_limited(run_id, usize::MAX)
    }

    pub(crate) fn drive_limited(&mut self, run_id: &str, max_steps: usize) -> Result<()> {
        if self.external_pool.is_none() && crate::service::supervisor_owns_execution(&self.root)? {
            // The supervisor owns all external capacity for this state
            // directory. Another Runtime may inspect or author concurrently,
            // but it must not claim work around the shared worker policy.
            return Ok(());
        }
        let state = self.ensure_run(run_id)?;
        if matches!(
            state.as_str(),
            "succeeded" | "failed" | "cancelled" | "paused" | "needs_reconciliation"
        ) {
            return Ok(());
        }
        if self.cancellation_pending(run_id)? {
            return Ok(());
        }
        if !self.ensure_dispatch_authority(run_id)? {
            return Ok(());
        }
        let (mut process, input, config) = self.load_run(run_id)?;
        // Apply inherited resource ceilings only after verifying the immutable
        // snapshot. Canonical bytes and the public process version stay intact.
        process.manifest.limits = self.effective_limits(run_id, &process.manifest.limits)?;
        let manifest = &process.manifest;
        if let Err(error) = self.verify_committed(run_id, manifest.limits.max_artifact_bytes) {
            self.set_run_state(run_id, "blocked", Some(&error.to_string()))?;
            return Ok(());
        }
        self.recover_interrupted(run_id, manifest)?;
        let mut advanced = 0;
        loop {
            if self.cancellation_pending(run_id)? {
                return Ok(());
            }
            if matches!(
                self.ensure_run(run_id)?.as_str(),
                "failed" | "needs_reconciliation"
            ) {
                return Ok(());
            }
            let records = self.step_records(run_id)?;
            let ready_child:Option<String>=self.db.query_row("SELECT c.parent_step_id FROM child_runs c JOIN runs child ON child.id=c.child_run_id JOIN steps s ON s.run_id=c.parent_run_id AND s.id=c.parent_step_id WHERE c.parent_run_id=?1 AND s.state='waiting_for_child' AND child.state IN ('succeeded','failed','cancelled') ORDER BY s.position LIMIT 1",[run_id],|row|row.get(0)).optional()?;
            if let Some(step_id) = ready_child {
                if advanced >= max_steps {
                    self.set_run_state(run_id, "queued", None)?;
                    return Ok(());
                }
                advanced += 1;
                let record = records
                    .iter()
                    .find(|record| record.id == step_id)
                    .ok_or_else(|| RuntimeError::Integrity("child parent step missing".into()))?;
                let mut step = manifest
                    .steps
                    .iter()
                    .find(|step| step.id == record.definition_id)
                    .ok_or_else(|| RuntimeError::Integrity("child definition missing".into()))?
                    .clone();
                step.id = step_id;
                if let Err(error) = self.settle_child(run_id, &step) {
                    self.fail_step(run_id, &step.id, &error)?;
                    return Ok(());
                }
                continue;
            }
            let outputs: BTreeMap<String, Value> = records
                .iter()
                .filter(|record| record.parent_step_id.is_none())
                .filter_map(|r| r.output.as_ref().map(|v| (r.id.clone(), v.clone())))
                .collect();
            if records
                .iter()
                .all(|r| matches!(r.state.as_str(), "succeeded" | "skipped"))
            {
                let result = resolve_bindings_with_context(
                    &manifest.output,
                    &input,
                    &outputs,
                    &config,
                    None,
                )
                .map_err(RuntimeError::from)
                .and_then(|output| {
                    bounded(&output)?;
                    validate_instance(&manifest.output_schema, &output)?;
                    self.checked_refs(&output, manifest.limits.max_artifact_bytes)?;
                    Ok(output)
                });
                match result {
                    Ok(output) => {
                        let tx = self.db.transaction()?;
                        tx.execute(
                            "UPDATE runs SET state='succeeded',output=?2,error=NULL WHERE id=?1",
                            params![run_id, serde_json::to_string(&output)?],
                        )?;
                        event(
                            &tx,
                            run_id,
                            "run.succeeded",
                            json!({"output_digest":content_digest(&output)?}),
                        )?;
                        tx.commit()?;
                    }
                    Err(e) => self.set_run_state(run_id, "failed", Some(&e.to_string()))?,
                }
                return Ok(());
            }
            let skip = records.iter().find(|record| {
                if record.state != "queued" {
                    return false;
                }
                let Some(step) = manifest
                    .steps
                    .iter()
                    .find(|step| step.id == record.definition_id)
                else {
                    return false;
                };
                step.when.as_ref().is_some_and(|when| {
                    records
                        .iter()
                        .any(|source| source.id == when.decision && source.state == "skipped")
                        || outputs
                            .get(&when.decision)
                            .is_some_and(|value| value["route"].as_str() != Some(&when.route))
                }) || step.join.as_ref().is_some_and(|join| {
                    records
                        .iter()
                        .any(|source| source.id == join.decision && source.state == "skipped")
                })
            });
            if let Some(record) = skip {
                if advanced >= max_steps {
                    self.set_run_state(run_id, "queued", None)?;
                    return Ok(());
                }
                advanced += 1;
                let tx = self.db.transaction()?;
                tx.execute(
                    "UPDATE steps SET state='skipped' WHERE run_id=?1 AND id=?2 AND state='queued'",
                    params![run_id, record.id],
                )?;
                event(
                    &tx,
                    run_id,
                    "step.skipped",
                    json!({"step_id":record.id,"reason":"unselected route"}),
                )?;
                tx.commit()?;
                continue;
            }
            let aggregate = records.iter().find(|record| {
                record.state == "expanded"
                    && records
                        .iter()
                        .filter(|child| child.parent_step_id.as_deref() == Some(&record.id))
                        .all(|child| child.state == "succeeded")
            });
            if let Some(parent) = aggregate {
                if advanced >= max_steps {
                    self.set_run_state(run_id, "queued", None)?;
                    return Ok(());
                }
                advanced += 1;
                if let Err(error) = self.complete_foreach(run_id, parent, &records, manifest) {
                    self.fail_step(run_id, &parent.id, &error)?;
                    return Ok(());
                }
                continue;
            }
            let candidate = records.iter().find_map(|record| {
                if record.state != "queued" {
                    return None;
                }
                let step = manifest
                    .steps
                    .iter()
                    .find(|step| step.id == record.definition_id)?;
                if !step.needs.iter().all(|id| {
                    outputs.contains_key(id)
                        || (step.join.is_some()
                            && records
                                .iter()
                                .any(|source| source.id == *id && source.state == "skipped"))
                }) {
                    return None;
                }
                if let Some(parent_id) = &record.parent_step_id {
                    let concurrency = step.foreach.as_ref()?.concurrency as usize;
                    let active = records
                        .iter()
                        .filter(|other| {
                            other.parent_step_id.as_ref() == Some(parent_id)
                                && matches!(
                                    other.state.as_str(),
                                    "running" | "waiting_for_agent" | "waiting_for_child"
                                )
                        })
                        .count();
                    if active >= concurrency {
                        return None;
                    }
                }
                if !self.external_can_queue(run_id, step) {
                    return None;
                }
                Some((record, step))
            });
            let Some((record, template)) = candidate else {
                let live_claim = records
                    .iter()
                    .filter(|record| record.state == "running")
                    .filter_map(|record| record.active_attempt.as_deref())
                    .try_fold(false, |found, attempt| {
                        crate::execution::claim_is_live(&self.root, attempt)
                            .map(|live| found || live)
                    })?;
                if records.iter().any(|r| r.state == "waiting_for_agent") {
                    self.set_run_state(run_id, "waiting_for_agent", None)?;
                } else if records.iter().any(|r| r.state == "waiting_for_child") {
                    self.set_run_state(run_id, "waiting_for_child", None)?;
                } else if self
                    .external_pool
                    .as_ref()
                    .is_some_and(|pool| pool.has_run(run_id))
                    || live_claim
                {
                    self.set_run_state(run_id, "running", None)?;
                } else if self
                    .external_queue
                    .as_ref()
                    .is_some_and(|queue| !queue.is_empty())
                {
                    // Claimed jobs in this preparation turn have not yet been
                    // handed to the asynchronous worker pool.
                    self.set_run_state(run_id, "queued", None)?;
                } else {
                    self.set_run_state(run_id, "blocked", Some("no step can advance"))?;
                }
                return Ok(());
            };
            if advanced >= max_steps {
                self.set_run_state(run_id, "queued", None)?;
                return Ok(());
            }
            advanced += 1;
            if record.parent_step_id.is_none() && template.foreach.is_some() {
                if let Err(error) = self.expand_foreach(
                    run_id,
                    record,
                    template,
                    BindingContext {
                        input: &input,
                        outputs: &outputs,
                        config: &config,
                    },
                    manifest,
                ) {
                    self.fail_step(run_id, &record.id, &error)?;
                    return Ok(());
                }
                continue;
            }
            let mut step = template.clone();
            step.id = record.id.clone();
            step.foreach = None;
            step.retry.max_attempts = record.retry_limit;
            if matches!(step.kind.as_str(), "decision" | "join") {
                let result = if let Some(policy) = &step.decision {
                    apr_core::routing::evaluate_decision(policy, &input, &outputs, &config, None)
                        .map_err(RuntimeError::from)
                        .and_then(|result| Ok(serde_json::to_value(result)?))
                } else if let Some(join) = &step.join {
                    outputs
                        .get(&join.decision)
                        .and_then(|decision| decision["route"].as_str())
                        .ok_or_else(|| RuntimeError::State("join has no committed decision".into()))
                        .and_then(|route| {
                            Ok(apr_core::routing::resolve_join(
                                join, route, &input, &outputs, &config,
                            )?)
                        })
                } else {
                    Err(RuntimeError::State("missing routing definition".into()))
                };
                match result {
                    Ok(output) => self.commit_control(run_id, &step, &output, manifest)?,
                    Err(error) => {
                        self.fail_step(run_id, &step.id, &error)?;
                        return Ok(());
                    }
                }
                continue;
            }
            let inputs = match resolve_bindings_with_context(
                &step.inputs,
                &input,
                &outputs,
                &config,
                record.item_value.as_ref(),
            ) {
                Ok(inputs) => inputs,
                Err(e) => {
                    self.fail_step(run_id, &step.id, &RuntimeError::from(e))?;
                    return Ok(());
                }
            };
            if step.kind == "call" {
                if let Err(error) = self.create_child(run_id, &step, &inputs) {
                    self.fail_step(run_id, &step.id, &error)?;
                    return Ok(());
                }
            } else if step.kind == "agent" {
                if let Err(error) = bounded_handoff(&step, &inputs).and_then(|_| {
                    self.checked_refs(&inputs, manifest.limits.max_artifact_bytes)
                        .map(|_| ())
                }) {
                    self.fail_step(run_id, &step.id, &error)?;
                    return Ok(());
                }
                self.create_handoff(run_id, &step, &inputs, manifest)?;
                // Other independent pure steps may still run before returning the saved request.
            } else if matches!(step.kind.as_str(), "script" | "connector") {
                self.execute_external(run_id, &step, &inputs, manifest, &config)?;
                if matches!(
                    self.ensure_run(run_id)?.as_str(),
                    "blocked" | "needs_reconciliation" | "failed"
                ) {
                    return Ok(());
                }
            } else {
                self.execute_builtin(run_id, &step, &inputs, manifest)?;
            }
        }
    }

    fn commit_control(
        &mut self,
        run_id: &str,
        step: &Step,
        output: &Value,
        manifest: &ProcessManifest,
    ) -> Result<()> {
        bounded(output)?;
        let refs = self.checked_refs(output, manifest.limits.max_artifact_bytes)?;
        let tx = self.db.transaction()?;
        let changed=tx.execute("UPDATE steps SET state='succeeded',output=?3,attempt_count=attempt_count+1,fence=fence+1 WHERE run_id=?1 AND id=?2 AND state='queued'",params![run_id,step.id,serde_json::to_string(output)?])?;
        if changed != 1 {
            return Err(RuntimeError::State(
                "routing step lost its dispatch identity".into(),
            ));
        }
        let (attempt, fence): (u32, u32) = tx.query_row(
            "SELECT attempt_count,fence FROM steps WHERE run_id=?1 AND id=?2",
            params![run_id, step.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.execute("INSERT INTO attempts(id,run_id,step_id,number,fence,state,started_at,finished_at) VALUES (?1,?2,?3,?4,?5,'succeeded',?6,?6)",params![new_id("attempt"),run_id,step.id,attempt,fence,now_ms()])?;
        link_artifacts(&tx, run_id, &step.id, &refs)?;
        let kind = if step.kind == "decision" {
            "decision.committed"
        } else {
            "join.completed"
        };
        event(
            &tx,
            run_id,
            kind,
            json!({"step_id":step.id,"route":output["route"],"output_digest":content_digest(output)?}),
        )?;
        tx.commit()?;
        fault_point(&format!("routing_committed:{}", step.id));
        Ok(())
    }

    fn expand_foreach(
        &mut self,
        run_id: &str,
        record: &StepRecord,
        step: &Step,
        context: BindingContext<'_>,
        manifest: &ProcessManifest,
    ) -> Result<()> {
        let definition = step
            .foreach
            .as_ref()
            .ok_or_else(|| RuntimeError::State("missing foreach definition".into()))?;
        let bindings = BTreeMap::from([("items".to_owned(), definition.items.clone())]);
        let resolved = resolve_bindings_with_context(
            &bindings,
            context.input,
            context.outputs,
            context.config,
            None,
        )?;
        let items = resolved["items"].as_array().ok_or_else(|| {
            RuntimeError::InvalidInput("foreach items must resolve to an array".into())
        })?;
        if items.len() > definition.max_items as usize {
            return Err(RuntimeError::LimitExceeded(
                "foreach max_items exceeded".into(),
            ));
        }
        bounded(&resolved["items"])?;
        let refs = self.checked_refs(&resolved["items"], manifest.limits.max_artifact_bytes)?;
        let mut keys = BTreeSet::new();
        let mut children = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let key = item
                .pointer(&definition.key)
                .ok_or_else(|| RuntimeError::InvalidInput("foreach item key is missing".into()))?;
            let valid = match key {
                Value::String(value) => {
                    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
                }
                Value::Number(value) => value.is_i64() || value.is_u64(),
                _ => false,
            };
            if !valid {
                return Err(RuntimeError::InvalidInput(
                    "foreach keys must be nonempty strings of at most 256 bytes or integers".into(),
                ));
            }
            let key_json = serde_json::to_string(key)?;
            if !keys.insert(key_json.clone()) {
                return Err(RuntimeError::InvalidInput(
                    "foreach item keys must be unique".into(),
                ));
            }
            let digest = content_digest(key)?;
            let id = format!("{}[{}]", step.id, &digest[7..]);
            children.push((id, index, key_json, serde_json::to_string(item)?));
        }
        let existing: usize = self.db.query_row(
            "SELECT COUNT(*) FROM steps WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )?;
        let (root, root_limits) = self.root_limits(run_id)?;
        let shared:usize=self.db.query_row("SELECT COUNT(*) FROM steps s JOIN runs r ON r.id=s.run_id WHERE COALESCE(r.root_run_id,r.id)=?1",[root],|row|row.get(0))?;
        if existing.saturating_add(children.len()) > manifest.limits.max_step_instances as usize
            || shared.saturating_add(children.len()) > root_limits.max_step_instances as usize
        {
            return Err(RuntimeError::LimitExceeded(
                "shared step instance budget exhausted".into(),
            ));
        }
        let digest = content_digest(&resolved["items"])?;
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO step_expansions(run_id,definition_id,items_json,items_digest,created_at) VALUES (?1,?2,?3,?4,?5)",
            params![run_id,step.id,serde_json::to_string(&resolved["items"])?,digest,now_ms()])?;
        for (id, index, key, item) in children {
            tx.execute("INSERT INTO steps(run_id,id,definition_id,parent_step_id,position,item_key,item_index,item_value,retry_limit,state) VALUES (?1,?2,?3,?3,?4,?5,?6,?7,?8,'queued')",
                params![run_id,id,step.id,record.position+index+1,key,index,item,step.retry.max_attempts])?;
        }
        tx.execute(
            "UPDATE steps SET state='expanded' WHERE run_id=?1 AND id=?2 AND state='queued'",
            params![run_id, step.id],
        )?;
        link_artifacts(&tx, run_id, &step.id, &refs)?;
        event(
            &tx,
            run_id,
            "foreach.expanded",
            json!({"step_id":step.id,"items":items.len(),"items_digest":digest}),
        )?;
        tx.commit()?;
        fault_point(&format!("foreach_expanded:{}", step.id));
        Ok(())
    }

    fn complete_foreach(
        &mut self,
        run_id: &str,
        parent: &StepRecord,
        records: &[StepRecord],
        manifest: &ProcessManifest,
    ) -> Result<()> {
        let children: Vec<_> = records
            .iter()
            .filter(|child| child.parent_step_id.as_deref() == Some(&parent.id))
            .collect();
        let mut bytes = 12usize; // {"items":[]} plus item object wrappers below.
        for child in &children {
            bytes = bytes
                .saturating_add(serde_json::to_vec(&child.output)?.len())
                .saturating_add(serde_json::to_vec(&child.item_key)?.len())
                .saturating_add(20);
            if bytes > 64 * 1024 {
                return Err(RuntimeError::LimitExceeded("foreach aggregate exceeds 64 KiB; return artifact references for large results".into()));
            }
        }
        let output = json!({"items":children.iter().map(|child|json!({"key":child.item_key,"output":child.output})).collect::<Vec<_>>()});
        bounded(&output)?;
        let refs = self.checked_refs(&output, manifest.limits.max_artifact_bytes)?;
        let tx = self.db.transaction()?;
        tx.execute("UPDATE steps SET state='succeeded',output=?3 WHERE run_id=?1 AND id=?2 AND state='expanded'",params![run_id,parent.id,serde_json::to_string(&output)?])?;
        link_artifacts(&tx, run_id, &parent.id, &refs)?;
        event(
            &tx,
            run_id,
            "foreach.completed",
            json!({"step_id":parent.id,"items":children.len(),"output_digest":content_digest(&output)?}),
        )?;
        tx.commit()?;
        Ok(())
    }

    fn fail_step(&mut self, run_id: &str, step_id: &str, error: &RuntimeError) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE steps SET state='failed',active_attempt=NULL WHERE run_id=?1 AND id=?2",
            params![run_id, step_id],
        )?;
        tx.execute("UPDATE attempts SET state='failed',finished_at=?3,error=?4 WHERE run_id=?1 AND step_id=?2 AND state IN ('running','waiting_for_child')",params![run_id,step_id,now_ms(),error.to_string()])?;
        tx.execute(
            "UPDATE runs SET state='failed',error=?2 WHERE id=?1",
            params![run_id, error.to_string()],
        )?;
        event(
            &tx,
            run_id,
            "step.failed",
            json!({"step_id":step_id,"error":error.to_string()}),
        )?;
        tx.commit()?;
        Ok(())
    }

    fn recover_interrupted(&mut self, run_id: &str, manifest: &ProcessManifest) -> Result<()> {
        for record in self
            .step_records(run_id)?
            .into_iter()
            .filter(|r| r.state == "running")
        {
            let queued_here = self.external_queue.as_ref().is_some_and(|queue| {
                queue.iter().any(|job| {
                    job.run == run_id
                        && record.active_attempt.as_deref() == Some(job.intent_id.as_str())
                })
            });
            let leased = match record.active_attempt.as_deref() {
                Some(attempt) => {
                    self.external_pool
                        .as_ref()
                        .is_some_and(|pool| pool.contains(attempt))
                        || crate::execution::claim_is_live(&self.root, attempt)?
                }
                None => false,
            };
            if queued_here || leased {
                // A live worker owns its durable claim lease. Other runtimes
                // must not recover it merely because they can acquire the
                // short-lived state writer lock.
                continue;
            }
            let step = manifest
                .steps
                .iter()
                .find(|s| s.id == record.definition_id)
                .ok_or_else(|| {
                    RuntimeError::State("saved step absent from frozen process".into())
                })?;
            if matches!(step.kind.as_str(), "script" | "connector") {
                self.recover_effect(run_id, &record.id)?;
                continue;
            }
            let exhausted = record.attempt_count >= record.retry_limit || step.effect != "pure";
            let tx = self.db.transaction()?;
            tx.execute("UPDATE attempts SET state='interrupted',finished_at=?2,error='executor stopped before commit' WHERE id=?1 AND state='running'", params![record.active_attempt,now_ms()])?;
            tx.execute("UPDATE steps SET state=?3,active_attempt=NULL,fence=fence+1 WHERE run_id=?1 AND id=?2", params![run_id,record.id,if exhausted {"failed"} else {"queued"}])?;
            if exhausted {
                tx.execute("UPDATE runs SET state='failed',error='interrupted step exhausted its retry budget' WHERE id=?1",[run_id])?;
            }
            event(
                &tx,
                run_id,
                "step.interrupted",
                json!({"step_id":record.id,"retry_available":!exhausted}),
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    fn execute_builtin(
        &mut self,
        run_id: &str,
        step: &Step,
        inputs: &Value,
        manifest: &ProcessManifest,
    ) -> Result<()> {
        let attempt_id = new_id("attempt");
        let tx = self.db.transaction()?;
        tx.execute("UPDATE steps SET state='running',attempt_count=attempt_count+1,fence=fence+1,active_attempt=?3 WHERE run_id=?1 AND id=?2 AND state='queued'", params![run_id,step.id,attempt_id])?;
        let (number, fence): (u32, i64) = tx.query_row(
            "SELECT attempt_count,fence FROM steps WHERE run_id=?1 AND id=?2",
            params![run_id, step.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        tx.execute("INSERT INTO attempts(id,run_id,step_id,number,fence,state,started_at) VALUES (?1,?2,?3,?4,?5,'running',?6)", params![attempt_id,run_id,step.id,number,fence,now_ms()])?;
        tx.execute(
            "UPDATE runs SET state='running',error=NULL WHERE id=?1",
            [run_id],
        )?;
        event(
            &tx,
            run_id,
            "step.started",
            json!({"step_id":step.id,"attempt":number}),
        )?;
        tx.commit()?;
        fault_point(&format!("step_started:{}", step.id));
        let started = Instant::now();
        let result = self
            .checked_refs(inputs, manifest.limits.max_artifact_bytes)
            .and_then(|_| self.builtin(step, inputs, manifest.limits.max_artifact_bytes))
            .and_then(|output| {
                if started.elapsed().as_millis() > u128::from(manifest.limits.max_step_ms) {
                    return Err(RuntimeError::LimitExceeded(
                        "step exceeded max_step_ms before commit".into(),
                    ));
                }
                bounded(&output)?;
                let refs = self.checked_refs(&output, manifest.limits.max_artifact_bytes)?;
                Ok((output, refs))
            });
        fault_point(&format!("step_executed:{}", step.id));
        let tx = self.db.transaction()?;
        let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM steps WHERE run_id=?1 AND id=?2 AND active_attempt=?3 AND fence=?4 AND state='running')", params![run_id,step.id,attempt_id,fence],|r|r.get(0))?;
        if !owned {
            return Err(RuntimeError::State("stale attempt cannot commit".into()));
        }
        match result {
            Ok((output, refs)) => {
                link_artifacts(&tx, run_id, &step.id, &refs)?;
                tx.execute("UPDATE steps SET state='succeeded',output=?3,active_attempt=NULL WHERE run_id=?1 AND id=?2", params![run_id,step.id,serde_json::to_string(&output)?])?;
                tx.execute(
                    "UPDATE attempts SET state='succeeded',finished_at=?2 WHERE id=?1",
                    params![attempt_id, now_ms()],
                )?;
                event(
                    &tx,
                    run_id,
                    "step.succeeded",
                    json!({"step_id":step.id,"output_digest":content_digest(&output)?}),
                )?;
            }
            Err(error) => {
                let exhausted = number >= step.retry.max_attempts;
                tx.execute(
                    "UPDATE steps SET state=?3,active_attempt=NULL WHERE run_id=?1 AND id=?2",
                    params![run_id, step.id, if exhausted { "failed" } else { "queued" }],
                )?;
                tx.execute(
                    "UPDATE attempts SET state='failed',finished_at=?2,error=?3 WHERE id=?1",
                    params![attempt_id, now_ms(), error.to_string()],
                )?;
                if exhausted {
                    tx.execute(
                        "UPDATE runs SET state='failed',error=?2 WHERE id=?1",
                        params![run_id, error.to_string()],
                    )?;
                }
                event(
                    &tx,
                    run_id,
                    "step.failed",
                    json!({"step_id":step.id,"code":error.code(),"retry_available":!exhausted}),
                )?;
            }
        }
        tx.commit()?;
        fault_point(&format!("step_committed:{}", step.id));
        Ok(())
    }

    fn create_handoff(
        &mut self,
        run_id: &str,
        step: &Step,
        inputs: &Value,
        manifest: &ProcessManifest,
    ) -> Result<()> {
        bounded_handoff(step, inputs)?;
        let count: u32 = self.db.query_row(
            "SELECT COUNT(*) FROM handoffs WHERE run_id=?1",
            [run_id],
            |r| r.get(0),
        )?;
        let (root, limits) = self.root_limits(run_id)?;
        let shared_count:u32=self.db.query_row("SELECT COUNT(*) FROM handoffs h JOIN runs r ON r.id=h.run_id WHERE COALESCE(r.root_run_id,r.id)=?1",[root],|row|row.get(0))?;
        if count >= manifest.limits.max_agent_requests || shared_count >= limits.max_agent_requests
        {
            self.set_run_state(run_id, "failed", Some("max_agent_requests exhausted"))?;
            return Ok(());
        }
        let id = new_id("request");
        let token = self.persist_token(&id, 1)?;
        let schema = step
            .result_schema
            .as_ref()
            .ok_or_else(|| RuntimeError::State("missing admitted result schema".into()))?;
        let refs = self.checked_refs(inputs, manifest.limits.max_artifact_bytes)?;
        let attempt = new_id("attempt");
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO handoffs(id,run_id,step_id,generation,input,result_schema,token_hash,expires_at,state) VALUES (?1,?2,?3,1,?4,?5,?6,?7,'pending')", params![id,run_id,step.id,serde_json::to_string(inputs)?,serde_json::to_string(schema)?,hash_token(&token),now_ms()+86_400_000])?;
        if tx.execute("UPDATE steps SET state='waiting_for_agent',attempt_count=attempt_count+1,fence=fence+1,active_attempt=?3 WHERE run_id=?1 AND id=?2 AND state='queued'",params![run_id,step.id,attempt])?!=1 {return Err(RuntimeError::State("caller step lost its dispatch identity".into()));}
        let (number, fence): (u32, u32) = tx.query_row(
            "SELECT attempt_count,fence FROM steps WHERE run_id=?1 AND id=?2",
            params![run_id, step.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.execute("INSERT INTO attempts(id,run_id,step_id,number,fence,state,started_at) VALUES (?1,?2,?3,?4,?5,'waiting_for_agent',?6)",params![attempt,run_id,step.id,number,fence,now_ms()])?;
        link_artifacts(&tx, run_id, &step.id, &refs)?;
        tx.execute(
            "UPDATE runs SET state='waiting_for_agent',error=NULL WHERE id=?1",
            [run_id],
        )?;
        event(
            &tx,
            run_id,
            "handoff.created",
            json!({"request_id":id,"step_id":step.id,"generation":1}),
        )?;
        tx.commit()?;
        fault_point(&format!("handoff_committed:{}", step.id));
        Ok(())
    }

    fn persist_token(&self, id: &str, generation: u32) -> Result<String> {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let mut file = tempfile::NamedTempFile::new_in(self.root.join("handoffs"))?;
        file.write_all(token.as_bytes())?;
        file.as_file().sync_all()?;
        match file.persist_noclobber(
            self.root
                .join("handoffs")
                .join(format!("{id}.{generation}")),
        ) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A prior executor may have published the next generation before
                // dying before its SQLite commit. Reuse that unexposed token.
                let existing = self.read_token(id, generation)?;
                std::fs::File::open(self.root.join("handoffs"))?.sync_all()?;
                return Ok(existing);
            }
            Err(error) => return Err(error.error.into()),
        }
        std::fs::File::open(self.root.join("handoffs"))?.sync_all()?;
        fault_point("token_published");
        Ok(token)
    }

    fn read_token(&self, id: &str, generation: u32) -> Result<String> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        let file = options.open(
            self.root
                .join("handoffs")
                .join(format!("{id}.{generation}")),
        )?;
        if !file.metadata()?.is_file() {
            return Err(RuntimeError::Integrity("invalid handoff token file".into()));
        }
        let mut token = String::new();
        file.take(65).read_to_string(&mut token)?;
        if token.len() != 64
            || !token
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(RuntimeError::Integrity("invalid handoff token file".into()));
        }
        Ok(token)
    }

    pub(crate) fn set_run_state(
        &mut self,
        run_id: &str,
        state: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let existing: (String, Option<String>) =
            self.db
                .query_row("SELECT state,error FROM runs WHERE id=?1", [run_id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        if existing.0 == state && existing.1.as_deref() == error {
            return Ok(());
        }
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE runs SET state=?2,error=?3 WHERE id=?1",
            params![run_id, state, error],
        )?;
        event(&tx, run_id, &format!("run.{state}"), json!({"error":error}))?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn envelope(&self, run_id: &str) -> Result<RunEnvelope> {
        self.ensure_run(run_id)?;
        let (version, state, revision, output, mut error): (
            String,
            String,
            u64,
            Option<String>,
            Option<String>,
        ) = self.db.query_row(
            "SELECT version,state,revision,output,error FROM runs WHERE id=?1",
            [run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let records = self.step_records(run_id)?;
        let completed_steps = records
            .iter()
            .filter(|r| matches!(r.state.as_str(), "succeeded" | "skipped"))
            .count();
        let (root_run_id,parent_run_id,parent_step_id):(String,Option<String>,Option<String>)=self.db.query_row("SELECT COALESCE(root_run_id,id),parent_run_id,parent_step_id FROM runs WHERE id=?1",[run_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        let mut child_query = self
            .db
            .prepare("SELECT parent_step_id,child_run_id FROM child_runs WHERE parent_run_id=?1")?;
        let children: BTreeMap<String, String> = child_query
            .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let pending: Option<(String,String,u32,String,String,i64)> = self.db.query_row("SELECT id,step_id,generation,input,result_schema,expires_at FROM handoffs WHERE run_id=?1 AND state='pending' ORDER BY rowid LIMIT 1",[run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        let mut pending = pending
            .filter(|_| matches!(state.as_str(), "waiting_for_agent" | "queued" | "running"));
        if pending.is_some() && self.ancestors_hold(run_id, true)? {
            pending = None;
            error = error.or_else(|| Some("ancestor run is paused, blocked or terminal".into()));
        }
        if let Some((id, _, _, _, _, 0)) = &pending {
            let restored: bool = self.db.query_row(
                "SELECT token_hash='' FROM handoffs WHERE id=?1",
                [id],
                |row| row.get(0),
            )?;
            if restored {
                error = Some("caller handoff must be renewed after restore".into());
                pending = None;
            }
        }
        let request = if let Some((id, step_id, generation, input, schema, expires)) = pending {
            let (process, _, _) = self.load_run(run_id)?;
            let step = process
                .manifest
                .steps
                .iter()
                .find(|s| {
                    records
                        .iter()
                        .any(|record| record.id == step_id && record.definition_id == s.id)
                })
                .ok_or_else(|| RuntimeError::State("handoff step missing".into()))?;
            let token = self.read_token(&id, generation)?;
            let expected: String =
                self.db
                    .query_row("SELECT token_hash FROM handoffs WHERE id=?1", [&id], |r| {
                        r.get(0)
                    })?;
            if hash_token(&token) != expected {
                return Err(RuntimeError::Integrity(
                    "saved handoff token hash mismatch; renew the claim".into(),
                ));
            }
            let expires_at = time::OffsetDateTime::from_unix_timestamp(expires / 1000)
                .map_err(|e| RuntimeError::State(e.to_string()))?
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(|e| RuntimeError::State(e.to_string()))?;
            Some(AgentRequest {
                request_id: id,
                step_instance_id: step_id,
                generation,
                capability: step.capability.clone().unwrap_or_default(),
                instructions: step.instructions.clone().unwrap_or_default(),
                input: serde_json::from_str(&input)?,
                result_schema: serde_json::from_str(&schema)?,
                continuation_token: token,
                expires_at,
            })
        } else {
            None
        };
        let summary = format!(
            "{completed_steps}/{} steps complete; {state}",
            records.len()
        );
        let config: String =
            self.db
                .query_row("SELECT config FROM runs WHERE id=?1", [run_id], |row| {
                    row.get(0)
                })?;
        let config_digest = content_digest(&serde_json::from_str(&config)?)?;
        let fixture: bool =
            self.db
                .query_row("SELECT fixture FROM runs WHERE id=?1", [run_id], |row| {
                    row.get(0)
                })?;
        Ok(RunEnvelope {
            protocol_version: "0.1".into(),
            run_id: run_id.into(),
            root_run_id: Some(root_run_id),
            parent_run_id,
            parent_step_id,
            process_version: version,
            config_digest,
            fixture,
            state,
            revision,
            summary,
            completed_steps,
            steps: records
                .into_iter()
                .map(|r| StepSummary {
                    child_run_id: children.get(&r.id).cloned(),
                    id: r.id,
                    state: r.state,
                    attempts: r.attempt_count,
                    definition_id: r.definition_id,
                    parent_step_id: r.parent_step_id,
                    item_key: r.item_key,
                    item_index: r.item_index,
                })
                .collect(),
            request,
            output: output.map(|s| serde_json::from_str(&s)).transpose()?,
            error,
            truncated: false,
        })
    }

    pub(crate) fn checked_refs(&self, value: &Value, max_bytes: u64) -> Result<Vec<ArtifactRef>> {
        let mut refs = Vec::new();
        collect_refs(value, &mut refs)?;
        for reference in &refs {
            self.artifacts.verify(reference, max_bytes)?;
            verify_registered_reference(&self.db, reference)?;
        }
        Ok(refs)
    }

    fn verify_committed(&self, run_id: &str, max_bytes: u64) -> Result<()> {
        let (_, input, config) = self.load_run(run_id)?;
        self.checked_refs(&input, max_bytes)?;
        self.checked_refs(&config, max_bytes)?;
        for record in self.step_records(run_id)? {
            if let Some(output) = record.output {
                self.checked_refs(&output, max_bytes)?;
            }
            if let Some(item) = record.item_value {
                self.checked_refs(&item, max_bytes)?;
            }
        }
        Ok(())
    }

    fn builtin(&self, step: &Step, input: &Value, max_bytes: u64) -> Result<Value> {
        match step.operation.as_deref() {
            Some(
                operation @ ("classifier.collect"
                | "classifier.review"
                | "classifier.candidate"
                | "classifier.evaluation_plan"
                | "classifier.evaluate"),
            ) => Ok(apr_core::classifier::execute(operation, input)?),
            Some("json.write") => {
                let value = required(input, "value")?;
                let bytes = canonical_bytes(value)?;
                Ok(
                    json!({"artifact":self.artifacts.put_with_quota(&bytes,"application/json",max_bytes,self.artifact_quota()?)?}),
                )
            }
            Some("json.select") => {
                let pointer = required(input, "pointer")?
                    .as_str()
                    .ok_or_else(|| RuntimeError::InvalidInput("pointer must be a string".into()))?;
                let value = required(input, "value")?.pointer(pointer).ok_or_else(|| {
                    RuntimeError::InvalidInput("JSON pointer does not exist".into())
                })?;
                Ok(json!({"value":value}))
            }
            Some("artifact.manifest") => {
                let refs: Vec<ArtifactRef> =
                    serde_json::from_value(required(input, "sources")?.clone())?;
                for reference in &refs {
                    self.artifacts.verify(reference, max_bytes)?;
                }
                let bytes = canonical_bytes(&json!({"schema_version":"0.1","artifacts":refs}))?;
                Ok(
                    json!({"artifact":self.artifacts.put_with_quota(&bytes,"application/json",max_bytes,self.artifact_quota()?)?}),
                )
            }
            Some("artifact.validate_set") => {
                let items = required(input, "items")?
                    .as_array()
                    .ok_or_else(|| RuntimeError::InvalidInput("items must be an array".into()))?;
                let mut refs = Vec::new();
                let mut keys = std::collections::BTreeSet::new();
                for item in items {
                    let reference = if let Some(key) = item.get("key") {
                        let key = key.as_str().ok_or_else(|| {
                            RuntimeError::InvalidInput("item key must be a string".into())
                        })?;
                        if !keys.insert(key) {
                            return Err(RuntimeError::InvalidInput(
                                "duplicate artifact item key".into(),
                            ));
                        }
                        required(required(item, "output")?, "image")?
                    } else {
                        item
                    };
                    let reference: ArtifactRef = serde_json::from_value(reference.clone())?;
                    self.artifacts.verify(&reference, max_bytes)?;
                    if let Some(prefix) = input.get("media_type_prefix").and_then(Value::as_str)
                        && !reference.media_type.starts_with(prefix)
                    {
                        return Err(RuntimeError::InvalidInput(
                            "artifact media type does not match expected prefix".into(),
                        ));
                    }
                    refs.push(reference);
                }
                Ok(json!({"report":{"valid":true,"count":refs.len()},"artifacts":refs}))
            }
            Some("artifact.bundle") => {
                if input.get("format").and_then(Value::as_str).unwrap_or("zip") != "zip" {
                    return Err(RuntimeError::InvalidInput(
                        "only ZIP bundles are supported".into(),
                    ));
                }
                let refs: Vec<ArtifactRef> =
                    serde_json::from_value(required(input, "artifacts")?.clone())?;
                Ok(
                    json!({"artifact":self.artifacts.bundle_with_quota(&refs,input.get("report"),max_bytes,self.artifact_quota()?)?}),
                )
            }
            _ => Err(RuntimeError::State("operation was not admitted".into())),
        }
    }
}

fn json_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<Value>> {
    row.get::<_, Option<String>>(index)?
        .map(|text| {
            serde_json::from_str(&text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    value
        .get(key)
        .ok_or_else(|| RuntimeError::InvalidInput(format!("missing builtin input: {key}")))
}

pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

pub(crate) fn check_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 256 || key.chars().any(char::is_control) {
        return Err(RuntimeError::InvalidInput(
            "idempotency key must contain 1–256 bytes without control characters".into(),
        ));
    }
    Ok(())
}

pub(crate) fn bounded(value: &Value) -> Result<()> {
    if serde_json::to_vec(value)?.len() > MAX_JSON_BYTES {
        return Err(RuntimeError::LimitExceeded(
            "JSON payload exceeds 64 KiB; import large content as an artifact".into(),
        ));
    }
    Ok(())
}

pub(crate) fn bounded_detail(value: &Value) -> Result<()> {
    if serde_json::to_vec(value)?.len() > MAX_DETAIL_PAYLOAD_BYTES {
        return Err(RuntimeError::LimitExceeded(
            "agent detail payload exceeds 48 KiB; reduce the schema or use artifact references for data".into(),
        ));
    }
    Ok(())
}

pub(crate) fn bounded_handoff(step: &Step, inputs: &Value) -> Result<()> {
    // Include every manifest- or input-controlled request field. The remaining
    // claim IDs, generation, token and timestamp have fixed, small encodings.
    bounded_detail(&json!({
        "step_instance_id": step.id,
        "capability": step.capability,
        "instructions": step.instructions,
        "input": inputs,
        "result_schema": step.result_schema,
    }))
}

pub(crate) fn collect_refs(value: &Value, refs: &mut Vec<ArtifactRef>) -> Result<()> {
    match value {
        Value::Object(map) if map.contains_key("artifact_id") => {
            refs.push(serde_json::from_value(value.clone())?)
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_refs(value, refs)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_refs(value, refs)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn link_artifacts(
    db: &Connection,
    run_id: &str,
    step_id: &str,
    refs: &[ArtifactRef],
) -> Result<()> {
    for reference in refs {
        verify_registered_reference(db, reference)?;
        db.execute(
            "INSERT OR IGNORE INTO artifacts(id,reference) VALUES (?1,?2)",
            params![reference.artifact_id, serde_json::to_string(reference)?],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO artifact_links(run_id,step_id,artifact_id) VALUES (?1,?2,?3)",
            params![run_id, step_id, reference.artifact_id],
        )?;
    }
    Ok(())
}

pub(crate) fn verify_registered_reference(db: &Connection, reference: &ArtifactRef) -> Result<()> {
    let registered: Option<String> = db
        .query_row(
            "SELECT reference FROM artifacts WHERE id=?1",
            [&reference.artifact_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(registered) = registered {
        let registered: ArtifactRef = serde_json::from_str(&registered)?;
        if &registered != reference {
            return Err(RuntimeError::Integrity(
                "artifact metadata differs from its registered reference".into(),
            ));
        }
    }
    Ok(())
}

/// The common transactional admission record used by direct and scheduled runs.
/// All executable snapshots and authority have been checked before this helper.
pub(crate) struct RunRecord<'a> {
    pub process: &'a AdmittedProcess,
    pub input: &'a Value,
    pub request_key: &'a str,
    pub assignment_id: Option<&'a str>,
    pub config: &'a crate::scope::ResolvedConfig,
    pub config_overrides: &'a Value,
    pub fixture: bool,
}

pub(crate) fn insert_run_record(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    record: RunRecord<'_>,
) -> Result<()> {
    let RunRecord {
        process,
        input,
        request_key,
        assignment_id,
        config,
        config_overrides,
        fixture,
    } = record;
    let request_digest = content_digest(
        &json!({"version":process.version,"input":input,"config":config.value,"config_overrides":config_overrides,"fixture":fixture}),
    )?;
    tx.execute("INSERT OR IGNORE INTO process_versions(version,manifest,canonical_format) VALUES (?1,?2,?3)",
        params![process.version, String::from_utf8_lossy(&process.canonical_bytes), apr_core::CANONICAL_FORMAT])?;
    tx.execute("INSERT INTO runs(id,root_run_id,request_key,request_digest,version,input,state,created_at,assignment_id,config,config_overrides,config_project_revision,config_assignment_revision,fixture) VALUES (?1,?1,?2,?3,?4,?5,'queued',?6,?7,?8,?9,?10,?11,?12)",
        params![id,request_key,request_digest,process.version,serde_json::to_string(input)?,now_ms(),assignment_id,
            serde_json::to_string(&config.value)?,serde_json::to_string(config_overrides)?,config.project_revision,config.assignment_revision,fixture])?;
    for (position, step) in process.manifest.steps.iter().enumerate() {
        tx.execute("INSERT INTO steps(run_id,id,definition_id,position,state,retry_limit) VALUES (?1,?2,?2,?3,'queued',?4)",
            params![id,step.id,position * 1001,step.retry.max_attempts])?;
    }
    Ok(())
}
