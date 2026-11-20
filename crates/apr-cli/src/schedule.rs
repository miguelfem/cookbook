use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use apr_runtime::service::ServiceClient;
use apr_runtime::{MAX_JSON_BYTES, Result, RuntimeError};
use clap::{Subcommand, ValueEnum};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Clone, Copy, ValueEnum)]
pub enum Platform {
    Launchd,
    Systemd,
}

#[derive(Subcommand)]
pub enum ScheduleCommand {
    /// Create an immutable schedule. Times use RFC3339; cron uses UTC.
    Create {
        #[arg(long)]
        attachment: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        process_ref: String,
        #[arg(long)]
        input_file: PathBuf,
        #[arg(long)]
        overrides_file: Option<PathBuf>,
        #[arg(long)]
        request_key: String,
        /// Run once at this RFC3339 timestamp, including Z or an explicit offset.
        #[arg(long, required_unless_present_any = ["every", "cron"], conflicts_with_all = ["every", "cron"])]
        at: Option<String>,
        /// Repeat every N seconds; first run is one interval from now by default.
        #[arg(long, conflicts_with_all = ["at", "cron"])]
        every: Option<u64>,
        /// Standard five-field numeric cron expression, evaluated in UTC.
        #[arg(long, conflicts_with_all = ["at", "every"])]
        cron: Option<String>,
        /// First nominal occurrence for interval jobs, as RFC3339.
        #[arg(long, requires = "every")]
        start_at: Option<String>,
    },
    List {
        #[arg(long)]
        attachment: PathBuf,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u16,
    },
    Inspect {
        schedule_id: String,
        #[arg(long)]
        attachment: PathBuf,
    },
    /// Stop future occurrences; an already accepted run is unaffected.
    Pause {
        schedule_id: String,
        #[arg(long)]
        attachment: PathBuf,
    },
    Resume {
        schedule_id: String,
        #[arg(long)]
        attachment: PathBuf,
    },
    /// Remove future scheduling; retain history and any already accepted run.
    Delete {
        schedule_id: String,
        #[arg(long)]
        attachment: PathBuf,
    },
    /// Write an optional login-service file. Does not install or enable it.
    ServiceFile {
        #[arg(long, value_enum)]
        platform: Platform,
        #[arg(long)]
        output: PathBuf,
    },
}

pub fn execute(state: &Path, command: ScheduleCommand) -> Result<Value> {
    let (attachment, action, args) = match command {
        ScheduleCommand::Create {
            attachment,
            name,
            process_ref,
            input_file,
            overrides_file,
            request_key,
            at,
            every,
            cron,
            start_at,
        } => {
            let input: Value =
                serde_json::from_slice(&crate::read_bounded(&input_file, MAX_JSON_BYTES)?)?;
            let overrides = match overrides_file {
                Some(path) => serde_json::from_slice(&crate::read_bounded(&path, MAX_JSON_BYTES)?)?,
                None => json!({}),
            };
            let trigger = match (at, every, cron) {
                (Some(at), None, None) => json!({"kind":"once","at_ms":timestamp(&at)?}),
                (None, Some(seconds), None) => {
                    let mut trigger = json!({"kind":"interval","every_seconds":seconds});
                    if let Some(start) = start_at {
                        trigger["start_at_ms"] = json!(timestamp(&start)?);
                    }
                    trigger
                }
                (None, None, Some(expression)) => json!({"kind":"cron","expression":expression}),
                _ => {
                    return Err(RuntimeError::InvalidInput(
                        "choose exactly one schedule trigger".into(),
                    ));
                }
            };
            (
                attachment,
                "manage",
                json!({"action":"schedule.create","name":name,"process_ref":process_ref,
                "input":input,"config_overrides":overrides,"request_key":request_key,"trigger":trigger}),
            )
        }
        ScheduleCommand::List {
            attachment,
            after,
            limit,
        } => {
            let mut args = json!({"view":"schedules","limit":limit});
            if let Some(after) = after {
                args["after"] = json!(after);
            }
            (attachment, "inspect", args)
        }
        ScheduleCommand::Inspect {
            schedule_id,
            attachment,
        } => (
            attachment,
            "inspect",
            json!({"view":"schedule","schedule_id":schedule_id}),
        ),
        ScheduleCommand::Pause {
            schedule_id,
            attachment,
        } => (
            attachment,
            "manage",
            json!({"action":"schedule.pause","schedule_id":schedule_id}),
        ),
        ScheduleCommand::Resume {
            schedule_id,
            attachment,
        } => (
            attachment,
            "manage",
            json!({"action":"schedule.resume","schedule_id":schedule_id}),
        ),
        ScheduleCommand::Delete {
            schedule_id,
            attachment,
        } => (
            attachment,
            "manage",
            json!({"action":"schedule.delete","schedule_id":schedule_id}),
        ),
        ScheduleCommand::ServiceFile { platform, output } => {
            return service_file(state, platform, &output);
        }
    };
    ServiceClient::from_file(&attachment)?.call(action, args, 64 * 1024)
}

fn timestamp(value: &str) -> Result<i64> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
        RuntimeError::InvalidInput("timestamp requires RFC3339 with an explicit offset".into())
    })?;
    i64::try_from(parsed.unix_timestamp_nanos() / 1_000_000)
        .map_err(|_| RuntimeError::InvalidInput("timestamp is out of range".into()))
}

fn service_file(state: &Path, platform: Platform, output: &Path) -> Result<Value> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let state = state.canonicalize()?;
    let executable = executable
        .to_str()
        .ok_or_else(|| RuntimeError::InvalidInput("non-UTF8 executable path".into()))?;
    let state = state
        .to_str()
        .ok_or_else(|| RuntimeError::InvalidInput("non-UTF8 state path".into()))?;
    if [executable, state]
        .iter()
        .any(|path| path.chars().any(char::is_control))
    {
        return Err(RuntimeError::InvalidInput(
            "service paths must not contain control characters".into(),
        ));
    }
    let (contents, filename) = match platform {
        Platform::Launchd => {
            let xml = |s: &str| {
                s.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
                    .replace('\'', "&apos;")
            };
            (
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>local.apr.service</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--state-dir</string><string>{}</string><string>service</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>ThrottleInterval</key><integer>10</integer>\n</dict></plist>\n",
                    xml(executable),
                    xml(state)
                ),
                "local.apr.service.plist",
            )
        }
        Platform::Systemd => {
            let quote = |s: &str| {
                format!(
                    "\"{}\"",
                    s.replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('%', "%%")
                        .replace('$', "$$")
                )
            };
            (
                format!(
                    "[Unit]\nDescription=APR local workflow service\n\n[Service]\nType=simple\nExecStart={} --state-dir {} service\nRestart=on-failure\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n",
                    quote(executable),
                    quote(state)
                ),
                "apr.service",
            )
        }
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    Ok(
        json!({"file":output,"suggested_filename":filename,"installed":false,
        "note":"Review and install as a user login service. Jobs require this device awake, and desktop jobs require an unlocked graphical session."}),
    )
}
