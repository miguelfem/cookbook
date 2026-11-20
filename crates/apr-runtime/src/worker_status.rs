//! Read-only worker counts. Scoped views never expose other assignments.
use std::collections::BTreeMap;

use rusqlite::params;
use serde_json::{Value, json};

use crate::{Result, Runtime, store::StoreLock};

impl Runtime {
    /// Trusted owner view across the local service; not a scoped mutation.
    pub fn worker_status(&self) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.worker_status_inner(None)
    }

    pub(crate) fn worker_status_inner(&self, assignment: Option<&str>) -> Result<Value> {
        let mut policy = self.worker_policy_inner()?;
        if let Some(assignment) = assignment {
            let names: std::collections::BTreeSet<String> = self.db
                .prepare("SELECT name FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND enabled=1")?
                .query_map([assignment], |row| row.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            policy.per_connection.retain(|name, _| names.contains(name));
        }
        let mut running = 0_u64;
        let mut running_local = 0_u64;
        let mut per_connection_running: BTreeMap<String, u64> = BTreeMap::new();
        let mut query = self.db.prepare("SELECT e.id,e.kind,e.binding_name FROM effect_intents e JOIN attempts a ON a.id=e.id JOIN steps s ON s.run_id=e.run_id AND s.id=e.step_id JOIN runs r ON r.id=e.run_id WHERE e.status='dispatched' AND a.state='running' AND s.active_attempt=e.id AND (?1 IS NULL OR r.assignment_id=?1)")?;
        for row in query.query_map([assignment], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            let (attempt, kind, name) = row?;
            if !crate::execution::claim_is_live(&self.root, &attempt)? {
                continue;
            }
            running += 1;
            if kind == "connection" {
                *per_connection_running.entry(name).or_default() += 1;
            } else {
                running_local += 1;
            }
        }
        // Dependency and item expansion can still change this count. It is a
        // saved queue estimate, never a claim that every step can launch now.
        let queued: u64 = self.db.query_row(
            "SELECT COUNT(*) FROM steps s JOIN runs r ON r.id=s.run_id WHERE s.state='queued' AND r.state IN ('queued','running','waiting_for_child') AND (?1 IS NULL OR r.assignment_id=?1) AND NOT EXISTS(WITH RECURSIVE ancestors(id,parent_run_id,state) AS (SELECT a.id,a.parent_run_id,a.state FROM runs a WHERE a.id=r.parent_run_id UNION ALL SELECT a.id,a.parent_run_id,a.state FROM runs a JOIN ancestors p ON a.id=p.parent_run_id) SELECT 1 FROM ancestors WHERE state IN ('paused','blocked','failed','cancelled','needs_reconciliation'))",
            params![assignment], |row| row.get(0),
        )?;
        Ok(
            json!({"view":"workers","policy":policy,"owner_controlled":true,
            "scope":if assignment.is_some(){"assignment"}else{"device"},
            "running":running,"running_local":running_local,"queued":queued,
            "queued_is_estimate":true,"per_connection_running":per_connection_running}),
        )
    }
}
