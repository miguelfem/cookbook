use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use rusqlite::{Connection, params};
use serde_json::Value;

use crate::{Result, RuntimeError};

pub(crate) const SCHEMA_VERSION: u32 = 18;

pub(crate) const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS process_versions (
    version TEXT PRIMARY KEY,
    manifest TEXT NOT NULL,
    canonical_format TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    request_key TEXT NOT NULL UNIQUE,
    request_digest TEXT NOT NULL,
    version TEXT NOT NULL REFERENCES process_versions(version),
    input TEXT NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0,
    output TEXT,
    error TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS steps (
    run_id TEXT NOT NULL REFERENCES runs(id),
    id TEXT NOT NULL,
    position INTEGER NOT NULL,
    state TEXT NOT NULL,
    output TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    active_attempt TEXT,
    fence INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, id)
);
CREATE TABLE IF NOT EXISTS attempts (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    number INTEGER NOT NULL,
    fence INTEGER NOT NULL,
    state TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    error TEXT,
    FOREIGN KEY (run_id, step_id) REFERENCES steps(run_id, id),
    UNIQUE (run_id, step_id, number)
);
CREATE TABLE IF NOT EXISTS handoffs (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    step_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    input TEXT NOT NULL,
    result_schema TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    state TEXT NOT NULL,
    FOREIGN KEY (run_id, step_id) REFERENCES steps(run_id, id),
    UNIQUE (run_id, step_id, generation)
);
CREATE TABLE IF NOT EXISTS receipts (
    run_id TEXT NOT NULL REFERENCES runs(id),
    submission_key TEXT NOT NULL,
    semantic_digest TEXT NOT NULL,
    receipt TEXT NOT NULL,
    PRIMARY KEY (run_id, submission_key)
);
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    reference TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS artifact_links (
    run_id TEXT NOT NULL REFERENCES runs(id),
    step_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    PRIMARY KEY (run_id, step_id, artifact_id)
);
CREATE TABLE IF NOT EXISTS events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES runs(id),
    kind TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS events_by_run ON events(run_id, sequence);
CREATE INDEX IF NOT EXISTS handoffs_by_run ON handoffs(run_id, state);
PRAGMA user_version = 1;
"#;

const MIGRATION_V2: &str = r#"
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    root TEXT NOT NULL,
    device TEXT NOT NULL,
    inode TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(root, device, inode)
);
CREATE TABLE assignments (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    name TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('active', 'closed')),
    created_at INTEGER NOT NULL
);
CREATE TABLE attachments (
    id TEXT PRIMARY KEY,
    assignment_id TEXT NOT NULL REFERENCES assignments(id),
    token_hash TEXT NOT NULL UNIQUE,
    mode TEXT NOT NULL CHECK(mode IN ('author', 'run')),
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE TABLE catalog (
    assignment_id TEXT NOT NULL REFERENCES assignments(id),
    version TEXT NOT NULL REFERENCES process_versions(version),
    name TEXT NOT NULL,
    registered_at INTEGER NOT NULL,
    PRIMARY KEY(assignment_id, version)
);
CREATE TABLE artifact_imports (
    assignment_id TEXT NOT NULL REFERENCES assignments(id),
    artifact_id TEXT NOT NULL REFERENCES artifacts(id),
    PRIMARY KEY(assignment_id, artifact_id)
);
ALTER TABLE runs ADD COLUMN assignment_id TEXT REFERENCES assignments(id);
CREATE INDEX runs_by_assignment ON runs(assignment_id, state);
CREATE INDEX catalog_by_assignment ON catalog(assignment_id, name);
CREATE INDEX attachments_by_assignment ON attachments(assignment_id);
"#;

const MIGRATION_V3: &str = r#"
CREATE TABLE process_config (
    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('project','assignment')),
    scope_id TEXT NOT NULL,
    version TEXT NOT NULL REFERENCES process_versions(version),
    values_json TEXT NOT NULL,
    revision INTEGER NOT NULL,
    PRIMARY KEY(scope_kind,scope_id,version)
);
CREATE TABLE promotion_grants (
    source_assignment_id TEXT NOT NULL REFERENCES assignments(id),
    target_assignment_id TEXT NOT NULL REFERENCES assignments(id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(source_assignment_id,target_assignment_id)
);
CREATE TABLE catalog_promotions (
    source_assignment_id TEXT NOT NULL REFERENCES assignments(id),
    target_assignment_id TEXT NOT NULL REFERENCES assignments(id),
    version TEXT NOT NULL REFERENCES process_versions(version),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(source_assignment_id,target_assignment_id,version)
);
ALTER TABLE runs ADD COLUMN config TEXT NOT NULL DEFAULT '{}';
ALTER TABLE runs ADD COLUMN config_overrides TEXT NOT NULL DEFAULT '{}';
ALTER TABLE runs ADD COLUMN config_project_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN config_assignment_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN fixture INTEGER NOT NULL DEFAULT 0;
"#;

const MIGRATION_V4: &str = r#"
ALTER TABLE steps ADD COLUMN definition_id TEXT;
ALTER TABLE steps ADD COLUMN parent_step_id TEXT;
ALTER TABLE steps ADD COLUMN item_key TEXT;
ALTER TABLE steps ADD COLUMN item_index INTEGER;
ALTER TABLE steps ADD COLUMN item_value TEXT;
ALTER TABLE steps ADD COLUMN retry_limit INTEGER NOT NULL DEFAULT 1;
UPDATE steps SET definition_id=id,position=position*1001,retry_limit=COALESCE((
    SELECT json_extract(j.value,'$.retry.max_attempts') FROM runs r
    JOIN process_versions v ON v.version=r.version,json_each(v.manifest,'$.steps') j
    WHERE r.id=steps.run_id AND json_extract(j.value,'$.id')=steps.id),1);
CREATE INDEX steps_by_parent ON steps(run_id,parent_step_id,item_index);
CREATE TABLE step_expansions (
    run_id TEXT NOT NULL,
    definition_id TEXT NOT NULL,
    items_json TEXT NOT NULL,
    items_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(run_id,definition_id),
    FOREIGN KEY(run_id,definition_id) REFERENCES steps(run_id,id)
);
"#;

const MIGRATION_V7: &str = r#"
ALTER TABLE runs ADD COLUMN root_run_id TEXT;
ALTER TABLE runs ADD COLUMN parent_run_id TEXT REFERENCES runs(id);
ALTER TABLE runs ADD COLUMN parent_step_id TEXT;
ALTER TABLE runs ADD COLUMN depth INTEGER NOT NULL DEFAULT 0;
UPDATE runs SET root_run_id=id;
CREATE INDEX runs_by_root ON runs(root_run_id,id);
CREATE TABLE run_call_snapshots (
    root_run_id TEXT NOT NULL REFERENCES runs(id),
    version TEXT NOT NULL REFERENCES process_versions(version),
    config TEXT NOT NULL,
    config_project_revision INTEGER NOT NULL,
    config_assignment_revision INTEGER NOT NULL,
    PRIMARY KEY(root_run_id,version)
);
CREATE TABLE child_runs (
    parent_run_id TEXT NOT NULL REFERENCES runs(id),
    parent_step_id TEXT NOT NULL,
    child_run_id TEXT NOT NULL UNIQUE REFERENCES runs(id),
    PRIMARY KEY(parent_run_id,parent_step_id),
    FOREIGN KEY(parent_run_id,parent_step_id) REFERENCES steps(run_id,id)
);
"#;

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    if path.exists() && std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(RuntimeError::Integrity(
            "state directory cannot be a symbolic link".into(),
        ));
    }
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(crate) fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(RuntimeError::Integrity(
            "runtime control path must be a regular file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

pub(crate) struct StoreLock(File);

impl StoreLock {
    pub fn acquire(root: &Path) -> Result<Self> {
        let file = private_file(&root.join("runtime.lock"))?;
        file.lock_exclusive()?;
        Ok(Self(file))
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

pub(crate) fn connect(root: &Path) -> Result<Connection> {
    let path = root.join("state.sqlite3");
    // Never open and close an out-of-band descriptor for an existing SQLite
    // inode: POSIX close would drop this process's SQLite advisory locks and
    // allow another process to replace WAL/SHM beneath live connections.
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.nlink() != 1
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(RuntimeError::Integrity(
                    "database must be an owned regular file with one link".into(),
                ));
            }
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?
                .sync_all()?;
        }
        Err(error) => return Err(error.into()),
    }
    let mut db = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(RuntimeError::State(
            "database was created by a newer APR version".into(),
        ));
    }
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "foreign_keys", true)?;
    db.pragma_update(None, "synchronous", "FULL")?;
    if version == 0 {
        let tx = db.transaction()?;
        tx.execute_batch(MIGRATION)?;
        tx.commit()?;
    }
    if version <= 1 {
        let tx = db.transaction()?;
        tx.execute_batch(MIGRATION_V2)?;
        // Re-encode legacy request keys into the null (owner CLI) scope. First
        // move every old key to a temporary namespace so crafted legacy strings
        // cannot collide with another row's new JSON-encoded key mid-migration.
        let legacy: Vec<(String, String)> = {
            let mut query = tx.prepare("SELECT id,request_key FROM runs")?;
            query
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let prefix = loop {
            let candidate = format!("apr-migration-{}:", uuid::Uuid::new_v4().simple());
            let used: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE substr(request_key,1,?1)=?2)",
                params![candidate.len(), candidate],
                |row| row.get(0),
            )?;
            if !used {
                break candidate;
            }
        };
        for (index, (id, _)) in legacy.iter().enumerate() {
            tx.execute(
                "UPDATE runs SET request_key=?2 WHERE id=?1",
                params![id, format!("{prefix}{index}")],
            )?;
        }
        for (id, key) in legacy {
            let encoded = serde_json::to_string(&serde_json::json!([null, key]))?;
            tx.execute(
                "UPDATE runs SET request_key=?2 WHERE id=?1",
                params![id, encoded],
            )?;
        }
        tx.pragma_update(None, "user_version", 2)?;
        fault_point("migration_v2_before_commit");
        tx.commit()?;
    }
    if version <= 2 {
        let tx = db.transaction()?;
        tx.execute_batch(MIGRATION_V3)?;
        let legacy: Vec<(String, String, String, String)> = {
            let mut query = tx.prepare("SELECT r.id,r.version,r.input,v.manifest FROM runs r JOIN process_versions v ON v.version=r.version")?;
            query
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<std::result::Result<_, _>>()?
        };
        for (id, version, input, manifest) in legacy {
            let manifest: Value = serde_json::from_str(&manifest)?;
            let config = manifest["config"]["defaults"].clone();
            let input: Value = serde_json::from_str(&input)?;
            let digest = apr_core::content_digest(
                &serde_json::json!({"version":version,"input":input,"config":config,"config_overrides":{},"fixture":false}),
            )?;
            tx.execute(
                "UPDATE runs SET config=?2,request_digest=?3 WHERE id=?1",
                params![id, serde_json::to_string(&config)?, digest],
            )?;
        }
        tx.pragma_update(None, "user_version", 3)?;
        fault_point("migration_v3_before_commit");
        tx.commit()?;
    }
    if version <= 3 {
        let tx = db.transaction()?;
        tx.execute_batch(MIGRATION_V4)?;
        tx.pragma_update(None, "user_version", 4)?;
        fault_point("migration_v4_before_commit");
        tx.commit()?;
    }
    if version <= 4 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::execution::MIGRATION_V5)?;
        tx.pragma_update(None, "user_version", 5)?;
        fault_point("migration_v5_before_commit");
        tx.commit()?;
    }
    if version <= 5 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::maintenance::MIGRATION_V6)?;
        tx.pragma_update(None, "user_version", 6)?;
        fault_point("migration_v6_before_commit");
        tx.commit()?;
    }
    if version <= 6 {
        let tx = db.transaction()?;
        tx.execute_batch(MIGRATION_V7)?;
        tx.pragma_update(None, "user_version", 7)?;
        fault_point("migration_v7_before_commit");
        tx.commit()?;
    }
    if version <= 7 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::maintenance::MIGRATION_V8)?;
        tx.pragma_update(None, "user_version", 8)?;
        fault_point("migration_v8_before_commit");
        tx.commit()?;
    }
    if version <= 8 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::schedule::MIGRATION_V9)?;
        tx.pragma_update(None, "user_version", 9)?;
        fault_point("migration_v9_before_commit");
        tx.commit()?;
    }
    if version <= 9 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::capacity::MIGRATION_V10)?;
        tx.pragma_update(None, "user_version", 10)?;
        fault_point("migration_v10_before_commit");
        tx.commit()?;
    }
    if version <= 10 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::setup::MIGRATION_V11)?;
        tx.pragma_update(None, "user_version", 11)?;
        fault_point("migration_v11_before_commit");
        tx.commit()?;
    }
    if version <= 11 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::billing::MIGRATION_V12)?;
        tx.pragma_update(None, "user_version", 12)?;
        tx.commit()?;
    }
    if version <= 12 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::interactions::MIGRATION_V13)?;
        tx.execute_batch(crate::sampling::TABLES)?;
        tx.pragma_update(None, "user_version", 13)?;
        tx.commit()?;
    }
    if version <= 13 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::operations::MIGRATION_V14)?;
        tx.pragma_update(None, "user_version", 14)?;
        tx.commit()?;
    }
    if version <= 14 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::connection_library::MIGRATION_V15)?;
        tx.pragma_update(None, "user_version", 15)?;
        tx.commit()?;
    }
    if version <= 15 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::improvement::MIGRATION_V16)?;
        tx.pragma_update(None, "user_version", 16)?;
        tx.commit()?;
    }
    if version <= 16 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::connection_library::MIGRATION_V17)?;
        crate::connection_library::backfill_accounts(&tx)?;
        tx.pragma_update(None, "user_version", 17)?;
        tx.commit()?;
    }
    if version <= 17 {
        let tx = db.transaction()?;
        tx.execute_batch(crate::worker_policy::MIGRATION_V18)?;
        tx.pragma_update(None, "user_version", 18)?;
        tx.commit()?;
    }
    Ok(db)
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub(crate) fn event(db: &Connection, run_id: &str, kind: &str, detail: Value) -> Result<()> {
    db.execute(
        "INSERT INTO events(run_id,kind,detail,created_at) VALUES (?1,?2,?3,?4)",
        params![run_id, kind, serde_json::to_string(&detail)?, now_ms()],
    )?;
    db.execute("UPDATE runs SET revision=revision+1 WHERE id=?1", [run_id])?;
    Ok(())
}

pub(crate) fn fault_point(point: &str) {
    #[cfg(feature = "fault-injection")]
    if std::env::var("APR_TEST_CRASH_AT").ok().as_deref() == Some(point) {
        // Compiled only into the explicit fault-injection test build.
        #[cfg(unix)]
        unsafe {
            if libc::kill(libc::getpid(), libc::SIGKILL) == 0 {
                // Delivery can race the next instruction on a multithreaded
                // process. Wait for SIGKILL rather than racing it with abort.
                loop {
                    libc::pause();
                }
            }
        }
        std::process::abort();
    }
    let _ = point;
}
