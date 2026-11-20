//! Process admission, deterministic identity, and data binding.

pub mod audit;
pub mod classifier;
pub mod routing;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

/// APR's versioned canonical JSON encoding (not RFC 8785).
pub const CANONICAL_FORMAT: &str = "apr-json-v1";
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

static MANIFEST_VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/process.schema.json"))
        .expect("checked-in process schema must parse");
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .expect("checked-in process schema must compile")
});

static ROUTING_MANIFEST_VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/process-v0.2.schema.json"))
            .expect("checked-in routing schema must parse");
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .expect("checked-in routing schema must compile")
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub sha256: String,
    pub media_type: String,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct AdmittedProcess {
    pub manifest: ProcessManifest,
    pub version: String,
    pub canonical_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessManifest {
    pub schema_version: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub config: ConfigDefinition,
    pub capabilities: Vec<Value>,
    pub limits: Limits,
    pub steps: Vec<Step>,
    pub output: BTreeMap<String, Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<BundledFile>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigDefinition {
    pub schema: Value,
    pub defaults: Value,
    pub run_override_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_step_ms: u64,
    pub max_artifact_bytes: u64,
    pub max_provider_calls: u32,
    pub max_agent_requests: u32,
    #[serde(
        default = "default_call_depth",
        skip_serializing_if = "is_default_call_depth"
    )]
    pub max_call_depth: u32,
    #[serde(
        default = "default_child_runs",
        skip_serializing_if = "is_default_child_runs"
    )]
    pub max_child_runs: u32,
    #[serde(
        default = "default_step_instances",
        skip_serializing_if = "is_default_step_instances"
    )]
    pub max_step_instances: u32,
}

fn default_call_depth() -> u32 {
    8
}
fn default_child_runs() -> u32 {
    256
}
fn default_step_instances() -> u32 {
    10_000
}
fn is_default_call_depth(value: &u32) -> bool {
    *value == default_call_depth()
}
fn is_default_child_runs(value: &u32) -> bool {
    *value == default_child_runs()
}
fn is_default_step_instances(value: &u32) -> bool {
    *value == default_step_instances()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retry {
    pub max_attempts: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundledFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub kind: String,
    pub effect: String,
    pub needs: Vec<String>,
    pub retry: Retry,
    pub inputs: BTreeMap<String, Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreach: Option<Foreach>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<routing::DecisionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<routing::BranchWhen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<routing::JoinDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<routing::CallDefinition>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Foreach {
    pub items: Binding,
    pub key: String,
    pub max_items: u32,
    pub concurrency: u32,
}

impl Step {
    /// Every data binding declared by this step, for admission and scoped checks.
    pub fn bindings(&self) -> Vec<&Binding> {
        self.inputs
            .values()
            .chain(self.foreach.iter().map(|each| &each.items))
            .chain(
                self.decision
                    .iter()
                    .flat_map(routing::DecisionPolicy::bindings),
            )
            .chain(self.join.iter().flat_map(|join| join.branches.values()))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Binding {
    Literal { literal: Value },
    Input { input: String },
    Config { config: String },
    Step { step: String, pointer: String },
    Item { item: String },
}

#[derive(Debug, thiserror::Error)]
#[error("{path}: {message}")]
pub struct CoreError {
    pub path: String,
    pub message: String,
}

/// Safe, bounded repair guidance for authoring clients. This deliberately
/// exposes an allowlisted field and stable reason instead of schema text or
/// the original manifest value (which may contain private paths or payloads).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionDiagnostic {
    pub field: String,
    pub reason: String,
    pub repair: String,
}

impl CoreError {
    pub fn diagnostic(&self) -> Option<AdmissionDiagnostic> {
        let message = self.message.to_ascii_lowercase();
        if message.contains("trusted python requires") {
            return Some(AdmissionDiagnostic {
                field: "steps[].script".into(),
                reason: "invalid_python_step_contract".into(),
                repair: "Use kind=script, effect=write, retry.max_attempts=1, a bundled entrypoint and result_schema. Declare capabilities:[{kind:script,language:python}]; language belongs on the capability, not the step.".into(),
            });
        }
        if message.contains("script requires python capability") {
            return Some(AdmissionDiagnostic {
                field: "capabilities[]".into(),
                reason: "missing_python_capability".into(),
                repair: "Declare the matching script capability with language=python and bundle the referenced source file.".into(),
            });
        }
        if message.contains("unsupported builtin operation") {
            return Some(AdmissionDiagnostic {
                field: "steps[].operation".into(),
                reason: "unsupported_builtin".into(),
                repair: "Use an operation listed by the installed schema/authoring guide, or move deterministic logic into an admitted trusted Python step.".into(),
            });
        }
        if message.contains("connection requires matching capability") {
            return Some(AdmissionDiagnostic {
                field: "capabilities[]".into(),
                reason: "missing_connection_capability".into(),
                repair: "Declare a connection capability with the exact connector name used by the step.".into(),
            });
        }
        let field = match self.path.split('/').nth(1) {
            Some("schema_version") => "schema_version",
            Some("name") => "name",
            Some("description") => "description",
            Some("input_schema") => "input_schema",
            Some("output_schema") => "output_schema",
            Some("config") => "config",
            Some("capabilities") => "capabilities[]",
            Some("limits") => "limits",
            Some("steps") => "steps[]",
            Some("output") => "output",
            _ => return None,
        };
        Some(AdmissionDiagnostic {
            field: field.into(),
            reason: "manifest_admission_failed".into(),
            repair: "Read the installed schema and correct this manifest field, then run process.validate again.".into(),
        })
    }
}

impl CoreError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Sort object keys recursively. Arrays retain their order; strings and numbers
/// use the pinned serde_json representation. No Unicode/number normalization is
/// performed, so 1 and 1.0 deliberately have different identities.
pub fn canonical_bytes(value: &Value) -> Result<Vec<u8>, CoreError> {
    fn normalize(value: &Value, depth: usize) -> Result<Value, CoreError> {
        if depth > 64 {
            return Err(CoreError::new("/", "JSON exceeds maximum depth of 64"));
        }
        match value {
            Value::Object(map) => {
                let sorted: BTreeMap<_, _> = map.iter().collect();
                let mut result = serde_json::Map::new();
                for (key, value) in sorted {
                    result.insert(key.clone(), normalize(value, depth + 1)?);
                }
                Ok(Value::Object(result))
            }
            Value::Array(values) => values
                .iter()
                .map(|v| normalize(v, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            _ => Ok(value.clone()),
        }
    }
    serde_json::to_vec(&normalize(value, 0)?)
        .map_err(|error| CoreError::new("/", error.to_string()))
}

/// Content identity includes the canonical-format domain separator.
pub fn content_digest(value: &Value) -> Result<String, CoreError> {
    let mut hash = Sha256::new();
    hash.update(CANONICAL_FORMAT.as_bytes());
    hash.update([0]);
    hash.update(canonical_bytes(value)?);
    Ok(format!("sha256:{:x}", hash.finalize()))
}

/// Parse source JSON without the ambiguous last-key-wins behavior of most parsers.
pub fn admit_json(bytes: &[u8]) -> Result<AdmittedProcess, CoreError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(CoreError::new("/", "manifest exceeds 1 MiB"));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = UniqueValue::deserialize(&mut deserializer)
        .map_err(|error| CoreError::new("/", error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| CoreError::new("/", error.to_string()))?;
    admit_value(&value.0)
}

pub fn admit_value(value: &Value) -> Result<AdmittedProcess, CoreError> {
    let canonical_bytes = canonical_bytes(value)?;
    if canonical_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(CoreError::new("/", "manifest exceeds 1 MiB"));
    }
    let validator = if value["schema_version"] == "0.2" {
        &ROUTING_MANIFEST_VALIDATOR
    } else {
        &MANIFEST_VALIDATOR
    };
    validator
        .validate(value)
        .map_err(|error| CoreError::new(error.instance_path.to_string(), error.to_string()))?;
    let manifest: ProcessManifest = serde_json::from_value(value.clone())
        .map_err(|error| CoreError::new("/", error.to_string()))?;
    validate_graph(&manifest)?;
    validate_supported(&manifest)?;
    if manifest.schema_version == "0.2" {
        let nodes: Vec<_> = manifest
            .steps
            .iter()
            .map(|step| routing::RouteNode {
                id: &step.id,
                kind: &step.kind,
                needs: &step.needs,
                inputs: &step.inputs,
                foreach_items: step.foreach.as_ref().map(|each| &each.items),
                when: step.when.as_ref(),
                decision: step.decision.as_ref(),
                join: step.join.as_ref(),
                call: step.call.as_ref(),
            })
            .collect();
        routing::validate_routing_graph(&nodes, &manifest.output)?;
        // Every definition has a saved row, including skipped branches. Only
        // the selected route of each decision creates its foreach item rows.
        let mut upper = manifest.steps.len();
        let mut choices: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        for step in &manifest.steps {
            let Some(each) = &step.foreach else {
                continue;
            };
            if let Some(when) = &step.when {
                *choices
                    .entry(&when.decision)
                    .or_default()
                    .entry(&when.route)
                    .or_default() += each.max_items as usize;
            } else {
                upper += each.max_items as usize;
            }
        }
        upper += choices
            .values()
            .map(|routes| routes.values().copied().max().unwrap_or(0))
            .sum::<usize>();
        if upper > manifest.limits.max_step_instances as usize {
            return Err(CoreError::new(
                "/limits/max_step_instances",
                "declared expansion exceeds the step instance budget",
            ));
        }
    }
    compile_schema(&manifest.input_schema, "/input_schema")?;
    compile_schema(&manifest.output_schema, "/output_schema")?;
    let config_validator = compile_schema(&manifest.config.schema, "/config/schema")?;
    config_validator
        .validate(&manifest.config.defaults)
        .map_err(|error| {
            CoreError::new(
                format!("/config/defaults{}", error.instance_path),
                error.to_string(),
            )
        })?;
    for (index, step) in manifest.steps.iter().enumerate() {
        if let Some(schema) = &step.result_schema {
            compile_schema(schema, &format!("/steps/{index}/result_schema"))?;
        }
    }
    Ok(AdmittedProcess {
        manifest,
        version: content_digest(value)?,
        canonical_bytes,
    })
}

fn validate_graph(manifest: &ProcessManifest) -> Result<(), CoreError> {
    let mut ids = BTreeMap::new();
    for (index, step) in manifest.steps.iter().enumerate() {
        if ids.insert(step.id.as_str(), index).is_some() {
            return Err(CoreError::new(
                format!("/steps/{index}/id"),
                "duplicate step ID",
            ));
        }
    }
    fn ancestors<'a>(
        index: usize,
        manifest: &'a ProcessManifest,
        ids: &BTreeMap<&str, usize>,
        visiting: &mut BTreeSet<usize>,
        done: &mut BTreeMap<usize, BTreeSet<&'a str>>,
    ) -> Result<BTreeSet<&'a str>, CoreError> {
        if let Some(found) = done.get(&index) {
            return Ok(found.clone());
        }
        if !visiting.insert(index) {
            return Err(CoreError::new(
                format!("/steps/{index}/needs"),
                "dependency cycle",
            ));
        }
        let mut found = BTreeSet::new();
        for needed in &manifest.steps[index].needs {
            let next = ids.get(needed.as_str()).ok_or_else(|| {
                CoreError::new(
                    format!("/steps/{index}/needs"),
                    format!("unknown step {needed}"),
                )
            })?;
            found.insert(needed.as_str());
            found.extend(ancestors(*next, manifest, ids, visiting, done)?);
        }
        visiting.remove(&index);
        done.insert(index, found.clone());
        Ok(found)
    }
    let mut done = BTreeMap::new();
    for (index, step) in manifest.steps.iter().enumerate() {
        let available = ancestors(index, manifest, &ids, &mut BTreeSet::new(), &mut done)?;
        for (name, binding) in &step.inputs {
            if let Binding::Step { step, .. } = binding
                && !available.contains(step.as_str())
            {
                return Err(CoreError::new(
                    format!("/steps/{index}/inputs/{}", escape(name)),
                    format!("step {step} is not a declared ancestor"),
                ));
            }
        }
        if let Some(expansion) = &step.foreach
            && let Binding::Step { step: source, .. } = &expansion.items
            && !available.contains(source.as_str())
        {
            return Err(CoreError::new(
                format!("/steps/{index}/foreach/items"),
                format!("step {source} is not a declared ancestor"),
            ));
        }
    }
    for (name, binding) in &manifest.output {
        if let Binding::Step { step, .. } = binding
            && !ids.contains_key(step.as_str())
        {
            return Err(CoreError::new(
                format!("/output/{}", escape(name)),
                format!("unknown step {step}"),
            ));
        }
    }
    Ok(())
}

fn escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn validate_supported(manifest: &ProcessManifest) -> Result<(), CoreError> {
    for key in &manifest.config.run_override_keys {
        if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
            return Err(CoreError::new(
                "/config/run_override_keys",
                "override keys must be top-level property names of 1–128 bytes without control characters",
            ));
        }
    }
    let mut files = BTreeSet::new();
    for file in manifest.files.as_deref().unwrap_or(&[]) {
        let path = std::path::Path::new(&file.path);
        if file.path.len() > 256
            || !file.path.ends_with(".py")
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || !files.insert(file.path.as_str())
        {
            return Err(CoreError::new(
                "/files",
                "source files must be unique relative Python paths",
            ));
        }
    }
    for (index, capability) in manifest.capabilities.iter().enumerate() {
        if !matches!(
            capability["kind"].as_str(),
            Some("builtin" | "artifacts" | "agent" | "script" | "connection")
        ) {
            return Err(CoreError::new(
                format!("/capabilities/{index}"),
                "unsupported capability kind",
            ));
        }
    }
    let mut used_files = BTreeSet::new();
    let mut agent_count = 0;
    for (index, step) in manifest.steps.iter().enumerate() {
        let path = format!("/steps/{index}");
        if (step.entrypoint.is_some() && step.kind != "script")
            || (step.connection.is_some() && step.kind != "connector")
        {
            return Err(CoreError::new(
                &path,
                "entrypoint and connection belong only to their execution kind",
            ));
        }
        match step.kind.as_str() {
            "decision" | "join" | "call" => {
                let effect = if step.kind == "call" {
                    "control"
                } else {
                    "pure"
                };
                if manifest.schema_version != "0.2"
                    || step.effect != effect
                    || step.retry.max_attempts != 1
                    || step.operation.is_some()
                    || step.capability.is_some()
                    || step.instructions.is_some()
                    || step.result_schema.is_some()
                {
                    return Err(CoreError::new(
                        &path,
                        "routing steps require version 0.2, their declared control/pure effect and one attempt without executor fields",
                    ));
                }
            }
            "builtin" => {
                if step.effect != "pure" {
                    return Err(CoreError::new(
                        format!("{path}/effect"),
                        "M0 builtins must declare pure effects",
                    ));
                }
                if !matches!(
                    step.operation.as_deref(),
                    Some(
                        "json.write"
                            | "json.select"
                            | "classifier.collect"
                            | "classifier.review"
                            | "classifier.candidate"
                            | "classifier.evaluation_plan"
                            | "classifier.evaluate"
                            | "artifact.manifest"
                            | "artifact.validate_set"
                            | "artifact.bundle"
                    )
                ) {
                    return Err(CoreError::new(
                        format!("{path}/operation"),
                        "unsupported builtin operation",
                    ));
                }
                if step.capability.is_some()
                    || step.instructions.is_some()
                    || step.result_schema.is_some()
                {
                    return Err(CoreError::new(
                        &path,
                        "agent fields are not supported on builtins",
                    ));
                }
            }
            "agent" => {
                if step.effect != "agent" || step.retry.max_attempts != 1 {
                    return Err(CoreError::new(
                        &path,
                        "agent steps require effect agent and exactly one attempt",
                    ));
                }
                if step.operation.is_some() {
                    return Err(CoreError::new(
                        format!("{path}/operation"),
                        "agent steps cannot declare an operation",
                    ));
                }
                agent_count += step
                    .foreach
                    .as_ref()
                    .map(|expansion| expansion.max_items)
                    .unwrap_or(1);
            }
            "script" => {
                let entrypoint = step
                    .entrypoint
                    .as_deref()
                    .ok_or_else(|| CoreError::new(&path, "script requires entrypoint"))?;
                if !files.contains(entrypoint)
                    || step.effect != "write"
                    || step.retry.max_attempts != 1
                    || step.result_schema.is_none()
                    || step.operation.is_some()
                    || step.instructions.is_some()
                    || step.capability.is_some()
                {
                    return Err(CoreError::new(
                        &path,
                        "trusted Python requires a declared source, write effect, one attempt and result schema",
                    ));
                }
                if !manifest
                    .capabilities
                    .iter()
                    .any(|c| c["kind"] == "script" && c["language"] == "python")
                {
                    return Err(CoreError::new(
                        &path,
                        "script requires Python capability declaration",
                    ));
                }
                used_files.insert(entrypoint);
            }
            "connector" => {
                let connection = step
                    .connection
                    .as_deref()
                    .ok_or_else(|| CoreError::new(&path, "connector requires connection"))?;
                if connection.is_empty()
                    || connection.len() > 64
                    || !connection
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
                    || !matches!(step.effect.as_str(), "read" | "write")
                    || step.retry.max_attempts != 1
                    || step.result_schema.is_none()
                    || step.instructions.is_some()
                    || step.capability.is_some()
                    || !matches!(
                        step.operation.as_deref(),
                        Some("http.json" | "jev.evaluate" | "mcp.call")
                    )
                {
                    return Err(CoreError::new(
                        &path,
                        "connector requires a named JSON operation, one attempt and result schema",
                    ));
                }
                if !manifest
                    .capabilities
                    .iter()
                    .any(|c| c["kind"] == "connection" && c["name"] == connection)
                {
                    return Err(CoreError::new(
                        &path,
                        "connection requires matching capability declaration",
                    ));
                }
                if manifest.limits.max_provider_calls == 0 {
                    return Err(CoreError::new(
                        &path,
                        "connector requires provider call budget",
                    ));
                }
            }
            _ => {
                return Err(CoreError::new(
                    format!("{path}/kind"),
                    "unsupported step kind",
                ));
            }
        }
        if let Some(expansion) = &step.foreach {
            validate_binding(&expansion.items, &format!("{path}/foreach/items"))?;
            if matches!(expansion.items, Binding::Item { .. }) {
                return Err(CoreError::new(
                    format!("{path}/foreach/items"),
                    "nested item expansion is unsupported",
                ));
            }
            validate_pointer(&expansion.key, &format!("{path}/foreach/key"))?;
            if expansion.concurrency > expansion.max_items {
                return Err(CoreError::new(
                    format!("{path}/foreach/concurrency"),
                    "concurrency cannot exceed max_items",
                ));
            }
        }
        for (name, binding) in &step.inputs {
            if matches!(binding, Binding::Item { .. }) && step.foreach.is_none() {
                return Err(CoreError::new(
                    format!("{path}/inputs/{}", escape(name)),
                    "item binding requires this step's foreach context",
                ));
            }
            validate_binding(binding, &format!("{path}/inputs/{}", escape(name)))?;
        }
    }
    if files != used_files {
        return Err(CoreError::new(
            "/files",
            "each source file must be a script entrypoint; auxiliary imports are unsupported",
        ));
    }
    if agent_count > manifest.limits.max_agent_requests {
        return Err(CoreError::new(
            "/limits/max_agent_requests",
            "budget is smaller than the number of agent steps",
        ));
    }
    for (name, binding) in &manifest.output {
        if matches!(binding, Binding::Item { .. }) {
            return Err(CoreError::new(
                format!("/output/{}", escape(name)),
                "item binding is not valid in final output",
            ));
        }
        validate_binding(binding, &format!("/output/{}", escape(name)))?;
    }
    Ok(())
}

fn validate_binding(binding: &Binding, path: &str) -> Result<(), CoreError> {
    match binding {
        Binding::Item { item } => validate_pointer(item, path),
        Binding::Input { input } => validate_pointer(input, path),
        Binding::Config { config } => validate_pointer(config, path),
        Binding::Step { pointer, .. } => validate_pointer(pointer, path),
        Binding::Literal { .. } => Ok(()),
    }
}

fn validate_pointer(pointer: &str, path: &str) -> Result<(), CoreError> {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return Err(CoreError::new(
            path,
            "JSON pointer must be empty or start with /",
        ));
    }
    let mut chars = pointer.chars();
    while let Some(character) = chars.next() {
        if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return Err(CoreError::new(path, "invalid JSON pointer escape"));
        }
    }
    Ok(())
}

/// Resolve explicit data bindings. There is no expression evaluation or ambient
/// configuration/environment lookup, and absent data never becomes JSON null.
pub fn resolve_bindings(
    bindings: &BTreeMap<String, Binding>,
    input: &Value,
    outputs: &BTreeMap<String, Value>,
) -> Result<Value, CoreError> {
    resolve_bindings_with_context(
        bindings,
        input,
        outputs,
        &Value::Object(Default::default()),
        None,
    )
}

/// Resolve bindings from explicitly supplied, frozen run data. Configuration is
/// ordinary JSON, never an ambient environment or credential lookup.
pub fn resolve_bindings_with_context(
    bindings: &BTreeMap<String, Binding>,
    input: &Value,
    outputs: &BTreeMap<String, Value>,
    config: &Value,
    item: Option<&Value>,
) -> Result<Value, CoreError> {
    let mut resolved = serde_json::Map::new();
    for (name, binding) in bindings {
        let path = format!("/bindings/{}", escape(name));
        validate_binding(binding, &path)?;
        let value = match binding {
            Binding::Literal { literal } => literal,
            Binding::Input { input: pointer } => input.pointer(pointer).ok_or_else(|| {
                CoreError::new(&path, format!("input pointer {pointer:?} does not exist"))
            })?,
            Binding::Config { config: pointer } => config.pointer(pointer).ok_or_else(|| {
                CoreError::new(&path, format!("config pointer {pointer:?} does not exist"))
            })?,
            Binding::Step { step, pointer } => outputs
                .get(step)
                .ok_or_else(|| {
                    CoreError::new(&path, format!("step {step} has no committed output"))
                })?
                .pointer(pointer)
                .ok_or_else(|| {
                    CoreError::new(
                        &path,
                        format!("step {step} pointer {pointer:?} does not exist"),
                    )
                })?,
            Binding::Item { item: pointer } => {
                item.and_then(|item| item.pointer(pointer)).ok_or_else(|| {
                    CoreError::new(
                        &path,
                        format!("item context or pointer {pointer:?} is unavailable"),
                    )
                })?
            }
        };
        resolved.insert(name.clone(), value.clone());
    }
    Ok(Value::Object(resolved))
}

/// Validate Draft 2020-12 data without filesystem or network reference lookup.
pub fn validate_instance(schema: &Value, instance: &Value) -> Result<(), CoreError> {
    canonical_bytes(instance)?; // Bound nesting before entering recursive validators.
    compile_schema(schema, "/schema")?
        .validate(instance)
        .map_err(|error| CoreError::new(error.instance_path.to_string(), error.to_string()))
}

/// Admit a standalone schema under APR's local-only, bounded schema policy.
pub fn validate_schema(schema: &Value) -> Result<(), CoreError> {
    compile_schema(schema, "/schema").map(|_| ())
}

/// Admit a provider's service-tool schema under the same bounded, local-only
/// policy as process schemas. Draft 7 requires an explicit root `$schema`;
/// an absent dialect defaults to Draft 2020-12.
pub fn validate_service_schema(schema: &Value) -> Result<(), CoreError> {
    compile_service_schema(schema, "/schema").map(|_| ())
}

/// Validate service-tool data using the schema's declared Draft 7 or 2020-12
/// semantics, without permitting remote schema retrieval.
pub fn validate_service_instance(schema: &Value, instance: &Value) -> Result<(), CoreError> {
    canonical_bytes(instance)?;
    compile_service_schema(schema, "/schema")?
        .validate(instance)
        .map_err(|error| CoreError::new(error.instance_path.to_string(), error.to_string()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SchemaDialect {
    Draft7,
    Draft202012,
}

impl SchemaDialect {
    fn recognizes(self, uri: &str) -> bool {
        match self {
            Self::Draft7 => uri == "http://json-schema.org/draft-07/schema#",
            Self::Draft202012 => uri == "https://json-schema.org/draft/2020-12/schema",
        }
    }
}

struct LocalOnly;

impl jsonschema::Retrieve for LocalOnly {
    fn retrieve(
        &self,
        _uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("external schema references are disabled".into())
    }
}

fn compile_schema(schema: &Value, path: &str) -> Result<jsonschema::Validator, CoreError> {
    compile_schema_with_dialect(schema, path, SchemaDialect::Draft202012)
}

fn compile_service_schema(schema: &Value, path: &str) -> Result<jsonschema::Validator, CoreError> {
    let dialect = match schema.get("$schema").and_then(Value::as_str) {
        None if schema.get("$schema").is_none() => SchemaDialect::Draft202012,
        Some("https://json-schema.org/draft/2020-12/schema") => SchemaDialect::Draft202012,
        Some("http://json-schema.org/draft-07/schema#") => SchemaDialect::Draft7,
        _ => {
            return Err(CoreError::new(
                format!("{path}/$schema"),
                "unsupported service schema dialect",
            ));
        }
    };
    compile_schema_with_dialect(schema, path, dialect)
}

fn compile_schema_with_dialect(
    schema: &Value,
    path: &str,
    dialect: SchemaDialect,
) -> Result<jsonschema::Validator, CoreError> {
    canonical_bytes(schema)?;
    check_schema_references(schema, path, dialect)?;
    let meta_result = match dialect {
        SchemaDialect::Draft7 => jsonschema::draft7::meta::validate(schema),
        SchemaDialect::Draft202012 => jsonschema::draft202012::meta::validate(schema),
    };
    meta_result.map_err(|error| {
        CoreError::new(format!("{path}{}", error.instance_path), error.to_string())
    })?;
    jsonschema::options()
        .with_draft(match dialect {
            SchemaDialect::Draft7 => jsonschema::Draft::Draft7,
            SchemaDialect::Draft202012 => jsonschema::Draft::Draft202012,
        })
        .with_retriever(LocalOnly)
        .with_pattern_options(jsonschema::PatternOptions::regex())
        .should_validate_formats(true)
        .build(schema)
        .map_err(|error| {
            CoreError::new(format!("{path}{}", error.instance_path), error.to_string())
        })
}

fn check_schema_references(
    schema: &Value,
    path: &str,
    dialect: SchemaDialect,
) -> Result<(), CoreError> {
    walk_schema(
        schema,
        schema,
        path,
        &mut BTreeSet::new(),
        &mut BTreeMap::new(),
        0,
        dialect,
    )
    .map(|_| ())
}

fn walk_schema(
    root: &Value,
    schema: &Value,
    path: &str,
    active: &mut BTreeSet<usize>,
    done: &mut BTreeMap<usize, usize>,
    depth: usize,
    dialect: SchemaDialect,
) -> Result<usize, CoreError> {
    if depth > 64 {
        return Err(CoreError::new(
            path,
            "schema reference expansion exceeds depth 64",
        ));
    }
    let identity = std::ptr::from_ref(schema) as usize;
    if let Some(cost) = done.get(&identity) {
        return Ok(*cost);
    }
    if !active.insert(identity) {
        return Err(CoreError::new(
            path,
            "recursive schema references are unsupported in M0",
        ));
    }
    let Some(object) = schema.as_object() else {
        active.remove(&identity);
        done.insert(identity, 1);
        return Ok(1);
    };
    let mut cost = 1;
    for keyword in ["$dynamicRef", "$recursiveRef"] {
        if object.contains_key(keyword) {
            return Err(CoreError::new(
                format!("{path}/{keyword}"),
                "dynamic and recursive schema references are unsupported in M0",
            ));
        }
    }
    if object.contains_key("$id") && !std::ptr::eq(root, schema) {
        return Err(CoreError::new(
            format!("{path}/$id"),
            "nested schema resources are unsupported in M0",
        ));
    }
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .filter(|pointer| {
                (pointer.is_empty() || pointer.starts_with('/')) && !pointer.contains('%')
            })
            .ok_or_else(|| {
                CoreError::new(
                    format!("{path}/$ref"),
                    "only local JSON pointer references without URI escapes are supported",
                )
            })?;
        validate_pointer(pointer, &format!("{path}/$ref"))?;
        let target = root.pointer(pointer).ok_or_else(|| {
            CoreError::new(
                format!("{path}/$ref"),
                "schema reference target does not exist",
            )
        })?;
        add_schema_cost(
            &mut cost,
            walk_schema(
                root,
                target,
                &format!("{path}/$ref->{pointer}"),
                active,
                done,
                depth + 1,
                dialect,
            )?,
            path,
        )?;
    }
    if let Some(uri) = object.get("$schema")
        && !uri.as_str().is_some_and(|uri| dialect.recognizes(uri))
    {
        return Err(CoreError::new(
            format!("{path}/$schema"),
            match dialect {
                SchemaDialect::Draft7 => "nested or unsupported schema dialect",
                SchemaDialect::Draft202012 => "only Draft 2020-12 is supported",
            },
        ));
    }
    // Only descend into schema positions: const/default/enum may contain arbitrary
    // task data with keys that happen to look like schema keywords.
    for keyword in [
        "$defs",
        "definitions",
        "properties",
        "patternProperties",
        "dependentSchemas",
    ] {
        if let Some(entries) = object.get(keyword).and_then(Value::as_object) {
            for (name, schema) in entries {
                add_schema_cost(
                    &mut cost,
                    walk_schema(
                        root,
                        schema,
                        &format!("{path}/{keyword}/{}", escape(name)),
                        active,
                        done,
                        depth + 1,
                        dialect,
                    )?,
                    path,
                )?;
            }
        }
    }
    if dialect == SchemaDialect::Draft7
        && let Some(dependencies) = object.get("dependencies").and_then(Value::as_object)
    {
        for (name, dependency) in dependencies {
            // Property-name lists are data; object and boolean dependencies are schemas.
            if dependency.is_object() || dependency.is_boolean() {
                add_schema_cost(
                    &mut cost,
                    walk_schema(
                        root,
                        dependency,
                        &format!("{path}/dependencies/{}", escape(name)),
                        active,
                        done,
                        depth + 1,
                        dialect,
                    )?,
                    path,
                )?;
            }
        }
    }
    for keyword in [
        "additionalProperties",
        "unevaluatedProperties",
        "unevaluatedItems",
        "contains",
        "propertyNames",
        "not",
        "if",
        "then",
        "else",
        "contentSchema",
    ] {
        if let Some(schema) = object.get(keyword) {
            add_schema_cost(
                &mut cost,
                walk_schema(
                    root,
                    schema,
                    &format!("{path}/{keyword}"),
                    active,
                    done,
                    depth + 1,
                    dialect,
                )?,
                path,
            )?;
        }
    }
    for keyword in ["items", "additionalItems"] {
        if keyword == "additionalItems" && dialect != SchemaDialect::Draft7 {
            continue;
        }
        if let Some(item) = object.get(keyword)
            && !item.is_array()
        {
            add_schema_cost(
                &mut cost,
                walk_schema(
                    root,
                    item,
                    &format!("{path}/{keyword}"),
                    active,
                    done,
                    depth + 1,
                    dialect,
                )?,
                path,
            )?;
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems", "items"] {
        if keyword == "items" && dialect != SchemaDialect::Draft7 {
            continue;
        }
        if let Some(schemas) = object.get(keyword).and_then(Value::as_array) {
            for (index, schema) in schemas.iter().enumerate() {
                add_schema_cost(
                    &mut cost,
                    walk_schema(
                        root,
                        schema,
                        &format!("{path}/{keyword}/{index}"),
                        active,
                        done,
                        depth + 1,
                        dialect,
                    )?,
                    path,
                )?;
            }
        }
    }
    active.remove(&identity);
    done.insert(identity, cost);
    Ok(cost)
}

fn add_schema_cost(total: &mut usize, cost: usize, path: &str) -> Result<(), CoreError> {
    *total = total.saturating_add(cost);
    if *total > 4096 {
        return Err(CoreError::new(path, "schema expansion exceeds 4096 nodes"));
    }
    Ok(())
}

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> serde::de::Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|value| UniqueValue(Value::Number(value)))
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate JSON object key {key:?}"
                        )));
                    }
                    values.insert(key, map.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}
