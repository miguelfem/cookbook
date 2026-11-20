#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Bundled views only. The inspector has assignment authority; owner commands
//! belong exclusively to the separate setup window and are never MCP methods.
mod apps;
mod authorization;
mod launch;
mod updates;

use apr_connectors::{Connection, Secret};
use apr_runtime::{Runtime, service::ServiceClient, setup::ConnectionProposal};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

type Reply = Result<Value, String>;
#[derive(Clone)]
struct Desktop {
    root: PathBuf,
    executable: PathBuf,
    selected: Arc<Mutex<Option<PathBuf>>>,
    install_plan: Arc<Mutex<Option<apr_integrations::Plan>>>,
    agent_plan: Arc<Mutex<Option<apr_integrations::global::GlobalPlan>>>,
    isolated: bool,
}
impl Desktop {
    fn runtime(&self) -> Result<Runtime, String> {
        Runtime::open(&self.root).map_err(public_error)
    }
    fn client(&self) -> Result<ServiceClient, String> {
        let path = self
            .selected
            .lock()
            .map_err(|_| "LOCAL_STATE_LOCKED")?
            .clone()
            .ok_or("Connect your agent in Settings, or open a saved workspace to view it here")?;
        ServiceClient::from_file(&path)
            .map(|c| c.with_launcher(&self.executable))
            .map_err(public_error)
    }
    fn select(&self, assignment: &str) -> Reply {
        let path = self
            .runtime()?
            .owner_attachment_file(assignment)
            .map_err(public_error)?;
        // Only a private path is persisted. Attachment credentials remain native.
        let bytes = serde_json::to_vec(&json!({"attachment_file":path}))
            .map_err(|_| "LOCAL_STATE_INVALID")?;
        let tmp = self.root.join(format!(
            "desktop-selection-{}-{}.new",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "LOCAL_CLOCK_INVALID")?
                .as_nanos()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        use std::io::Write;
        let mut file = options.open(&tmp).map_err(|_| "LOCAL_STATE_WRITE_FAILED")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "LOCAL_STATE_WRITE_FAILED")?;
        std::fs::rename(tmp, self.root.join("desktop-selection.json"))
            .map_err(|_| "LOCAL_STATE_WRITE_FAILED")?;
        *self.selected.lock().map_err(|_| "LOCAL_STATE_LOCKED")? = Some(path);
        *self.install_plan.lock().map_err(|_| "LOCAL_STATE_LOCKED")? = None;
        Ok(json!({"selected":true,"assignment_id":assignment}))
    }
}
fn public_error(error: apr_runtime::RuntimeError) -> String {
    error.to_string()
}
fn require(window: &tauri::WebviewWindow, label: &str) -> Result<(), String> {
    if window.label() != label {
        Err("OWNER_WINDOW_REQUIRED".into())
    } else {
        Ok(())
    }
}
fn secret(value: Option<String>) -> Result<Option<Secret>, String> {
    value
        .filter(|s| !s.is_empty())
        .map(|s| Secret::new(s.into_bytes()).map_err(|_| "Invalid secret format".into()))
        .transpose()
}
#[tauri::command]
fn desktop_status(state: tauri::State<'_, Desktop>) -> Value {
    match state.client() {
        Ok(client) => {
            json!({"selected":true,"assignment_id":client.assignment_id(),"mode":client.mode(),"device":std::env::consts::OS,"window_close":"saved work continues while this device is awake"})
        }
        Err(_) => {
            json!({"selected":false,"device":std::env::consts::OS,"next_action":"Open setup"})
        }
    }
}
#[tauri::command]
async fn scoped_call(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Desktop>,
    action: String,
    args: Value,
    detail: bool,
) -> Reply {
    require(&window, "main")?;
    if !matches!(
        action.as_str(),
        "discover" | "inspect" | "run" | "resume" | "manage"
    ) {
        return Err("SCOPED_ACTION_REQUIRED".into());
    }
    let client = state.client()?;
    tauri::async_runtime::spawn_blocking(move || {
        client
            .call(&action, args, if detail { 65536 } else { 8192 })
            .map_err(public_error)
    })
    .await
    .map_err(|_| "LOCAL_TASK_FAILED".to_string())?
}
fn setup_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("setup") {
        window
            .show()
            .and_then(|_| window.set_focus())
            .map_err(|_| "WINDOW_UNAVAILABLE".to_string())?;
    } else {
        tauri::WebviewWindowBuilder::new(app, "setup", tauri::WebviewUrl::App("setup.html".into()))
            .incognito(app.state::<Desktop>().isolated)
            .title("CookBook · Settings & access")
            .inner_size(940.0, 800.0)
            .min_inner_size(680.0, 500.0)
            .on_navigation(|url| {
                (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                    || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
            })
            .build()
            .map_err(|_| "WINDOW_UNAVAILABLE".to_string())?;
    }
    Ok(())
}
#[tauri::command]
fn open_setup(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    require(&window, "main")?;
    setup_window(&app)
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum OwnerAction {
    WorkerPolicy,
    WorkerStatus,
    SetWorkerPolicy {
        policy: apr_runtime::worker_policy::WorkerPolicy,
        expected_revision: u64,
    },
    CredentialStoreStatus,
    MigratePlatformCredentials,
    PythonStatus {
        assignment: String,
    },
    PreviewPython {
        interpreter: PathBuf,
    },
    GrantPython {
        assignment: String,
        interpreter: PathBuf,
        sha256: String,
    },
    ListLibrary {
        #[serde(default)]
        after: Option<String>,
    },
    ListAccounts {
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        connection: Option<Connection>,
    },
    RevokeAccount {
        account_id: String,
        revision: u64,
    },
    PreviewAccountReuse {
        request_id: String,
        digest: String,
        account_id: String,
        revision: u64,
    },
    SaveTool {
        assignment: String,
        name: String,
        revision: u64,
    },
    DisableTool {
        library_id: String,
        revision: u64,
    },
    ListBindings {
        assignment: String,
    },
    ListAssignments,
    CreateProject {
        root: PathBuf,
        name: String,
    },
    SelectAssignment {
        assignment: String,
    },
    ListApprovals {
        assignment: String,
        #[serde(default)]
        after: Option<String>,
    },
    Propose {
        assignment: String,
        proposal: ConnectionProposal,
    },
    Approve {
        request_id: String,
        digest: String,
        secret: Option<String>,
        #[serde(default)]
        options: apr_runtime::connection_library::ApprovalOptions,
    },
    Reject {
        request_id: String,
        digest: String,
    },
    Revoke {
        assignment: String,
        name: String,
    },
    Rebind {
        run_id: String,
    },
    PinCommand {
        executable: PathBuf,
    },
    DiscoverMcp {
        connection: Connection,
        secret: Option<String>,
        native_trust: bool,
        #[serde(default)]
        automatic_protocol: bool,
        #[serde(default)]
        kind: Option<String>,
    },
    ConnectMcpAccount {
        connection: Connection,
        secret: Option<String>,
        saved_account: Option<apr_runtime::connection_library::SavedAccount>,
        kind: String,
        #[serde(default)]
        automatic_protocol: bool,
    },
    PreviewAgent {
        client: apr_integrations::Client,
        #[serde(default)]
        trust_apr_run: bool,
    },
    ApplyAgent {
        plan_id: String,
    },
    PreviewClient {
        root: PathBuf,
        client: apr_integrations::Client,
    },
    ApplyClient {
        plan_id: String,
    },
    DoctorClient {
        root: PathBuf,
        client: apr_integrations::Client,
    },
    MaintenanceStatus,
    UpdatePreferences,
    SetUpdatePreferences {
        enabled: bool,
    },
    FinishMaintenance,
    SetPriceSource {
        assignment: String,
        name: String,
        source: apr_runtime::operations::PriceSource,
        enabled: bool,
    },
    CheckPriceSource {
        assignment: String,
        name: String,
    },
    PriceSources {
        assignment: String,
    },
    ApprovePriceReview {
        request_id: String,
        digest: String,
    },
    McpInteractions {
        assignment: String,
    },
    RespondMcp {
        assignment: String,
        response: apr_runtime::interactions::InteractionResponse,
    },
    Reconcile {
        request: apr_runtime::execution::Reconciliation,
    },
    SetBudget {
        assignment: String,
        policy: apr_runtime::billing::BudgetPolicy,
    },
    Billing {
        assignment: String,
        starts_at_ms: i64,
        ends_at_ms: i64,
    },
    ImportInvoice {
        assignment: String,
        invoice: apr_runtime::billing::Invoice,
    },
    Budget {
        assignment: String,
    },
    DiscoverOauth {
        resource: String,
        metadata_url: Option<String>,
        client_id: String,
        scopes: Vec<String>,
    },
    StopService,
}
#[tauri::command]
async fn owner_request(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Desktop>,
    request: OwnerAction,
) -> Reply {
    require(&window, "setup")?;
    let refresh = matches!(
        &request,
        OwnerAction::CreateProject { .. }
            | OwnerAction::GrantPython { .. }
            | OwnerAction::MigratePlatformCredentials
            | OwnerAction::SelectAssignment { .. }
            | OwnerAction::Approve { .. }
            | OwnerAction::Revoke { .. }
            | OwnerAction::Rebind { .. }
            | OwnerAction::RespondMcp { .. }
            | OwnerAction::Reconcile { .. }
            | OwnerAction::ApprovePriceReview { .. }
            | OwnerAction::SetBudget { .. }
            | OwnerAction::SetWorkerPolicy { .. }
            | OwnerAction::FinishMaintenance
    );
    let desktop = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || owner_dispatch(&desktop, request))
        .await
        .map_err(|_| "LOCAL_TASK_FAILED".to_string())??;
    if refresh {
        let _ = app.emit_to("main", "cookbook:changed", json!({}));
    }
    Ok(result)
}
fn install_request(
    desktop: &Desktop,
    root: PathBuf,
    client: apr_integrations::Client,
) -> Result<apr_integrations::InstallRequest, String> {
    let canonical = root.canonicalize().map_err(|_| "Project is unavailable")?;
    let selected_id = desktop.client()?.assignment_id().to_string();
    let assignments = desktop
        .runtime()?
        .owner_assignments()
        .map_err(public_error)?;
    if !assignments["assignments"]
        .as_array()
        .ok_or("PROJECTS_UNAVAILABLE")?
        .iter()
        .any(|a| {
            a["assignment_id"] == selected_id
                && a["root"]
                    .as_str()
                    .is_some_and(|p| Path::new(p) == canonical)
        })
    {
        return Err("Choose the matching assignment before changing client settings".into());
    }
    let attachment = desktop
        .selected
        .lock()
        .map_err(|_| "LOCAL_STATE_LOCKED")?
        .clone()
        .ok_or("Choose an assignment first")?;
    Ok(apr_integrations::InstallRequest {
        project_dir: canonical,
        attachment_file: attachment,
        executable: desktop.executable.clone(),
        client,
    })
}
fn owner_dispatch(desktop: &Desktop, request: OwnerAction) -> Reply {
    if !matches!(
        &request,
        OwnerAction::ListAssignments
            | OwnerAction::WorkerPolicy
            | OwnerAction::WorkerStatus
            | OwnerAction::CredentialStoreStatus
            | OwnerAction::PythonStatus { .. }
            | OwnerAction::PreviewPython { .. }
            | OwnerAction::ListApprovals { .. }
            | OwnerAction::MaintenanceStatus
            | OwnerAction::FinishMaintenance
            | OwnerAction::UpdatePreferences
            | OwnerAction::McpInteractions { .. }
            | OwnerAction::Billing { .. }
            | OwnerAction::Budget { .. }
            | OwnerAction::PriceSources { .. }
            | OwnerAction::StopService
    ) && desktop
        .runtime()?
        .maintenance_status()
        .map_err(public_error)?["paused_for_update"]
        == true
    {
        return Err(
            "Runtime is paused for an update. Complete maintenance before changing configuration"
                .into(),
        );
    }
    match request {
        OwnerAction::WorkerPolicy => Ok(serde_json::to_value(
            desktop.runtime()?.worker_policy().map_err(public_error)?,
        )
        .map_err(|_| "LOCAL_STATE_INVALID")?),
        OwnerAction::WorkerStatus => desktop.runtime()?.worker_status().map_err(public_error),
        OwnerAction::SetWorkerPolicy {
            policy,
            expected_revision,
        } => Ok(serde_json::to_value(
            desktop
                .runtime()?
                .set_worker_policy(policy, expected_revision)
                .map_err(public_error)?,
        )
        .map_err(|_| "LOCAL_STATE_INVALID")?),
        OwnerAction::CredentialStoreStatus => desktop
            .runtime()?
            .credential_store_status()
            .map_err(public_error),
        OwnerAction::MigratePlatformCredentials => desktop
            .runtime()?
            .migrate_platform_credentials()
            .map_err(public_error),
        OwnerAction::PythonStatus { assignment } => desktop
            .runtime()?
            .python_execution_status(&assignment)
            .map_err(public_error),
        OwnerAction::PreviewPython { interpreter } => {
            apr_runtime::worker::InterpreterPin::inspect(&interpreter)
                .map(|pin| json!(pin))
                .map_err(|_| {
                    "Cannot inspect this Python interpreter. Use its absolute path.".into()
                })
        }
        OwnerAction::GrantPython {
            assignment,
            interpreter,
            sha256,
        } => desktop
            .runtime()?
            .grant_python_reviewed(&assignment, &interpreter, &sha256)
            .map_err(public_error),
        OwnerAction::ListAssignments => {
            desktop.runtime()?.owner_assignments().map_err(public_error)
        }
        OwnerAction::CreateProject { root, name } => {
            let value = desktop
                .runtime()?
                .owner_project_setup(&root, &name)
                .map_err(public_error)?;
            desktop.select(
                value["assignment_id"]
                    .as_str()
                    .ok_or("MISSING_ASSIGNMENT")?,
            )?;
            Ok(value)
        }
        OwnerAction::SelectAssignment { assignment } => desktop.select(&assignment),
        OwnerAction::ListLibrary { after } => desktop
            .runtime()?
            .connection_library(after.as_deref(), true)
            .map_err(public_error),
        OwnerAction::ListAccounts { after, connection } => desktop
            .runtime()?
            .saved_accounts_for(after.as_deref(), connection.as_ref())
            .map_err(public_error),
        OwnerAction::PreviewAccountReuse {
            request_id,
            digest,
            account_id,
            revision,
        } => desktop
            .runtime()?
            .preview_reviewed_account_reuse(&request_id, &digest, &account_id, revision)
            .map_err(public_error),
        OwnerAction::RevokeAccount {
            account_id,
            revision,
        } => desktop
            .runtime()?
            .revoke_saved_account(&account_id, revision)
            .map_err(public_error),
        OwnerAction::SaveTool {
            assignment,
            name,
            revision,
        } => desktop
            .runtime()?
            .save_connection_to_library(&assignment, &name, revision)
            .map_err(public_error),
        OwnerAction::DisableTool {
            library_id,
            revision,
        } => desktop
            .runtime()?
            .disable_library_connection(&library_id, revision)
            .map_err(public_error),
        OwnerAction::ListBindings { assignment } => desktop
            .runtime()?
            .execution_bindings(&assignment)
            .map_err(public_error),
        OwnerAction::ListApprovals { assignment, after } => desktop
            .runtime()?
            .connection_requests_page(&assignment, after.as_deref())
            .map_err(public_error),
        OwnerAction::Propose {
            assignment,
            proposal,
        } => desktop
            .runtime()?
            .propose_connection(&assignment, proposal)
            .map_err(public_error),
        OwnerAction::Approve {
            request_id,
            digest,
            secret: input,
            options,
        } => desktop
            .runtime()?
            .approve_connection_with_options(&request_id, &digest, secret(input)?, options)
            .map_err(public_error),
        OwnerAction::Reject { request_id, digest } => desktop
            .runtime()?
            .reject_connection(&request_id, &digest)
            .map_err(public_error),
        OwnerAction::Revoke { assignment, name } => desktop
            .runtime()?
            .revoke_execution(&assignment, "connection", &name)
            .map_err(public_error),
        OwnerAction::Rebind { run_id } => desktop
            .runtime()?
            .rebind_execution(&run_id)
            .map_err(public_error),
        OwnerAction::PinCommand { executable } => {
            if !executable.is_absolute() {
                return Err("Choose an absolute executable path".into());
            }
            apr_connectors::mcp::pin_executable(&executable)
                .map(|digest| json!({"executable":executable,"sha256":digest}))
                .map_err(|_| "Cannot pin this executable".into())
        }
        OwnerAction::DiscoverMcp {
            connection,
            secret: input,
            native_trust,
            kind,
            automatic_protocol,
        } => {
            if connection.mcp.as_ref().is_some_and(|m| m.is_stdio()) && !native_trust {
                return Err(
                    "Confirm that this command runs with your OS privileges before discovery"
                        .into(),
                );
            }
            desktop
                .runtime()?
                .discover_mcp_kind_with_protocol(
                    &connection,
                    secret(input)?,
                    kind.as_deref().unwrap_or("tools"),
                    automatic_protocol,
                )
                .map_err(public_error)
        }
        OwnerAction::ConnectMcpAccount {
            connection,
            secret: input,
            saved_account,
            kind,
            automatic_protocol,
        } => desktop
            .runtime()?
            .connect_mcp_account_protocol(
                &connection,
                secret(input)?,
                saved_account,
                &kind,
                automatic_protocol,
            )
            .map_err(public_error),
        OwnerAction::PreviewAgent {
            client,
            trust_apr_run,
        } => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or("HOME_UNAVAILABLE")?;
            let plan = apr_integrations::global::GlobalManager::open(&desktop.root)
                .map_err(|e| e.to_string())?
                .preview_with_options(
                    &home,
                    &desktop.executable,
                    client,
                    apr_integrations::global::GlobalInstallOptions { trust_apr_run },
                )
                .map_err(|e| e.to_string())?;
            let value = serde_json::to_value(&plan).map_err(|_| "PREVIEW_FAILED")?;
            *desktop
                .agent_plan
                .lock()
                .map_err(|_| "LOCAL_STATE_LOCKED")? = Some(plan);
            Ok(value)
        }
        OwnerAction::ApplyAgent { plan_id } => {
            let mut saved = desktop
                .agent_plan
                .lock()
                .map_err(|_| "LOCAL_STATE_LOCKED")?;
            let plan = saved.as_ref().ok_or("Preview agent connection first")?;
            if plan.plan_id != plan_id {
                return Err("Settings preview changed".into());
            }
            let report = apr_integrations::global::GlobalManager::open(&desktop.root)
                .map_err(|e| e.to_string())?
                .apply(plan)
                .map_err(|e| e.to_string())?;
            *saved = None;
            serde_json::to_value(report).map_err(|_| "APPLY_FAILED".into())
        }
        OwnerAction::PreviewClient { root, client } => {
            let installer =
                apr_integrations::Manager::open(&desktop.root).map_err(|e| e.to_string())?;
            let plan = installer
                .preview(&install_request(desktop, root, client)?)
                .map_err(|e| e.to_string())?;
            let value = serde_json::to_value(&plan).map_err(|_| "PREVIEW_FAILED")?;
            *desktop
                .install_plan
                .lock()
                .map_err(|_| "LOCAL_STATE_LOCKED")? = Some(plan);
            Ok(value)
        }
        OwnerAction::ApplyClient { plan_id } => {
            let mut saved = desktop
                .install_plan
                .lock()
                .map_err(|_| "LOCAL_STATE_LOCKED")?;
            let plan = saved.as_ref().ok_or("Preview client settings first")?;
            if plan.plan_id != plan_id {
                return Err("Settings preview changed".into());
            }
            let report = apr_integrations::Manager::open(&desktop.root)
                .map_err(|e| e.to_string())?
                .apply(plan)
                .map_err(|e| e.to_string())?;
            *saved = None;
            serde_json::to_value(report).map_err(|_| "APPLY_FAILED".into())
        }
        OwnerAction::DoctorClient { root, client } => {
            let report = apr_integrations::Manager::open(&desktop.root)
                .map_err(|e| e.to_string())?
                .doctor(&install_request(desktop, root, client)?)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(report).map_err(|_| "DIAGNOSTICS_FAILED".into())
        }
        OwnerAction::MaintenanceStatus => desktop
            .runtime()?
            .maintenance_status()
            .map_err(public_error),
        OwnerAction::UpdatePreferences => desktop
            .runtime()?
            .update_preferences()
            .map_err(public_error),
        OwnerAction::SetUpdatePreferences { enabled } => desktop
            .runtime()?
            .set_update_preferences(enabled)
            .map_err(public_error),
        OwnerAction::FinishMaintenance => desktop
            .runtime()?
            .finish_desktop_update()
            .map_err(public_error),
        OwnerAction::SetPriceSource {
            assignment,
            name,
            source,
            enabled,
        } => desktop
            .runtime()?
            .set_price_source(&assignment, &name, source, enabled)
            .map_err(public_error),
        OwnerAction::CheckPriceSource { assignment, name } => desktop
            .runtime()?
            .check_price_source(&assignment, &name)
            .map_err(public_error),
        OwnerAction::PriceSources { assignment } => desktop
            .runtime()?
            .price_sources(&assignment)
            .map_err(public_error),
        OwnerAction::ApprovePriceReview { request_id, digest } => desktop
            .runtime()?
            .approve_price_review(&request_id, &digest)
            .map_err(public_error),
        OwnerAction::McpInteractions { assignment } => desktop
            .runtime()?
            .mcp_interactions(&assignment, None)
            .map_err(public_error),
        OwnerAction::RespondMcp {
            assignment,
            response,
        } => desktop
            .runtime()?
            .owner_respond_mcp(&assignment, response)
            .map_err(public_error),
        OwnerAction::Reconcile { request } => {
            desktop.runtime()?.reconcile(&request).map_err(public_error)
        }
        OwnerAction::SetBudget { assignment, policy } => desktop
            .runtime()?
            .set_billing_budget(&assignment, policy)
            .map_err(public_error),
        OwnerAction::Billing {
            assignment,
            starts_at_ms,
            ends_at_ms,
        } => desktop
            .runtime()?
            .billing_period(&assignment, starts_at_ms, ends_at_ms)
            .map_err(public_error),
        OwnerAction::ImportInvoice {
            assignment,
            invoice,
        } => desktop
            .runtime()?
            .import_invoice(&assignment, invoice)
            .map_err(public_error),
        OwnerAction::Budget { assignment } => desktop
            .runtime()?
            .billing_budget(&assignment)
            .map_err(public_error),
        OwnerAction::DiscoverOauth {
            resource,
            metadata_url,
            client_id,
            scopes,
        } => desktop
            .runtime()?
            .discover_oauth(&resource, metadata_url.as_deref(), &client_id, scopes)
            .and_then(|v| {
                serde_json::to_value(v)
                    .map_err(|_| apr_connectors::DispatchError::InvalidConnection)
            })
            .map_err(|error| format!("OAuth discovery: {error}")),
        OwnerAction::StopService => desktop.client()?.request_owner_stop().map_err(public_error),
    }
}
fn runtime_executable(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|_| "EXECUTABLE_UNAVAILABLE")?;
    let bundled = current
        .parent()
        .ok_or("EXECUTABLE_UNAVAILABLE")?
        .join("apr");
    if bundled.is_file() {
        return Ok(bundled);
    }
    if let Ok(resources) = app.path().resource_dir() {
        let binary = resources.join("apr");
        if binary.is_file() {
            return Ok(binary);
        }
    }
    if cfg!(debug_assertions) {
        let development = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!("apr-{}", env!("APR_TARGET_TRIPLE")));
        if development.is_file() {
            return Ok(development);
        }
    }
    Err("Bundled APR executable missing; run the desktop preparation/build script".into())
}
fn main() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() == 1 && (arguments[0] == "--help" || arguments[0] == "-h") {
        println!("{}", launch::USAGE);
        return;
    }
    let explicit_root = launch::parse_args(&arguments).unwrap_or_else(|error| {
        eprintln!("{error}\n{}", launch::USAGE);
        std::process::exit(2);
    });
    let isolated = explicit_root.is_some();
    let previews = apps::Previews::default();
    let protocol = previews.clone();
    tauri::Builder::default()
        .manage(previews)
        .register_uri_scheme_protocol("cookbook-app", move |_, request| {
            protocol.response(request.uri().path())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(authorization::Authorizations::default())
        .manage(updates::UpdateState::default())
        .setup(move |app| {
            let root = launch::runtime_root(explicit_root.as_deref(), || {
                app.path()
                    .app_data_dir()
                    .map(|path| path.join("runtime"))
                    .map_err(|_| "Application data directory is unavailable".into())
            })
            .map_err(std::io::Error::other)?;
            let _ = Runtime::open(&root)?;
            let root = root.canonicalize()?;
            let selected = launch::selected_attachment(&root);
            let needs_setup = selected.is_none()
                || Runtime::open(&root)?.maintenance_status()?["paused_for_update"] == true;
            let executable = runtime_executable(app.handle()).map_err(std::io::Error::other)?;
            app.manage(Desktop {
                root,
                executable,
                selected: Arc::new(Mutex::new(selected)),
                install_plan: Arc::new(Mutex::new(None)),
                agent_plan: Arc::new(Mutex::new(None)),
                isolated,
            });
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .incognito(isolated)
            .title("CookBook APR")
            .inner_size(1280.0, 850.0)
            .min_inner_size(760.0, 580.0)
            .on_navigation(|url| {
                (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
                    || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
            })
            .build()?;
            updates::start_checks(app.handle().clone());
            if needs_setup {
                setup_window(app.handle()).map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop_status,
            scoped_call,
            open_setup,
            owner_request,
            authorization::oauth_begin,
            authorization::oauth_discover_begin,
            authorization::oauth_status,
            authorization::oauth_cancel,
            updates::update_check,
            updates::update_install,
            apps::app_preview_open
        ])
        .run(tauri::generate_context!())
        .expect("CookBook could not start");
}
