//! MCP transport adapter for an explicitly attached APR assignment.
//!
//! The independent local service owns runs and permissions. Closing this bridge
//! never cancels a run or fabricates an answer to a caller handoff.

mod guides;
mod schema;

use apr_runtime::agent_projects::ProjectBroker;
use std::error::Error;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use apr_runtime::service::ServiceClient;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, ReadBuf};

/// Schema-only representation: deserialization remains serde_json::Value.
/// Some MCP clients lower boolean schemas (`true`) to strings. Explicit JSON
/// types and recursive references preserve raw objects, arrays and primitives.
struct JsonValueSchema;

impl JsonSchema for JsonValueSchema {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "AprJsonValue".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let child = generator.subschema_for::<Self>();
        schemars::json_schema!({
            "type": ["null", "boolean", "number", "string", "array", "object"],
            "items": child,
            "properties": {},
            "additionalProperties": child
        })
    }
}

const MAX_MCP_LINE_BYTES: usize = 1024 * 1024;
const MAX_ACTION_BYTES: usize = 64 * 1024;
const DEFAULT_RESULT_BYTES: usize = 8 * 1024;
const DETAIL_RESULT_BYTES: usize = 64 * 1024;
// Reserve room for the MCP tool-result wrapper, without duplicating the JSON.
const RESULT_WRAPPER_BYTES: usize = 256;
const DEFAULT_RUN_WAIT_MS: u64 = 1000;
const MAX_RUN_WAIT_MS: u64 = 5000;

/// Serve MCP over stdin/stdout using a private, validated attachment file.
pub async fn serve(attachment_path: &Path) -> Result<(), Box<dyn Error + Send + Sync>> {
    let client = ServiceClient::from_file(attachment_path)?;
    let bridge = Bridge::new(client);
    let input = LineLimitedReader::new(tokio::io::stdin(), MAX_MCP_LINE_BYTES);
    let connection = bridge.serve((input, tokio::io::stdout())).await?;
    connection.waiting().await?;
    Ok(())
}

/// Serve the explicitly installed user-level connection. Each project is
/// authorized by the client's own filesystem access, never by a path alone.
pub async fn serve_projects(state: &Path) -> Result<(), Box<dyn Error + Send + Sync>> {
    let bridge = Bridge::projects(ProjectBroker::open(state)?);
    let input = LineLimitedReader::new(tokio::io::stdin(), MAX_MCP_LINE_BYTES);
    let connection = bridge.serve((input, tokio::io::stdout())).await?;
    connection.waiting().await?;
    Ok(())
}

#[derive(Clone)]
enum Connection {
    Fixed(Arc<ServiceClient>),
    Projects(Arc<Mutex<ProjectBroker>>),
}

#[derive(Clone)]
struct Bridge {
    connection: Connection,
    tool_router: ToolRouter<Self>,
}

impl Bridge {
    fn new(client: ServiceClient) -> Self {
        let mut tool_router = Self::tool_router();
        compact_tool_schemas(&mut tool_router);
        tool_router.disable_route("apr_project");
        if client.mode() != "author" {
            tool_router.disable_route("apr_manage");
        }
        Self {
            connection: Connection::Fixed(Arc::new(client)),
            tool_router,
        }
    }

    fn projects(broker: ProjectBroker) -> Self {
        Self {
            connection: Connection::Projects(Arc::new(Mutex::new(broker))),
            tool_router: {
                let mut router = Self::tool_router();
                compact_tool_schemas(&mut router);
                require_project_workspaces(&mut router);
                router
            },
        }
    }

    fn client(&self, workspace: Option<&str>) -> Result<ServiceClient, &'static str> {
        match (&self.connection, workspace) {
            (Connection::Fixed(client), None) => Ok((**client).clone()),
            (Connection::Fixed(_), Some(_)) => Err("FORBIDDEN"),
            (Connection::Projects(_), None) => Err("WORKSPACE_REQUIRED"),
            (Connection::Projects(broker), Some(handle)) => broker
                .lock()
                .map_err(|_| "INTERNAL_ERROR")?
                .client(handle)
                .map_err(|_| "UNAUTHORIZED"),
        }
    }

    async fn dispatch(
        &self,
        workspace: Option<&str>,
        action: &'static str,
        args: Value,
        budget: usize,
    ) -> CallToolResult {
        self.dispatch_with_wait(workspace, action, args, budget, 0)
            .await
    }

    async fn dispatch_with_wait(
        &self,
        workspace: Option<&str>,
        action: &'static str,
        args: Value,
        budget: usize,
        wait_ms: u64,
    ) -> CallToolResult {
        match serde_json::to_vec(&args) {
            Ok(bytes) if bytes.len() <= MAX_ACTION_BYTES => {}
            _ => return public_error("LIMIT_EXCEEDED"),
        }
        let client = match self.client(workspace) {
            Ok(client) => client,
            Err(code) => return public_error(code),
        };
        let response = tokio::task::spawn_blocking(move || {
            let value = client.call(action, args, budget - RESULT_WRAPPER_BYTES)?;
            wait_for_run(value, Duration::from_millis(wait_ms), |run_id, view| {
                // Each read rechecks the attachment and assignment. Never read
                // the database here, reissue start, or drive work in the bridge.
                match view {
                    WaitInspection::Run => client.call(
                        "inspect",
                        json!({"view":"run", "run_id":run_id}),
                        budget - RESULT_WRAPPER_BYTES,
                    ),
                    WaitInspection::Tree => client
                        .call(
                            "inspect",
                            json!({"view":"tree_status", "run_id":run_id}),
                            DETAIL_RESULT_BYTES - RESULT_WRAPPER_BYTES,
                        )
                        .or_else(|error| {
                            // A newly installed bridge may briefly meet an older
                            // service. Fall back only for unsupported input; never
                            // conceal failed authorization or transport errors.
                            if error.code() == "INVALID_INPUT" {
                                client.call(
                                    "inspect",
                                    json!({"view":"tree","run_id":run_id,"limit":25}),
                                    DETAIL_RESULT_BYTES - RESULT_WRAPPER_BYTES,
                                )
                            } else {
                                Err(error)
                            }
                        }),
                }
            })
        })
        .await;
        match response {
            Ok(Ok(value)) => {
                let failed = value.get("error").is_some();
                let result = structured_result(value, failed);
                if serde_json::to_vec(&result).is_ok_and(|bytes| bytes.len() <= budget) {
                    result
                } else {
                    // The service must compact whole envelopes while retaining
                    // identity and available_views. Never slice a handoff here.
                    public_error("LIMIT_EXCEEDED")
                }
            }
            Ok(Err(error)) => public_error_for(&error),
            Err(_) => public_error("INTERNAL_ERROR"),
        }
    }
}

fn compact_tool_schemas(router: &mut ToolRouter<Bridge>) {
    for (name, discriminator) in [
        ("apr_inspect", "view"),
        ("apr_resume", "action"),
        ("apr_manage", "action"),
    ] {
        if let Some(route) = router.map.get_mut(name) {
            route.attr.input_schema =
                Arc::new(schema::compact(&route.attr.input_schema, discriminator));
        }
    }
}

fn require_project_workspaces(router: &mut ToolRouter<Bridge>) {
    // Installed guides can be discovered before bootstrap. Every other ordinary
    // project tool already requires a handle at dispatch; advertise that fact.
    for name in ["apr_run", "apr_inspect", "apr_resume", "apr_manage"] {
        if let Some(route) = router.map.get_mut(name) {
            let schema = Arc::make_mut(&mut route.attr.input_schema);
            let required = schema.entry("required").or_insert_with(|| json!([]));
            if let Some(required) = required.as_array_mut()
                && !required.contains(&json!("workspace"))
            {
                required.push(json!("workspace"));
            }
            schema["properties"]["workspace"] = json!({
                "type":"string", "minLength":1,
                "description":"Required handle returned by apr_project connect in this MCP session."
            });
        }
    }
}

/// Wait only after durable acceptance. Individual IPC calls retain their own
/// timeouts; this is a polling budget, not a total tool wall-time guarantee.
/// Work remains owned by the service if the caller disconnects or waiting ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitInspection {
    Run,
    Tree,
}

fn composed_run_needs_attention(tree: &Value) -> bool {
    if tree["view"] == "tree_status" {
        return tree["complete"].as_bool() != Some(true)
            || tree["runnable_without_caller"].as_bool() != Some(true)
            || !tree["total_runs"]
                .as_u64()
                .is_some_and(|n| (1..=4097).contains(&n));
    }
    // This optimization is only for complete, short run families whose saved
    // states can advance without caller action. Unknown or partial projections
    // retain the established behavior of returning the durable root promptly.
    if tree["truncated"].as_bool() != Some(false) {
        return true;
    }
    let Some(runs) = tree["runs"].as_array() else {
        return true;
    };
    runs.is_empty()
        || runs.iter().any(|run| {
            run["actionable_waiting"] == true
                || !matches!(
                    run["state"].as_str(),
                    Some("queued" | "running" | "waiting_for_child" | "succeeded")
                )
        })
}

fn wait_for_run(
    mut value: Value,
    wait: Duration,
    mut inspect: impl FnMut(&str, WaitInspection) -> apr_runtime::Result<Value>,
) -> apr_runtime::Result<Value> {
    let Some(run_id) = value["run_id"].as_str().map(str::to_owned) else {
        return Ok(value);
    };
    let started = Instant::now();
    while matches!(
        value["state"].as_str(),
        Some("queued" | "running" | "waiting_for_child")
    ) {
        let remaining = wait.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        if value["state"] == "waiting_for_child"
            && composed_run_needs_attention(&inspect(&run_id, WaitInspection::Tree)?)
        {
            break;
        }
        std::thread::sleep(
            wait.saturating_sub(started.elapsed())
                .min(Duration::from_millis(25)),
        );
        // Include a final observation, even for a budget shorter than one
        // polling interval. IPC has its own timeout, as with earlier reads.
        // Propagate authorization and transport failures rather than returning
        // a cached result after a failed permission check.
        value = inspect(&run_id, WaitInspection::Run)?;
    }
    Ok(value)
}

#[tool_router]
impl Bridge {
    #[tool(
        name = "apr_project",
        description = "Create or reconnect CookBook in the current project, without user setup. First prepare with the absolute project root. Write the returned proof using your authorized filesystem tool; then connect with challenge_id. Save the returned workspace and include it on every APR call. Never connect an unrelated folder. This grants no credentials or native execution permissions.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn project(&self, Parameters(args): Parameters<ProjectArgs>) -> CallToolResult {
        let Connection::Projects(broker) = &self.connection else {
            return public_error("FORBIDDEN");
        };
        let broker = broker.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut broker = broker
                .lock()
                .map_err(|_| apr_runtime::RuntimeError::State("broker lock".into()))?;
            match args {
                ProjectArgs::Prepare { root, name } => {
                    broker.prepare(Path::new(&root), name.as_deref())
                }
                ProjectArgs::Connect { challenge_id } => broker.connect(&challenge_id),
            }
        })
        .await;
        match result {
            Ok(Ok(value)) => structured_result(value, false),
            Ok(Err(error)) => public_error(error.code()),
            Err(_) => public_error("INTERNAL_ERROR"),
        }
    }

    #[tool(
        name = "apr_discover",
        description = "Discover reusable recipes or installed guidance. Use guide=overview for CookBook's capability map; workflow_design for converting solved work and existing project procedures; authoring for registration and execution. Load only needed contracts: connections for MCP/API/model tools, adapters for CLI/SDK wrappers, jev for optional question/score nodes, routing for branches and parallel batches. Guides report running-service features, not grants or quality. Do not combine guide with recipe search fields.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn discover(
        &self,
        Parameters(Scoped { workspace, args }): Parameters<Scoped<DiscoverArgs>>,
    ) -> CallToolResult {
        if let Some(topic) = args.guide {
            if args.query.is_some()
                || args.limit.is_some()
                || args.process_ref.is_some()
                || args.detail.is_some()
            {
                return public_error("INVALID_INPUT");
            }
            // Keep revocation/expiry checks and report the actual running service,
            // which may predate a newly installed MCP binary.
            let client = match self.client(workspace.as_deref()) {
                Ok(client) => client,
                Err("WORKSPACE_REQUIRED") => {
                    let mut guide =
                        guides::read(topic, &Value::Null, "project_connection_required");
                    guide["connect"] = json!(
                        "Call apr_project prepare for the current project, write its verification file using your authorized filesystem tool, then connect. Include the resulting workspace on subsequent calls."
                    );
                    return structured_result(guide, false);
                }
                Err(code) => return public_error(code),
            };
            let mode = client.mode().to_owned();
            let service = tokio::task::spawn_blocking(move || {
                client.call("service.info", json!({}), DEFAULT_RESULT_BYTES)
            })
            .await;
            return match service {
                Ok(Ok(info)) => {
                    let value = structured_result(guides::read(topic, &info, &mode), false);
                    if serde_json::to_vec(&value)
                        .is_ok_and(|bytes| bytes.len() <= DETAIL_RESULT_BYTES)
                    {
                        value
                    } else {
                        public_error("LIMIT_EXCEEDED")
                    }
                }
                Ok(Err(error)) => public_error(error.code()),
                Err(_) => public_error("INTERNAL_ERROR"),
            };
        }
        if args.limit.is_some_and(|limit| limit == 0 || limit > 20)
            || (args.detail.is_some() && args.process_ref.is_none())
        {
            return public_error("INVALID_INPUT");
        }
        let budget = if args.detail.is_some() {
            DETAIL_RESULT_BYTES
        } else {
            DEFAULT_RESULT_BYTES
        };
        self.dispatch(workspace.as_deref(), "discover", json!(args), budget)
            .await
    }

    #[tool(
        name = "apr_run",
        description = "Preflight or start a registered recipe. Use a known process_ref directly. Start briefly waits for completion and returns output when ready; use that output without another inspection. Long runs return a durable run_id; inspect later with {\"view\":\"run\",\"run_id\":\"...\"}. If truncated=true, inspect with detail=true. wait_ms defaults to 1000 (0 returns immediately; maximum 5000). Reuse the request key when retrying the same start.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn run(
        &self,
        Parameters(Scoped { workspace, args }): Parameters<Scoped<RunArgs>>,
    ) -> CallToolResult {
        if args.wait_ms > MAX_RUN_WAIT_MS {
            return public_error("INVALID_INPUT");
        }
        let wait_ms = if matches!(args.mode, RunMode::Start) {
            args.wait_ms
        } else {
            0
        };
        let result_view = args.result_view;
        let budget = if matches!(result_view, RunResultView::Output) {
            DETAIL_RESULT_BYTES
        } else {
            DEFAULT_RESULT_BYTES
        };
        let result = self
            .dispatch_with_wait(workspace.as_deref(), "run", json!(args), budget, wait_ms)
            .await;
        format_completed_run(result, result_view)
    }

    #[tool(
        name = "apr_inspect",
        description = "Read an authorized run with {\"view\":\"run\",\"run_id\":\"...\"}; view is required for every inspection. Also supports graphs, run trees, pending requests, schedules, events and artifacts. Inspect only if apr_run has not already returned the needed output. Use detail=true for truncated output or a complete bounded handoff, schedule or artifact view.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn inspect(
        &self,
        Parameters(Scoped { workspace, args }): Parameters<Scoped<InspectArgs>>,
    ) -> CallToolResult {
        let mut value = json!(args);
        let detail = value
            .as_object_mut()
            .and_then(|map| map.remove("detail"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let budget = if detail {
            DETAIL_RESULT_BYTES
        } else {
            DEFAULT_RESULT_BYTES
        };
        self.dispatch(workspace.as_deref(), "inspect", value, budget)
            .await
    }

    #[tool(
        name = "apr_resume",
        description = "Submit an observed caller result, reconcile an uncertain effect, pause, continue, retry, renew or cancel a run. Never invent a native tool result; inspect incomplete handoffs with detail=true.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn resume(
        &self,
        Parameters(Scoped { workspace, args }): Parameters<Scoped<ResumeArgs>>,
    ) -> CallToolResult {
        self.dispatch(
            workspace.as_deref(),
            "resume",
            json!(args),
            DEFAULT_RESULT_BYTES,
        )
        .await
    }

    #[tool(
        name = "apr_manage",
        description = "Author mode only: validate, test, register or promote a recipe, request connection approval, preview cost rules, manage configuration and schedules, or import a project-relative artifact. Schedule creation authorizes future runs independently of this client. Set detail=true for a complete bounded response.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn manage(
        &self,
        Parameters(Scoped { workspace, args }): Parameters<Scoped<ManageArgs>>,
    ) -> CallToolResult {
        let mut value = json!(args);
        let detail = value
            .as_object_mut()
            .and_then(|map| map.remove("detail"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        self.dispatch(
            workspace.as_deref(),
            "manage",
            value,
            if detail {
                DETAIL_RESULT_BYTES
            } else {
                DEFAULT_RESULT_BYTES
            },
        )
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        let instructions = match &self.connection {
            Connection::Projects(_) => {
                "Connect the current project with apr_project: prepare, write its exact proof with your authorized filesystem tool, then connect. Include the returned workspace on every APR call; there is no implicit current project. Reuse a known process_ref and input contract directly; unchanged prepared runs need no authoring skill or guide reload. Discover when selecting a recipe or checking an unknown contract. apr_run returns completed output when ready; inspect unfinished or truncated runs with view=run and run_id. Before authoring, load apr_discover guide=overview and the relevant installed guide. Check runtime.features.jev before proposing Jev execution; feature inclusion is not account access or approval. Fulfill saved caller requests only through tools actually available here. Credentials and access approvals remain owner-controlled. Closing MCP does not cancel independent work."
            }
            Connection::Fixed(client) if client.mode() == "author" => {
                "This attachment is bound to one assignment; omit workspace. Reuse a known process_ref and input contract directly; unchanged prepared runs need no authoring skill or guide reload. Discover when selecting a recipe or checking an unknown contract. apr_run returns completed output when ready; inspect unfinished or truncated runs with view=run and run_id. Before authoring, load apr_discover guide=overview and the relevant installed guide. Check runtime.features.jev before proposing Jev execution; feature inclusion is not account access or approval. Fulfill saved caller requests only through tools actually available here. Credentials and access approvals remain owner-controlled. Closing MCP does not cancel independent work."
            }
            Connection::Fixed(_) => {
                "This attachment is bound to one assignment; omit workspace. Reuse a known process_ref and input contract directly; unchanged prepared runs need no authoring skill or guide reload. Discover when selecting a recipe or checking an unknown contract. apr_run returns completed output when ready; inspect unfinished or truncated runs with view=run and run_id. Fulfill saved caller requests only through tools actually available here. Credentials and access approvals remain owner-controlled. Closing MCP does not cancel independent work."
            }
        };
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("apr", env!("CARGO_PKG_VERSION")))
            .with_instructions(instructions)
    }
}

#[derive(Serialize, JsonSchema)]
struct Scoped<T> {
    /// Required for user-level connections: the handle returned by apr_project.
    /// Omit for legacy fixed-attachment connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    #[serde(flatten)]
    args: T,
}

// Serde flatten silently drops unknown fields on some inner structs, even with
// deny_unknown_fields. Remove only workspace, then use the strict inner parser.
impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for Scoped<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut object = serde_json::Map::<String, Value>::deserialize(deserializer)?;
        let workspace = object
            .remove("workspace")
            .map(serde_json::from_value::<Option<String>>)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .flatten();
        let args =
            serde_json::from_value(Value::Object(object)).map_err(serde::de::Error::custom)?;
        Ok(Self { workspace, args })
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
enum ProjectArgs {
    Prepare { root: String, name: Option<String> },
    Connect { challenge_id: String },
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DiscoverArgs {
    /// Read one bundled guide, without query/limit/process_ref/detail. No provider call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guide: Option<guides::Topic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    /// Maximum number of summaries, from 1 to 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<DiscoverDetail>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DiscoverDetail {
    Schema,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    process_ref: String,
    /// Raw JSON matching the discovered input schema; do not encode objects or arrays as strings.
    #[schemars(with = "JsonValueSchema")]
    input: Value,
    request_key: String,
    mode: RunMode,
    /// Polling budget after durable acceptance, excluding start/IPC timeouts.
    /// Zero disables waiting. This never limits or cancels recipe execution.
    #[serde(default = "default_run_wait_ms", skip_serializing)]
    #[schemars(range(min = 0, max = 5000), extend("default" = 1000))]
    wait_ms: u64,
    /// Full is the default. Compact keeps receipts and output; output keeps the result and run ID;
    /// receipt keeps the run ID for viewing the saved output outside the conversation.
    /// Only successful, untruncated outputs are projected.
    #[serde(default, skip_serializing)]
    result_view: RunResultView,
    #[serde(default = "empty_object")]
    #[schemars(with = "std::collections::BTreeMap<String, JsonValueSchema>")]
    config_overrides: Value,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum RunResultView {
    #[default]
    Full,
    Compact,
    Output,
    Receipt,
}

fn default_run_wait_ms() -> u64 {
    DEFAULT_RUN_WAIT_MS
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum RunMode {
    Preflight,
    Start,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
enum InspectArgs {
    /// Last ten improvement cycles for an authorized recipe version.
    Improvements {
        process_ref: String,
        #[serde(default)]
        detail: bool,
    },
    /// Redacted durable improvement status/report; held-out labels remain private.
    Improvement {
        cycle_id: String,
        #[serde(default)]
        detail: bool,
    },
    /// Bounded retained run counts for one exact recipe version.
    ImprovementEvidence {
        process_ref: String,
        run_ids: Vec<String>,
        #[serde(default)]
        detail: bool,
    },
    /// Tools explicitly approved by the owner for all CookBook projects. No credentials.
    ConnectionLibrary {
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    /// Public schemas of one immutable approved library tool.
    LibraryContract {
        library_id: String,
        revision: u64,
        #[serde(default)]
        detail: bool,
    },
    /// Public contract of an approved scoped connection; credentials and endpoint details remain private.
    ConnectionContract {
        name: String,
        #[serde(default)]
        detail: bool,
    },
    /// Retained opt-in Other cases for one classifier version. Page with next_cursor.
    LearningCases {
        process_ref: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    McpApp {
        run_id: String,
        resource_step_id: String,
        tool_step_id: String,
    },
    PriceSources {
        #[serde(default)]
        detail: bool,
    },
    McpInteractions {
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    /// Exact whole-period ledger totals, with currencies and evidence kept separate.
    Billing {
        starts_at_ms: i64,
        ends_at_ms: i64,
        #[serde(default)]
        detail: bool,
    },
    Budget {
        #[serde(default)]
        detail: bool,
    },
    /// Pending scoped connection requests; only owner setup can approve them.
    Approvals {
        #[serde(default)]
        detail: bool,
    },
    Readiness {
        process_ref: String,
        #[serde(default)]
        detail: bool,
    },
    /// Local retained-run metrics and a bounded page of external-call costs.
    /// Reported and estimated amounts are not additive. run_id selects its tree.
    Analytics {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    Capacity {
        #[serde(default)]
        detail: bool,
    },
    Workers {
        #[serde(default)]
        detail: bool,
    },
    TreeStatus {
        run_id: String,
        #[serde(default)]
        detail: bool,
    },
    Graph {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    Tree {
        run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    JevTuning {
        process_ref: String,
        #[serde(default)]
        detail: bool,
    },
    Schedules {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default)]
        detail: bool,
    },
    Schedule {
        schedule_id: String,
        #[serde(default)]
        detail: bool,
    },
    Runs {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default)]
        detail: bool,
    },
    Bindings {
        #[serde(default)]
        detail: bool,
    },
    Effects {
        run_id: String,
        #[serde(default)]
        detail: bool,
    },
    Config {
        run_id: String,
        #[serde(default)]
        detail: bool,
    },
    Run {
        run_id: String,
        #[serde(default)]
        detail: bool,
    },
    Pending {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default)]
        detail: bool,
    },
    Events {
        run_id: String,
        #[serde(default)]
        after: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u16>,
        #[serde(default)]
        detail: bool,
    },
    Artifact {
        reference: ArtifactReference,
        #[serde(default)]
        detail: bool,
    },
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ArtifactReference {
    artifact_id: String,
    sha256: String,
    media_type: String,
    bytes: u64,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
enum ResumeArgs {
    #[serde(rename = "mcp.respond")]
    McpRespond {
        interaction_id: String,
        request_key: String,
        #[schemars(with = "JsonValueSchema")]
        responses: Value,
    },
    Reconcile {
        run_id: String,
        step_instance_id: String,
        intent_id: String,
        outcome: String,
        evidence: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(with = "Option<JsonValueSchema>")]
        output: Option<Value>,
    },
    Submit {
        protocol_version: String,
        run_id: String,
        request_id: String,
        step_instance_id: String,
        generation: u32,
        submission_key: String,
        continuation_token: String,
        /// Raw observed JSON matching the handoff result schema; strings remain strings.
        #[schemars(with = "JsonValueSchema")]
        output: Value,
        /// An object identifying how the result was observed; requires provenance.source.
        #[schemars(with = "std::collections::BTreeMap<String, JsonValueSchema>")]
        provenance: Value,
    },
    Renew {
        run_id: String,
    },
    Continue {
        run_id: String,
    },
    Pause {
        run_id: String,
    },
    Retry {
        run_id: String,
        step_instance_id: String,
    },
    Cancel {
        run_id: String,
    },
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum JevChange {
    Question {
        step_id: String,
        question_id: String,
        /// Complete question obtained from jev_tuning; preserve its type and option identities.
        #[schemars(with = "JsonValueSchema")]
        question: Value,
    },
    Policy {
        step_id: String,
        /// Complete decision policy obtained from jev_tuning; preserve its routes and bindings.
        #[schemars(with = "JsonValueSchema")]
        policy: Value,
    },
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TuneMode {
    Preview,
    Publish,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "action", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
enum ManageArgs {
    #[serde(rename = "improvement.open")]
    ImprovementOpen {
        request_key: String,
        process_ref: String,
        run_ids: Vec<String>,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "improvement.propose")]
    ImprovementPropose {
        cycle_id: String,
        #[schemars(with = "JsonValueSchema")]
        proposal: Value,
        candidate_path: String,
        #[schemars(with = "JsonValueSchema")]
        review: Value,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "improvement.freeze")]
    ImprovementFreeze {
        cycle_id: String,
        #[schemars(with = "JsonValueSchema")]
        policy: Value,
        #[schemars(with = "JsonValueSchema")]
        development: Value,
        #[schemars(with = "JsonValueSchema")]
        holdout: Value,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "improvement.evaluate")]
    ImprovementEvaluate {
        cycle_id: String,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "improvement.close")]
    ImprovementClose {
        cycle_id: String,
        outcome: String,
        reason: String,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "improvement.decide")]
    ImprovementDecide {
        cycle_id: String,
        decision: String,
        candidate_path: String,
        expected_report_digest: String,
        expected_selected_ref: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    /// Reuse an exact tool already approved for all projects; cannot edit its contract or key.
    #[serde(rename = "connection.use")]
    UseConnection {
        library_id: String,
        revision: u64,
        name: String,
        #[serde(default)]
        detail: bool,
    },
    /// Advisory review of caller boundaries and output budgets; no grants, dispatch, or savings claim.
    #[serde(rename = "process.audit")]
    AuditProcess {
        path: String,
        #[schemars(with = "JsonValueSchema")]
        review: Value,
        #[serde(default)]
        detail: bool,
    },
    /// Request an owner-reviewed connection and optional payload-derived cost plan.
    /// No secrets. Authenticated requests use credential_ref="pending".
    #[serde(rename = "connection.propose")]
    ProposeConnection {
        name: String,
        request_key: String,
        rationale: String,
        #[schemars(with = "JsonValueSchema")]
        connection: Value,
        #[serde(default)]
        detail: bool,
    },
    /// Discover public OAuth metadata for an HTTPS resource. This never logs in, registers, or grants access.
    #[serde(rename = "connection.discover")]
    DiscoverConnection {
        resource: String,
        #[serde(default)]
        client_id: String,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata_url: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    /// Pure fixture preview of compiled recipe cost rules; no calls or grants.
    #[serde(rename = "cost.preview")]
    PreviewCost {
        #[schemars(with = "JsonValueSchema")]
        connection: Value,
        #[schemars(with = "JsonValueSchema")]
        request: Value,
        #[schemars(with = "JsonValueSchema")]
        response: Value,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "process.tune_jev")]
    TuneJev {
        process_ref: String,
        mode: TuneMode,
        changes: Vec<JevChange>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_process_ref: Option<String>,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "schedule.create")]
    CreateSchedule {
        name: String,
        process_ref: String,
        #[schemars(with = "JsonValueSchema")]
        input: Value,
        request_key: String,
        trigger: ScheduleTrigger,
        #[serde(default = "empty_object")]
        #[schemars(with = "std::collections::BTreeMap<String, JsonValueSchema>")]
        config_overrides: Value,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "schedule.pause")]
    PauseSchedule { schedule_id: String },
    #[serde(rename = "schedule.resume")]
    ResumeSchedule { schedule_id: String },
    #[serde(rename = "schedule.delete")]
    DeleteSchedule { schedule_id: String },
    #[serde(rename = "process.validate")]
    Validate { path: String },
    #[serde(rename = "process.register")]
    Register { path: String },
    /// Select an already registered recipe for future discovery with compare-and-swap revision.
    #[serde(rename = "process.select")]
    Select {
        process_ref: String,
        expected_selected_ref: Option<String>,
        expected_selection_revision: u64,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "process.test")]
    Test {
        path: String,
        /// Raw JSON matching the process input schema; do not JSON-encode it as a string.
        #[schemars(with = "JsonValueSchema")]
        input: Value,
        request_key: String,
        #[serde(default = "empty_object")]
        #[schemars(with = "std::collections::BTreeMap<String, JsonValueSchema>")]
        config_overrides: Value,
    },
    #[serde(rename = "process.promote")]
    Promote {
        process_ref: String,
        target_assignment_id: String,
    },
    #[serde(rename = "config.read")]
    ReadConfig {
        process_ref: String,
        #[serde(default)]
        detail: bool,
    },
    #[serde(rename = "config.set")]
    SetConfig {
        process_ref: String,
        #[schemars(with = "std::collections::BTreeMap<String, JsonValueSchema>")]
        values: Value,
    },
    #[serde(rename = "artifact.import")]
    Import { path: String, media_type: String },
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
enum ScheduleTrigger {
    Once {
        /// UTC Unix timestamp in milliseconds.
        at_ms: i64,
    },
    Interval {
        every_seconds: u64,
        /// Optional first occurrence; default is now plus one interval.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_at_ms: Option<i64>,
    },
    Cron {
        /// Five numeric fields (minute, hour, day, month, weekday), in UTC.
        expression: String,
    },
}

fn structured_result(value: Value, failed: bool) -> CallToolResult {
    // The SDK permits empty content plus structuredContent. Its `structured`
    // constructor repeats the entire JSON as text, so populate these fields
    // directly to preserve the output budget and avoid duplicated model input.
    let mut result = CallToolResult::success(Vec::new());
    result.structured_content = Some(value);
    result.is_error = Some(failed);
    result
}

fn format_completed_run(result: CallToolResult, view: RunResultView) -> CallToolResult {
    if matches!(view, RunResultView::Full) || result.is_error == Some(true) {
        return result;
    }
    let Some(value) = result.structured_content.as_ref() else {
        return result;
    };
    // Never reinterpret partial output, errors or actionable caller requests.
    // The service has already authenticated, bounded and completed this result.
    if value["state"] != "succeeded"
        || value["truncated"] != false
        || value.get("output").is_none()
        || value.get("error").is_some()
        || value.get("request").is_some()
    {
        return result;
    }
    let Some(run_id) = value["run_id"].as_str() else {
        return result;
    };
    match view {
        RunResultView::Output => {
            return structured_result(
                json!({"run_id":run_id,"state":"succeeded","result_view":"output",
                       "output_complete":true,"output":value["output"]}),
                false,
            );
        }
        RunResultView::Receipt => {
            return structured_result(
                json!({"run_id":run_id,"state":"succeeded","result_view":"receipt",
                       "output_available":true}),
                false,
            );
        }
        RunResultView::Compact => {}
        RunResultView::Full => return result,
    }
    let mut compact = result.clone();
    let Some(map) = compact
        .structured_content
        .as_mut()
        .and_then(Value::as_object_mut)
    else {
        return result;
    };
    // Remove only known progress fields. Keep every receipt/lineage field and
    // any future field; full scoped inspection retains the original audit.
    for field in ["steps", "completed_steps", "summary"] {
        map.remove(field);
    }
    map.insert("result_view".into(), json!("compact"));
    map.insert("output_complete".into(), json!(true));
    match (serde_json::to_vec(&compact), serde_json::to_vec(&result)) {
        (Ok(smaller), Ok(original))
            if smaller.len() < original.len() && smaller.len() <= DEFAULT_RESULT_BYTES =>
        {
            compact
        }
        _ => result,
    }
}

fn public_error(code: &str) -> CallToolResult {
    public_error_with_diagnostic(code, None)
}

fn public_error_for(error: &apr_runtime::RuntimeError) -> CallToolResult {
    let diagnostic = match error {
        apr_runtime::RuntimeError::Remote { diagnostic, .. } => diagnostic.clone(),
        apr_runtime::RuntimeError::Core(core) => core.diagnostic(),
        _ => None,
    };
    public_error_with_diagnostic(error.code(), diagnostic)
}

fn public_error_with_diagnostic(
    code: &str,
    diagnostic: Option<apr_core::AdmissionDiagnostic>,
) -> CallToolResult {
    let code = match code {
        "WORKSPACE_REQUIRED"
        | "INVALID_MANIFEST"
        | "INVALID_INPUT"
        | "NOT_FOUND"
        | "SCOPE_DENIED"
        | "PERMISSION_REQUIRED"
        | "REQUEST_CONFLICT"
        | "SUBMISSION_CONFLICT"
        | "STALE_HANDOFF"
        | "LIMIT_EXCEEDED"
        | "ARTIFACT_INTEGRITY"
        | "INVALID_STATE"
        | "UNAUTHORIZED"
        | "FORBIDDEN"
        | "SERVICE_UNAVAILABLE"
        | "UNSUPPORTED_VERSION" => code,
        _ => "INTERNAL_ERROR",
    };
    let message = match code {
        "WORKSPACE_REQUIRED" => {
            "Connect this project through apr_project first, then include its workspace handle on every APR call."
        }
        "INVALID_MANIFEST" => "The process file failed admission.",
        "INVALID_INPUT" => "The request does not match the supported input contract.",
        "NOT_FOUND" => "The requested item is not available in this assignment.",
        "SCOPE_DENIED" | "PERMISSION_REQUIRED" => "This attachment does not permit that operation.",
        "REQUEST_CONFLICT" => "The request key already identifies different work.",
        "SUBMISSION_CONFLICT" => "The submission key already identifies a different result.",
        "STALE_HANDOFF" => "The caller handoff is stale. Inspect the current request.",
        "LIMIT_EXCEEDED" => {
            "The request or response exceeds its byte or execution limit. Inspect view capacity for owner ceilings; use smaller bounded requests or ask the owner to adjust capacity."
        }
        "ARTIFACT_INTEGRITY" => "The artifact could not be validated or stored safely.",
        "INVALID_STATE" => "The saved state does not permit this operation.",
        "UNAUTHORIZED" => "The attachment is invalid, revoked or closed.",
        "FORBIDDEN" => "This attachment does not permit that operation.",
        "SERVICE_UNAVAILABLE" => "The local service could not be reached or started.",
        "UNSUPPORTED_VERSION" => "The local service or protocol version is incompatible.",
        _ => "The local service could not complete this request.",
    };
    let mut error = json!({"code": code, "message": message});
    if let Some(diagnostic) = diagnostic {
        error["diagnostic"] = serde_json::to_value(diagnostic).unwrap_or(Value::Null);
    }
    structured_result(json!({"error": error}), true)
}

/// Limit bytes before each newline before rmcp's line buffer or parser sees them.
/// The byte count lives on the reader, so cancelling an individual read cannot
/// reset the count of a partially received frame.
struct LineLimitedReader<R> {
    reader: R,
    line_bytes: usize,
    limit: usize,
    rejected: bool,
}

impl<R> LineLimitedReader<R> {
    fn new(reader: R, limit: usize) -> Self {
        Self {
            reader,
            line_bytes: 0,
            limit,
            rejected: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LineLimitedReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.rejected {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP frame exceeds limit",
            )));
        }
        let mut chunk = [0_u8; 8192];
        let capacity = output.remaining().min(chunk.len());
        if capacity == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut input = ReadBuf::new(&mut chunk[..capacity]);
        match Pin::new(&mut self.reader).poll_read(cx, &mut input) {
            Poll::Ready(Ok(())) => {
                for byte in input.filled() {
                    if *byte == b'\n' {
                        self.line_bytes = 0;
                    } else {
                        self.line_bytes += 1;
                        if self.line_bytes > self.limit {
                            self.rejected = true;
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "MCP frame exceeds limit",
                            )));
                        }
                    }
                }
                output.put_slice(input.filled());
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apr_runtime::RuntimeError;
    use apr_runtime::service::AttachmentFile;
    use tokio::io::AsyncReadExt;

    fn bridge(mode: &str) -> Bridge {
        let config = AttachmentFile {
            protocol_version: "0.1".into(),
            state_dir: "/unused".into(),
            attachment_id: "attachment_test".into(),
            assignment_id: "assignment_test".into(),
            mode: mode.into(),
            token: "DO_NOT_EXPOSE".into(),
        };
        Bridge::new(ServiceClient::with_executable(config, "/unused/apr"))
    }

    fn schema_witness(field: &Value, root: &Value) -> Value {
        if let Some(reference) = field.get("$ref").and_then(Value::as_str) {
            let name = reference
                .strip_prefix("#/$defs/")
                .expect("local definition");
            return schema_witness(&root["$defs"][name], root);
        }
        if let Some(value) = field.get("const") {
            return value.clone();
        }
        if let Some(value) = field.get("enum").and_then(Value::as_array) {
            return value.first().expect("nonempty enum").clone();
        }
        if let Some(branch) = field.get("oneOf").and_then(Value::as_array) {
            return schema_witness(branch.first().expect("nonempty union"), root);
        }
        let kind = field["type"]
            .as_str()
            .or_else(|| field["type"].as_array().and_then(|kinds| kinds[0].as_str()))
            .expect("supported witness type");
        match kind {
            "object" => {
                let mut object = serde_json::Map::new();
                if let Some(required) = field.get("required").and_then(Value::as_array) {
                    for name in required {
                        let name = name.as_str().expect("property name");
                        object.insert(
                            name.into(),
                            schema_witness(&field["properties"][name], root),
                        );
                    }
                }
                Value::Object(object)
            }
            "array" => json!([]),
            "string" => json!("x"),
            "integer" | "number" => json!(1),
            "boolean" => json!(false),
            "null" => Value::Null,
            other => panic!("unsupported witness type {other}"),
        }
    }

    #[test]
    fn normal_surface_is_small_and_authoring_is_hidden_from_run_mode() {
        let run = bridge("run");
        let names: Vec<_> = run
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect();
        assert_eq!(
            names,
            ["apr_discover", "apr_inspect", "apr_resume", "apr_run"]
        );
        assert!(!run.tool_router.has_route("apr_manage"));
        let author = bridge("author");
        assert_eq!(author.tool_router.list_all().len(), 5);
        for tool in author.tool_router.list_all() {
            let readonly = matches!(tool.name.as_ref(), "apr_discover" | "apr_inspect");
            assert_eq!(tool.annotations.unwrap().read_only_hint, Some(readonly));
            assert!(
                !serde_json::to_string(&tool.input_schema)
                    .unwrap()
                    .contains("DO_NOT_EXPOSE")
            );
        }
    }

    #[test]
    fn arguments_reject_extra_fields_and_results_do_not_duplicate_json() {
        assert!(serde_json::from_value::<RunArgs>(json!({
            "process_ref":"x", "input":{}, "request_key":"k", "mode":"start", "attachment_id":"forged"
        })).is_err());
        assert!(
            serde_json::from_value::<ResumeArgs>(
                json!({"action":"renew","run_id":"r","token":"forged"})
            )
            .is_err()
        );
        let value = json!({"state":"waiting_for_agent", "run_id":"run_example"});
        let result = structured_result(value.clone(), false);
        assert!(result.content.is_empty());
        assert_eq!(result.structured_content, Some(value));
        let result = public_error(RuntimeError::InvalidInput("PRIVATE_VALUE".into()).code());
        assert!(result.is_error.unwrap());
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("PRIVATE_VALUE")
        );
    }

    #[test]
    fn run_wait_preserves_boundaries_and_propagates_authorization_errors() {
        let queued = json!({"run_id":"run_test", "state":"queued", "revision":1});
        fn no_read(_: &str, _: WaitInspection) -> apr_runtime::Result<Value> {
            panic!("unexpected inspection")
        }
        assert_eq!(
            wait_for_run(queued.clone(), Duration::ZERO, no_read).unwrap(),
            queued
        );
        for state in [
            "succeeded",
            "failed",
            "cancelled",
            "blocked",
            "paused",
            "needs_reconciliation",
            "waiting_for_agent",
        ] {
            let result = json!({"run_id":"run_test", "state":state, "truncated":true,
                "available_views":["run"]});
            assert_eq!(
                wait_for_run(result.clone(), Duration::from_secs(1), no_read).unwrap(),
                result
            );
        }
        let preflight = json!({"ready":true});
        assert_eq!(
            wait_for_run(preflight.clone(), Duration::from_secs(1), no_read).unwrap(),
            preflight
        );
        let mut reads = 0;
        let result = wait_for_run(queued.clone(), Duration::from_millis(60), |id, view| {
            assert_eq!(id, "run_test");
            assert_eq!(view, WaitInspection::Run);
            reads += 1;
            Ok(queued.clone())
        })
        .unwrap();
        assert_eq!(result, queued);
        assert!(reads <= 3, "waiting must not spin or restart the run");
        let completed = json!({"run_id":"run_test", "state":"succeeded"});
        assert_eq!(
            wait_for_run(queued.clone(), Duration::from_millis(1), |_, view| {
                assert_eq!(view, WaitInspection::Run);
                Ok(completed.clone())
            })
            .unwrap(),
            completed
        );
        let error = wait_for_run(queued, Duration::from_secs(1), |_, _| {
            Err(RuntimeError::Remote {
                code: "UNAUTHORIZED".into(),
                message: "revoked".into(),
                diagnostic: None,
            })
        })
        .unwrap_err();
        assert_eq!(error.code(), "UNAUTHORIZED");
    }

    #[test]
    fn aggregate_family_wait_is_bounded_and_fails_closed() {
        let safe = json!({"view":"tree_status","complete":true,"total_runs":65,"runnable_without_caller":true});
        assert!(!composed_run_needs_attention(&safe));
        for summary in [
            json!({"view":"tree_status","complete":true,"total_runs":65,"runnable_without_caller":false}),
            json!({"view":"tree_status","complete":false,"total_runs":65,"runnable_without_caller":true}),
            json!({"view":"tree_status","complete":true,"total_runs":0,"runnable_without_caller":true}),
            json!({"view":"tree_status","complete":true,"total_runs":4098,"runnable_without_caller":true}),
            json!({"view":"tree_status","total_runs":65,"runnable_without_caller":true}),
        ] {
            assert!(composed_run_needs_attention(&summary));
        }
    }

    #[test]
    fn run_wait_continues_safe_composition_and_stops_for_descendant_attention() {
        let waiting = json!({"run_id":"run_root", "state":"waiting_for_child"});
        assert_eq!(
            wait_for_run(waiting.clone(), Duration::ZERO, no_wait_read).unwrap(),
            waiting,
            "zero wait must perform no inspection"
        );

        let safe_tree = json!({"view":"tree","truncated":false,"runs":[
            {"run_id":"run_root","state":"waiting_for_child","actionable_waiting":false},
            {"run_id":"run_child","state":"running","actionable_waiting":false}
        ]});
        let completed = json!({"run_id":"run_root", "state":"succeeded", "output":{"ok":true}});
        let mut reads = Vec::new();
        let result = wait_for_run(waiting.clone(), Duration::from_millis(50), |_, view| {
            reads.push(view);
            Ok(match view {
                WaitInspection::Tree => safe_tree.clone(),
                WaitInspection::Run => completed.clone(),
            })
        })
        .unwrap();
        assert_eq!(result, completed);
        assert_eq!(reads, [WaitInspection::Tree, WaitInspection::Run]);

        for tree in [
            json!({"view":"tree","truncated":false,"runs":[
                {"run_id":"run_root","state":"waiting_for_child","actionable_waiting":false},
                {"run_id":"run_child","state":"waiting_for_agent","actionable_waiting":true}
            ]}),
            json!({"view":"tree","truncated":false,"runs":[
                {"run_id":"run_root","state":"waiting_for_child","actionable_waiting":false},
                {"run_id":"run_child","state":"needs_reconciliation","actionable_waiting":false}
            ]}),
            json!({"view":"tree","truncated":false,"runs":[
                {"run_id":"run_root","state":"waiting_for_child","actionable_waiting":false},
                {"run_id":"run_child","state":"paused","actionable_waiting":false}
            ]}),
            json!({"view":"tree","truncated":false,"runs":[
                {"run_id":"run_root","state":"waiting_for_child","actionable_waiting":false},
                {"run_id":"run_child","state":"failed","actionable_waiting":false}
            ]}),
            json!({"view":"tree","truncated":true,"runs":[]}),
        ] {
            let mut reads = 0;
            assert_eq!(
                wait_for_run(waiting.clone(), Duration::from_secs(1), |_, view| {
                    assert_eq!(view, WaitInspection::Tree);
                    reads += 1;
                    Ok(tree.clone())
                })
                .unwrap(),
                waiting
            );
            assert_eq!(reads, 1, "attention must return without a polling delay");
        }
        let error = wait_for_run(waiting, Duration::from_secs(1), |_, view| {
            assert_eq!(view, WaitInspection::Tree);
            Err(RuntimeError::Remote {
                code: "UNAUTHORIZED".into(),
                message: "revoked".into(),
                diagnostic: None,
            })
        })
        .unwrap_err();
        assert_eq!(error.code(), "UNAUTHORIZED");

        fn no_wait_read(_: &str, _: WaitInspection) -> apr_runtime::Result<Value> {
            panic!("zero wait inspected")
        }
    }

    #[test]
    fn workspace_arguments_match_the_advertised_schema_without_accepting_extra_fields() {
        for (name, args) in [
            (
                "apr_discover",
                json!({"workspace":"workspace_example","guide":"overview"}),
            ),
            (
                "apr_run",
                json!({"workspace":"workspace_example","process_ref":"p","input":{},"request_key":"k","mode":"start"}),
            ),
            (
                "apr_inspect",
                json!({"workspace":"workspace_example","view":"runs"}),
            ),
            (
                "apr_inspect",
                json!({"workspace":"workspace_example","view":"connection_contract","name":"synthetic.contract"}),
            ),
            (
                "apr_manage",
                json!({"workspace":"workspace_example","action":"process.register","path":"recipe.json"}),
            ),
            (
                "apr_manage",
                json!({"workspace":"workspace_example","action":"connection.discover","resource":"https://example.test/mcp"}),
            ),
            (
                "apr_resume",
                json!({"workspace":"workspace_example","action":"renew","run_id":"r"}),
            ),
        ] {
            let tool = bridge("author")
                .tool_router
                .list_all()
                .into_iter()
                .find(|tool| tool.name == name)
                .unwrap();
            let schema = serde_json::to_value(tool.input_schema).unwrap();
            jsonschema::validator_for(&schema)
                .unwrap()
                .validate(&args)
                .unwrap();
        }
        assert!(serde_json::from_value::<Scoped<ManageArgs>>(json!({"workspace":"w","action":"process.register","path":"a","attachment_id":"forged"})).is_err());
        assert!(serde_json::from_value::<Scoped<RunArgs>>(json!({"workspace":"w","process_ref":"p","input":{},"request_key":"k","mode":"start","extra":true})).is_err());
    }

    #[test]
    fn project_tools_require_a_workspace_but_fixed_tools_and_public_guides_do_not() {
        let state = tempfile::tempdir().unwrap();
        let project = Bridge::projects(ProjectBroker::open(state.path()).unwrap());
        let fixed = bridge("author");
        for (name, args) in [
            (
                "apr_run",
                json!({"process_ref":"p","input":{},"request_key":"k","mode":"start"}),
            ),
            ("apr_inspect", json!({"view":"runs"})),
            (
                "apr_manage",
                json!({"action":"process.register","path":"recipe.json"}),
            ),
            ("apr_resume", json!({"action":"renew","run_id":"r"})),
        ] {
            let schema =
                serde_json::to_value(&project.tool_router.get(name).unwrap().input_schema).unwrap();
            let validator = jsonschema::validator_for(&schema).unwrap();
            assert!(!validator.is_valid(&args), "{name} omitted handle");
            let legacy =
                serde_json::to_value(&fixed.tool_router.get(name).unwrap().input_schema).unwrap();
            assert!(jsonschema::validator_for(&legacy).unwrap().is_valid(&args));
            for value in [Value::Null, json!(""), json!(17)] {
                let mut invalid = args.clone();
                invalid["workspace"] = value;
                assert!(!validator.is_valid(&invalid), "{name} invalid handle");
            }
            let mut valid = args;
            valid["workspace"] = json!("workspace_example");
            assert!(validator.is_valid(&valid), "{name} supplied handle");
        }
        let schema = serde_json::to_value(
            &project
                .tool_router
                .get("apr_discover")
                .unwrap()
                .input_schema,
        )
        .unwrap();
        assert!(
            jsonschema::validator_for(&schema)
                .unwrap()
                .is_valid(&json!({"guide":"overview"}))
        );
    }

    #[test]
    fn compact_tool_schemas_preserve_all_inspection_variants() {
        let original = Bridge::tool_router();
        let compact = bridge("author");
        let mut checked_cases = 0;
        for name in ["apr_inspect", "apr_resume", "apr_manage"] {
            let before = original.get(name).unwrap();
            let after = compact.tool_router.get(name).unwrap();
            let before_schema = serde_json::to_value(&before.input_schema).unwrap();
            let after_schema = serde_json::to_value(&after.input_schema).unwrap();
            eprintln!(
                "{name} schema: {} -> {} bytes",
                serde_json::to_vec(&before_schema).unwrap().len(),
                serde_json::to_vec(&after_schema).unwrap().len()
            );
            let before_validator = jsonschema::validator_for(&before_schema).unwrap();
            let after_validator = jsonschema::validator_for(&after_schema).unwrap();
            let discriminator = if name == "apr_inspect" {
                "view"
            } else {
                "action"
            };
            assert_eq!(
                before_schema["oneOf"].as_array().unwrap().len(),
                after_schema["oneOf"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|branch| {
                        branch["properties"][discriminator]["enum"]
                            .as_array()
                            .map_or(1, Vec::len)
                    })
                    .sum::<usize>()
            );
            for branch in before_schema["oneOf"].as_array().unwrap() {
                let tag = branch["properties"][discriminator]["const"]
                    .as_str()
                    .unwrap();
                if let Some(description) = branch.get("description").and_then(Value::as_str) {
                    assert!(
                        after_schema["oneOf"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|candidate| {
                                candidate["description"]
                                    .as_str()
                                    .is_some_and(|text| text.contains(description))
                            }),
                        "description lost for {tag}"
                    );
                }
                let mut base = serde_json::Map::new();
                base.insert(discriminator.into(), json!(tag));
                for required in branch["required"].as_array().unwrap() {
                    let field = required.as_str().unwrap();
                    if field == discriminator {
                        continue;
                    }
                    let example = schema_witness(&branch["properties"][field], &before_schema);
                    base.insert(field.into(), example);
                }
                let mut cases = vec![Value::Object(base.clone())];
                assert!(
                    before_validator.is_valid(&cases[0]),
                    "baseline witness invalid for {name} {tag}: {}",
                    cases[0]
                );
                let mut without_tag = base.clone();
                without_tag.remove(discriminator);
                cases.push(Value::Object(without_tag));
                for (field, example) in &base {
                    if field == discriminator {
                        continue;
                    }
                    let mut missing = base.clone();
                    missing.remove(field);
                    cases.push(Value::Object(missing));
                    let mut wrong_type = base.clone();
                    wrong_type.insert(
                        field.clone(),
                        if example.is_boolean() {
                            json!([])
                        } else {
                            json!(true)
                        },
                    );
                    cases.push(Value::Object(wrong_type));
                }
                for (field, _) in branch["properties"].as_object().unwrap() {
                    if field == discriminator {
                        continue;
                    }
                    for value in [
                        Value::Null,
                        json!(true),
                        json!(7),
                        json!("x"),
                        json!([]),
                        json!({}),
                    ] {
                        let mut candidate = base.clone();
                        candidate.insert(field.clone(), value);
                        cases.push(Value::Object(candidate));
                    }
                }
                for (field, value) in [
                    ("workspace", json!("handle")),
                    ("workspace", json!(42)),
                    ("extra", json!(42)),
                    (discriminator, json!("unknown_view_or_action")),
                ] {
                    let mut candidate = base.clone();
                    candidate.insert(field.into(), value);
                    cases.push(Value::Object(candidate));
                }
                for case in cases {
                    assert_eq!(
                        before_validator.is_valid(&case),
                        after_validator.is_valid(&case),
                        "{name} {tag}: {case}"
                    );
                    checked_cases += 1;
                }
            }
        }
        eprintln!("schema equivalence cases: {checked_cases}");
        assert!(
            serde_json::from_value::<Scoped<InspectArgs>>(
                json!({"view":"run","run_id":"r","extra":true})
            )
            .is_err()
        );
    }

    #[test]
    fn compact_metadata_reduces_each_connection_surface() {
        let original = Bridge::tool_router();
        let mut compact = original.clone();
        compact_tool_schemas(&mut compact);
        for (mode, removed) in [
            ("run", vec!["apr_project", "apr_manage"]),
            ("author", vec!["apr_project"]),
            ("global", vec![]),
        ] {
            let mut before = original.clone();
            let mut after = compact.clone();
            for name in removed {
                before.disable_route(name);
                after.disable_route(name);
            }
            let old_bytes = serde_json::to_vec(&before.list_all()).unwrap().len();
            let new_bytes = serde_json::to_vec(&after.list_all()).unwrap().len();
            assert!(new_bytes < old_bytes, "{mode} metadata did not shrink");
            eprintln!("{mode} tools/list: {old_bytes} -> {new_bytes} bytes");
        }
        let run = bridge("run").get_info().instructions.unwrap();
        let author = bridge("author").get_info().instructions.unwrap();
        let state = tempfile::tempdir().unwrap();
        let global = Bridge::projects(ProjectBroker::open(state.path()).unwrap())
            .get_info()
            .instructions
            .unwrap();
        eprintln!(
            "initialize instructions bytes: run={} author={} global={}",
            run.len(),
            author.len(),
            global.len()
        );
        assert!(run.len() < author.len());
        assert!(!run.contains("Before authoring"));
        assert!(author.contains("Before authoring"));
        assert!(global.contains("apr_project"));
    }

    #[tokio::test]
    async fn frame_limits_apply_across_reads_and_reset_only_at_newlines() {
        let input = &b"abc\ndef\n"[..];
        let mut reader = LineLimitedReader::new(input, 3);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, input);

        let mut reader = LineLimitedReader::new(&b"abcd\n"[..], 3);
        let mut prefix = [0_u8; 2];
        reader.read_exact(&mut prefix).await.unwrap();
        assert_eq!(&prefix, b"ab");
        let error = reader.read_exact(&mut prefix).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            reader.read_exact(&mut prefix).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
