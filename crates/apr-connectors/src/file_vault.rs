//! Private, local TOML credential store. This module never consults Keychain.

use crate::{CredentialStore, DispatchError, Secret, identifier};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

const MAX_ENTRIES: usize = 256;
const MAX_FILE_BYTES: u64 = 3 * 1024 * 1024;
const FILE_NAME: &str = "credentials.toml";
const LOCK_NAME: &str = "credentials.toml.lock";

fn unavailable() -> DispatchError {
    DispatchError::CredentialUnavailable
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDocument {
    version: u32,
    credentials: BTreeMap<String, String>,
}

impl FileDocument {
    fn empty() -> Self {
        Self {
            version: 1,
            credentials: BTreeMap::new(),
        }
    }

    fn validate(&self) -> Result<(), DispatchError> {
        if self.version != 1 || self.credentials.len() > MAX_ENTRIES {
            return Err(unavailable());
        }
        for (reference, value) in &self.credentials {
            if !identifier(reference) || !valid_secret(value.as_bytes()) {
                return Err(unavailable());
            }
        }
        Ok(())
    }
}

impl Drop for FileDocument {
    fn drop(&mut self) {
        for value in self.credentials.values_mut() {
            value.zeroize();
        }
    }
}

fn valid_secret(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.len() <= 8192
        && bytes.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

fn check_reference(reference: &str) -> Result<(), DispatchError> {
    if identifier(reference) {
        Ok(())
    } else {
        Err(DispatchError::InvalidConnection)
    }
}

fn owner_uid() -> u32 {
    // SAFETY: geteuid has no arguments, does not retain a pointer, and is thread-safe.
    unsafe { libc::geteuid() }
}

fn check_directory(path: &Path) -> Result<(), DispatchError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != owner_uid()
        || metadata.permissions().mode() & 0o7777 != 0o700
    {
        return Err(unavailable());
    }
    Ok(())
}

fn check_file(file: &File, path: &Path) -> Result<(), DispatchError> {
    let opened = file.metadata().map_err(|_| unavailable())?;
    let named = fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if !opened.file_type().is_file()
        || !named.file_type().is_file()
        || opened.uid() != owner_uid()
        || named.uid() != owner_uid()
        || opened.permissions().mode() & 0o7777 != 0o600
        || named.permissions().mode() & 0o7777 != 0o600
        || opened.nlink() != 1
        || named.nlink() != 1
        || opened.dev() != named.dev()
        || opened.ino() != named.ino()
    {
        return Err(unavailable());
    }
    Ok(())
}

fn open_existing(path: &Path) -> Result<Option<File>, DispatchError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unavailable()),
        Ok(metadata) if !metadata.file_type().is_file() => return Err(unavailable()),
        Ok(_) => {}
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| unavailable())?;
    check_file(&file, path)?;
    Ok(Some(file))
}

/// Stores protected account values in `root/credentials.toml`.
///
/// The caller chooses the root; existing insecure directories and files are
/// refused rather than silently changing their permissions.
pub struct FileVault {
    root: PathBuf,
}

impl FileVault {
    pub fn new(root: &Path) -> Result<Self, DispatchError> {
        if !root.is_absolute() {
            return Err(DispatchError::InvalidConnection);
        }
        match fs::symlink_metadata(root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DirBuilder::new()
                    .mode(0o700)
                    .create(root)
                    .map_err(|_| unavailable())?;
            }
            Err(_) => return Err(unavailable()),
            Ok(_) => {}
        }
        check_directory(root)?;
        let vault = Self {
            root: root.to_path_buf(),
        };
        // Reject an existing insecure or corrupt file at construction time.
        let _guard = vault.lock(false)?;
        vault.read_document()?.validate()?;
        Ok(vault)
    }

    fn path(&self) -> PathBuf {
        self.root.join(FILE_NAME)
    }

    fn lock(&self, exclusive: bool) -> Result<File, DispatchError> {
        check_directory(&self.root)?;
        let path = self.root.join(LOCK_NAME);
        let file = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                    .open(&path)
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        self.open_lock(&path)?
                    }
                    Err(_) => return Err(unavailable()),
                }
            }
            Err(_) => return Err(unavailable()),
            Ok(_) => self.open_lock(&path)?,
        };
        check_file(&file, &path)?;
        if exclusive {
            FileExt::lock_exclusive(&file).map_err(|_| unavailable())?;
        } else {
            FileExt::lock_shared(&file).map_err(|_| unavailable())?;
        }
        check_directory(&self.root)?;
        check_file(&file, &path)?;
        Ok(file)
    }

    fn open_lock(&self, path: &Path) -> Result<File, DispatchError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| unavailable())?;
        check_file(&file, path)?;
        Ok(file)
    }

    fn read_document(&self) -> Result<FileDocument, DispatchError> {
        let path = self.path();
        let Some(mut file) = open_existing(&path)? else {
            return Ok(FileDocument::empty());
        };
        if file.metadata().map_err(|_| unavailable())?.len() > MAX_FILE_BYTES {
            return Err(unavailable());
        }
        let mut raw = Zeroizing::new(Vec::new());
        Read::by_ref(&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|_| unavailable())?;
        if raw.len() as u64 > MAX_FILE_BYTES {
            return Err(unavailable());
        }
        let source = std::str::from_utf8(&raw).map_err(|_| unavailable())?;
        let parsed: FileDocument = toml_edit::de::from_str(source).map_err(|_| unavailable())?;
        parsed.validate()?;
        Ok(parsed)
    }

    fn publish(&self, document: &FileDocument) -> Result<(), DispatchError> {
        document.validate()?;
        check_directory(&self.root)?;
        // A parsed document owns every value, and both raw read and serialized
        // buffers are zeroized when they leave scope.
        let rendered =
            Zeroizing::new(toml_edit::ser::to_string(document).map_err(|_| unavailable())?);
        if rendered.len() as u64 > MAX_FILE_BYTES {
            return Err(unavailable());
        }
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|_| unavailable())?;
        temporary
            .write_all(rendered.as_bytes())
            .map_err(|_| unavailable())?;
        temporary.as_file().sync_all().map_err(|_| unavailable())?;
        if temporary
            .as_file()
            .metadata()
            .map_err(|_| unavailable())?
            .permissions()
            .mode()
            & 0o7777
            != 0o600
        {
            return Err(unavailable());
        }
        // Refuse a pre-existing target with unsafe metadata before replacement.
        let _ = open_existing(&self.path())?;
        temporary.persist(self.path()).map_err(|_| unavailable())?;
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| unavailable())?;
        Ok(())
    }

    pub fn set(&self, reference: &str, secret: Secret) -> Result<(), DispatchError> {
        self.replace(reference, secret)
    }

    /// Checks only presence, without returning or duplicating a secret value.
    pub fn contains(&self, reference: &str) -> Result<bool, DispatchError> {
        check_reference(reference)?;
        let _guard = self.lock(false)?;
        Ok(self.read_document()?.credentials.contains_key(reference))
    }

    pub fn remove(&self, reference: &str) -> Result<(), DispatchError> {
        check_reference(reference)?;
        let _guard = self.lock(true)?;
        let mut document = self.read_document()?;
        if let Some(mut removed) = document.credentials.remove(reference) {
            removed.zeroize();
            self.publish(&document)?;
        }
        Ok(())
    }

    /// Adds absent references in one atomic publish; existing values are kept.
    /// Duplicate or invalid input references reject the complete batch.
    pub fn import_preserving(&self, entries: Vec<(String, Secret)>) -> Result<(), DispatchError> {
        let mut seen = BTreeSet::new();
        for (reference, _) in &entries {
            check_reference(reference)?;
            if !seen.insert(reference) {
                return Err(DispatchError::InvalidConnection);
            }
        }
        let _guard = self.lock(true)?;
        let mut document = self.read_document()?;
        let mut changed = false;
        for (reference, secret) in entries {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                document.credentials.entry(reference)
            {
                slot.insert(secret.text().to_owned());
                changed = true;
            }
        }
        if changed {
            self.publish(&document)?;
        }
        Ok(())
    }
}

impl CredentialStore for FileVault {
    fn lock_refresh(
        &self,
        reference: &str,
    ) -> Result<Box<dyn crate::CredentialRefreshGuard + '_>, DispatchError> {
        check_reference(reference)?;
        check_directory(&self.root)?;
        // A separate permanent inode per opaque reference avoids holding the
        // credential-file lock over network I/O or serializing other accounts.
        let path = self.root.join(format!(
            "oauth-refresh-{:x}.lock",
            Sha256::digest(reference.as_bytes())
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|_| unavailable())?;
        check_file(&file, &path)?;
        FileExt::lock_exclusive(&file).map_err(|_| unavailable())?;
        check_directory(&self.root)?;
        check_file(&file, &path)?;
        Ok(Box::new(file))
    }

    fn replace_if_matches(
        &self,
        reference: &str,
        expected: &Secret,
        next: Secret,
    ) -> Result<(), DispatchError> {
        check_reference(reference)?;
        let _guard = self.lock(true)?;
        let mut document = self.read_document()?;
        let value = document
            .credentials
            .get_mut(reference)
            .ok_or_else(unavailable)?;
        if value.as_str() != expected.text() {
            return Err(unavailable());
        }
        value.zeroize();
        *value = next.text().to_owned();
        self.publish(&document)
    }

    fn get(&self, reference: &str) -> Result<Secret, DispatchError> {
        check_reference(reference)?;
        let _guard = self.lock(false)?;
        let document = self.read_document()?;
        let value = document
            .credentials
            .get(reference)
            .ok_or_else(unavailable)?;
        Secret::new(value.as_bytes().to_vec())
    }

    fn replace(&self, reference: &str, secret: Secret) -> Result<(), DispatchError> {
        check_reference(reference)?;
        let _guard = self.lock(true)?;
        let mut document = self.read_document()?;
        if let Some(mut replaced) = document
            .credentials
            .insert(reference.to_owned(), secret.text().to_owned())
        {
            replaced.zeroize();
        }
        self.publish(&document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn secret(value: &str) -> Secret {
        Secret::new(value.as_bytes().to_vec()).unwrap()
    }

    fn fixture() -> (tempfile::TempDir, FileVault) {
        let temp = tempfile::tempdir().unwrap();
        let vault = FileVault::new(&temp.path().join("vault")).unwrap();
        (temp, vault)
    }

    #[test]
    fn conditional_rotation_preserves_owner_replacement_and_removal() {
        let (_temp, vault) = fixture();
        vault.set("oauth:test", secret("first")).unwrap();
        let expected = vault.get("oauth:test").unwrap();
        vault
            .set("oauth:test", secret("owner-replacement"))
            .unwrap();
        assert!(
            vault
                .replace_if_matches("oauth:test", &expected, secret("stale-refresh"))
                .is_err()
        );
        assert_eq!(vault.get("oauth:test").unwrap().text(), "owner-replacement");
        let current = vault.get("oauth:test").unwrap();
        vault
            .replace_if_matches("oauth:test", &current, secret("rotated"))
            .unwrap();
        assert_eq!(vault.get("oauth:test").unwrap().text(), "rotated");
        vault.remove("oauth:test").unwrap();
        assert!(
            vault
                .replace_if_matches("oauth:test", &current, secret("resurrected"))
                .is_err()
        );
        assert!(!vault.contains("oauth:test").unwrap());
    }

    #[test]
    fn persistence_rotation_deletion_and_private_modes() {
        let (temp, vault) = fixture();
        assert!(!vault.contains("service:one").unwrap());
        vault.set("service:one", secret("first-value")).unwrap();
        assert_eq!(vault.get("service:one").unwrap().text(), "first-value");
        vault
            .replace("service:one", secret("second-value"))
            .unwrap();
        let reopened = FileVault::new(&temp.path().join("vault")).unwrap();
        assert_eq!(reopened.get("service:one").unwrap().text(), "second-value");
        assert!(
            !fs::read_to_string(reopened.path())
                .unwrap()
                .contains("first-value")
        );
        assert_eq!(
            fs::metadata(temp.path().join("vault"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
        assert_eq!(
            fs::metadata(reopened.path()).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        assert_eq!(
            fs::metadata(temp.path().join("vault").join(LOCK_NAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
        reopened.remove("service:one").unwrap();
        reopened.remove("service:one").unwrap();
        assert!(!vault.contains("service:one").unwrap());
        assert!(vault.get("service:one").is_err());
    }

    #[test]
    fn import_preserves_existing_and_rejects_invalid_batch_without_publish() {
        let (_temp, vault) = fixture();
        vault.set("old", secret("keep-me")).unwrap();
        vault
            .import_preserving(vec![
                ("old".into(), secret("do-not-replace")),
                ("new".into(), secret("new-value")),
            ])
            .unwrap();
        assert_eq!(vault.get("old").unwrap().text(), "keep-me");
        assert_eq!(vault.get("new").unwrap().text(), "new-value");
        let before = fs::read(vault.path()).unwrap();
        assert!(
            vault
                .import_preserving(vec![
                    ("third".into(), secret("third-value")),
                    ("third".into(), secret("another-value"))
                ])
                .is_err()
        );
        assert_eq!(fs::read(vault.path()).unwrap(), before);
        assert!(!vault.contains("third").unwrap());
    }

    #[test]
    fn concurrent_independent_handles_do_not_lose_updates() {
        let (temp, _vault) = fixture();
        let root = temp.path().join("vault");
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let root = root.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let vault = FileVault::new(&root).unwrap();
                    barrier.wait();
                    for j in 0..8 {
                        vault
                            .set(&format!("thread-{i}-item-{j}"), secret("value"))
                            .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let vault = FileVault::new(&root).unwrap();
        for i in 0..8 {
            for j in 0..8 {
                assert!(vault.contains(&format!("thread-{i}-item-{j}")).unwrap());
            }
        }
    }

    #[test]
    fn invalid_corrupt_and_unsafe_files_are_refused() {
        let (temp, vault) = fixture();
        assert!(vault.set("../escape", secret("hidden-value")).is_err());
        assert!(vault.contains("bad/ref").is_err());
        let path = vault.path();
        fs::write(&path, "version = 2\n[credentials]\nitem = 'hidden-value'\n").unwrap();
        assert!(FileVault::new(&temp.path().join("vault")).is_err());
        assert!(vault.get("item").is_err());
        fs::write(&path, "not valid = [\n").unwrap();
        assert!(vault.get("item").is_err());
        fs::write(&path, "version = 1\n[credentials]\nitem = 'hidden-value'\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(vault.get("item").is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = temp.path().join("hardlink");
        fs::hard_link(&path, &link).unwrap();
        assert!(vault.get("item").is_err());
        fs::remove_file(link).unwrap();
        fs::remove_file(&path).unwrap();
        let target = temp.path().join("target");
        fs::write(&target, "version = 1\n[credentials]\n").unwrap();
        symlink(&target, &path).unwrap();
        assert!(vault.get("item").is_err());
        fs::remove_file(&path).unwrap();
        let _socket = UnixListener::bind(&path).unwrap();
        assert!(vault.get("item").is_err());
        fs::remove_file(&path).unwrap();
        let lock = temp.path().join("vault").join(LOCK_NAME);
        fs::remove_file(&lock).unwrap();
        symlink(&target, &lock).unwrap();
        assert!(vault.contains("item").is_err());
        fs::remove_file(&lock).unwrap();
        let root = temp.path().join("vault");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(vault.get("item").is_err());
    }

    #[test]
    fn failures_never_render_secret_values() {
        let (_temp, vault) = fixture();
        let path = vault.path();
        fs::write(
            &path,
            "version = 1\n[credentials]\nitem = 'very-private-token'\nunknown = [\n",
        )
        .unwrap();
        let error = vault.get("item").err().unwrap();
        assert!(!error.to_string().contains("very-private-token"));
        assert!(!format!("{error:?}").contains("very-private-token"));
    }
}
