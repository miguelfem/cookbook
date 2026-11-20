//! Bounded, immutable objects.
//!
//! Atomic exclusive publication adapts `writeFileAtomicExclusive` from OpenClaw
//! Lobster, MIT, Copyright (c) 2026 Vignesh:
//! https://github.com/openclaw/lobster/blob/041124ad4ae5f05179302970a235fe5d34bac45b/src/state/atomic_file.ts
//! See THIRD_PARTY_NOTICES.md. Unlike that implementation, a published object is
//! never unlinked on a later sync failure; recovery verifies its content.
//!
//! Directory descriptors keep publication anchored
//! even if a path component is renamed while an operation is in progress.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;

use apr_core::ArtifactRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("invalid artifact reference: {0}")]
    InvalidReference(String),
    #[error("invalid media type: {0}")]
    InvalidMediaType(String),
    #[error("artifact exceeds the byte limit of {limit}")]
    TooLarge { limit: u64 },
    #[error("managed artifact quota of {limit} bytes would be exceeded")]
    QuotaExceeded { limit: u64 },
    #[error("artifact {digest} failed integrity verification: {reason}")]
    Corrupt { digest: String, reason: String },
    #[error("unsafe artifact path: {0}")]
    UnsafePath(String),
    #[error("artifact I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("artifact ZIP failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct ArtifactStore {
    objects: File,
}

/// A maintenance snapshot, bound to an exact directory entry and inode.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredObject {
    pub name: String,
    pub bytes: u64,
    pub modified_ms: i64,
    device: u64,
    inode: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

fn maintenance_name(name: &str) -> bool {
    (name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        || name
            .strip_prefix(".stage-")
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

impl ArtifactStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ArtifactError> {
        let root = root.as_ref();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)?;
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)
            .map_err(|error| path_error(error, "state directory"))?;
        let objects = directory_at(&root, "objects")?;
        Ok(Self {
            objects: directory_at(&objects, "sha256")?,
        })
    }

    /// Open a backup's existing object tree without mutating the source backup.
    pub(crate) fn open_existing(root: &Path) -> Result<Self, ArtifactError> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(root)?;
        let objects = file_at(&root, "objects", flags, 0)?;
        Ok(Self {
            objects: file_at(&objects, "sha256", flags, 0)?,
        })
    }

    pub fn put(
        &self,
        bytes: &[u8],
        media_type: &str,
        max_bytes: u64,
    ) -> Result<ArtifactRef, ArtifactError> {
        self.put_with_quota(bytes, media_type, max_bytes, None)
    }

    /// The runtime holds its store lock across quota lookup and publication.
    pub(crate) fn put_with_quota(
        &self,
        bytes: &[u8],
        media_type: &str,
        max_bytes: u64,
        quota: Option<u64>,
    ) -> Result<ArtifactRef, ArtifactError> {
        validate_media_type(media_type)?;
        enforce_limit(bytes.len() as u64, max_bytes)?;
        self.publish(&mut &bytes[..], media_type, max_bytes, quota)
    }

    pub fn import(
        &self,
        path: &Path,
        media_type: &str,
        max_bytes: u64,
    ) -> Result<ArtifactRef, ArtifactError> {
        self.import_with_quota(path, media_type, max_bytes, None)
    }

    pub(crate) fn import_with_quota(
        &self,
        path: &Path,
        media_type: &str,
        max_bytes: u64,
        quota: Option<u64>,
    ) -> Result<ArtifactRef, ArtifactError> {
        validate_media_type(media_type)?;
        // O_NONBLOCK prevents opening a pipe/device from hanging before the
        // regular-file check. Symlinks are deliberately unsupported imports.
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| path_error(error, "import source"))?;
        let metadata = source.metadata()?;
        if !metadata.is_file() {
            return Err(ArtifactError::UnsafePath(
                "import source must be a regular file".into(),
            ));
        }
        enforce_limit(metadata.len(), max_bytes)?;
        self.publish(&mut source, media_type, max_bytes, quota)
    }

    pub fn read(&self, reference: &ArtifactRef, max_bytes: u64) -> Result<Vec<u8>, ArtifactError> {
        let mut file = self.checked_file(reference, max_bytes)?;
        let capacity = usize::try_from(reference.bytes)
            .map_err(|_| ArtifactError::TooLarge { limit: max_bytes })?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let (size, digest) = copy_bounded(&mut file, &mut bytes, reference.bytes)?;
        check_contents(reference, size, &digest)?;
        Ok(bytes)
    }

    pub fn verify(&self, reference: &ArtifactRef, max_bytes: u64) -> Result<(), ArtifactError> {
        let mut file = self.checked_file(reference, max_bytes)?;
        let (size, digest) = copy_bounded(&mut file, &mut io::sink(), reference.bytes)?;
        check_contents(reference, size, &digest)
    }

    /// Stream a verified immutable snapshot without allocating the whole object.
    /// Maintenance callers hold the runtime lock while copying referenced data.
    pub(crate) fn copy_to(
        &self,
        reference: &ArtifactRef,
        writer: &mut impl Write,
    ) -> Result<(), ArtifactError> {
        let mut file = self.checked_file(reference, reference.bytes)?;
        let (bytes, digest) = copy_bounded(&mut file, writer, reference.bytes)?;
        check_contents(reference, bytes, &digest)
    }

    /// Inventory only names owned by this store, relative to the descriptor that
    /// was pinned when it opened. Unknown files are deliberately left alone.
    pub(crate) fn inventory(&self) -> Result<Vec<StoredObject>, ArtifactError> {
        let directory = file_at(
            &self.objects,
            ".",
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0,
        )?;
        let fd = directory.into_raw_fd();
        // SAFETY: fd is a newly owned directory descriptor. fdopendir assumes
        // ownership on success; on failure it remains ours to close.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: the failed fdopendir did not consume fd.
            unsafe {
                libc::close(fd);
            }
            return Err(error.into());
        }
        struct DirectoryStream(*mut libc::DIR);
        impl Drop for DirectoryStream {
            fn drop(&mut self) {
                // SAFETY: this guard uniquely owns the live DIR pointer.
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let stream = DirectoryStream(stream);
        let mut objects = Vec::new();
        loop {
            // Reset errno so EOF can be distinguished from a directory error.
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(target_os = "linux")]
            unsafe {
                *libc::__errno_location() = 0;
            }
            // SAFETY: stream is live; the returned entry is read before the next
            // call mutates its storage.
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error().unwrap_or(0) != 0 {
                    return Err(error.into());
                }
                break;
            }
            // SAFETY: readdir guarantees a terminated d_name for a valid entry.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            let Ok(name) = name.to_str() else {
                continue;
            };
            if maintenance_name(name) {
                objects.push(self.inventory_entry(name)?);
            }
        }
        objects.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(objects)
    }

    pub(crate) fn remove_snapshot(&self, object: &StoredObject) -> Result<(), ArtifactError> {
        if !maintenance_name(&object.name) || self.inventory_entry(&object.name)? != *object {
            return Err(ArtifactError::UnsafePath(
                "object changed after retention preview".into(),
            ));
        }
        let name = CString::new(object.name.as_str())
            .map_err(|_| ArtifactError::UnsafePath("invalid object name".into()))?;
        // SAFETY: the basename was validated and the pinned directory is live.
        let removed = unsafe { libc::unlinkat(self.objects.as_raw_fd(), name.as_ptr(), 0) };
        if removed != 0 {
            return Err(io::Error::last_os_error().into());
        }
        self.objects.sync_all()?;
        Ok(())
    }

    fn inventory_entry(&self, name: &str) -> Result<StoredObject, ArtifactError> {
        let file = file_at(
            &self.objects,
            name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(ArtifactError::UnsafePath(
                "object inventory requires regular files".into(),
            ));
        }
        Ok(StoredObject {
            name: name.into(),
            bytes: metadata.len(),
            modified_ms: metadata
                .mtime()
                .saturating_mul(1000)
                .saturating_add(metadata.mtime_nsec() / 1_000_000),
            device: metadata.dev(),
            inode: metadata.ino(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }

    /// Export a deterministic ZIP. Input order and repeated references do not
    /// affect its digest; conflicting metadata for the same object is rejected.
    pub fn bundle(
        &self,
        refs: &[ArtifactRef],
        report: Option<&serde_json::Value>,
        max_bytes: u64,
    ) -> Result<ArtifactRef, ArtifactError> {
        self.bundle_with_quota(refs, report, max_bytes, None)
    }

    pub(crate) fn bundle_with_quota(
        &self,
        refs: &[ArtifactRef],
        report: Option<&serde_json::Value>,
        max_bytes: u64,
        quota: Option<u64>,
    ) -> Result<ArtifactRef, ArtifactError> {
        let mut artifacts = BTreeMap::new();
        for reference in refs {
            validate_reference(reference)?;
            enforce_limit(reference.bytes, max_bytes)?;
            if let Some(previous) = artifacts.insert(reference.sha256.as_str(), reference)
                && previous != reference
            {
                return Err(ArtifactError::InvalidReference(
                    "conflicting metadata for the same digest".into(),
                ));
            }
        }
        let manifest = serde_json::json!({
            "format": "apr-artifact-bundle-v1",
            "artifacts": artifacts.values().collect::<Vec<_>>(),
            "report": report,
        });
        // Reject predictable size failures before opening the ZIP writer. Its
        // destructor attempts finalization after errors, so preflighting also
        // keeps routine quota errors out of the library's stderr diagnostics.
        let mut counter = CountingWriter {
            bytes: 0,
            limit: max_bytes,
        };
        serde_json::to_writer(&mut counter, &manifest)
            .map_err(ArtifactError::from)
            .map_err(|error| normalize_limit_error(error, max_bytes))?;
        let mut archive_bytes = 22_u64; // End-of-central-directory record.
        for (name_bytes, content_bytes) in std::iter::once((13_u64, counter.bytes))
            .chain(artifacts.values().map(|reference| (72, reference.bytes)))
        {
            // Local and central headers plus two copies of the ASCII filename.
            archive_bytes = archive_bytes
                .checked_add(76 + 2 * name_bytes)
                .and_then(|bytes| bytes.checked_add(content_bytes))
                .ok_or(ArtifactError::TooLarge { limit: max_bytes })?;
            enforce_limit(archive_bytes, max_bytes)?;
        }
        let mut staged = StagedObject::new(&self.objects)?;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        let build = (|| -> Result<(), ArtifactError> {
            let mut zip = zip::ZipWriter::new(LimitedWriter {
                file: &mut staged.file,
                max_bytes,
                position: 0,
            });
            zip.start_file("manifest.json", options)?;
            serde_json::to_writer(&mut zip, &manifest)?;
            for reference in artifacts.values() {
                let mut source = self.checked_file(reference, max_bytes)?;
                zip.start_file(format!("objects/{}", reference.sha256), options)?;
                let (bytes, digest) = copy_bounded(&mut source, &mut zip, reference.bytes)?;
                check_contents(reference, bytes, &digest)?;
            }
            zip.finish()?;
            Ok(())
        })();
        build.map_err(|error| normalize_limit_error(error, max_bytes))?;
        staged.file.rewind()?;
        // Publication independently hashes the final archive and uses the same
        // fsync/no-clobber path as imported objects. The intermediate is bounded.
        self.publish(&mut staged.file, "application/zip", max_bytes, quota)
    }

    fn checked_file(&self, reference: &ArtifactRef, max_bytes: u64) -> Result<File, ArtifactError> {
        validate_reference(reference)?;
        enforce_limit(reference.bytes, max_bytes)?;
        let file = file_at(
            &self.objects,
            &reference.sha256,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )
        .map_err(|error| path_error(error, &reference.sha256))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(ArtifactError::UnsafePath(
                "object must be a regular file".into(),
            ));
        }
        if metadata.len() != reference.bytes {
            return Err(corrupt(reference, "stored size differs from the reference"));
        }
        Ok(file)
    }

    fn publish(
        &self,
        input: &mut impl Read,
        media_type: &str,
        max_bytes: u64,
        quota: Option<u64>,
    ) -> Result<ArtifactRef, ArtifactError> {
        let mut staged = StagedObject::new(&self.objects)?;
        let (bytes, sha256) = copy_bounded(input, &mut staged.file, max_bytes)?;
        let reference = ArtifactRef {
            artifact_id: format!("sha256:{sha256}"),
            sha256,
            media_type: media_type.into(),
            bytes,
        };
        staged.file.sync_all()?;
        // Cached objects never consume new quota, but must pass the same full
        // integrity verification even when the configured quota was lowered.
        match self.verify(&reference, max_bytes) {
            Ok(()) => return Ok(reference),
            Err(ArtifactError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if let Some(limit) = quota {
            let used = self
                .inventory()?
                .iter()
                .filter(|object| object.name.len() == 64)
                .try_fold(0_u64, |sum, object| {
                    sum.checked_add(object.bytes)
                        .ok_or(ArtifactError::QuotaExceeded { limit })
                })?;
            if used
                .checked_add(reference.bytes)
                .is_none_or(|bytes| bytes > limit)
            {
                return Err(ArtifactError::QuotaExceeded { limit });
            }
        }
        match link_at(&self.objects, &staged.name, &reference.sha256) {
            Ok(()) => {
                self.objects.sync_all()?;
                self.verify(&reference, max_bytes)?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // Never overwrite an existing object, including a corrupt one.
                self.verify(&reference, max_bytes)?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(reference)
    }
}

pub(crate) fn validate_reference(reference: &ArtifactRef) -> Result<(), ArtifactError> {
    if reference.sha256.len() != 64
        || !reference
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || reference.artifact_id != format!("sha256:{}", reference.sha256)
    {
        return Err(ArtifactError::InvalidReference(
            "expected matching lowercase SHA-256 digest and sha256: identifier".into(),
        ));
    }
    validate_media_type(&reference.media_type)
}

pub(crate) fn validate_media_type(media_type: &str) -> Result<(), ArtifactError> {
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&byte))
    };
    if media_type.len() > 255
        || !media_type
            .split_once('/')
            .is_some_and(|(kind, subtype)| token(kind) && token(subtype))
    {
        return Err(ArtifactError::InvalidMediaType(media_type.into()));
    }
    Ok(())
}

fn enforce_limit(bytes: u64, limit: u64) -> Result<(), ArtifactError> {
    if bytes > limit {
        Err(ArtifactError::TooLarge { limit })
    } else {
        Ok(())
    }
}

fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl Write,
    limit: u64,
) -> Result<(u64, String), ArtifactError> {
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or(ArtifactError::TooLarge { limit })?;
        enforce_limit(bytes, limit)?;
        hasher.update(&buffer[..read]);
        writer.write_all(&buffer[..read])?;
    }
    Ok((bytes, format!("{:x}", hasher.finalize())))
}

fn check_contents(reference: &ArtifactRef, bytes: u64, digest: &str) -> Result<(), ArtifactError> {
    if bytes != reference.bytes || digest != reference.sha256 {
        Err(corrupt(
            reference,
            "stored content differs from the reference",
        ))
    } else {
        Ok(())
    }
}

fn corrupt(reference: &ArtifactRef, reason: &str) -> ArtifactError {
    ArtifactError::Corrupt {
        digest: reference.sha256.clone(),
        reason: reason.into(),
    }
}

fn path_error(error: io::Error, context: &str) -> ArtifactError {
    if error.raw_os_error() == Some(libc::ELOOP) {
        ArtifactError::UnsafePath(format!("symlinks are not allowed: {context}"))
    } else {
        ArtifactError::Io(error)
    }
}

// All object operations use a pinned directory descriptor. These small POSIX
// wrappers never follow object symlinks or replace an existing destination.
fn file_at(directory: &File, name: &str, flags: i32, mode: libc::mode_t) -> io::Result<File> {
    let name = CString::new(name)?;
    // SAFETY: the directory and C string remain live during openat; the mode is
    // supplied even when unused. The returned fd is uniquely owned on success.
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags,
            mode as libc::c_uint,
        )
    };
    if descriptor < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: openat returned a new owned descriptor, transferred once.
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

fn directory_at(parent: &File, name: &str) -> Result<File, ArtifactError> {
    let encoded_name = CString::new(name).expect("fixed directory names contain no NUL");
    // SAFETY: descriptor and C string are valid for the duration of mkdirat.
    let created = unsafe { libc::mkdirat(parent.as_raw_fd(), encoded_name.as_ptr(), 0o700) };
    if created < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error.into());
        }
    } else {
        parent.sync_all()?;
    }
    file_at(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    )
    .map_err(|error| path_error(error, name))
}

fn link_at(directory: &File, source: &str, destination: &str) -> io::Result<()> {
    let source = CString::new(source)?;
    let destination = CString::new(destination)?;
    // SAFETY: both C strings and the shared directory descriptor remain valid;
    // flags=0 gives no-clobber hard-link semantics without following symlinks.
    let result = unsafe {
        libc::linkat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            0,
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

struct StagedObject<'a> {
    directory: &'a File,
    name: String,
    file: File,
}

impl<'a> StagedObject<'a> {
    fn new(directory: &'a File) -> Result<Self, ArtifactError> {
        let name = format!(".stage-{}", uuid::Uuid::new_v4());
        let file = file_at(
            directory,
            &name,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )?;
        Ok(Self {
            directory,
            name,
            file,
        })
    }
}

impl Drop for StagedObject<'_> {
    fn drop(&mut self) {
        let name = CString::new(self.name.as_str()).expect("generated names contain no NUL");
        // SAFETY: name and borrowed directory remain live; unlinkat only removes
        // the temporary directory entry, never another hard link to the object.
        unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) };
    }
}

struct LimitedWriter<'a> {
    file: &'a mut File,
    max_bytes: u64,
    position: u64,
}

impl Write for LimitedWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self
            .position
            .checked_add(buffer.len() as u64)
            .is_none_or(|end| end > self.max_bytes)
        {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "bundle byte limit",
            ));
        }
        let written = self.file.write(buffer)?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for LimitedWriter<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.position = self.file.seek(position)?;
        if self.position > self.max_bytes {
            Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "bundle byte limit",
            ))
        } else {
            Ok(self.position)
        }
    }
}

fn normalize_limit_error(error: ArtifactError, limit: u64) -> ArtifactError {
    let exceeded = match &error {
        ArtifactError::Io(error) | ArtifactError::Zip(zip::result::ZipError::Io(error)) => {
            error.kind() == io::ErrorKind::FileTooLarge
        }
        ArtifactError::Json(error) => error.io_error_kind() == Some(io::ErrorKind::FileTooLarge),
        _ => false,
    };
    if exceeded {
        ArtifactError::TooLarge { limit }
    } else {
        error
    }
}

struct CountingWriter {
    bytes: u64,
    limit: u64,
}

impl Write for CountingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let total = self
            .bytes
            .checked_add(buffer.len() as u64)
            .filter(|total| *total <= self.limit)
            .ok_or_else(|| io::Error::new(io::ErrorKind::FileTooLarge, "bundle byte limit"))?;
        self.bytes = total;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
