use apr_connectors::{Auth, Connection, Secret};
use apr_runtime::{
    Runtime,
    connection_library::{ApprovalOptions, ReviewedAccountReuse, SavedAccount, SavedCredential},
    setup::ConnectionProposal,
};
use serde_json::{Value, json};

fn base64url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::new();
    for chunk in input.chunks(3) {
        let bits = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | (chunk.get(2).copied().unwrap_or(0) as u32);
        encoded.push(ALPHABET[((bits >> 18) & 63) as usize] as char);
        encoded.push(ALPHABET[((bits >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(ALPHABET[((bits >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            encoded.push(ALPHABET[(bits & 63) as usize] as char);
        }
    }
    encoded
}

struct Fixture {
    temp: tempfile::TempDir,
    runtime: Runtime,
    a: String,
    b: String,
    token: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::open(temp.path().join("state")).unwrap();
        let mut assignments = Vec::new();
        for name in ["one", "two"] {
            let root = temp.path().join(name);
            std::fs::create_dir(&root).unwrap();
            assignments.push(
                runtime.owner_project_setup(&root, name).unwrap()["assignment_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        let a = assignments[0].clone();
        let b = assignments[1].clone();
        let token = runtime.create_attachment(&b, "author").unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            temp,
            runtime,
            a,
            b,
            token,
        }
    }
    fn proposal(&mut self, name: &str, auth: Auth) -> Value {
        self.runtime
            .propose_connection(
                &self.a,
                ConnectionProposal {
                    name: name.into(),
                    request_key: name.into(),
                    rationale: "Synthetic reusable service, no external dispatch".into(),
                    connection: Connection {
                        label: name.into(),
                        credential_label: None,
                        endpoint: "https://example.com/service".into(),
                        method: "POST".into(),
                        auth,
                        input_schema: json!({"type":"object"}),
                        output_schema: json!({"type":"object"}),
                        timeout_ms: 1000,
                        max_response_bytes: 4096,
                        mcp: None,
                        metering: None,
                        loopback_test: false,
                    },
                },
            )
            .unwrap()
    }
    fn approve(&mut self, p: &Value, options: ApprovalOptions) -> Value {
        self.runtime
            .approve_connection_with_options(
                p["request_id"].as_str().unwrap(),
                p["digest"].as_str().unwrap(),
                None,
                options,
            )
            .unwrap()
    }
    fn use_tool(&mut self, library: &Value, name: &str) -> apr_runtime::Result<Value> {
        self.runtime.scoped_call(&self.token,"manage",json!({"action":"connection.use","library_id":library["library_id"],"revision":library["revision"],"name":name}))
    }
}

#[test]
fn legacy_approval_needs_owner_publication_and_reuse_cannot_widen_authority() {
    let mut f = Fixture::new();
    let p = f.proposal("read", Auth::None);
    f.approve(&p, Default::default());
    assert!(
        f.runtime.connection_library(None, false).unwrap()["connections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        f.runtime
            .save_connection_to_library(&f.a, "read", 99)
            .is_err()
    );
    let library = f
        .runtime
        .save_connection_to_library(&f.a, "read", 1)
        .unwrap();
    assert_eq!(
        f.runtime
            .save_connection_to_library(&f.a, "read", 1)
            .unwrap(),
        library
    );
    let contract = f
        .runtime
        .scoped_call(
            &f.token,
            "inspect",
            json!({"view":"library_contract","library_id":library["library_id"],"revision":1}),
        )
        .unwrap();
    assert!(contract.get("auth").is_none());
    assert!(contract.get("endpoint").is_none());
    assert_eq!(f.use_tool(&library, "shared").unwrap()["reused"], false);
    assert_eq!(f.use_tool(&library, "shared").unwrap()["reused"], true);
    // Re-publishing a derived grant retains identity, so disable/deny cannot be bypassed.
    assert_eq!(
        f.runtime
            .save_connection_to_library(&f.b, "shared", 1)
            .unwrap(),
        library
    );
    let operator = f.runtime.create_attachment(&f.b, "run").unwrap();
    assert!(f.runtime.scoped_call(operator["token"].as_str().unwrap(),"manage",json!({"action":"connection.use","library_id":library["library_id"],"revision":1,"name":"operator"})).is_err());
    assert!(f.runtime.scoped_call(&f.token,"manage",json!({"action":"connection.use","library_id":library["library_id"],"revision":1,"name":"other","endpoint":"https://evil.invalid"})).is_err());
    assert!(
        f.use_tool(
            &json!({"library_id":library["library_id"],"revision":9}),
            "stale"
        )
        .is_err()
    );
    f.runtime
        .revoke_execution(&f.b, "connection", "shared")
        .unwrap();
    assert!(f.use_tool(&library, "new-alias").is_err());
    f.runtime
        .disable_library_connection(library["library_id"].as_str().unwrap(), 1)
        .unwrap();
    assert!(
        f.runtime.connection_library(None, false).unwrap()["connections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"],
        json!([])
    );
}

#[test]
fn global_approval_replays_without_regrant_and_restore_disables_library() {
    let mut f = Fixture::new();
    let p = f.proposal("read", Auth::None);
    let receipt = f.approve(
        &p,
        ApprovalOptions {
            saved_account: None,
            all_projects: true,
            saved_credential: None,
            reviewed_account_reuse: None,
        },
    );
    let replay = f.approve(
        &p,
        ApprovalOptions {
            saved_account: None,
            all_projects: true,
            saved_credential: None,
            reviewed_account_reuse: None,
        },
    );
    assert_eq!(receipt, replay);
    let library = &receipt["library"];
    f.use_tool(library, "read").unwrap();
    let backup = f.temp.path().join("backup");
    f.runtime.backup_to(&backup).unwrap();
    let restored_path = f.temp.path().join("restored");
    Runtime::restore_from(&backup, &restored_path).unwrap();
    let restored = Runtime::open(&restored_path).unwrap();
    assert_eq!(
        restored.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
    assert_eq!(
        restored.connection_library(None, true).unwrap()["connections"][0]["enabled"],
        false
    );
    assert_eq!(
        restored.saved_accounts(None).unwrap()["accounts"],
        json!([])
    );
    f.runtime
        .disable_library_connection(library["library_id"].as_str().unwrap(), 1)
        .unwrap();
    assert!(
        f.runtime
            .approve_connection_with_options(
                p["request_id"].as_str().unwrap(),
                p["digest"].as_str().unwrap(),
                None,
                ApprovalOptions {
                    saved_account: None,
                    all_projects: true,
                    saved_credential: None,
                    reviewed_account_reuse: None,
                }
            )
            .is_err()
    );
}

#[test]
fn saved_key_is_reused_only_for_the_same_reviewed_destination_and_auth() {
    // Synthetic in-memory backend is confined to this test process. No OS keychain access.
    let mut f = Fixture::new();
    let p = f.proposal(
        "first",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    let first = f
        .runtime
        .approve_connection_with_options(
            p["request_id"].as_str().unwrap(),
            p["digest"].as_str().unwrap(),
            Some(Secret::new(b"synthetic-not-an-account-key".to_vec()).unwrap()),
            ApprovalOptions {
                saved_account: None,
                all_projects: true,
                saved_credential: None,
                reviewed_account_reuse: None,
            },
        )
        .unwrap();
    let id = first["library"]["library_id"].as_str().unwrap().to_owned();
    let p = f.proposal(
        "another-operation",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    f.approve(
        &p,
        ApprovalOptions {
            saved_account: None,
            all_projects: true,
            reviewed_account_reuse: None,
            saved_credential: Some(SavedCredential {
                library_id: id.clone(),
                revision: 1,
            }),
        },
    );
    let db = rusqlite::Connection::open(f.temp.path().join("state/state.sqlite3")).unwrap();
    let refs:Vec<String>=db.prepare("SELECT json_extract(payload,'$.auth.credential_ref') FROM execution_grants WHERE kind='connection' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0], refs[1]);
    let serialized =
        serde_json::to_string(&f.runtime.connection_library(None, false).unwrap()).unwrap();
    assert!(!serialized.contains(&refs[0]));
    assert!(!serialized.contains("synthetic-not-an-account-key"));
    let wrong = f.proposal(
        "wrong-auth",
        Auth::Header {
            credential_ref: "pending".into(),
            name: "x-api-key".into(),
        },
    );
    assert!(
        f.runtime
            .approve_connection_with_options(
                wrong["request_id"].as_str().unwrap(),
                wrong["digest"].as_str().unwrap(),
                None,
                ApprovalOptions {
                    saved_account: None,
                    all_projects: true,
                    reviewed_account_reuse: None,
                    saved_credential: Some(SavedCredential {
                        library_id: id.clone(),
                        revision: 1
                    })
                }
            )
            .is_err()
    );
    let c:Connection=serde_json::from_value(json!({"label":"other-host","endpoint":"https://other.example/service","method":"POST","auth":{"kind":"bearer","credential_ref":"pending"},"input_schema":{"type":"object"},"output_schema":{"type":"object"},"timeout_ms":1000,"max_response_bytes":4096})).unwrap();
    let wrong = f
        .runtime
        .propose_connection(
            &f.a,
            ConnectionProposal {
                name: "wrong-host".into(),
                request_key: "wrong-host".into(),
                rationale: "Different service needs its own key".into(),
                connection: c,
            },
        )
        .unwrap();
    assert!(
        f.runtime
            .approve_connection_with_options(
                wrong["request_id"].as_str().unwrap(),
                wrong["digest"].as_str().unwrap(),
                None,
                ApprovalOptions {
                    saved_account: None,
                    all_projects: true,
                    reviewed_account_reuse: None,
                    saved_credential: Some(SavedCredential {
                        library_id: id,
                        revision: 1
                    })
                }
            )
            .is_err()
    );
    let revoked = f
        .runtime
        .revoke_saved_account(first["account"]["account_id"].as_str().unwrap(), 1)
        .unwrap();
    assert_eq!(revoked["disabled_tools"], 2);
    assert_eq!(
        f.runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
}

#[test]
fn workspace_only_account_can_approve_another_operation_without_sharing_a_tool() {
    let mut f = Fixture::new();
    let first = f.proposal(
        "workspace-read",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    let receipt = f
        .runtime
        .approve_connection_with_options(
            first["request_id"].as_str().unwrap(),
            first["digest"].as_str().unwrap(),
            Some(Secret::new(b"synthetic-account-only".to_vec()).unwrap()),
            Default::default(),
        )
        .unwrap();
    assert!(receipt["library"].is_null());
    assert_eq!(
        f.runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
    let account = receipt["account"].clone();
    assert_eq!(
        f.runtime.saved_accounts(None).unwrap()["accounts"][0]["account_id"],
        account["account_id"]
    );
    let second = f.proposal(
        "workspace-write",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    let second_receipt = f
        .runtime
        .approve_connection_with_options(
            second["request_id"].as_str().unwrap(),
            second["digest"].as_str().unwrap(),
            None,
            ApprovalOptions {
                all_projects: false,
                saved_credential: None,
                reviewed_account_reuse: None,
                saved_account: Some(SavedAccount {
                    account_id: account["account_id"].as_str().unwrap().into(),
                    revision: 1,
                }),
            },
        )
        .unwrap();
    assert_eq!(second_receipt["account"], account);
    assert_eq!(
        f.runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
    let wrong = f.proposal(
        "wrong-auth",
        Auth::Header {
            credential_ref: "pending".into(),
            name: "x-api-key".into(),
        },
    );
    assert!(
        f.runtime
            .approve_connection_with_options(
                wrong["request_id"].as_str().unwrap(),
                wrong["digest"].as_str().unwrap(),
                None,
                ApprovalOptions {
                    all_projects: false,
                    saved_credential: None,
                    reviewed_account_reuse: None,
                    saved_account: Some(SavedAccount {
                        account_id: account["account_id"].as_str().unwrap().into(),
                        revision: 1,
                    })
                }
            )
            .is_err()
    );
    let backup = f.temp.path().join("account-backup");
    f.runtime.backup_to(&backup).unwrap();
    let restored_path = f.temp.path().join("account-restored");
    Runtime::restore_from(&backup, &restored_path).unwrap();
    assert!(!backup.join("credentials.toml").exists());
    assert!(!restored_path.join("credentials.toml").exists());
    let restored = Runtime::open(&restored_path).unwrap();
    assert_eq!(
        restored.saved_accounts(None).unwrap()["accounts"][0]["enabled"],
        false
    );
    let blocked = f
        .runtime
        .revoke_saved_account(account["account_id"].as_str().unwrap(), 1)
        .unwrap();
    assert_eq!(blocked["disabled_bindings"], 2);
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"],
        json!([])
    );
    assert!(
        f.runtime
            .revoke_saved_account(account["account_id"].as_str().unwrap(), 1)
            .is_err()
    );
}

#[test]
fn disabling_shared_tool_preserves_separate_saved_account() {
    let mut f = Fixture::new();
    let p = f.proposal(
        "shared",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    let receipt = f
        .runtime
        .approve_connection_with_options(
            p["request_id"].as_str().unwrap(),
            p["digest"].as_str().unwrap(),
            Some(Secret::new(b"synthetic-shared-account".to_vec()).unwrap()),
            ApprovalOptions {
                all_projects: true,
                saved_credential: None,
                reviewed_account_reuse: None,
                saved_account: None,
            },
        )
        .unwrap();
    f.runtime
        .disable_library_connection(receipt["library"]["library_id"].as_str().unwrap(), 1)
        .unwrap();
    assert_eq!(
        f.runtime.saved_accounts(None).unwrap()["accounts"][0]["enabled"],
        true
    );
    let p = f.proposal(
        "another",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    f.runtime
        .approve_connection_with_options(
            p["request_id"].as_str().unwrap(),
            p["digest"].as_str().unwrap(),
            None,
            ApprovalOptions {
                all_projects: false,
                saved_credential: None,
                reviewed_account_reuse: None,
                saved_account: Some(SavedAccount {
                    account_id: receipt["account"]["account_id"].as_str().unwrap().into(),
                    revision: 1,
                }),
            },
        )
        .unwrap();
}

#[test]
fn migration_backfills_only_active_legacy_accounts_without_publishing_tools() {
    let mut f = Fixture::new();
    let p = f.proposal(
        "active",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    f.runtime
        .approve_connection(
            p["request_id"].as_str().unwrap(),
            p["digest"].as_str().unwrap(),
            Some(Secret::new(b"synthetic-active".to_vec()).unwrap()),
        )
        .unwrap();
    let p = f.proposal(
        "revoked",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    f.runtime
        .approve_connection(
            p["request_id"].as_str().unwrap(),
            p["digest"].as_str().unwrap(),
            Some(Secret::new(b"synthetic-revoked".to_vec()).unwrap()),
        )
        .unwrap();
    f.runtime
        .revoke_execution(&f.a, "connection", "revoked")
        .unwrap();
    let path = f.temp.path().join("state/state.sqlite3");
    drop(f.runtime);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "DROP TABLE worker_policy; DROP TABLE saved_accounts; PRAGMA user_version=16;",
    )
    .unwrap();
    drop(db);
    let runtime = Runtime::open(f.temp.path().join("state")).unwrap();
    let accounts = runtime.saved_accounts(None).unwrap();
    assert_eq!(accounts["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(
        runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
}

#[test]
fn oauth_account_reuse_requires_the_exact_authorization_contract() {
    let mut f = Fixture::new();
    let cfg = json!({"issuer":"https://auth.example.com","authorization_endpoint":"https://auth.example.com/authorize",
        "token_endpoint":"https://auth.example.com/token","client_id":"synthetic-client",
        "resource":"https://example.com/service","scopes":["read"],"require_issuer_response":false});
    let connection:Connection=serde_json::from_value(json!({"label":"oauth account","endpoint":"https://example.com/service",
        "method":"POST","auth":{"kind":"oauth","credential_ref":"pending","config":cfg},
        "input_schema":{"type":"object"},"output_schema":{"type":"object"},"timeout_ms":1000,"max_response_bytes":4096})).unwrap();
    // Trusted direct setup stores a synthetic blob; this exercises compatibility,
    // not token exchange or provider acceptance.
    f.runtime
        .configure_connection(
            &f.a,
            "oauth-base",
            connection,
            Some(Secret::new(b"synthetic-oauth".to_vec()).unwrap()),
        )
        .unwrap();
    let account = f.runtime.saved_accounts(None).unwrap()["accounts"][0].clone();
    let mut changed = cfg.clone();
    changed["scopes"] = json!(["read", "write"]);
    let wrong = f.proposal(
        "oauth-different-scope",
        Auth::Oauth {
            credential_ref: "pending".into(),
            config: serde_json::from_value(changed).unwrap(),
        },
    );
    assert!(
        f.runtime
            .approve_connection_with_options(
                wrong["request_id"].as_str().unwrap(),
                wrong["digest"].as_str().unwrap(),
                None,
                ApprovalOptions {
                    all_projects: false,
                    saved_credential: None,
                    reviewed_account_reuse: None,
                    saved_account: Some(SavedAccount {
                        account_id: account["account_id"].as_str().unwrap().into(),
                        revision: 1,
                    })
                }
            )
            .is_err()
    );
    let exact = f.proposal(
        "oauth-exact",
        Auth::Oauth {
            credential_ref: "pending".into(),
            config: serde_json::from_value(cfg).unwrap(),
        },
    );
    assert!(
        f.runtime
            .approve_connection_with_options(
                exact["request_id"].as_str().unwrap(),
                exact["digest"].as_str().unwrap(),
                None,
                ApprovalOptions {
                    all_projects: false,
                    saved_credential: None,
                    reviewed_account_reuse: None,
                    saved_account: Some(SavedAccount {
                        account_id: account["account_id"].as_str().unwrap().into(),
                        revision: 1,
                    })
                }
            )
            .is_ok()
    );
}

fn native_adapter(name: &str, lock: &str, env: &str) -> Connection {
    serde_json::from_value(json!({
        "label":name,"endpoint":"stdio:local","method":"POST",
        "auth":{"kind":"header","name":"x-api-key","credential_ref":"pending"},
        "input_schema":{"type":"object","properties":{"host":{"const":"https://api.alpha.example"}}},
        "output_schema":{"type":"object"},"timeout_ms":2000,"max_response_bytes":4096,
        "mcp":{"protocol":"2026-07-28","transport":{"kind":"stdio",
            "executable":"/usr/bin/python3","sha256":"a".repeat(64),
            "args":["/opt/alpha/adapter.py","--lock-sha256",lock],"credential_env":env},
            "tool":{"name":name,"input_schema":{"type":"object"},"output_schema":{"type":"object"}}}
    })).unwrap()
}

fn native_request(f: &mut Fixture, name: &str, key: &str, connection: Connection) -> Value {
    f.runtime
        .propose_connection(
            &f.a,
            ConnectionProposal {
                name: name.into(),
                request_key: key.into(),
                rationale: "Synthetic adapter revision".into(),
                connection,
            },
        )
        .unwrap()
}

#[test]
fn owner_can_page_large_pending_requests_without_deciding_or_losing_any() {
    let mut f = Fixture::new();
    let mut expected = std::collections::BTreeSet::new();
    for index in 0..5 {
        let name = format!("large_{index}");
        let mut connection = native_adapter(&name, "1", "SYNTHETIC_KEY");
        connection.input_schema["description"] = json!("x".repeat(12 * 1024));
        let request = native_request(&mut f, &name, &name, connection);
        expected.insert(request["request_id"].as_str().unwrap().to_owned());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut after = None;
    loop {
        let page = f
            .runtime
            .connection_requests_page(&f.a, after.as_deref())
            .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < 49 * 1024);
        for item in page["requests"].as_array().unwrap() {
            assert_eq!(item["state"], "pending");
            assert!(seen.insert(item["request_id"].as_str().unwrap().to_owned()));
        }
        after = page["next_cursor"].as_str().map(str::to_owned);
        if after.is_none() {
            break;
        }
        assert!(
            f.runtime
                .connection_requests_page(&f.b, after.as_deref())
                .is_err()
        );
    }
    assert_eq!(seen, expected);
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"],
        json!([])
    );
}

#[test]
fn source_lock_only_review_preserves_other_grant_request_and_account() {
    let mut f = Fixture::new();
    let old_connection = native_adapter("alpha_read", "old-lock", "ALPHA_KEY");
    let old = native_request(&mut f, "alpha_base", "source-base", old_connection.clone());
    let first = f
        .runtime
        .approve_connection(
            old["request_id"].as_str().unwrap(),
            old["digest"].as_str().unwrap(),
            Some(Secret::new(b"synthetic-key".to_vec()).unwrap()),
        )
        .unwrap();
    let unrelated_grant = f.proposal("preexisting_grant", Auth::None);
    f.approve(&unrelated_grant, Default::default());
    let unrelated_pending = f.proposal("unrelated_pending", Auth::None);
    let saved_account_before = f.runtime.saved_accounts(None).unwrap();
    let grants_before = f.runtime.execution_bindings(&f.a).unwrap()["bindings"].clone();
    let pending_before = f.runtime.connection_requests(&f.a).unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["request_id"] == unrelated_pending["request_id"])
        .unwrap()
        .clone();

    let new_connection = native_adapter("alpha_read", "new-lock", "ALPHA_KEY");
    let mut old_json = serde_json::to_value(old_connection).unwrap();
    let mut new_json = serde_json::to_value(&new_connection).unwrap();
    assert_eq!(old_json["mcp"]["transport"]["args"][2], "old-lock");
    assert_eq!(new_json["mcp"]["transport"]["args"][2], "new-lock");
    old_json["mcp"]["transport"]["args"][2] = json!("same-lock");
    new_json["mcp"]["transport"]["args"][2] = json!("same-lock");
    assert_eq!(
        old_json, new_json,
        "only the source lock argument may differ"
    );
    let proposal = native_request(&mut f, "alpha_revision", "source-revision", new_connection);
    let account_id = first["account"]["account_id"].as_str().unwrap();
    let preview = f
        .runtime
        .preview_reviewed_account_reuse(
            proposal["request_id"].as_str().unwrap(),
            proposal["digest"].as_str().unwrap(),
            account_id,
            1,
        )
        .unwrap();
    assert_eq!(
        preview["old_account_contract"]["transport"]["args"][2],
        "old-lock"
    );
    assert_eq!(
        preview["new_account_contract"]["transport"]["args"][2],
        "new-lock"
    );
    assert_eq!(
        preview["old_operations"][0]["connection"]["mcp"]["tool"],
        preview["new_operation"]["mcp"]["tool"]
    );
    let options = || ApprovalOptions {
        reviewed_account_reuse: Some(ReviewedAccountReuse {
            account_id: account_id.into(),
            revision: 1,
            review_digest: preview["review_digest"].as_str().unwrap().into(),
        }),
        ..Default::default()
    };
    let receipt = f
        .runtime
        .approve_connection_with_options(
            proposal["request_id"].as_str().unwrap(),
            proposal["digest"].as_str().unwrap(),
            None,
            options(),
        )
        .unwrap();
    assert_eq!(receipt["account"], first["account"]);
    assert!(
        f.runtime
            .approve_connection_with_options(
                proposal["request_id"].as_str().unwrap(),
                proposal["digest"].as_str().unwrap(),
                Some(Secret::new(b"must-not-rotate".to_vec()).unwrap()),
                options()
            )
            .is_err()
    );
    let grants_after = f.runtime.execution_bindings(&f.a).unwrap()["bindings"].clone();
    let preserved: Vec<_> = grants_after
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["name"] != "alpha_revision")
        .cloned()
        .collect();
    assert_eq!(
        json!(preserved),
        grants_before,
        "preexisting grants are byte-for-byte unchanged"
    );
    let pending_after = f.runtime.connection_requests(&f.a).unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["request_id"] == unrelated_pending["request_id"])
        .unwrap()
        .clone();
    assert_eq!(
        pending_after, pending_before,
        "unrelated pending request remains unchanged"
    );
    assert_eq!(
        f.runtime.saved_accounts(None).unwrap(),
        saved_account_before
    );
}

#[test]
fn reviewed_native_revision_reuses_one_account_and_preserves_old_grant() {
    let mut f = Fixture::new();
    let old = native_request(
        &mut f,
        "alpha_read",
        "old",
        native_adapter("alpha_read", "1", "ALPHA_KEY"),
    );
    let first = f
        .runtime
        .approve_connection(
            old["request_id"].as_str().unwrap(),
            old["digest"].as_str().unwrap(),
            Some(Secret::new(b"synthetic-key".to_vec()).unwrap()),
        )
        .unwrap();
    let account = first["account"].clone();
    let account_id = account["account_id"].as_str().unwrap();
    let next = native_request(
        &mut f,
        "alpha_write",
        "new",
        native_adapter("alpha_write", "2", "ALPHA_KEY"),
    );
    let next_id = next["request_id"].as_str().unwrap();
    let next_digest = next["digest"].as_str().unwrap();
    let candidate = native_adapter("alpha_write", "2", "ALPHA_KEY");
    let list = f
        .runtime
        .saved_accounts_for(None, Some(&candidate))
        .unwrap();
    assert_eq!(list["accounts"][0]["compatible"], false);
    assert_eq!(list["accounts"][0]["review_required"], true);
    assert!(
        f.runtime
            .approve_connection_with_options(
                next_id,
                next_digest,
                None,
                ApprovalOptions {
                    saved_account: Some(SavedAccount {
                        account_id: account_id.into(),
                        revision: 1
                    }),
                    ..Default::default()
                }
            )
            .is_err()
    );
    let preview = f
        .runtime
        .preview_reviewed_account_reuse(next_id, next_digest, account_id, 1)
        .unwrap();
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "preview cannot grant the pending operation"
    );
    assert!(
        f.runtime
            .preview_reviewed_account_reuse(next_id, "wrong-request-digest", account_id, 1)
            .is_err()
    );
    assert_eq!(
        preview["old_operations"][0]["connection"]["mcp"]["tool"]["name"],
        "alpha_read"
    );
    assert_eq!(
        preview["new_operation"]["mcp"]["tool"]["name"],
        "alpha_write"
    );
    assert_eq!(preview["old_account_contract"]["transport"]["args"][2], "1");
    assert_eq!(preview["new_account_contract"]["transport"]["args"][2], "2");
    let options = || ApprovalOptions {
        reviewed_account_reuse: Some(ReviewedAccountReuse {
            account_id: account_id.into(),
            revision: 1,
            review_digest: preview["review_digest"].as_str().unwrap().into(),
        }),
        ..Default::default()
    };
    assert!(
        f.runtime
            .approve_connection_with_options(
                next_id,
                next_digest,
                None,
                ApprovalOptions {
                    reviewed_account_reuse: Some(ReviewedAccountReuse {
                        account_id: account_id.into(),
                        revision: 1,
                        review_digest: "wrong-review-digest".into()
                    }),
                    ..Default::default()
                }
            )
            .is_err()
    );
    let second = f
        .runtime
        .approve_connection_with_options(next_id, next_digest, None, options())
        .unwrap();
    assert_eq!(second["account"], account);
    assert_eq!(
        second,
        f.runtime
            .approve_connection_with_options(next_id, next_digest, None, options())
            .unwrap()
    );
    assert!(
        f.runtime
            .approve_connection_with_options(next_id, next_digest, None, Default::default())
            .is_err()
    );
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let stale = native_request(
        &mut f,
        "alpha_write",
        "stale",
        native_adapter("alpha_write", "5", "ALPHA_KEY"),
    );
    f.runtime
        .revoke_execution(&f.a, "connection", "alpha_write")
        .unwrap();
    assert!(
        f.runtime
            .preview_reviewed_account_reuse(
                stale["request_id"].as_str().unwrap(),
                stale["digest"].as_str().unwrap(),
                account_id,
                1
            )
            .is_err()
    );
    let third = native_request(
        &mut f,
        "alpha_third",
        "third",
        native_adapter("alpha_third", "3", "ALPHA_KEY"),
    );
    assert!(
        f.runtime
            .preview_reviewed_account_reuse(
                third["request_id"].as_str().unwrap(),
                third["digest"].as_str().unwrap(),
                account_id,
                2
            )
            .is_err()
    );
    let other_env = native_request(
        &mut f,
        "other_provider",
        "other",
        native_adapter("other_provider", "4", "OTHER_KEY"),
    );
    assert!(
        f.runtime
            .preview_reviewed_account_reuse(
                other_env["request_id"].as_str().unwrap(),
                other_env["digest"].as_str().unwrap(),
                account_id,
                1
            )
            .is_err()
    );
    let mut other_command =
        serde_json::to_value(native_adapter("other_command", "4", "ALPHA_KEY")).unwrap();
    other_command["mcp"]["transport"]["executable"] = json!("/opt/other-provider/cli");
    let other_command = native_request(
        &mut f,
        "other_command",
        "other-command",
        serde_json::from_value(other_command).unwrap(),
    );
    assert!(
        f.runtime
            .preview_reviewed_account_reuse(
                other_command["request_id"].as_str().unwrap(),
                other_command["digest"].as_str().unwrap(),
                account_id,
                1
            )
            .is_err()
    );
    let mut changed_host =
        serde_json::to_value(native_adapter("changed_host", "4", "ALPHA_KEY")).unwrap();
    changed_host["mcp"]["transport"]["args"][0] = json!("https://api.other.example");
    let changed_host = native_request(
        &mut f,
        "changed_host",
        "changed-host",
        serde_json::from_value(changed_host).unwrap(),
    );
    assert!(
        f.runtime
            .approve_connection_with_options(
                changed_host["request_id"].as_str().unwrap(),
                changed_host["digest"].as_str().unwrap(),
                None,
                ApprovalOptions {
                    saved_account: Some(SavedAccount {
                        account_id: account_id.into(),
                        revision: 1
                    }),
                    ..Default::default()
                }
            )
            .is_err()
    );
    f.runtime.revoke_saved_account(account_id, 1).unwrap();
    assert!(
        f.runtime
            .preview_reviewed_account_reuse(
                third["request_id"].as_str().unwrap(),
                third["digest"].as_str().unwrap(),
                account_id,
                1
            )
            .is_err()
    );
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"],
        json!([])
    );
}

#[test]
fn owner_connects_account_without_grant_then_approves_workspace_operation() {
    let mut f = Fixture::new();
    let connection = Connection {
        label: "signed in account".into(),
        credential_label: None,
        endpoint: "https://example.com/service".into(),
        method: "POST".into(),
        auth: Auth::Bearer {
            credential_ref: "pending".into(),
        },
        input_schema: json!({"type":"object"}),
        output_schema: json!({"type":"object"}),
        timeout_ms: 1000,
        max_response_bytes: 4096,
        mcp: None,
        metering: None,
        loopback_test: false,
    };
    let account = f
        .runtime
        .connect_account(
            &connection,
            Secret::new(b"synthetic-account-login".to_vec()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        f.runtime.connection_requests(&f.a).unwrap()["requests"],
        json!([])
    );
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"],
        json!([])
    );
    assert_eq!(
        f.runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
    let p = f.proposal(
        "later-operation",
        Auth::Bearer {
            credential_ref: "pending".into(),
        },
    );
    let receipt = f
        .runtime
        .approve_connection_with_options(
            p["request_id"].as_str().unwrap(),
            p["digest"].as_str().unwrap(),
            None,
            ApprovalOptions {
                all_projects: false,
                saved_credential: None,
                reviewed_account_reuse: None,
                saved_account: Some(SavedAccount {
                    account_id: account["account_id"].as_str().unwrap().into(),
                    revision: 1,
                }),
            },
        )
        .unwrap();
    assert_eq!(receipt["account"], account);
    assert_eq!(
        f.runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
}

#[test]
fn owner_connect_account_validates_oauth_bundle_before_storing_any_authority() {
    let mut f = Fixture::new();
    let cfg = json!({"issuer":"https://auth.example.com","authorization_endpoint":"https://auth.example.com/authorize",
        "token_endpoint":"https://auth.example.com/token","client_id":"synthetic-client",
        "resource":"https://example.com/service","scopes":["read"],"require_issuer_response":false});
    let config: apr_connectors::oauth::OAuthConfig = serde_json::from_value(cfg).unwrap();
    let connection = Connection {
        label: "oauth account".into(),
        credential_label: None,
        endpoint: "https://example.com/service".into(),
        method: "POST".into(),
        auth: Auth::Oauth {
            credential_ref: "pending".into(),
            config: config.clone(),
        },
        input_schema: json!({"type":"object"}),
        output_schema: json!({"type":"object"}),
        timeout_ms: 1000,
        max_response_bytes: 4096,
        mcp: None,
        metering: None,
        loopback_test: false,
    };
    assert!(
        f.runtime
            .connect_account(
                &connection,
                Secret::new(b"not-an-oauth-bundle".to_vec()).unwrap()
            )
            .is_err()
    );
    assert_eq!(
        f.runtime.saved_accounts(None).unwrap()["accounts"],
        json!([])
    );
    let digest = apr_core::content_digest(&serde_json::to_value(&config).unwrap()).unwrap();
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let bundle = json!({"access":"synthetic-access","refresh":"synthetic-refresh","expires":expires,
        "config_digest":digest,"scope":["read"]});
    let sealed = base64url(&serde_json::to_vec(&bundle).unwrap());
    let account = f
        .runtime
        .connect_account(&connection, Secret::new(sealed.into_bytes()).unwrap())
        .unwrap();
    assert_eq!(
        f.runtime.execution_bindings(&f.a).unwrap()["bindings"],
        json!([])
    );
    assert_eq!(
        f.runtime.connection_library(None, false).unwrap()["connections"],
        json!([])
    );
    let p = f.proposal(
        "oauth-later",
        Auth::Oauth {
            credential_ref: "pending".into(),
            config,
        },
    );
    let receipt = f
        .runtime
        .approve_connection_with_options(
            p["request_id"].as_str().unwrap(),
            p["digest"].as_str().unwrap(),
            None,
            ApprovalOptions {
                all_projects: false,
                saved_credential: None,
                reviewed_account_reuse: None,
                saved_account: Some(SavedAccount {
                    account_id: account["account_id"].as_str().unwrap().into(),
                    revision: 1,
                }),
            },
        )
        .unwrap();
    assert_eq!(receipt["account"], account);
}
