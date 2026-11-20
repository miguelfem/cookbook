use std::collections::BTreeMap;
use std::path::PathBuf;

use apr_runtime::computer::{ComputerOptions, DecisionSource};
use apr_runtime::{Result, RuntimeError};
use clap::{Subcommand, ValueEnum};
use serde_json::Value;

#[derive(Clone, Copy, ValueEnum)]
pub enum DecisionMode {
    Jev,
    Caller,
}

#[derive(Subcommand)]
pub enum ComputerCommand {
    /// Write a background browser recipe. Never open Chrome or install software.
    BrowserScaffold {
        directory: PathBuf,
        #[arg(long, default_value = "browser.task")]
        name: String,
        #[arg(long)]
        start_url: String,
        /// Explicit HTTP(S) origin; repeat for required first/third-party origins.
        #[arg(long = "origin", required = true)]
        origins: Vec<String>,
        /// Absolute path to an explicitly installed Playwright Chromium executable.
        #[arg(long)]
        browser_executable: PathBuf,
        /// JSON click/label/expected_text actions from a successful reference run.
        /// Each action remains its own durable step; no model calls on this path.
        #[arg(long)]
        recorded_plan: Option<PathBuf>,
        #[arg(long, default_value_t = 6)]
        max_steps: u32,
        #[arg(long, default_value_t = 300_000)]
        max_elapsed_ms: u64,
        #[arg(long, default_value_t = 0.8)]
        min_confidence: f64,
        #[arg(long, value_enum, default_value = "caller")]
        decisions: DecisionMode,
        #[arg(long, default_value = "jev-latest")]
        model: String,
        #[arg(long, default_value = "typesafe")]
        connection: String,
        #[arg(long)]
        success_text: Option<String>,
    },
    /// Write a new process bundle with upstream credit; never touch the screen.
    Scaffold {
        directory: PathBuf,
        #[arg(long, default_value = "computer.task")]
        name: String,
        /// Allowed macOS application bundle ID; repeat for each application.
        #[arg(long = "app", required = true)]
        apps: Vec<String>,
        #[arg(long, default_value_t = 6)]
        max_steps: u32,
        #[arg(long, default_value_t = 300_000)]
        max_elapsed_ms: u64,
        #[arg(long, default_value_t = 0.8)]
        min_confidence: f64,
        #[arg(long, default_value = "jev-latest")]
        model: String,
        #[arg(long, default_value = "typesafe")]
        connection: String,
        /// Caller mode uses saved agent requests and needs no Jev feature/account.
        #[arg(long, value_enum, default_value = "caller")]
        decisions: DecisionMode,
        /// Explicit non-secret text permitted for typing, as NAME=VALUE; repeatable.
        #[arg(long = "text")]
        texts: Vec<String>,
        /// Exact visible text that confirms completion; otherwise caller review is required.
        #[arg(long)]
        success_text: Option<String>,
    },
}

pub fn execute(command: ComputerCommand) -> Result<Value> {
    match command {
        ComputerCommand::BrowserScaffold {
            directory,
            name,
            start_url,
            origins,
            browser_executable,
            recorded_plan,
            max_steps,
            max_elapsed_ms,
            min_confidence,
            decisions,
            model,
            connection,
            success_text,
        } => {
            let actions: Option<Vec<Value>> = recorded_plan
                .map(|path| -> Result<_> {
                    if std::fs::metadata(&path)?.len() > 65536 {
                        return Err(RuntimeError::InvalidInput(
                            "recorded plan exceeds 64 KiB".into(),
                        ));
                    }
                    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
                })
                .transpose()?;
            apr_runtime::browser::scaffold(
                &directory,
                &apr_runtime::browser::BrowserOptions {
                    computer: ComputerOptions {
                        name,
                        max_steps: actions
                            .as_ref()
                            .map_or(max_steps, |actions| actions.len() as u32),
                        max_elapsed_ms,
                        min_confidence,
                        decision_source: match decisions {
                            DecisionMode::Jev => DecisionSource::Jev,
                            DecisionMode::Caller => DecisionSource::Caller,
                        },
                        model,
                        connection,
                        success_text,
                        ..ComputerOptions::default()
                    },
                    start_url,
                    allowed_origins: origins,
                    browser_executable,
                    recorded_actions: actions,
                },
            )
        }
        ComputerCommand::Scaffold {
            directory,
            name,
            apps,
            max_steps,
            max_elapsed_ms,
            min_confidence,
            model,
            connection,
            decisions,
            texts,
            success_text,
        } => {
            let mut allowed_text = BTreeMap::new();
            for text in texts {
                let (name, value) = text.split_once('=').ok_or_else(|| {
                    RuntimeError::InvalidInput("--text requires NAME=VALUE".into())
                })?;
                if allowed_text
                    .insert(name.to_owned(), value.to_owned())
                    .is_some()
                {
                    return Err(RuntimeError::InvalidInput("duplicate text name".into()));
                }
            }
            apr_runtime::computer::scaffold(
                &directory,
                &ComputerOptions {
                    name,
                    allowed_apps: apps,
                    max_steps,
                    max_elapsed_ms,
                    min_confidence,
                    model,
                    connection,
                    decision_source: match decisions {
                        DecisionMode::Jev => DecisionSource::Jev,
                        DecisionMode::Caller => DecisionSource::Caller,
                    },
                    allowed_text,
                    success_text,
                },
            )
        }
    }
}
