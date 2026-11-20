use std::path::{Path, PathBuf};

use apr_runtime::{Runtime, RuntimeError, maintenance::RetentionPlan};
use clap::{ArgAction, Subcommand};
use serde_json::{Value, json};

const MAX_PLAN_BYTES: usize = 16 * 1024 * 1024;

#[derive(Subcommand)]
pub enum StorageCommand {
    /// Report payload-free local storage totals and quota policy.
    Usage,
    /// Limit committed artifact object bytes, or explicitly remove that limit.
    Quota {
        #[arg(
            long,
            required_unless_present = "unlimited",
            conflicts_with = "unlimited"
        )]
        bytes: Option<u64>,
        #[arg(long)]
        unlimited: bool,
    },
    /// Retain a saved run or remove its explicit retention pin.
    PinRun {
        run: String,
        #[arg(long, action = ArgAction::Set)]
        pinned: bool,
    },
    /// Retain a registered artifact or remove its explicit retention pin.
    PinArtifact {
        id: String,
        #[arg(long, action = ArgAction::Set)]
        pinned: bool,
    },
    /// Preview eligible terminal runs and unreferenced objects, without deleting.
    RetentionPreview {
        #[arg(long)]
        terminal_before_ms: i64,
        #[arg(long)]
        orphan_before_ms: i64,
        /// Save a concrete plan to a new private file beneath the state directory.
        #[arg(long)]
        output_file: Option<PathBuf>,
    },
    /// Apply a reviewed retention plan only while references and policy still match.
    RetentionApply {
        #[arg(long)]
        plan_file: PathBuf,
    },
}

pub fn execute(state: &Path, command: StorageCommand) -> apr_runtime::Result<Value> {
    let mut runtime = Runtime::open(state)?;
    match command {
        StorageCommand::Usage => Ok(serde_json::to_value(runtime.storage_usage()?)?),
        StorageCommand::Quota {
            bytes,
            unlimited: _,
        } => runtime.set_artifact_quota(bytes),
        StorageCommand::PinRun { run, pinned } => {
            runtime.pin_run(&run, pinned)?;
            Ok(json!({"run_id":run,"pinned":pinned}))
        }
        StorageCommand::PinArtifact { id, pinned } => {
            runtime.pin_artifact(&id, pinned)?;
            Ok(json!({"artifact_id":id,"pinned":pinned}))
        }
        StorageCommand::RetentionPreview {
            terminal_before_ms,
            orphan_before_ms,
            output_file,
        } => {
            let plan = runtime.retention_preview(terminal_before_ms, orphan_before_ms)?;
            let value = serde_json::to_value(&plan)?;
            if serde_json::to_vec(&value)?.len() > MAX_PLAN_BYTES {
                return Err(RuntimeError::LimitExceeded(
                    "retention plan exceeds 16 MiB".into(),
                ));
            }
            if let Some(path) = output_file {
                let path = crate::save_private_json(state, &path, &value)?;
                Ok(
                    json!({"plan_file":path,"run_count":plan.run_ids.len(),"artifact_count":plan.artifact_ids.len(),"orphan_count":plan.orphan_object_names.len(),"bytes_reclaimable":plan.bytes_reclaimable}),
                )
            } else {
                Ok(value)
            }
        }
        StorageCommand::RetentionApply { plan_file } => {
            let plan: RetentionPlan =
                serde_json::from_slice(&crate::read_bounded(&plan_file, MAX_PLAN_BYTES)?)?;
            Ok(serde_json::to_value(runtime.retention_apply(&plan)?)?)
        }
    }
}
