mod adapter;
mod capacity;
mod clients;
mod computer;
mod inspector;
mod integration;
mod maintenance;
mod owner;
mod schedule;

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use apr_core::{ArtifactRef, MAX_MANIFEST_BYTES, admit_json};
use apr_runtime::service::AttachmentFile;
use apr_runtime::{MAX_JSON_BYTES, Runtime, RuntimeError, Submission};
use clap::{Parser, Subcommand, ValueEnum, error::ErrorKind};
use serde_json::{Value, json};

const DEFAULT_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Parser)]
#[command(
    name = "apr",
    version,
    about = "Run and resume durable local processes"
)]
struct Cli {
    /// Local runtime state directory.
    #[arg(long, global = true, default_value = ".apr/local")]
    state_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Prepare and check a reviewed local MCP adapter; installs or grants nothing.
    Adapter {
        #[command(subcommand)]
        command: adapter::AdapterCommand,
    },
    /// Maintainer/owner public OAuth client identity; never contains account secrets.
    ConnectionClients {
        #[command(subcommand)]
        command: clients::ClientCommand,
    },
    /// Run a scoped improvement manage/inspect action from a bounded JSON file.
    Improvement {
        #[arg(long)]
        attachment: PathBuf,
        #[arg(long)]
        request_file: PathBuf,
        /// Use the read-only inspect interface instead of author manage.
        #[arg(long)]
        inspect: bool,
    },
    /// Manage device-owner execution capacity for newly accepted roots.
    Capacity {
        #[command(subcommand)]
        command: capacity::CapacityCommand,
    },
    /// Schedule an assignment's registered processes through the local service.
    Schedule {
        #[command(subcommand)]
        command: schedule::ScheduleCommand,
    },
    /// Scaffold an optional bounded computer-use process; installs or grants nothing.
    Computer {
        #[command(subcommand)]
        command: computer::ComputerCommand,
    },
    /// Save a private checksummed database and artifact backup to a new absolute path.
    Backup {
        #[arg(long)]
        output: PathBuf,
    },
    /// Restore a backup into the new absent absolute --state-dir destination.
    Restore { backup: PathBuf },
    /// Inspect storage and explicitly manage retention through owner controls.
    Storage {
        #[command(subcommand)]
        command: maintenance::StorageCommand,
    },
    /// Serve the assignment inspector on loopback until this process exits.
    Inspector {
        #[arg(long)]
        attachment: PathBuf,
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Configure ordinary project values through trusted owner controls.
    Config {
        #[command(subcommand)]
        command: owner::ConfigCommand,
    },
    /// Manage grants that attachments cannot give themselves.
    Permission {
        #[command(subcommand)]
        command: owner::PermissionCommand,
    },
    /// Grant, revoke, rebind, or reconcile independent native/provider execution.
    Execution {
        #[command(subcommand)]
        command: owner::ExecutionCommand,
    },
    /// Manage project-local Codex or Claude Code settings and the APR skill.
    Integrate {
        #[command(subcommand)]
        command: integration::IntegrationCommand,
    },
    /// Trust a local project path for assignment-scoped operations.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Create or close an explicit assignment within a project.
    Assignment {
        #[command(subcommand)]
        command: AssignmentCommand,
    },
    /// Create or revoke a private client attachment.
    Attachment {
        #[command(subcommand)]
        command: AttachmentCommand,
    },
    /// Run the local service in the foreground (normally started on demand).
    Service,
    /// Serve a fixed attachment or the one-time user-level project connection.
    Mcp {
        #[arg(
            long,
            required_unless_present = "projects",
            conflicts_with = "projects"
        )]
        attachment_file: Option<PathBuf>,
        #[arg(long)]
        projects: bool,
    },
    /// Inspect a process manifest without creating runtime state.
    Process {
        #[command(subcommand)]
        command: ProcessCommand,
    },
    /// Start or recover a run using an idempotent request key.
    Run {
        manifest: PathBuf,
        #[arg(long)]
        input_file: PathBuf,
        #[arg(long)]
        request_key: String,
    },
    /// Read a saved run and its pending request.
    Inspect { run: String },
    /// Advance saved work until it finishes or needs a caller result.
    Continue { run: String },
    /// Renew a pending caller handoff, invalidating its previous token.
    Renew { run: String },
    /// Submit a complete result envelope for a saved agent request.
    Resume {
        run: String,
        #[arg(long)]
        result_file: PathBuf,
    },
    /// Read a bounded page of saved run events.
    Events {
        run: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Import and export immutable artifact snapshots.
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommand,
    },
}

#[derive(Subcommand)]
enum ProjectCommand {
    Attach {
        path: PathBuf,
    },
    /// Reattach restored project identity to a local directory; revoke old grants.
    Relocate {
        id: String,
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum AssignmentCommand {
    Create {
        #[arg(long)]
        project: String,
        #[arg(long)]
        name: String,
    },
    Close {
        id: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum AttachmentMode {
    Author,
    Run,
}

impl AttachmentMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Run => "run",
        }
    }
}

#[derive(Subcommand)]
enum AttachmentCommand {
    Create {
        #[arg(long)]
        assignment: String,
        #[arg(long, value_enum)]
        mode: AttachmentMode,
        #[arg(long)]
        output_file: PathBuf,
    },
    Revoke {
        id: String,
    },
}

#[derive(Subcommand)]
enum ProcessCommand {
    /// Check the supported manifest and report its immutable version.
    Validate { manifest: PathBuf },
}

#[derive(Subcommand)]
enum ArtifactCommand {
    /// Snapshot a regular file in the local artifact store.
    Import {
        path: PathBuf,
        #[arg(long)]
        media_type: String,
        #[arg(long, default_value_t = DEFAULT_ARTIFACT_BYTES)]
        max_bytes: u64,
    },
    /// Verify an artifact and export to a new file, without overwriting.
    Read {
        #[arg(long)]
        reference_file: PathBuf,
        #[arg(long)]
        output_file: PathBuf,
        #[arg(long, default_value_t = DEFAULT_ARTIFACT_BYTES)]
        max_bytes: u64,
    },
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return if error.print().is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        Err(_) => {
            report_error(&RuntimeError::InvalidInput(
                "invalid command-line arguments".into(),
            ));
            return ExitCode::from(2);
        }
    };
    match dispatch(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            match error {
                CliError::Runtime(error) => report_error(&error),
                CliError::Integration(error) => {
                    let mut stderr = io::stderr().lock();
                    let _ = serde_json::to_writer(
                        &mut stderr,
                        &json!({"error":{"code":error.code(),"message":error.to_string(),"reason":error.reason()}}),
                    );
                    let _ = stderr.write_all(b"\n");
                }
            }
            ExitCode::FAILURE
        }
    }
}

enum CliError {
    Runtime(RuntimeError),
    Integration(apr_integrations::Error),
}
impl From<RuntimeError> for CliError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}
impl From<apr_integrations::Error> for CliError {
    fn from(error: apr_integrations::Error) -> Self {
        Self::Integration(error)
    }
}

fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli {
        Cli {
            command: Command::Inspector { attachment, port },
            ..
        } => Ok(inspector::serve(&attachment, port)?),
        Cli {
            state_dir,
            command: Command::Service,
        } => Ok(apr_runtime::service::serve(&state_dir)?),
        Cli {
            state_dir,
            command: Command::Integrate { command },
        } => Ok(write_json(integration::execute(&state_dir, command)?)?),
        Cli {
            state_dir,
            command:
                Command::Mcp {
                    attachment_file,
                    projects,
                },
        } => {
            // MCP exclusively owns stdout until this transport exits. Never
            // append the ordinary CLI JSON result to the protocol stream.
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(RuntimeError::from)?;
            let result = if projects {
                runtime.block_on(apr_mcp::serve_projects(&state_dir))
            } else {
                runtime.block_on(apr_mcp::serve(attachment_file.as_deref().ok_or_else(
                    || RuntimeError::InvalidInput("attachment file required".into()),
                )?))
            };
            // Tokio stdin uses an uncancellable blocking read. A rejected
            // frame must terminate this bridge even if its caller keeps stdin
            // open; the independent service retains ownership of accepted work.
            runtime.shutdown_timeout(std::time::Duration::from_millis(100));
            result
                .map_err(|error| match error.downcast::<RuntimeError>() {
                    Ok(error) => *error,
                    Err(_) => RuntimeError::Service("MCP transport could not complete".into()),
                })
                .map_err(Into::into)
        }
        cli => Ok(execute(cli).and_then(write_json)?),
    }
}

fn execute(cli: Cli) -> apr_runtime::Result<Value> {
    match cli.command {
        Command::Improvement {
            attachment,
            request_file,
            inspect,
        } => {
            let request: Value =
                serde_json::from_slice(&read_bounded(&request_file, MAX_JSON_BYTES)?)?;
            let verb = if inspect { "inspect" } else { "manage" };
            if !request.is_object()
                || !request
                    .get(if inspect { "view" } else { "action" })
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.starts_with("improvement"))
            {
                return Err(RuntimeError::InvalidInput(
                    "improvement request requires an improvement action/view".into(),
                ));
            }
            apr_runtime::service::ServiceClient::from_file(&attachment)?
                .with_launcher(std::env::current_exe()?)
                .call(verb, request, 64 * 1024)
        }
        Command::Schedule { command } => schedule::execute(&cli.state_dir, command),
        Command::Adapter { command } => adapter::execute(command),
        Command::Computer { command } => computer::execute(command),
        Command::ConnectionClients { command } => clients::execute(&cli.state_dir, command),
        Command::Backup { output } => Ok(serde_json::to_value(
            Runtime::open(&cli.state_dir)?.backup_to(&output)?,
        )?),
        Command::Restore { backup } => Ok(serde_json::to_value(Runtime::restore_from(
            &backup,
            &cli.state_dir,
        )?)?),
        Command::Storage { command } => maintenance::execute(&cli.state_dir, command),
        Command::Capacity { command } => capacity::execute(&cli.state_dir, command),
        Command::Config { command } => owner::config(&cli.state_dir, command),
        Command::Permission { command } => owner::permission(&cli.state_dir, command),
        Command::Execution { command } => owner::execution(&cli.state_dir, command),
        Command::Project {
            command: ProjectCommand::Attach { path },
        } => Runtime::open(&cli.state_dir)?.attach_project(&path),
        Command::Project {
            command: ProjectCommand::Relocate { id, path },
        } => Runtime::open(&cli.state_dir)?.relocate_project(&id, &path),
        Command::Assignment {
            command: AssignmentCommand::Create { project, name },
        } => Runtime::open(&cli.state_dir)?.create_assignment(&project, &name),
        Command::Assignment {
            command: AssignmentCommand::Close { id },
        } => Runtime::open(&cli.state_dir)?.close_assignment(&id),
        Command::Attachment {
            command:
                AttachmentCommand::Create {
                    assignment,
                    mode,
                    output_file,
                },
        } => create_attachment_file(&cli.state_dir, &assignment, mode, &output_file),
        Command::Attachment {
            command: AttachmentCommand::Revoke { id },
        } => Runtime::open(&cli.state_dir)?.revoke_attachment(&id),
        Command::Service
        | Command::Mcp { .. }
        | Command::Integrate { .. }
        | Command::Inspector { .. } => Err(RuntimeError::State(
            "transport command was not dispatched".into(),
        )),
        Command::Process {
            command: ProcessCommand::Validate { manifest },
        } => {
            let bytes = read_bounded(&manifest, MAX_MANIFEST_BYTES)?;
            let process = admit_json(&bytes)?;
            Ok(json!({
                "valid": true,
                "process_name": process.manifest.name,
                "process_version": process.version,
                "steps": process.manifest.steps.len(),
            }))
        }
        Command::Run {
            manifest,
            input_file,
            request_key,
        } => {
            let bytes = read_bounded(&manifest, MAX_MANIFEST_BYTES)?;
            let process = admit_json(&bytes)?;
            let input = serde_json::from_slice(&read_bounded(&input_file, MAX_JSON_BYTES)?)?;
            let mut runtime = Runtime::open(&cli.state_dir)?;
            Ok(serde_json::to_value(runtime.start(
                &process,
                input,
                &request_key,
            )?)?)
        }
        Command::Inspect { run } => {
            let runtime = Runtime::open(&cli.state_dir)?;
            Ok(serde_json::to_value(runtime.inspect(&run)?)?)
        }
        Command::Continue { run } => {
            let mut runtime = Runtime::open(&cli.state_dir)?;
            Ok(serde_json::to_value(runtime.advance(&run)?)?)
        }
        Command::Renew { run } => {
            let mut runtime = Runtime::open(&cli.state_dir)?;
            Ok(serde_json::to_value(runtime.renew_handoff(&run)?)?)
        }
        Command::Resume { run, result_file } => {
            let submission: Submission =
                serde_json::from_slice(&read_bounded(&result_file, MAX_JSON_BYTES)?)?;
            if submission.run_id != run {
                return Err(RuntimeError::InvalidInput(
                    "submission targets a different run".into(),
                ));
            }
            let mut runtime = Runtime::open(&cli.state_dir)?;
            let receipt = runtime.resume(&submission)?;
            let run = runtime.inspect(&run)?;
            Ok(json!({"receipt": receipt, "run": run}))
        }
        Command::Events { run, after, limit } => {
            let runtime = Runtime::open(&cli.state_dir)?;
            Ok(serde_json::to_value(runtime.events(&run, after, limit)?)?)
        }
        Command::Artifact {
            command:
                ArtifactCommand::Import {
                    path,
                    media_type,
                    max_bytes,
                },
        } => {
            check_byte_limit(max_bytes)?;
            let mut runtime = Runtime::open(&cli.state_dir)?;
            Ok(serde_json::to_value(runtime.import_artifact(
                &path,
                &media_type,
                max_bytes,
            )?)?)
        }
        Command::Artifact {
            command:
                ArtifactCommand::Read {
                    reference_file,
                    output_file,
                    max_bytes,
                },
        } => {
            check_byte_limit(max_bytes)?;
            let reference: ArtifactRef =
                serde_json::from_slice(&read_bounded(&reference_file, MAX_JSON_BYTES)?)?;
            let runtime = Runtime::open(&cli.state_dir)?;
            let bytes = runtime.read_artifact(&reference, max_bytes)?;
            // create_new rejects existing files and symlinks, including a target
            // created after verification. Artifact bytes never go to stdout.
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output_file)?;
            output.write_all(&bytes)?;
            output.sync_all()?;
            Ok(json!({"artifact": reference, "bytes_written": bytes.len()}))
        }
    }
}

fn create_attachment_file(
    state_dir: &Path,
    assignment: &str,
    mode: AttachmentMode,
    output_file: &Path,
) -> apr_runtime::Result<Value> {
    // Reserve the destination before minting a capability. A pre-existing file
    // or symlink cannot be overwritten, even if it appears concurrently.
    let mut runtime = Runtime::open(state_dir)?;
    let mut created = PrivateOutput::new(state_dir, output_file)?;
    let attachment: AttachmentFile =
        serde_json::from_value(runtime.create_attachment(assignment, mode.as_str())?)?;
    let saved = (|| -> apr_runtime::Result<()> {
        serde_json::to_writer(&mut created.file, &attachment)?;
        created.file.write_all(b"\n")?;
        created.file.sync_all()?;
        created.directory.sync_all()?;
        Ok(())
    })();
    if let Err(error) = saved {
        let _ = runtime.revoke_attachment(&attachment.attachment_id);
        return Err(error);
    }
    created.keep = true;
    Ok(json!({
        "protocol_version": attachment.protocol_version,
        "attachment_id": attachment.attachment_id,
        "assignment_id": attachment.assignment_id,
        "mode": attachment.mode,
        "attachment_file": created.path,
    }))
}

fn save_private_json(state_dir: &Path, path: &Path, value: &Value) -> apr_runtime::Result<PathBuf> {
    let mut output = PrivateOutput::new(state_dir, path)?;
    serde_json::to_writer(&mut output.file, value)?;
    output.file.write_all(b"\n")?;
    output.file.sync_all()?;
    output.directory.sync_all()?;
    output.keep = true;
    Ok(output.path.clone())
}

struct PrivateOutput {
    file: File,
    directory: File,
    name: CString,
    path: PathBuf,
    keep: bool,
}

impl PrivateOutput {
    fn new(state_dir: &Path, path: &Path) -> apr_runtime::Result<Self> {
        let current = std::env::current_dir()?;
        let source_root = if state_dir.is_absolute() {
            state_dir.to_path_buf()
        } else {
            current.join(state_dir)
        };
        let root = state_dir.canonicalize()?;
        let requested = if path.is_absolute() {
            path.to_path_buf()
        } else {
            current.join(path)
        };
        let relative = requested
            .strip_prefix(&root)
            .or_else(|_| requested.strip_prefix(&source_root))
            .map_err(|_| {
                RuntimeError::InvalidInput(
                    "attachment output must be inside the runtime state directory".into(),
                )
            })?;
        let components: Vec<_> = relative.components().collect();
        if components.is_empty()
            || components
                .iter()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(RuntimeError::InvalidInput(
                "attachment output requires a file beneath the state directory".into(),
            ));
        }
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&root)?;
        for part in &components[..components.len() - 1] {
            let Component::Normal(part) = part else {
                unreachable!()
            };
            let name = CString::new(part.as_bytes())
                .map_err(|_| RuntimeError::InvalidInput("invalid attachment path".into()))?;
            // SAFETY: the live directory descriptor anchors this single path
            // component; the C string is valid for the duration of each call.
            let made = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
            if made == -1 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error().into());
            }
            if made == 0 {
                directory.sync_all()?;
            }
            // SAFETY: openat uses a valid owned parent descriptor and rejects
            // symlinks instead of following a substituted directory entry.
            let raw = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw == -1 {
                return Err(RuntimeError::InvalidInput(
                    "attachment parents must be private directories without symlinks".into(),
                ));
            }
            // SAFETY: a successful openat returns a new descriptor owned here.
            let next = unsafe { File::from_raw_fd(raw) };
            let metadata = next.metadata()?;
            // SAFETY: geteuid has no preconditions or side effects.
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(RuntimeError::InvalidInput(
                    "attachment parents must be private owned directories".into(),
                ));
            }
            directory = next;
        }
        let Component::Normal(name) = components.last().unwrap() else {
            unreachable!()
        };
        let name = CString::new(name.as_bytes())
            .map_err(|_| RuntimeError::InvalidInput("invalid attachment filename".into()))?;
        // SAFETY: flags atomically create a new private file relative to the
        // validated parent. Existing files and symlinks cannot be overwritten.
        let raw = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if raw == -1 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: raw is a newly owned descriptor after successful openat.
        let file = unsafe { File::from_raw_fd(raw) };
        Ok(Self {
            file,
            directory,
            name,
            path: root.join(relative),
            keep: false,
        })
    }
}

impl Drop for PrivateOutput {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        // Remove only the inode this command created, never a replacement that
        // another local operation put at the same name after a failure.
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the descriptor/name remain live; metadata points to writable
        // storage of the required size and is read only after a successful call.
        let status = unsafe {
            libc::fstatat(
                self.directory.as_raw_fd(),
                self.name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if status == 0
            && let Ok(file) = self.file.metadata()
        {
            // SAFETY: successful fstatat initialized the complete stat value.
            let metadata = unsafe { metadata.assume_init() };
            // dev_t is signed 32-bit on Darwin and already u64 on Linux.
            #[allow(clippy::unnecessary_cast)]
            let device = metadata.st_dev as u64;
            if file.dev() == device && file.ino() == metadata.st_ino {
                // SAFETY: unlink only the still-matching entry this guard made.
                unsafe { libc::unlinkat(self.directory.as_raw_fd(), self.name.as_ptr(), 0) };
            }
        }
    }
}

fn check_byte_limit(limit: u64) -> apr_runtime::Result<()> {
    if limit == 0 {
        Err(RuntimeError::InvalidInput(
            "byte limit must be positive".into(),
        ))
    } else {
        Ok(())
    }
}

fn read_bounded(path: &Path, limit: usize) -> apr_runtime::Result<Vec<u8>> {
    // Reject ordinary directories, symlinks, devices and pipes before opening.
    // Recheck the opened handle and cap the read even if its length changes.
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(RuntimeError::InvalidInput(
            "JSON input must be a regular file".into(),
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(RuntimeError::InvalidInput(
            "JSON input must be a regular file".into(),
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(RuntimeError::LimitExceeded(
            "JSON file exceeds the byte limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(RuntimeError::LimitExceeded(
            "JSON file exceeds the byte limit".into(),
        ));
    }
    Ok(bytes)
}

fn write_json(value: Value) -> apr_runtime::Result<()> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &value)?;
    output.write_all(b"\n")?;
    Ok(())
}

fn report_error(error: &RuntimeError) {
    // Parser/schema errors may contain input values. Emit public diagnostics,
    // never a raw error chain, file contents, continuation token or local path.
    let code = error.code();
    let message = match code {
        "INVALID_MANIFEST" => "The process manifest failed admission.",
        "INVALID_INPUT" => {
            "The request or JSON input is invalid. Check the command help and input schema."
        }
        "NOT_FOUND" => "The requested run or artifact was not found.",
        "REQUEST_CONFLICT" => {
            "The request key already identifies different inputs or a different process version."
        }
        "SUBMISSION_CONFLICT" => "The submission key already identifies a different result.",
        "STALE_HANDOFF" => {
            "The handoff is no longer valid. Inspect the current run before submitting a result."
        }
        "LIMIT_EXCEEDED" => {
            "The request exceeds a configured size or execution limit. Reduce the workload; for owner nesting capacity inspect `apr capacity show` and `apr capacity set --help`."
        }
        "ARTIFACT_INTEGRITY" => "The artifact could not be validated or stored safely.",
        "INVALID_STATE" => "The saved runtime state does not permit this operation.",
        "UNAUTHORIZED" => "The attachment is invalid, revoked or closed.",
        "FORBIDDEN" => "This attachment does not permit that operation.",
        "SERVICE_UNAVAILABLE" => "The local service could not be reached or started.",
        "UNSUPPORTED_VERSION" => "The local service or protocol version is incompatible.",
        _ => "The operation could not be completed. Check local storage and permissions.",
    };
    let mut stderr = io::stderr().lock();
    let _ = serde_json::to_writer(
        &mut stderr,
        &json!({"error": {"code": code, "message": message}}),
    );
    let _ = stderr.write_all(b"\n");
}
