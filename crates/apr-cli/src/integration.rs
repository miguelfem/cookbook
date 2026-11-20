use std::path::{Path, PathBuf};

use apr_integrations::global::{GlobalInstallOptions, GlobalManager, GlobalPlan};
use apr_integrations::{Client, InstallRequest, Manager, Plan};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::Value;

use crate::{CliError, read_bounded};

#[derive(Clone, Copy, ValueEnum)]
pub enum ClientChoice {
    Codex,
    Claude,
}
impl From<ClientChoice> for Client {
    fn from(value: ClientChoice) -> Self {
        match value {
            ClientChoice::Codex => Self::Codex,
            ClientChoice::Claude => Self::Claude,
        }
    }
}

#[derive(Args)]
pub struct SetupArgs {
    #[arg(long, value_enum)]
    client: ClientChoice,
    #[arg(long)]
    project: PathBuf,
    #[arg(long)]
    attachment_file: PathBuf,
    /// Installed APR executable; defaults to the currently running binary.
    #[arg(long)]
    executable: Option<PathBuf>,
}

impl SetupArgs {
    fn request(self) -> std::io::Result<InstallRequest> {
        Ok(InstallRequest {
            project_dir: self.project,
            attachment_file: self.attachment_file,
            executable: self
                .executable
                .map(Ok)
                .unwrap_or_else(std::env::current_exe)?,
            client: self.client.into(),
        })
    }
}

#[derive(Subcommand)]
pub enum IntegrationCommand {
    /// Preview one-time user-scoped APR setup without changing user settings.
    AgentPreview {
        #[arg(long, value_enum)]
        client: ClientChoice,
        /// Defaults to the current user's actual home directory.
        #[arg(long)]
        home: Option<PathBuf>,
        /// Installed APR executable; defaults to the currently running binary.
        #[arg(long)]
        executable: Option<PathBuf>,
        #[arg(long)]
        output_file: Option<PathBuf>,
        /// Include owner-reviewed Codex approval for apr_run in the managed entry.
        #[arg(long, default_value_t = false)]
        codex_trust_apr_run: bool,
    },
    /// Apply a reviewed one-time user-scoped setup plan.
    AgentApply {
        #[arg(long)]
        plan_file: PathBuf,
    },
    /// Remove only unchanged APR-owned global settings and skill.
    AgentUninstall {
        #[arg(long, value_enum)]
        client: ClientChoice,
        /// Defaults to the current user's actual home directory.
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Preview an owned project MCP entry and skill without changing either.
    Preview {
        #[command(flatten)]
        setup: SetupArgs,
        /// Save the nonsecret plan to a new private file inside the state directory.
        #[arg(long)]
        output_file: Option<PathBuf>,
    },
    /// Apply a reviewed plan only while its preimages are still current.
    Apply {
        #[arg(long)]
        plan_file: PathBuf,
    },
    /// Refresh an existing owned setup, preserving unrelated settings.
    Update {
        #[command(flatten)]
        setup: SetupArgs,
    },
    /// Check saved configuration, client availability, and authenticated APR service readiness.
    Doctor {
        #[arg(long, value_enum)]
        client: ClientChoice,
        #[arg(long)]
        project: PathBuf,
        /// Optional client binary for a bounded --version probe; never runs a model.
        #[arg(long)]
        client_executable: Option<PathBuf>,
    },
    /// Remove only the unchanged owned APR entry and skill; retain state and attachments.
    Uninstall {
        #[arg(long, value_enum)]
        client: ClientChoice,
        #[arg(long)]
        project: PathBuf,
    },
}

pub fn execute(state_dir: &Path, command: IntegrationCommand) -> Result<Value, CliError> {
    let value = match command {
        IntegrationCommand::AgentPreview {
            client,
            home,
            executable,
            output_file,
            codex_trust_apr_run,
        } => {
            // A one-time connection does not require an existing project or
            // attachment to initialize the private runtime store.
            apr_runtime::Runtime::open(state_dir)?;
            let home = home
                .map(Ok)
                .unwrap_or_else(home_dir)
                .map_err(apr_runtime::RuntimeError::from)?;
            let executable = executable
                .map(Ok)
                .unwrap_or_else(std::env::current_exe)
                .map_err(apr_runtime::RuntimeError::from)?;
            let plan = GlobalManager::open(state_dir)?.preview_with_options(
                &home,
                &executable,
                client.into(),
                GlobalInstallOptions {
                    trust_apr_run: codex_trust_apr_run,
                },
            )?;
            let value = serde_json::to_value(plan).map_err(apr_runtime::RuntimeError::from)?;
            if let Some(path) = output_file {
                crate::save_private_json(state_dir, &path, &value)?;
            }
            value
        }
        IntegrationCommand::AgentApply { plan_file } => {
            let plan: GlobalPlan = serde_json::from_slice(&read_bounded(&plan_file, 1024 * 1024)?)
                .map_err(apr_runtime::RuntimeError::from)?;
            serde_json::to_value(GlobalManager::open(state_dir)?.apply(&plan)?)
                .map_err(apr_runtime::RuntimeError::from)?
        }
        IntegrationCommand::AgentUninstall { client, home } => {
            let home = home
                .map(Ok)
                .unwrap_or_else(home_dir)
                .map_err(apr_runtime::RuntimeError::from)?;
            serde_json::to_value(GlobalManager::open(state_dir)?.uninstall(&home, client.into())?)
                .map_err(apr_runtime::RuntimeError::from)?
        }
        IntegrationCommand::Preview { setup, output_file } => {
            let manager = Manager::open(state_dir)?;
            let request = setup.request().map_err(apr_runtime::RuntimeError::from)?;
            let plan = manager.preview(&request)?;
            let value = serde_json::to_value(plan).map_err(apr_runtime::RuntimeError::from)?;
            if let Some(path) = output_file {
                crate::save_private_json(state_dir, &path, &value)?;
            }
            value
        }
        IntegrationCommand::Apply { plan_file } => {
            let manager = Manager::open(state_dir)?;
            let plan: Plan = serde_json::from_slice(&read_bounded(&plan_file, 1024 * 1024)?)
                .map_err(apr_runtime::RuntimeError::from)?;
            serde_json::to_value(manager.apply(&plan)?).map_err(apr_runtime::RuntimeError::from)?
        }
        IntegrationCommand::Update { setup } => {
            let manager = Manager::open(state_dir)?;
            let request = setup.request().map_err(apr_runtime::RuntimeError::from)?;
            serde_json::to_value(manager.update(&request)?)
                .map_err(apr_runtime::RuntimeError::from)?
        }
        IntegrationCommand::Doctor {
            client,
            project,
            client_executable,
        } => {
            let manager = Manager::open(state_dir)?;
            let request = manager.installed_request(&project, client.into())?;
            serde_json::to_value(
                manager.doctor_with_client(&request, client_executable.as_deref())?,
            )
            .map_err(apr_runtime::RuntimeError::from)?
        }
        IntegrationCommand::Uninstall { client, project } => {
            let manager = Manager::open(state_dir)?;
            serde_json::to_value(manager.uninstall(&project, client.into())?)
                .map_err(apr_runtime::RuntimeError::from)?
        }
    };
    Ok(value)
}

fn home_dir() -> std::io::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is unavailable"))
}
