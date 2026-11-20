//! Opt-in trusted Python execution, not a sandbox. Code retains the user's
//! filesystem/network access and can deliberately escape its process group.
//! The caller owns grants, frozen source/dependency provenance and effect policy;
//! inspecting an interpreter here never grants permission to execute it.
//!
//! One UTF-8 source file is supported. stdin is a single versioned JSON request
//! `{protocol_version:"0.1",input,config,artifacts}`; stdout must be a versioned
//! JSON object `{protocol_version:"0.1",output,artifacts?:[...]}`. Artifact input
//! files are readonly snapshots, and declared flat output files are captured
//! after the process group exits. This is bounded exchange, not confinement.
//! Dependencies must already be
//! prepared in the explicitly approved interpreter. Nothing installs packages.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use apr_core::ArtifactRef;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_CODE_BYTES: usize = 1024 * 1024;
const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
const MAX_INTERPRETER_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARTIFACT_FILES: usize = 32;

/// A dispatch-time change detector, not an authorization grant or a proof of
/// dependency integrity. Native libraries/stdlib and hostile same-user path
/// replacement require a separately managed deployment, not this trusted mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpreterPin {
    pub path: PathBuf,
    pub resolved_path: PathBuf,
    pub sha256: String,
    pub device: u64,
    pub inode: u64,
    pub bytes: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
}

impl InterpreterPin {
    pub fn inspect(path: &Path) -> Result<Self, WorkerError> {
        if !path.is_absolute() {
            return Err(WorkerError::InvalidInterpreter);
        }
        let resolved_path = path
            .canonicalize()
            .map_err(|_| WorkerError::InvalidInterpreter)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&resolved_path)
            .map_err(|_| WorkerError::InvalidInterpreter)?;
        let before = file
            .metadata()
            .map_err(|_| WorkerError::InvalidInterpreter)?;
        if !before.is_file()
            || before.mode() & 0o111 == 0
            || before.mode() & 0o6000 != 0
            || before.len() == 0
            || before.len() > MAX_INTERPRETER_BYTES
        {
            return Err(WorkerError::InvalidInterpreter);
        }
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut bytes = 0_u64;
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| WorkerError::InvalidInterpreter)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > MAX_INTERPRETER_BYTES {
                return Err(WorkerError::InvalidInterpreter);
            }
            digest.update(&buffer[..count]);
        }
        let after = file
            .metadata()
            .map_err(|_| WorkerError::InvalidInterpreter)?;
        if bytes != before.len() || identity(&before) != identity(&after) {
            return Err(WorkerError::InterpreterChanged);
        }
        Ok(Self {
            path: path.to_path_buf(),
            resolved_path,
            sha256: format!("{:x}", digest.finalize()),
            device: before.dev(),
            inode: before.ino(),
            bytes,
            modified_seconds: before.mtime(),
            modified_nanoseconds: before.mtime_nsec(),
        })
    }
}

fn identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    )
}

#[derive(Clone, Copy, Debug)]
pub struct WorkerLimits {
    pub timeout: Duration,
    pub max_input_bytes: usize,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_artifact_bytes: u64,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_input_bytes: 128 * 1024,
            max_stdout_bytes: 64 * 1024,
            max_stderr_bytes: 8 * 1024,
            max_artifact_bytes: MAX_ARTIFACT_BYTES,
        }
    }
}

pub struct WorkerRequest<'a> {
    /// Runtime-issued run identity, never supplied by workflow input/config.
    pub run_id: Option<&'a str>,
    pub interpreter: &'a InterpreterPin,
    pub code: &'a [u8],
    pub input: &'a Value,
    pub config: &'a Value,
    pub artifacts: &'a [WorkerInputArtifact<'a>],
    pub limits: WorkerLimits,
}

pub struct WorkerInputArtifact<'a> {
    pub reference: &'a ArtifactRef,
    pub bytes: &'a [u8],
}

pub struct WorkerArtifact {
    pub name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for WorkerArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerArtifact")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

pub struct WorkerOutput {
    pub output: Value,
    pub artifacts: Vec<WorkerArtifact>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub elapsed_ms: u64,
}

impl std::fmt::Debug for WorkerOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerOutput")
            .field("stdout_bytes", &self.stdout_bytes)
            .field("stderr_bytes", &self.stderr_bytes)
            .field("elapsed_ms", &self.elapsed_ms)
            .field("artifact_count", &self.artifacts.len())
            .finish_non_exhaustive()
    }
}

/// Diagnostics contain categories and an exit code only, never source, payloads,
/// child stderr, interpreter paths, or a child-generated exception message.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkerError {
    #[error("trusted interpreter must be an absolute executable regular file")]
    InvalidInterpreter,
    #[error("trusted interpreter changed after it was pinned")]
    InterpreterChanged,
    #[error("worker limits are invalid")]
    InvalidLimits,
    #[error("worker source must be bounded UTF-8")]
    InvalidCode,
    #[error("worker input exceeded its byte limit")]
    InputLimit,
    #[error("worker could not be started")]
    Spawn,
    #[error("worker supervision failed")]
    Io,
    #[error("worker deadline exceeded; native effects may remain")]
    TimedOut,
    #[error("worker was cancelled; native effects may remain")]
    Cancelled,
    #[error("worker stdout exceeded its byte limit")]
    StdoutLimit,
    #[error("worker stderr exceeded its byte limit")]
    StderrLimit,
    #[error("worker output does not match protocol 0.1")]
    Protocol,
    #[error("worker artifact exchange does not match its contract")]
    ArtifactProtocol,
    #[error("worker artifact failed identity or regular-file verification")]
    ArtifactIntegrity,
    #[error("worker artifacts exceeded their combined byte or file limit")]
    ArtifactLimit,
    #[error("worker exited unsuccessfully (code {code:?})")]
    Exit { code: Option<i32> },
}

impl WorkerError {
    /// Stable category only; never copy child stderr or an exception message.
    pub fn diagnostic(&self) -> Value {
        let code = match self {
            Self::InvalidInterpreter => "invalid_interpreter",
            Self::InterpreterChanged => "interpreter_changed",
            Self::InvalidLimits => "invalid_limits",
            Self::InvalidCode => "invalid_source",
            Self::InputLimit => "input_limit",
            Self::Spawn => "spawn_failed",
            Self::Io => "supervision_failed",
            Self::TimedOut => "deadline_exceeded",
            Self::Cancelled => "cancelled",
            Self::StdoutLimit => "stdout_limit",
            Self::StderrLimit => "stderr_limit",
            Self::Protocol => "invalid_worker_protocol",
            Self::ArtifactProtocol => "invalid_artifact_protocol",
            Self::ArtifactIntegrity => "artifact_integrity",
            Self::ArtifactLimit => "artifact_limit",
            Self::Exit { .. } => "worker_exit",
        };
        let mut result = serde_json::json!({"stage":"native_worker","code":code});
        if let Self::Exit { code } = self {
            result["exit_code"] = serde_json::json!(code);
        }
        result
    }
}

#[derive(Serialize)]
struct InputEnvelope<'a> {
    protocol_version: &'static str,
    input: &'a Value,
    config: &'a Value,
    artifacts: BTreeMap<String, MaterializedArtifact<'a>>,
}

#[derive(Serialize)]
struct MaterializedArtifact<'a> {
    path: PathBuf,
    reference: &'a ArtifactRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputEnvelope {
    protocol_version: String,
    output: Value,
    #[serde(default)]
    artifacts: Vec<ArtifactDeclaration>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactDeclaration {
    name: String,
    path: String,
    media_type: String,
}

struct BoundedInput {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedInput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("input limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Executes a single caller-approved native script. Cancellation/timeout kill
/// the owned process group and reap its leader; they do not undo native effects.
pub fn execute(
    request: WorkerRequest<'_>,
    cancelled: &AtomicBool,
) -> Result<WorkerOutput, WorkerError> {
    let started = Instant::now();
    if cancelled.load(Ordering::Acquire) {
        return Err(WorkerError::Cancelled);
    }
    if request.limits.timeout.is_zero()
        || request.limits.timeout > Duration::from_secs(3600)
        || [
            request.limits.max_input_bytes,
            request.limits.max_stdout_bytes,
            request.limits.max_stderr_bytes,
        ]
        .iter()
        .any(|limit| *limit == 0 || *limit > MAX_CAPTURE_BYTES)
        || request.limits.max_artifact_bytes > MAX_ARTIFACT_BYTES
    {
        return Err(WorkerError::InvalidLimits);
    }
    if request.code.len() > MAX_CODE_BYTES || std::str::from_utf8(request.code).is_err() {
        return Err(WorkerError::InvalidCode);
    }
    if InterpreterPin::inspect(&request.interpreter.path)
        .map_err(|_| WorkerError::InterpreterChanged)?
        != *request.interpreter
    {
        return Err(WorkerError::InterpreterChanged);
    }
    let directory = tempfile::Builder::new()
        .prefix("apr-trusted-worker-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .map_err(|_| WorkerError::Io)?;
    let directory_handle = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.path())
        .map_err(|_| WorkerError::Io)?;
    let (artifacts, input_artifact_bytes) =
        materialize_artifacts(&request, directory.path(), cancelled, started)?;
    let mut input = BoundedInput {
        bytes: Vec::new(),
        limit: request.limits.max_input_bytes,
    };
    serde_json::to_writer(
        &mut input,
        &InputEnvelope {
            protocol_version: "0.1",
            input: request.input,
            config: request.config,
            artifacts,
        },
    )
    .map_err(|_| WorkerError::InputLimit)?;
    let script = directory.path().join("script.py");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&script)
        .map_err(|_| WorkerError::Io)?;
    file.write_all(request.code).map_err(|_| WorkerError::Io)?;
    drop(file);
    check_stop(cancelled, started, request.limits.timeout)?;
    let child = Command::new(&request.interpreter.path)
        .args(["-I", "-B", "-u", "-X", "utf8"])
        .arg(&script)
        .current_dir(directory.path())
        .env_clear()
        .envs(request.run_id.map(|id| ("APR_RUN_ID", id)))
        .env("HOME", directory.path())
        .env("TMPDIR", directory.path())
        .env("TMP", directory.path())
        .env("TEMP", directory.path())
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|_| WorkerError::Spawn)?;
    let mut child = Supervised::new(child)?;
    let output = supervise(&mut child, &input.bytes, request.limits, cancelled, started)?;
    let result: OutputEnvelope =
        serde_json::from_slice(&output.stdout).map_err(|_| WorkerError::Protocol)?;
    if result.protocol_version != "0.1" {
        return Err(WorkerError::Protocol);
    }
    let artifacts = capture_artifacts(
        &directory_handle,
        &result,
        request.artifacts.len(),
        input_artifact_bytes,
        request.limits,
        cancelled,
        started,
    )?;
    Ok(WorkerOutput {
        output: result.output,
        artifacts,
        stdout_bytes: output.stdout.len() as u64,
        stderr_bytes: output.stderr_bytes,
        elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

fn check_stop(
    cancelled: &AtomicBool,
    started: Instant,
    timeout: Duration,
) -> Result<(), WorkerError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(WorkerError::Cancelled);
    }
    if started.elapsed() >= timeout {
        return Err(WorkerError::TimedOut);
    }
    Ok(())
}

fn materialize_artifacts<'a>(
    request: &WorkerRequest<'a>,
    directory: &Path,
    cancelled: &AtomicBool,
    started: Instant,
) -> Result<(BTreeMap<String, MaterializedArtifact<'a>>, u64), WorkerError> {
    if request.artifacts.len() > MAX_ARTIFACT_FILES {
        return Err(WorkerError::ArtifactLimit);
    }
    let mut files = BTreeMap::new();
    let mut total = 0_u64;
    for artifact in request.artifacts {
        check_stop(cancelled, started, request.limits.timeout)?;
        let reference = artifact.reference;
        crate::artifacts::validate_reference(reference)
            .map_err(|_| WorkerError::ArtifactIntegrity)?;
        if reference.bytes != artifact.bytes.len() as u64 {
            return Err(WorkerError::ArtifactIntegrity);
        }
        total = total
            .checked_add(reference.bytes)
            .ok_or(WorkerError::ArtifactLimit)?;
        if total > request.limits.max_artifact_bytes {
            return Err(WorkerError::ArtifactLimit);
        }
        if files.contains_key(&reference.artifact_id) {
            return Err(WorkerError::ArtifactProtocol);
        }
        let mut hasher = Sha256::new();
        for chunk in artifact.bytes.chunks(64 * 1024) {
            check_stop(cancelled, started, request.limits.timeout)?;
            hasher.update(chunk);
        }
        if format!("{:x}", hasher.finalize()) != reference.sha256 {
            return Err(WorkerError::ArtifactIntegrity);
        }
        let path = directory.join(format!("input-{}", reference.sha256));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|_| WorkerError::Io)?;
        for chunk in artifact.bytes.chunks(64 * 1024) {
            check_stop(cancelled, started, request.limits.timeout)?;
            file.write_all(chunk).map_err(|_| WorkerError::Io)?;
        }
        files.insert(
            reference.artifact_id.clone(),
            MaterializedArtifact { path, reference },
        );
    }
    Ok((files, total))
}

fn exchange_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn capture_artifacts(
    directory: &File,
    result: &OutputEnvelope,
    input_count: usize,
    input_bytes: u64,
    limits: WorkerLimits,
    cancelled: &AtomicBool,
    started: Instant,
) -> Result<Vec<WorkerArtifact>, WorkerError> {
    if result.artifacts.len() > MAX_ARTIFACT_FILES.saturating_sub(input_count) {
        return Err(WorkerError::ArtifactLimit);
    }
    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for artifact in &result.artifacts {
        if !exchange_name(&artifact.name)
            || !exchange_name(&artifact.path)
            || artifact.path == "script.py"
            || artifact.path.starts_with("input-")
            || !names.insert(artifact.name.as_str())
            || !paths.insert(artifact.path.as_str())
        {
            return Err(WorkerError::ArtifactProtocol);
        }
        crate::artifacts::validate_media_type(&artifact.media_type)
            .map_err(|_| WorkerError::ArtifactProtocol)?;
    }
    replace_placeholders(
        &mut result.output.clone(),
        &|name| {
            if names.contains(name) {
                Ok(Value::Null)
            } else {
                Err(WorkerError::ArtifactProtocol)
            }
        },
        0,
    )?;
    let mut total = input_bytes;
    let mut captured = Vec::new();
    for artifact in &result.artifacts {
        check_stop(cancelled, started, limits.timeout)?;
        let name =
            CString::new(artifact.path.as_str()).map_err(|_| WorkerError::ArtifactProtocol)?;
        // SAFETY: the flat basename was validated; directory is pinned before
        // execution, so child renames cannot redirect this read to another tree.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(WorkerError::ArtifactIntegrity);
        }
        // SAFETY: openat returned a uniquely owned live descriptor.
        let mut file = unsafe { File::from_raw_fd(fd) };
        let before = file
            .metadata()
            .map_err(|_| WorkerError::ArtifactIntegrity)?;
        if !before.is_file() || before.nlink() != 1 {
            return Err(WorkerError::ArtifactIntegrity);
        }
        if before.len() > limits.max_artifact_bytes.saturating_sub(total) {
            return Err(WorkerError::ArtifactLimit);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(
                usize::try_from(before.len()).map_err(|_| WorkerError::ArtifactLimit)?,
            )
            .map_err(|_| WorkerError::ArtifactLimit)?;
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            check_stop(cancelled, started, limits.timeout)?;
            let count = file
                .read(&mut chunk)
                .map_err(|_| WorkerError::ArtifactIntegrity)?;
            if count == 0 {
                break;
            }
            if count as u64 > before.len().saturating_sub(bytes.len() as u64) {
                return Err(WorkerError::ArtifactIntegrity);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        let after = file
            .metadata()
            .map_err(|_| WorkerError::ArtifactIntegrity)?;
        if bytes.len() as u64 != before.len()
            || identity(&before) != identity(&after)
            || after.nlink() != 1
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err(WorkerError::ArtifactIntegrity);
        }
        total += bytes.len() as u64;
        captured.push(WorkerArtifact {
            name: artifact.name.clone(),
            media_type: artifact.media_type.clone(),
            bytes,
        });
    }
    Ok(captured)
}

/// Replace only exact single-key `{ "worker_artifact": "name" }` objects.
/// Ordinary strings and objects with additional fields remain ordinary data.
pub fn substitute_artifacts(
    mut output: Value,
    references: &BTreeMap<String, ArtifactRef>,
) -> Result<Value, WorkerError> {
    replace_placeholders(
        &mut output,
        &|name| {
            let reference = references.get(name).ok_or(WorkerError::ArtifactProtocol)?;
            crate::artifacts::validate_reference(reference)
                .map_err(|_| WorkerError::ArtifactIntegrity)?;
            serde_json::to_value(reference).map_err(|_| WorkerError::ArtifactProtocol)
        },
        0,
    )?;
    Ok(output)
}

fn replace_placeholders(
    value: &mut Value,
    resolve: &impl Fn(&str) -> Result<Value, WorkerError>,
    depth: usize,
) -> Result<(), WorkerError> {
    if depth > 128 {
        return Err(WorkerError::ArtifactProtocol);
    }
    match value {
        Value::Object(object) if object.len() == 1 && object.contains_key("worker_artifact") => {
            let name = object["worker_artifact"]
                .as_str()
                .ok_or(WorkerError::ArtifactProtocol)?;
            *value = resolve(name)?;
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                replace_placeholders(item, resolve, depth + 1)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_placeholders(item, resolve, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

struct Supervised {
    child: Child,
    group: libc::pid_t,
    reaped: bool,
}
impl Supervised {
    fn new(mut child: Child) -> Result<Self, WorkerError> {
        let Ok(group) = libc::pid_t::try_from(child.id()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(WorkerError::Spawn);
        };
        Ok(Self {
            child,
            group,
            reaped: false,
        })
    }

    fn exited(&self) -> Result<bool, WorkerError> {
        // WNOWAIT deliberately keeps the leader unreaped, reserving its PID
        // while group cleanup runs. try_wait would create a PID reuse race.
        // SAFETY: siginfo is fully initialized and waitid targets our own child.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let status = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if status != 0 {
            return Err(WorkerError::Io);
        }
        Ok(info.si_signo != 0)
    }

    fn terminate_group(&self) {
        // SAFETY: this group belongs to our unreaped child. Native trusted code
        // may have escaped it; no unrelated PIDs or executable names are scanned.
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
    }

    fn finish(&mut self) -> Result<ExitStatus, WorkerError> {
        self.terminate_group();
        let status = self.child.wait().map_err(|_| WorkerError::Io)?;
        self.reaped = true;
        Ok(status)
    }
}
impl Drop for Supervised {
    fn drop(&mut self) {
        if !self.reaped {
            self.terminate_group();
            let _ = self.child.wait();
        }
    }
}

struct Captured {
    stdout: Vec<u8>,
    stderr_bytes: u64,
}

fn nonblocking(file: &impl AsRawFd) -> Result<(), WorkerError> {
    // SAFETY: AsRawFd borrows a live descriptor for both fcntl calls.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags == -1
        || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
    {
        return Err(WorkerError::Io);
    }
    Ok(())
}

fn supervise(
    child: &mut Supervised,
    input: &[u8],
    limits: WorkerLimits,
    cancelled: &AtomicBool,
    started: Instant,
) -> Result<Captured, WorkerError> {
    let mut stdin = Some(child.child.stdin.take().ok_or(WorkerError::Io)?);
    let mut stdout = child.child.stdout.take().ok_or(WorkerError::Io)?;
    let mut stderr = child.child.stderr.take().ok_or(WorkerError::Io)?;
    nonblocking(stdin.as_ref().unwrap())?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let mut sent = 0;
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut result = Captured {
        stdout: Vec::new(),
        stderr_bytes: 0,
    };
    let mut status = None;
    let mut buffer = [0_u8; 8192];
    loop {
        check_stop(cancelled, started, limits.timeout)?;
        for is_stdout in [true, false] {
            let open = if is_stdout {
                &mut stdout_open
            } else {
                &mut stderr_open
            };
            let reader: &mut dyn Read = if is_stdout { &mut stdout } else { &mut stderr };
            while *open {
                check_stop(cancelled, started, limits.timeout)?;
                match reader.read(&mut buffer) {
                    Ok(0) => *open = false,
                    Ok(count) => {
                        if is_stdout {
                            if count > limits.max_stdout_bytes.saturating_sub(result.stdout.len()) {
                                return Err(WorkerError::StdoutLimit);
                            }
                            result.stdout.extend_from_slice(&buffer[..count]);
                        } else {
                            result.stderr_bytes += count as u64;
                            if result.stderr_bytes > limits.max_stderr_bytes as u64 {
                                return Err(WorkerError::StderrLimit);
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(WorkerError::Io),
                }
            }
        }
        if let Some(pipe) = stdin.as_mut() {
            match pipe.write(&input[sent..]) {
                Ok(count) => {
                    sent += count;
                    if sent == input.len() {
                        stdin = None;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => stdin = None,
                Err(_) => return Err(WorkerError::Io),
            }
            // Dropping the completed stdin pipe lets json.load(sys.stdin) reach EOF.
        }
        if status.is_none() && child.exited()? {
            status = Some(child.finish()?);
        }
        if let Some(status) = status
            && !stdout_open
            && !stderr_open
        {
            return complete(status, result);
        }
        let pipes = [
            (stdin.as_ref().map_or(-1, AsRawFd::as_raw_fd), libc::POLLOUT),
            (
                if stdout_open { stdout.as_raw_fd() } else { -1 },
                libc::POLLIN,
            ),
            (
                if stderr_open { stderr.as_raw_fd() } else { -1 },
                libc::POLLIN,
            ),
        ];
        wait_pipes(&pipes, started, limits.timeout)?;
    }
}

fn complete(status: ExitStatus, result: Captured) -> Result<Captured, WorkerError> {
    if !status.success() {
        return Err(WorkerError::Exit {
            code: status.code(),
        });
    }
    Ok(result)
}

fn wait_pipes(
    pipes: &[(libc::c_int, libc::c_short)],
    started: Instant,
    timeout: Duration,
) -> Result<(), WorkerError> {
    let remaining = timeout.saturating_sub(started.elapsed());
    let millis = remaining.as_millis().clamp(1, 20) as libc::c_int;
    let mut descriptors: Vec<libc::pollfd> = pipes
        .iter()
        .map(|(fd, events)| libc::pollfd {
            fd: *fd,
            events: *events,
            revents: 0,
        })
        .collect();
    // SAFETY: the array is writable for its full declared length; descriptors
    // are live borrowed pipes or -1 entries ignored by poll.
    let result = unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            millis,
        )
    };
    if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
        return Err(WorkerError::Io);
    }
    Ok(())
}
