use apr_runtime::{Runtime, oauth_clients::ClientIdentity};
use serde_json::json;

#[test]
fn public_identity_is_owner_configured_persists_and_never_connects_accounts() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::open(temp.path().join("state")).unwrap();
    assert_eq!(
        runtime.oauth_client_identity().unwrap(),
        ClientIdentity {
            client_id_metadata_url: Some("https://itscookbook.dev/client.json".into()),
            pre_registered_clients: Vec::new(),
        }
    );
    let identity: ClientIdentity = serde_json::from_value(json!({
        "client_id_metadata_url":"https://identity.example.com/client.json",
        "pre_registered_clients":[{"issuer":"https://auth.example.com","client_id":"public-cookbook-client"}]
    })).unwrap();
    runtime.set_oauth_client_identity(&identity).unwrap();
    drop(runtime);
    let runtime = Runtime::open(temp.path().join("state")).unwrap();
    assert_eq!(runtime.oauth_client_identity().unwrap(), identity);
    assert_eq!(runtime.saved_accounts(None).unwrap()["accounts"], json!([]));
    assert_eq!(
        runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
}

#[test]
fn identity_rejects_secrets_ambiguous_issuers_unsafe_urls_and_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let runtime = Runtime::open(&root).unwrap();
    for payload in [
        json!({"client_secret":"never-stored"}),
        json!({"client_id_metadata_url":"http://identity.example.com/client.json"}),
        json!({"client_id_metadata_url":"https://127.0.0.1/client.json"}),
        json!({"client_id_metadata_url":"https://identity.example.com/"}),
        json!({"client_id_metadata_url":"https://identity.example.com/client.json?token=value"}),
        json!({"pre_registered_clients":[{"issuer":"https://auth.example.com","client_id":"a"},{"issuer":"https://auth.example.com","client_id":"b"}]}),
        json!({"pre_registered_clients":[{"issuer":"https://auth.example.com","client_id":"a","client_secret":"hidden"}]}),
    ] {
        if let Ok(identity) = serde_json::from_value::<ClientIdentity>(payload) {
            assert!(runtime.set_oauth_client_identity(&identity).is_err());
        }
    }
    let elsewhere = temp.path().join("untouched.json");
    std::fs::write(&elsewhere, "{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.join("oauth-client-identity.json")).unwrap();
    assert!(runtime.oauth_client_identity().is_err());
    assert!(
        runtime
            .set_oauth_client_identity(&ClientIdentity::default())
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(elsewhere).unwrap(), "{}");
}
