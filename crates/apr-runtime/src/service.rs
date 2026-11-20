//! Private local IPC. Connections carry attachment credentials; execution is
//! owned by the supervisor worker and never by a client socket or MCP process.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Result, Runtime, RuntimeError};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const DEFAULT_RESPONSE_BYTES: usize = 8 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentFile {
    pub protocol_version: String,
    pub state_dir: PathBuf,
    pub attachment_id: String,
    pub assignment_id: String,
    pub mode: String,
    pub token: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol_version: String,
    request_id: String,
    attachment_token: String,
    action: String,
    args: Value,
    response_budget: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    protocol_version: String,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<PublicError>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<apr_core::AdmissionDiagnostic>,
}

#[derive(Clone)]
pub struct ServiceClient {
    attachment: AttachmentFile,
    executable: PathBuf,
}

impl ServiceClient {
    pub(crate) fn attachment_for_project_broker(&self) -> &AttachmentFile {
        &self.attachment
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.len() > MAX_RESPONSE_BYTES as u64
        {
            return Err(RuntimeError::InvalidInput(
                "attachment file must be a private owned regular file under 64 KiB".into(),
            ));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(RuntimeError::LimitExceeded(
                "attachment file is too large".into(),
            ));
        }
        let attachment: AttachmentFile = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeError::InvalidInput("invalid attachment file".into()))?;
        validate_attachment_file(&attachment)?;
        let root = attachment.state_dir.canonicalize()?;
        let actual = path.canonicalize()?;
        if !actual.starts_with(&root) || metadata.nlink() != 1 {
            return Err(RuntimeError::InvalidInput(
                "attachment files must remain inside their protected runtime state directory"
                    .into(),
            ));
        }
        Ok(Self::with_executable(attachment, std::env::current_exe()?))
    }

    pub fn with_executable(attachment: AttachmentFile, executable: impl AsRef<Path>) -> Self {
        Self {
            attachment,
            executable: executable.as_ref().to_path_buf(),
        }
    }

    pub fn assignment_id(&self) -> &str {
        &self.attachment.assignment_id
    }

    pub fn mode(&self) -> &str {
        &self.attachment.mode
    }

    /// Select the installed APR executable without exposing the attachment secret.
    pub fn with_launcher(mut self, executable: impl AsRef<Path>) -> Self {
        self.executable = executable.as_ref().to_path_buf();
        self
    }

    pub fn state_dir(&self) -> &Path {
        &self.attachment.state_dir
    }

    /// Trusted native owner control. Not an MCP or inspector action. The service
    /// checks its unpredictable instance ID before stopping; stale requests do nothing.
    pub fn request_owner_stop(&self) -> Result<Value> {
        if !socket_path(&self.attachment.state_dir)?.exists() {
            return Ok(json!({"stop_requested":false,"state":"not_running"}));
        }
        let info = self.call("service.info", json!({}), DEFAULT_RESPONSE_BYTES)?;
        let id = info["instance_id"]
            .as_str()
            .ok_or_else(|| RuntimeError::Integrity("service identity missing".into()))?;
        let path = self.attachment.state_dir.join("owner-stop.json");
        let temporary = self
            .attachment
            .state_dir
            .join(format!("owner-stop-{}.tmp", Uuid::new_v4().simple()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        file.write_all(serde_json::to_string(&json!({"instance_id":id}))?.as_bytes())?;
        file.sync_all()?;
        fs::rename(temporary, path)?;
        Ok(
            json!({"stop_requested":true,"state":"stopping","in_flight_effects":"may need reconciliation"}),
        )
    }

    pub fn call(&self, action: &str, args: Value, response_budget: usize) -> Result<Value> {
        validate_attachment_file(&self.attachment)?;
        let request = Request {
            protocol_version: "0.1".into(),
            request_id: Uuid::new_v4().to_string(),
            attachment_token: self.attachment.token.clone(),
            action: action.into(),
            args,
            response_budget,
        };
        validate_request(&request)?;
        let mut bytes = serde_json::to_vec(&request)?;
        if bytes.len() + 1 > MAX_FRAME_BYTES {
            return Err(RuntimeError::LimitExceeded(
                "IPC request exceeds 1 MiB".into(),
            ));
        }
        bytes.push(b'\n');
        let path = socket_path(&self.attachment.state_dir)?;
        let stream = match UnixStream::connect(&path) {
            Ok(stream) => stream,
            Err(error) if may_start(&error) => {
                self.start_service()?;
                self.wait_for_service(&path)?
            }
            Err(error) => return Err(error.into()),
        };
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        let mut stream = stream;
        stream.write_all(&bytes)?;
        let bytes = read_frame(&mut stream, MAX_RESPONSE_BYTES, Duration::from_secs(10))?;
        let response: Response = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeError::Protocol("invalid service response".into()))?;
        if response.protocol_version != "0.1" || response.request_id != request.request_id {
            return Err(RuntimeError::Protocol(
                "service response identity or version mismatch".into(),
            ));
        }
        match (response.result, response.error) {
            (Some(value), None) => Ok(value),
            (None, Some(error)) => Err(RuntimeError::Remote {
                code: error.code,
                message: error.message,
                diagnostic: error.diagnostic,
            }),
            _ => Err(RuntimeError::Protocol(
                "service response must contain exactly one result or error".into(),
            )),
        }
    }

    fn start_service(&self) -> Result<()> {
        let mut command = Command::new(&self.executable);
        command
            .arg("--state-dir")
            .arg(&self.attachment.state_dir)
            .arg("service")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Survive launcher process-group teardown while allowing posix_spawn.
        // A pre_exec hook forces fork/exec and its pipe handshake during
        // concurrent startup. A separate session is not part of this contract.
        command.process_group(0);
        let mut child = command.spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn wait_for_service(&self, path: &Path) -> Result<UnixStream> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match UnixStream::connect(path) {
                Ok(stream) => return Ok(stream),
                Err(error) if may_start(&error) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                Err(_) => {
                    return Err(RuntimeError::Service(
                        "local service did not become ready; run apr service for diagnostics"
                            .into(),
                    ));
                }
            }
        }
    }
}

fn may_start(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

fn validate_attachment_file(attachment: &AttachmentFile) -> Result<()> {
    if attachment.protocol_version != "0.1" {
        return Err(RuntimeError::Protocol(
            "unsupported attachment protocol".into(),
        ));
    }
    if !attachment.state_dir.is_absolute()
        || !matches!(attachment.mode.as_str(), "author" | "run")
        || attachment.token.len() != 64
        || !attachment.token.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err(RuntimeError::InvalidInput(
            "invalid attachment configuration".into(),
        ));
    }
    Ok(())
}

/// Short, owner-only endpoint paths also work for long project paths on macOS.
/// The state directory's canonical identity selects the endpoint; it is not a credential.
pub fn socket_path(state_dir: &Path) -> Result<PathBuf> {
    let state_dir = state_dir.canonicalize()?;
    use std::os::unix::ffi::OsStrExt;
    let hash = format!("{:x}", Sha256::digest(state_dir.as_os_str().as_bytes()));
    let uid = unsafe { libc::geteuid() };
    let parent = PathBuf::from(format!("/tmp/apr-{uid}"));
    endpoint_directory(&parent)?;
    let directory = parent.join(&hash[..32]);
    endpoint_directory(&directory)?;
    Ok(directory.join("rpc.sock"))
}

/// Direct Runtime driving must yield execution ownership while the local
/// supervisor is alive. StoreLock is deliberately short during external work,
/// so this process lifetime lock is the cross-process dispatch boundary.
pub(crate) fn supervisor_owns_execution(state_dir: &Path) -> Result<bool> {
    let socket = socket_path(state_dir)?;
    let directory = socket
        .parent()
        .ok_or_else(|| RuntimeError::Service("invalid endpoint".into()))?;
    let file = crate::store::private_file(&directory.join("service.lock"))?;
    match file.try_lock_exclusive() {
        Ok(()) => {
            FileExt::unlock(&file)?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn endpoint_directory(path: &Path) -> Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(RuntimeError::Service(
            "endpoint directory must be private and owned by this user".into(),
        ));
    }
    Ok(())
}

struct Supervisor {
    _lock: File,
    path: PathBuf,
    inode: u64,
    device: u64,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && metadata.ino() == self.inode
            && metadata.dev() == self.device
        {
            let _ = fs::remove_file(&self.path);
        }
        // The lock inode remains permanently; deleting it permits two owners.
    }
}

struct Work {
    request: Request,
    reply: mpsc::Sender<Response>,
    cancel_latch: Option<crate::execution::CancelLatch>,
}
struct ConnectionCount(Arc<AtomicUsize>);
impl Drop for ConnectionCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

/// Run one supervisor in the foreground. The CLI's MCP bridge starts this as a
/// detached child when absent; ordinary users can run it for diagnostics too.
pub fn serve(state_dir: &Path) -> Result<()> {
    crate::store::private_dir(state_dir)?;
    let path = socket_path(state_dir)?;
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::Service("invalid endpoint".into()))?;
    let lock = crate::store::private_file(&parent.join("service.lock"))?;
    lock.try_lock_exclusive().map_err(|_| {
        RuntimeError::Service("a supervisor already owns this state directory".into())
    })?;
    // Claim process-wide dispatch ownership before opening the runtime. A
    // direct offline driver already inside StoreLock may finish its bounded
    // wave, but it cannot overlap this supervisor's first dispatch.
    let mut runtime = Runtime::open(state_dir)?;
    let reader_runtime = Runtime::open(state_dir)?;
    let execution_interrupt = runtime.interrupt.clone();
    let signal_state = runtime.root.clone();
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() } =>
        {
            // Winning the permanent lock proves no cooperating supervisor owns
            // this socket. Never replace an arbitrary filesystem entry.
            fs::remove_file(&path)?;
        }
        Ok(_) => {
            return Err(RuntimeError::Service(
                "refusing to replace a non-socket endpoint".into(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = UnixListener::bind(&path)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let metadata = fs::symlink_metadata(&path)?;
    let _supervisor = Supervisor {
        _lock: lock,
        path,
        inode: metadata.ino(),
        device: metadata.dev(),
    };
    runtime.start_external_pool();
    let shutdown = Arc::new(AtomicBool::new(false));
    let term = signal_hook::flag::register(signal_hook::consts::SIGTERM, shutdown.clone())?;
    let interrupt = signal_hook::flag::register(signal_hook::consts::SIGINT, shutdown.clone())?;
    let (sender, receiver) = mpsc::sync_channel::<Work>(32);
    let worker_stop = shutdown.clone();
    let instance = Uuid::new_v4().to_string();
    let reader_instance = instance.clone();
    let owner_instance = instance.clone();
    let reader_stop = shutdown.clone();
    let (read_sender, read_receiver) = mpsc::sync_channel::<Work>(32);
    let reader = std::thread::spawn(move || {
        while !reader_stop.load(Ordering::Acquire) {
            match read_receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(work) => {
                    let result = if work.request.action == "service.info" {
                        reader_runtime.scoped_read(&work.request.attachment_token,"inspect",json!({"view":"pending","limit":1}))
                            .and_then(|_| reader_runtime.worker_policy().map(|policy|json!({"instance_id":reader_instance,"process_id":std::process::id(),"protocol_version":"0.1","runtime_version":env!("CARGO_PKG_VERSION"),"features":{"jev":cfg!(feature="jev"),"classifier_learning":true,"connection_library":true,"recipe_improvement":true,"parallel_execution":true,"max_parallel_workers":policy.max_workers,"supported_parallel_workers":crate::execution::MAX_EXTERNAL_WORKERS}})))
                    } else {
                        reader_runtime.scoped_read(
                            &work.request.attachment_token,
                            &work.request.action,
                            work.request.args,
                        )
                    };
                    if let Some(receipt) = &work.cancel_latch {
                        reader_runtime.interrupt.release_latch(receipt);
                    }
                    let _ = work.reply.send(make_response(
                        work.request.request_id,
                        result,
                        work.request.response_budget,
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    let worker = std::thread::spawn(move || {
        let mut pending = true;
        let mut cursor = None;
        let mut next_schedule_tick = Instant::now();
        let mut next_advance = Instant::now();
        let mut consecutive_advance_errors = 0_u32;
        while !worker_stop.load(Ordering::Acquire) {
            let timeout = if pending && Instant::now() >= next_advance {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(200)
            };
            match receiver.recv_timeout(timeout) {
                Ok(work) => {
                    let action = work.request.action.as_str();
                    let starts_work = matches!(action, "run" | "resume")
                        || (action == "manage"
                            && work.request.args.get("action").and_then(Value::as_str)
                                == Some("process.test"));
                    let result = if action == "service.info" {
                        runtime.scoped_call(&work.request.attachment_token,"inspect",json!({"view":"pending","limit":1}))
                            .and_then(|_| runtime.worker_policy().map(|policy|json!({"instance_id":instance,"process_id":std::process::id(),"protocol_version":"0.1","runtime_version":env!("CARGO_PKG_VERSION"),"features":{"jev":cfg!(feature="jev"),"classifier_learning":true,"connection_library":true,"recipe_improvement":true,"parallel_execution":true,"max_parallel_workers":policy.max_workers,"supported_parallel_workers":crate::execution::MAX_EXTERNAL_WORKERS}})))
                    } else {
                        runtime.scoped_call(
                            &work.request.attachment_token,
                            action,
                            work.request.args,
                        )
                    };
                    if let Some(receipt) = &work.cancel_latch {
                        runtime.interrupt.release_latch(receipt);
                    }
                    if result.is_ok() && (starts_work || action != "service.info") {
                        pending = true;
                    }
                    let response = make_response(
                        work.request.request_id,
                        result,
                        work.request.response_budget,
                    );
                    let _ = work.reply.send(response);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            // Schedules must wake an idle supervisor even when there is no
            // currently runnable work and no attached client sends a request.
            if Instant::now() >= next_schedule_tick
                && !worker_stop.load(Ordering::Acquire)
                && worker_gate_open()
            {
                match runtime.tick_schedules(crate::store::now_ms()) {
                    Ok(_) => {}
                    Err(error) => eprintln!("APR schedule tick stopped: {}", error.code()),
                }
                match runtime.tick_sampling_workers() {
                    Ok(_) => {}
                    Err(error) => eprintln!("APR sampling worker stopped: {}", error.code()),
                }
                if let Err(error) = runtime.tick_price_sources() {
                    eprintln!("APR price check stopped: {}", error.code());
                }
                // A trusted direct Runtime can author queued work while this
                // supervisor owns dispatch. Poll it on the idle tick so those
                // durable starts do not depend on a later IPC request.
                pending = true;
                next_schedule_tick = Instant::now() + Duration::from_millis(200);
            }
            if pending
                && Instant::now() >= next_advance
                && !worker_stop.load(Ordering::Acquire)
                && worker_gate_open()
            {
                if let Err(error) = runtime.sync_pool_cancellations() {
                    eprintln!("APR worker cancellation check deferred: {}", error.code());
                }
                match runtime.drive_pending_pool(&mut cursor) {
                    Ok(advanced) => {
                        pending = advanced;
                        consecutive_advance_errors = 0;
                        next_advance = Instant::now();
                    }
                    Err(error) => {
                        eprintln!("APR background advance deferred: {}", error.code());
                        // One failed finish can coexist with successful siblings
                        // whose dependents are now queued. An error cannot prove
                        // quiescence. Recheck with capped backoff while keeping
                        // IPC, cancellation and schedule ticks responsive.
                        pending = true;
                        consecutive_advance_errors = consecutive_advance_errors.saturating_add(1);
                        let delay_ms = (200_u64
                            << consecutive_advance_errors.saturating_sub(1).min(5))
                        .min(5_000);
                        next_advance = Instant::now() + Duration::from_millis(delay_ms);
                    }
                }
            }
        }
        // Shutdown signals every active attempt first. Preserve observed
        // cancellation/effect uncertainty durably before releasing the
        // supervisor's ownership of this state directory.
        while runtime.external_pool_len() > 0 {
            match runtime.settle_external_pool() {
                Ok(true) => {}
                Ok(false) => std::thread::sleep(Duration::from_millis(10)),
                Err(error) => eprintln!("APR shutdown settlement deferred: {}", error.code()),
            }
        }
    });
    let active = Arc::new(AtomicUsize::new(0));
    while !shutdown.load(Ordering::Acquire) {
        if worker.is_finished() || reader.is_finished() {
            shutdown.store(true, Ordering::Release);
            break;
        }
        if owner_stop_requested(state_dir, &owner_instance) {
            shutdown.store(true, Ordering::Release);
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if active
                    .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |n| {
                        if n < 32 { Some(n + 1) } else { None }
                    })
                    .is_err()
                {
                    drop(stream);
                    continue;
                }
                let count = ConnectionCount(active.clone());
                let sender = sender.clone();
                let read_sender = read_sender.clone();
                let interrupt = execution_interrupt.clone();
                let state_dir = signal_state.clone();
                std::thread::spawn(move || {
                    let _count = count;
                    handle_connection(stream, sender, read_sender, &state_dir, &interrupt);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(error) => {
                shutdown.store(true, Ordering::Release);
                execution_interrupt.signal(None);
                drop(sender);
                drop(read_sender);
                let _ = worker.join();
                let _ = reader.join();
                signal_hook::low_level::unregister(term);
                signal_hook::low_level::unregister(interrupt);
                return Err(error.into());
            }
        }
    }
    execution_interrupt.signal(None);
    drop(sender);
    drop(read_sender);
    let worker_result = worker.join();
    let reader_result = reader.join();
    signal_hook::low_level::unregister(term);
    signal_hook::low_level::unregister(interrupt);
    if worker_result.is_err() || reader_result.is_err() {
        return Err(RuntimeError::Service(
            "supervisor coordinator stopped unexpectedly".into(),
        ));
    }
    Ok(())
}

fn owner_stop_requested(root: &Path, instance: &str) -> bool {
    let path = root.join("owner-stop.json");
    let Ok(mut file) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)
    else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.len() > 1024
    {
        return false;
    }
    let mut bytes = Vec::new();
    if Read::by_ref(&mut file)
        .take(1025)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return false;
    }
    let requested = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .is_some_and(|v| v["instance_id"] == instance);
    let _ = fs::remove_file(path);
    requested
}

fn handle_connection(
    mut stream: UnixStream,
    sender: mpsc::SyncSender<Work>,
    read_sender: mpsc::SyncSender<Work>,
    state_dir: &Path,
    interrupt: &crate::execution::ExecutionInterrupt,
) {
    if stream.set_write_timeout(Some(IO_TIMEOUT)).is_err() {
        return;
    }
    let frame = read_frame(&mut stream, MAX_FRAME_BYTES, IO_TIMEOUT);
    let parsed = frame.and_then(|bytes| {
        serde_json::from_slice::<Request>(&bytes)
            .map_err(|_| RuntimeError::InvalidInput("invalid IPC request".into()))
    });
    let response = match parsed {
        Ok(request) => {
            if let Err(error) = validate_request(&request) {
                make_response(request.request_id, Err(error), DEFAULT_RESPONSE_BYTES)
            } else {
                let is_cancel = request.action == "resume" && request.args["action"] == "cancel";
                let cancel_latch = if is_cancel {
                    crate::execution::signal_cancel_if_authorized(
                        state_dir,
                        &request.attachment_token,
                        &request.args,
                        interrupt,
                    )
                } else {
                    None
                };
                if is_cancel && cancel_latch.is_none() {
                    make_response(
                        request.request_id,
                        Err(RuntimeError::Unauthorized),
                        request.response_budget,
                    )
                } else {
                    let sender = if matches!(
                        request.action.as_str(),
                        "inspect" | "discover" | "service.info"
                    ) {
                        read_sender
                    } else {
                        sender
                    };
                    let id = request.request_id.clone();
                    let budget = request.response_budget;
                    let (reply, receiver) = mpsc::channel();
                    match sender.try_send(Work {
                        request,
                        reply,
                        cancel_latch,
                    }) {
                        Ok(()) => receiver
                            .recv_timeout(Duration::from_secs(8))
                            .unwrap_or_else(|_| {
                                make_response(
                                    id,
                                    Err(RuntimeError::Service(
                                        "request queued; inspect saved state before retrying"
                                            .into(),
                                    )),
                                    budget,
                                )
                            }),
                        Err(mpsc::TrySendError::Full(work))
                        | Err(mpsc::TrySendError::Disconnected(work)) => {
                            if let Some(receipt) = &work.cancel_latch {
                                interrupt.release_latch(receipt);
                            }
                            make_response(
                                id,
                                Err(RuntimeError::Service(
                                    "service queue is busy; retry the same request key".into(),
                                )),
                                budget,
                            )
                        }
                    }
                }
            }
        }
        Err(error) => make_response(String::new(), Err(error), DEFAULT_RESPONSE_BYTES),
    };
    if let Ok(mut bytes) = serde_json::to_vec(&response) {
        bytes.push(b'\n');
        let _ = stream.write_all(&bytes);
    }
}

fn validate_request(request: &Request) -> Result<()> {
    if request.protocol_version != "0.1" {
        return Err(RuntimeError::Protocol("unsupported IPC protocol".into()));
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || request.action.len() > 64
        || !(1024..=MAX_RESPONSE_BYTES).contains(&request.response_budget)
        || request.attachment_token.len() != 64
    {
        return Err(RuntimeError::InvalidInput(
            "invalid request fields or response budget".into(),
        ));
    }
    Ok(())
}

fn read_frame(stream: &mut UnixStream, limit: usize, timeout: Duration) -> Result<Vec<u8>> {
    // SO_RCVTIMEO changes can fail on Darwin after the peer closes, even while
    // unread response bytes remain. Poll a nonblocking fd against one deadline
    // instead; restore blocking writes for the response path on every exit.
    stream.set_nonblocking(true)?;
    let result = read_frame_nonblocking(stream, limit, timeout);
    let restore = stream.set_nonblocking(false);
    let bytes = result?;
    restore?;
    Ok(bytes)
}

fn read_frame_nonblocking(
    stream: &mut UnixStream,
    limit: usize,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut buffer = [0_u8; 8192];
    let started = Instant::now();
    loop {
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| RuntimeError::Service("IPC frame deadline exceeded".into()))?;
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let wait_ms = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
        // The stream owns this live fd; poll neither retains nor closes it.
        let ready = unsafe { libc::poll(&mut descriptor, 1, wait_ms) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if ready == 0 {
            continue;
        }
        let count = match stream.read(&mut buffer) {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if count == 0 {
            return Err(RuntimeError::Protocol(
                "connection closed before a complete frame".into(),
            ));
        }
        let end = buffer[..count].iter().position(|b| *b == b'\n');
        let take = end.unwrap_or(count);
        if result.len() + take + 1 > limit {
            return Err(RuntimeError::LimitExceeded(
                "IPC frame exceeds its byte limit".into(),
            ));
        }
        result.extend_from_slice(&buffer[..take]);
        if end.is_some() {
            return Ok(result);
        }
    }
}

fn make_response(mut request_id: String, result: Result<Value>, budget: usize) -> Response {
    // Invalid correlation material is untrusted input too. Do not echo an
    // oversized ID into an error, bypassing the response's output budget.
    if request_id.len() > 128 {
        request_id.clear();
    }
    let mut response = match result {
        Ok(value) => Response {
            protocol_version: "0.1".into(),
            request_id,
            result: Some(value),
            error: None,
        },
        Err(error) => Response {
            protocol_version: "0.1".into(),
            request_id,
            result: None,
            error: Some(PublicError {
                code: error.code().into(),
                message: public_message(error.code()).into(),
                diagnostic: match &error {
                    RuntimeError::Core(core) => core.diagnostic(),
                    _ => None,
                },
            }),
        },
    };
    if !response_fits(&response, budget) {
        let original = response.result.take().unwrap_or(Value::Null);
        // Optional step summaries can be omitted while the complete actionable
        // request remains intact. Never cut input/schema/claim fields to fit.
        let mut reduced = original.clone();
        omit_step_summaries(&mut reduced);
        response.result = Some(reduced);
        if fit_optional_navigation(&mut response, budget) {
            return response;
        }
        let compact = if let Some(artifact) = original.get("artifact") {
            json!({"artifact":artifact,"truncated":true,"text_available":false,
                "reason":"Encoded artifact preview exceeds this response budget; retain the artifact reference."})
        } else if budget >= MAX_RESPONSE_BYTES - 1024 {
            // Explicit detail is already the largest supported view. Returning
            // another suggestion to request detail would strand the caller.
            response.result = None;
            response.error = Some(PublicError {
                code: "LIMIT_EXCEEDED".into(),
                message: "Complete result exceeds the detail budget; use artifact references or a narrower view.".into(),
                diagnostic: None,
            });
            return response;
        } else if let Some(run) = original.get("run") {
            let mut value = compact_identity(&original);
            value["run"] = compact_identity(run);
            if let Some(receipt) = original.get("receipt") {
                value["receipt"] = receipt.clone();
            }
            value
        } else {
            compact_identity(&original)
        };
        response.result = Some(compact);
        response.error = None;
        if !fit_optional_navigation(&mut response, budget) {
            response.result = None;
            response.error=Some(PublicError {code:"LIMIT_EXCEEDED".into(),message:"Result exceeds the requested response budget; use bounded artifacts or a narrower view.".into(),diagnostic:None});
        }
    }
    response
}

fn response_fits(response: &Response, budget: usize) -> bool {
    serde_json::to_vec(response)
        .map(|bytes| bytes.len() < budget)
        .unwrap_or(false)
}

fn add_view(map: &mut serde_json::Map<String, Value>, view: &str) {
    let views = map.entry("available_views").or_insert_with(|| json!([]));
    if let Some(views) = views.as_array_mut()
        && !views.iter().any(|existing| existing == view)
    {
        views.push(json!(view));
    }
}

fn retain_child_navigation(map: &mut serde_json::Map<String, Value>, steps: &[Value]) {
    let children: Vec<_> = steps
        .iter()
        .filter_map(|step| {
            Some(json!({"parent_step_id":step.get("id")?.as_str()?,
                "child_run_id":step.get("child_run_id")?.as_str()?}))
        })
        .collect();
    if !children.is_empty() {
        map.insert("child_count".into(), json!(children.len()));
        map.insert("children_truncated".into(), json!(children.len() > 10));
        map.insert(
            "children".into(),
            json!(children.into_iter().take(10).collect::<Vec<_>>()),
        );
        // This existing scoped view supports `after` pagination and includes
        // each child's parent_run_id and parent_step_id, so omitted links are
        // recoverable without needing a larger parent response.
        add_view(map, "inspect_runs");
    }
}

fn omit_step_summaries(value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    if let Some(steps) = map.remove("steps") {
        if let Some(steps) = steps.as_array() {
            retain_child_navigation(map, steps);
        }
        map.insert("truncated".into(), json!(true));
        add_view(map, "inspect_with_detail_true_or_discover_detail_schema");
    }
    if let Some(run) = map.get_mut("run") {
        omit_step_summaries(run);
    }
}

// Shrink only whole optional navigation entries. Never modify a handoff's
// request, input, schema, claim, receipt, or the fixed run-family identity.
fn fit_optional_navigation(response: &mut Response, budget: usize) -> bool {
    while !response_fits(response, budget) {
        if !response.result.as_mut().is_some_and(shrink_navigation) {
            return false;
        }
    }
    true
}

fn shrink_navigation(value: &mut Value) -> bool {
    let Some(map) = value.as_object_mut() else {
        return false;
    };
    if let Some(children) = map.get_mut("children").and_then(Value::as_array_mut)
        && children.pop().is_some()
    {
        map.insert("children_truncated".into(), json!(true));
        return true;
    }
    if let Some(run) = map.get_mut("run")
        && shrink_navigation(run)
    {
        return true;
    }
    for (key, cursor_key) in [
        ("runs", "run_id"),
        ("schedules", "schedule_id"),
        ("events", "sequence"),
        ("processes", ""),
    ] {
        if let Some(entries) = map.get_mut(key).and_then(Value::as_array_mut)
            && entries.len() > 1
        {
            entries.pop();
            let last_cursor = entries
                .last()
                .and_then(|entry| entry.get(cursor_key))
                .cloned();
            if map.contains_key("next_cursor")
                && let Some(cursor) = last_cursor
            {
                map.insert("next_cursor".into(), cursor);
            }
            map.insert("truncated".into(), json!(true));
            if key == "runs" {
                add_view(map, "inspect_runs");
            }
            return true;
        }
    }
    if map.remove("error").is_some() {
        map.insert("error_omitted".into(), json!(true));
        return true;
    }
    false
}

fn compact_identity(original: &Value) -> Value {
    let mut compact = serde_json::Map::new();
    for key in [
        "protocol_version",
        "run_id",
        "schedule_id",
        "name",
        "next_due_ms",
        "last_occurrence",
        "root_run_id",
        "parent_run_id",
        "parent_step_id",
        "fixture",
        "process_ref",
        "process_name",
        "process_version",
        "state",
        "revision",
        "completed_steps",
        "config_digest",
        "project_revision",
        "assignment_revision",
    ] {
        if let Some(value) = original.get(key) {
            compact.insert(key.into(), value.clone());
        }
    }
    if let Some(error) = original.get("error").and_then(Value::as_str) {
        if error.len() <= 512 {
            compact.insert("error".into(), json!(error));
        } else {
            compact.insert("error_omitted".into(), json!(true));
        }
    }
    compact.insert("truncated".into(), json!(true));
    if let Some(steps) = original.get("steps").and_then(Value::as_array) {
        retain_child_navigation(&mut compact, steps);
    }
    if let Some(runs) = original.get("runs").and_then(Value::as_array) {
        if original.get("next_cursor").is_some()
            && let Some(last) = runs.iter().take(10).next_back()
        {
            compact.insert("next_cursor".into(), last["run_id"].clone());
        }
        compact.insert(
            "runs".into(),
            Value::Array(
                runs.iter()
                    .take(10)
                    .map(|run| {
                        let mut identity = serde_json::Map::new();
                        for key in [
                            "run_id",
                            "root_run_id",
                            "parent_run_id",
                            "parent_step_id",
                            "fixture",
                            "state",
                            "revision",
                            "completed_steps",
                        ] {
                            if let Some(value) = run.get(key) {
                                identity.insert(key.into(), value.clone());
                            }
                        }
                        Value::Object(identity)
                    })
                    .collect(),
            ),
        );
    }
    if let Some(schedules) = original.get("schedules").and_then(Value::as_array) {
        let schedules:Vec<_> = schedules.iter().take(10).map(|entry|
            json!({"schedule_id":entry.get("schedule_id"),"state":entry.get("state"),"next_due_ms":entry.get("next_due_ms")})).collect();
        if let Some(last) = schedules.last() {
            compact.insert("next_cursor".into(), last["schedule_id"].clone());
        }
        compact.insert("schedules".into(), json!(schedules));
        add_view(&mut compact, "inspect_schedule_with_detail_true");
    }
    if let Some(processes) = original.get("processes").and_then(Value::as_array) {
        compact.insert("processes".into(),Value::Array(processes.iter().take(10).map(|process| {
            json!({"process_ref":process.get("process_ref"),"process_name":process.get("process_name")})
        }).collect()));
    }
    if let Some(events) = original.get("events").and_then(Value::as_array) {
        let events:Vec<Value>=events.iter().take(10).map(|event|json!({"sequence":event.get("sequence"),"kind":event.get("kind"),"detail_omitted":true})).collect();
        if let Some(last) = events.last() {
            compact.insert("next_cursor".into(), last["sequence"].clone());
        }
        compact.insert("events".into(), Value::Array(events));
    }
    add_view(
        &mut compact,
        "inspect_with_detail_true_or_discover_detail_schema",
    );
    add_view(&mut compact, "config.read_with_detail_true");
    if original.get("schedule_id").is_some() {
        add_view(&mut compact, "inspect_schedule_with_detail_true");
    }
    if original.get("runs").is_some() || original.get("run_id").is_some() {
        add_view(&mut compact, "inspect_runs");
    }
    Value::Object(compact)
}

fn public_message(code: &str) -> &'static str {
    match code {
        "SCOPE_DENIED" | "FORBIDDEN" => "This attachment cannot access that resource or action.",
        "UNAUTHORIZED" => "This attachment is invalid, expired, revoked, or closed.",
        "INVALID_INPUT" | "INVALID_MANIFEST" => {
            "The request does not match the supported schema or local path constraints."
        }
        "REQUEST_CONFLICT" => {
            "The request key already belongs to different inputs or a different process version."
        }
        "SUBMISSION_CONFLICT" => "The submission key already belongs to a different result.",
        "STALE_HANDOFF" => {
            "Inspect the current handoff; its token, generation, or state has changed."
        }
        "ARTIFACT_INTEGRITY" => "An artifact or saved control file failed integrity verification.",
        "LIMIT_EXCEEDED" => {
            "The request exceeds a declared byte or execution limit. Inspect view capacity for owner ceilings; use smaller bounded requests or ask the owner to adjust capacity."
        }
        "NOT_FOUND" => "The selected resource is not available in this assignment.",
        "UNSUPPORTED_VERSION" => "This endpoint does not support the requested protocol version.",
        "SERVICE_UNAVAILABLE" => {
            "The local service is unavailable or busy; inspect saved state before retrying."
        }
        _ => "APR could not complete the request; inspect the local state and diagnostics.",
    }
}

fn worker_gate_open() -> bool {
    #[cfg(feature = "fault-injection")]
    if let Some(path) = std::env::var_os("APR_TEST_WORKER_GATE") {
        return Path::new(&path).is_file();
    }
    true
}

#[cfg(test)]
mod schedule_budget_tests {
    use super::*;

    #[test]
    fn compact_schedules_preserve_identity_and_list_cursor() {
        let detail = json!({"schedule_id":"schedule_saved","state":"active","process_ref":"saved_version","input":{"large":"x".repeat(20_000)},"config":{}});
        let response = make_response("test".into(), Ok(detail), 1024);
        let result = response.result.unwrap();
        assert_eq!(result["schedule_id"], "schedule_saved");
        assert_eq!(result["truncated"], true);
        let list = json!({"schedules":(0..20).map(|index|json!({"schedule_id":format!("schedule_{index:02}"),"name":"x".repeat(160),"state":"active","last_error":"x".repeat(500)})).collect::<Vec<_>>(),"next_cursor":"schedule_19","truncated":false});
        let response = make_response("test".into(), Ok(list), 2048);
        assert!(response_fits(&response, 2048));
        let result = response.result.unwrap();
        let rows = result["schedules"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.len() < 20);
        assert_eq!(result["next_cursor"], rows.last().unwrap()["schedule_id"]);
        assert_eq!(result["truncated"], true);
    }
}
