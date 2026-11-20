use std::fs;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::{Client, ClientStatus, Error, Result};

pub(crate) fn ensure_untracked(project: &Path, relative: &Path) -> Result<()> {
    if !project
        .ancestors()
        .any(|parent| fs::symlink_metadata(parent.join(".git")).is_ok())
    {
        return Ok(());
    }
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_"))
    {
        command.env_remove(key);
    }
    command
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false", "-C"])
        .arg(project)
        .args(["ls-files", "-z", "--error-unmatch", "--"])
        .arg(relative);
    match bounded_output(command, 4096) {
        Some((status, _)) if status.success() => Err(Error::Conflict("config_is_tracked")),
        Some((status, _)) if status.code() == Some(1) => Ok(()),
        _ => Err(Error::Conflict("git_tracking_unknown")),
    }
}

pub(crate) fn client(client: Client, supplied: Option<&Path>) -> ClientStatus {
    let candidates = supplied
        .map(|path| vec![path.to_path_buf()])
        .unwrap_or_else(|| {
            let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
                .map(|paths| {
                    std::env::split_paths(&paths)
                        .filter(|path| path.is_absolute())
                        .map(|path| path.join(client.name()))
                        .collect()
                })
                .unwrap_or_default();
            if client == Client::Codex {
                paths.push("/Applications/ChatGPT.app/Contents/Resources/codex".into());
            }
            paths
        });
    let mut failure = None;
    for candidate in candidates {
        let Ok(path) = candidate.canonicalize() else {
            continue;
        };
        if !fs::metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        {
            continue;
        }
        let mut command = Command::new(&path);
        command.arg("--version");
        if let Some((status, output)) = bounded_output(command, 4096)
            && status.success()
            && let Ok(version) = String::from_utf8(output)
            && let Some(version) = version.lines().find(|line| !line.trim().is_empty())
            && version.len() <= 200
            && version.chars().all(|value| !value.is_control())
        {
            return ClientStatus {
                status: "available".into(),
                executable: Some(path),
                version: Some(version.trim().into()),
            };
        }
        failure = Some(path);
    }
    ClientStatus {
        status: if failure.is_some() {
            "unavailable"
        } else {
            "missing"
        }
        .into(),
        executable: failure,
        version: None,
    }
}

/// Only used for read-only local version and Git-index probes. The process gets
/// no stdin, its stderr is discarded, and both output and elapsed time are capped.
pub(crate) fn bounded_output(mut command: Command, limit: usize) -> Option<(ExitStatus, Vec<u8>)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let mut output = child.stdout.take()?;
    // SAFETY: the live pipe descriptor supports fcntl; O_NONBLOCK prevents a
    // client wrapper or descendant from holding this diagnostic open forever.
    let flags = unsafe { libc::fcntl(output.as_raw_fd(), libc::F_GETFL) };
    let nonblocking = flags != -1
        && unsafe { libc::fcntl(output.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
            != -1;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = Vec::new();
    let mut status = None;
    let result = if nonblocking {
        loop {
            let mut buffer = [0_u8; 1024];
            let eof = match output.read(&mut buffer) {
                Ok(0) => true,
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    false
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break None,
            };
            if bytes.len() > limit || Instant::now() >= deadline {
                break None;
            }
            if status.is_none() {
                match child.try_wait() {
                    Ok(value) => status = value,
                    Err(_) => break None,
                }
            }
            if eof && let Some(status) = status {
                break Some((status, bytes));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    } else {
        None
    };
    if result.is_none() {
        // SAFETY: this is the process group created for this owned probe only.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    result
}
