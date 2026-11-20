//! Local credential storage and explicit owner migration. Neither operation is
//! available through author MCP. Ordinary dispatch never accesses the OS vault.
use crate::store::StoreLock;
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::{CredentialStore, PlatformVault};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type AccountSnapshot = Vec<(String, u64, String)>;

impl Runtime {
    fn credential_accounts(&self) -> Result<AccountSnapshot> {
        let mut query = self.db.prepare(
            "SELECT id,revision,credential_ref FROM saved_accounts WHERE enabled=1 ORDER BY id LIMIT 1025",
        )?;
        let accounts = query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if accounts.len() > 1024 {
            return Err(RuntimeError::State(
                "credential migration account limit exceeded".into(),
            ));
        }
        Ok(accounts)
    }

    /// Metadata only: no credential contents or automatic Keychain lookup.
    pub fn credential_store_status(&self) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let vault = self.vault()?;
        let accounts = self.credential_accounts()?;
        let mut missing = 0;
        for (_, _, reference) in &accounts {
            if !vault.contains(reference).map_err(store_error)? {
                missing += 1;
            }
        }
        Ok(
            json!({"backend":"local_toml", "path":self.root.join("credentials.toml"),
            "saved_accounts":accounts.len(), "available_accounts":accounts.len()-missing,
            "missing_accounts":missing, "legacy_migration_available":missing>0,
            "encrypted":false, "permissions":"owner_only", "included_in_backups":false}),
        )
    }

    /// Explicit owner action. Old entries are preserved for rollback; credentials
    /// already present locally are never overwritten. This is the only runtime
    /// path that reads the legacy platform credential store.
    pub fn migrate_platform_credentials(&self) -> Result<Value> {
        let namespace = format!(
            "state-{:x}",
            Sha256::digest(self.root.as_os_str().as_encoded_bytes())
        );
        let legacy = PlatformVault::new(&namespace).map_err(store_error)?;
        self.import_legacy_credentials(&legacy)
    }

    fn import_legacy_credentials(&self, legacy: &dyn CredentialStore) -> Result<Value> {
        let vault = self.vault()?;
        let snapshot = {
            let _lock = StoreLock::acquire(&self.root)?;
            self.credential_accounts()?
        };
        let mut entries = Vec::new();
        for (_, _, reference) in &snapshot {
            if !vault.contains(reference).map_err(store_error)? {
                // Never hold the runtime lock while the OS waits for consent.
                // Fail atomically if any requested legacy credential is unavailable.
                let secret = legacy.get(reference).map_err(|_| RuntimeError::State(
                    "legacy credential unavailable; complete operating-system consent or reconnect the account; no credentials were imported".into()
                ))?;
                entries.push((reference.clone(), secret));
            }
        }
        let imported = entries.len();
        {
            let _lock = StoreLock::acquire(&self.root)?;
            if self.credential_accounts()? != snapshot {
                return Err(RuntimeError::State(
                    "saved accounts changed during migration; review and retry".into(),
                ));
            }
            if !entries.is_empty() {
                vault.import_preserving(entries).map_err(store_error)?;
            }
        }
        let mut result = self.credential_store_status()?;
        result["imported"] = json!(imported);
        result["legacy_entries_preserved"] = json!(true);
        Ok(result)
    }
}

fn store_error(_: apr_connectors::DispatchError) -> RuntimeError {
    RuntimeError::State(
        "private credential store unavailable; check its ownership, permissions and format".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use apr_connectors::{DispatchError, Secret};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Legacy {
        calls: AtomicUsize,
        fail: bool,
    }
    impl CredentialStore for Legacy {
        fn get(&self, _: &str) -> std::result::Result<Secret, DispatchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(DispatchError::CredentialUnavailable);
            }
            Secret::new(b"synthetic-migration-only".to_vec())
        }
    }
    fn account(runtime: &Runtime, id: &str, reference: &str, enabled: bool) {
        runtime.db.execute("INSERT INTO saved_accounts(id,revision,enabled,label,credential_ref,contract,created_at) VALUES(?1,3,?2,'synthetic',?3,'{}',0)", rusqlite::params![id,enabled,reference]).unwrap();
    }
    #[test]
    fn migration_is_explicit_idempotent_and_preserves_account_revisions() {
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(root.path()).unwrap();
        account(&runtime, "active", "credential_active", true);
        account(&runtime, "revoked", "credential_revoked", false);
        let legacy = Legacy {
            calls: AtomicUsize::new(0),
            fail: false,
        };
        assert_eq!(
            runtime.credential_store_status().unwrap()["missing_accounts"],
            1
        );
        assert_eq!(legacy.calls.load(Ordering::SeqCst), 0);
        let before = runtime.credential_accounts().unwrap();
        assert_eq!(
            runtime.import_legacy_credentials(&legacy).unwrap()["imported"],
            1
        );
        assert_eq!(
            runtime.import_legacy_credentials(&legacy).unwrap()["imported"],
            0
        );
        assert_eq!(legacy.calls.load(Ordering::SeqCst), 1);
        assert_eq!(before, runtime.credential_accounts().unwrap());
        assert!(
            !runtime
                .vault()
                .unwrap()
                .contains("credential_revoked")
                .unwrap()
        );
        drop(runtime);
        assert_eq!(
            Runtime::open(root.path())
                .unwrap()
                .credential_store_status()
                .unwrap()["available_accounts"],
            1
        );
    }
    #[test]
    fn account_revocation_during_os_consent_aborts_import_without_deadlocking() {
        struct RevokingLegacy(std::path::PathBuf);
        impl CredentialStore for RevokingLegacy {
            fn get(&self, _: &str) -> std::result::Result<Secret, DispatchError> {
                // A separate owner can revoke while the old OS store awaits consent.
                let mut other = Runtime::open(&self.0).unwrap();
                other.revoke_saved_account("active", 3).unwrap();
                Secret::new(b"synthetic-revoked".to_vec())
            }
        }
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(root.path()).unwrap();
        account(&runtime, "active", "credential_active", true);
        let legacy = RevokingLegacy(root.path().to_owned());
        assert!(runtime.import_legacy_credentials(&legacy).is_err());
        assert!(
            !runtime
                .vault()
                .unwrap()
                .contains("credential_active")
                .unwrap()
        );
        assert_eq!(
            runtime.credential_store_status().unwrap()["saved_accounts"],
            0
        );
    }

    #[test]
    fn revocation_reports_failed_cleanup_without_restoring_authority() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::open(root.path()).unwrap();
        account(&runtime, "active", "credential_active", true);
        runtime
            .vault()
            .unwrap()
            .set(
                "credential_active",
                Secret::new(b"synthetic-only".to_vec()).unwrap(),
            )
            .unwrap();
        std::fs::set_permissions(
            root.path().join("credentials.toml"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let receipt = runtime.revoke_saved_account("active", 3).unwrap();
        assert_eq!(receipt["enabled"], false);
        assert_eq!(receipt["credential_removed"], false);
        assert!(runtime.credential_accounts().unwrap().is_empty());
    }

    #[test]
    fn failed_migration_keeps_existing_credentials() {
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(root.path()).unwrap();
        account(&runtime, "active", "credential_active", true);
        runtime
            .vault()
            .unwrap()
            .set(
                "credential_existing",
                Secret::new(b"existing-synthetic".to_vec()).unwrap(),
            )
            .unwrap();
        let before = std::fs::read(root.path().join("credentials.toml")).unwrap();
        let legacy = Legacy {
            calls: AtomicUsize::new(0),
            fail: true,
        };
        assert!(runtime.import_legacy_credentials(&legacy).is_err());
        assert_eq!(
            before,
            std::fs::read(root.path().join("credentials.toml")).unwrap()
        );
    }
}
