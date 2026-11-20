//! Owner-controlled backups and retention. Backups contain private workflow
//! data, but omit bearer files and protected credentials. Restored authority is
//! disabled; waiting callers must explicitly renew their claims.

pub(crate) const MIGRATION_V6: &str = r#"
CREATE TABLE run_pins (run_id TEXT PRIMARY KEY REFERENCES runs(id));
CREATE TABLE artifact_pins (artifact_id TEXT PRIMARY KEY REFERENCES artifacts(id));
"#;

pub(crate) const MIGRATION_V8: &str = r#"
CREATE TABLE artifact_quota (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 max_bytes INTEGER CHECK(max_bytes IS NULL OR max_bytes>=0)
);
INSERT INTO artifact_quota(singleton,max_bytes) VALUES (1,NULL);
"#;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use apr_core::ArtifactRef;
use rusqlite::{
    Connection, OptionalExtension,
    backup::{Backup, StepResult},
    params,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::artifacts::{ArtifactStore, StoredObject};
use crate::store::{self, StoreLock, now_ms};
use crate::{Result, Runtime, RuntimeError};

const BACKUP_FORMAT: &str = "apr-backup-v1";
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupManifest {
    format: String,
    created_at_ms: i64,
    database_schema: u32,
    database_bytes: u64,
    database_sha256: String,
    artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub format: &'static str,
    pub destination: PathBuf,
    pub database_sha256: String,
    pub artifacts: usize,
    pub artifact_bytes: u64,
    pub runs: u64,
    pub handoffs_requiring_renewal: u64,
    pub runs_requiring_reconciliation: u64,
}

/// A preview is a concrete deletion plan. Applying it rechecks the same policy
/// against current references, pins and object identities under the store lock.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetentionPlan {
    pub format: String,
    pub terminal_before_ms: i64,
    pub orphan_before_ms: i64,
    pub run_ids: Vec<String>,
    pub artifact_ids: Vec<String>,
    pub orphan_object_names: Vec<String>,
    pub bytes_reclaimable: u64,
    pub retained_runs: u64,
    pub retained_artifacts: u64,
    state_identity: String,
    objects: Vec<StoredObject>,
}

#[derive(Debug, Serialize)]
pub struct RetentionReport {
    pub removed_runs: usize,
    pub removed_artifacts: usize,
    pub removed_objects: usize,
    pub removed_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct StorageUsage {
    pub schema_version: u32,
    pub runs: u64,
    pub registered_artifacts: u64,
    pub registered_artifact_bytes: u64,
    pub stored_object_bytes: u64,
    pub orphan_object_bytes: u64,
    pub database_bytes: u64,
    pub quota_mode: &'static str,
    pub quota_bytes: Option<u64>,
    pub quota_used_bytes: u64,
}

impl Runtime {
    /// Limit committed immutable object bytes, including unlinked orphans.
    /// Database, staging and native filesystem use are outside this quota.
    pub fn set_artifact_quota(&mut self, bytes: Option<u64>) -> Result<Value> {
        if bytes.is_some_and(|bytes| bytes > i64::MAX as u64) {
            return Err(invalid(
                "artifact quota must fit a nonnegative signed 64-bit integer",
            ));
        }
        let _lock = StoreLock::acquire(&self.root)?;
        self.db.execute(
            "UPDATE artifact_quota SET max_bytes=?1 WHERE singleton=1",
            [bytes],
        )?;
        Ok(
            serde_json::json!({"protocol_version":"0.1","quota_mode":if bytes.is_some() {"managed_artifacts"} else {"unlimited"},"quota_bytes":bytes}),
        )
    }

    /// Internal callers hold StoreLock so changing the quota cannot race a
    /// managed publication. Never cache this value in a Runtime handle.
    pub(crate) fn artifact_quota(&self) -> Result<Option<u64>> {
        Ok(self.db.query_row(
            "SELECT max_bytes FROM artifact_quota WHERE singleton=1",
            [],
            |row| row.get(0),
        )?)
    }

    /// Owner acknowledgement of a new project location. This changes the local
    /// directory binding only; frozen sources and workflow data remain intact.
    pub fn relocate_project(&mut self, project_id: &str, new_root: &Path) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let root = new_root.canonicalize()?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&root)?;
        let metadata = directory.metadata()?;
        // SAFETY: geteuid has no preconditions and returns the current owner.
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(invalid(
                "project relocation requires a directory owned by the current user",
            ));
        }
        let root_text = root
            .to_str()
            .ok_or_else(|| invalid("project path must be UTF-8"))?;
        let device = metadata.dev().to_string();
        let inode = metadata.ino().to_string();
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [project_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(RuntimeError::NotFound("project is not registered".into()));
        }
        let conflict: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id<>?1 AND root=?2 AND device=?3 AND inode=?4)", params![project_id,root_text,device,inode], |row|row.get(0))?;
        if conflict {
            return Err(invalid(
                "directory is already attached to a different project",
            ));
        }
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE projects SET root=?2,device=?3,inode=?4 WHERE id=?1",
            params![project_id, root_text, device, inode],
        )?;
        let revoked = tx.execute("UPDATE attachments SET revoked_at=?2 WHERE revoked_at IS NULL AND assignment_id IN (SELECT id FROM assignments WHERE project_id=?1)",params![project_id,now_ms()])?;
        tx.execute("INSERT OR IGNORE INTO connection_library_blocks(assignment_id,library_id) SELECT u.assignment_id,u.library_id FROM connection_library_uses u JOIN assignments a ON a.id=u.assignment_id WHERE a.project_id=?1",[project_id])?;
        let disabled = tx.execute("UPDATE execution_grants SET enabled=0,revision=revision+1 WHERE enabled<>0 AND assignment_id IN (SELECT id FROM assignments WHERE project_id=?1)",[project_id])?;
        tx.execute("UPDATE runs SET state='blocked',error='project location changed; explicit continuation required' WHERE state IN ('queued','running') AND assignment_id IN (SELECT id FROM assignments WHERE project_id=?1)",[project_id])?;
        tx.commit()?;
        Ok(
            serde_json::json!({"protocol_version":"0.1","project_id":project_id,"root":root,"relocated":true,"revoked_attachments":revoked,"disabled_execution_grants":disabled}),
        )
    }

    /// Payload-free owner diagnostics. This quota applies to immutable objects,
    /// not total state-directory or trusted native process filesystem usage.
    pub fn storage_usage(&self) -> Result<StorageUsage> {
        let _lock = StoreLock::acquire(&self.root)?;
        let references = artifact_references(&self.db)?;
        let registered: BTreeSet<_> = references
            .iter()
            .map(|reference| reference.sha256.as_str())
            .collect();
        let inventory = self.artifacts.inventory()?;
        let stored_object_bytes = inventory.iter().try_fold(0_u64, |sum, object| {
            sum.checked_add(object.bytes)
                .ok_or_else(|| integrity("storage size overflow"))
        })?;
        let orphan_object_bytes = inventory
            .iter()
            .filter(|object| !registered.contains(object.name.as_str()))
            .try_fold(0_u64, |sum, object| {
                sum.checked_add(object.bytes)
                    .ok_or_else(|| integrity("storage size overflow"))
            })?;
        let registered_artifact_bytes = references.iter().try_fold(0_u64, |sum, reference| {
            sum.checked_add(reference.bytes)
                .ok_or_else(|| integrity("storage size overflow"))
        })?;
        let quota_used_bytes = inventory
            .iter()
            .filter(|object| object.name.len() == 64)
            .try_fold(0_u64, |sum, object| {
                sum.checked_add(object.bytes)
                    .ok_or_else(|| integrity("storage size overflow"))
            })?;
        let quota_bytes = self.artifact_quota()?;
        let mut database_bytes = 0_u64;
        for name in ["state.sqlite3", "state.sqlite3-wal", "state.sqlite3-shm"] {
            // Never independently open/close a live SQLite file in this process:
            // POSIX close can release another connection's advisory locks.
            match fs::symlink_metadata(self.root.join(name)) {
                Ok(metadata) => {
                    // SAFETY: geteuid has no preconditions.
                    if !metadata.is_file()
                        || metadata.file_type().is_symlink()
                        || metadata.nlink() != 1
                        || metadata.uid() != unsafe { libc::geteuid() }
                    {
                        return Err(integrity(
                            "SQLite storage entry has unsafe ownership or file type",
                        ));
                    }
                    database_bytes = database_bytes
                        .checked_add(metadata.len())
                        .ok_or_else(|| integrity("storage size overflow"))?;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && name != "state.sqlite3" => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(StorageUsage {
            schema_version: store::SCHEMA_VERSION,
            runs: self
                .db
                .query_row("SELECT count(*) FROM runs", [], |row| row.get(0))?,
            registered_artifacts: references.len() as u64,
            registered_artifact_bytes,
            stored_object_bytes,
            orphan_object_bytes,
            database_bytes,
            quota_mode: if quota_bytes.is_some() {
                "managed_artifacts"
            } else {
                "unlimited"
            },
            quota_bytes,
            quota_used_bytes,
        })
    }

    pub fn pin_run(&mut self, run_id: &str, pinned: bool) -> Result<()> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_run(run_id)?;
        if pinned {
            self.db.execute(
                "INSERT OR IGNORE INTO run_pins(run_id) VALUES (?1)",
                [run_id],
            )?;
        } else {
            self.db
                .execute("DELETE FROM run_pins WHERE run_id=?1", [run_id])?;
        }
        Ok(())
    }

    pub fn pin_artifact(&mut self, artifact_id: &str, pinned: bool) -> Result<()> {
        let _lock = StoreLock::acquire(&self.root)?;
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=?1)",
            [artifact_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(RuntimeError::NotFound("artifact is not registered".into()));
        }
        if pinned {
            self.db.execute(
                "INSERT OR IGNORE INTO artifact_pins(artifact_id) VALUES (?1)",
                [artifact_id],
            )?;
        } else {
            self.db.execute(
                "DELETE FROM artifact_pins WHERE artifact_id=?1",
                [artifact_id],
            )?;
        }
        Ok(())
    }

    pub fn retention_preview(
        &self,
        terminal_before_ms: i64,
        orphan_before_ms: i64,
    ) -> Result<RetentionPlan> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.retention_plan(terminal_before_ms, orphan_before_ms)
    }

    pub fn retention_apply(&mut self, plan: &RetentionPlan) -> Result<RetentionReport> {
        let _lock = StoreLock::acquire(&self.root)?;
        if self.retention_plan(plan.terminal_before_ms, plan.orphan_before_ms)? != *plan {
            return Err(RuntimeError::State(
                "retention preview is stale; create a new preview".into(),
            ));
        }
        let tx = self.db.transaction()?;
        tx.pragma_update(None, "defer_foreign_keys", true)?;
        for run_id in &plan.run_ids {
            tx.execute(
                "DELETE FROM child_runs WHERE parent_run_id=?1 OR child_run_id=?1",
                [run_id],
            )?;
            tx.execute(
                "DELETE FROM run_call_snapshots WHERE root_run_id=?1",
                [run_id],
            )?;
        }
        for run_id in &plan.run_ids {
            for table in [
                "effect_intents",
                "run_bindings",
                "receipts",
                "handoffs",
                "attempts",
                "artifact_links",
                "events",
                "step_expansions",
                "steps",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE run_id=?1"), [run_id])?;
            }
            tx.execute("DELETE FROM runs WHERE id=?1", [run_id])?;
        }
        for artifact_id in &plan.artifact_ids {
            tx.execute("DELETE FROM artifacts WHERE id=?1", [artifact_id])?;
        }
        tx.commit()?;
        // The database stops referencing deleted objects before unlinking them.
        // A crash here leaves collectible orphans, never a live missing object.
        store::fault_point("retention_committed");
        for object in &plan.objects {
            self.artifacts.remove_snapshot(object)?;
        }
        Ok(RetentionReport {
            removed_runs: plan.run_ids.len(),
            removed_artifacts: plan.artifact_ids.len(),
            removed_objects: plan.objects.len(),
            removed_bytes: plan.bytes_reclaimable,
        })
    }

    fn retention_plan(
        &self,
        terminal_before_ms: i64,
        orphan_before_ms: i64,
    ) -> Result<RetentionPlan> {
        let now = now_ms();
        if terminal_before_ms < 0
            || terminal_before_ms > now
            || orphan_before_ms < 0
            || orphan_before_ms > now.saturating_sub(60_000)
        {
            return Err(invalid(
                "retention cutoffs must be past timestamps; orphan grace is at least 60 seconds",
            ));
        }
        let mut statement = self.db.prepare("SELECT r.id FROM runs r WHERE r.state IN ('succeeded','failed','cancelled') AND NOT EXISTS(SELECT 1 FROM run_pins p WHERE p.run_id=r.id) AND NOT EXISTS(SELECT 1 FROM effect_intents e WHERE e.run_id=r.id AND e.status IN ('dispatched','uncertain')) AND NOT EXISTS(SELECT 1 FROM handoffs h WHERE h.run_id=r.id AND h.state='pending') AND COALESCE((SELECT MAX(created_at) FROM events e WHERE e.run_id=r.id),r.created_at) < ?1 ORDER BY r.id")?;
        let mut run_ids = statement
            .query_map([terminal_before_ms], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let eligible: BTreeSet<_> = run_ids.iter().map(String::as_str).collect();
        let trees = self
            .db
            .prepare("SELECT id,COALESCE(root_run_id,id) FROM runs")?
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let preserved_roots: BTreeSet<_> = trees
            .iter()
            .filter(|(run, _)| !eligible.contains(run.as_str()))
            .map(|(_, root)| root.as_str())
            .collect();
        let preserved_runs: BTreeSet<_> = trees
            .iter()
            .filter(|(_, root)| preserved_roots.contains(root.as_str()))
            .map(|(run, _)| run.as_str())
            .collect();
        run_ids.retain(|run| !preserved_runs.contains(run.as_str()));
        let removed_runs: BTreeSet<_> = run_ids.iter().map(String::as_str).collect();
        let mut protected = BTreeSet::new();
        for query in [
            "SELECT artifact_id FROM artifact_imports",
            "SELECT artifact_id FROM artifact_pins",
        ] {
            for id in strings(&self.db, query)? {
                protected.insert(id);
            }
        }
        let mut links = self
            .db
            .prepare("SELECT run_id,artifact_id FROM artifact_links")?;
        for record in links.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (run, id) = record?;
            if !removed_runs.contains(run.as_str()) {
                protected.insert(id);
            }
        }
        // Catalog versions, source snapshots and configuration are retained.
        // Protect literal references even before a process has its first run.
        for query in [
            "SELECT manifest FROM process_versions",
            "SELECT values_json FROM process_config",
            "SELECT config FROM run_call_snapshots",
            "SELECT template_json FROM schedules WHERE state IN ('active','paused')",
        ] {
            for encoded in strings(&self.db, query)? {
                protect_references(&serde_json::from_str(&encoded)?, &mut protected);
            }
        }
        let references = artifact_references(&self.db)?;
        let by_id: BTreeMap<_, _> = references
            .iter()
            .map(|reference| (reference.artifact_id.clone(), reference))
            .collect();
        let mut inspected = BTreeSet::new();
        loop {
            let next = protected
                .iter()
                .find(|id| !inspected.contains(*id))
                .cloned();
            let Some(id) = next else {
                break;
            };
            inspected.insert(id.clone());
            if let Some(reference) = by_id.get(&id)
                && (reference.media_type == "application/json"
                    || reference.media_type.ends_with("+json"))
            {
                if reference.bytes > MAX_MANIFEST_BYTES {
                    // Large opaque JSON may contain dependencies. Retain all
                    // registered objects rather than guess at a partial parse.
                    protected.extend(by_id.keys().cloned());
                    break;
                }
                let bytes = self.artifacts.read(reference, MAX_MANIFEST_BYTES)?;
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    protect_references(&value, &mut protected);
                }
            }
        }
        let inventory = self.artifacts.inventory()?;
        let by_name: BTreeMap<_, _> = inventory
            .iter()
            .map(|object| (object.name.as_str(), object))
            .collect();
        let mut artifact_ids = Vec::new();
        let mut objects = Vec::new();
        let mut registered_names = BTreeSet::new();
        for reference in &references {
            registered_names.insert(reference.sha256.as_str());
            if protected.contains(&reference.artifact_id) {
                continue;
            }
            let Some(object) = by_name.get(reference.sha256.as_str()) else {
                return Err(integrity(
                    "registered artifact is missing during retention preview",
                ));
            };
            if object.modified_ms >= orphan_before_ms {
                continue;
            }
            artifact_ids.push(reference.artifact_id.clone());
            objects.push((*object).clone());
        }
        let mut orphan_object_names = Vec::new();
        for object in inventory {
            if !registered_names.contains(object.name.as_str())
                && object.modified_ms < orphan_before_ms
            {
                orphan_object_names.push(object.name.clone());
                objects.push(object);
            }
        }
        objects.sort_by(|left, right| left.name.cmp(&right.name));
        let bytes_reclaimable = objects.iter().try_fold(0_u64, |sum, object| {
            sum.checked_add(object.bytes)
                .ok_or_else(|| integrity("retention size overflow"))
        })?;
        let metadata = fs::metadata(self.root.join("state.sqlite3"))?;
        let state_identity = apr_core::content_digest(
            &serde_json::json!({"root":self.root,"device":metadata.dev(),"inode":metadata.ino()}),
        )?;
        let count: u64 = self
            .db
            .query_row("SELECT count(*) FROM runs", [], |row| row.get(0))?;
        Ok(RetentionPlan {
            format: "apr-retention-v1".into(),
            terminal_before_ms,
            orphan_before_ms,
            retained_runs: count - run_ids.len() as u64,
            retained_artifacts: (references.len() - artifact_ids.len()) as u64,
            run_ids,
            artifact_ids,
            orphan_object_names,
            bytes_reclaimable,
            state_identity,
            objects,
        })
    }

    /// A complete owner-only snapshot. SQLite and object copies share the runtime
    /// lock so retention cannot unlink a snapshot input during publication.
    pub fn backup_to(&self, destination: &Path) -> Result<BackupReport> {
        let _lock = StoreLock::acquire(&self.root)?;
        let staged = StagedDirectory::new(destination)?;
        if staged.destination.starts_with(&self.root) {
            return Err(invalid("backup destination must be outside runtime state"));
        }
        let database = staged.path().join("state.sqlite3");
        new_private_file(&database)?.sync_all()?;
        let mut copy = Connection::open(&database)?;
        {
            let backup = Backup::new(&self.db, &mut copy)?;
            let started = Instant::now();
            loop {
                match backup.step(256)? {
                    StepResult::Done => break,
                    StepResult::More => {}
                    StepResult::Busy | StepResult::Locked => {
                        if started.elapsed() > Duration::from_secs(30) {
                            return Err(RuntimeError::State(
                                "snapshot database remained busy".into(),
                            ));
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    _ => {
                        return Err(RuntimeError::State(
                            "unsupported SQLite backup status".into(),
                        ));
                    }
                }
            }
        }
        secure_database(&copy)?;
        sanitize_restored_authority(&mut copy)?;
        let references = artifact_references(&copy)?;
        let source = &self.artifacts;
        copy_artifacts(source, staged.path(), &references)?;
        validate_database(&copy)?;
        let mut report = report_for(&copy, &staged.destination, &references)?;
        copy.pragma_update(None, "journal_mode", "DELETE")?;
        copy.close().map_err(|(_, error)| error)?;
        File::open(&database)?.sync_all()?;
        let (database_bytes, database_sha256) = hash_file(&database)?;
        report.database_sha256 = database_sha256.clone();
        let manifest = BackupManifest {
            format: BACKUP_FORMAT.into(),
            created_at_ms: now_ms(),
            database_schema: store::SCHEMA_VERSION,
            database_bytes,
            database_sha256,
            artifacts: references,
        };
        let bytes = serde_json::to_vec(&manifest)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(RuntimeError::LimitExceeded(
                "backup inventory exceeds 16 MiB".into(),
            ));
        }
        let mut file = new_private_file(&staged.path().join("backup.json"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        staged.publish()?;
        Ok(report)
    }

    /// Restores into a new absent state directory. No bearer files are copied,
    /// no credential backend is queried, and no workflow is dispatched here.
    pub fn restore_from(backup: &Path, destination: &Path) -> Result<BackupReport> {
        if !backup.is_absolute() || fs::symlink_metadata(backup)?.file_type().is_symlink() {
            return Err(invalid(
                "backup must be an absolute directory without a symlink root",
            ));
        }
        let backup = backup.canonicalize()?;
        let staged = StagedDirectory::new(destination)?;
        if staged.destination.starts_with(&backup) {
            return Err(invalid("restore destination must be outside the backup"));
        }
        let mut manifest_file = open_regular(&backup.join("backup.json"))?;
        if manifest_file.metadata()?.len() > MAX_MANIFEST_BYTES {
            return Err(RuntimeError::LimitExceeded(
                "backup inventory exceeds 16 MiB".into(),
            ));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut manifest_file)
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(RuntimeError::LimitExceeded(
                "backup inventory exceeds 16 MiB".into(),
            ));
        }
        let manifest: BackupManifest = serde_json::from_slice(&bytes)?;
        if manifest.format != BACKUP_FORMAT
            || !(8..=store::SCHEMA_VERSION).contains(&manifest.database_schema)
        {
            return Err(RuntimeError::State(
                "unsupported backup format or database schema".into(),
            ));
        }
        let database = staged.path().join("state.sqlite3");
        copy_checked_file(
            &backup.join("state.sqlite3"),
            &database,
            manifest.database_bytes,
            &manifest.database_sha256,
        )?;
        let mut db = Connection::open(&database)?;
        secure_database(&db)?;
        validate_database(&db)?;
        let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != manifest.database_schema {
            return Err(integrity("backup schema metadata mismatch"));
        }
        let references = artifact_references(&db)?;
        if references != manifest.artifacts {
            return Err(integrity(
                "backup artifact inventory differs from its database",
            ));
        }
        let source = ArtifactStore::open_existing(&backup)?;
        copy_artifacts(&source, staged.path(), &references)?;
        // Verify the source inventory before migrating a private staged copy.
        // A v8 backup remains portable after the scheduled-jobs schema upgrade.
        if version < store::SCHEMA_VERSION {
            db.close().map_err(|(_, error)| error)?;
            db = store::connect(staged.path())?;
            secure_database(&db)?;
            validate_database(&db)?;
        }
        sanitize_restored_authority(&mut db)?;
        let mut report = report_for(&db, &staged.destination, &references)?;
        db.pragma_update(None, "journal_mode", "DELETE")?;
        db.close().map_err(|(_, error)| error)?;
        File::open(&database)?.sync_all()?;
        report.database_sha256 = hash_file(&database)?.1;
        private_subdirectory(&staged.path().join("handoffs"))?;
        staged.publish()?;
        Ok(report)
    }
}

fn sanitize_restored_authority(db: &mut Connection) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute(
        "UPDATE attachments SET token_hash='restored:'||id,revoked_at=COALESCE(revoked_at,?1)",
        [now_ms()],
    )?;
    tx.execute(
        "UPDATE execution_grants SET enabled=0,revision=revision+1 WHERE enabled<>0",
        [],
    )?;
    tx.execute("DELETE FROM promotion_grants", [])?;
    tx.execute(
        "UPDATE connection_library SET enabled=0,revision=revision+1 WHERE enabled=1",
        [],
    )?;
    tx.execute(
        "UPDATE saved_accounts SET enabled=0,revision=revision+1 WHERE enabled=1",
        [],
    )?;
    tx.execute("UPDATE schedules SET state='paused',last_error='restored schedules require explicit resumption',updated_at=?1 WHERE state IN ('active','paused')", [now_ms()])?;
    tx.execute("UPDATE handoffs SET token_hash='',expires_at=CASE WHEN state='pending' THEN 0 ELSE expires_at END", [])?;
    tx.execute("UPDATE runs SET state='waiting_for_agent',error='caller handoff must be renewed after restore' WHERE state IN ('waiting_for_agent','queued','running') AND EXISTS(SELECT 1 FROM handoffs h WHERE h.run_id=runs.id AND h.state='pending')", [])?;
    tx.execute("UPDATE runs SET state='blocked',error='restored work requires explicit continuation' WHERE state IN ('queued','running')", [])?;
    // An external intent is durable evidence that an effect may have occurred.
    // It always wins over generic interruption recovery or caller renewal.
    tx.execute(
        "UPDATE effect_intents SET status='uncertain' WHERE status='dispatched'",
        [],
    )?;
    tx.execute("UPDATE attempts SET state='needs_reconciliation',finished_at=COALESCE(finished_at,?1),error='restored external outcome requires reconciliation' WHERE id IN (SELECT id FROM effect_intents WHERE status='uncertain')", [now_ms()])?;
    tx.execute("UPDATE steps SET state='needs_reconciliation',active_attempt=NULL,fence=fence+1 WHERE EXISTS(SELECT 1 FROM effect_intents e WHERE e.run_id=steps.run_id AND e.step_id=steps.id AND e.status='uncertain') AND state<>'needs_reconciliation'", [])?;
    tx.execute("UPDATE runs SET state='needs_reconciliation',error='restored external outcome requires reconciliation' WHERE EXISTS(SELECT 1 FROM effect_intents e WHERE e.run_id=runs.id AND e.status='uncertain')", [])?;
    tx.commit()?;
    Ok(())
}

fn secure_database(db: &Connection) -> Result<()> {
    db.pragma_update(None, "trusted_schema", false)?;
    db.pragma_update(None, "foreign_keys", true)?;
    let executable_schema: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type IN ('trigger','view') OR sql LIKE '%CREATE VIRTUAL TABLE%')", [], |row|row.get(0))?;
    if executable_schema {
        return Err(integrity("backup contains unsupported executable schema"));
    }
    Ok(())
}

fn validate_database(db: &Connection) -> Result<()> {
    let check: String = db.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if check != "ok" {
        return Err(integrity("backup database failed integrity verification"));
    }
    let violations: Option<i64> = db
        .query_row(
            "SELECT 1 FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if violations.is_some() {
        return Err(integrity("backup database contains broken references"));
    }
    Ok(())
}

fn artifact_references(db: &Connection) -> Result<Vec<ArtifactRef>> {
    let mut query = db.prepare("SELECT id,reference FROM artifacts ORDER BY id")?;
    let records = query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    records
        .into_iter()
        .map(|(id, value)| {
            let reference: ArtifactRef = serde_json::from_str(&value)?;
            if id != reference.artifact_id || !valid_digest(&reference.sha256) {
                return Err(integrity("invalid stored artifact identity"));
            }
            Ok(reference)
        })
        .collect()
}

fn report_for(
    db: &Connection,
    destination: &Path,
    references: &[ArtifactRef],
) -> Result<BackupReport> {
    let sum = references.iter().try_fold(0_u64, |sum, reference| {
        sum.checked_add(reference.bytes)
            .ok_or_else(|| integrity("artifact byte total overflow"))
    })?;
    Ok(BackupReport {
        format: BACKUP_FORMAT,
        destination: destination.to_path_buf(),
        database_sha256: String::new(),
        artifacts: references.len(),
        artifact_bytes: sum,
        runs: db.query_row("SELECT count(*) FROM runs", [], |row| row.get(0))?,
        handoffs_requiring_renewal: db.query_row(
            "SELECT count(*) FROM handoffs WHERE state='pending'",
            [],
            |row| row.get(0),
        )?,
        runs_requiring_reconciliation: db.query_row(
            "SELECT count(*) FROM runs WHERE state='needs_reconciliation'",
            [],
            |row| row.get(0),
        )?,
    })
}

fn copy_artifacts(
    source: &ArtifactStore,
    destination: &Path,
    references: &[ArtifactRef],
) -> Result<()> {
    let objects = destination.join("objects");
    private_subdirectory(&objects)?;
    let sha256 = objects.join("sha256");
    private_subdirectory(&sha256)?;
    for reference in references {
        if !valid_digest(&reference.sha256) {
            return Err(integrity("invalid backup object name"));
        }
        let mut file = new_private_file(&sha256.join(&reference.sha256))?;
        source.copy_to(reference, &mut file)?;
        file.sync_all()?;
    }
    File::open(&sha256)?.sync_all()?;
    File::open(&objects)?.sync_all()?;
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn new_private_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}

fn open_regular(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(integrity("backup entries must be regular files"));
    }
    Ok(file)
}

fn private_subdirectory(path: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<(u64, String)> {
    let mut source = open_regular(path)?;
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or_else(|| integrity("file size overflow"))?;
        hash.update(&buffer[..read]);
    }
    Ok((count, format!("{:x}", hash.finalize())))
}

fn copy_checked_file(source: &Path, destination: &Path, bytes: u64, digest: &str) -> Result<()> {
    if !valid_digest(digest) {
        return Err(integrity("invalid backup file digest"));
    }
    let mut input = open_regular(source)?;
    if input.metadata()?.len() != bytes {
        return Err(integrity("backup file size mismatch"));
    }
    let mut output = new_private_file(destination)?;
    let mut hasher = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or_else(|| integrity("file size overflow"))?;
        if count > bytes {
            return Err(integrity("backup file grew during copy"));
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    if count != bytes || format!("{:x}", hasher.finalize()) != digest {
        return Err(integrity("backup file checksum mismatch"));
    }
    output.sync_all()?;
    Ok(())
}

struct StagedDirectory {
    temporary: tempfile::TempDir,
    parent: File,
    destination: PathBuf,
}

impl StagedDirectory {
    fn new(destination: &Path) -> Result<Self> {
        if !destination.is_absolute() {
            return Err(invalid("destination must be absolute"));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("destination requires a parent"))?
            .canonicalize()?;
        let name = destination
            .file_name()
            .ok_or_else(|| invalid("destination requires a name"))?;
        let destination = parent.join(name);
        match fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => return Err(invalid("destination must not already exist")),
        }
        let temporary = tempfile::Builder::new()
            .prefix(".apr-snapshot-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&parent)?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent)?;
        Ok(Self {
            temporary,
            parent,
            destination,
        })
    }

    fn path(&self) -> &Path {
        self.temporary.path()
    }

    fn publish(self) -> Result<()> {
        File::open(self.path())?.sync_all()?;
        let source = CString::new(self.path().file_name().unwrap().as_encoded_bytes())
            .map_err(|_| invalid("invalid staging name"))?;
        let destination = CString::new(self.destination.file_name().unwrap().as_encoded_bytes())
            .map_err(|_| invalid("invalid destination name"))?;
        // SAFETY: both basenames and the pinned parent descriptor remain live;
        // exclusive rename refuses any destination created during the snapshot.
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.parent.as_raw_fd(),
                source.as_ptr(),
                self.parent.as_raw_fd(),
                destination.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                self.parent.as_raw_fd(),
                source.as_ptr(),
                self.parent.as_raw_fd(),
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let result = -1;
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // Once published, a late sync failure never rolls back the snapshot.
        self.parent.sync_all()?;
        Ok(())
    }
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::InvalidInput(message.into())
}
fn integrity(message: &str) -> RuntimeError {
    RuntimeError::Integrity(message.into())
}

fn strings(db: &Connection, query: &str) -> Result<Vec<String>> {
    Ok(db
        .prepare(query)?
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn protect_references(value: &Value, protected: &mut BTreeSet<String>) {
    match value {
        Value::String(id) if id.strip_prefix("sha256:").is_some_and(valid_digest) => {
            protected.insert(id.clone());
        }
        Value::Array(items) => {
            for item in items {
                protect_references(item, protected);
            }
        }
        Value::Object(object) => {
            for item in object.values() {
                protect_references(item, protected);
            }
        }
        _ => {}
    }
}
