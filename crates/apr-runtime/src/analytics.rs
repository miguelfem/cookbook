//! Read-only local analytics. Saved attempts are the accounting grain; child
//! costs are not copied to parents and reconciliation replaces, never adds to,
//! an attempt's last observation. No provider bodies or credentials are read.

use crate::{Result, Runtime, RuntimeError};
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeMap;

impl Runtime {
    pub(crate) fn analytics_view(
        &self,
        assignment: &str,
        run_id: Option<&str>,
        limit: Option<usize>,
        after: Option<&str>,
    ) -> Result<Value> {
        let limit = limit.unwrap_or(10);
        if !(1..=20).contains(&limit)
            || after.is_some_and(|s| s.len() > 160 || s.chars().any(char::is_control))
        {
            return Err(RuntimeError::InvalidInput("invalid analytics page".into()));
        }
        // Run-tree mode deliberately selects the root, even when called on a
        // child, so every displayed subtotal uses the same explicit scope.
        let root: Option<String> = run_id
            .map(|id| {
                self.db.query_row(
                    "SELECT COALESCE(root_run_id,id) FROM runs WHERE id=?1 AND assignment_id=?2",
                    params![id, assignment],
                    |r| r.get(0),
                )
            })
            .transpose()?;
        let predicate = "r.assignment_id=?1 AND (?2 IS NULL OR COALESCE(r.root_run_id,r.id)=?2)";
        let mut states = BTreeMap::<String, u64>::new();
        let mut query = self.db.prepare(&format!(
            "SELECT r.state,COUNT(*) FROM runs r WHERE {predicate} AND r.parent_run_id IS NULL GROUP BY r.state"
        ))?;
        for row in query.query_map(params![assignment, root], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })? {
            let (state, count) = row?;
            states.insert(state, count);
        }
        let child_runs: u64 = self.db.query_row(
            &format!(
                "SELECT COUNT(*) FROM runs r WHERE {predicate} AND r.parent_run_id IS NOT NULL"
            ),
            params![assignment, root],
            |r| r.get(0),
        )?;
        let (attempts, retries, finished_attempts, duration_ms): (u64,u64,u64,Option<f64>) = self.db.query_row(
            &format!("SELECT COUNT(*),COALESCE(SUM(a.number>1 AND NOT EXISTS(SELECT 1 FROM mcp_interactions i WHERE i.continued_by=a.id)),0),COUNT(a.finished_at),AVG(CASE WHEN a.finished_at IS NOT NULL THEN MAX(0,a.finished_at-a.started_at) END) FROM attempts a JOIN runs r ON r.id=a.run_id WHERE {predicate}"),
            params![assignment, root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        )?;
        let external_attempts: u64 = self.db.query_row(&format!(
            "SELECT COUNT(*) FROM effect_intents e JOIN runs r ON r.id=e.run_id WHERE {predicate} AND e.kind='connection'"
        ), params![assignment, root], |r| r.get(0))?;
        let pending_handoffs: u64 = self.db.query_row(&format!(
            "SELECT COUNT(*) FROM handoffs h JOIN runs r ON r.id=h.run_id WHERE {predicate} AND h.state='pending'"
        ), params![assignment, root], |r| r.get(0))?;
        let sql = format!("SELECT e.id,e.run_id,e.step_id,e.binding_name,e.binding_revision,e.status,e.created_at,
            (SELECT v.detail FROM events v WHERE v.run_id=e.run_id AND v.kind='usage.observed'
             AND json_extract(v.detail,'$.intent_id')=e.id ORDER BY v.sequence DESC LIMIT 1)
            FROM effect_intents e JOIN runs r ON r.id=e.run_id WHERE {predicate}
            AND e.kind='connection' AND (?3 IS NULL OR e.id>?3) ORDER BY e.id LIMIT ?4");
        let mut query = self.db.prepare(&sql)?;
        let rows = query.query_map(params![assignment, root, after, limit + 1], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        let mut items = Vec::new();
        let mut page_bytes = 0;
        let mut truncated = false;
        let mut next_cursor = None;
        let mut unknown = 0_u64;
        let mut not_sent = 0_u64;
        let mut totals = BTreeMap::<String, (u128, u128, u64, u64)>::new();
        for row in rows {
            let (id, run, step, binding, revision, status, created, saved) = row?;
            let usage: Option<Value> = saved.map(|s| serde_json::from_str(&s)).transpose()?;
            let item = json!({"intent_id":id,"run_id":run,"step_instance_id":step,
                "connection":binding,"binding_revision":revision,"status":status,
                "created_at_ms":created,"usage":usage});
            let size = serde_json::to_vec(&item)?.len();
            if items.len() == limit || page_bytes + size > 40 * 1024 {
                truncated = true;
                break;
            }
            page_bytes += size;
            if status == "not_sent" {
                not_sent += 1;
            }
            let observation = usage.as_ref().and_then(|v| v.get("observation"));
            let estimate = observation
                .and_then(|o| o["estimated_nanos"].as_str())
                .and_then(|n| n.parse::<u128>().ok());
            let reported = observation
                .and_then(|o| o["reported_nanos"].as_str())
                .and_then(|n| n.parse::<u128>().ok());
            if status != "not_sent"
                && reported.is_none()
                && observation.is_none_or(|o| o["estimate_status"] != "estimated")
            {
                unknown += 1;
            }
            if let Some(currency) = observation.and_then(|o| o["currency"].as_str()) {
                let entry = totals.entry(currency.to_string()).or_default();
                // These are distinct views of the same call, NOT additive costs.
                if let Some(n) = estimate {
                    entry.0 = entry.0.checked_add(n).ok_or_else(|| {
                        RuntimeError::LimitExceeded("cost subtotal overflow".into())
                    })?;
                    entry.2 += 1;
                }
                if let Some(n) = reported {
                    entry.1 = entry.1.checked_add(n).ok_or_else(|| {
                        RuntimeError::LimitExceeded("cost subtotal overflow".into())
                    })?;
                    entry.3 += 1;
                }
            }
            next_cursor = Some(id);
            items.push(item);
        }
        let currencies: Vec<Value> = totals.into_iter().map(|(currency,(estimated,reported,est_count,rep_count))| {
            json!({"currency":currency,"estimated_nanos":(est_count>0).then(||estimated.to_string()),
                "reported_nanos":(rep_count>0).then(||reported.to_string()),
                "estimated_calls":est_count,"reported_calls":rep_count})
        }).collect();
        let completed = states.get("succeeded").copied().unwrap_or(0);
        let terminal = completed
            + states.get("failed").copied().unwrap_or(0)
            + states.get("cancelled").copied().unwrap_or(0);
        Ok(json!({
            "view":"analytics", "scope":if root.is_some(){"run_tree"}else{"assignment"},
            "root_run_id":root, "retained_data_only":true, "telemetry":"local_only",
            "summary":{"root_run_states":states,"child_runs":child_runs,"attempts":attempts,
                "retries":retries,"finished_attempts":finished_attempts,
                "mean_finished_attempt_ms":duration_ms,"pending_handoffs":pending_handoffs,
                "external_attempts":external_attempts,
                "terminal_root_success_rate":(terminal>0).then(||completed as f64/terminal as f64)},
            "cost_page":{"currencies":currencies,"unknown_or_partial_calls":unknown,"not_sent_calls":not_sent,
                "scope":"this_page_only","coverage":"configured_meters_only",
                "reported_and_estimated_are_not_additive":true},
            "calls":items,"truncated":truncated,"next_cursor":if truncated {next_cursor}else{None},
            "limitations":["Caller-client/subscription costs are not visible.",
                "Provider-reported amounts are not invoice-reconciled.",
                "Use billing view for whole-period totals and budget view for broker reservations; provider invoices remain separate evidence."]
        }))
    }
}
