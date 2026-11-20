//! Owner-approved native and network execution. Credentials are never returned
//! through scoped APIs. An attempt is durably recorded before external dispatch.

use crate::engine::{bounded, link_artifacts};
use crate::store::{StoreLock, event, fault_point, now_ms, private_dir, private_file};
use crate::worker::{self, InterpreterPin, WorkerLimits, WorkerRequest};
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::{Auth, Connection, FileVault, HttpBroker, Secret};
use apr_core::{AdmittedProcess, ProcessManifest, Step, content_digest, validate_instance};
use fs2::FileExt;
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const MAX_EXTERNAL_WORKERS: usize = 64;

/// A claim's file remains on disk after settlement. The permanent inode avoids
/// an unlink/recreate race with a second runtime checking the same attempt.
struct ClaimLease {
    _file: File,
}

impl ClaimLease {
    fn acquire(root: &Path, attempt: &str) -> Result<Self> {
        let directory = root.join("execution-claims");
        private_dir(&directory)?;
        let file = private_file(&directory.join(attempt))?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }
}

pub(crate) fn claim_is_live(root: &Path, attempt: &str) -> Result<bool> {
    if !attempt.starts_with("attempt_") || !attempt[8..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(false);
    }
    let directory = root.join("execution-claims");
    let directory_metadata = match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !directory_metadata.is_dir()
        || directory_metadata.file_type().is_symlink()
        || directory_metadata.uid() != unsafe { libc::geteuid() }
        || directory_metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RuntimeError::Integrity(
            "execution claim directory is not private and owned".into(),
        ));
    }
    let path = directory.join(attempt);
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RuntimeError::Integrity(
            "execution claim lease is not a private owned file".into(),
        ));
    }
    match file.try_lock_exclusive() {
        Ok(()) => {
            FileExt::unlock(&file)?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(error.into()),
    }
}

struct ActiveSlot {
    run: String,
    kind: String,
    name: String,
}

struct CompletedExternal {
    job: PreparedExternal,
    observation: ExternalObservation,
}

/// The coordinator alone mutates state. Workers own only frozen dispatch data
/// and send observations back as they complete, independently of sibling work.
pub(crate) struct ExternalPool {
    sender: mpsc::Sender<CompletedExternal>,
    receiver: mpsc::Receiver<CompletedExternal>,
    active: BTreeMap<String, ActiveSlot>,
}

impl ExternalPool {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            active: BTreeMap::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.active.len()
    }
    pub(crate) fn contains(&self, attempt: &str) -> bool {
        self.active.contains_key(attempt)
    }
    pub(crate) fn has_run(&self, run: &str) -> bool {
        self.active.values().any(|slot| slot.run == run)
    }

    fn can_admit(
        &self,
        queue: &[PreparedExternal],
        kind: &str,
        name: &str,
        policy: &crate::worker_policy::WorkerPolicy,
    ) -> bool {
        if self.active.len() + queue.len() >= policy.max_workers as usize {
            return false;
        }
        if kind == "python" {
            let local = self
                .active
                .values()
                .filter(|slot| slot.kind == "python")
                .count()
                + queue
                    .iter()
                    .filter(|job| job.kind_and_name().0 == "python")
                    .count();
            return local < policy.max_local_workers as usize;
        }
        if kind == "connection"
            && let Some(cap) = policy.per_connection.get(name)
        {
            let active = self
                .active
                .values()
                .filter(|slot| slot.kind == "connection" && slot.name == name)
                .count()
                + queue
                    .iter()
                    .filter(|job| job.kind_and_name() == ("connection", name))
                    .count();
            return active < *cap as usize;
        }
        true
    }

    fn spawn(&mut self, job: PreparedExternal) -> std::result::Result<(), Box<PreparedExternal>> {
        let (kind, name) = job.kind_and_name();
        let slot = ActiveSlot {
            run: job.run.clone(),
            kind: kind.into(),
            name: name.into(),
        };
        let attempt = job.intent_id.clone();
        let sender = self.sender.clone();
        // Put the job in a cell so a thread creation error returns its durable
        // reservation to the coordinator for conservative settlement.
        let cell = Arc::new(Mutex::new(Some(job)));
        let worker_cell = cell.clone();
        match std::thread::Builder::new()
            .name("apr-external".into())
            .spawn(move || {
                let job = worker_cell
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .expect("worker job");
                let observation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    dispatch_prepared(&job)
                }))
                .unwrap_or_else(|_| uncertain_observation());
                let _ = sender.send(CompletedExternal { job, observation });
            }) {
            Ok(_handle) => {
                self.active.insert(attempt, slot);
                Ok(())
            }
            Err(_) => Err(Box::new(
                cell.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .expect("unstarted worker job"),
            )),
        }
    }

    fn take_completed(&mut self) -> Vec<CompletedExternal> {
        let mut completed = Vec::new();
        while let Ok(item) = self.receiver.try_recv() {
            self.active.remove(&item.job.intent_id);
            completed.push(item);
        }
        completed
    }
}

struct ActiveWorker {
    lineage: Vec<String>,
    flag: Arc<AtomicBool>,
}

#[derive(Default)]
struct InterruptState {
    active: BTreeMap<String, ActiveWorker>,
    cancel_latches: BTreeMap<String, BTreeSet<u64>>,
    next_generation: u64,
    stopping: bool,
}

#[derive(Clone, Default)]
pub(crate) struct ExecutionInterrupt(Arc<Mutex<InterruptState>>);

#[derive(Clone)]
pub(crate) struct CancelLatch {
    target: String,
    generation: u64,
}

impl ExecutionInterrupt {
    fn begin_attempt(&self, attempt: &str, lineage: Vec<String>) -> ActiveExecution {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let flag = Arc::new(AtomicBool::new(false));
        if state.stopping
            || lineage
                .iter()
                .any(|run| state.cancel_latches.contains_key(run))
        {
            flag.store(true, Ordering::Release);
        }
        state.active.insert(
            attempt.into(),
            ActiveWorker {
                lineage,
                flag: flag.clone(),
            },
        );
        ActiveExecution {
            registry: self.clone(),
            attempt: attempt.into(),
            flag,
        }
    }
    fn latch(&self, target: &str) -> CancelLatch {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.next_generation = state.next_generation.wrapping_add(1).max(1);
        let generation = state.next_generation;
        state
            .cancel_latches
            .entry(target.into())
            .or_default()
            .insert(generation);
        for active in state.active.values() {
            if active.lineage.iter().any(|run| run == target) {
                active.flag.store(true, Ordering::Release);
            }
        }
        CancelLatch {
            target: target.into(),
            generation,
        }
    }

    pub(crate) fn release_latch(&self, receipt: &CancelLatch) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let remove_target =
            state
                .cancel_latches
                .get_mut(&receipt.target)
                .is_some_and(|generations| {
                    generations.remove(&receipt.generation);
                    generations.is_empty()
                });
        if remove_target {
            state.cancel_latches.remove(&receipt.target);
        }
    }

    fn pending_for(&self, lineage: &[String]) -> bool {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.stopping
            || lineage
                .iter()
                .any(|run| state.cancel_latches.contains_key(run))
    }

    pub(crate) fn signal(&self, run: Option<&str>) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if run.is_none() {
            // The service is exiting. Keep this instance permanently closed to
            // attempts admitted after the shutdown signal reaches the registry.
            state.stopping = true;
        }
        for active in state.active.values() {
            if run.is_none_or(|run| active.lineage.iter().any(|member| member == run)) {
                active.flag.store(true, Ordering::Release);
            }
        }
    }

    fn signal_attempt(&self, attempt: &str) {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.get(attempt) {
            active.flag.store(true, Ordering::Release);
        }
    }
}
pub(crate) struct ActiveExecution {
    registry: ExecutionInterrupt,
    attempt: String,
    pub(crate) flag: Arc<AtomicBool>,
}
impl Drop for ActiveExecution {
    fn drop(&mut self) {
        let mut state = self.registry.0.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .active
            .get(&self.attempt)
            .is_some_and(|active| Arc::ptr_eq(&active.flag, &self.flag))
        {
            state.active.remove(&self.attempt);
        }
    }
}

struct OwnedWorkerInput {
    reference: apr_core::ArtifactRef,
    bytes: Vec<u8>,
}

enum ExternalDispatch {
    Python {
        pin: InterpreterPin,
        source: Vec<u8>,
        artifacts: Vec<OwnedWorkerInput>,
    },
    Connection(Box<Connection>),
}

/// A durably reserved external attempt. It owns every value needed by the
/// DB-free dispatch phase; only the coordinator may validate and commit it.
pub(crate) struct PreparedExternal {
    pub(crate) run: String,
    pub(crate) step_id: String,
    pub(crate) intent_id: String,
    step: Step,
    input: Value,
    config: Value,
    binding: Value,
    dispatch: ExternalDispatch,
    continuation: Option<(apr_connectors::mcp::interaction::Continuation, u32)>,
    fence: i64,
    timeout_ms: u64,
    artifact_limit: u64,
    result_artifact_limit: u64,
    root: std::path::PathBuf,
    active: ActiveExecution,
    _claim_lease: ClaimLease,
}

impl PreparedExternal {
    fn kind_and_name(&self) -> (&str, &str) {
        binding_for(&self.step).unwrap_or(("", ""))
    }
}

struct ExternalObservation {
    result: std::result::Result<RawExternalOutput, bool>,
    cost: Option<apr_connectors::metering::CostObservation>,
    connection_failure: Option<&'static str>,
    diagnostic: Option<Value>,
}

struct RawExternalOutput {
    output: Value,
    artifacts: Vec<worker::WorkerArtifact>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub run_id: String,
    pub step_instance_id: String,
    pub intent_id: String,
    /// observed_result, confirmed_no_effect, or cancel_uncertain.
    pub outcome: String,
    pub evidence: String,
    #[serde(default)]
    pub output: Option<Value>,
}

pub(crate) const MIGRATION_V5: &str = r#"
CREATE TABLE execution_grants (
 assignment_id TEXT NOT NULL REFERENCES assignments(id), kind TEXT NOT NULL,
 name TEXT NOT NULL, revision INTEGER NOT NULL, enabled INTEGER NOT NULL,
 payload TEXT NOT NULL, PRIMARY KEY(assignment_id,kind,name)
);
CREATE TABLE process_sources (
 version TEXT NOT NULL REFERENCES process_versions(version), path TEXT NOT NULL,
 sha256 TEXT NOT NULL, source BLOB NOT NULL, PRIMARY KEY(version,path)
);
CREATE TABLE run_bindings (
 run_id TEXT NOT NULL REFERENCES runs(id), kind TEXT NOT NULL, name TEXT NOT NULL,
 revision INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(run_id,kind,name)
);
CREATE TABLE effect_intents (
 id TEXT PRIMARY KEY REFERENCES attempts(id), run_id TEXT NOT NULL REFERENCES runs(id),
 step_id TEXT NOT NULL, kind TEXT NOT NULL, binding_name TEXT NOT NULL,
 binding_revision INTEGER NOT NULL, binding_snapshot TEXT NOT NULL, request_digest TEXT NOT NULL, request_snapshot TEXT NOT NULL,
 status TEXT NOT NULL, evidence TEXT, created_at INTEGER NOT NULL,
 FOREIGN KEY(run_id,step_id) REFERENCES steps(run_id,id)
);
CREATE INDEX effect_intents_run ON effect_intents(run_id,status);
"#;

impl Runtime {
    pub(crate) fn start_external_pool(&mut self) {
        self.external_pool = Some(ExternalPool::new());
    }

    pub(crate) fn external_pool_len(&self) -> usize {
        self.external_pool.as_ref().map_or(0, ExternalPool::len)
    }

    /// Owner CLI controls use a different Runtime and cannot reach this
    /// instance's interrupt registry. Observe their committed fences and
    /// signal only the exact attempts they invalidated. Pause and grant
    /// revocation leave an active attempt's fence intact.
    pub(crate) fn sync_pool_cancellations(&self) -> Result<()> {
        let Some(pool) = self.external_pool.as_ref() else {
            return Ok(());
        };
        if pool.active.is_empty() {
            return Ok(());
        }
        let _lock = StoreLock::acquire(&self.root)?;
        let mut query = self.db.prepare("SELECT a.id FROM attempts a JOIN steps s ON s.run_id=a.run_id AND s.id=a.step_id WHERE a.state='running' AND s.state='running' AND s.active_attempt=a.id")?;
        let valid: BTreeSet<String> = query
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for attempt in pool.active.keys() {
            if !valid.contains(attempt) {
                self.interrupt.signal_attempt(attempt);
            }
        }
        Ok(())
    }

    /// Called by the service coordinator between IPC requests. Each finished
    /// observation is committed before another slot is filled.
    pub(crate) fn settle_external_pool(&mut self) -> Result<bool> {
        let completed = self
            .external_pool
            .as_mut()
            .map_or_else(Vec::new, ExternalPool::take_completed);
        if completed.is_empty() {
            return Ok(false);
        }
        let _lock = StoreLock::acquire(&self.root)?;
        let mut first_error = None;
        for item in completed {
            if let Err(error) = self.settle_external_result(&item.job, item.observation)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(true)
        }
    }

    /// StoreLock must be held by the caller. Spawn failure is an observed
    /// not-sent attempt, so it is settled without replaying its reservation.
    pub(crate) fn spawn_external_pool_jobs(&mut self, jobs: Vec<PreparedExternal>) -> Result<()> {
        let mut first_error = None;
        for job in jobs {
            let pool = self
                .external_pool
                .as_mut()
                .ok_or_else(|| RuntimeError::State("external pool is not active".into()))?;
            if let Err(job) = pool.spawn(job) {
                let observation = ExternalObservation {
                    result: Err(false),
                    cost: None,
                    connection_failure: Some("external worker could not start"),
                    diagnostic: Some(json!({"stage":"dispatch","code":"spawn_failed"})),
                };
                if let Err(error) = self.settle_external_result(&job, observation)
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn settle_external_result(
        &mut self,
        job: &PreparedExternal,
        observation: ExternalObservation,
    ) -> Result<()> {
        let status: String = self.db.query_row(
            "SELECT status FROM effect_intents WHERE id=?1",
            [&job.intent_id],
            |row| row.get(0),
        )?;
        if status != "dispatched" {
            // A durable cancellation may have fenced this in-flight effect
            // before the observation arrived. Never revive its attempt.
            return Ok(());
        }
        match self.finish_prepared_external(job, observation) {
            Ok(true) => {
                self.db.execute("UPDATE runs SET state='queued',error=NULL WHERE id=?1 AND state IN ('running','queued','waiting_for_agent','waiting_for_child')", [&job.run])?;
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(error) => {
                let _ = self.recover_prepared_finish(job);
                Err(error)
            }
        }
    }

    fn execution_lineage(&self, run: &str) -> Result<Vec<String>> {
        let mut query = self.db.prepare(
            "WITH RECURSIVE lineage(id,parent_run_id) AS (SELECT id,parent_run_id FROM runs WHERE id=?1 UNION SELECT parent.id,parent.parent_run_id FROM runs parent JOIN lineage child ON parent.id=child.parent_run_id) SELECT id FROM lineage",
        )?;
        let lineage = query
            .query_map([run], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        if lineage.is_empty() {
            return Err(RuntimeError::State("run does not exist".into()));
        }
        Ok(lineage)
    }

    pub(crate) fn cancellation_pending(&self, run: &str) -> Result<bool> {
        Ok(self.interrupt.pending_for(&self.execution_lineage(run)?))
    }

    pub(crate) fn begin_interruptible(&self, run: &str, attempt: &str) -> Result<ActiveExecution> {
        Ok(self
            .interrupt
            .begin_attempt(attempt, self.execution_lineage(run)?))
    }

    /// Owner-only interpreter grant metadata. Inspection never grants execution.
    pub fn python_execution_status(&self, assignment: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_assignment_active(assignment)?;
        let existing: Option<(u64, bool, String)> = self.db.query_row(
            "SELECT revision,enabled,payload FROM execution_grants WHERE assignment_id=?1 AND kind='python' AND name='python'",
            [assignment], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        ).optional()?;
        match existing {
            Some((revision, enabled, payload)) => {
                let pin: InterpreterPin = serde_json::from_str(&payload)?;
                let current = InterpreterPin::inspect(&pin.path).is_ok_and(|value| value == pin);
                Ok(
                    json!({"assignment_id":assignment,"enabled":enabled,"revision":revision,"interpreter":pin,"unchanged":current}),
                )
            }
            None => Ok(
                json!({"assignment_id":assignment,"enabled":false,"revision":null,"interpreter":null,"unchanged":false}),
            ),
        }
    }

    /// Explicit trusted-native grant. Inspection alone never creates a grant.
    pub fn grant_python(&mut self, assignment: &str, interpreter: &Path) -> Result<Value> {
        self.grant_python_inner(assignment, interpreter, None)
    }

    /// Owner-reviewed grant bound to the interpreter digest shown during review.
    pub fn grant_python_reviewed(
        &mut self,
        assignment: &str,
        interpreter: &Path,
        expected_sha256: &str,
    ) -> Result<Value> {
        self.grant_python_inner(assignment, interpreter, Some(expected_sha256))
    }

    fn grant_python_inner(
        &mut self,
        assignment: &str,
        interpreter: &Path,
        expected_sha256: Option<&str>,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_assignment_active(assignment)?;
        let pin = InterpreterPin::inspect(interpreter)
            .map_err(|_| RuntimeError::InvalidInput("interpreter cannot be pinned".into()))?;
        if expected_sha256.is_some_and(|expected| expected != pin.sha256) {
            return Err(RuntimeError::InvalidInput(
                "interpreter digest changed since review".into(),
            ));
        }
        let existing: Option<(u64, bool, String)> = self
            .db
            .query_row(
                "SELECT revision,enabled,payload FROM execution_grants WHERE assignment_id=?1 AND kind='python' AND name='python'",
                [assignment],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((revision, true, payload)) = existing
            && serde_json::from_str::<InterpreterPin>(&payload)? == pin
        {
            return Ok(
                json!({"assignment_id":assignment,"kind":"python","name":"python","revision":revision,"enabled":true}),
            );
        }
        self.save_grant(assignment, "python", "python", &serde_json::to_value(pin)?)
    }

    /// Set a new immutable binding revision and put its credential only in the
    /// private local store. A changed binding deliberately blocks old waiting runs.
    pub fn configure_connection(
        &mut self,
        assignment: &str,
        name: &str,
        mut connection: Connection,
        secret: Option<Secret>,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_assignment_active(assignment)?;
        check_name(name)?;
        let reference = format!("credential-{}", uuid::Uuid::new_v4().simple());
        match &mut connection.auth {
            Auth::None => {
                if secret.is_some() {
                    return Err(RuntimeError::InvalidInput(
                        "unauthenticated binding does not accept a secret".into(),
                    ));
                }
            }
            Auth::Bearer { credential_ref }
            | Auth::Header { credential_ref, .. }
            | Auth::Oauth { credential_ref, .. } => {
                if secret.is_none() {
                    return Err(RuntimeError::InvalidInput(
                        "authenticated binding requires protected secret input".into(),
                    ));
                }
                *credential_ref = reference.clone();
            }
        }
        connection
            .validate()
            .map_err(|_| RuntimeError::InvalidInput("invalid connection configuration".into()))?;
        let vault = self.vault()?;
        if let Some(secret) = secret {
            vault.set(&reference, secret).map_err(|_| {
                RuntimeError::State("protected credential store unavailable".into())
            })?;
        }
        match self.save_grant(
            assignment,
            "connection",
            name,
            &serde_json::to_value(connection)?,
        ) {
            Ok(result) => Ok(result),
            Err(error) => {
                let _ = vault.remove(&reference);
                Err(error)
            }
        }
    }

    pub fn revoke_execution(&mut self, assignment: &str, kind: &str, name: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let tx = self.db.transaction()?;
        if kind == "connection" {
            // A project-specific denial wins over the library's general approval,
            // including attempts to bind the same tool under another alias.
            tx.execute("INSERT OR IGNORE INTO connection_library_blocks(assignment_id,library_id) SELECT u.assignment_id,u.library_id FROM connection_library_uses u JOIN execution_grants g ON g.assignment_id=u.assignment_id AND g.name=u.name AND g.kind='connection' AND g.revision=u.binding_revision WHERE u.assignment_id=?1 AND u.name=?2",params![assignment,name])?;
            tx.execute("UPDATE execution_grants SET enabled=0,revision=revision+1 WHERE assignment_id=?1 AND kind='connection' AND name<>?2 AND enabled=1 AND EXISTS(SELECT 1 FROM connection_library_uses u JOIN connection_library_blocks b ON b.assignment_id=u.assignment_id AND b.library_id=u.library_id WHERE u.assignment_id=execution_grants.assignment_id AND u.name=execution_grants.name AND u.binding_revision=execution_grants.revision)",params![assignment,name])?;
        }
        let changed = tx.execute("UPDATE execution_grants SET enabled=0,revision=revision+1 WHERE assignment_id=?1 AND kind=?2 AND name=?3 AND enabled=1",params![assignment,kind,name])?;
        tx.commit()?;
        Ok(json!({"revoked":changed>0,"kind":kind,"name":name}))
    }

    /// Explicit owner acknowledgement after restoring or replacing a binding.
    /// Pending uncertain effects must be reconciled before any new dispatch.
    pub fn rebind_execution(&mut self, requested_run: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let (root, _) = self.root_limits(requested_run)?;
        let run = root.as_str();
        let unresolved:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM effect_intents e JOIN runs r ON r.id=e.run_id WHERE COALESCE(r.root_run_id,r.id)=?1 AND e.status IN ('dispatched','uncertain'))",[run],|r|r.get(0))?;
        if unresolved {
            return Err(RuntimeError::State(
                "reconcile uncertain effects in the run tree before rebinding".into(),
            ));
        }
        let assignment: Option<String> =
            self.db
                .query_row("SELECT assignment_id FROM runs WHERE id=?1", [run], |r| {
                    r.get(0)
                })?;
        let assignment = assignment.ok_or(RuntimeError::Forbidden)?;
        self.ensure_assignment_active(&assignment)?;
        let processes = self.pinned_processes(run)?;
        for process in &processes {
            self.execution_preflight(process, Some(&assignment))?;
        }
        let state = self.ensure_run(run)?;
        if !matches!(
            state.as_str(),
            "paused"
                | "blocked"
                | "queued"
                | "waiting_for_agent"
                | "waiting_for_child"
                | "waiting_for_children"
        ) {
            return Err(RuntimeError::State(
                "run tree cannot be rebound in its current state".into(),
            ));
        }
        let tx = self.db.transaction()?;
        // Each dispatched intent retains its own immutable binding snapshot.
        tx.execute("DELETE FROM run_bindings WHERE run_id IN (SELECT id FROM runs WHERE COALESCE(root_run_id,id)=?1)",[run])?;
        for process in &processes {
            freeze_execution_bindings(&tx, run, &process.manifest, Some(&assignment))?;
        }
        tx.execute("INSERT INTO run_bindings(run_id,kind,name,revision,payload) SELECT r.id,b.kind,b.name,b.revision,b.payload FROM runs r JOIN run_bindings b ON b.run_id=?1 WHERE r.root_run_id=?1 AND r.id<>?1",[run])?;
        event(
            &tx,
            run,
            "execution.rebound",
            json!({"owner_action":true,"scope":"run_tree"}),
        )?;
        tx.commit()?;
        Ok(json!({"run_id":run,"rebound":true,"state":state}))
    }

    pub(crate) fn execution_catalog(&self, assignment: &str) -> Result<Value> {
        let mut statement = self.db.prepare("SELECT kind,name,revision,enabled FROM execution_grants WHERE assignment_id=?1 ORDER BY kind,name")?;
        let grants = statement.query_map([assignment],|row|Ok(json!({"kind":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"revision":row.get::<_,u64>(2)?,"enabled":row.get::<_,bool>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(json!({"bindings":grants}))
    }

    pub(crate) fn effect_view(&self, run: &str) -> Result<Value> {
        let mut statement=self.db.prepare("SELECT id,step_id,kind,binding_name,binding_revision,request_digest,status FROM effect_intents WHERE run_id=?1 ORDER BY created_at,id LIMIT 101")?;
        let mut intents=statement.query_map([run],|r|Ok(json!({"intent_id":r.get::<_,String>(0)?,"step_instance_id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"binding":r.get::<_,String>(3)?,"binding_revision":r.get::<_,u64>(4)?,"request_digest":r.get::<_,String>(5)?,"status":r.get::<_,String>(6)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let truncated = intents.len() > 100;
        intents.truncate(100);
        Ok(json!({"run_id":run,"intents":intents,"truncated":truncated}))
    }

    pub fn reconcile(&mut self, request: &Reconciliation) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.reconcile_inner(request)
    }

    pub(crate) fn reconcile_inner(&mut self, request: &Reconciliation) -> Result<Value> {
        if request.evidence.trim().is_empty() || request.evidence.len() > 2000 {
            return Err(RuntimeError::InvalidInput(
                "bounded observed evidence is required".into(),
            ));
        }
        if let Some(result) = self.reconcile_sampling(request)? {
            return Ok(result);
        }
        let row:Option<(String,String,String,u32,String,String,String)>=self.db.query_row(
            "SELECT e.status,e.kind,e.binding_name,s.attempt_count,e.binding_snapshot,e.request_snapshot,e.request_digest FROM effect_intents e JOIN steps s ON s.run_id=e.run_id AND s.id=e.step_id WHERE e.id=?1 AND e.run_id=?2 AND e.step_id=?3 AND s.state IN ('needs_reconciliation','blocked','cancelled')",
            params![request.intent_id,request.run_id,request.step_instance_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
        let (status, kind, name, attempts, binding, saved_input, request_digest) =
            row.ok_or_else(|| RuntimeError::State("intent is not awaiting reconciliation".into()))?;
        if !matches!(status.as_str(), "uncertain" | "not_sent") {
            return Err(RuntimeError::State("intent is already resolved".into()));
        }
        let manifest:String=self.db.query_row("SELECT v.manifest FROM process_versions v JOIN runs r ON r.version=v.version WHERE r.id=?1",[&request.run_id],|r|r.get(0))?;
        let process = apr_core::admit_json(manifest.as_bytes())?;
        // Foreach instance IDs carry the source definition in their persisted row.
        let definition: String = self.db.query_row(
            "SELECT COALESCE(definition_id,id) FROM steps WHERE run_id=?1 AND id=?2",
            params![request.run_id, request.step_instance_id],
            |r| r.get(0),
        )?;
        let step = process
            .manifest
            .steps
            .iter()
            .find(|s| s.id == definition)
            .ok_or_else(|| RuntimeError::State("saved step is absent from manifest".into()))?;
        let mut references = Vec::new();
        let previous_state = self.ensure_run(&request.run_id)?;
        let mut committed_output = None;
        let mut cost_observation = None;
        let limits = self.effective_limits(&request.run_id, &process.manifest.limits)?;
        let (intent_state, mut step_state, mut run_state) = match request.outcome.as_str() {
            "observed_result" => {
                let output = request.output.as_ref().ok_or_else(|| {
                    RuntimeError::InvalidInput("observed_result needs output".into())
                })?;
                bounded(output)?;
                let mut output = output.clone();
                if kind == "connection" {
                    let connection: Connection = serde_json::from_str(&binding)?;
                    if connection.mcp.is_some() {
                        apr_connectors::mcp::validate_completed_output(&connection, &output)
                            .map_err(|_| {
                                RuntimeError::InvalidInput(
                                    "observed MCP result does not match the approved completion contract"
                                        .into(),
                                )
                            })?;
                    } else {
                        validate_instance(&connection.output_schema, &output)?;
                    }
                    let metering_input: Value = serde_json::from_str(&saved_input)?;
                    if content_digest(&metering_input)? != request_digest {
                        return Err(RuntimeError::Integrity(
                            "saved request digest differs".into(),
                        ));
                    }
                    cost_observation = connection
                        .metering
                        .as_ref()
                        .map(|m| m.observe_exchange(&metering_input, &output));
                }
                if step.operation.as_deref() == Some("jev.evaluate") {
                    #[cfg(not(feature = "jev"))]
                    return Err(RuntimeError::State(
                        "Jev support is required to reconcile its response".into(),
                    ));
                    #[cfg(feature = "jev")]
                    {
                        let input: Value = serde_json::from_str(&saved_input)?;
                        if content_digest(&input)? != request_digest {
                            return Err(RuntimeError::Integrity(
                                "saved request digest differs".into(),
                            ));
                        }
                        let prepared = apr_provider_jev::prepare(input).map_err(|_| {
                            RuntimeError::Integrity("saved Jev request is invalid".into())
                        })?;
                        output = serde_json::to_value(
                            apr_provider_jev::observe(&prepared, output).map_err(|_| {
                                RuntimeError::InvalidInput(
                                    "observed Jev response does not match the saved request".into(),
                                )
                            })?,
                        )?;
                    }
                }
                #[cfg(not(feature = "jev"))]
                let _ = (&saved_input, &request_digest, &mut output);
                bounded(&output)?;
                if let Some(schema) = &step.result_schema {
                    validate_instance(schema, &output)?;
                }
                references = self.checked_refs(&output, limits.max_artifact_bytes)?;
                let assignment: Option<String> = self.db.query_row(
                    "SELECT assignment_id FROM runs WHERE id=?1",
                    [&request.run_id],
                    |r| r.get(0),
                )?;
                if let Some(assignment) = assignment {
                    self.authorize_artifacts(&assignment, &output)?;
                }
                committed_output = Some(output);
                ("reconciled_success", "succeeded", "queued")
            }
            "confirmed_no_effect" => {
                if request.output.is_some() || attempts >= 5 {
                    return Err(RuntimeError::LimitExceeded(
                        "reconciliation retry allowance exhausted or output is unexpected".into(),
                    ));
                }
                let state = if self.current_binding(&request.run_id, &kind, &name).is_ok() {
                    "queued"
                } else {
                    "blocked"
                };
                ("reconciled_no_effect", "queued", state)
            }
            "cancel_uncertain" => {
                if request.output.is_some() {
                    return Err(RuntimeError::InvalidInput(
                        "cancel does not accept output".into(),
                    ));
                }
                ("cancelled_uncertain", "cancelled", "cancelled")
            }
            _ => {
                return Err(RuntimeError::InvalidInput(
                    "unsupported reconciliation outcome".into(),
                ));
            }
        };
        if matches!(previous_state.as_str(), "cancelled" | "failed" | "paused") {
            run_state = previous_state.as_str();
            if previous_state == "cancelled" && step_state != "succeeded" {
                step_state = "cancelled";
            }
        }
        let tx = self.db.transaction()?;
        let changed=tx.execute("UPDATE effect_intents SET status=?2,evidence=?3 WHERE id=?1 AND status IN ('uncertain','not_sent')",params![request.intent_id,intent_state,request.evidence])?;
        if changed != 1 {
            return Err(RuntimeError::State("intent already resolved".into()));
        }
        link_artifacts(&tx, &request.run_id, &request.step_instance_id, &references)?;
        tx.execute(
            "UPDATE steps SET state=?3,output=?4,active_attempt=NULL WHERE run_id=?1 AND id=?2",
            params![
                request.run_id,
                request.step_instance_id,
                step_state,
                committed_output
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?
            ],
        )?;
        if request.outcome == "cancel_uncertain" {
            crate::scope::cancel_in_transaction(&tx, &request.run_id, "effect.cancel_uncertain")?;
        } else {
            if !matches!(run_state, "cancelled" | "failed" | "paused") {
                let unresolved:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM effect_intents WHERE run_id=?1 AND status IN ('dispatched','uncertain'))",[&request.run_id],|r|r.get(0))?;
                if unresolved {
                    run_state = "needs_reconciliation";
                }
            }
            tx.execute("UPDATE runs SET state=?2,error=CASE WHEN state IN ('failed','paused','cancelled') THEN error ELSE NULL END WHERE id=?1",params![request.run_id,run_state])?;
        }
        let observed_value = cost_observation
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        crate::billing::settle(
            &tx,
            &request.intent_id,
            intent_state,
            observed_value.as_ref(),
            "caller_reported",
        )?;
        if let Some(observation) = cost_observation {
            event(
                &tx,
                &request.run_id,
                "usage.observed",
                json!({
                    "intent_id": request.intent_id,
                    "provenance": "caller_reported",
                    "observation": observation
                }),
            )?;
        }
        event(
            &tx,
            &request.run_id,
            "effect.reconciled",
            json!({"step_id":request.step_instance_id,"intent_id":request.intent_id,"outcome":request.outcome,"evidence_digest":content_digest(&json!(request.evidence))?}),
        )?;
        tx.commit()?;
        Ok(
            json!({"intent_id":request.intent_id,"status":intent_state,"run":self.envelope(&request.run_id)?}),
        )
    }

    fn save_grant(
        &mut self,
        assignment: &str,
        kind: &str,
        name: &str,
        payload: &Value,
    ) -> Result<Value> {
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO execution_grants(assignment_id,kind,name,revision,enabled,payload) VALUES (?1,?2,?3,1,1,?4) ON CONFLICT(assignment_id,kind,name) DO UPDATE SET revision=execution_grants.revision+1,enabled=1,payload=excluded.payload",params![assignment,kind,name,serde_json::to_string(payload)?])?;
        if kind == "connection" {
            crate::connection_library::register_account(
                &tx,
                &serde_json::from_value(payload.clone())?,
            )?;
        }
        let revision: u64 = tx.query_row(
            "SELECT revision FROM execution_grants WHERE assignment_id=?1 AND kind=?2 AND name=?3",
            params![assignment, kind, name],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(
            json!({"assignment_id":assignment,"kind":kind,"name":name,"revision":revision,"enabled":true}),
        )
    }

    pub(crate) fn vault(&self) -> Result<FileVault> {
        FileVault::new(&self.root)
            .map_err(|_| RuntimeError::State("protected credential store unavailable".into()))
    }

    pub(crate) fn snapshot_sources(
        &mut self,
        process: &AdmittedProcess,
        sources: Vec<(String, Vec<u8>)>,
    ) -> Result<()> {
        let declared = process.manifest.files.as_deref().unwrap_or(&[]);
        if declared.len() != sources.len() {
            return Err(RuntimeError::InvalidInput(
                "all declared source files are required".into(),
            ));
        }
        let tx = self.db.transaction()?;
        tx.execute("INSERT OR IGNORE INTO process_versions(version,manifest,canonical_format) VALUES (?1,?2,?3)",params![process.version,String::from_utf8_lossy(&process.canonical_bytes),apr_core::CANONICAL_FORMAT])?;
        let mut total = 0;
        for (path, bytes) in sources {
            total += bytes.len();
            if total > 1024 * 1024 || std::str::from_utf8(&bytes).is_err() {
                return Err(RuntimeError::LimitExceeded(
                    "source bundle must be UTF-8 and at most 1 MiB".into(),
                ));
            }
            let digest = format!("{:x}", Sha256::digest(&bytes));
            if !declared
                .iter()
                .any(|file| file.path == path && file.sha256 == digest)
            {
                return Err(RuntimeError::Integrity(
                    "source digest does not match manifest".into(),
                ));
            }
            tx.execute("INSERT OR IGNORE INTO process_sources(version,path,sha256,source) VALUES (?1,?2,?3,?4)",params![process.version,path,digest,bytes])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn execution_preflight(
        &self,
        process: &AdmittedProcess,
        assignment: Option<&str>,
    ) -> Result<()> {
        for step in &process.manifest.steps {
            #[cfg(not(feature = "jev"))]
            if step.operation.as_deref() == Some("jev.evaluate") {
                return Err(RuntimeError::State(
                    "Jev support is not enabled in this build".into(),
                ));
            }
            let Some((kind, name)) = binding_for(step) else {
                continue;
            };
            let assignment = assignment.ok_or(RuntimeError::Forbidden)?;
            let allowed:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM execution_grants WHERE assignment_id=?1 AND kind=?2 AND name=?3 AND enabled=1)",params![assignment,kind,name],|r|r.get(0))?;
            if !allowed {
                return Err(RuntimeError::Forbidden);
            }
            if kind == "python" {
                let path = step.entrypoint.as_deref().ok_or(RuntimeError::Forbidden)?;
                let present: bool = self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM process_sources WHERE version=?1 AND path=?2)",
                    params![process.version, path],
                    |r| r.get(0),
                )?;
                if !present {
                    return Err(RuntimeError::InvalidInput(
                        "script has no admitted source snapshot".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn current_binding(
        &self,
        run: &str,
        kind: &str,
        name: &str,
    ) -> Result<(u64, Value)> {
        let binding:Option<(u64,String)>=self.db.query_row("SELECT b.revision,b.payload FROM run_bindings b JOIN runs r ON r.id=b.run_id JOIN execution_grants g ON g.assignment_id=r.assignment_id AND g.kind=b.kind AND g.name=b.name WHERE b.run_id=?1 AND b.kind=?2 AND b.name=?3 AND g.enabled=1 AND g.revision=b.revision AND g.payload=b.payload",params![run,kind,name],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (revision, payload) = binding.ok_or(RuntimeError::Forbidden)?;
        Ok((revision, serde_json::from_str(&payload)?))
    }

    pub(crate) fn execute_external(
        &mut self,
        run: &str,
        step: &Step,
        input: &Value,
        manifest: &ProcessManifest,
        config: &Value,
    ) -> Result<()> {
        let Some(prepared) = self.prepare_external(run, step, input, manifest, config)? else {
            return Ok(());
        };
        if let Some(queue) = self.external_queue.as_mut() {
            queue.push(prepared);
            return Ok(());
        }
        self.finish_external_wave(vec![prepared])
    }

    /// A missing or malformed binding is deliberately queueable: ordinary
    /// preparation records that error. Capacity is shared across root and
    /// child runs. OAuth refresh synchronization lives in the credential store.
    pub(crate) fn external_can_queue(&self, _run: &str, step: &Step) -> bool {
        let Some(queue) = self.external_queue.as_ref() else {
            return true;
        };
        let Some(policy) = self.dispatch_policy.as_ref() else {
            return false;
        };
        let Some((kind, name)) = binding_for(step) else {
            return true;
        };
        let admitted = if let Some(pool) = self.external_pool.as_ref() {
            pool.can_admit(queue, kind, name, policy)
        } else {
            if queue.len() >= policy.max_workers as usize {
                return false;
            }
            if kind == "python" {
                queue
                    .iter()
                    .filter(|job| job.kind_and_name().0 == "python")
                    .count()
                    < policy.max_local_workers as usize
            } else if kind == "connection" {
                policy.per_connection.get(name).is_none_or(|cap| {
                    queue
                        .iter()
                        .filter(|job| job.kind_and_name() == (kind, name))
                        .count()
                        < *cap as usize
                })
            } else {
                true
            }
        };
        if !admitted {
            return false;
        }
        true
    }

    fn prepare_external(
        &mut self,
        run: &str,
        step: &Step,
        input: &Value,
        manifest: &ProcessManifest,
        config: &Value,
    ) -> Result<Option<PreparedExternal>> {
        if self.cancellation_pending(run)? {
            return Ok(None);
        }
        let (kind, name) = binding_for(step).ok_or(RuntimeError::Forbidden)?;
        let (revision, binding) = match self.current_binding(run, kind, name) {
            Ok(binding) => binding,
            Err(_) => {
                self.set_run_state(
                    run,
                    "blocked",
                    Some("execution binding is missing, revoked or changed"),
                )?;
                return Ok(None);
            }
        };
        bounded(input)?;
        if kind == "connection" {
            let connection: Connection = serde_json::from_value(binding.clone())?;
            if (step.operation.as_deref() == Some("mcp.call")) != connection.mcp.is_some() {
                self.set_run_state(
                    run,
                    "blocked",
                    Some("recipe operation differs from its approved connection transport"),
                )?;
                return Ok(None);
            }
        }
        if step.operation.as_deref() == Some("jev.evaluate") {
            #[cfg(not(feature = "jev"))]
            {
                self.set_run_state(
                    run,
                    "blocked",
                    Some("Jev support is not enabled in this build"),
                )?;
                return Ok(None);
            }
            #[cfg(feature = "jev")]
            {
                let connection: Connection = serde_json::from_value(binding.clone())?;
                if (connection.endpoint != apr_provider_jev::ENDPOINT && !connection.loopback_test)
                    || !matches!(connection.auth, Auth::Bearer { .. })
                {
                    self.set_run_state(
                        run,
                        "blocked",
                        Some("Jev requires a protected TypeSafe binding"),
                    )?;
                    return Ok(None);
                }
                apr_provider_jev::prepare(input.clone())
                    .map_err(|_| RuntimeError::InvalidInput("invalid Jev request".into()))?;
            }
        }
        let used: u64 = self.db.query_row(
            "SELECT count(*) FROM effect_intents WHERE run_id=?1 AND kind='connection'",
            [run],
            |r| r.get(0),
        )?;
        if kind == "connection"
            && (used >= manifest.limits.max_provider_calls as u64
                || !self.shared_provider_budget_available(run)?)
        {
            self.set_run_state(run, "failed", Some("provider call budget exhausted"))?;
            return Ok(None);
        }
        let source = if kind == "python" {
            let (bytes,digest):(Vec<u8>,String)=self.db.query_row("SELECT s.source,s.sha256 FROM process_sources s JOIN runs r ON r.version=s.version WHERE r.id=?1 AND s.path=?2",params![run,step.entrypoint],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if format!("{:x}", Sha256::digest(&bytes)) != digest {
                return Err(RuntimeError::Integrity(
                    "saved source digest mismatch".into(),
                ));
            }
            Some(bytes)
        } else {
            None
        };
        let artifact_limit = manifest.limits.max_artifact_bytes.min(64 * 1024 * 1024);
        let mut input_refs = BTreeMap::new();
        if kind == "python" {
            for reference in self
                .checked_refs(input, artifact_limit)?
                .into_iter()
                .chain(self.checked_refs(config, artifact_limit)?)
            {
                input_refs.insert(reference.artifact_id.clone(), reference);
            }
        }
        if input_refs.len() > 32 {
            return Err(RuntimeError::LimitExceeded(
                "worker exchange exceeds 32 files".into(),
            ));
        }
        let mut materialized = Vec::new();
        let mut total = 0_u64;
        for reference in input_refs.values() {
            total = total.saturating_add(reference.bytes);
            if total > artifact_limit {
                return Err(RuntimeError::LimitExceeded(
                    "worker artifact input exceeds byte allowance".into(),
                ));
            }
            materialized.push((reference, self.artifacts.read(reference, artifact_limit)?));
        }
        let worker_inputs: Vec<_> = materialized
            .iter()
            .map(|(reference, bytes)| OwnedWorkerInput {
                reference: (*reference).clone(),
                bytes: bytes.clone(),
            })
            .collect();
        let id = format!("attempt_{}", uuid::Uuid::new_v4().simple());
        // Register before any reservation. Registration and cancellation both
        // take the interrupt mutex, so a later authorized latch either sees
        // this attempt or this attempt starts already signalled.
        let active = self.begin_interruptible(run, &id)?;
        if active.flag.load(Ordering::Acquire) {
            return Ok(None);
        }
        // Hold this lease before committing the dispatch intent. A second
        // Runtime may acquire StoreLock while the worker runs, but it cannot
        // mistake this claim for crash residue.
        let claim_lease = ClaimLease::acquire(&self.root, &id)?;
        let tx = self.db.transaction()?;
        let continuation = if kind == "connection" {
            match crate::interactions::claim(&tx, run, &step.id, &id, input, &binding) {
                Ok(value) => value,
                Err(error) => {
                    drop(tx);
                    self.set_run_state(run, "blocked", Some(&error.to_string()))?;
                    return Ok(None);
                }
            }
        } else {
            None
        };
        let saved_binding = binding.clone();

        if kind == "connection"
            && let Err(error) = crate::billing::reserve(&tx, run, &id, name, revision, &binding)
        {
            drop(tx);
            self.set_run_state(run, "blocked", Some(&error.to_string()))?;
            return Ok(None);
        }
        let changed=tx.execute("UPDATE steps SET state='running',attempt_count=attempt_count+1,fence=fence+1,active_attempt=?3 WHERE run_id=?1 AND id=?2 AND state='queued'",params![run,step.id,id])?;
        if changed != 1 {
            return Err(RuntimeError::State("step is not dispatchable".into()));
        }
        let (number, fence): (u32, i64) = tx.query_row(
            "SELECT attempt_count,fence FROM steps WHERE run_id=?1 AND id=?2",
            params![run, step.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        tx.execute("INSERT INTO attempts(id,run_id,step_id,number,fence,state,started_at) VALUES (?1,?2,?3,?4,?5,'running',?6)",params![id,run,step.id,number,fence,now_ms()])?;
        tx.execute("INSERT INTO effect_intents(id,run_id,step_id,kind,binding_name,binding_revision,binding_snapshot,request_digest,request_snapshot,status,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'dispatched',?10)",params![id,run,step.id,kind,name,revision,serde_json::to_string(&binding)?,content_digest(input)?,serde_json::to_string(input)?,now_ms()])?;
        tx.execute(
            "UPDATE runs SET state='running',error=NULL WHERE id=?1",
            [run],
        )?;
        event(
            &tx,
            run,
            "effect.dispatched",
            json!({"step_id":step.id,"intent_id":id,"kind":kind,"binding_revision":revision}),
        )?;
        tx.commit()?;
        fault_point(&format!("effect_dispatched:{}", step.id));
        let dispatch = if kind == "python" {
            ExternalDispatch::Python {
                pin: serde_json::from_value(binding.clone())?,
                source: source.unwrap_or_default(),
                artifacts: worker_inputs,
            }
        } else {
            let mut connection: Connection = serde_json::from_value(binding.clone())?;
            connection.timeout_ms = connection.timeout_ms.min(manifest.limits.max_step_ms);
            ExternalDispatch::Connection(Box::new(connection))
        };
        Ok(Some(PreparedExternal {
            run: run.into(),
            step_id: step.id.clone(),
            intent_id: id,
            step: step.clone(),
            input: input.clone(),
            config: config.clone(),
            binding: saved_binding,
            dispatch,
            continuation,
            fence,
            timeout_ms: manifest.limits.max_step_ms.min(120_000),
            artifact_limit,
            result_artifact_limit: manifest.limits.max_artifact_bytes,
            root: self.root.clone(),
            active,
            _claim_lease: claim_lease,
        }))
    }

    pub(crate) fn finish_external_wave(&mut self, jobs: Vec<PreparedExternal>) -> Result<()> {
        if jobs.is_empty() {
            return Ok(());
        }
        if jobs.len() > MAX_EXTERNAL_WORKERS {
            return Err(RuntimeError::LimitExceeded(
                "external worker wave exceeds its fixed bound".into(),
            ));
        }
        let mut affected = std::collections::BTreeSet::new();
        for job in &jobs {
            affected.insert(job.run.clone());
        }
        let tx = self.db.transaction()?;
        for run in &affected {
            tx.execute(
                "UPDATE runs SET state='running',error=NULL WHERE id=?1 AND state IN ('queued','running')",
                [run],
            )?;
        }
        tx.commit()?;
        fault_point("external_wave_prepared");

        let observations = std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|job| {
                    std::thread::Builder::new()
                        .name("apr-external".into())
                        .spawn_scoped(scope, move || dispatch_prepared(job))
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| match handle {
                    Ok(handle) => handle.join().unwrap_or_else(|_| uncertain_observation()),
                    Err(_) => uncertain_observation(),
                })
                .collect::<Vec<_>>()
        });

        let mut succeeded = std::collections::BTreeSet::new();
        let mut first_error = None;
        for (job, observation) in jobs.iter().zip(observations) {
            match self.finish_prepared_external(job, observation) {
                Ok(true) => {
                    succeeded.insert(job.run.clone());
                }
                Ok(false) => {}
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    // The external phase completed but its durable finish did not.
                    // Fence the still-owned intent as uncertain before attempting
                    // later siblings; it must never be replayed to recover state.
                    let _ = self.recover_prepared_finish(job);
                }
            }
        }
        let requeue = (|| -> Result<()> {
            let tx = self.db.transaction()?;
            for run in succeeded {
                tx.execute(
                    "UPDATE runs SET state='queued',error=NULL WHERE id=?1 AND state IN ('running','queued','waiting_for_agent','waiting_for_child')",
                    [run],
                )?;
            }
            tx.commit()?;
            Ok(())
        })();
        if let Some(error) = first_error {
            let _ = requeue;
            Err(error)
        } else {
            requeue
        }
    }

    fn recover_prepared_finish(&mut self, job: &PreparedExternal) -> Result<()> {
        let tx = self.db.transaction()?;
        let changed = tx.execute(
            "UPDATE steps SET state='needs_reconciliation',active_attempt=NULL,fence=fence+1 WHERE run_id=?1 AND id=?2 AND active_attempt=?3 AND fence=?4 AND state='running'",
            params![job.run, job.step_id, job.intent_id, job.fence],
        )?;
        if changed == 1 {
            tx.execute(
                "UPDATE effect_intents SET status='uncertain' WHERE id=?1 AND status='dispatched'",
                [&job.intent_id],
            )?;
            tx.execute(
                "UPDATE attempts SET state='needs_reconciliation',finished_at=?2,error='external result could not be committed' WHERE id=?1 AND state='running'",
                params![job.intent_id, now_ms()],
            )?;
            set_external_run_state(
                &tx,
                &job.run,
                "needs_reconciliation",
                "external outcome requires reconciliation",
            )?;
            crate::billing::settle(&tx, &job.intent_id, "uncertain", None, "broker_observed")?;
            event(
                &tx,
                &job.run,
                "effect.unfinished",
                json!({"step_id":job.step_id,"intent_id":job.intent_id,"state":"needs_reconciliation","reason":"commit_error"}),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn finish_prepared_external(
        &mut self,
        job: &PreparedExternal,
        observation: ExternalObservation,
    ) -> Result<bool> {
        let ExternalObservation {
            result,
            cost,
            connection_failure,
            mut diagnostic,
        } = observation;
        if let Ok(raw) = &result
            && raw.output["resultType"] == "input_required"
            && job.binding.get("mcp").is_some()
        {
            let tx = self.db.transaction()?;
            let owned: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM steps WHERE run_id=?1 AND id=?2 AND active_attempt=?3 AND fence=?4 AND state='running')",
                params![job.run, job.step_id, job.intent_id, job.fence],
                |row| row.get(0),
            )?;
            if !owned {
                return Err(RuntimeError::State(
                    "stale external attempt cannot commit".into(),
                ));
            }
            let prior: (String, Option<String>) = tx.query_row(
                "SELECT state,error FROM runs WHERE id=?1",
                [&job.run],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            crate::interactions::save_wait(
                &tx,
                (&job.run, &job.step_id, &job.intent_id),
                &job.input,
                &job.binding,
                &raw.output,
                job.continuation.as_ref().map_or(1, |(_, round)| *round),
                cost.as_ref(),
            )?;
            restore_higher_run_state(&tx, &job.run, prior, "blocked")?;
            tx.commit()?;
            return Ok(false);
        }

        let observed_success = result.is_ok();
        let result = result.and_then(|mut raw| {
            let mut generated = BTreeMap::new();
            if !raw.artifacts.is_empty() {
                let mut references = BTreeMap::new();
                for artifact in raw.artifacts {
                    let reference = self
                        .artifacts
                        .put_with_quota(
                            &artifact.bytes,
                            &artifact.media_type,
                            job.artifact_limit,
                            self.artifact_quota().map_err(|_| true)?,
                        )
                        .map_err(|_| true)?;
                    generated.insert(reference.artifact_id.clone(), reference.clone());
                    references.insert(artifact.name, reference);
                }
                raw.output =
                    worker::substitute_artifacts(raw.output, &references).map_err(|_| true)?;
            }
            #[cfg(feature = "jev")]
            let output = if job.step.operation.as_deref() == Some("jev.evaluate") {
                let request = apr_provider_jev::prepare(job.input.clone()).map_err(|_| true)?;
                serde_json::to_value(
                    apr_provider_jev::observe(&request, raw.output).map_err(|_| true)?,
                )
                .map_err(|_| true)?
            } else {
                raw.output
            };
            #[cfg(not(feature = "jev"))]
            let output = raw.output;
            bounded(&output).map_err(|_| true)?;
            if let Some(schema) = &job.step.result_schema {
                validate_instance(schema, &output).map_err(|_| true)?;
            }
            let references = self
                .checked_refs(&output, job.result_artifact_limit)
                .map_err(|_| true)?;
            let assignment: Option<String> = self
                .db
                .query_row(
                    "SELECT assignment_id FROM runs WHERE id=?1",
                    [&job.run],
                    |r| r.get(0),
                )
                .map_err(|_| true)?;
            if let Some(assignment) = assignment {
                for reference in &references {
                    if generated.get(&reference.artifact_id) != Some(reference) {
                        self.authorize_artifacts(
                            &assignment,
                            &serde_json::to_value(reference).map_err(|_| true)?,
                        )
                        .map_err(|_| true)?;
                    }
                }
            }
            Ok((output, references))
        });
        if observed_success && result.is_err() {
            diagnostic = Some(json!({"stage":"result_validation","code":"result_contract_failed"}));
        }
        fault_point(&format!("effect_observed:{}", job.step_id));
        let tx = self.db.transaction()?;
        let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM steps WHERE run_id=?1 AND id=?2 AND active_attempt=?3 AND fence=?4 AND state='running')",params![job.run,job.step_id,job.intent_id,job.fence],|r|r.get(0))?;
        if !owned {
            return Err(RuntimeError::State(
                "stale external attempt cannot commit".into(),
            ));
        }
        let succeeded = result.is_ok();
        match result {
            Ok((output, references)) => {
                link_artifacts(&tx, &job.run, &job.step_id, &references)?;
                tx.execute("UPDATE steps SET state='succeeded',output=?3,active_attempt=NULL WHERE run_id=?1 AND id=?2",params![job.run,job.step_id,serde_json::to_string(&output)?])?;
                tx.execute(
                    "UPDATE attempts SET state='succeeded',finished_at=?2 WHERE id=?1",
                    params![job.intent_id, now_ms()],
                )?;
                tx.execute(
                    "UPDATE effect_intents SET status='succeeded' WHERE id=?1",
                    [&job.intent_id],
                )?;
                event(
                    &tx,
                    &job.run,
                    "effect.succeeded",
                    json!({"step_id":job.step_id,"intent_id":job.intent_id,"output_digest":content_digest(&output)?}),
                )?;
            }
            Err(uncertain) => {
                let state = if uncertain {
                    "needs_reconciliation"
                } else {
                    "blocked"
                };
                tx.execute(
                    "UPDATE steps SET state=?3,active_attempt=NULL WHERE run_id=?1 AND id=?2",
                    params![job.run, job.step_id, state],
                )?;
                tx.execute(
                    "UPDATE attempts SET state=?2,finished_at=?3,error=?4 WHERE id=?1",
                    params![
                        job.intent_id,
                        state,
                        now_ms(),
                        diagnostic
                            .as_ref()
                            .map(Value::to_string)
                            .unwrap_or_else(|| state.to_owned())
                    ],
                )?;
                tx.execute(
                    "UPDATE effect_intents SET status=?2 WHERE id=?1",
                    params![
                        job.intent_id,
                        if uncertain { "uncertain" } else { "not_sent" }
                    ],
                )?;
                set_external_run_state(
                    &tx,
                    &job.run,
                    state,
                    if uncertain {
                        "external outcome requires reconciliation"
                    } else {
                        connection_failure.unwrap_or("external request was not sent")
                    },
                )?;
                event(
                    &tx,
                    &job.run,
                    "effect.unfinished",
                    json!({"step_id":job.step_id,"intent_id":job.intent_id,"state":state,"diagnostic":diagnostic}),
                )?;
            }
        }
        // Accounting commits with the effect outcome, including a response whose
        // workflow output failed validation. Never redispatch to recover metrics.
        let billing_status: String = tx.query_row(
            "SELECT status FROM effect_intents WHERE id=?1",
            [&job.intent_id],
            |r| r.get(0),
        )?;
        let observed_value = cost.as_ref().map(serde_json::to_value).transpose()?;
        crate::billing::settle(
            &tx,
            &job.intent_id,
            &billing_status,
            observed_value.as_ref(),
            "broker_observed",
        )?;
        if let Some(observation) = cost {
            event(
                &tx,
                &job.run,
                "usage.observed",
                json!({
                    "intent_id": job.intent_id,
                    "provenance": "broker_observed",
                    "observation": observation
                }),
            )?;
        }
        tx.commit()?;
        fault_point(&format!("effect_committed:{}", job.step_id));
        Ok(succeeded)
    }

    pub(crate) fn recover_effect(&mut self, run: &str, step_id: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE effect_intents SET status='uncertain' WHERE run_id=?1 AND step_id=?2 AND status='dispatched'",params![run,step_id])?;
        tx.execute("UPDATE attempts SET state='needs_reconciliation',finished_at=?3,error='executor interrupted after dispatch intent' WHERE run_id=?1 AND step_id=?2 AND state='running'",params![run,step_id,now_ms()])?;
        tx.execute("UPDATE steps SET state='needs_reconciliation',active_attempt=NULL,fence=fence+1 WHERE run_id=?1 AND id=?2",params![run,step_id])?;
        tx.execute("UPDATE runs SET state='needs_reconciliation',error='external outcome requires reconciliation' WHERE id=?1",[run])?;
        event(&tx, run, "effect.interrupted", json!({"step_id":step_id}))?;
        tx.commit()?;
        Ok(())
    }
}

fn dispatch_prepared(job: &PreparedExternal) -> ExternalObservation {
    // A queued job may be cancelled after its durable reservation but before
    // this worker thread begins. In that case no native process, MCP server or
    // HTTP transport is allowed to start.
    if job.active.flag.load(Ordering::Acquire) {
        return ExternalObservation {
            result: Err(false),
            cost: None,
            connection_failure: None,
            diagnostic: Some(json!({"stage":"dispatch","code":"cancelled_before_dispatch"})),
        };
    }
    match &job.dispatch {
        ExternalDispatch::Python {
            pin,
            source,
            artifacts,
        } => {
            let inputs: Vec<_> = artifacts
                .iter()
                .map(|artifact| worker::WorkerInputArtifact {
                    reference: &artifact.reference,
                    bytes: &artifact.bytes,
                })
                .collect();
            let executed = worker::execute(
                WorkerRequest {
                    run_id: Some(&job.run),
                    interpreter: pin,
                    code: source,
                    input: &job.input,
                    config: &job.config,
                    artifacts: &inputs,
                    limits: WorkerLimits {
                        timeout: Duration::from_millis(job.timeout_ms),
                        max_artifact_bytes: job.artifact_limit,
                        ..WorkerLimits::default()
                    },
                },
                &job.active.flag,
            );
            let diagnostic = executed.as_ref().err().map(worker::WorkerError::diagnostic);
            let result = executed
                .map(|result| RawExternalOutput {
                    output: result.output,
                    artifacts: result.artifacts,
                })
                // Native code can produce effects before any reported worker error.
                .map_err(|_| true);
            ExternalObservation {
                result,
                cost: None,
                connection_failure: None,
                diagnostic,
            }
        }
        ExternalDispatch::Connection(connection) => {
            let vault = match FileVault::new(&job.root) {
                Ok(vault) => vault,
                Err(_) => {
                    return ExternalObservation {
                        result: Err(false),
                        cost: None,
                        connection_failure: Some(
                            "Protected account credentials are unavailable; unlock or reconnect the account in CookBook",
                        ),
                        diagnostic: Some(
                            json!({"stage":"credentials","code":"credential_unavailable"}),
                        ),
                    };
                }
            };
            let dispatched = if let Some((next, _)) = &job.continuation {
                apr_connectors::mcp::dispatch_continuation(
                    connection,
                    &job.input,
                    &vault,
                    Some(&job.active.flag),
                    Some(next),
                )
            } else {
                HttpBroker::new(vault).dispatch_cancellable(
                    connection,
                    &job.input,
                    Some(&job.active.flag),
                )
            };
            match dispatched {
                Ok(result) => ExternalObservation {
                    cost: connection
                        .metering
                        .as_ref()
                        .map(|metering| metering.observe_exchange(&job.input, &result.output)),
                    result: Ok(RawExternalOutput {
                        output: result.output,
                        artifacts: Vec::new(),
                    }),
                    connection_failure: None,
                    diagnostic: None,
                },
                Err(error) => ExternalObservation {
                    result: Err(error.may_have_executed()),
                    cost: None,
                    diagnostic: Some(json!({"stage":"connector","code": match error {
                        apr_connectors::DispatchError::Uncertain => "dispatch_uncertain",
                        apr_connectors::DispatchError::InvalidInput => "invalid_request",
                        apr_connectors::DispatchError::InvalidConnection => "invalid_connection",
                        apr_connectors::DispatchError::CredentialUnavailable => "credential_unavailable",
                        apr_connectors::DispatchError::OAuthReconnectRequired => "oauth_reconnect_required",
                        apr_connectors::DispatchError::OAuthTokenUnavailable => "oauth_token_unavailable",
                        apr_connectors::DispatchError::McpHttpStatus(_) => "mcp_http_status",
                        _ => "oauth_setup_failed",
                    }})),
                    // Fixed messages only: never persist provider bodies or credentials.
                    connection_failure: match error {
                        apr_connectors::DispatchError::OAuthReconnectRequired => Some(
                            "OAuth authorization was rejected or expired; reconnect the account in CookBook",
                        ),
                        apr_connectors::DispatchError::OAuthTokenUnavailable => Some(
                            "OAuth provider is unavailable; retry later or inspect the account in CookBook",
                        ),
                        apr_connectors::DispatchError::CredentialUnavailable => Some(
                            "Protected account credentials are unavailable; unlock or reconnect the account in CookBook",
                        ),
                        _ => None,
                    },
                },
            }
        }
    }
}

fn uncertain_observation() -> ExternalObservation {
    ExternalObservation {
        result: Err(true),
        cost: None,
        connection_failure: None,
        diagnostic: Some(json!({"stage":"dispatch","code":"executor_interrupted"})),
    }
}

fn run_state_priority(state: &str) -> u8 {
    match state {
        "cancelled" => 7,
        "failed" | "succeeded" => 6,
        "needs_reconciliation" => 5,
        "paused" => 4,
        "blocked" => 2,
        _ => 1,
    }
}

fn set_external_run_state(
    tx: &Transaction<'_>,
    run: &str,
    desired: &str,
    error: &str,
) -> Result<()> {
    let current: String = tx.query_row("SELECT state FROM runs WHERE id=?1", [run], |row| {
        row.get(0)
    })?;
    if run_state_priority(desired) >= run_state_priority(&current) {
        tx.execute(
            "UPDATE runs SET state=?2,error=?3 WHERE id=?1",
            params![run, desired, error],
        )?;
    }
    Ok(())
}

fn restore_higher_run_state(
    tx: &Transaction<'_>,
    run: &str,
    prior: (String, Option<String>),
    applied: &str,
) -> Result<()> {
    if run_state_priority(&prior.0) > run_state_priority(applied) {
        tx.execute(
            "UPDATE runs SET state=?2,error=?3 WHERE id=?1",
            params![run, prior.0, prior.1],
        )?;
    }
    Ok(())
}

pub(crate) fn freeze_execution_bindings(
    tx: &Transaction<'_>,
    run: &str,
    manifest: &ProcessManifest,
    assignment: Option<&str>,
) -> Result<()> {
    for step in &manifest.steps {
        let Some((kind, name)) = binding_for(step) else {
            continue;
        };
        let changed=tx.execute("INSERT OR IGNORE INTO run_bindings(run_id,kind,name,revision,payload) SELECT ?1,kind,name,revision,payload FROM execution_grants WHERE assignment_id=?2 AND kind=?3 AND name=?4 AND enabled=1",params![run,assignment.ok_or(RuntimeError::Forbidden)?,kind,name])?;
        if changed == 0 {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_bindings WHERE run_id=?1 AND kind=?2 AND name=?3)",
                params![run, kind, name],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(RuntimeError::Forbidden);
            }
        }
    }
    let mut extra = Vec::new();
    for step in &manifest.steps {
        let Some(("connection", name)) = binding_for(step) else {
            continue;
        };
        let payload: String = tx.query_row(
            "SELECT payload FROM run_bindings WHERE run_id=?1 AND kind='connection' AND name=?2",
            params![run, name],
            |r| r.get(0),
        )?;
        let cfg: Connection = serde_json::from_str(&payload)?;
        if let Some(worker) = cfg.mcp.as_ref().and_then(|m| m.sampling_worker.as_ref()) {
            if !manifest
                .capabilities
                .iter()
                .any(|c| c["kind"] == "connection" && c["name"] == worker.connection)
            {
                return Err(RuntimeError::Forbidden);
            }
            extra.push(worker.connection.clone());
        }
    }
    for name in extra {
        let payload:String=tx.query_row("SELECT payload FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=?2 AND enabled=1",params![assignment.ok_or(RuntimeError::Forbidden)?,name],|r|r.get(0))?;
        let cfg: Connection = serde_json::from_str(&payload)?;
        if cfg.mcp.is_some() {
            return Err(RuntimeError::Forbidden);
        }
        tx.execute("INSERT OR IGNORE INTO run_bindings(run_id,kind,name,revision,payload) SELECT ?1,kind,name,revision,payload FROM execution_grants WHERE assignment_id=?2 AND kind='connection' AND name=?3 AND enabled=1",params![run,assignment,name])?;
    }
    Ok(())
}
pub(crate) fn binding_for(step: &Step) -> Option<(&str, &str)> {
    match step.kind.as_str() {
        "script" => Some(("python", "python")),
        "connector" => step.connection.as_deref().map(|name| ("connection", name)),
        _ => None,
    }
}
fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(RuntimeError::InvalidInput(
            "binding name must contain 1–80 ASCII letters, digits, hyphens, underscores or dots"
                .into(),
        ));
    }
    Ok(())
}

/// Read-only fast path while the single writer is supervising external code.
/// It latches the exact authorized cancellation target and signals every active
/// descendant. The ordinary queued control still performs the durable state
/// transition and returns the normal authorization result.
pub(crate) fn signal_cancel_if_authorized(
    root: &Path,
    token: &str,
    args: &Value,
    interrupt: &ExecutionInterrupt,
) -> Option<CancelLatch> {
    use std::os::unix::fs::MetadataExt;
    let object = args.as_object()?;
    if object.len() != 2 || args["action"] != "cancel" {
        return None;
    }
    let run = args["run_id"].as_str()?;
    let checked = (|| -> Result<()> {
        let db = rusqlite::Connection::open_with_flags(
            root.join("state.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        db.busy_timeout(Duration::from_millis(100))?;
        let row:Option<(String,String,String)>=db.query_row("SELECT p.root,p.device,p.inode FROM attachments t JOIN assignments a ON a.id=t.assignment_id JOIN projects p ON p.id=a.project_id JOIN runs r ON r.assignment_id=a.id WHERE r.id=?1 AND t.token_hash=?2 AND t.revoked_at IS NULL AND t.expires_at>?3 AND a.state='active'",params![run,format!("{:x}",Sha256::digest(token.as_bytes())),now_ms()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (path, device, inode) = row.ok_or(RuntimeError::Unauthorized)?;
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.dev().to_string() != device
            || metadata.ino().to_string() != inode
        {
            return Err(RuntimeError::Unauthorized);
        }
        Ok(())
    })();
    if checked.is_ok() {
        Some(interrupt.latch(run))
    } else {
        None
    }
}

#[cfg(test)]
mod interrupt_tests {
    use super::*;

    fn lineage(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn family_latch_signals_existing_attempts_and_future_descendants() {
        let interrupt = ExecutionInterrupt::default();
        let existing = interrupt.begin_attempt(
            "attempt-existing",
            lineage(&["child", "cancel-target", "root"]),
        );
        assert!(!existing.flag.load(Ordering::Acquire));

        let receipt = interrupt.latch("cancel-target");
        assert!(existing.flag.load(Ordering::Acquire));

        // This models a child committed after the read-only authorization
        // snapshot. Its complete lineage still intersects the persistent latch.
        let future = interrupt.begin_attempt(
            "attempt-future-child",
            lineage(&["grandchild", "child", "cancel-target", "root"]),
        );
        assert!(future.flag.load(Ordering::Acquire));

        interrupt.release_latch(&receipt);
        let after_durable_cancel = interrupt.begin_attempt(
            "attempt-after-release",
            lineage(&["other-child", "cancel-target", "root"]),
        );
        assert!(!after_durable_cancel.flag.load(Ordering::Acquire));
    }

    #[test]
    fn rejected_enqueue_releases_only_its_generation() {
        let interrupt = ExecutionInterrupt::default();
        let queued = interrupt.latch("cancel-target");
        let rejected = interrupt.latch("cancel-target");

        interrupt.release_latch(&rejected);
        assert!(interrupt.pending_for(&lineage(&["child", "cancel-target"])));

        interrupt.release_latch(&queued);
        assert!(!interrupt.pending_for(&lineage(&["child", "cancel-target"])));
    }

    #[test]
    fn a_cancelled_wave_yield_keeps_work_pending_after_request_rejection() {
        let directory = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::open(directory.path()).unwrap();
        let process = apr_core::admit_json(&serde_json::to_vec(&json!({
            "schema_version":"0.1","name":"interrupt.pending","description":"Synthetic pending work",
            "input_schema":{"type":"object"},"output_schema":{"type":"object"},
            "config":{"schema":{"type":"object","additionalProperties":false},"defaults":{},"run_override_keys":[]},
            "capabilities":[{"kind":"builtin"}],
            "limits":{"max_step_ms":1000,"max_artifact_bytes":4096,"max_provider_calls":0,"max_agent_requests":0},
            "steps":[{"id":"copy","kind":"builtin","operation":"json.select","effect":"pure",
                "needs":[],"retry":{"max_attempts":1},
                "inputs":{"value":{"literal":"complete"},"pointer":{"literal":""}}}],
            "output":{"answer":{"step":"copy","pointer":"/value"}}
        })).unwrap()).unwrap();
        let run = runtime
            .start_inner(&process, json!({}), "pending", None, false)
            .unwrap();
        let receipt = runtime.interrupt.latch(&run.run_id);
        let mut cursor = None;
        assert!(runtime.drive_pending_batch(&mut cursor).unwrap());
        runtime.interrupt.release_latch(&receipt);
        assert!(runtime.drive_pending_batch(&mut cursor).unwrap());
        assert_eq!(
            runtime.inspect(&run.run_id).unwrap().output,
            Some(json!({"answer":"complete"}))
        );
        assert!(!runtime.drive_pending_batch(&mut cursor).unwrap());
    }

    #[test]
    fn shutdown_latch_rejects_attempts_admitted_after_signal() {
        let interrupt = ExecutionInterrupt::default();
        interrupt.signal(None);

        let late = interrupt.begin_attempt("late-attempt", lineage(&["child", "root"]));
        assert!(late.flag.load(Ordering::Acquire));
        assert!(interrupt.pending_for(&lineage(&["child", "root"])));
    }
}
