//! Frozen child-process graphs and durable parent/child dispatch identity.

use std::collections::BTreeMap;

use apr_core::{
    AdmittedProcess, ArtifactRef, Binding, Step, admit_json, content_digest,
    routing::{CallTarget, analyze_call_graph},
    validate_instance,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::{
    Result, Runtime, RuntimeError,
    engine::{bounded, link_artifacts, new_id},
    scope::ResolvedConfig,
    store::{event, fault_point, now_ms},
};

pub(crate) struct FrozenChild {
    pub(crate) process: AdmittedProcess,
    pub(crate) config: ResolvedConfig,
}

pub(crate) struct CallSnapshots {
    pub(crate) children: Vec<FrozenChild>,
    pub(crate) references: Vec<ArtifactRef>,
}

pub(crate) struct RetriedParent {
    run_id: String,
    step_id: String,
    child_id: String,
}

impl Runtime {
    pub(crate) fn prepare_child_retry(&self, run_id: &str) -> Result<Vec<RetriedParent>> {
        let cancelled:bool=self.db.query_row("WITH RECURSIVE chain(id,parent_run_id) AS (SELECT id,parent_run_id FROM runs WHERE id=?1 UNION ALL SELECT r.id,r.parent_run_id FROM runs r JOIN chain c ON r.id=c.parent_run_id) SELECT EXISTS(SELECT 1 FROM chain c JOIN events e ON e.run_id=c.id WHERE e.kind='run.cancelled')",[run_id],|row|row.get(0))?;
        if cancelled {
            return Err(RuntimeError::State(
                "cancelled work cannot be retried".into(),
            ));
        }
        let mut current = run_id.to_owned();
        let mut parents = Vec::new();
        loop {
            let parent:Option<(String,String,Option<String>,String,String,bool)>=self.db.query_row("SELECT p.id,p.state,p.error,c.parent_step_id,s.state,EXISTS(SELECT 1 FROM steps other WHERE other.run_id=p.id AND other.id<>s.id AND other.state='failed') FROM runs c JOIN runs p ON p.id=c.parent_run_id JOIN steps s ON s.run_id=p.id AND s.id=c.parent_step_id WHERE c.id=?1",[&current],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
            let Some((id, state, error, step_id, step_state, other_failure)) = parent else {
                break;
            };
            if matches!(state.as_str(), "succeeded" | "cancelled")
                || (state == "failed"
                    && (error.as_deref() != Some("required child did not succeed")
                        || step_state != "failed"
                        || other_failure))
            {
                return Err(RuntimeError::State(
                    "a terminal ancestor cannot be resumed by this child retry".into(),
                ));
            }
            if state == "failed" {
                parents.push(RetriedParent {
                    run_id: id.clone(),
                    step_id,
                    child_id: current,
                });
            }
            current = id;
        }
        Ok(parents)
    }

    pub(crate) fn prepare_call_snapshots(
        &self,
        root: &AdmittedProcess,
        assignment: Option<&str>,
        fixture: bool,
    ) -> Result<CallSnapshots> {
        self.validate_capacity_limits(&root.manifest.limits)?;
        let graph_limit = self.capacity_policy()?.max_graph_versions as usize;
        let mut processes = BTreeMap::from([(root.version.clone(), root.clone())]);
        let mut graph = BTreeMap::new();
        let mut pending = vec![root.version.clone()];
        while let Some(version) = pending.pop() {
            if graph.contains_key(&version) {
                continue;
            }
            let process = &processes[&version];
            if fixture
                && process
                    .manifest
                    .steps
                    .iter()
                    .any(|step| matches!(step.kind.as_str(), "script" | "connector"))
            {
                return Err(RuntimeError::Forbidden);
            }
            let edges: Vec<_> = process
                .manifest
                .steps
                .iter()
                .filter_map(|step| {
                    step.call.as_ref().map(|call| CallTarget {
                        process_ref: call.process_ref.clone(),
                        instances: step.foreach.as_ref().map_or(1, |each| each.max_items),
                    })
                })
                .collect();
            for edge in &edges {
                if !processes.contains_key(&edge.process_ref) {
                    let process = self.catalog_process(
                        assignment.ok_or(RuntimeError::Forbidden)?,
                        &edge.process_ref,
                    )?;
                    processes.insert(edge.process_ref.clone(), process);
                    if processes.len() > graph_limit {
                        return Err(RuntimeError::LimitExceeded(
                            "reachable process versions exceed the owner graph capacity; inspect capacity".into(),
                        ));
                    }
                }
                pending.push(edge.process_ref.clone());
            }
            graph.insert(version, edges);
        }
        let analyzed =
            analyze_call_graph(&root.version, &graph, root.manifest.limits.max_call_depth)?;
        let mut bounds = BTreeMap::new();
        for version in &analyzed.postorder {
            let process = &processes[version];
            let bound = execution_bound(process, &bounds);
            let limits = &process.manifest.limits;
            if bound.depth > limits.max_call_depth
                || bound.children > limits.max_child_runs
                || bound.steps > limits.max_step_instances
            {
                return Err(RuntimeError::LimitExceeded(
                    "reachable execution exceeds a process's declared depth, child or step budget"
                        .into(),
                ));
            }
            bounds.insert(version.clone(), bound);
        }
        let mut references = Vec::new();
        let mut children = Vec::new();
        for (version, process) in processes {
            if let Some(assignment) = assignment {
                self.authorize_literals(assignment, &process.manifest)?;
            }
            let artifact_limit = process
                .manifest
                .limits
                .max_artifact_bytes
                .min(root.manifest.limits.max_artifact_bytes);
            for binding in process
                .manifest
                .steps
                .iter()
                .flat_map(Step::bindings)
                .chain(process.manifest.output.values())
            {
                if let Binding::Literal { literal } = binding {
                    references.extend(self.checked_refs(literal, artifact_limit)?);
                }
            }
            if version == root.version {
                continue;
            }
            self.execution_preflight(&process, assignment)?;
            let config = self.resolve_configuration(&process, assignment, &json!({}))?;
            if let Some(assignment) = assignment {
                self.authorize_artifacts(assignment, &config.value)?;
            }
            references.extend(self.checked_refs(&config.value, artifact_limit)?);
            children.push(FrozenChild { process, config });
        }
        Ok(CallSnapshots {
            children,
            references,
        })
    }

    pub(crate) fn pinned_processes(&self, run_id: &str) -> Result<Vec<AdmittedProcess>> {
        let (root, _) = self.root_limits(run_id)?;
        let mut query=self.db.prepare("SELECT v.version,v.manifest FROM process_versions v WHERE v.version IN (SELECT version FROM runs WHERE id=?1 UNION SELECT version FROM run_call_snapshots WHERE root_run_id=?1) ORDER BY v.version")?;
        query
            .query_map([root], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (version, manifest) = row?;
                let process = admit_json(manifest.as_bytes())?;
                if process.version != version {
                    return Err(RuntimeError::Integrity(
                        "pinned child process digest mismatch".into(),
                    ));
                }
                Ok(process)
            })
            .collect()
    }

    pub(crate) fn create_child(&mut self, parent: &str, step: &Step, input: &Value) -> Result<()> {
        let call = step
            .call
            .as_ref()
            .ok_or_else(|| RuntimeError::State("missing child call definition".into()))?;
        let existing: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM child_runs WHERE parent_run_id=?1 AND parent_step_id=?2)",
            params![parent, step.id],
            |row| row.get(0),
        )?;
        if existing {
            return Ok(());
        }
        let (root, limits) = self.root_limits(parent)?;
        let (assignment, depth, fixture): (String, u32, bool) = self.db.query_row(
            "SELECT assignment_id,depth,fixture FROM runs WHERE id=?1",
            [parent],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        self.ensure_assignment_active(&assignment)?;
        let snapshot:Option<(String,String,u64,u64)>=self.db.query_row("SELECT p.manifest,s.config,s.config_project_revision,s.config_assignment_revision FROM run_call_snapshots s JOIN process_versions p ON p.version=s.version WHERE s.root_run_id=?1 AND s.version=?2",params![root,call.process_ref],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
        let (manifest, config, project_revision, assignment_revision) =
            snapshot.ok_or(RuntimeError::Forbidden)?;
        let process = admit_json(manifest.as_bytes())?;
        if process.version != call.process_ref {
            return Err(RuntimeError::Integrity(
                "frozen child process digest mismatch".into(),
            ));
        }
        let config: Value = serde_json::from_str(&config)?;
        bounded(input)?;
        validate_instance(&process.manifest.input_schema, input)
            .map_err(|error| RuntimeError::InvalidInput(error.to_string()))?;
        self.authorize_artifacts(&assignment, input)?;
        self.authorize_artifacts(&assignment, &config)?;
        let artifact_limit = process
            .manifest
            .limits
            .max_artifact_bytes
            .min(limits.max_artifact_bytes);
        let refs = self.checked_refs(input, artifact_limit)?;
        let config_refs = self.checked_refs(&config, artifact_limit)?;
        let (children,steps):(u32,u32)=self.db.query_row("SELECT (SELECT COUNT(*) FROM runs WHERE COALESCE(root_run_id,id)=?1 AND id<>?1),(SELECT COUNT(*) FROM steps s JOIN runs r ON r.id=s.run_id WHERE COALESCE(r.root_run_id,r.id)=?1)",[&root],|row|Ok((row.get(0)?,row.get(1)?)))?;
        if depth >= limits.max_call_depth
            || children >= limits.max_child_runs
            || steps.saturating_add(process.manifest.steps.len() as u32) > limits.max_step_instances
        {
            return Err(RuntimeError::LimitExceeded(
                "shared child depth, count or step budget exhausted".into(),
            ));
        }
        let child = new_id("run");
        let attempt = new_id("attempt");
        let digest = content_digest(
            &json!({"version":process.version,"input":input,"config":config,"config_overrides":{},"fixture":fixture}),
        )?;
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO runs(id,root_run_id,parent_run_id,parent_step_id,depth,request_key,request_digest,version,input,state,created_at,assignment_id,config,config_overrides,config_project_revision,config_assignment_revision,fixture) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',?10,?11,?12,'{}',?13,?14,?15)",params![child,root,parent,step.id,depth+1,serde_json::to_string(&json!([assignment,"$child",parent,step.id]))?,digest,process.version,serde_json::to_string(input)?,now_ms(),assignment,serde_json::to_string(&config)?,project_revision,assignment_revision,fixture])?;
        for (position, definition) in process.manifest.steps.iter().enumerate() {
            tx.execute("INSERT INTO steps(run_id,id,definition_id,position,state,retry_limit) VALUES (?1,?2,?2,?3,'queued',?4)",params![child,definition.id,position*1001,definition.retry.max_attempts])?;
        }
        tx.execute("INSERT INTO run_bindings(run_id,kind,name,revision,payload) SELECT ?1,kind,name,revision,payload FROM run_bindings WHERE run_id=?2",params![child,root])?;
        tx.execute(
            "INSERT INTO child_runs(parent_run_id,parent_step_id,child_run_id) VALUES (?1,?2,?3)",
            params![parent, step.id, child],
        )?;
        tx.execute("UPDATE steps SET state='waiting_for_child',attempt_count=attempt_count+1,fence=fence+1,active_attempt=?3 WHERE run_id=?1 AND id=?2 AND state='queued'",params![parent,step.id,attempt])?;
        let (number, fence): (u32, u32) = tx.query_row(
            "SELECT attempt_count,fence FROM steps WHERE run_id=?1 AND id=?2",
            params![parent, step.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.execute("INSERT INTO attempts(id,run_id,step_id,number,fence,state,started_at) VALUES (?1,?2,?3,?4,?5,'waiting_for_child',?6)",params![attempt,parent,step.id,number,fence,now_ms()])?;
        link_artifacts(&tx, &child, "$input", &refs)?;
        link_artifacts(&tx, &child, "$config", &config_refs)?;
        event(
            &tx,
            &child,
            "run.created",
            json!({"process_version":process.version,"root_run_id":root,"parent_run_id":parent,"parent_step_instance_id":step.id}),
        )?;
        event(
            &tx,
            parent,
            "child.created",
            json!({"step_id":step.id,"child_run_id":child,"process_version":process.version}),
        )?;
        tx.commit()?;
        fault_point(&format!("child_created:{}", step.id));
        Ok(())
    }

    /// Returns true only when a saved terminal child was folded into its parent.
    pub(crate) fn settle_child(&mut self, parent: &str, step: &Step) -> Result<bool> {
        let child:Option<(String,String,Option<String>)>=self.db.query_row("SELECT r.id,r.state,r.output FROM child_runs c JOIN runs r ON r.id=c.child_run_id WHERE c.parent_run_id=?1 AND c.parent_step_id=?2",params![parent,step.id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let (child, state, output) = child
            .ok_or_else(|| RuntimeError::Integrity("waiting child identity is missing".into()))?;
        if !matches!(state.as_str(), "succeeded" | "failed" | "cancelled") {
            return Ok(false);
        }
        if state != "succeeded" {
            let tx = self.db.transaction()?;
            tx.execute(
                "UPDATE steps SET state='failed' WHERE run_id=?1 AND id=?2",
                params![parent, step.id],
            )?;
            tx.execute("UPDATE attempts SET state='failed',finished_at=?3,error='child did not succeed' WHERE run_id=?1 AND step_id=?2 AND state='waiting_for_child'",params![parent,step.id,now_ms()])?;
            tx.execute(
                "UPDATE runs SET state='failed',error='required child did not succeed' WHERE id=?1",
                [parent],
            )?;
            event(
                &tx,
                parent,
                "child.failed",
                json!({"step_id":step.id,"child_run_id":child,"child_state":state}),
            )?;
            tx.commit()?;
            return Ok(true);
        }
        let output: Value = serde_json::from_str(&output.ok_or_else(|| {
            RuntimeError::Integrity("successful child has no committed output".into())
        })?)?;
        let pointer = &step
            .call
            .as_ref()
            .ok_or_else(|| RuntimeError::State("missing call definition".into()))?
            .output_pointer;
        let output = output.pointer(pointer).ok_or_else(|| {
            RuntimeError::InvalidInput("child output projection does not exist".into())
        })?;
        bounded(output)?;
        let (process, _, _) = self.load_run(parent)?;
        let limits = self.effective_limits(parent, &process.manifest.limits)?;
        let refs = self.checked_refs(output, limits.max_artifact_bytes)?;
        let tx = self.db.transaction()?;
        tx.execute("UPDATE steps SET state='succeeded',output=?3,active_attempt=NULL WHERE run_id=?1 AND id=?2 AND state='waiting_for_child'",params![parent,step.id,serde_json::to_string(output)?])?;
        tx.execute("UPDATE attempts SET state='succeeded',finished_at=?3 WHERE run_id=?1 AND step_id=?2 AND state='waiting_for_child'",params![parent,step.id,now_ms()])?;
        // Save the wakeup with the join: a crash after committing the child
        // output must not strand the parent in its old waiting state.
        tx.execute(
            "UPDATE runs SET state='queued',error=NULL WHERE id=?1",
            [parent],
        )?;
        link_artifacts(&tx, parent, &step.id, &refs)?;
        event(
            &tx,
            parent,
            "child.completed",
            json!({"step_id":step.id,"child_run_id":child,"output_digest":content_digest(output)?}),
        )?;
        tx.commit()?;
        fault_point(&format!("child_joined:{}", step.id));
        Ok(true)
    }
}

pub(crate) fn freeze_call_snapshots(
    tx: &rusqlite::Transaction<'_>,
    root: &str,
    assignment: Option<&str>,
    snapshots: &CallSnapshots,
) -> Result<()> {
    for child in &snapshots.children {
        tx.execute("INSERT INTO run_call_snapshots(root_run_id,version,config,config_project_revision,config_assignment_revision) VALUES (?1,?2,?3,?4,?5)",params![root,child.process.version,serde_json::to_string(&child.config.value)?,child.config.project_revision,child.config.assignment_revision])?;
        crate::execution::freeze_execution_bindings(tx, root, &child.process.manifest, assignment)?;
    }
    link_artifacts(tx, root, "$frozen_graph", &snapshots.references)?;
    Ok(())
}

pub(crate) fn restore_parent_calls(
    tx: &rusqlite::Transaction<'_>,
    parents: &[RetriedParent],
) -> Result<()> {
    for parent in parents {
        tx.execute("UPDATE steps SET state='waiting_for_child' WHERE run_id=?1 AND id=?2 AND state='failed'",params![parent.run_id,parent.step_id])?;
        tx.execute("UPDATE attempts SET state='waiting_for_child',finished_at=NULL,error=NULL WHERE run_id=?1 AND step_id=?2 AND state='failed'",params![parent.run_id,parent.step_id])?;
        tx.execute(
            "UPDATE runs SET state='queued',error=NULL WHERE id=?1 AND state='failed'",
            [&parent.run_id],
        )?;
        event(
            tx,
            &parent.run_id,
            "child.retry_requested",
            json!({"step_id":parent.step_id,"child_run_id":parent.child_id}),
        )?;
    }
    Ok(())
}

// Children are processed before parents, once per version. Distinct declared
// routes of one decision are mutually exclusive. Sum work within each route,
// take the largest route, and still charge every saved top-level step row.
#[derive(Clone, Copy, Default)]
struct ExecutionBound {
    depth: u32,
    children: u32,
    steps: u32,
}
impl ExecutionBound {
    fn add(&mut self, other: Self) {
        self.depth = self.depth.max(other.depth);
        self.children = self.children.saturating_add(other.children);
        self.steps = self.steps.saturating_add(other.steps);
    }
}
fn execution_bound(
    process: &AdmittedProcess,
    children: &BTreeMap<String, ExecutionBound>,
) -> ExecutionBound {
    let mut total = ExecutionBound {
        steps: process.manifest.steps.len() as u32,
        ..ExecutionBound::default()
    };
    let mut choices: BTreeMap<&str, BTreeMap<&str, ExecutionBound>> = BTreeMap::new();
    for step in &process.manifest.steps {
        let instances = step.foreach.as_ref().map_or(1, |each| each.max_items);
        let mut work = ExecutionBound {
            steps: if step.foreach.is_some() { instances } else { 0 },
            ..ExecutionBound::default()
        };
        if let Some(call) = &step.call {
            let child = children[&call.process_ref];
            work.depth = child.depth.saturating_add(1);
            work.children = instances.saturating_mul(child.children.saturating_add(1));
            work.steps = work
                .steps
                .saturating_add(instances.saturating_mul(child.steps));
        }
        if let Some(guard) = &step.when {
            choices
                .entry(&guard.decision)
                .or_default()
                .entry(&guard.route)
                .or_default()
                .add(work);
        } else {
            total.add(work);
        }
    }
    for routes in choices.values() {
        let mut maximum = ExecutionBound::default();
        for work in routes.values() {
            maximum.depth = maximum.depth.max(work.depth);
            maximum.children = maximum.children.max(work.children);
            maximum.steps = maximum.steps.max(work.steps);
        }
        total.add(maximum);
    }
    total
}
