//! Assignment authority and the small agent-facing catalog/run interface.
//!
//! Scope comes only from a locally issued attachment. IDs and SHA-256 digests
//! supplied in tool arguments never grant access by themselves.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use apr_core::{
    AdmittedProcess, ArtifactRef, Binding, MAX_MANIFEST_BYTES, ProcessManifest, admit_json,
    content_digest, validate_instance,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::engine::{
    StartOptions, bounded, bounded_detail, bounded_handoff, check_key, collect_refs,
    verify_registered_reference,
};
use crate::store::{StoreLock, event, now_ms};
use crate::{Result, Runtime, RuntimeError, Submission};

const ATTACHMENT_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ARTIFACT_PREVIEW_BYTES: u64 = 32 * 1024;

// A terminal child wakes its waiting parent without a separate notification.
// Ancestor controls apply at every boundary, while preserving each child's state.
const DISPATCH_ELIGIBLE: &str = "(r.state IN ('queued','running') OR (r.state IN ('waiting_for_child','waiting_for_agent') AND EXISTS(SELECT 1 FROM child_runs c JOIN runs child ON child.id=c.child_run_id JOIN steps s ON s.run_id=c.parent_run_id AND s.id=c.parent_step_id WHERE c.parent_run_id=r.id AND s.state='waiting_for_child' AND child.state IN ('succeeded','failed','cancelled')))) AND NOT EXISTS(WITH RECURSIVE ancestors(id,parent_run_id,state) AS (SELECT a.id,a.parent_run_id,a.state FROM runs a WHERE a.id=r.parent_run_id UNION ALL SELECT a.id,a.parent_run_id,a.state FROM runs a JOIN ancestors previous ON a.id=previous.parent_run_id) SELECT 1 FROM ancestors WHERE state IN ('paused','blocked','failed','cancelled','needs_reconciliation'))";

struct Scope {
    attachment_id: String,
    assignment_id: String,
    project_root: PathBuf,
    mode: String,
    device: String,
    inode: String,
}

pub(crate) struct ResolvedConfig {
    pub value: Value,
    pub project_revision: u64,
    pub assignment_revision: u64,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoverArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    process_ref: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    process_ref: String,
    input: Value,
    request_key: String,
    mode: String,
    #[serde(default = "empty_object")]
    config_overrides: Value,
}

#[derive(Deserialize)]
#[serde(tag = "view", rename_all = "snake_case", deny_unknown_fields)]
enum InspectArgs {
    Improvements {
        process_ref: String,
    },
    Improvement {
        cycle_id: String,
    },
    ImprovementEvidence {
        process_ref: String,
        run_ids: Vec<String>,
    },
    LearningCases {
        process_ref: String,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        after: Option<String>,
    },
    McpApp {
        run_id: String,
        resource_step_id: String,
        tool_step_id: String,
    },
    PriceSources,
    McpInteractions {
        #[serde(default)]
        after: Option<String>,
    },
    Billing {
        starts_at_ms: i64,
        ends_at_ms: i64,
    },
    Budget,
    Approvals,
    ConnectionLibrary {
        #[serde(default)]
        after: Option<String>,
    },
    LibraryContract {
        library_id: String,
        revision: u64,
    },
    ConnectionContract {
        name: String,
    },
    Readiness {
        process_ref: String,
    },
    Analytics {
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        after: Option<String>,
    },
    Capacity,
    Workers,
    TreeStatus {
        run_id: String,
    },
    Graph {
        #[serde(default)]
        process_ref: Option<String>,
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        after: Option<String>,
    },
    Tree {
        run_id: String,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        after: Option<String>,
    },
    JevTuning {
        process_ref: String,
    },
    Schedule {
        schedule_id: String,
    },
    Schedules {
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        after: Option<String>,
    },
    Effects {
        run_id: String,
    },
    Bindings,
    Run {
        run_id: String,
    },
    Config {
        run_id: String,
    },
    Runs {
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        after: Option<String>,
    },
    Pending {
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Events {
        run_id: String,
        #[serde(default)]
        after: Option<u64>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Artifact {
        reference: ArtifactRef,
    },
}

#[derive(Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
enum ManageArgs {
    #[serde(rename = "improvement.open")]
    ImprovementOpen {
        request_key: String,
        process_ref: String,
        run_ids: Vec<String>,
    },
    #[serde(rename = "improvement.propose")]
    ImprovementPropose {
        cycle_id: String,
        proposal: Value,
        candidate_path: String,
        review: apr_core::audit::ReviewInput,
    },
    #[serde(rename = "improvement.freeze")]
    ImprovementFreeze {
        cycle_id: String,
        policy: Value,
        development: Value,
        holdout: Value,
    },
    #[serde(rename = "improvement.evaluate")]
    ImprovementEvaluate { cycle_id: String },
    #[serde(rename = "improvement.close")]
    ImprovementClose {
        cycle_id: String,
        outcome: String,
        reason: String,
    },
    #[serde(rename = "improvement.decide")]
    ImprovementDecide {
        cycle_id: String,
        decision: String,
        candidate_path: String,
        expected_report_digest: String,
        expected_selected_ref: Option<String>,
    },
    #[serde(rename = "connection.use")]
    ConnectionUse {
        library_id: String,
        revision: u64,
        name: String,
    },
    #[serde(rename = "process.audit")]
    ProcessAudit {
        path: String,
        review: apr_core::audit::ReviewInput,
    },
    #[serde(rename = "connection.propose")]
    ConnectionPropose(crate::setup::ConnectionProposal),
    #[serde(rename = "connection.discover")]
    ConnectionDiscover {
        resource: String,
        #[serde(default)]
        client_id: String,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        metadata_url: Option<String>,
    },
    #[serde(rename = "cost.preview")]
    CostPreview {
        connection: apr_connectors::Connection,
        request: Value,
        response: Value,
    },
    #[serde(rename = "process.tune_jev")]
    ProcessTuneJev(crate::tuning::TuneJevArgs),
    #[serde(rename = "schedule.create")]
    ScheduleCreate(crate::schedule::ScheduleCreate),
    #[serde(rename = "schedule.pause")]
    SchedulePause { schedule_id: String },
    #[serde(rename = "schedule.resume")]
    ScheduleResume { schedule_id: String },
    #[serde(rename = "schedule.delete")]
    ScheduleDelete { schedule_id: String },
    #[serde(rename = "process.validate")]
    ProcessValidate { path: String },
    #[serde(rename = "process.register")]
    ProcessRegister { path: String },
    #[serde(rename = "process.select")]
    ProcessSelect {
        process_ref: String,
        expected_selected_ref: Option<String>,
        expected_selection_revision: u64,
    },
    #[serde(rename = "artifact.import")]
    ArtifactImport { path: String, media_type: String },
    #[serde(rename = "config.read")]
    ConfigRead { process_ref: String },
    #[serde(rename = "config.set")]
    ConfigSet { process_ref: String, values: Value },
    #[serde(rename = "process.test")]
    ProcessTest {
        path: String,
        input: Value,
        request_key: String,
        #[serde(default = "empty_object")]
        config_overrides: Value,
    },
    #[serde(rename = "process.promote")]
    ProcessPromote {
        process_ref: String,
        target_assignment_id: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ResumeControl {
    Renew {
        run_id: String,
    },
    Continue {
        run_id: String,
    },
    Cancel {
        run_id: String,
    },
    Pause {
        run_id: String,
    },
    Retry {
        run_id: String,
        step_instance_id: String,
    },
}

impl Runtime {
    /// Owner authorization for a source assignment to copy pinned catalog
    /// entries into one same-project destination. This grants no execution or
    /// artifact authority and is never available through an attachment call.
    pub fn set_promotion_grant(
        &mut self,
        source_assignment_id: &str,
        target_assignment_id: &str,
        enabled: bool,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.require_same_project(source_assignment_id, target_assignment_id)?;
        if enabled {
            self.ensure_assignment_active(source_assignment_id)?;
            self.ensure_assignment_active(target_assignment_id)?;
            self.db.execute("INSERT OR IGNORE INTO promotion_grants(source_assignment_id,target_assignment_id,created_at) VALUES (?1,?2,?3)",
                params![source_assignment_id,target_assignment_id,now_ms()])?;
        } else {
            self.db.execute("DELETE FROM promotion_grants WHERE source_assignment_id=?1 AND target_assignment_id=?2",params![source_assignment_id,target_assignment_id])?;
        }
        Ok(
            json!({"source_assignment_id":source_assignment_id,"target_assignment_id":target_assignment_id,"enabled":enabled}),
        )
    }

    fn require_same_project(&self, source: &str, target: &str) -> Result<()> {
        let same:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM assignments s JOIN assignments t ON s.project_id=t.project_id WHERE s.id=?1 AND t.id=?2)",
            params![source,target],|row|row.get(0))?;
        if !same {
            return Err(RuntimeError::Forbidden);
        }
        Ok(())
    }

    /// Trusted owner control for ordinary project settings, not credential data.
    pub fn set_project_config(
        &mut self,
        project_id: &str,
        process_ref: &str,
        values: Value,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let source: Option<String> = self.db.query_row(
            "SELECT c.assignment_id FROM catalog c JOIN assignments a ON a.id=c.assignment_id WHERE a.project_id=?1 AND c.version=?2 AND a.state='active' LIMIT 1",
            params![project_id,process_ref], |row|row.get(0)).optional()?;
        let source = source.ok_or(RuntimeError::Forbidden)?;
        self.ensure_assignment_active(&source)?;
        let process = self.catalog_process(&source, process_ref)?;
        let candidate = overlay_config(&process.manifest.config.defaults, &values)?;
        validate_instance(&process.manifest.config.schema, &candidate)
            .map_err(|error| RuntimeError::InvalidInput(error.to_string()))?;
        bounded_detail(&json!({"config":candidate,"config_overrides":{}}))?;
        self.write_config_layer("project", project_id, process_ref, &values)?;
        let (_, revision) = self.config_layer("project", project_id, process_ref)?;
        Ok(
            json!({"process_ref":process_ref,"project_id":project_id,"project_revision":revision,"config_digest":content_digest(&candidate)?}),
        )
    }

    fn config_layer(&self, kind: &str, scope_id: &str, version: &str) -> Result<(Value, u64)> {
        let row: Option<(String,u64)> = self.db.query_row(
            "SELECT values_json,revision FROM process_config WHERE scope_kind=?1 AND scope_id=?2 AND version=?3",
            params![kind,scope_id,version],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        row.map(|(values, revision)| Ok((serde_json::from_str(&values)?, revision)))
            .unwrap_or_else(|| Ok((json!({}), 0)))
    }

    fn write_config_layer(
        &self,
        kind: &str,
        scope_id: &str,
        version: &str,
        values: &Value,
    ) -> Result<()> {
        self.db.execute("INSERT INTO process_config(scope_kind,scope_id,version,values_json,revision) VALUES (?1,?2,?3,?4,1) ON CONFLICT(scope_kind,scope_id,version) DO UPDATE SET values_json=excluded.values_json,revision=process_config.revision+1",
            params![kind,scope_id,version,serde_json::to_string(values)?])?;
        Ok(())
    }

    pub(crate) fn resolve_configuration(
        &self,
        process: &AdmittedProcess,
        assignment: Option<&str>,
        overrides: &Value,
    ) -> Result<ResolvedConfig> {
        let overrides_map = overrides.as_object().ok_or_else(|| {
            RuntimeError::InvalidInput("config overrides must be an object".into())
        })?;
        for key in overrides_map.keys() {
            if !process.manifest.config.run_override_keys.contains(key) {
                return Err(RuntimeError::InvalidInput(format!(
                    "config override key {key:?} is not permitted"
                )));
            }
        }
        let mut config = process.manifest.config.defaults.clone();
        let (mut project_revision, mut assignment_revision) = (0, 0);
        if let Some(assignment) = assignment {
            let project: String = self.db.query_row(
                "SELECT project_id FROM assignments WHERE id=?1",
                [assignment],
                |row| row.get(0),
            )?;
            let (values, revision) = self.config_layer("project", &project, &process.version)?;
            config = overlay_config(&config, &values)?;
            project_revision = revision;
            let (values, revision) =
                self.config_layer("assignment", assignment, &process.version)?;
            config = overlay_config(&config, &values)?;
            assignment_revision = revision;
        }
        config = overlay_config(&config, overrides)?;
        validate_instance(&process.manifest.config.schema, &config)
            .map_err(|error| RuntimeError::InvalidInput(error.to_string()))?;
        bounded_detail(&json!({"config":config,"config_overrides":overrides}))?;
        Ok(ResolvedConfig {
            value: config,
            project_revision,
            assignment_revision,
        })
    }

    /// Trusted owner control; this operation is never exposed by scoped_call.
    pub fn attach_project(&mut self, root: &Path) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let root = root.canonicalize()?;
        let (device, inode) = directory_identity(&root)?;
        let root_text = root
            .to_str()
            .ok_or_else(|| RuntimeError::InvalidInput("project path must be UTF-8".into()))?;
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT id FROM projects WHERE root=?1 AND device=?2 AND inode=?3",
                params![root_text, device, inode],
                |row| row.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing {
            id
        } else {
            let id = id("project");
            self.db.execute(
                "INSERT INTO projects(id,root,device,inode,created_at) VALUES (?1,?2,?3,?4,?5)",
                params![id, root_text, device, inode, now_ms()],
            )?;
            id
        };
        Ok(json!({"protocol_version":"0.1","project_id":id,"root":root_text}))
    }

    pub fn create_assignment(&mut self, project_id: &str, name: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        if name.trim().is_empty() || name.len() > 160 || name.chars().any(char::is_control) {
            return Err(RuntimeError::InvalidInput(
                "assignment name must contain 1–160 bytes without control characters".into(),
            ));
        }
        let project: Option<(String, String, String)> = self
            .db
            .query_row(
                "SELECT root,device,inode FROM projects WHERE id=?1",
                [project_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (root, device, inode) =
            project.ok_or_else(|| RuntimeError::NotFound("project is not registered".into()))?;
        verify_directory(&root, &device, &inode)?;
        let id = id("assignment");
        self.db.execute("INSERT INTO assignments(id,project_id,name,state,created_at) VALUES (?1,?2,?3,'active',?4)", params![id,project_id,name,now_ms()])?;
        Ok(
            json!({"protocol_version":"0.1","assignment_id":id,"project_id":project_id,"name":name,"state":"active"}),
        )
    }

    pub fn create_attachment(&mut self, assignment_id: &str, mode: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        if !matches!(mode, "author" | "run") {
            return Err(RuntimeError::InvalidInput(
                "mode must be author or run".into(),
            ));
        }
        self.ensure_assignment_active(assignment_id)?;
        let id = id("attachment");
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        self.db.execute("INSERT INTO attachments(id,assignment_id,token_hash,mode,expires_at,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![id,assignment_id,token_hash(&token),mode,now_ms()+ATTACHMENT_TTL_MS,now_ms()])?;
        Ok(
            json!({"protocol_version":"0.1","state_dir":self.root,"attachment_id":id,"assignment_id":assignment_id,"mode":mode,"token":token}),
        )
    }

    pub fn revoke_attachment(&mut self, attachment_id: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        if self.db.execute(
            "UPDATE attachments SET revoked_at=COALESCE(revoked_at,?2) WHERE id=?1",
            params![attachment_id, now_ms()],
        )? != 1
        {
            return Err(RuntimeError::NotFound(
                "attachment is not registered".into(),
            ));
        }
        Ok(json!({"attachment_id":attachment_id,"revoked":true}))
    }

    pub fn close_assignment(&mut self, assignment_id: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let tx = self.db.transaction()?;
        if tx.execute(
            "UPDATE assignments SET state='closed' WHERE id=?1",
            [assignment_id],
        )? != 1
        {
            return Err(RuntimeError::NotFound(
                "assignment is not registered".into(),
            ));
        }
        let runs: Vec<String> = {
            let mut query = tx.prepare("SELECT id FROM runs WHERE assignment_id=?1 AND state NOT IN ('succeeded','failed','cancelled')")?;
            query
                .query_map([assignment_id], |row| row.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        for run_id in &runs {
            cancel_in_transaction(&tx, run_id, "assignment.closed")?;
        }
        tx.execute("UPDATE schedules SET state='paused',last_error='assignment closed',updated_at=?2 WHERE assignment_id=?1 AND state='active'",params![assignment_id,now_ms()])?;
        tx.commit()?;
        Ok(json!({"assignment_id":assignment_id,"state":"closed","cancelled_runs":runs.len()}))
    }

    /// A coherent SQLite read snapshot, independent of the execution writer lock.
    /// Only read actions are accepted; authority is checked inside the snapshot.
    pub(crate) fn scoped_read(&self, token: &str, action: &str, args: Value) -> Result<Value> {
        bounded(&args)?;
        let tx = self.db.unchecked_transaction()?;
        let result = (|| {
            let scope = self.authenticate(token)?;
            if !matches!(action, "inspect" | "discover") {
                self.ensure_not_maintaining()?;
            }
            match action {
                "discover" => self.discover(&scope, serde_json::from_value(args)?),
                "inspect" => self.scoped_inspect(&scope, serde_json::from_value(args)?),
                _ => Err(RuntimeError::Forbidden),
            }
        })();
        tx.rollback()?;
        result
    }

    /// Authenticate and authorize every invocation while holding writer ownership.
    pub fn scoped_call(&mut self, token: &str, action: &str, args: Value) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let scope = self.authenticate(token)?;
        bounded(&args)?;
        match action {
            "discover" => self.discover(&scope, serde_json::from_value(args)?),
            "run" => self.scoped_run(&scope, serde_json::from_value(args)?),
            "inspect" => self.scoped_inspect(&scope, serde_json::from_value(args)?),
            "resume" => self.scoped_resume(&scope, args),
            "manage" => {
                if scope.mode != "author" {
                    return Err(RuntimeError::Forbidden);
                }
                self.scoped_manage(&scope, serde_json::from_value(args)?)
            }
            _ => Err(RuntimeError::InvalidInput(
                "unsupported scoped action".into(),
            )),
        }
    }

    /// Advance one dispatch boundary per eligible run. Release the process lock
    /// between runs so owner revocation/closure can take effect between steps.
    pub fn drive_pending(&mut self) -> Result<usize> {
        if self.external_pool.is_none() && crate::service::supervisor_owns_execution(&self.root)? {
            return Ok(0);
        }
        if self.ensure_not_maintaining().is_err() {
            return Ok(0);
        }
        let ids: Vec<String> = {
            let _lock = StoreLock::acquire(&self.root)?;
            let mut query = self.db.prepare(&format!("SELECT r.id FROM runs r WHERE {DISPATCH_ELIGIBLE} ORDER BY r.created_at,r.id LIMIT 32"))?;
            query
                .query_map([], |row| row.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let mut advanced = 0;
        for run_id in ids {
            let _lock = StoreLock::acquire(&self.root)?;
            if self.ensure_not_maintaining().is_err() {
                break;
            }
            if matches!(
                self.ensure_run(&run_id)?.as_str(),
                "queued" | "running" | "waiting_for_child" | "waiting_for_agent"
            ) {
                if let Err(error) = self.drive_limited(&run_id, 1) {
                    self.set_run_state(&run_id, "blocked", Some(&error.to_string()))?;
                }
                advanced += 1;
            }
        }
        Ok(advanced)
    }

    /// Advance at most one run by one dispatch boundary. A service-owned cursor
    /// rotates through eligible run IDs so controls and shutdown can be checked
    /// between every dispatch without starving a later run.
    pub fn drive_next_pending(&mut self, after: &mut Option<String>) -> Result<bool> {
        if self.external_pool.is_none() && crate::service::supervisor_owns_execution(&self.root)? {
            return Ok(false);
        }
        let _lock = StoreLock::acquire(&self.root)?;
        self.drive_next_pending_inner(after)
    }

    /// Prepare one bounded batch across fair run dispatches, then
    /// overlap only their external work. The coordinator retains StoreLock for
    /// the entire wave, including each fenced result commit. Other writers and
    /// crash recovery can never mistake a live claim for an abandoned attempt.
    pub fn drive_pending_batch(&mut self, after: &mut Option<String>) -> Result<bool> {
        if self.external_pool.is_none() && crate::service::supervisor_owns_execution(&self.root)? {
            return Ok(false);
        }
        let _lock = StoreLock::acquire(&self.root)?;
        if self.ensure_not_maintaining().is_err() {
            return Ok(false);
        }
        if self.external_queue.is_some() {
            return Err(RuntimeError::State(
                "external wave is already active".into(),
            ));
        }
        self.dispatch_policy = Some(self.worker_policy_inner()?);
        self.external_queue = Some(Vec::new());
        let prepared: Result<bool> = (|| {
            let mut advanced = false;
            // Bound pure/control preparation too, so owner requests and
            // schedules regain the coordinator between small waves.
            for _ in 0..32 {
                if !self.drive_next_pending_inner(after)? {
                    break;
                }
                advanced = true;
                if self.external_queue.as_ref().map_or(0, Vec::len)
                    >= self
                        .dispatch_policy
                        .as_ref()
                        .map_or(4, |policy| policy.max_workers as usize)
                {
                    break;
                }
            }
            Ok(advanced)
        })();
        let jobs = self.external_queue.take().unwrap_or_default();
        // Settle every already-reserved job even if a later preparation failed.
        let finished = self.finish_external_wave(jobs);
        self.dispatch_policy = None;
        prepared.and_then(|advanced| {
            finished?;
            // Yielding for an in-flight cancel is not quiescence. If its queue
            // admission or later authority recheck fails, releasing the latch
            // must not strand this or another run until an unrelated write.
            Ok(advanced
                || self.db.query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM runs r WHERE {DISPATCH_ELIGIBLE})"),
                    [],
                    |row| row.get::<_, bool>(0),
                )?)
        })
    }

    /// Service-owned asynchronous dispatch. The writer lock covers only
    /// preparation and thread admission; completion arrives over the pool
    /// channel and is committed separately before a slot is refilled.
    pub(crate) fn drive_pending_pool(&mut self, after: &mut Option<String>) -> Result<bool> {
        let settled = self.settle_external_pool()?;
        let _lock = StoreLock::acquire(&self.root)?;
        if self.ensure_not_maintaining().is_err() {
            return Ok(self.external_pool_len() > 0);
        }
        if self.external_queue.is_some() {
            return Err(RuntimeError::State(
                "external preparation is already active".into(),
            ));
        }
        self.dispatch_policy = Some(self.worker_policy_inner()?);
        let cap = self
            .dispatch_policy
            .as_ref()
            .map_or(4, |policy| policy.max_workers as usize);
        self.external_queue = Some(Vec::new());
        let prepared: Result<bool> = (|| {
            let mut advanced = false;
            if self.external_pool_len() >= cap {
                return Ok(advanced);
            }
            // Keep pure and control dispatch fair and leave room for IPC and
            // schedule work between coordinator turns.
            for _ in 0..32 {
                if !self.drive_next_pending_inner(after)? {
                    break;
                }
                advanced = true;
                if self.external_pool_len() + self.external_queue.as_ref().map_or(0, Vec::len)
                    >= cap
                {
                    break;
                }
            }
            Ok(advanced)
        })();
        let jobs = self.external_queue.take().unwrap_or_default();
        let spawned = self.spawn_external_pool_jobs(jobs);
        self.dispatch_policy = None;
        let advanced = prepared?;
        spawned?;
        Ok(settled
            || advanced
            || self.external_pool_len() > 0
            || self.db.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM runs r WHERE {DISPATCH_ELIGIBLE})"),
                [],
                |row| row.get::<_, bool>(0),
            )?)
    }

    fn drive_next_pending_inner(&mut self, after: &mut Option<String>) -> Result<bool> {
        if self.ensure_not_maintaining().is_err() {
            return Ok(false);
        }
        let mut next: Option<String> = self
            .db
            .query_row(
                &format!("SELECT r.id FROM runs r WHERE {DISPATCH_ELIGIBLE} AND (?1 IS NULL OR r.id>?1) ORDER BY r.id LIMIT 1"),
                [after.as_deref()],
                |row| row.get(0),
            )
            .optional()?;
        if next.is_none() && after.is_some() {
            next = self
                .db
                .query_row(
                    &format!(
                        "SELECT r.id FROM runs r WHERE {DISPATCH_ELIGIBLE} ORDER BY r.id LIMIT 1"
                    ),
                    [],
                    |row| row.get(0),
                )
                .optional()?;
        }
        let Some(run_id) = next else {
            *after = None;
            return Ok(false);
        };
        // An authorized cancel can arrive while this coordinator holds the
        // writer lock. Stop the bounded wave at this family member so the
        // queued durable control can acquire the lock and transition it.
        if self.cancellation_pending(&run_id)? {
            return Ok(false);
        }
        if let Err(error) = self.drive_limited(&run_id, 1) {
            self.set_run_state(&run_id, "blocked", Some(&error.to_string()))?;
        }
        *after = Some(run_id);
        Ok(true)
    }

    fn authenticate(&self, token: &str) -> Result<Scope> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RuntimeError::Unauthorized);
        }
        let found: Option<(String,String,String,String,String,String)> = self.db.query_row(
            "SELECT a.id,p.root,p.device,p.inode,t.mode,t.id FROM attachments t JOIN assignments a ON a.id=t.assignment_id JOIN projects p ON p.id=a.project_id WHERE t.token_hash=?1 AND t.revoked_at IS NULL AND t.expires_at>?2 AND a.state='active'",
            params![token_hash(token),now_ms()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
        let (assignment_id, root, device, inode, mode, attachment_id) =
            found.ok_or(RuntimeError::Unauthorized)?;
        verify_directory(&root, &device, &inode).map_err(|_| RuntimeError::Unauthorized)?;
        Ok(Scope {
            attachment_id,
            assignment_id,
            project_root: root.into(),
            mode,
            device,
            inode,
        })
    }

    pub(crate) fn ensure_assignment_active(&self, assignment: &str) -> Result<()> {
        let project: Option<(String,String,String)> = self.db.query_row(
            "SELECT p.root,p.device,p.inode FROM assignments a JOIN projects p ON p.id=a.project_id WHERE a.id=?1 AND a.state='active'",
            [assignment], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let (root, device, inode) = project.ok_or(RuntimeError::Forbidden)?;
        verify_directory(&root, &device, &inode).map_err(|_| RuntimeError::Forbidden)
    }

    pub(crate) fn ensure_dispatch_authority(&mut self, run_id: &str) -> Result<bool> {
        if self.ancestors_hold(run_id, true)? {
            return Ok(false);
        }
        let assignment: Option<String> = self.db.query_row(
            "SELECT assignment_id FROM runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )?;
        let Some(assignment) = assignment else {
            return Ok(true);
        };
        let state: Option<String> = self
            .db
            .query_row(
                "SELECT state FROM assignments WHERE id=?1",
                [&assignment],
                |row| row.get(0),
            )
            .optional()?;
        if state.as_deref() == Some("closed") {
            self.cancel_inner(run_id)?;
            return Ok(false);
        }
        if self.ensure_assignment_active(&assignment).is_err() {
            self.set_run_state(run_id,"blocked",Some("registered project is unavailable or changed; restore the original workspace before continuing"))?;
            return Ok(false);
        }
        Ok(true)
    }

    pub(crate) fn ancestors_hold(&self, run_id: &str, temporary: bool) -> Result<bool> {
        Ok(self.db.query_row("WITH RECURSIVE ancestors(id,parent_run_id,state) AS (SELECT a.id,a.parent_run_id,a.state FROM runs a JOIN runs child ON child.parent_run_id=a.id WHERE child.id=?1 UNION ALL SELECT a.id,a.parent_run_id,a.state FROM runs a JOIN ancestors previous ON a.id=previous.parent_run_id) SELECT EXISTS(SELECT 1 FROM ancestors WHERE state IN ('succeeded','failed','cancelled') OR (?2 AND state IN ('paused','blocked','needs_reconciliation')))",params![run_id,temporary],|row|row.get(0))?)
    }

    pub(crate) fn authorize_artifacts(&self, assignment: &str, value: &Value) -> Result<()> {
        let mut references = Vec::new();
        collect_refs(value, &mut references)?;
        for reference in references {
            let owned: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM artifact_imports WHERE assignment_id=?1 AND artifact_id=?2 UNION ALL SELECT 1 FROM artifact_links l JOIN runs r ON r.id=l.run_id WHERE r.assignment_id=?1 AND l.artifact_id=?2 UNION ALL SELECT 1 FROM schedules s,json_each(s.template_json,'$.references') ref WHERE s.assignment_id=?1 AND s.state IN ('active','paused') AND json_extract(ref.value,'$.artifact_id')=?2)",
                params![assignment,reference.artifact_id], |row| row.get(0))?;
            if !owned {
                return Err(RuntimeError::Forbidden);
            }
        }
        Ok(())
    }

    pub(crate) fn authorize_literals(
        &self,
        assignment: &str,
        process: &ProcessManifest,
    ) -> Result<()> {
        self.authorize_artifacts(assignment, &process.config.defaults)?;
        for binding in process
            .steps
            .iter()
            .flat_map(|step| step.bindings())
            .chain(process.output.values())
        {
            if let Binding::Literal { literal } = binding {
                self.authorize_artifacts(assignment, literal)?;
            }
        }
        Ok(())
    }

    pub(crate) fn cancel_inner(&mut self, run_id: &str) -> Result<Value> {
        self.ensure_run(run_id)?;
        let tx = self.db.transaction()?;
        cancel_in_transaction(&tx, run_id, "run.cancelled")?;
        tx.commit()?;
        Ok(serde_json::to_value(self.envelope(run_id)?)?)
    }

    fn discover(&self, scope: &Scope, args: DiscoverArgs) -> Result<Value> {
        if args.limit.is_some_and(|limit| limit == 0 || limit > 20) {
            return Err(RuntimeError::InvalidInput(
                "discovery limit must be 1–20".into(),
            ));
        }
        if let Some(version) = args.process_ref {
            if args.detail.as_deref() != Some("schema") || args.query.is_some() {
                return Err(RuntimeError::InvalidInput(
                    "process_ref requires detail schema and no query".into(),
                ));
            }
            let process = self.catalog_process(&scope.assignment_id, &version)?;
            let result = process_descriptor(&process);
            bounded_detail(&result)?;
            return Ok(result);
        }
        if args.detail.is_some() {
            return Err(RuntimeError::InvalidInput(
                "detail requires process_ref".into(),
            ));
        }
        let query = args.query.unwrap_or_default();
        if query.len() > 200 {
            return Err(RuntimeError::InvalidInput(
                "discovery query exceeds 200 bytes".into(),
            ));
        }
        let limit = args.limit.unwrap_or(10);
        let pattern = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut statement = self.db.prepare(
            "SELECT c.version,c.name,json_extract(v.manifest,'$.description') FROM catalog c JOIN process_versions v ON v.version=c.version WHERE c.assignment_id=?1 AND (c.name LIKE ?2 ESCAPE '\\' OR json_extract(v.manifest,'$.description') LIKE ?2 ESCAPE '\\') ORDER BY c.registered_at DESC,c.name,c.version LIMIT ?3")?;
        let rows =
            statement.query_map(params![scope.assignment_id, pattern, limit + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
        let mut processes = rows
            .map(|row| {
                let (version, name, description) = row?;
                let (selected, selection_revision) = self.selection(&scope.assignment_id, &name)?;
                Ok(
                    json!({"process_ref":version,"process_name":name,"description":description,
                    "selected_for_future_work":selected.as_deref()==Some(version.as_str()),
                    "selected_process_ref":selected,"selection_revision":selection_revision}),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let truncated = processes.len() > limit;
        processes.truncate(limit);
        Ok(json!({"processes":processes,"truncated":truncated}))
    }

    pub(crate) fn catalog_process(
        &self,
        assignment: &str,
        version: &str,
    ) -> Result<AdmittedProcess> {
        let manifest: Option<String> = self.db.query_row(
            "SELECT v.manifest FROM catalog c JOIN process_versions v ON v.version=c.version WHERE c.assignment_id=?1 AND c.version=?2",
            params![assignment,version], |row|row.get(0)).optional()?;
        let process = admit_json(manifest.ok_or(RuntimeError::Forbidden)?.as_bytes())?;
        if process.version != version {
            return Err(RuntimeError::Integrity(
                "registered process digest mismatch".into(),
            ));
        }
        Ok(process)
    }

    fn scoped_run(&mut self, scope: &Scope, args: RunArgs) -> Result<Value> {
        if !matches!(args.mode.as_str(), "preflight" | "start") {
            return Err(RuntimeError::InvalidInput(
                "run mode must be preflight or start".into(),
            ));
        }
        check_key(&args.request_key)?;
        let process = self.catalog_process(&scope.assignment_id, &args.process_ref)?;
        self.authorize_artifacts(&scope.assignment_id, &args.input)?;
        self.authorize_literals(&scope.assignment_id, &process.manifest)?;
        validate_instance(&process.manifest.input_schema, &args.input)
            .map_err(|error| RuntimeError::InvalidInput(error.to_string()))?;
        self.checked_refs(&args.input, process.manifest.limits.max_artifact_bytes)?;
        for binding in process
            .manifest
            .steps
            .iter()
            .flat_map(|step| step.bindings())
            .chain(process.manifest.output.values())
        {
            if let Binding::Literal { literal } = binding {
                self.checked_refs(literal, process.manifest.limits.max_artifact_bytes)?;
            }
        }
        if args.mode == "preflight" {
            self.prepare_call_snapshots(&process, Some(&scope.assignment_id), false)?;
            self.execution_preflight(&process, Some(&scope.assignment_id))?;
            let config = self.resolve_configuration(
                &process,
                Some(&scope.assignment_id),
                &args.config_overrides,
            )?;
            self.authorize_artifacts(&scope.assignment_id, &config.value)?;
            self.checked_refs(&config.value, process.manifest.limits.max_artifact_bytes)?;
            return Ok(
                json!({"protocol_version":"0.1","valid":true,"mode":"preflight","process_ref":process.version,
                "config_digest":content_digest(&config.value)?,
                "steps":process.manifest.steps.len(),"agent_requests":process.manifest.steps.iter().filter(|step|step.kind=="agent").map(|step|step.foreach.as_ref().map_or(1,|each|each.max_items)).sum::<u32>(),
                "limits":process.manifest.limits,"dispatches":0}),
            );
        }
        Ok(serde_json::to_value(self.start_with_config_inner(
            &process,
            args.input,
            &args.request_key,
            StartOptions {
                assignment_id: Some(&scope.assignment_id),
                drive: false,
                config_overrides: &args.config_overrides,
                fixture: false,
            },
        )?)?)
    }

    fn authorize_run(&self, assignment: &str, run_id: &str) -> Result<()> {
        let owned: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND assignment_id=?2)",
            params![run_id, assignment],
            |row| row.get(0),
        )?;
        if !owned {
            return Err(RuntimeError::Forbidden);
        }
        Ok(())
    }

    fn scoped_inspect(&self, scope: &Scope, args: InspectArgs) -> Result<Value> {
        match args {
            InspectArgs::Improvements { process_ref } => {
                self.improvements_view(&scope.assignment_id, &process_ref)
            }
            InspectArgs::Improvement { cycle_id } => {
                self.improvement_view(&scope.assignment_id, &cycle_id)
            }
            InspectArgs::ImprovementEvidence {
                process_ref,
                run_ids,
            } => self.improvement_evidence(&scope.assignment_id, &process_ref, &run_ids),
            InspectArgs::LearningCases {
                process_ref,
                limit,
                after,
            } => self.learning_cases(&scope.assignment_id, &process_ref, limit, after.as_deref()),
            InspectArgs::McpApp {
                run_id,
                resource_step_id,
                tool_step_id,
            } => {
                self.authorize_run(&scope.assignment_id, &run_id)?;
                self.mcp_app_view(&run_id, &resource_step_id, &tool_step_id)
            }
            InspectArgs::PriceSources => self.price_sources(&scope.assignment_id),
            InspectArgs::McpInteractions { after } => {
                self.mcp_interactions(&scope.assignment_id, after.as_deref())
            }
            InspectArgs::Billing {
                starts_at_ms,
                ends_at_ms,
            } => self.billing_period(&scope.assignment_id, starts_at_ms, ends_at_ms),
            InspectArgs::Budget => self.billing_budget(&scope.assignment_id),
            InspectArgs::ConnectionLibrary { after } => {
                self.connection_library(after.as_deref(), false)
            }
            InspectArgs::LibraryContract {
                library_id,
                revision,
            } => self.library_contract(&library_id, revision),
            InspectArgs::ConnectionContract { name } => {
                self.connection_contract(&scope.assignment_id, &name)
            }
            InspectArgs::Approvals => self.connection_requests(&scope.assignment_id),
            InspectArgs::Readiness { process_ref } => {
                self.recipe_readiness(&scope.assignment_id, &process_ref)
            }
            InspectArgs::Analytics {
                run_id,
                limit,
                after,
            } => {
                if let Some(run) = &run_id {
                    self.authorize_run(&scope.assignment_id, run)?;
                }
                self.analytics_view(
                    &scope.assignment_id,
                    run_id.as_deref(),
                    limit,
                    after.as_deref(),
                )
            }
            InspectArgs::Capacity => Ok(
                json!({"view":"capacity","policy":self.capacity_policy()?,"owner_controlled":true,"legacy_omitted_call_depth":8,"recommended_call_depth":32}),
            ),
            InspectArgs::Workers => self.worker_status_inner(Some(&scope.assignment_id)),
            InspectArgs::TreeStatus { run_id } => {
                self.authorize_run(&scope.assignment_id, &run_id)?;
                self.run_tree_status(&scope.assignment_id, &run_id)
            }
            InspectArgs::Graph {
                process_ref,
                run_id,
                limit,
                after,
            } => {
                let process = match (&process_ref, &run_id) {
                    (Some(version), None) => self.catalog_process(&scope.assignment_id, version)?,
                    (None, Some(run)) => {
                        self.authorize_run(&scope.assignment_id, run)?;
                        self.load_run(run)?.0
                    }
                    _ => {
                        return Err(RuntimeError::InvalidInput(
                            "graph requires exactly one of process_ref or run_id".into(),
                        ));
                    }
                };
                self.graph_view(
                    &process,
                    run_id.as_deref(),
                    scope.mode == "author",
                    limit,
                    after,
                )
            }
            InspectArgs::Tree {
                run_id,
                limit,
                after,
            } => {
                self.authorize_run(&scope.assignment_id, &run_id)?;
                self.run_tree_view(&scope.assignment_id, &run_id, limit, after)
            }
            InspectArgs::JevTuning { process_ref } => {
                if scope.mode != "author" {
                    return Err(RuntimeError::Forbidden);
                }
                self.jev_tuning(&scope.assignment_id, &process_ref)
            }
            InspectArgs::Schedule { schedule_id } => {
                self.schedule_view(&scope.assignment_id, &schedule_id, true)
            }
            InspectArgs::Schedules { limit, after } => {
                self.schedule_list(&scope.assignment_id, limit, after)
            }
            InspectArgs::Effects { run_id } => {
                self.authorize_run(&scope.assignment_id, &run_id)?;
                self.effect_view(&run_id)
            }
            InspectArgs::Bindings => self.execution_catalog(&scope.assignment_id),
            InspectArgs::Run { run_id } => {
                self.authorize_run(&scope.assignment_id, &run_id)?;
                Ok(serde_json::to_value(self.envelope(&run_id)?)?)
            }
            InspectArgs::Config { run_id } => {
                self.authorize_run(&scope.assignment_id, &run_id)?;
                self.run_config_view(&run_id)
            }
            InspectArgs::Runs { limit, after } => {
                let limit = limit.unwrap_or(10);
                if limit == 0
                    || limit > 20
                    || after.as_ref().is_some_and(|cursor| {
                        cursor.len() > 160 || cursor.chars().any(char::is_control)
                    })
                {
                    return Err(RuntimeError::InvalidInput(
                        "invalid run history limit or cursor".into(),
                    ));
                }
                let mut query=self.db.prepare("SELECT r.id,r.version,r.state,r.revision,r.fixture,r.config,(SELECT COUNT(*) FROM steps s WHERE s.run_id=r.id AND s.state IN ('succeeded','skipped')),(SELECT COUNT(*) FROM steps s WHERE s.run_id=r.id),COALESCE(r.root_run_id,r.id),r.parent_run_id,r.parent_step_id,(SELECT json_extract(v.manifest,'$.name') FROM process_versions v WHERE v.version=r.version) FROM runs r WHERE r.assignment_id=?1 AND (?2 IS NULL OR r.id>?2) ORDER BY r.id LIMIT ?3")?;
                let rows = query
                    .query_map(params![scope.assignment_id, after, limit + 1], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, u64>(3)?,
                            row.get::<_, bool>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, u64>(6)?,
                            row.get::<_, u64>(7)?,
                            row.get::<_, String>(8)?,
                            row.get::<_, Option<String>>(9)?,
                            row.get::<_, Option<String>>(10)?,
                            row.get::<_, String>(11)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let truncated = rows.len() > limit;
                let mut runs = Vec::new();
                let mut next_cursor = after;
                for (
                    id,
                    version,
                    state,
                    revision,
                    fixture,
                    config,
                    completed,
                    total,
                    root,
                    parent,
                    parent_step,
                    process_name,
                ) in rows.into_iter().take(limit)
                {
                    next_cursor = Some(id.clone());
                    runs.push(json!({"protocol_version":"0.1","run_id":id,"process_version":version,
                        "root_run_id":root,"parent_run_id":parent,"parent_step_id":parent_step,"process_name":process_name,
                        "state":state,"revision":revision,"fixture":fixture,"config_digest":content_digest(&serde_json::from_str(&config)?)?,
                        "completed_steps":completed,"summary":format!("{completed}/{total} steps complete; {state}"),
                        "truncated":true,"available_views":["inspect_run"]}));
                }
                Ok(json!({"runs":runs,"next_cursor":next_cursor,"truncated":truncated}))
            }
            InspectArgs::Pending { run_id, limit } => {
                let limit = limit.unwrap_or(10);
                if limit == 0 || limit > 20 {
                    return Err(RuntimeError::InvalidInput(
                        "pending limit must be 1–20".into(),
                    ));
                }
                let ids: Vec<String> = if let Some(run_id) = run_id {
                    self.authorize_run(&scope.assignment_id, &run_id)?;
                    vec![run_id]
                } else {
                    let mut query = self.db.prepare("SELECT r.id FROM runs r WHERE r.assignment_id=?1 AND r.state IN ('waiting_for_agent','queued','running') AND EXISTS(SELECT 1 FROM handoffs h WHERE h.run_id=r.id AND h.state='pending' AND h.token_hash<>'') AND NOT EXISTS(WITH RECURSIVE ancestors(id,parent_run_id,state) AS (SELECT a.id,a.parent_run_id,a.state FROM runs a WHERE a.id=r.parent_run_id UNION ALL SELECT a.id,a.parent_run_id,a.state FROM runs a JOIN ancestors previous ON a.id=previous.parent_run_id) SELECT 1 FROM ancestors WHERE state IN ('succeeded','paused','blocked','failed','cancelled','needs_reconciliation')) ORDER BY r.created_at,r.id LIMIT ?2")?;
                    query
                        .query_map(params![scope.assignment_id, limit + 1], |row| row.get(0))?
                        .collect::<std::result::Result<_, _>>()?
                };
                let truncated = ids.len() > limit;
                let mut runs = Vec::new();
                for id in ids.into_iter().take(limit) {
                    let envelope = self.envelope(&id)?;
                    if envelope.request.is_some() {
                        runs.push(envelope);
                    }
                }
                Ok(json!({"runs":runs,"truncated":truncated}))
            }
            InspectArgs::Events {
                run_id,
                after,
                limit,
            } => {
                self.authorize_run(&scope.assignment_id, &run_id)?;
                let limit = limit.unwrap_or(20);
                let after = after.unwrap_or(0);
                if limit == 0 || limit > 100 || after > i64::MAX as u64 {
                    return Err(RuntimeError::InvalidInput(
                        "invalid event cursor or limit".into(),
                    ));
                }
                let events = self.events_inner(&run_id, after, limit)?;
                let next_cursor = events.last().map(|event| event.sequence).unwrap_or(after);
                Ok(
                    json!({"events":events,"next_cursor":next_cursor,"truncated":events.len()==limit}),
                )
            }
            InspectArgs::Artifact { reference } => {
                self.authorize_artifacts(&scope.assignment_id, &serde_json::to_value(&reference)?)?;
                verify_registered_reference(&self.db, &reference)?;
                if reference.bytes > MAX_ARTIFACT_PREVIEW_BYTES {
                    return Ok(
                        json!({"artifact":reference,"truncated":true,"reason":"artifact exceeds the 32 KiB inline preview limit"}),
                    );
                }
                let bytes = self
                    .artifacts
                    .read(&reference, MAX_ARTIFACT_PREVIEW_BYTES)?;
                match String::from_utf8(bytes) {
                    Ok(text) => Ok(json!({"artifact":reference,"text":text,"truncated":false})),
                    Err(_) => {
                        Ok(json!({"artifact":reference,"truncated":false,"text_available":false}))
                    }
                }
            }
        }
    }

    fn scoped_resume(&mut self, scope: &Scope, mut args: Value) -> Result<Value> {
        if args.get("action").and_then(Value::as_str) == Some("mcp.respond") {
            args.as_object_mut()
                .ok_or(RuntimeError::Forbidden)?
                .remove("action");
            let request: crate::interactions::InteractionResponse = serde_json::from_value(args)?;
            return self.respond_mcp_inner(&scope.assignment_id, request, false);
        }
        if args.get("action").and_then(Value::as_str) == Some("reconcile") {
            args.as_object_mut()
                .ok_or(RuntimeError::Forbidden)?
                .remove("action");
            let request: crate::execution::Reconciliation = serde_json::from_value(args)?;
            self.authorize_run(&scope.assignment_id, &request.run_id)?;
            if let Some(output) = &request.output {
                self.authorize_artifacts(&scope.assignment_id, output)?;
            }
            return self.reconcile_inner(&request);
        }

        if args.get("action").and_then(Value::as_str) == Some("submit") {
            let submission: Submission = serde_json::from_value(args)?;
            self.authorize_run(&scope.assignment_id, &submission.run_id)?;
            self.authorize_artifacts(&scope.assignment_id, &submission.output)?;
            let receipt = self.resume_inner(&submission, false)?;
            return Ok(json!({"receipt":receipt,"run":self.envelope(&submission.run_id)?}));
        }
        let control: ResumeControl = serde_json::from_value(args)?;
        let run_id = match &control {
            ResumeControl::Renew { run_id }
            | ResumeControl::Continue { run_id }
            | ResumeControl::Cancel { run_id }
            | ResumeControl::Pause { run_id }
            | ResumeControl::Retry { run_id, .. } => run_id,
        };
        self.authorize_run(&scope.assignment_id, run_id)?;
        match control {
            ResumeControl::Renew { run_id } => {
                Ok(serde_json::to_value(self.renew_inner(&run_id)?)?)
            }
            ResumeControl::Continue { run_id } => {
                if matches!(self.ensure_run(&run_id)?.as_str(), "blocked" | "paused") {
                    self.set_run_state(&run_id, "queued", None)?;
                }
                Ok(serde_json::to_value(self.envelope(&run_id)?)?)
            }
            ResumeControl::Cancel { run_id } => self.cancel_inner(&run_id),
            ResumeControl::Pause { run_id } => {
                if !matches!(
                    self.ensure_run(&run_id)?.as_str(),
                    "queued"
                        | "running"
                        | "waiting_for_agent"
                        | "waiting_for_child"
                        | "blocked"
                        | "paused"
                ) {
                    return Err(RuntimeError::State("only active work can be paused".into()));
                }
                self.set_run_state(&run_id, "paused", None)?;
                Ok(serde_json::to_value(self.envelope(&run_id)?)?)
            }
            ResumeControl::Retry {
                run_id,
                step_instance_id,
            } => self.retry_inner(&run_id, &step_instance_id),
        }
    }

    fn scoped_manage(&mut self, scope: &Scope, args: ManageArgs) -> Result<Value> {
        let validate_only = matches!(&args, ManageArgs::ProcessValidate { .. });
        match args {
            ManageArgs::ImprovementOpen {
                request_key,
                process_ref,
                run_ids,
            } => self.improvement_open(&scope.assignment_id, &request_key, &process_ref, run_ids),
            ManageArgs::ImprovementPropose {
                cycle_id,
                proposal,
                candidate_path,
                review,
            } => {
                let process = admit_json(&read_project_file(
                    scope,
                    &self.root,
                    &candidate_path,
                    MAX_MANIFEST_BYTES,
                )?)?;
                self.improvement_propose(
                    &scope.assignment_id,
                    &cycle_id,
                    proposal,
                    &process,
                    review,
                )
            }
            ManageArgs::ImprovementFreeze {
                cycle_id,
                policy,
                development,
                holdout,
            } => self.improvement_freeze(
                &scope.assignment_id,
                &cycle_id,
                policy,
                development,
                holdout,
            ),
            ManageArgs::ImprovementEvaluate { cycle_id } => {
                self.improvement_evaluate(&scope.assignment_id, &cycle_id)
            }
            ManageArgs::ImprovementClose {
                cycle_id,
                outcome,
                reason,
            } => self.improvement_close(&scope.assignment_id, &cycle_id, &outcome, &reason),
            ManageArgs::ImprovementDecide {
                cycle_id,
                decision,
                candidate_path,
                expected_report_digest,
                expected_selected_ref,
            } => {
                let process = admit_json(&read_project_file(
                    scope,
                    &self.root,
                    &candidate_path,
                    MAX_MANIFEST_BYTES,
                )?)?;
                self.improvement_decide(
                    &scope.assignment_id,
                    &cycle_id,
                    &decision,
                    &process,
                    &expected_report_digest,
                    expected_selected_ref.as_deref(),
                )
            }
            ManageArgs::ProcessSelect {
                process_ref,
                expected_selected_ref,
                expected_selection_revision,
            } => self.select_registered(
                &scope.assignment_id,
                &process_ref,
                expected_selected_ref.as_deref(),
                expected_selection_revision,
            ),
            ManageArgs::ConnectionUse {
                library_id,
                revision,
                name,
            } => self.use_library_connection(&scope.assignment_id, &library_id, revision, &name),
            ManageArgs::ProcessAudit { path, review } => {
                let bytes = read_project_file(scope, &self.root, &path, MAX_MANIFEST_BYTES)?;
                let process = admit_json(&bytes)?;
                Ok(
                    json!({"process_ref":process.version,"audit":apr_core::audit::audit_recipe(&process.manifest,&review)}),
                )
            }
            ManageArgs::ConnectionPropose(args) => {
                self.propose_connection_inner(&scope.assignment_id, args)
            }
            ManageArgs::ConnectionDiscover {
                resource,
                client_id,
                scopes,
                metadata_url,
            } => {
                let config = match self.discover_oauth(
                    &resource,
                    metadata_url.as_deref(),
                    &client_id,
                    scopes,
                ) {
                    Ok(config) => config,
                    Err(apr_connectors::DispatchError::OAuthRegistrationUnsupported) => return Ok(json!({"status":"client_registration_unsupported","next":"This service needs a CookBook provider integration. Report the unsupported service; do not ask the user to register an OAuth application or silently substitute a PAT or project key. A provider-documented API-key route requires a separate reviewed proposal.","grant_created":false})),
                    Err(apr_connectors::DispatchError::InvalidConnection | apr_connectors::DispatchError::InvalidInput) => return Err(RuntimeError::InvalidInput("OAuth metadata did not match the exact public resource and required security policy".into())),
                    Err(_) => return Ok(json!({"status":"discovery_unavailable","next":"Check the official endpoint and public authorization metadata. No authentication route has been verified.","grant_created":false})),
                };
                Ok(json!({
                    "status": "authentication_required",
                    "config": config,
                    "next": "owner_approval",
                    "scope_review": "Review every returned scope before owner sign-in. Empty input uses the resource's advertised scopes; discovery itself grants nothing.",
                    "grant_created": false,
                }))
            }
            ManageArgs::CostPreview {
                connection,
                request,
                response,
            } => crate::setup::preview_cost(connection, request, response),
            ManageArgs::ProcessTuneJev(args) => self.tune_jev(&scope.assignment_id, args),
            ManageArgs::ScheduleCreate(args) => {
                self.create_schedule(&scope.assignment_id, &scope.attachment_id, args)
            }
            ManageArgs::SchedulePause { schedule_id } => {
                self.control_schedule(&scope.assignment_id, &schedule_id, "pause")
            }
            ManageArgs::ScheduleResume { schedule_id } => {
                self.control_schedule(&scope.assignment_id, &schedule_id, "resume")
            }
            ManageArgs::ScheduleDelete { schedule_id } => {
                self.control_schedule(&scope.assignment_id, &schedule_id, "delete")
            }
            ManageArgs::ProcessValidate { path } | ManageArgs::ProcessRegister { path } => {
                let bytes = read_project_file(scope, &self.root, &path, MAX_MANIFEST_BYTES)?;
                let process = admit_json(&bytes)?;
                let sources = read_sources(scope, &self.root, &path, &process)?;
                bounded_detail(&process_descriptor(&process))?;
                for step in &process.manifest.steps {
                    if step.kind == "agent" {
                        bounded_handoff(step, &json!({}))?;
                    }
                }
                if validate_only {
                    return Ok(
                        json!({"valid":true,"process_ref":process.version,"process_name":process.manifest.name,"steps":process.manifest.steps.len()}),
                    );
                }
                self.authorize_literals(&scope.assignment_id, &process.manifest)?;
                self.snapshot_sources(&process, sources)?;
                let tx = self.db.transaction()?;
                tx.execute("INSERT OR IGNORE INTO process_versions(version,manifest,canonical_format) VALUES (?1,?2,?3)",params![process.version,String::from_utf8_lossy(&process.canonical_bytes),apr_core::CANONICAL_FORMAT])?;
                tx.execute("INSERT OR IGNORE INTO catalog(assignment_id,version,name,registered_at) VALUES (?1,?2,?3,?4)",params![scope.assignment_id,process.version,process.manifest.name,now_ms()])?;
                tx.commit()?;
                Ok(
                    json!({"protocol_version":"0.1","process_ref":process.version,"process_name":process.manifest.name,"registered":true}),
                )
            }
            ManageArgs::ArtifactImport { path, media_type } => {
                let bytes = read_project_file(scope, &self.root, &path, MAX_IMPORT_BYTES)?;
                let reference = self.artifacts.put_with_quota(
                    &bytes,
                    &media_type,
                    MAX_IMPORT_BYTES as u64,
                    self.artifact_quota()?,
                )?;
                verify_registered_reference(&self.db, &reference)?;
                let tx = self.db.transaction()?;
                tx.execute(
                    "INSERT OR IGNORE INTO artifacts(id,reference) VALUES (?1,?2)",
                    params![reference.artifact_id, serde_json::to_string(&reference)?],
                )?;
                tx.execute("INSERT OR IGNORE INTO artifact_imports(assignment_id,artifact_id) VALUES (?1,?2)",params![scope.assignment_id,reference.artifact_id])?;
                tx.commit()?;
                Ok(serde_json::to_value(reference)?)
            }
            ManageArgs::ConfigRead { process_ref } => {
                let process = self.catalog_process(&scope.assignment_id, &process_ref)?;
                let config =
                    self.resolve_configuration(&process, Some(&scope.assignment_id), &json!({}))?;
                config_report(&process, &config)
            }
            ManageArgs::ConfigSet {
                process_ref,
                values,
            } => {
                let process = self.catalog_process(&scope.assignment_id, &process_ref)?;
                let project: String = self.db.query_row(
                    "SELECT project_id FROM assignments WHERE id=?1",
                    [&scope.assignment_id],
                    |row| row.get(0),
                )?;
                let (project_values, project_revision) =
                    self.config_layer("project", &project, &process_ref)?;
                let (_, assignment_revision) =
                    self.config_layer("assignment", &scope.assignment_id, &process_ref)?;
                let base = overlay_config(&process.manifest.config.defaults, &project_values)?;
                let candidate = overlay_config(&base, &values)?;
                validate_instance(&process.manifest.config.schema, &candidate)
                    .map_err(|error| RuntimeError::InvalidInput(error.to_string()))?;
                self.authorize_artifacts(&scope.assignment_id, &candidate)?;
                self.checked_refs(&candidate, process.manifest.limits.max_artifact_bytes)?;
                let config = ResolvedConfig {
                    value: candidate,
                    project_revision,
                    assignment_revision: assignment_revision + 1,
                };
                config_report(&process, &config)?;
                self.write_config_layer("assignment", &scope.assignment_id, &process_ref, &values)?;
                Ok(
                    json!({"process_ref":process_ref,"config_digest":content_digest(&config.value)?,
                    "project_revision":config.project_revision,"assignment_revision":config.assignment_revision}),
                )
            }
            ManageArgs::ProcessTest {
                path,
                input,
                request_key,
                config_overrides,
            } => {
                let bytes = read_project_file(scope, &self.root, &path, MAX_MANIFEST_BYTES)?;
                let process = admit_json(&bytes)?;
                bounded_detail(&process_descriptor(&process))?;
                for step in &process.manifest.steps {
                    if !matches!(
                        (step.kind.as_str(), step.effect.as_str()),
                        ("builtin", "pure")
                            | ("agent", "agent")
                            | ("decision", "pure")
                            | ("join", "pure")
                            | ("call", "control")
                    ) {
                        return Err(RuntimeError::Forbidden);
                    }
                    if step.kind == "agent" {
                        bounded_handoff(step, &json!({}))?;
                    }
                }
                Ok(serde_json::to_value(self.start_with_config_inner(
                    &process,
                    input,
                    &request_key,
                    StartOptions {
                        assignment_id: Some(&scope.assignment_id),
                        drive: false,
                        config_overrides: &config_overrides,
                        fixture: true,
                    },
                )?)?)
            }
            ManageArgs::ProcessPromote {
                process_ref,
                target_assignment_id,
            } => {
                let allowed:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM promotion_grants WHERE source_assignment_id=?1 AND target_assignment_id=?2)",
                    params![scope.assignment_id,target_assignment_id],|row|row.get(0))?;
                if !allowed {
                    return Err(RuntimeError::Forbidden);
                }
                self.require_same_project(&scope.assignment_id, &target_assignment_id)?;
                self.ensure_assignment_active(&target_assignment_id)?;
                let process = self.catalog_process(&scope.assignment_id, &process_ref)?;
                self.authorize_literals(&target_assignment_id, &process.manifest)?;
                let tx = self.db.transaction()?;
                tx.execute("INSERT OR IGNORE INTO catalog(assignment_id,version,name,registered_at) VALUES (?1,?2,?3,?4)",
                    params![target_assignment_id,process.version,process.manifest.name,now_ms()])?;
                tx.execute("INSERT OR IGNORE INTO catalog_promotions(source_assignment_id,target_assignment_id,version,created_at) VALUES (?1,?2,?3,?4)",
                    params![scope.assignment_id,target_assignment_id,process.version,now_ms()])?;
                tx.commit()?;
                Ok(
                    json!({"process_ref":process.version,"process_name":process.manifest.name,"target_assignment_id":target_assignment_id,"promoted":true}),
                )
            }
        }
    }
}

fn overlay_config(base: &Value, layer: &Value) -> Result<Value> {
    let mut values = base
        .as_object()
        .cloned()
        .ok_or_else(|| RuntimeError::InvalidInput("config must be an object".into()))?;
    let layer = layer
        .as_object()
        .ok_or_else(|| RuntimeError::InvalidInput("config values must be an object".into()))?;
    for (key, value) in layer {
        values.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(values))
}

fn config_report(process: &AdmittedProcess, config: &ResolvedConfig) -> Result<Value> {
    let result = json!({"process_ref":process.version,"config":config.value,"config_digest":content_digest(&config.value)?,
        "project_revision":config.project_revision,"assignment_revision":config.assignment_revision,
        "run_override_keys":process.manifest.config.run_override_keys});
    bounded_detail(&result)?;
    Ok(result)
}

fn process_descriptor(process: &AdmittedProcess) -> Value {
    json!({"protocol_version":"0.1","process_ref":process.version,"process_name":process.manifest.name,
        "description":process.manifest.description,"input_schema":process.manifest.input_schema,
        "output_schema":process.manifest.output_schema,"capabilities":process.manifest.capabilities,
        "limits":process.manifest.limits,"config":process.manifest.config})
}

pub(crate) fn cancel_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    reason: &str,
) -> Result<()> {
    let family: Vec<String> = {
        let mut query=tx.prepare("WITH RECURSIVE family(id) AS (SELECT id FROM runs WHERE id=?1 UNION ALL SELECT child.id FROM runs child JOIN family parent ON child.parent_run_id=parent.id) SELECT r.id FROM runs r JOIN family f ON f.id=r.id")?;
        query
            .query_map([run_id], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?
    };
    for member in family {
        // A cancellable worker may still be running outside the short state
        // writer lock. Its dispatch is already durable and may have produced
        // an effect. Fence it as uncertain in the same transaction that
        // cancels the run, before the control response is sent.
        let active_intents: Vec<(String, String)> = {
            let mut query = tx.prepare(
                "SELECT id,step_id FROM effect_intents WHERE run_id=?1 AND status='dispatched'",
            )?;
            query
                .query_map([&member], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        for (intent, step_id) in active_intents {
            tx.execute(
                "UPDATE effect_intents SET status='uncertain' WHERE id=?1 AND status='dispatched'",
                [&intent],
            )?;
            crate::billing::settle(tx, &intent, "uncertain", None, "broker_observed")?;
            event(
                tx,
                &member,
                "effect.unfinished",
                json!({"step_id":step_id,"intent_id":intent,"state":"uncertain","reason":"run_cancelled"}),
            )?;
        }
        let mut changed=tx.execute(
        "UPDATE runs SET state='cancelled',error=NULL WHERE id=?1 AND state NOT IN ('succeeded','failed','cancelled')",
        [&member],
    )?;
        changed+=tx.execute("UPDATE steps SET state='cancelled',active_attempt=NULL,fence=fence+1 WHERE run_id=?1 AND state NOT IN ('succeeded','failed','skipped','cancelled')", [&member])?;
        changed+=tx.execute(
        "UPDATE attempts SET state='cancelled',finished_at=?2 WHERE run_id=?1 AND state IN ('running','waiting_for_child','waiting_for_agent')",
        params![member, now_ms()],
    )?;
        changed += tx.execute(
            "UPDATE handoffs SET state='cancelled' WHERE run_id=?1 AND state='pending'",
            [&member],
        )?;
        let recorded: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE run_id=?1 AND kind='run.cancelled')",
            [&member],
            |row| row.get(0),
        )?;
        if changed > 0 || (member == run_id && !recorded) {
            event(
                tx,
                &member,
                "run.cancelled",
                json!({"reason":reason,"requested_run_id":run_id}),
            )?;
        }
    }
    Ok(())
}

fn directory_identity(path: &Path) -> Result<(String, String)> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::Forbidden);
    }
    Ok((metadata.dev().to_string(), metadata.ino().to_string()))
}

fn verify_directory(root: &str, device: &str, inode: &str) -> Result<()> {
    let path = Path::new(root);
    if path.canonicalize()? != path {
        return Err(RuntimeError::Forbidden);
    }
    let actual = directory_identity(path)?;
    if actual.0 != device || actual.1 != inode {
        return Err(RuntimeError::Forbidden);
    }
    Ok(())
}

fn id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}
fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn read_project_file(
    scope: &Scope,
    state_root: &Path,
    relative: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let path = Path::new(relative);
    let components: Vec<_> = path.components().collect();
    if components.is_empty()
        || components
            .iter()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(RuntimeError::Forbidden);
    }
    if scope.project_root.join(path).starts_with(state_root) {
        return Err(RuntimeError::Forbidden);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&scope.project_root)
        .map_err(|_| RuntimeError::Forbidden)?;
    let root_metadata = file.metadata()?;
    if root_metadata.dev().to_string() != scope.device
        || root_metadata.ino().to_string() != scope.inode
    {
        return Err(RuntimeError::Forbidden);
    }
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(RuntimeError::Forbidden);
        };
        let name = CString::new(name.as_bytes()).map_err(|_| RuntimeError::Forbidden)?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC
            | if index + 1 < components.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // Each opened component is relative to its pinned parent descriptor.
        let descriptor = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 {
            return Err(RuntimeError::Forbidden);
        }
        // SAFETY: openat returned a new owned file descriptor.
        file = unsafe { File::from_raw_fd(descriptor) };
    }
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(RuntimeError::Forbidden);
    }
    if metadata.len() > max_bytes as u64 {
        return Err(RuntimeError::LimitExceeded(
            "project file exceeds its import limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(RuntimeError::LimitExceeded(
            "project file exceeds its import limit".into(),
        ));
    }
    Ok(bytes)
}

fn read_sources(
    scope: &Scope,
    state: &Path,
    manifest_path: &str,
    process: &AdmittedProcess,
) -> Result<Vec<(String, Vec<u8>)>> {
    let parent = Path::new(manifest_path).parent().unwrap_or(Path::new(""));
    let mut sources = Vec::new();
    let mut total = 0;
    for file in process.manifest.files.as_deref().unwrap_or(&[]) {
        let path = parent.join(&file.path);
        let bytes = read_project_file(
            scope,
            state,
            path.to_str().ok_or(RuntimeError::Forbidden)?,
            MAX_MANIFEST_BYTES,
        )?;
        total += bytes.len();
        if total > MAX_MANIFEST_BYTES || std::str::from_utf8(&bytes).is_err() {
            return Err(RuntimeError::LimitExceeded(
                "source bundle must be UTF-8 and at most 1 MiB".into(),
            ));
        }
        if format!("{:x}", Sha256::digest(&bytes)) != file.sha256 {
            return Err(RuntimeError::Integrity(
                "source digest does not match manifest".into(),
            ));
        }
        sources.push((file.path.clone(), bytes));
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::ManageArgs;
    use serde_json::json;

    #[test]
    fn connection_discover_defaults_are_scoped_and_reject_forged_fields() {
        let args: ManageArgs = serde_json::from_value(json!({
            "action": "connection.discover",
            "resource": "https://example.test/mcp"
        }))
        .unwrap();
        assert!(
            matches!(args, ManageArgs::ConnectionDiscover { client_id, scopes, metadata_url, .. } if client_id.is_empty() && scopes.is_empty() && metadata_url.is_none())
        );
        assert!(
            serde_json::from_value::<ManageArgs>(json!({
                "action": "connection.discover",
                "resource": "https://example.test/mcp",
                "attachment_id": "forged"
            }))
            .is_err()
        );
    }
}
