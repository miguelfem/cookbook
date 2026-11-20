use std::path::Path;

use apr_core::routing::{MAX_CALL_DEPTH, MAX_CHILD_RUNS, MAX_GRAPH_VERSIONS, MAX_STEP_INSTANCES};
use apr_runtime::{Runtime, capacity::CapacityPolicy};
use clap::Subcommand;
use serde_json::{Value, json};

#[derive(Subcommand)]
pub enum CapacityCommand {
    /// Show owner capacity for new roots and non-overridable hard ceilings.
    Show,
    /// Replace owner capacity. Accepted runs retain their admitted limits.
    Set {
        /// Maximum requested nesting depth for newly accepted roots.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=MAX_CALL_DEPTH as i64))]
        max_call_depth: u32,
        /// Maximum requested child-run budget for newly accepted roots.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=MAX_CHILD_RUNS as i64))]
        max_child_runs: u32,
        /// Maximum requested total step-instance budget for newly accepted roots.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=MAX_STEP_INSTANCES as i64))]
        max_step_instances: u32,
        /// Maximum distinct frozen process versions, including the root.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=MAX_GRAPH_VERSIONS as i64))]
        max_graph_versions: u32,
    },
}

pub fn execute(state: &Path, command: CapacityCommand) -> apr_runtime::Result<Value> {
    let mut runtime = Runtime::open(state)?;
    let policy = match command {
        CapacityCommand::Show => runtime.capacity_policy()?,
        CapacityCommand::Set {
            max_call_depth,
            max_child_runs,
            max_step_instances,
            max_graph_versions,
        } => runtime.set_capacity_policy(CapacityPolicy {
            max_call_depth,
            max_child_runs,
            max_step_instances,
            max_graph_versions,
        })?,
    };
    Ok(
        json!({"policy":policy,"hard_ceilings":CapacityPolicy::HARD_CEILINGS,"applies_to":"new_roots","existing_runs_changed":false}),
    )
}
