use std::io::{self, Read};
use std::path::{Path, PathBuf};

use apr_runtime::{MAX_JSON_BYTES, Runtime, RuntimeError, worker_policy::WorkerPolicy};
use clap::{ArgAction, Subcommand, ValueEnum};
use serde_json::Value;

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Set ordinary project defaults for a registered process (not credentials).
    ProjectSet {
        #[arg(long)]
        project: String,
        #[arg(long)]
        process_ref: String,
        #[arg(long)]
        values_file: PathBuf,
    },
}
#[derive(Subcommand)]
pub enum PermissionCommand {
    /// Permit or revoke process promotion between two same-project assignments.
    Promote {
        #[arg(long)]
        source: String,
        #[arg(long)]
        target: String,
        #[arg(long, action=ArgAction::Set)]
        enabled: bool,
    },
}
#[derive(Clone, Copy, ValueEnum)]
pub enum ExecutionKind {
    Python,
    Connection,
}
#[derive(Subcommand)]
pub enum ExecutionCommand {
    /// Show the device-wide simultaneous job policy and its revision.
    WorkerPolicyShow,
    /// Show running claims, approximate queued steps, and configured limits.
    WorkerStatus,
    /// Replace the owner policy after reviewing the JSON file and current revision.
    WorkerPolicySet {
        #[arg(long)]
        policy_file: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
    /// Show local credential storage metadata; never displays secrets.
    CredentialStoreStatus,
    /// Explicitly copy saved credentials from the legacy OS store to local TOML.
    /// The OS may request consent once per existing entry. Never runs implicitly.
    MigratePlatformCredentials,
    /// List reusable tools saved by the owner, without credential contents.
    LibraryList {
        #[arg(long)]
        after: Option<String>,
    },
    /// Owner-only saved accounts, independent of globally shared tools.
    AccountList {
        #[arg(long)]
        after: Option<String>,
    },
    /// Revoke an account and stop future calls through all of its tool grants.
    AccountRevoke {
        #[arg(long)]
        id: String,
        #[arg(long)]
        revision: u64,
    },
    /// Public OAuth metadata only; no registration, login or grant is created.
    OauthDiscover {
        #[arg(long)]
        resource: String,
        #[arg(long)]
        metadata_url: Option<String>,
        #[arg(long, default_value = "")]
        client_id: String,
        #[arg(long, value_delimiter = ',')]
        scopes: Vec<String>,
    },
    /// Make an existing approved tool and its stored credential available to all projects.
    LibrarySave {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        revision: u64,
    },
    /// Disable a saved tool and all unchanged project bindings derived from it.
    LibraryDisable {
        #[arg(long)]
        id: String,
        #[arg(long)]
        revision: u64,
    },
    /// Whole-period retained billing ledger; not a paginated subtotal.
    Billing {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        starts_at_ms: i64,
        #[arg(long)]
        ends_at_ms: i64,
    },
    /// Set an owner-reviewed dispatch reservation policy from a nonsecret JSON file.
    BudgetSet {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        policy_file: PathBuf,
    },
    BudgetShow {
        #[arg(long)]
        assignment: String,
    },
    InvoiceImport {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        invoice_file: PathBuf,
    },
    McpInputs {
        #[arg(long)]
        assignment: String,
    },
    McpRespond {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        response_file: PathBuf,
    },
    PriceSourceSet {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        source_file: PathBuf,
        #[arg(long,action=ArgAction::Set)]
        enabled: bool,
    },
    PriceCheck {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        name: String,
    },
    PriceApprove {
        #[arg(long)]
        request: String,
        #[arg(long)]
        digest: String,
    },
    /// Browser sign-in for a reviewed proposal; codes and tokens never print.
    OauthAuthorize {
        #[arg(long)]
        request: String,
        #[arg(long)]
        digest: String,
        #[arg(long)]
        open_browser: bool,
    },
    MaintenanceStatus,
    MaintenancePrepare {
        #[arg(long)]
        target_version: String,
    },
    MaintenanceFinish,

    /// List recipe access requests; this does not grant them.
    Requests {
        #[arg(long)]
        assignment: String,
    },
    /// Submit a reviewed public connection proposal. Never put a key in the file.
    RequestConnection {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        proposal_file: PathBuf,
    },
    /// Approve the exact reviewed proposal hash. Secret bytes come only from stdin.
    ApproveConnection {
        #[arg(long)]
        request: String,
        #[arg(long)]
        digest: String,
        #[arg(long)]
        secret_stdin: bool,
        #[arg(long)]
        all_projects: bool,
        #[arg(
            long,
            requires = "saved_credential_revision",
            conflicts_with = "secret_stdin"
        )]
        saved_credential: Option<String>,
        #[arg(long, requires = "saved_credential")]
        saved_credential_revision: Option<u64>,
        #[arg(long, requires = "saved_account_revision", conflicts_with_all = ["secret_stdin", "saved_credential"])]
        saved_account: Option<String>,
        #[arg(long, requires = "saved_account")]
        saved_account_revision: Option<u64>,
        /// Exact owner preview digest for reusing an account with changed adapter code.
        #[arg(long, requires = "saved_account")]
        account_review_digest: Option<String>,
    },
    /// Review a local adapter update against a saved account; does not read its key.
    ReviewAccountReuse {
        #[arg(long)]
        request: String,
        #[arg(long)]
        digest: String,
        #[arg(long)]
        account: String,
        #[arg(long)]
        revision: u64,
    },
    RejectConnection {
        #[arg(long)]
        request: String,
        #[arg(long)]
        digest: String,
    },
    /// Discover tools without granting execution; native discovery requires explicit trust.
    McpDiscover {
        #[arg(long)]
        connection_file: PathBuf,
        #[arg(long)]
        secret_stdin: bool,
        #[arg(long)]
        trust_native: bool,
        #[arg(long,default_value="tools",value_parser=["tools","resources","prompts"])]
        kind: String,
        /// Negotiate only metadata; return a concrete revision for later approval.
        #[arg(long)]
        automatic_protocol: bool,
        #[arg(
            long,
            requires = "saved_account_revision",
            conflicts_with = "secret_stdin"
        )]
        saved_account: Option<String>,
        #[arg(long, requires = "saved_account")]
        saved_account_revision: Option<u64>,
    },
    /// Resolve and hash an executable without running it.
    McpPin {
        executable: PathBuf,
    },
    /// Test the agent-authored cost model against selected payloads; no external call.
    CostPreview {
        #[arg(long)]
        connection_file: PathBuf,
        #[arg(long)]
        request_file: PathBuf,
        #[arg(long)]
        response_file: PathBuf,
    },
    /// Grant an assignment access to a pinned native Python interpreter.
    GrantPython {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        interpreter: PathBuf,
    },
    /// Configure a scoped connection; optional secret bytes come only from stdin.
    ConfigureConnection {
        #[arg(long)]
        assignment: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        connection_file: PathBuf,
        #[arg(long)]
        secret_stdin: bool,
    },
    Revoke {
        #[arg(long)]
        assignment: String,
        #[arg(long, value_enum)]
        kind: ExecutionKind,
        #[arg(long)]
        name: String,
    },
    /// Acknowledge current grants for a waiting run after reconciling uncertain effects.
    Rebind {
        run: String,
    },
    /// Resolve an uncertain effect using an observed result or explicit evidence.
    Reconcile {
        #[arg(long)]
        request_file: PathBuf,
    },
}

pub fn config(state: &Path, command: ConfigCommand) -> apr_runtime::Result<Value> {
    match command {
        ConfigCommand::ProjectSet {
            project,
            process_ref,
            values_file,
        } => {
            let values =
                serde_json::from_slice(&crate::read_bounded(&values_file, MAX_JSON_BYTES)?)?;
            Runtime::open(state)?.set_project_config(&project, &process_ref, values)
        }
    }
}
pub fn permission(state: &Path, command: PermissionCommand) -> apr_runtime::Result<Value> {
    match command {
        PermissionCommand::Promote {
            source,
            target,
            enabled,
        } => Runtime::open(state)?.set_promotion_grant(&source, &target, enabled),
    }
}
pub fn execution(state: &Path, command: ExecutionCommand) -> apr_runtime::Result<Value> {
    match command {
        ExecutionCommand::WorkerPolicyShow => Ok(serde_json::to_value(
            Runtime::open(state)?.worker_policy()?,
        )?),
        ExecutionCommand::WorkerStatus => Runtime::open(state)?.worker_status(),
        ExecutionCommand::WorkerPolicySet {
            policy_file,
            expected_revision,
        } => {
            let policy: WorkerPolicy =
                serde_json::from_slice(&crate::read_bounded(&policy_file, MAX_JSON_BYTES)?)?;
            Ok(serde_json::to_value(
                Runtime::open(state)?.set_worker_policy(policy, expected_revision)?,
            )?)
        }
        ExecutionCommand::CredentialStoreStatus => Runtime::open(state)?.credential_store_status(),
        ExecutionCommand::MigratePlatformCredentials => {
            Runtime::open(state)?.migrate_platform_credentials()
        }
        ExecutionCommand::Billing {
            assignment,
            starts_at_ms,
            ends_at_ms,
        } => Runtime::open(state)?.billing_period(&assignment, starts_at_ms, ends_at_ms),
        ExecutionCommand::BudgetSet {
            assignment,
            policy_file,
        } => Runtime::open(state)?.set_billing_budget(
            &assignment,
            serde_json::from_slice(&crate::read_bounded(&policy_file, MAX_JSON_BYTES)?)?,
        ),
        ExecutionCommand::BudgetShow { assignment } => {
            Runtime::open(state)?.billing_budget(&assignment)
        }
        ExecutionCommand::InvoiceImport {
            assignment,
            invoice_file,
        } => Runtime::open(state)?.import_invoice(
            &assignment,
            serde_json::from_slice(&crate::read_bounded(&invoice_file, MAX_JSON_BYTES)?)?,
        ),
        ExecutionCommand::McpInputs { assignment } => {
            Runtime::open(state)?.mcp_interactions(&assignment, None)
        }
        ExecutionCommand::McpRespond {
            assignment,
            response_file,
        } => Runtime::open(state)?.owner_respond_mcp(
            &assignment,
            serde_json::from_slice(&crate::read_bounded(&response_file, MAX_JSON_BYTES)?)?,
        ),
        ExecutionCommand::PriceSourceSet {
            assignment,
            name,
            source_file,
            enabled,
        } => Runtime::open(state)?.set_price_source(
            &assignment,
            &name,
            serde_json::from_slice(&crate::read_bounded(&source_file, MAX_JSON_BYTES)?)?,
            enabled,
        ),
        ExecutionCommand::PriceCheck { assignment, name } => {
            Runtime::open(state)?.check_price_source(&assignment, &name)
        }
        ExecutionCommand::PriceApprove { request, digest } => {
            Runtime::open(state)?.approve_price_review(&request, &digest)
        }
        ExecutionCommand::MaintenanceStatus => Runtime::open(state)?.maintenance_status(),
        ExecutionCommand::MaintenancePrepare { target_version } => {
            Runtime::open(state)?.prepare_desktop_update(&target_version)
        }
        ExecutionCommand::MaintenanceFinish => Runtime::open(state)?.finish_desktop_update(),
        ExecutionCommand::OauthAuthorize {
            request,
            digest,
            open_browser,
        } => {
            if !open_browser {
                return Err(RuntimeError::InvalidInput(
                    "Review this exact request and explicitly pass --open-browser".into(),
                ));
            }
            let mut runtime = Runtime::open(state)?;
            let cfg = runtime.oauth_request_config(&request, &digest)?;
            let (pending, url) =
                apr_connectors::oauth::PendingAuthorization::begin(cfg).map_err(|error| {
                    RuntimeError::State(format!("Cannot prepare browser sign-in: {error}"))
                })?;
            #[cfg(target_os = "macos")]
            let opener = "/usr/bin/open";
            #[cfg(target_os = "linux")]
            let opener = "/usr/bin/xdg-open";
            let mut child = std::process::Command::new(opener)
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            let tokens = pending
                .finish(&std::sync::atomic::AtomicBool::new(false))
                .map_err(|_| RuntimeError::State("Authorization expired or was rejected".into()))?;
            runtime.approve_connection(&request, &digest, Some(tokens))
        }

        ExecutionCommand::Requests { assignment } => {
            Runtime::open(state)?.connection_requests(&assignment)
        }
        ExecutionCommand::RequestConnection {
            assignment,
            proposal_file,
        } => {
            let proposal =
                serde_json::from_slice(&crate::read_bounded(&proposal_file, MAX_JSON_BYTES)?)?;
            Runtime::open(state)?.propose_connection(&assignment, proposal)
        }
        ExecutionCommand::ApproveConnection {
            request,
            digest,
            secret_stdin,
            all_projects,
            saved_credential,
            saved_credential_revision,
            saved_account,
            saved_account_revision,
            account_review_digest,
        } => Runtime::open(state)?.approve_connection_with_options(
            &request,
            &digest,
            read_secret(secret_stdin)?,
            apr_runtime::connection_library::ApprovalOptions {
                all_projects,
                saved_credential: saved_credential.zip(saved_credential_revision).map(
                    |(library_id, revision)| apr_runtime::connection_library::SavedCredential {
                        library_id,
                        revision,
                    },
                ),
                saved_account: saved_account
                    .clone()
                    .zip(saved_account_revision)
                    .filter(|_| account_review_digest.is_none())
                    .map(
                        |(account_id, revision)| apr_runtime::connection_library::SavedAccount {
                            account_id,
                            revision,
                        },
                    ),
                reviewed_account_reuse: saved_account
                    .zip(saved_account_revision)
                    .zip(account_review_digest)
                    .map(|((account_id, revision), review_digest)| {
                        apr_runtime::connection_library::ReviewedAccountReuse {
                            account_id,
                            revision,
                            review_digest,
                        }
                    }),
            },
        ),
        ExecutionCommand::ReviewAccountReuse {
            request,
            digest,
            account,
            revision,
        } => Runtime::open(state)?
            .preview_reviewed_account_reuse(&request, &digest, &account, revision),
        ExecutionCommand::LibraryList { after } => {
            Runtime::open(state)?.connection_library(after.as_deref(), true)
        }
        ExecutionCommand::AccountList { after } => {
            Runtime::open(state)?.saved_accounts(after.as_deref())
        }
        ExecutionCommand::AccountRevoke { id, revision } => {
            Runtime::open(state)?.revoke_saved_account(&id, revision)
        }
        ExecutionCommand::OauthDiscover {
            resource,
            metadata_url,
            client_id,
            scopes,
        } => {
            let config = Runtime::open(state)?
                .discover_oauth(&resource, metadata_url.as_deref(), &client_id, scopes)
                .map_err(|error| RuntimeError::InvalidInput(format!("OAuth discovery: {error}")))?;
            Ok(
                serde_json::json!({"status":"authentication_required","config":config,"grant_created":false}),
            )
        }
        ExecutionCommand::LibrarySave {
            assignment,
            name,
            revision,
        } => Runtime::open(state)?.save_connection_to_library(&assignment, &name, revision),
        ExecutionCommand::LibraryDisable { id, revision } => {
            Runtime::open(state)?.disable_library_connection(&id, revision)
        }
        ExecutionCommand::RejectConnection { request, digest } => {
            Runtime::open(state)?.reject_connection(&request, &digest)
        }
        ExecutionCommand::McpDiscover {
            connection_file,
            secret_stdin,
            trust_native,
            kind,
            automatic_protocol,
            saved_account,
            saved_account_revision,
        } => {
            let connection: apr_connectors::Connection =
                serde_json::from_slice(&crate::read_bounded(&connection_file, MAX_JSON_BYTES)?)?;
            if connection.mcp.as_ref().is_some_and(|m| m.is_stdio()) && !trust_native {
                return Err(RuntimeError::InvalidInput("native MCP discovery executes code with your OS privileges; review it and explicitly pass --trust-native".into()));
            }
            let runtime = Runtime::open(state)?;
            if let Some(account) = saved_account {
                runtime.discover_mcp_with_account_protocol(
                    &connection,
                    &account,
                    saved_account_revision.ok_or_else(|| {
                        RuntimeError::InvalidInput("saved account revision required".into())
                    })?,
                    &kind,
                    automatic_protocol,
                )
            } else {
                runtime.discover_mcp_kind_with_protocol(
                    &connection,
                    read_secret(secret_stdin)?,
                    &kind,
                    automatic_protocol,
                )
            }
        }
        ExecutionCommand::McpPin { executable } => {
            if !executable.is_absolute() {
                return Err(RuntimeError::InvalidInput(
                    "an absolute executable path is required".into(),
                ));
            }
            let digest = apr_connectors::mcp::pin_executable(&executable)
                .map_err(|_| RuntimeError::InvalidInput("cannot pin this executable".into()))?;
            Ok(serde_json::json!({"executable":executable,"sha256":digest}))
        }
        ExecutionCommand::CostPreview {
            connection_file,
            request_file,
            response_file,
        } => apr_runtime::setup::preview_cost(
            serde_json::from_slice(&crate::read_bounded(&connection_file, MAX_JSON_BYTES)?)?,
            serde_json::from_slice(&crate::read_bounded(&request_file, MAX_JSON_BYTES)?)?,
            serde_json::from_slice(&crate::read_bounded(&response_file, MAX_JSON_BYTES)?)?,
        ),
        ExecutionCommand::GrantPython {
            assignment,
            interpreter,
        } => Runtime::open(state)?.grant_python(&assignment, &interpreter),
        ExecutionCommand::ConfigureConnection {
            assignment,
            name,
            connection_file,
            secret_stdin,
        } => {
            let connection =
                serde_json::from_slice(&crate::read_bounded(&connection_file, MAX_JSON_BYTES)?)?;
            let secret = if secret_stdin {
                let mut bytes = Vec::new();
                let read = io::stdin().lock().take(8193).read_to_end(&mut bytes);
                // Secret owns a zeroizing buffer even when validation fails.
                let secret = apr_connectors::Secret::new(bytes);
                read?;
                Some(secret.map_err(|_| {
                    RuntimeError::InvalidInput(
                        "secret must be 1–8192 printable ASCII bytes without whitespace".into(),
                    )
                })?)
            } else {
                None
            };
            Runtime::open(state)?.configure_connection(&assignment, &name, connection, secret)
        }
        ExecutionCommand::Revoke {
            assignment,
            kind,
            name,
        } => Runtime::open(state)?.revoke_execution(
            &assignment,
            match kind {
                ExecutionKind::Python => "python",
                ExecutionKind::Connection => "connection",
            },
            &name,
        ),
        ExecutionCommand::Rebind { run } => Runtime::open(state)?.rebind_execution(&run),
        ExecutionCommand::Reconcile { request_file } => {
            let request =
                serde_json::from_slice(&crate::read_bounded(&request_file, MAX_JSON_BYTES)?)?;
            Runtime::open(state)?.reconcile(&request)
        }
    }
}

fn read_secret(enabled: bool) -> apr_runtime::Result<Option<apr_connectors::Secret>> {
    if !enabled {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    let read = io::stdin().lock().take(8193).read_to_end(&mut bytes);
    let secret = apr_connectors::Secret::new(bytes);
    read?;
    secret.map(Some).map_err(|_| {
        RuntimeError::InvalidInput(
            "secret must be 1–8192 printable ASCII bytes without whitespace".into(),
        )
    })
}
