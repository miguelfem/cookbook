use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Error, Result, digest};

pub(crate) const MAX_FILE_BYTES: usize = 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Root {
    pub path: PathBuf,
    directory: File,
    private: bool,
}

pub(crate) struct Lock(File);

impl Drop for Lock {
    fn drop(&mut self) {
        // Closing this descriptor alone leaves flock held by any duplicate or
        // briefly inherited pre-exec descriptor. Release ownership explicitly.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

impl Root {
    pub fn open(path: &Path, private: bool) -> Result<Self> {
        let path = path.canonicalize()?;
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        check_directory(&directory, private)?;
        Ok(Self {
            path,
            directory,
            private,
        })
    }

    fn parent(&self, relative: &Path, create: bool) -> Result<Option<(File, CString)>> {
        let parts: Vec<_> = relative.components().collect();
        if parts.is_empty()
            || parts
                .iter()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(Error::InvalidInput("invalid_relative_path"));
        }
        let mut directory = self.directory.try_clone()?;
        for part in &parts[..parts.len() - 1] {
            let Component::Normal(name) = part else {
                unreachable!()
            };
            let name = cstring(name.as_bytes())?;
            if create {
                // SAFETY: the descriptor and component C string remain valid.
                let made = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                if made == -1 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                    return Err(io::Error::last_os_error().into());
                }
                if made == 0 {
                    directory.sync_all()?;
                }
            }
            // SAFETY: no-follow traversal remains anchored to the owned parent.
            let raw = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw == -1 {
                let error = io::Error::last_os_error();
                if !create && error.kind() == io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(Error::UnsafePath);
            }
            // SAFETY: successful openat gives this File a new owned descriptor.
            directory = unsafe { File::from_raw_fd(raw) };
            check_directory(&directory, self.private)?;
        }
        let Component::Normal(name) = parts.last().unwrap() else {
            unreachable!()
        };
        Ok(Some((directory, cstring(name.as_bytes())?)))
    }

    pub fn read(&self, relative: &Path) -> Result<Option<Vec<u8>>> {
        let Some((directory, name)) = self.parent(relative, false)? else {
            return Ok(None);
        };
        // SAFETY: the anchored descriptor and name remain live; nonblocking avoids FIFOs.
        let raw = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if raw == -1 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(Error::UnsafePath)
            };
        }
        // SAFETY: successful openat returns a new owned descriptor.
        let file = unsafe { File::from_raw_fd(raw) };
        let metadata = file.metadata()?;
        check_regular(&metadata, self.private)?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(Error::LimitExceeded);
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::LimitExceeded);
        }
        Ok(Some(bytes))
    }

    pub fn write(
        &self,
        relative: &Path,
        expected: Option<&str>,
        bytes: Option<&[u8]>,
    ) -> Result<()> {
        if bytes.is_some_and(|bytes| bytes.len() > MAX_FILE_BYTES) {
            return Err(Error::LimitExceeded);
        }
        let current = self.read(relative)?;
        if current.as_deref().map(digest).as_deref() != expected {
            return Err(Error::Conflict("file_changed"));
        }
        if current.as_deref() == bytes {
            return Ok(());
        }
        let Some((directory, name)) = self.parent(relative, bytes.is_some())? else {
            return Ok(());
        };
        if bytes.is_none() {
            // SAFETY: unlink the checked directory entry, without following a symlink.
            if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } == -1 {
                return Err(io::Error::last_os_error().into());
            }
            directory.sync_all()?;
            return Ok(());
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::InvalidInput("invalid_clock"))?
            .as_nanos();
        let temporary = cstring(
            format!(
                ".apr-write-{}-{}-{stamp}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            )
            .as_bytes(),
        )?;
        // SAFETY: exclusive creation cannot clobber another file or follow a symlink.
        let raw = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if raw == -1 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: successful openat returns a new owned descriptor.
        let mut output = unsafe { File::from_raw_fd(raw) };
        let result = (|| {
            output.write_all(bytes.unwrap())?;
            output.sync_all()?;
            if self.read(relative)?.as_deref().map(digest).as_deref() != expected {
                return Err(Error::Conflict("file_changed"));
            }
            let changed = if expected.is_none() {
                rename_new(&directory, &temporary, &name)
            } else {
                // SAFETY: rename is confined to the validated parent descriptor.
                unsafe {
                    libc::renameat(
                        directory.as_raw_fd(),
                        temporary.as_ptr(),
                        directory.as_raw_fd(),
                        name.as_ptr(),
                    )
                }
            };
            if changed == -1 {
                return Err(io::Error::last_os_error().into());
            }
            directory.sync_all()?;
            Ok(())
        })();
        // SAFETY: this is our exclusive temporary name; final publication is retained.
        unsafe { libc::unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0) };
        result
    }

    pub fn lock(&self, relative: &Path) -> Result<Lock> {
        let (directory, name) = self.parent(relative, true)?.ok_or(Error::UnsafePath)?;
        // SAFETY: a permanent no-follow lock inode is created privately if absent.
        let raw = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0o600,
            )
        };
        if raw == -1 {
            return Err(Error::UnsafePath);
        }
        // SAFETY: successful openat returns a new owned descriptor.
        let file = unsafe { File::from_raw_fd(raw) };
        check_regular(&file.metadata()?, true)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| match error.kind() {
            io::ErrorKind::WouldBlock => Error::Busy,
            _ => Error::Io(error),
        })?;
        Ok(Lock(file))
    }
}

fn rename_new(directory: &File, from: &CString, to: &CString) -> i32 {
    // These platform operations publish one inode atomically and refuse to
    // replace an existing name, including a dangling symlink.
    #[cfg(target_os = "macos")]
    // SAFETY: the live descriptor and both C strings belong to this operation.
    unsafe {
        libc::renameatx_np(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    }
    #[cfg(target_os = "linux")]
    // SAFETY: the live descriptor and both C strings belong to this operation.
    unsafe {
        libc::renameat2(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    }
}

fn cstring(bytes: &[u8]) -> Result<CString> {
    CString::new(bytes).map_err(|_| Error::InvalidInput("invalid_path"))
}

fn check_directory(file: &File, private: bool) -> Result<()> {
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & if private { 0o077 } else { 0o022 } != 0
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

pub(crate) fn check_regular(metadata: &std::fs::Metadata, private: bool) -> Result<()> {
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & if private { 0o077 } else { 0o022 } != 0
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn dropping_owner_unlocks_even_when_its_descriptor_was_duplicated() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = Root::open(directory.path(), true).unwrap();
        let path = Path::new("integration.lock");
        let owner = root.lock(path).unwrap();
        // dup and fork share the same open-file description. CLOEXEC does not
        // close an inherited descriptor until the child actually reaches exec.
        let duplicate = owner.0.try_clone().unwrap();
        let independent = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.path().join(path))
            .unwrap();
        let held = fs2::FileExt::try_lock_exclusive(&independent).unwrap_err();
        assert_eq!(held.kind(), io::ErrorKind::WouldBlock);
        assert!(matches!(root.lock(path), Err(Error::Busy)));

        drop(owner);
        let acquired = fs2::FileExt::try_lock_exclusive(&independent);
        assert!(
            acquired.is_ok(),
            "released owner retained its flock: {acquired:?}"
        );
        assert!(matches!(root.lock(path), Err(Error::Busy)));
        // Closing a duplicate from the old owner cannot release the new owner.
        drop(duplicate);
        assert!(matches!(root.lock(path), Err(Error::Busy)));
        fs2::FileExt::unlock(&independent).unwrap();

        let next = root.lock(path).unwrap();
        assert!(matches!(root.lock(path), Err(Error::Busy)));
        drop(next);
        root.lock(path).unwrap();
    }
}
