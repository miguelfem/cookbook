//! Device-owner capacity policy for new root admissions, separate from manifests.
//!
//! The writer lock serializes policy changes with acceptance. Existing roots keep
//! their admitted manifest limits; attachments cannot mutate this policy.
use apr_core::{
    Limits,
    routing::{MAX_CALL_DEPTH, MAX_CHILD_RUNS, MAX_GRAPH_VERSIONS, MAX_STEP_INSTANCES},
};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{
    Result, Runtime, RuntimeError,
    store::{StoreLock, now_ms},
};

pub(crate) const MIGRATION_V10: &str = r#"
CREATE TABLE capacity_policy (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    max_call_depth INTEGER NOT NULL CHECK(max_call_depth BETWEEN 1 AND 128),
    max_child_runs INTEGER NOT NULL CHECK(max_child_runs BETWEEN 1 AND 4096),
    max_step_instances INTEGER NOT NULL CHECK(max_step_instances BETWEEN 1 AND 100000),
    max_graph_versions INTEGER NOT NULL CHECK(max_graph_versions BETWEEN 1 AND 4096),
    updated_at INTEGER NOT NULL
);
INSERT INTO capacity_policy VALUES (1,32,1024,10000,4096,0);
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityPolicy {
    pub max_call_depth: u32,
    pub max_child_runs: u32,
    pub max_step_instances: u32,
    pub max_graph_versions: u32,
}

impl Default for CapacityPolicy {
    fn default() -> Self {
        Self {
            max_call_depth: 32,
            max_child_runs: 1024,
            max_step_instances: 10_000,
            max_graph_versions: 4096,
        }
    }
}

impl CapacityPolicy {
    pub const HARD_CEILINGS: Self = Self {
        max_call_depth: MAX_CALL_DEPTH,
        max_child_runs: MAX_CHILD_RUNS,
        max_step_instances: MAX_STEP_INSTANCES,
        max_graph_versions: MAX_GRAPH_VERSIONS,
    };

    fn validate(&self) -> Result<()> {
        for (field, requested, ceiling) in [
            ("max_call_depth", self.max_call_depth, MAX_CALL_DEPTH),
            ("max_child_runs", self.max_child_runs, MAX_CHILD_RUNS),
            (
                "max_step_instances",
                self.max_step_instances,
                MAX_STEP_INSTANCES,
            ),
            (
                "max_graph_versions",
                self.max_graph_versions,
                MAX_GRAPH_VERSIONS,
            ),
        ] {
            if requested == 0 || requested > ceiling {
                return Err(RuntimeError::InvalidInput(format!(
                    "capacity {field} must be 1..={ceiling}; requested {requested}"
                )));
            }
        }
        Ok(())
    }
}

impl Runtime {
    /// Read the current device policy. This does not change accepted runs.
    pub fn capacity_policy(&self) -> Result<CapacityPolicy> {
        let policy = self.db.query_row("SELECT max_call_depth,max_child_runs,max_step_instances,max_graph_versions FROM capacity_policy WHERE singleton=1", [], |row| Ok(CapacityPolicy {
            max_call_depth: row.get(0)?, max_child_runs: row.get(1)?, max_step_instances: row.get(2)?, max_graph_versions: row.get(3)?,
        }))?;
        policy.validate()?;
        Ok(policy)
    }

    /// Trusted owner operation; intentionally absent from attachment/MCP tools.
    pub fn set_capacity_policy(&mut self, policy: CapacityPolicy) -> Result<CapacityPolicy> {
        policy.validate()?;
        let _lock = StoreLock::acquire(&self.root)?;
        self.db.execute("UPDATE capacity_policy SET max_call_depth=?1,max_child_runs=?2,max_step_instances=?3,max_graph_versions=?4,updated_at=?5 WHERE singleton=1",params![policy.max_call_depth,policy.max_child_runs,policy.max_step_instances,policy.max_graph_versions,now_ms()])?;
        Ok(policy)
    }

    /// Check new-root requested budgets while the admission caller holds StoreLock.
    pub(crate) fn validate_capacity_limits(&self, limits: &Limits) -> Result<()> {
        let policy = self.capacity_policy()?;
        for (field, requested, allowed) in [
            (
                "max_call_depth",
                limits.max_call_depth,
                policy.max_call_depth,
            ),
            (
                "max_child_runs",
                limits.max_child_runs,
                policy.max_child_runs,
            ),
            (
                "max_step_instances",
                limits.max_step_instances,
                policy.max_step_instances,
            ),
        ] {
            if requested > allowed {
                return Err(RuntimeError::LimitExceeded(format!(
                    "requested {field}={requested} exceeds owner capacity {allowed}; reduce the manifest budget or ask the owner to review `apr capacity show` and `apr capacity set`"
                )));
            }
        }
        Ok(())
    }

    /// Count distinct frozen versions including the root, not expanded instances.
    pub(crate) fn validate_graph_capacity(&self, unique_versions: usize) -> Result<()> {
        let allowed = self.capacity_policy()?.max_graph_versions;
        if unique_versions > allowed as usize {
            return Err(RuntimeError::LimitExceeded(format!(
                "requested graph versions={unique_versions} exceeds owner capacity {allowed}; reduce the graph or ask the owner to review `apr capacity show` and `apr capacity set`"
            )));
        }
        Ok(())
    }
}
