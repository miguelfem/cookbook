//! Device-owner worker limits. Changes affect future claims; admitted work and
//! active effects retain their existing authority and finish normally.
use std::collections::BTreeMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{
    Result, Runtime, RuntimeError,
    store::{StoreLock, now_ms},
};

pub(crate) const MIGRATION_V18: &str = r#"
CREATE TABLE worker_policy (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    revision INTEGER NOT NULL CHECK(revision >= 0),
    max_workers INTEGER NOT NULL CHECK(max_workers BETWEEN 1 AND 64),
    max_local_workers INTEGER NOT NULL CHECK(max_local_workers BETWEEN 1 AND 64 AND max_local_workers <= max_workers),
    per_connection_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
INSERT INTO worker_policy VALUES (1,0,4,4,'{}',0);
"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPolicy {
    pub revision: u64,
    pub max_workers: u32,
    pub max_local_workers: u32,
    pub per_connection: BTreeMap<String, u32>,
}

impl Default for WorkerPolicy {
    fn default() -> Self {
        Self {
            revision: 0,
            max_workers: 4,
            max_local_workers: 4,
            per_connection: BTreeMap::new(),
        }
    }
}

impl WorkerPolicy {
    pub fn validate(&self) -> Result<()> {
        if !(1..=64).contains(&self.max_workers) {
            return Err(RuntimeError::InvalidInput(
                "simultaneous jobs must be between 1 and 64".into(),
            ));
        }
        if !(1..=self.max_workers).contains(&self.max_local_workers) {
            return Err(RuntimeError::InvalidInput(
                "local job limit must be between 1 and the simultaneous job limit".into(),
            ));
        }
        if self.per_connection.len() > 256 {
            return Err(RuntimeError::InvalidInput(
                "too many per-connection job limits".into(),
            ));
        }
        for (name, cap) in &self.per_connection {
            if name.is_empty()
                || name.chars().count() > 128
                || name.trim() != name
                || name.chars().any(char::is_control)
            {
                return Err(RuntimeError::InvalidInput(
                    "connection limit names must be nonempty, at most 128 characters, and contain no surrounding space or control characters".into(),
                ));
            }
            if !(1..=self.max_workers).contains(cap) {
                return Err(RuntimeError::InvalidInput(format!(
                    "connection job limit for {name} must be between 1 and the simultaneous job limit"
                )));
            }
        }
        Ok(())
    }
}

impl Runtime {
    /// Read the owner policy. Scoped attachment and MCP routes expose status only.
    pub fn worker_policy(&self) -> Result<WorkerPolicy> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.worker_policy_inner()
    }

    /// The caller already owns the coordinator lock while preparing claims.
    pub(crate) fn worker_policy_inner(&self) -> Result<WorkerPolicy> {
        let policy = self.db.query_row(
            "SELECT revision,max_workers,max_local_workers,per_connection_json FROM worker_policy WHERE singleton=1",
            [],
            |row| {
                let json: String = row.get(3)?;
                let per_connection = serde_json::from_str(&json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
                })?;
                Ok(WorkerPolicy {
                    revision: row.get(0)?,
                    max_workers: row.get(1)?,
                    max_local_workers: row.get(2)?,
                    per_connection,
                })
            },
        )?;
        policy.validate()?;
        Ok(policy)
    }

    /// Trusted owner operation. A stale Settings window cannot overwrite a newer edit.
    pub fn set_worker_policy(
        &mut self,
        mut policy: WorkerPolicy,
        expected_revision: u64,
    ) -> Result<WorkerPolicy> {
        policy.validate()?;
        if policy.revision != expected_revision {
            return Err(RuntimeError::InvalidInput(
                "worker policy revision does not match the reviewed version".into(),
            ));
        }
        let _lock = StoreLock::acquire(&self.root)?;
        let tx = self.db.transaction()?;
        let changed = tx.execute(
            "UPDATE worker_policy SET revision=revision+1,max_workers=?1,max_local_workers=?2,per_connection_json=?3,updated_at=?4 WHERE singleton=1 AND revision=?5",
            params![policy.max_workers, policy.max_local_workers, serde_json::to_string(&policy.per_connection)?, now_ms(), expected_revision],
        )?;
        if changed != 1 {
            return Err(RuntimeError::State(
                "worker policy changed; reload Settings before saving".into(),
            ));
        }
        tx.commit()?;
        policy.revision += 1;
        Ok(policy)
    }
}
