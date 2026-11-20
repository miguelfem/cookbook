//! Narrow, immutable Jev question/policy tuning; never executes a provider.
use std::collections::BTreeSet;

use apr_core::{
    Binding, ProcessManifest, Step, admit_json, canonical_bytes, routing::DecisionPolicy,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Result, Runtime, RuntimeError, engine::bounded_detail, store::now_ms};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TuneJevArgs {
    pub process_ref: String,
    pub mode: String,
    pub changes: Vec<TuningChange>,
    #[serde(default)]
    pub expected_process_ref: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TuningChange {
    Question {
        step_id: String,
        question_id: String,
        question: Value,
    },
    Policy {
        step_id: String,
        policy: DecisionPolicy,
    },
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::InvalidInput(message.into())
}

fn questions(step: &Step) -> Option<&serde_json::Map<String, Value>> {
    if step.kind != "connector" || step.operation.as_deref() != Some("jev.evaluate") {
        return None;
    }
    match step.inputs.get("questions")? {
        Binding::Literal { literal } => literal.as_object(),
        _ => None,
    }
}

pub(crate) fn editable_jev_step(manifest: &ProcessManifest, step: &Step) -> bool {
    questions(step).is_some() || editable_policy(manifest, step)
}

fn editable_policy(manifest: &ProcessManifest, step: &Step) -> bool {
    let Some(policy) = &step.decision else {
        return false;
    };
    let bindings = policy.bindings();
    !bindings.is_empty()
        && bindings.iter().all(|binding| {
            let Binding::Step {
                step: source,
                pointer,
            } = binding
            else {
                return false;
            };
            let Some(source) = manifest
                .steps
                .iter()
                .find(|candidate| &candidate.id == source)
            else {
                return false;
            };
            let Some(questions) = questions(source) else {
                return false;
            };
            let mut parts = pointer.split('/');
            if parts.next() != Some("") || parts.next() != Some("answers") {
                return false;
            }
            let Some(id) = parts.next() else {
                return false;
            };
            let id = id.replace("~1", "/").replace("~0", "~");
            questions.contains_key(&id)
                && matches!(
                    parts.next(),
                    Some("choice" | "confidence" | "score" | "noul")
                )
                && parts.next().is_none()
        })
}

impl Runtime {
    pub(crate) fn jev_tuning(&self, assignment: &str, process_ref: &str) -> Result<Value> {
        let process = self.catalog_process(assignment, process_ref)?;
        let mut editable_questions = Vec::new();
        let mut policies = Vec::new();
        for step in &process.manifest.steps {
            if let Some(questions) = questions(step) {
                for (id, question) in questions {
                    editable_questions
                        .push(json!({"step_id":step.id,"question_id":id,"question":question}));
                }
            }
            if editable_policy(&process.manifest, step) {
                policies.push(json!({"step_id":step.id,"policy":step.decision}));
            }
        }
        let detail = json!({"process_ref":process_ref,"questions":editable_questions,"policies":policies,
            "limits":{"same_question_ids":true,"same_choice_options":true,"same_score_levels":true,"same_routes_and_bindings":true}});
        bounded_detail(&detail)?;
        Ok(detail)
    }

    /// Called only by scoped author manage while holding the runtime writer lock.
    pub(crate) fn tune_jev(&mut self, assignment: &str, args: TuneJevArgs) -> Result<Value> {
        if !matches!(args.mode.as_str(), "preview" | "publish")
            || args.changes.is_empty()
            || args.changes.len() > 100
        {
            return Err(invalid(
                "tuning requires preview or publish and one to 100 changes",
            ));
        }
        let base = self.catalog_process(assignment, &args.process_ref)?;
        // Edit the stored wire representation to avoid changing unrelated optional/default fields.
        let mut manifest: Value = serde_json::from_slice(&base.canonical_bytes)?;
        let mut seen = BTreeSet::new();
        for change in &args.changes {
            match change {
                TuningChange::Question {
                    step_id,
                    question_id,
                    question,
                } => {
                    if !seen.insert((step_id.as_str(), Some(question_id.as_str()))) {
                        return Err(invalid("duplicate question change"));
                    }
                    let (index, step) = base
                        .manifest
                        .steps
                        .iter()
                        .enumerate()
                        .find(|(_, step)| &step.id == step_id)
                        .ok_or(RuntimeError::Forbidden)?;
                    let old = questions(step)
                        .and_then(|questions| questions.get(question_id))
                        .ok_or(RuntimeError::Forbidden)?;
                    validate_question_change(old, question)?;
                    manifest["steps"][index]["inputs"]["questions"]["literal"][question_id] =
                        question.clone();
                }
                TuningChange::Policy { step_id, policy } => {
                    if !seen.insert((step_id.as_str(), None)) {
                        return Err(invalid("duplicate policy change"));
                    }
                    let (index, step) = base
                        .manifest
                        .steps
                        .iter()
                        .enumerate()
                        .find(|(_, step)| &step.id == step_id)
                        .ok_or(RuntimeError::Forbidden)?;
                    if !editable_policy(&base.manifest, step) {
                        return Err(RuntimeError::Forbidden);
                    }
                    let old = step.decision.as_ref().ok_or(RuntimeError::Forbidden)?;
                    preserve_policy_structure(old, policy)?;
                    manifest["steps"][index]["decision"] = serde_json::to_value(policy)?;
                }
            }
        }
        let candidate = admit_json(&canonical_bytes(&manifest)?)?;
        self.authorize_literals(assignment, &candidate.manifest)?;
        for step in &candidate.manifest.steps {
            if let Some(questions) = questions(step) {
                validate_questions(questions)?;
            }
        }
        if args
            .expected_process_ref
            .as_ref()
            .is_some_and(|expected| expected != &candidate.version)
        {
            return Err(invalid(
                "candidate differs from the reviewed process version",
            ));
        }
        if args.mode == "publish" && args.expected_process_ref.is_none() {
            return Err(invalid(
                "publish requires expected_process_ref from preview",
            ));
        }
        // Preserve the original registered bytes, even if project files changed or vanished.
        let mut sources = Vec::new();
        for file in base.manifest.files.as_deref().unwrap_or(&[]) {
            let bytes: Option<Vec<u8>> = self
                .db
                .query_row(
                    "SELECT source FROM process_sources WHERE version=?1 AND path=?2 AND sha256=?3",
                    params![base.version, file.path, file.sha256],
                    |row| row.get(0),
                )
                .optional()?;
            let bytes = bytes.ok_or_else(|| {
                RuntimeError::Integrity("registered source snapshot is missing".into())
            })?;
            if format!("{:x}", Sha256::digest(&bytes)) != file.sha256 {
                return Err(RuntimeError::Integrity(
                    "registered source snapshot digest mismatch".into(),
                ));
            }
            sources.push((file.path.clone(), bytes));
        }
        let result = json!({"base_process_ref":base.version,"process_ref":candidate.version,"registered":args.mode=="publish","valid":true,
            "changes":args.changes,"validation":{"provider_inference":false,"execution_authorized":false,"adapter_included":cfg!(feature="jev")}});
        bounded_detail(&result)?;
        if args.mode == "publish" {
            self.snapshot_sources(&candidate, sources)?;
            self.db.execute("INSERT OR IGNORE INTO catalog(assignment_id,version,name,registered_at) VALUES (?1,?2,?3,?4)",params![assignment,candidate.version,candidate.manifest.name,now_ms()])?;
        }
        Ok(result)
    }
}

fn preserve_policy_structure(old: &DecisionPolicy, new: &DecisionPolicy) -> Result<()> {
    if old.routes() != new.routes() || old.bindings() != new.bindings() {
        return Err(invalid("tuning cannot change routes or data bindings"));
    }
    match (old, new) {
        (DecisionPolicy::Choice { .. }, DecisionPolicy::Choice { .. }) => {}
        (DecisionPolicy::Rules { cases: old, .. }, DecisionPolicy::Rules { cases: new, .. })
            if old.len() == new.len() => {}
        _ => {
            return Err(invalid(
                "tuning cannot change the decision source or rule count",
            ));
        }
    }
    apr_core::routing::validate_policy(new)?;
    Ok(())
}

fn validate_question_change(old: &Value, new: &Value) -> Result<()> {
    validate_question(new)?;
    if old["type"] != new["type"] {
        return Err(invalid("question type is immutable"));
    }
    match new["type"].as_str() {
        Some("choice")
            if old["criteria"]
                .as_object()
                .map(|value| value.keys().collect::<Vec<_>>())
                != new["criteria"]
                    .as_object()
                    .map(|value| value.keys().collect::<Vec<_>>()) =>
        {
            return Err(invalid("choice option IDs are immutable"));
        }
        Some("score")
            if old["criteria"].as_array().map(Vec::len)
                != new["criteria"].as_array().map(Vec::len) =>
        {
            return Err(invalid("score level count is immutable"));
        }
        _ => {}
    }
    Ok(())
}

fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
fn structured(value: &Value) -> bool {
    value.is_object() || value.is_array() || value.as_str().is_some_and(|text| !text.is_empty())
}

// Offline authoring must also work in default builds where the adapter is absent.
// These structural constraints mirror the provider's pure request contract; an
// included adapter additionally validates the same questions with prepare().
fn validate_question(question: &Value) -> Result<()> {
    let object = question
        .as_object()
        .ok_or_else(|| invalid("question must be an object"))?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "type" | "instructions" | "criteria"))
        || !structured(&question["instructions"])
    {
        return Err(invalid("question has unsupported fields or instructions"));
    }
    let valid = match question["type"].as_str() {
        Some("choice") => question["criteria"].as_object().is_some_and(|options| {
            (2..=32).contains(&options.len())
                && options.iter().all(|(key, value)| {
                    label(key) && (value.is_null() || value.is_string() || value.is_object())
                })
        }),
        Some("score") => question["criteria"].as_array().is_some_and(|levels| {
            (2..=32).contains(&levels.len()) && levels.iter().all(structured)
        }),
        Some("noul") => question.get("criteria").is_none_or(|criteria| {
            criteria.is_null()
                || criteria.as_object().is_some_and(|criteria| {
                    criteria.len() == 2
                        && ["true", "false"].iter().all(|key| {
                            criteria
                                .get(*key)
                                .and_then(Value::as_str)
                                .is_some_and(|value| !value.is_empty())
                        })
                })
        }),
        _ => false,
    };
    if !valid {
        return Err(invalid("question does not match the Jev contract"));
    }
    Ok(())
}

fn validate_questions(questions: &serde_json::Map<String, Value>) -> Result<()> {
    if questions.is_empty() || questions.len() > 16 || questions.keys().any(|key| !label(key)) {
        return Err(invalid("invalid Jev question IDs or count"));
    }
    for question in questions.values() {
        validate_question(question)?;
    }
    let request = json!({"model":"offline-validation","state":{},"questions":questions});
    if canonical_bytes(&request)?.len() > 64 * 1024 {
        return Err(invalid("Jev question request exceeds 64 KiB"));
    }
    #[cfg(feature = "jev")]
    apr_provider_jev::prepare(request)
        .map_err(|_| invalid("Jev adapter rejected the questions"))?;
    Ok(())
}
