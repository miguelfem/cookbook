//! Bounded outbound MCP with explicit feature and interaction policies. Remote HTTP and explicitly trusted stdio
//! servers share the broker's credential, destination and uncertainty boundary.
//! No package installation, ambient roots or tool-driven grants.

pub mod interaction;
pub mod sampling;
pub mod subscription;

use crate::{Connection, CredentialStore, DispatchError, DispatchOutput, SafeResolver, Secret};
use apr_core::{content_digest, validate_service_instance, validate_service_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use ureq::http;
use ureq::unversioned::transport::DefaultConnector;

const MAX_TOOLS: usize = 256;
const MAX_PAGES: usize = 16;
const MAX_MESSAGES: usize = 128;
fn legacy() -> String {
    "2025-11-25".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpBinding {
    #[serde(default = "legacy")]
    pub protocol: String,
    pub transport: Transport,
    /// None is allowed only for owner-controlled discovery, never recipe dispatch.
    #[serde(default)]
    pub tool: Option<ToolContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<FeatureContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<interaction::InteractionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling_worker: Option<sampling::SamplingWorker>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transport {
    Http,
    Stdio {
        executable: String,
        sha256: String,
        #[serde(default)]
        args: Vec<String>,
        /// The one explicitly reviewed environment variable receiving the token.
        /// The child is trusted with it; this is NOT protected script execution.
        #[serde(default)]
        credential_env: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolContract {
    pub name: String,
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_resource_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeatureContract {
    Subscription { notifications: subscription::Filter },
    Resource { uri: String },
    Prompt { name: String, arguments: Value },
}

impl McpBinding {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if !matches!(
            self.protocol.as_str(),
            "2025-03-26" | "2025-06-18" | "2025-11-25" | "2026-07-28"
        ) {
            return Err(DispatchError::InvalidConnection);
        }
        if self.tool.is_some() && self.feature.is_some() {
            return Err(DispatchError::InvalidConnection);
        }
        if let Some(worker) = &self.sampling_worker {
            worker.validate()?;
            if self.interaction.as_ref().is_none_or(|p| !p.sampling) {
                return Err(DispatchError::InvalidConnection);
            }
        }
        if let Some(policy) = &self.interaction {
            policy.validate()?;
            if self.protocol != "2026-07-28" {
                return Err(DispatchError::InvalidConnection);
            }
        }
        if let Some(feature) = &self.feature {
            match feature {
                FeatureContract::Subscription { notifications } => {
                    notifications.validate()?;
                    if self.protocol != "2026-07-28" {
                        return Err(DispatchError::InvalidConnection);
                    }
                }
                FeatureContract::Resource { uri }
                    if uri.is_empty() || uri.len() > 4096 || uri.chars().any(char::is_control) =>
                {
                    return Err(DispatchError::InvalidConnection);
                }
                FeatureContract::Prompt { name, arguments }
                    if name.is_empty()
                        || name.len() > 128
                        || !arguments.is_array()
                        || serde_json::to_vec(arguments)
                            .map_err(|_| DispatchError::InvalidConnection)?
                            .len()
                            > 8192 =>
                {
                    return Err(DispatchError::InvalidConnection);
                }
                _ => {}
            }
        }
        if let Some(tool) = &self.tool {
            tool.validate()?;
        }
        if let Transport::Stdio {
            executable,
            sha256,
            args,
            credential_env,
        } = &self.transport
            && (!Path::new(executable).is_absolute()
                || executable.len() > 4096
                || sha256.len() != 64
                || !sha256.bytes().all(|c| c.is_ascii_hexdigit())
                || args.len() > 64
                || args.iter().any(|a| a.len() > 4096 || a.contains('\0'))
                || credential_env.as_ref().is_some_and(|v| !safe_env(v)))
        {
            return Err(DispatchError::InvalidConnection);
        }
        Ok(())
    }
    pub fn is_stdio(&self) -> bool {
        matches!(self.transport, Transport::Stdio { .. })
    }
}
impl ToolContract {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if self.ui_resource_uri.as_ref().is_some_and(|u| {
            !u.starts_with("ui://") || u.len() > 4096 || u.chars().any(char::is_control)
        }) {
            return Err(DispatchError::InvalidConnection);
        }
        if self.name.is_empty()
            || self.name.len() > 128
            || !self
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-/".contains(&c))
        {
            return Err(DispatchError::InvalidConnection);
        }
        for schema in std::iter::once(&self.input_schema).chain(self.output_schema.iter()) {
            if serde_json::to_vec(schema)
                .map_err(|_| DispatchError::InvalidConnection)?
                .len()
                > 16384
            {
                return Err(DispatchError::InvalidConnection);
            }
            validate_service_schema(schema).map_err(|_| DispatchError::InvalidConnection)?;
        }
        Ok(())
    }
    fn from_advertisement(value: &Value) -> Result<Self, DispatchError> {
        let tool = Self {
            name: value
                .get("name")
                .and_then(Value::as_str)
                .ok_or(DispatchError::InvalidConnection)?
                .into(),
            input_schema: value
                .get("inputSchema")
                .cloned()
                .ok_or(DispatchError::InvalidConnection)?,
            output_schema: value.get("outputSchema").cloned(),
            ui_resource_uri: value
                .pointer("/_meta/ui/resourceUri")
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        tool.validate()?;
        Ok(tool)
    }
    fn digest(&self) -> Result<String, DispatchError> {
        content_digest(&serde_json::to_value(self).map_err(|_| DispatchError::InvalidConnection)?)
            .map_err(|_| DispatchError::InvalidConnection)
    }
}

fn safe_env(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        && !name.starts_with("LD_")
        && !name.starts_with("DYLD_")
        && !matches!(
            name,
            "PATH"
                | "HOME"
                | "SHELL"
                | "ENV"
                | "BASH_ENV"
                | "PYTHONPATH"
                | "PYTHONHOME"
                | "NODE_OPTIONS"
                | "RUBYOPT"
                | "PERL5OPT"
        )
}

/// Hash the target without executing, while preserving the approved command path.
/// A virtualenv's Python symlink must remain the spawned path so Python discovers
/// its own site-packages. This digest does not pin installed dependencies.
pub fn pin_executable(path: &Path) -> Result<String, DispatchError> {
    if !path.is_absolute() {
        return Err(DispatchError::InvalidConnection);
    }
    let target = path
        .canonicalize()
        .map_err(|_| DispatchError::InvalidConnection)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file = options
        .open(target)
        .map_err(|_| DispatchError::InvalidConnection)?;
    let meta = file
        .metadata()
        .map_err(|_| DispatchError::InvalidConnection)?;
    if !meta.is_file() || meta.len() > 512 * 1024 * 1024 {
        return Err(DispatchError::InvalidConnection);
    }
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| DispatchError::InvalidConnection)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > 512 * 1024 * 1024 {
            return Err(DispatchError::InvalidConnection);
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn discover<V: CredentialStore>(
    connection: &Connection,
    vault: &V,
) -> Result<Value, DispatchError> {
    let mut session = Session::connect(connection, vault, None)?;
    let tools = session.list_tools()?;
    Ok(
        json!({"protocol":session.binding.protocol,"tools":tools,"tools_only":true,"native_trusted":session.binding.is_stdio()}),
    )
}

/// Resolve an outbound MCP revision during owner-controlled setup. The caller
/// must store the returned concrete revision in the approved binding; dispatch
/// never probes or retries a tool call on another revision.
pub fn negotiate_protocol<V: CredentialStore>(
    connection: &Connection,
    vault: &V,
) -> Result<String, DispatchError> {
    connection.validate()?;
    let negotiation_deadline = Instant::now() + Duration::from_millis(connection.timeout_ms);
    let mut modern_connection = connection.clone();
    modern_connection
        .mcp
        .as_mut()
        .ok_or(DispatchError::InvalidConnection)?
        .protocol = "2026-07-28".into();
    let mut modern = Session::open(&modern_connection, vault, None)?;
    modern.deadline = negotiation_deadline;
    let id = json!(uuid::Uuid::new_v4().to_string());
    let probe = json!({
        "jsonrpc":"2.0", "id":id, "method":"server/discover",
        "params":modern.params(json!({}))
    });
    let response = modern.exchange(&probe, Some(&id));
    let legacy = match response {
        Ok(Some(response)) if valid_probe_reply(&response, &id) => {
            if let Some(result) = response.get("result") {
                let versions = result
                    .get("supportedVersions")
                    .and_then(Value::as_array)
                    .ok_or(DispatchError::InvalidConnection)?;
                if result.get("resultType").and_then(Value::as_str) != Some("complete")
                    || !result.get("capabilities").is_some_and(Value::is_object)
                    || versions.is_empty()
                    || versions.len() > 32
                    || !versions.iter().all(Value::is_string)
                    || !versions.iter().any(|v| v == "2026-07-28")
                {
                    return Err(DispatchError::InvalidConnection);
                }
                return Ok("2026-07-28".into());
            }
            let error = response
                .get("error")
                .filter(|v| v.is_object())
                .ok_or(DispatchError::InvalidConnection)?;
            if error.get("message").and_then(Value::as_str).is_none() {
                return Err(DispatchError::InvalidConnection);
            }
            match error.get("code").and_then(Value::as_i64) {
                Some(-32022) => {
                    let versions = error
                        .pointer("/data/supported")
                        .and_then(Value::as_array)
                        .ok_or(DispatchError::InvalidConnection)?;
                    if versions.is_empty()
                        || versions.len() > 32
                        || !versions.iter().all(Value::is_string)
                    {
                        return Err(DispatchError::InvalidConnection);
                    }
                    // A server advertising only legacy revisions is legacy
                    // evidence. A modern revision we cannot speak is not.
                    if versions.iter().all(|v| {
                        matches!(v.as_str(), Some("2025-11-25" | "2025-06-18" | "2025-03-26"))
                    }) {
                        true
                    } else {
                        return Err(DispatchError::InvalidConnection);
                    }
                }
                Some(_) => true,
                None => return Err(DispatchError::InvalidConnection),
            }
        }
        Err(DispatchError::McpHttpStatus(status))
            if modern.native.is_none()
                && (400..500).contains(&status)
                && !matches!(status, 401 | 403 | 408 | 429) =>
        {
            true
        }
        Err(error) => return Err(error),
        _ => return Err(DispatchError::InvalidConnection),
    };
    if !legacy {
        return Err(DispatchError::InvalidConnection);
    }
    // A fresh process/session prevents a legacy initialize from inheriting
    // state created by a failed modern probe.
    drop(modern);
    let mut legacy_connection = connection.clone();
    legacy_connection
        .mcp
        .as_mut()
        .ok_or(DispatchError::InvalidConnection)?
        .protocol = "2025-11-25".into();
    let mut session = Session::open(&legacy_connection, vault, None)?;
    session.deadline = negotiation_deadline;
    let initialized = session.rpc(
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cookbook-apr","version":"0.1.0"}}),
        false,
    )?;
    let selected = initialized
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|v| matches!(*v, "2025-11-25" | "2025-06-18" | "2025-03-26"))
        .ok_or(DispatchError::InvalidConnection)?;
    if !initialized
        .get("capabilities")
        .is_some_and(Value::is_object)
    {
        return Err(DispatchError::InvalidConnection);
    }
    let selected = selected.to_owned();
    session.protocol_override = Some(selected.clone());
    session.notify("notifications/initialized", json!({}))?;
    Ok(selected)
}

fn valid_probe_reply(response: &Value, id: &Value) -> bool {
    response.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && (response.get("result").is_some() ^ response.get("error").is_some())
        && (response.get("id") == Some(id)
            || (response.get("id") == Some(&Value::Null)
                && response.get("result").is_none()
                && matches!(
                    response.pointer("/error/code").and_then(Value::as_i64),
                    Some(-32700 | -32600 | -32601 | -32602)
                )))
}

pub(crate) fn dispatch<V: CredentialStore>(
    connection: &Connection,
    input: &Value,
    vault: &V,
    cancel: Option<&AtomicBool>,
) -> Result<DispatchOutput, DispatchError> {
    dispatch_continuation(connection, input, vault, cancel, None)
}
pub fn dispatch_continuation<V: CredentialStore>(
    connection: &Connection,
    input: &Value,
    vault: &V,
    cancel: Option<&AtomicBool>,
    continuation: Option<&interaction::Continuation>,
) -> Result<DispatchOutput, DispatchError> {
    connection.validate()?;
    let binding = connection
        .mcp
        .as_ref()
        .ok_or(DispatchError::InvalidConnection)?;
    if continuation.is_some() && binding.interaction.is_none() {
        return Err(DispatchError::InvalidConnection);
    }
    validate_service_instance(&connection.input_schema, input)
        .map_err(|_| DispatchError::InvalidInput)?;
    if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
        return Err(DispatchError::InvalidConnection);
    }
    let start = Instant::now();
    let mut session = Session::connect(connection, vault, cancel)?;
    if let Some(FeatureContract::Subscription { notifications }) = &binding.feature {
        return subscription::window(&mut session, notifications, input);
    }

    let before_error = if session.native.is_some() {
        DispatchError::Uncertain
    } else {
        DispatchError::InvalidConnection
    };
    let (method, mut params) = if let Some(approved) = &binding.tool {
        approved.validate()?;
        validate_service_instance(&approved.input_schema, input).map_err(|_| before_error)?;
        let tools = session.list_tools().map_err(|_| before_error)?;
        if tools
            .iter()
            .find(|t| t.name == approved.name)
            .ok_or(before_error)?
            .digest()?
            != approved.digest()?
        {
            return Err(before_error);
        }
        (
            "tools/call",
            json!({"name":approved.name,"arguments":input}),
        )
    } else {
        match binding.feature.as_ref().ok_or(before_error)? {
            FeatureContract::Subscription { .. } => return Err(DispatchError::InvalidConnection),
            FeatureContract::Resource { uri } => {
                if input != &json!({}) {
                    return Err(before_error);
                }
                ("resources/read", json!({"uri":uri}))
            }
            FeatureContract::Prompt { name, arguments } => {
                let prompts = session.list_collection("prompts/list", "prompts")?;
                let current = prompts
                    .iter()
                    .find(|p| p["name"] == *name)
                    .ok_or(before_error)?;
                let actual = current.get("arguments").cloned().unwrap_or(json!([]));
                if &actual != arguments
                    || !input
                        .as_object()
                        .is_some_and(|m| m.values().all(Value::is_string))
                {
                    return Err(before_error);
                }
                ("prompts/get", json!({"name":name,"arguments":input}))
            }
        }
    };
    if let Some(c) = continuation {
        params["inputResponses"] = c.input_responses.clone();
        if let Some(state) = &c.request_state {
            params["requestState"] = json!(state);
        }
    }
    let output = session.rpc(method, params, true)?;
    if output["resultType"] == "input_required" {
        binding
            .interaction
            .as_ref()
            .ok_or(DispatchError::Uncertain)?
            .validate_request(&output)?;
    } else {
        validate_completed_output(connection, &output)?;
    }
    let size = serde_json::to_vec(&output)
        .map_err(|_| DispatchError::Uncertain)?
        .len();
    Ok(DispatchOutput {
        output,
        status: 200,
        response_bytes: size,
        elapsed_ms: start.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

/// Pure completion checks shared by broker dispatch and observed-result
/// reconciliation. Transport authentication remains the broker's responsibility.
pub fn validate_completed_output(
    connection: &Connection,
    output: &Value,
) -> Result<(), DispatchError> {
    let binding = connection
        .mcp
        .as_ref()
        .ok_or(DispatchError::InvalidConnection)?;
    let result_type = output.get("resultType");
    if !output.is_object()
        || (binding.protocol == "2026-07-28"
            && result_type.and_then(Value::as_str) != Some("complete"))
        || result_type.is_some_and(|value| value.as_str() != Some("complete"))
        || output
            .get("isError")
            .is_some_and(|value| value.as_bool() != Some(false))
    {
        return Err(DispatchError::Uncertain);
    }
    if let Some(approved) = &binding.tool
        && let Some(schema) = &approved.output_schema
    {
        validate_service_instance(schema, &output["structuredContent"])
            .map_err(|_| DispatchError::Uncertain)?;
    }
    if let Some(FeatureContract::Resource { uri }) = &binding.feature
        && !output["contents"].as_array().is_some_and(|items| {
            !items.is_empty()
                && items.iter().all(|item| {
                    item["uri"] == *uri && (item["text"].is_string() || item["blob"].is_string())
                })
        })
    {
        return Err(DispatchError::Uncertain);
    }
    if matches!(&binding.feature, Some(FeatureContract::Prompt { .. }))
        && !output["messages"].is_array()
    {
        return Err(DispatchError::Uncertain);
    }
    validate_service_instance(&connection.output_schema, output)
        .map_err(|_| DispatchError::Uncertain)
}

/// Read-only metadata discovery for owner-selected resources or prompts.
pub fn discover_features<V: CredentialStore>(
    connection: &Connection,
    vault: &V,
    kind: &str,
) -> Result<Value, DispatchError> {
    let mut session = Session::connect(connection, vault, None)?;
    let (method, key) = match kind {
        "resources" => ("resources/list", "resources"),
        "prompts" => ("prompts/list", "prompts"),
        _ => return Err(DispatchError::InvalidInput),
    };
    Ok(json!({"kind":kind,"items":session.list_collection(method,key)?}))
}

struct Session<'a> {
    connection: &'a Connection,
    binding: &'a McpBinding,
    secret: Option<Secret>,
    native: Option<Native>,
    session_id: Option<String>,
    protocol_override: Option<String>,
    deadline: Instant,
    cancel: Option<&'a AtomicBool>,
}
impl<'a> Session<'a> {
    fn connect<V: CredentialStore>(
        connection: &'a Connection,
        vault: &V,
        cancel: Option<&'a AtomicBool>,
    ) -> Result<Self, DispatchError> {
        let mut session = Self::open(connection, vault, cancel)?;
        if session.binding.protocol != "2026-07-28" {
            let initialized = session.rpc("initialize", json!({"protocolVersion":session.binding.protocol,"capabilities":{},"clientInfo":{"name":"cookbook-apr","version":"0.1.0"}}), false)?;
            if initialized.get("protocolVersion").and_then(Value::as_str)
                != Some(session.binding.protocol.as_str())
            {
                return Err(if session.native.is_some() {
                    DispatchError::Uncertain
                } else {
                    DispatchError::InvalidConnection
                });
            }
            session.notify("notifications/initialized", json!({}))?;
        }
        Ok(session)
    }
    fn open<V: CredentialStore>(
        connection: &'a Connection,
        vault: &V,
        cancel: Option<&'a AtomicBool>,
    ) -> Result<Self, DispatchError> {
        connection.validate()?;
        let binding = connection
            .mcp
            .as_ref()
            .ok_or(DispatchError::InvalidConnection)?;
        let secret = crate::oauth::resolve(connection, vault)?;
        let native = if let Transport::Stdio {
            executable,
            sha256,
            args,
            credential_env,
        } = &binding.transport
        {
            if pin_executable(Path::new(executable))? != *sha256 {
                return Err(DispatchError::InvalidConnection);
            }
            Some(Native::spawn(
                executable,
                args,
                credential_env.as_deref(),
                secret.as_ref(),
            )?)
        } else {
            None
        };
        let session = Self {
            connection,
            binding,
            secret,
            native,
            session_id: None,
            protocol_override: None,
            deadline: Instant::now() + Duration::from_millis(connection.timeout_ms),
            cancel,
        };
        Ok(session)
    }
    fn remaining(&self) -> Result<Duration, DispatchError> {
        if self.cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(DispatchError::Uncertain);
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .ok_or(DispatchError::Uncertain)
    }
    fn protocol(&self) -> &str {
        self.protocol_override
            .as_deref()
            .unwrap_or(&self.binding.protocol)
    }
    fn params(&self, mut params: Value) -> Value {
        if self.protocol() == "2026-07-28" {
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/clientInfo":{"name":"cookbook-apr","version":"0.1.0"},
                "io.modelcontextprotocol/clientCapabilities":self.binding.interaction.as_ref().map(|p|p.capabilities()).unwrap_or(json!({}))
            });
        }
        params
    }
    fn rpc(&mut self, method: &str, params: Value, effect: bool) -> Result<Value, DispatchError> {
        let id = json!(uuid::Uuid::new_v4().to_string());
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":self.params(params)});
        let response = self.exchange(&message, Some(&id));
        // Metadata failures on remote transport are before a tool invocation.
        let response = response.map_err(|error| {
            if !effect && self.native.is_none() {
                match error {
                    DispatchError::McpHttpStatus(_) => error,
                    _ => DispatchError::InvalidConnection,
                }
            } else {
                match error {
                    DispatchError::McpHttpStatus(_) => DispatchError::Uncertain,
                    _ => error,
                }
            }
        })?;
        let response = response.ok_or(DispatchError::Uncertain)?;
        if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || response.get("id") != Some(&id)
            || response.get("error").is_some()
        {
            return Err(if effect || self.native.is_some() {
                DispatchError::Uncertain
            } else {
                DispatchError::InvalidConnection
            });
        }
        let result = response
            .get("result")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or(DispatchError::Uncertain)?;
        let result_type = result.get("resultType").and_then(Value::as_str);
        if (self.binding.protocol == "2026-07-28" && result_type != Some("complete"))
            || result_type.is_some_and(|t| t != "complete")
        {
            if result_type != Some("input_required")
                || !matches!(method, "tools/call" | "prompts/get" | "resources/read")
            {
                return Err(DispatchError::Uncertain);
            }
            self.binding
                .interaction
                .as_ref()
                .ok_or(DispatchError::Uncertain)?
                .validate_request(&result)?;
        }
        Ok(result)
    }
    fn notify(&mut self, method: &str, params: Value) -> Result<(), DispatchError> {
        let message = json!({"jsonrpc":"2.0","method":method,"params":params});
        self.exchange(&message, None).map(|_| ())
    }
    fn list_collection(&mut self, method: &str, key: &str) -> Result<Vec<Value>, DispatchError> {
        let mut results = Vec::new();
        let mut cursor = None;
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..MAX_PAGES {
            let p = cursor
                .as_ref()
                .map(|c| json!({"cursor":c}))
                .unwrap_or(json!({}));
            let page = self.rpc(method, p, false)?;
            let items = page[key]
                .as_array()
                .ok_or(DispatchError::InvalidConnection)?;
            if results.len() + items.len() > MAX_TOOLS {
                return Err(DispatchError::InvalidConnection);
            }
            results.extend(items.iter().cloned());
            cursor = page["nextCursor"].as_str().map(str::to_string);
            match &cursor {
                None => return Ok(results),
                Some(c) if c.len() <= 1024 && seen.insert(c.clone()) => {}
                _ => return Err(DispatchError::InvalidConnection),
            }
        }
        Err(DispatchError::InvalidConnection)
    }
    fn list_tools(&mut self) -> Result<Vec<ToolContract>, DispatchError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..MAX_PAGES {
            let params = cursor
                .as_ref()
                .map(|cursor| json!({"cursor":cursor}))
                .unwrap_or(json!({}));
            let page = self.rpc("tools/list", params, false)?;
            for value in page
                .get("tools")
                .and_then(Value::as_array)
                .ok_or(DispatchError::InvalidConnection)?
            {
                let tool = ToolContract::from_advertisement(value)?;
                if tools.iter().any(|t: &ToolContract| t.name == tool.name)
                    || tools.len() >= MAX_TOOLS
                {
                    return Err(DispatchError::InvalidConnection);
                }
                tools.push(tool);
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            match &cursor {
                None => return Ok(tools),
                Some(cursor) if cursor.len() <= 1024 && seen.insert(cursor.clone()) => {}
                _ => return Err(DispatchError::InvalidConnection),
            }
        }
        Err(DispatchError::InvalidConnection)
    }
    fn exchange(
        &mut self,
        message: &Value,
        id: Option<&Value>,
    ) -> Result<Option<Value>, DispatchError> {
        let remaining = self.remaining()?;
        let mut bytes = serde_json::to_vec(message).map_err(|_| DispatchError::InvalidInput)?;
        if bytes.len() > 65536 {
            return Err(DispatchError::InvalidInput);
        }
        let allow_null_probe_error =
            self.protocol() == "2026-07-28" && message["method"] == "server/discover";
        let result = if let Some(native) = &mut self.native {
            bytes.push(b'\n');
            if allow_null_probe_error {
                native.exchange_probe(
                    &bytes,
                    id,
                    self.deadline,
                    self.cancel,
                    self.connection.max_response_bytes,
                )?
            } else {
                native.exchange(
                    &bytes,
                    id,
                    self.deadline,
                    self.cancel,
                    self.connection.max_response_bytes,
                )?
            }
        } else {
            let mut request = http::Request::builder()
                .method("POST")
                .uri(&self.connection.endpoint)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", self.protocol());
            if self.protocol() == "2026-07-28" {
                request = request.header("Mcp-Method", message["method"].as_str().unwrap_or(""));
                if let Some(name) = message.pointer("/params/name").and_then(Value::as_str) {
                    request = request.header("Mcp-Name", name);
                }
            }
            if let Some(session) = &self.session_id {
                request = request.header("Mcp-Session-Id", session);
            }
            if let Some(secret) = &self.secret {
                request = match &self.connection.auth {
                    crate::Auth::Bearer { .. } | crate::Auth::Oauth { .. } => {
                        request.header("Authorization", format!("Bearer {}", secret.text()))
                    }
                    crate::Auth::Header { name, .. } => request.header(name, secret.text()),
                    crate::Auth::None => request,
                };
            }
            let config = ureq::Agent::config_builder()
                .proxy(None)
                .https_only(!self.connection.loopback_test)
                .max_redirects(0)
                .http_status_as_error(false)
                .max_idle_connections(0)
                .timeout_global(Some(remaining))
                .timeout_resolve(Some(remaining))
                .timeout_connect(Some(remaining))
                .timeout_recv_body(Some(remaining))
                .build();
            let agent = ureq::Agent::with_parts(
                config,
                DefaultConnector::default(),
                SafeResolver {
                    loopback: self.connection.loopback_test,
                },
            );
            let mut response = agent
                .run(
                    request
                        .body(bytes)
                        .map_err(|_| DispatchError::InvalidConnection)?,
                )
                .map_err(|_| DispatchError::Uncertain)?;
            if !(200..300).contains(&response.status().as_u16()) {
                let status = response.status().as_u16();
                // Only the read-only modern era probe inspects 4xx bodies.
                // A well-formed JSON-RPC error may be either era; classify it
                // below. Malformed JSON is never a reason to downgrade.
                if (400..500).contains(&status)
                    && !matches!(status, 401 | 403 | 408 | 429)
                    && message["method"] == "server/discover"
                {
                    let kind = response
                        .headers()
                        .get("Content-Type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if kind.split(';').next().unwrap_or("").trim() == "application/json" {
                        let body = response
                            .body_mut()
                            .with_config()
                            .limit(self.connection.max_response_bytes as u64)
                            .read_to_vec()
                            .map_err(|_| DispatchError::InvalidConnection)?;
                        if !body.is_empty() {
                            let reply = serde_json::from_slice(&body)
                                .map_err(|_| DispatchError::InvalidConnection)?;
                            return Ok(Some(reply));
                        }
                    }
                }
                return Err(DispatchError::McpHttpStatus(status));
            }
            if self.protocol() != "2026-07-28"
                && message["method"] == "initialize"
                && let Some(session) = response.headers().get("Mcp-Session-Id")
            {
                let session = session.to_str().map_err(|_| DispatchError::Uncertain)?;
                if session.is_empty()
                    || session.len() > 256
                    || !session.bytes().all(|b| (0x21..=0x7e).contains(&b))
                {
                    return Err(DispatchError::Uncertain);
                }
                self.session_id = Some(session.into());
            }
            if let Some(response_id) = id {
                let content_type = response
                    .headers()
                    .get("Content-Type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                if content_type.split(';').next().unwrap_or("").trim() == "text/event-stream" {
                    Some(read_sse(
                        response.body_mut().as_reader(),
                        response_id,
                        self.connection.max_response_bytes,
                    )?)
                } else if content_type.split(';').next().unwrap_or("").trim() == "application/json"
                {
                    let bytes = response
                        .body_mut()
                        .with_config()
                        .limit(self.connection.max_response_bytes as u64)
                        .read_to_vec()
                        .map_err(|_| DispatchError::Uncertain)?;
                    Some(serde_json::from_slice(&bytes).map_err(|_| DispatchError::Uncertain)?)
                } else {
                    return Err(DispatchError::Uncertain);
                }
            } else {
                if response.status().as_u16() != 202 && response.status().as_u16() != 204 {
                    return Err(DispatchError::Uncertain);
                }
                None
            }
        };
        if let Some(result) = &result
            && (self
                .secret
                .as_ref()
                .is_some_and(|s| crate::contains_secret(result, s.text()))
                || self
                    .session_id
                    .as_ref()
                    .is_some_and(|s| crate::contains_secret(result, s)))
        {
            return Err(DispatchError::Uncertain);
        }
        Ok(result)
    }
}

fn read_sse(mut reader: impl Read, id: &Value, maximum: usize) -> Result<Value, DispatchError> {
    let mut line = Vec::new();
    let mut data = String::new();
    let mut count = 0;
    let mut messages = 0;
    loop {
        let mut byte = [0];
        let n = reader
            .read(&mut byte)
            .map_err(|_| DispatchError::Uncertain)?;
        if n == 0 {
            return Err(DispatchError::Uncertain);
        }
        count += n;
        if count > maximum {
            return Err(DispatchError::Uncertain);
        }
        if byte[0] != b'\n' {
            line.push(byte[0]);
            continue;
        }
        let text = std::str::from_utf8(&line)
            .map_err(|_| DispatchError::Uncertain)?
            .trim_end_matches('\r');
        if text.is_empty() && !data.is_empty() {
            messages += 1;
            if messages > MAX_MESSAGES {
                return Err(DispatchError::Uncertain);
            }
            let value: Value = serde_json::from_str(&data).map_err(|_| DispatchError::Uncertain)?;
            data.clear();
            if value.get("id") == Some(id) && value.get("method").is_none() {
                return Ok(value);
            }
            if value.get("id").is_some() {
                return Err(DispatchError::Uncertain);
            }
        } else if let Some(part) = text.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(part.strip_prefix(' ').unwrap_or(part));
        }
        line.clear();
    }
}

struct Native {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    _directory: tempfile::TempDir,
}
impl Native {
    fn spawn(
        executable: &str,
        args: &[String],
        env: Option<&str>,
        secret: Option<&Secret>,
    ) -> Result<Self, DispatchError> {
        let directory = tempfile::tempdir().map_err(|_| DispatchError::InvalidConnection)?;
        let mut command = Command::new(executable);
        command
            .args(args)
            .env_clear()
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // No inherited API keys, loader flags or caller configuration.
        if let (Some(env), Some(secret)) = (env, secret) {
            command.env(env, secret.text());
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|_| DispatchError::InvalidConnection)?;
        let stdin = child.stdin.take().ok_or(DispatchError::Uncertain)?;
        let stdout = child.stdout.take().ok_or(DispatchError::Uncertain)?;
        let native = Self {
            child,
            stdin,
            stdout,
            _directory: directory,
        };
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            for fd in [native.stdin.as_raw_fd(), native.stdout.as_raw_fd()] {
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags < 0
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                {
                    return Err(DispatchError::Uncertain);
                }
            }
        }
        Ok(native)
    }
    fn exchange(
        &mut self,
        bytes: &[u8],
        id: Option<&Value>,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
        maximum: usize,
    ) -> Result<Option<Value>, DispatchError> {
        self.exchange_inner(bytes, id, deadline, cancel, maximum, false)
    }
    fn exchange_probe(
        &mut self,
        bytes: &[u8],
        id: Option<&Value>,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
        maximum: usize,
    ) -> Result<Option<Value>, DispatchError> {
        self.exchange_inner(bytes, id, deadline, cancel, maximum, true)
    }
    fn exchange_inner(
        &mut self,
        bytes: &[u8],
        id: Option<&Value>,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
        maximum: usize,
        allow_null_probe_error: bool,
    ) -> Result<Option<Value>, DispatchError> {
        let alive = || {
            if Instant::now() >= deadline || cancel.is_some_and(|f| f.load(Ordering::Acquire)) {
                Err(DispatchError::Uncertain)
            } else {
                Ok(())
            }
        };
        let mut written = 0;
        while written < bytes.len() {
            alive()?;
            match self.stdin.write(&bytes[written..]) {
                Ok(0) => return Err(DispatchError::Uncertain),
                Ok(n) => written += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(_) => return Err(DispatchError::Uncertain),
            }
        }
        let Some(id) = id else {
            return Ok(None);
        };
        let mut line = Vec::new();
        let mut total = 0;
        let mut messages = 0;
        loop {
            alive()?;
            let mut byte = [0_u8];
            match self.stdout.read(&mut byte) {
                Ok(0) => return Err(DispatchError::Uncertain),
                Ok(_) => {
                    total += 1;
                    if total > maximum {
                        return Err(DispatchError::Uncertain);
                    }
                    if byte[0] != b'\n' {
                        line.push(byte[0]);
                        continue;
                    }
                    messages += 1;
                    if messages > MAX_MESSAGES {
                        return Err(DispatchError::Uncertain);
                    }
                    let value: Value =
                        serde_json::from_slice(&line).map_err(|_| DispatchError::Uncertain)?;
                    line.clear();
                    if (value.get("id") == Some(id)
                        || (allow_null_probe_error
                            && value.get("id") == Some(&Value::Null)
                            && matches!(
                                value.pointer("/error/code").and_then(Value::as_i64),
                                Some(-32700 | -32600 | -32601 | -32602)
                            )))
                        && value.get("method").is_none()
                    {
                        return Ok(Some(value));
                    }
                    if value.get("id").is_some() {
                        return Err(DispatchError::Uncertain);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(_) => return Err(DispatchError::Uncertain),
            }
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sse_reads_matching_result_and_ignores_notifications() {
        let data = b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\"result\":{\"content\":[]}}\n\n";
        assert_eq!(read_sse(&data[..], &json!(4), 65536).unwrap()["id"], 4);
        assert!(read_sse(&data[..], &json!(4), 8).is_err());
    }
    #[test]
    fn loader_environment_is_not_a_credential_slot() {
        for name in [
            "PATH",
            "LD_PRELOAD",
            "NODE_OPTIONS",
            "DYLD_INSERT_LIBRARIES",
            "PYTHONPATH",
        ] {
            assert!(!safe_env(name));
        }
        assert!(safe_env("SERVICE_API_KEY"));
    }
}
