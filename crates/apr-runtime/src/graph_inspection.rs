//! Safe, lazy projections of immutable workflow definitions and run families.
use apr_core::{AdmittedProcess, routing::DecisionPolicy};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::{Result, Runtime, RuntimeError, engine::bounded_detail};

const INSTANCE_PREVIEW: usize = 20;
const PAGE_BYTES: usize = 44 * 1024;

fn page_limit(limit: Option<usize>, after: Option<&str>, default: usize) -> Result<usize> {
    let limit = limit.unwrap_or(default);
    if limit == 0
        || limit > 100
        || after.is_some_and(|v| v.len() > 160 || v.chars().any(char::is_control))
    {
        return Err(RuntimeError::InvalidInput(
            "graph/tree limit must be 1–100 and cursor must be a bounded identity".into(),
        ));
    }
    Ok(limit)
}

impl Runtime {
    pub(crate) fn graph_view(
        &self,
        process: &AdmittedProcess,
        run_id: Option<&str>,
        author: bool,
        limit: Option<usize>,
        after: Option<String>,
    ) -> Result<Value> {
        let limit = page_limit(limit, after.as_deref(), 50)?;
        let mut definitions: Vec<_> = process.manifest.steps.iter().collect();
        definitions.sort_by(|a, b| a.id.cmp(&b.id));
        if after
            .as_ref()
            .is_some_and(|id| !definitions.iter().any(|step| &step.id == id))
        {
            return Err(RuntimeError::InvalidInput(
                "graph cursor is not a step in this process".into(),
            ));
        }
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut next_cursor = None;
        let mut truncated = false;
        let mut bytes = 0;
        for step in definitions
            .into_iter()
            .filter(|step| after.as_ref().is_none_or(|id| step.id > *id))
        {
            if nodes.len() == limit {
                truncated = true;
                break;
            }
            let mut instances = Vec::new();
            let mut instance_count = 0u64;
            let mut state: Option<String> = None;
            let mut selected_route: Option<String> = None;
            if let Some(run) = run_id {
                let status: Option<(String,Option<String>)> = self.db.query_row("SELECT state,CASE WHEN ?3 THEN json_extract(output,'$.route') ELSE NULL END FROM steps WHERE run_id=?1 AND id=?2",params![run,step.id,step.decision.is_some()],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
                if let Some((saved_state, route)) = status {
                    state = Some(saved_state);
                    // Only admit a declared route, never arbitrary step output.
                    selected_route = route.filter(|route| {
                        step.decision
                            .as_ref()
                            .is_some_and(|policy| policy.routes().contains(route))
                    });
                }
                instance_count = self.db.query_row("SELECT COUNT(*) FROM steps WHERE run_id=?1 AND definition_id=?2 AND (?3=0 OR parent_step_id IS NOT NULL)",params![run,step.id,step.foreach.is_some()],|row|row.get(0))?;
                let mut query = self.db.prepare("SELECT s.id,s.state,c.child_run_id FROM steps s LEFT JOIN child_runs c ON c.parent_run_id=s.run_id AND c.parent_step_id=s.id WHERE s.run_id=?1 AND s.definition_id=?2 AND (?3=0 OR s.parent_step_id IS NOT NULL) ORDER BY s.position,s.id LIMIT ?4")?;
                for row in query.query_map(
                    params![run, step.id, step.foreach.is_some(), INSTANCE_PREVIEW],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )? {
                    let (id, state, child) = row?;
                    instances
                        .push(json!({"step_instance_id":id,"state":state,"child_run_id":child}));
                }
            }
            let decision = step.decision.as_ref().map(|policy| match policy {
                DecisionPolicy::Rules {routes,default,..} => json!({"source":"rules","routes":routes,"default":default}),
                DecisionPolicy::Choice {routes,min_confidence,no_match,needs_clarification,..} => json!({"source":"choice","routes":routes,"min_confidence":min_confidence,"no_match":no_match,"needs_clarification":needs_clarification}),
            });
            let node = json!({"id":step.id,"kind":step.kind,"effect":step.effect,"needs":step.needs,"when":step.when,"foreach":step.foreach.is_some(),"state":state,"selected_route":selected_route,"decision":decision,"join":step.join.as_ref().map(|join|json!({"decision":join.decision})),"call":step.call.as_ref().map(|call|json!({"process_ref":call.process_ref})),"instances":instances,"instance_count":instance_count,"instances_truncated":instance_count>INSTANCE_PREVIEW as u64,"editable_jev":author && crate::tuning::editable_jev_step(&process.manifest,step)});
            let incoming: Vec<_> = step.needs.iter().map(|from|json!({"from":from,"to":step.id,"route":step.when.as_ref().filter(|when| &when.decision==from).map(|when|&when.route)})).collect();
            let size = serde_json::to_vec(&node)?.len() + serde_json::to_vec(&incoming)?.len();
            if bytes + size > PAGE_BYTES {
                if nodes.is_empty() {
                    return Err(RuntimeError::LimitExceeded(
                        "one graph node exceeds inspection page budget".into(),
                    ));
                }
                truncated = true;
                break;
            }
            bytes += size;
            next_cursor = Some(step.id.clone());
            nodes.push(node);
            edges.extend(incoming);
        }
        let context = match run_id {
            Some(run) => self.db.query_row(
                "SELECT COALESCE(root_run_id,id),parent_run_id,state FROM runs WHERE id=?1",
                [run],
                |row| {
                    Ok((
                        Some(row.get::<_, String>(0)?),
                        row.get::<_, Option<String>>(1)?,
                        Some(row.get::<_, String>(2)?),
                    ))
                },
            )?,
            None => (None, None, None),
        };
        let result = json!({"view":"graph","process_ref":process.version,"process_name":process.manifest.name,"run_id":run_id,"root_run_id":context.0,"parent_run_id":context.1,"state":context.2,"nodes":nodes,"edges":edges,"next_cursor":if truncated {next_cursor} else {None},"truncated":truncated});
        bounded_detail(&result)?;
        Ok(result)
    }

    /// Constant-size summary of an entire admitted family. No per-child payloads
    /// or foreign assignments are exposed; pagination cannot hide a handoff.
    pub(crate) fn run_tree_status(&self, assignment: &str, run_id: &str) -> Result<Value> {
        let root: String = self.db.query_row(
            "SELECT COALESCE(root_run_id,id) FROM runs WHERE id=?1 AND assignment_id=?2",
            params![run_id, assignment],
            |row| row.get(0),
        )?;
        let mut query = self.db.prepare("SELECT r.state,COUNT(*),SUM(CASE WHEN EXISTS(SELECT 1 FROM handoffs h WHERE h.run_id=r.id AND h.state='pending' AND h.token_hash<>'') THEN 1 ELSE 0 END) FROM runs r WHERE r.assignment_id=?1 AND COALESCE(r.root_run_id,r.id)=?2 GROUP BY r.state")?;
        let mut states = std::collections::BTreeMap::new();
        let mut total = 0_u64;
        let mut needs_attention = false;
        for row in query.query_map(params![assignment, root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })? {
            let (state, count, handoffs) = row?;
            total += count;
            needs_attention |= handoffs > 0
                || !matches!(
                    state.as_str(),
                    "queued" | "running" | "waiting_for_child" | "succeeded"
                );
            states.insert(state, count);
        }
        let complete = (1..=u64::from(apr_core::routing::MAX_CHILD_RUNS) + 1).contains(&total);
        Ok(
            json!({"view":"tree_status","root_run_id":root,"total_runs":total,
            "state_counts":states,"complete":complete,
            "runnable_without_caller":complete && !needs_attention}),
        )
    }

    pub(crate) fn run_tree_view(
        &self,
        assignment: &str,
        run_id: &str,
        limit: Option<usize>,
        after: Option<String>,
    ) -> Result<Value> {
        let limit = page_limit(limit, after.as_deref(), 25)?;
        let root: String = self.db.query_row(
            "SELECT COALESCE(root_run_id,id) FROM runs WHERE id=?1 AND assignment_id=?2",
            params![run_id, assignment],
            |row| row.get(0),
        )?;
        if let Some(cursor) = &after {
            let belongs: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND COALESCE(root_run_id,id)=?2 AND assignment_id=?3)",params![cursor,root,assignment],|row|row.get(0))?;
            if !belongs {
                return Err(RuntimeError::InvalidInput(
                    "tree cursor is not in this run family".into(),
                ));
            }
        }
        let mut query = self.db.prepare("SELECT r.id,r.version,r.parent_run_id,r.parent_step_id,r.depth,r.state, (r.state IN ('waiting_for_agent','queued','running') AND EXISTS(SELECT 1 FROM handoffs h WHERE h.run_id=r.id AND h.state='pending' AND h.token_hash<>'') AND NOT EXISTS(WITH RECURSIVE ancestors(id,parent_run_id,state) AS (SELECT a.id,a.parent_run_id,a.state FROM runs a WHERE a.id=r.parent_run_id UNION ALL SELECT a.id,a.parent_run_id,a.state FROM runs a JOIN ancestors previous ON a.id=previous.parent_run_id) SELECT 1 FROM ancestors WHERE state IN ('succeeded','paused','blocked','failed','cancelled','needs_reconciliation'))), (SELECT json_extract(v.manifest,'$.name') FROM process_versions v WHERE v.version=r.version) FROM runs r WHERE r.assignment_id=?1 AND COALESCE(r.root_run_id,r.id)=?2 AND (?3 IS NULL OR r.id>?3) ORDER BY r.id LIMIT ?4")?;
        let rows = query.query_map(params![assignment,root,after,limit+1],|row| Ok(json!({"run_id":row.get::<_,String>(0)?,"process_ref":row.get::<_,String>(1)?,"parent_run_id":row.get::<_,Option<String>>(2)?,"parent_step_id":row.get::<_,Option<String>>(3)?,"depth":row.get::<_,u32>(4)?,"state":row.get::<_,String>(5)?,"actionable_waiting":row.get::<_,bool>(6)?,"process_name":row.get::<_,String>(7)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let truncated = rows.len() > limit;
        let runs: Vec<_> = rows.into_iter().take(limit).collect();
        let next_cursor = if truncated {
            runs.last().map(|run| run["run_id"].clone())
        } else {
            None
        };
        let result = json!({"view":"tree","root_run_id":root,"runs":runs,"next_cursor":next_cursor,"truncated":truncated});
        bounded_detail(&result)?;
        Ok(result)
    }
}
