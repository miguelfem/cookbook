//! Durable assignment-scoped jobs. A firing records its occurrence, queued run,
//! and next due time in one transaction; executing the run remains ordinary APR
//! work. Missed windows coalesce, and an active run prevents overlap.

use std::collections::{BTreeMap, BTreeSet};

use apr_core::{AdmittedProcess, ArtifactRef, content_digest, validate_instance};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

use crate::{
    Result, Runtime, RuntimeError,
    engine::{
        RunRecord, bounded, bounded_detail, check_key, insert_run_record, link_artifacts, new_id,
    },
    scope::ResolvedConfig,
    store::{StoreLock, event, fault_point, now_ms},
};

pub(crate) const MIGRATION_V9: &str = r#"
CREATE TABLE schedules (
 id TEXT PRIMARY KEY,
 assignment_id TEXT NOT NULL REFERENCES assignments(id),
 author_attachment_id TEXT NOT NULL REFERENCES attachments(id),
 request_key TEXT NOT NULL,
 request_digest TEXT NOT NULL,
 name TEXT NOT NULL,
 trigger_json TEXT NOT NULL,
 template_json TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('active','paused','completed','deleted')),
 next_due_ms INTEGER,
 last_error TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 UNIQUE(assignment_id,request_key)
);
CREATE INDEX schedules_due ON schedules(state,next_due_ms);
CREATE INDEX schedules_by_assignment ON schedules(assignment_id,id);
ALTER TABLE runs ADD COLUMN schedule_id TEXT REFERENCES schedules(id);
ALTER TABLE runs ADD COLUMN scheduled_for_ms INTEGER;
CREATE INDEX runs_by_schedule ON runs(schedule_id);
CREATE TABLE schedule_occurrences (
 schedule_id TEXT NOT NULL REFERENCES schedules(id),
 due_at_ms INTEGER NOT NULL,
 run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
 original_run_id TEXT,
 outcome TEXT NOT NULL CHECK(outcome IN ('started','skipped_overlap')),
 recorded_at_ms INTEGER NOT NULL,
 PRIMARY KEY(schedule_id,due_at_ms)
);
CREATE INDEX schedule_occurrences_runs ON schedule_occurrences(run_id);
"#;

const MAX_TIMESTAMP_MS: i64 = 253_402_300_799_999;
const MAX_INTERVAL_SECONDS: u64 = 10 * 366 * 24 * 60 * 60;
const MAX_SCHEDULES_PER_ASSIGNMENT: u64 = 100;
const MAX_TEMPLATE_BYTES: usize = 16 * 1024 * 1024;

/// All calendar schedules use UTC. Numeric five-field cron supports `*`, lists,
/// ranges, and steps; Sunday is 0 or 7. No timezone/DST or seconds-field syntax.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScheduleTrigger {
    Once {
        at_ms: i64,
    },
    Interval {
        every_seconds: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_at_ms: Option<i64>,
    },
    Cron {
        expression: String,
    },
}

impl ScheduleTrigger {
    fn first_due(&self, now: i64) -> Result<i64> {
        timestamp(now)?;
        let due = match self {
            Self::Once { at_ms } => *at_ms,
            Self::Interval {
                every_seconds,
                start_at_ms,
            } => {
                let interval = interval_ms(*every_seconds)?;
                start_at_ms.unwrap_or(now.checked_add(interval).ok_or_else(time_overflow)?)
            }
            Self::Cron { expression } => Cron::parse(expression)?.next_after(now)?,
        };
        timestamp(due)?;
        Ok(due)
    }

    fn after_firing(&self, due: i64, now: i64) -> Result<Option<i64>> {
        match self {
            Self::Once { .. } => Ok(None),
            Self::Interval { every_seconds, .. } => {
                let interval = interval_ms(*every_seconds)?;
                let windows = (now - due) / interval + 1;
                let next = due
                    .checked_add(windows.checked_mul(interval).ok_or_else(time_overflow)?)
                    .ok_or_else(time_overflow)?;
                timestamp(next)?;
                Ok(Some(next))
            }
            Self::Cron { expression } => Ok(Some(Cron::parse(expression)?.next_after(now)?)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScheduleCreate {
    pub name: String,
    pub process_ref: String,
    pub input: Value,
    pub request_key: String,
    #[serde(default = "empty_object")]
    pub config_overrides: Value,
    pub trigger: ScheduleTrigger,
}
fn empty_object() -> Value {
    json!({})
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenProcess {
    version: String,
    config: Value,
    project_revision: u64,
    assignment_revision: u64,
}
impl FrozenProcess {
    fn new(process: &AdmittedProcess, config: ResolvedConfig) -> Self {
        Self {
            version: process.version.clone(),
            config: config.value,
            project_revision: config.project_revision,
            assignment_revision: config.assignment_revision,
        }
    }
    fn config(&self) -> ResolvedConfig {
        ResolvedConfig {
            value: self.config.clone(),
            project_revision: self.project_revision,
            assignment_revision: self.assignment_revision,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenBinding {
    kind: String,
    name: String,
    revision: u64,
    payload: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenTemplate {
    root: FrozenProcess,
    children: Vec<FrozenProcess>,
    input: Value,
    config_overrides: Value,
    bindings: Vec<FrozenBinding>,
    references: Vec<ArtifactRef>,
}

type ScheduleViewRow = (
    String,
    String,
    String,
    Option<i64>,
    Option<String>,
    String,
    String,
    i64,
    i64,
);

struct Job {
    id: String,
    assignment: String,
    trigger: ScheduleTrigger,
    template: FrozenTemplate,
    due: i64,
}

/// Bounded periodic work; timestamps can be supplied by deterministic tests.
#[derive(Default, Debug, Serialize)]
pub struct ScheduleTick {
    pub started: usize,
    pub skipped_overlap: usize,
    pub paused: usize,
}

impl Runtime {
    pub(crate) fn create_schedule(
        &mut self,
        assignment: &str,
        author_attachment_id: &str,
        args: ScheduleCreate,
    ) -> Result<Value> {
        check_key(&args.request_key)?;
        if args.name.trim().is_empty()
            || args.name.len() > 160
            || args.name.chars().any(char::is_control)
        {
            return Err(invalid(
                "schedule name must contain 1–160 bytes without control characters",
            ));
        }
        bounded(&args.input)?;
        bounded(&args.config_overrides)?;
        let now = now_ms();
        let due = args.trigger.first_due(now)?;
        let digest = content_digest(&json!({"name":args.name,"process_ref":args.process_ref,
            "input":args.input,"config_overrides":args.config_overrides,"trigger":args.trigger}))?;
        let existing: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT id,request_digest FROM schedules WHERE assignment_id=?1 AND request_key=?2",
                params![assignment, args.request_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id, previous)) = existing {
            if previous != digest {
                return Err(RuntimeError::RequestConflict);
            }
            return self.schedule_view(assignment, &id, false);
        }
        let count: u64 = self.db.query_row(
            "SELECT count(*) FROM schedules WHERE assignment_id=?1 AND state<>'deleted'",
            [assignment],
            |row| row.get(0),
        )?;
        if count >= MAX_SCHEDULES_PER_ASSIGNMENT {
            return Err(RuntimeError::LimitExceeded(
                "an assignment may have at most 100 undeleted schedules".into(),
            ));
        }
        let process = self.catalog_process(assignment, &args.process_ref)?;
        self.ensure_assignment_active(assignment)?;
        self.authorize_artifacts(assignment, &args.input)?;
        self.authorize_literals(assignment, &process.manifest)?;
        validate_instance(&process.manifest.input_schema, &args.input)
            .map_err(|error| invalid(&error.to_string()))?;
        self.execution_preflight(&process, Some(assignment))?;
        let config =
            self.resolve_configuration(&process, Some(assignment), &args.config_overrides)?;
        self.authorize_artifacts(assignment, &config.value)?;
        let mut references =
            self.checked_refs(&args.input, process.manifest.limits.max_artifact_bytes)?;
        references
            .extend(self.checked_refs(&config.value, process.manifest.limits.max_artifact_bytes)?);
        let calls = self.prepare_call_snapshots(&process, Some(assignment), false)?;
        references.extend(calls.references);
        let mut bindings = BTreeMap::new();
        for candidate in
            std::iter::once(&process).chain(calls.children.iter().map(|child| &child.process))
        {
            for step in &candidate.manifest.steps {
                let Some((kind, name)) = crate::execution::binding_for(step) else {
                    continue;
                };
                let (revision,payload): (u64,String) = self.db.query_row(
                    "SELECT revision,payload FROM execution_grants WHERE assignment_id=?1 AND kind=?2 AND name=?3 AND enabled=1",
                    params![assignment,kind,name], |row| Ok((row.get(0)?,row.get(1)?))).optional()?.ok_or(RuntimeError::Forbidden)?;
                bindings.insert(
                    (kind.to_owned(), name.to_owned()),
                    FrozenBinding {
                        kind: kind.into(),
                        name: name.into(),
                        revision,
                        payload,
                    },
                );
            }
        }
        references.sort_by(|a, b| a.artifact_id.cmp(&b.artifact_id));
        references.dedup_by(|a, b| a.artifact_id == b.artifact_id);
        let template = FrozenTemplate {
            root: FrozenProcess::new(&process, config),
            children: calls
                .children
                .into_iter()
                .map(|child| FrozenProcess::new(&child.process, child.config))
                .collect(),
            input: args.input,
            config_overrides: args.config_overrides,
            bindings: bindings.into_values().collect(),
            references,
        };
        bounded_detail(
            &json!({"input":template.input,"config":template.root.config,
            "config_overrides":template.config_overrides,"child_process_refs":template.children.iter().take(20).map(|child| &child.version).collect::<Vec<_>>()}),
        )?;
        let encoded = serde_json::to_string(&template)?;
        if encoded.len() > MAX_TEMPLATE_BYTES {
            return Err(RuntimeError::LimitExceeded(
                "frozen schedule template exceeds 16 MiB".into(),
            ));
        }
        let id = new_id("schedule");
        self.db.execute("INSERT INTO schedules(id,assignment_id,author_attachment_id,request_key,request_digest,name,trigger_json,template_json,state,next_due_ms,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'active',?9,?10,?10)",
            params![id,assignment,author_attachment_id,args.request_key,digest,args.name,serde_json::to_string(&args.trigger)?,encoded,due,now])?;
        self.schedule_view(assignment, &id, false)
    }

    pub(crate) fn control_schedule(
        &mut self,
        assignment: &str,
        id: &str,
        action: &str,
    ) -> Result<Value> {
        let state: String = self
            .db
            .query_row(
                "SELECT state FROM schedules WHERE id=?1 AND assignment_id=?2",
                params![id, assignment],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(RuntimeError::Forbidden)?;
        let next_state = match action {
            "pause" if matches!(state.as_str(), "active" | "paused") => "paused",
            "resume" if matches!(state.as_str(), "active" | "paused") => {
                self.ensure_assignment_active(assignment)?;
                let encoded: String = self.db.query_row(
                    "SELECT template_json FROM schedules WHERE id=?1",
                    [id],
                    |row| row.get(0),
                )?;
                let template: FrozenTemplate = serde_json::from_str(&encoded)?;
                self.validate_schedule_template(assignment, &template)?;
                "active"
            }
            "delete" => "deleted",
            _ => {
                return Err(RuntimeError::State(
                    "completed or deleted schedules cannot be resumed; create a new schedule"
                        .into(),
                ));
            }
        };
        // Keep the tombstone/request key and occurrence history; deleting never
        // cancels accepted runs and a retransmitted create cannot recreate a job.
        self.db.execute(
            "UPDATE schedules SET state=?2,last_error=NULL,updated_at=?3 WHERE id=?1",
            params![id, next_state, now_ms()],
        )?;
        self.schedule_view(assignment, id, false)
    }

    pub(crate) fn schedule_list(
        &self,
        assignment: &str,
        limit: Option<usize>,
        after: Option<String>,
    ) -> Result<Value> {
        let limit = limit.unwrap_or(10);
        if !(1..=20).contains(&limit)
            || after
                .as_ref()
                .is_some_and(|s| s.len() > 160 || s.chars().any(char::is_control))
        {
            return Err(invalid("invalid schedule limit or cursor"));
        }
        let mut query = self.db.prepare("SELECT id FROM schedules WHERE assignment_id=?1 AND state<>'deleted' AND (?2 IS NULL OR id>?2) ORDER BY id LIMIT ?3")?;
        let ids: Vec<String> = query
            .query_map(params![assignment, after, limit + 1], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        let truncated = ids.len() > limit;
        let mut cursor = after;
        let mut schedules = Vec::new();
        for id in ids.into_iter().take(limit) {
            cursor = Some(id.clone());
            schedules.push(self.schedule_view(assignment, &id, false)?);
        }
        Ok(json!({"schedules":schedules,"next_cursor":cursor,"truncated":truncated}))
    }

    pub(crate) fn schedule_view(&self, assignment: &str, id: &str, detail: bool) -> Result<Value> {
        let row: Option<ScheduleViewRow> = self.db.query_row(
            "SELECT name,state,trigger_json,next_due_ms,last_error,template_json,author_attachment_id,created_at,updated_at FROM schedules WHERE id=?1 AND assignment_id=?2",
            params![id,assignment], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?))).optional()?;
        let (name, state, trigger, due, error, template, author, created, updated) =
            row.ok_or(RuntimeError::Forbidden)?;
        let template: FrozenTemplate = serde_json::from_str(&template)?;
        let last: Option<(i64,Option<String>,String)> = self.db.query_row(
            "SELECT due_at_ms,original_run_id,outcome FROM schedule_occurrences WHERE schedule_id=?1 ORDER BY due_at_ms DESC LIMIT 1", [id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let mut result = json!({"schedule_id":id,"name":name,"state":state,"process_ref":template.root.version,
            "trigger":serde_json::from_str::<Value>(&trigger)?,"timezone":"UTC","next_due_ms":due,
            "config_digest":content_digest(&template.root.config)?,"last_error":error,
            "last_occurrence":last.map(|(due,run,outcome)|json!({"due_at_ms":due,"run_id":run,"outcome":outcome})),
            "created_at_ms":created,"updated_at_ms":updated});
        if detail {
            let mut query=self.db.prepare("SELECT due_at_ms,original_run_id,outcome,recorded_at_ms FROM schedule_occurrences WHERE schedule_id=?1 ORDER BY due_at_ms DESC LIMIT 20")?;
            let history=query.query_map([id],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?)))?
                .map(|row| { let (due,run,outcome,recorded)=row?; Ok(json!({"due_at_ms":due,"run_id":run,"outcome":outcome,"recorded_at_ms":recorded})) })
                .collect::<Result<Vec<_>>>()?;
            result["author_attachment_id"] = json!(author);
            result["input"] = template.input;
            result["config"] = template.root.config;
            result["config_overrides"] = template.config_overrides;
            result["child_process_refs"] = json!(
                template
                    .children
                    .iter()
                    .take(20)
                    .map(|child| &child.version)
                    .collect::<Vec<_>>()
            );
            result["child_process_refs_truncated"] = json!(template.children.len() > 20);
            result["history"] = json!(history);
            result["history_limit"] = json!(20);
            // Never publish credential references or connection payloads.
        }
        Ok(result)
    }

    /// Create at most 32 occurrences without executing effects. A tick always
    /// rechecks assignment/project identity and the exact saved grant revisions.
    pub fn tick_schedules(&mut self, at_ms: i64) -> Result<ScheduleTick> {
        timestamp(at_ms)?;
        let _lock = StoreLock::acquire(&self.root)?;
        if self.ensure_not_maintaining().is_err() {
            return Ok(ScheduleTick::default());
        }
        let rows = {
            let mut query=self.db.prepare("SELECT id,assignment_id,trigger_json,template_json,next_due_ms FROM schedules WHERE state='active' AND next_due_ms<=?1 ORDER BY next_due_ms,id LIMIT 32")?;
            query
                .query_map([at_ms], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut result = ScheduleTick::default();
        for (id, assignment, trigger, template, due) in rows {
            let attempt = (|| {
                let job = Job {
                    id: id.clone(),
                    assignment,
                    trigger: serde_json::from_str(&trigger)?,
                    template: serde_json::from_str(&template)?,
                    due,
                };
                self.fire_schedule(&job, at_ms)
            })();
            match attempt {
                Ok(true) => result.started += 1,
                Ok(false) => result.skipped_overlap += 1,
                Err(error) => {
                    self.db.execute("UPDATE schedules SET state='paused',last_error=?2,updated_at=?3 WHERE id=?1",
                        params![id,format!("{}: {}",error.code(),error).chars().take(500).collect::<String>(),at_ms])?;
                    result.paused += 1;
                }
            }
        }
        Ok(result)
    }

    fn validate_schedule_template(
        &self,
        assignment: &str,
        template: &FrozenTemplate,
    ) -> Result<AdmittedProcess> {
        self.ensure_assignment_active(assignment)?;
        self.validate_graph_capacity(1 + template.children.len())?;
        let mut root = None;
        for candidate in std::iter::once(&template.root).chain(&template.children) {
            let process = self.catalog_process(assignment, &candidate.version)?;
            self.execution_preflight(&process, Some(assignment))?;
            self.authorize_literals(assignment, &process.manifest)?;
            validate_instance(&process.manifest.config.schema, &candidate.config)
                .map_err(|error| invalid(&error.to_string()))?;
            // An existing schedule's accepted artifact bindings survive retention
            // of the run that originally produced the artifact. The schedule
            // itself is durable authority and retains those immutable objects.
            self.checked_refs(
                &candidate.config,
                process.manifest.limits.max_artifact_bytes,
            )?;
            if root.is_none() {
                root = Some(process);
            }
        }
        for binding in &template.bindings {
            let valid:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM execution_grants WHERE assignment_id=?1 AND kind=?2 AND name=?3 AND enabled=1 AND revision=?4 AND payload=?5)",
                params![assignment,binding.kind,binding.name,binding.revision,binding.payload],|row|row.get(0))?;
            if !valid {
                return Err(RuntimeError::State("a frozen schedule binding changed or was revoked; delete and recreate the schedule after reviewing its new authority".into()));
            }
        }
        let root = root.ok_or_else(|| RuntimeError::Integrity("empty schedule template".into()))?;
        self.validate_capacity_limits(&root.manifest.limits)?;
        validate_instance(&root.manifest.input_schema, &template.input)
            .map_err(|error| invalid(&error.to_string()))?;
        self.checked_refs(&template.input, root.manifest.limits.max_artifact_bytes)?;
        for reference in &template.references {
            self.artifacts.verify(reference, reference.bytes)?;
        }
        Ok(root)
    }

    fn fire_schedule(&mut self, job: &Job, now: i64) -> Result<bool> {
        let process = self.validate_schedule_template(&job.assignment, &job.template)?;
        let next = job.trigger.after_firing(job.due, now)?;
        let busy:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM runs root JOIN runs r ON COALESCE(r.root_run_id,r.id)=root.id WHERE root.schedule_id=?1 AND (r.state NOT IN ('succeeded','failed','cancelled') OR EXISTS(SELECT 1 FROM effect_intents e WHERE e.run_id=r.id AND e.status IN ('dispatched','uncertain'))))",
            [&job.id],|row|row.get(0))?;
        let tx = self.db.transaction()?;
        let run = if busy {
            None
        } else {
            let run = new_id("run");
            // Structured internal namespace cannot collide with a caller's
            // string request_key, even when that caller can inspect this job.
            let key = serde_json::to_string(&json!([
                job.assignment,
                {"schedule_id":job.id,"due_at_ms":job.due}
            ]))?;
            insert_run_record(
                &tx,
                &run,
                RunRecord {
                    process: &process,
                    input: &job.template.input,
                    request_key: &key,
                    assignment_id: Some(&job.assignment),
                    config: &job.template.root.config(),
                    config_overrides: &job.template.config_overrides,
                    fixture: false,
                },
            )?;
            tx.execute(
                "UPDATE runs SET schedule_id=?2,scheduled_for_ms=?3 WHERE id=?1",
                params![run, job.id, job.due],
            )?;
            for binding in &job.template.bindings {
                tx.execute("INSERT INTO run_bindings(run_id,kind,name,revision,payload) VALUES (?1,?2,?3,?4,?5)",
                    params![run,binding.kind,binding.name,binding.revision,binding.payload])?;
            }
            for child in &job.template.children {
                tx.execute("INSERT INTO run_call_snapshots(root_run_id,version,config,config_project_revision,config_assignment_revision) VALUES (?1,?2,?3,?4,?5)",
                    params![run,child.version,serde_json::to_string(&child.config)?,child.project_revision,child.assignment_revision])?;
            }
            link_artifacts(&tx, &run, "$schedule", &job.template.references)?;
            event(
                &tx,
                &run,
                "run.created",
                json!({"process_version":process.version,"schedule_id":job.id,"due_at_ms":job.due}),
            )?;
            Some(run)
        };
        tx.execute("INSERT INTO schedule_occurrences(schedule_id,due_at_ms,run_id,original_run_id,outcome,recorded_at_ms) VALUES (?1,?2,?3,?3,?4,?5)",
            params![job.id,job.due,run,if busy {"skipped_overlap"} else {"started"},now])?;
        tx.execute("UPDATE schedules SET next_due_ms=?2,state=?3,last_error=NULL,updated_at=?4 WHERE id=?1",
            params![job.id,next,if next.is_some() {"active"} else {"completed"},now])?;
        // next_due_ms is the durable replay high-water mark, committed with the
        // run. Bound diagnostics even if a one-second job waits for days.
        // Run provenance remains on runs, so old resumed runs still block overlap.
        tx.execute("DELETE FROM schedule_occurrences WHERE schedule_id=?1 AND due_at_ms NOT IN (SELECT due_at_ms FROM schedule_occurrences WHERE schedule_id=?1 ORDER BY due_at_ms DESC LIMIT 100)", [&job.id])?;
        fault_point("schedule_firing_before_commit");
        tx.commit()?;
        fault_point("schedule_firing_committed");
        Ok(!busy)
    }
}

fn timestamp(value: i64) -> Result<()> {
    if !(0..=MAX_TIMESTAMP_MS).contains(&value) {
        return Err(invalid(
            "schedule timestamp must be Unix milliseconds from 1970 through 9999 UTC",
        ));
    }
    Ok(())
}
fn interval_ms(seconds: u64) -> Result<i64> {
    if !(1..=MAX_INTERVAL_SECONDS).contains(&seconds) {
        return Err(invalid(
            "schedule interval must be 1 second through 10 years",
        ));
    }
    Ok(seconds as i64 * 1000)
}
fn invalid(message: &str) -> RuntimeError {
    RuntimeError::InvalidInput(message.into())
}
fn time_overflow() -> RuntimeError {
    invalid("schedule has no next occurrence in the supported time range")
}

struct Cron {
    minutes: BTreeSet<u8>,
    hours: BTreeSet<u8>,
    days: BTreeSet<u8>,
    months: BTreeSet<u8>,
    weekdays: BTreeSet<u8>,
    day_wildcard: bool,
    weekday_wildcard: bool,
}
impl Cron {
    fn parse(expression: &str) -> Result<Self> {
        if expression.len() > 200 {
            return Err(invalid("cron expression exceeds 200 bytes"));
        }
        let fields: Vec<_> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(invalid(
                "cron requires five numeric UTC fields: minute hour day month weekday",
            ));
        }
        Ok(Self {
            minutes: cron_field(fields[0], 0, 59)?,
            hours: cron_field(fields[1], 0, 23)?,
            days: cron_field(fields[2], 1, 31)?,
            months: cron_field(fields[3], 1, 12)?,
            weekdays: cron_field(fields[4], 0, 7)?
                .into_iter()
                .map(|n| n % 7)
                .collect(),
            day_wildcard: fields[2].starts_with('*'),
            weekday_wildcard: fields[4].starts_with('*'),
        })
    }
    fn next_after(&self, after: i64) -> Result<i64> {
        timestamp(after)?;
        let first = after / 60_000 * 60_000 + 60_000;
        let mut day = OffsetDateTime::from_unix_timestamp(first / 1000)
            .map_err(|_| time_overflow())?
            .date();
        for _ in 0..=366 * 8 {
            let dom = self.days.contains(&day.day());
            let dow = self
                .weekdays
                .contains(&day.weekday().number_days_from_sunday());
            let matches_day = if self.day_wildcard || self.weekday_wildcard {
                dom && dow
            } else {
                dom || dow
            };
            if self.months.contains(&(day.month() as u8)) && matches_day {
                for hour in &self.hours {
                    for minute in &self.minutes {
                        let candidate = day
                            .with_hms(*hour, *minute, 0)
                            .map_err(|_| time_overflow())?
                            .assume_utc()
                            .unix_timestamp()
                            * 1000;
                        if candidate >= first {
                            timestamp(candidate)?;
                            return Ok(candidate);
                        }
                    }
                }
            }
            day = day
                .checked_add(Duration::days(1))
                .ok_or_else(time_overflow)?;
        }
        Err(invalid("cron has no occurrence within eight years"))
    }
}
fn cron_field(field: &str, min: u8, max: u8) -> Result<BTreeSet<u8>> {
    let mut values = BTreeSet::new();
    let parse = |text: &str| -> Result<u8> {
        if text.is_empty() || !text.bytes().all(|n| n.is_ascii_digit()) {
            return Err(invalid("cron fields must be numeric"));
        }
        let n = text
            .parse::<u8>()
            .map_err(|_| invalid("invalid cron number"))?;
        if n < min || n > max {
            return Err(invalid("cron number is outside its field range"));
        }
        Ok(n)
    };
    for part in field.split(',') {
        let (range, step) = if let Some((range, step)) = part.split_once('/') {
            let step = step
                .parse::<u8>()
                .map_err(|_| invalid("invalid cron step"))?;
            if step == 0 || step > max - min + 1 {
                return Err(invalid("invalid cron step"));
            }
            (range, step)
        } else {
            (part, 1)
        };
        let (start, end) = if range == "*" {
            (min, max)
        } else if let Some((start, end)) = range.split_once('-') {
            (parse(start)?, parse(end)?)
        } else {
            let n = parse(range)?;
            if part.contains('/') { (n, max) } else { (n, n) }
        };
        if start > end {
            return Err(invalid("cron ranges must be ascending"));
        }
        values.extend((start..=end).step_by(step as usize));
    }
    if values.is_empty() {
        return Err(invalid("cron field is empty"));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(year: i32, month: time::Month, day: u8, hour: u8, minute: u8) -> i64 {
        time::Date::from_calendar_date(year, month, day)
            .unwrap()
            .with_hms(hour, minute, 0)
            .unwrap()
            .assume_utc()
            .unix_timestamp()
            * 1000
    }
    #[test]
    fn cron_handles_leap_days_weekdays_lists_ranges_steps_and_sunday_alias() {
        use time::Month::*;
        assert_eq!(
            Cron::parse("0 9 29 2 *")
                .unwrap()
                .next_after(at(2025, January, 1, 0, 0))
                .unwrap(),
            at(2028, February, 29, 9, 0)
        );
        assert_eq!(
            Cron::parse("*/15 9-17 * * 1-5")
                .unwrap()
                .next_after(at(2026, September, 18, 17, 45))
                .unwrap(),
            at(2026, September, 21, 9, 0)
        );
        assert_eq!(
            Cron::parse("5,35 9 * * 7")
                .unwrap()
                .next_after(at(2026, September, 19, 0, 0))
                .unwrap(),
            at(2026, September, 20, 9, 5)
        );
        assert_eq!(
            Cron::parse("0 0 1 * 1")
                .unwrap()
                .next_after(at(2026, September, 1, 0, 0))
                .unwrap(),
            at(2026, September, 7, 0, 0)
        );
        assert!(
            Cron::parse("0 0 31 2 *")
                .unwrap()
                .next_after(at(2026, January, 1, 0, 0))
                .is_err()
        );
        for bad in [
            "* * * *",
            "60 * * * *",
            "*/0 * * * *",
            "* * * * MON",
            "* 8-2 * * *",
            "* * 0 * *",
            "* * * 13 *",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn interval_coalesces_without_drifting_or_replaying_clock_rollback() {
        let trigger = ScheduleTrigger::Interval {
            every_seconds: 60,
            start_at_ms: Some(1000),
        };
        assert_eq!(trigger.after_firing(1000, 181_999).unwrap(), Some(241_000));
        assert!(
            ScheduleTrigger::Interval {
                every_seconds: 0,
                start_at_ms: None
            }
            .first_due(0)
            .is_err()
        );
        assert!(ScheduleTrigger::Once { at_ms: -1 }.first_due(0).is_err());
    }
}
