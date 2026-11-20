use apr_connectors::{CredentialStore, PlatformVault, Secret};

#[test]
#[ignore = "requires an unlocked native credential backend; never run in account-free CI"]
fn native_credential_backend_roundtrip_uses_only_a_new_synthetic_entry() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let vault =
        PlatformVault::new(&format!("verification_{}_{}", std::process::id(), stamp)).unwrap();
    struct Cleanup(PlatformVault);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.remove("synthetic");
        }
    }
    let cleanup = Cleanup(vault);
    cleanup
        .0
        .set(
            "synthetic",
            Secret::new(b"synthetic-apr-credential-backend-test".to_vec()).unwrap(),
        )
        .unwrap();
    assert!(cleanup.0.get("synthetic").is_ok());
    cleanup.0.remove("synthetic").unwrap();
    assert!(cleanup.0.get("synthetic").is_err());
}
