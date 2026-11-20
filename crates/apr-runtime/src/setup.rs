//! Recipe-author connection proposals and separate, trusted owner approvals.
//! The scoped agent can request access and preview costing; it cannot grant it.
use crate::store::{StoreLock, now_ms};
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::{Auth, Connection, CredentialStore, DispatchError, Secret};
use apr_core::content_digest;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) const MIGRATION_V11: &str = r#"
CREATE TABLE connection_requests (
 id TEXT PRIMARY KEY, assignment_id TEXT NOT NULL REFERENCES assignments(id),
 name TEXT NOT NULL, request_key TEXT NOT NULL, digest TEXT NOT NULL,
 payload TEXT NOT NULL, rationale TEXT NOT NULL, expected_revision INTEGER NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','approved','rejected')),
 receipt TEXT, created_at INTEGER NOT NULL, decided_at INTEGER,
 UNIQUE(assignment_id,request_key)
);
CREATE INDEX connection_requests_scope ON connection_requests(assignment_id,state,created_at);
"#;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProposal {
    pub name: String,
    pub request_key: String,
    pub rationale: String,
    pub connection: Connection,
}

fn public_proposal(proposal: &ConnectionProposal) -> Result<()> {
    if proposal.name.is_empty()
        || proposal.name.len() > 80
        || !proposal
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        || proposal.request_key.is_empty()
        || proposal.request_key.len() > 128
        || proposal.rationale.trim().is_empty()
        || proposal.rationale.len() > 2000
    {
        return Err(RuntimeError::InvalidInput(
            "invalid connection request".into(),
        ));
    }
    // Credential references are allocated only by the owner approval path.
    match &proposal.connection.auth {
        Auth::None => {}
        Auth::Bearer { credential_ref }
        | Auth::Header { credential_ref, .. }
        | Auth::Oauth { credential_ref, .. }
            if credential_ref == "pending" => {}
        _ => {
            return Err(RuntimeError::InvalidInput(
                "proposals use credential_ref pending; never include a key".into(),
            ));
        }
    }
    if proposal
        .connection
        .mcp
        .as_ref()
        .is_some_and(|m| m.tool.is_none() && m.feature.is_none())
    {
        return Err(RuntimeError::InvalidInput(
            "discover and select an MCP tool before requesting execution access".into(),
        ));
    }
    proposal
        .connection
        .validate()
        .map_err(|_| RuntimeError::InvalidInput("invalid connection or cost contract".into()))?;
    if serde_json::to_vec(proposal)?.len() > 40 * 1024 {
        return Err(RuntimeError::LimitExceeded(
            "connection proposal exceeds 48 KiB".into(),
        ));
    }
    Ok(())
}

impl Runtime {
    /// Owner-only preview for reusing one saved key with one pending native
    /// adapter revision. This is an eligibility result, never a grant.
    pub fn preview_reviewed_account_reuse(
        &self,
        id: &str,
        reviewed_digest: &str,
        account_id: &str,
        account_revision: u64,
    ) -> Result<Value> {
        let row: Option<(String, String, String, String, u64, i64)> = self.db.query_row(
            "SELECT assignment_id,digest,payload,state,expected_revision,created_at FROM connection_requests WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        ).optional()?;
        let (assignment, digest, payload, state, revision, created) =
            row.ok_or_else(|| RuntimeError::NotFound("connection request not found".into()))?;
        self.ensure_assignment_active(&assignment)?;
        if digest != reviewed_digest || state != "pending" || now_ms() - created > 7 * 86400 * 1000
        {
            return Err(RuntimeError::State(
                "review a current pending connection request".into(),
            ));
        }
        let connection: Connection = serde_json::from_str(&payload)?;
        let current: u64 = self.db.query_row(
            "SELECT COALESCE((SELECT revision FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=(SELECT name FROM connection_requests WHERE id=?2)),0)",
            params![assignment, id], |r| r.get(0)
        )?;
        if current != revision {
            return Err(RuntimeError::State(
                "connection changed after proposal".into(),
            ));
        }
        let (reference, old_contract) =
            self.reviewed_account_candidate(account_id, account_revision, &connection)?;
        let new_contract = crate::connection_library::account_contract(&connection)?;
        let mut old_operations = Vec::new();
        let mut query = self.db.prepare(
            "SELECT assignment_id,name,revision,payload FROM execution_grants WHERE kind='connection' AND enabled=1 AND json_extract(payload,'$.auth.credential_ref')=?1 ORDER BY assignment_id,name LIMIT 65"
        )?;
        for row in query.query_map([&reference], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            if old_operations.len() == 64 {
                return Err(RuntimeError::LimitExceeded(
                    "too many old operations for one review".into(),
                ));
            }
            let (assignment_id, name, operation_revision, payload) = row?;
            let mut operation: Value = serde_json::from_str(&payload)?;
            operation["auth"]["credential_ref"] = json!("stored");
            old_operations.push(json!({"assignment_id":assignment_id,"name":name,"revision":operation_revision,"connection":operation}));
        }
        let mut new_operation: Value = serde_json::from_str(&payload)?;
        new_operation["auth"]["credential_ref"] = json!("pending");
        let mut preview = json!({
            "request_id":id,"request_digest":digest,"account_id":account_id,"account_revision":account_revision,
            "old_account_contract":old_contract,"new_account_contract":new_contract,
            "old_operations":old_operations,"new_operation":new_operation,
            "note":"Review executable, full launch arguments, hashes, authentication, schemas and operation scope. Opaque adapter code may contain destinations that these fields cannot reveal."
        });
        if serde_json::to_vec(&preview)?.len() > 256 * 1024 {
            return Err(RuntimeError::LimitExceeded(
                "account review is too large; narrow the saved operations before review".into(),
            ));
        }
        preview["review_digest"] = json!(content_digest(&preview)?);
        Ok(preview)
    }

    /// Trusted local UI/CLI may create the same request as an authorized agent.
    pub fn propose_connection(
        &mut self,
        assignment: &str,
        proposal: ConnectionProposal,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.propose_connection_inner(assignment, proposal)
    }
    pub(crate) fn propose_connection_inner(
        &mut self,
        assignment: &str,
        proposal: ConnectionProposal,
    ) -> Result<Value> {
        self.ensure_assignment_active(assignment)?;
        public_proposal(&proposal)?;
        let value = serde_json::to_value(&proposal)?;
        let digest = content_digest(&value)?;
        let prior: Option<(String,String,String)> = self.db.query_row(
            "SELECT id,digest,state FROM connection_requests WHERE assignment_id=?1 AND request_key=?2",
            params![assignment,proposal.request_key], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((id, existing, state)) = prior {
            if existing != digest {
                return Err(RuntimeError::InvalidInput(
                    "request key already has another proposal".into(),
                ));
            }
            let current:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM connection_requests r JOIN execution_grants g ON g.assignment_id=r.assignment_id AND g.name=r.name AND g.kind='connection' WHERE r.id=?1 AND g.enabled=1 AND g.revision=r.expected_revision+1)",[&id],|r|r.get(0))?;
            return Ok(
                json!({"request_id":id,"digest":digest,"state":state,"granted":state=="approved"&&current,"next_action":if state=="pending"{"Review in CookBook setup"}else{"Inspect current connection state"}}),
            );
        }
        let pending: u64 = self.db.query_row(
            "SELECT count(*) FROM connection_requests WHERE assignment_id=?1 AND state='pending'",
            [assignment],
            |r| r.get(0),
        )?;
        if pending >= 100 {
            return Err(RuntimeError::LimitExceeded(
                "resolve pending connection requests first".into(),
            ));
        }
        let revision: u64 = self.db.query_row("SELECT revision FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=?2",params![assignment,proposal.name],|r|r.get(0)).optional()?.unwrap_or(0);
        let id = format!("approval-{}", uuid::Uuid::new_v4().simple());
        self.db.execute("INSERT INTO connection_requests(id,assignment_id,name,request_key,digest,payload,rationale,expected_revision,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9)",
            params![id,assignment,proposal.name,proposal.request_key,digest,serde_json::to_string(&proposal.connection)?,proposal.rationale,revision,now_ms()])?;
        Ok(
            json!({"request_id":id,"digest":digest,"state":"pending","granted":false,"next_action":"Review in CookBook setup"}),
        )
    }
    /// Proposal details contain only agent-submitted public configuration.
    pub fn connection_requests(&self, assignment: &str) -> Result<Value> {
        self.connection_requests_page(assignment, None)
    }
    /// Bounded owner pages retain pending-first ordering without requiring a
    /// decision on unrelated requests before later requests can be inspected.
    pub fn connection_requests_page(&self, assignment: &str, after: Option<&str>) -> Result<Value> {
        let (rank, created) = if let Some(id) = after {
            self.db.query_row(
                "SELECT CASE state WHEN 'pending' THEN 0 ELSE 1 END,created_at FROM connection_requests WHERE assignment_id=?1 AND id=?2",
                params![assignment,id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))
            ).optional()?.ok_or_else(|| RuntimeError::InvalidInput("request cursor is unavailable in this workspace".into()))?
        } else {
            (0, 0)
        };
        let mut query = self.db.prepare("SELECT id,name,digest,payload,rationale,state,expected_revision,created_at FROM connection_requests WHERE assignment_id=?1 AND (?2 IS NULL OR (CASE state WHEN 'pending' THEN 0 ELSE 1 END,-created_at,id)>(?3,-?4,?2)) ORDER BY CASE state WHEN 'pending' THEN 0 ELSE 1 END,created_at DESC,id LIMIT 21")?;
        let mut total = 0;
        let mut items = Vec::new();
        let mut truncated = false;
        for row in query.query_map(params![assignment, after, rank, created], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, u64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })? {
            let (id, name, digest, payload, rationale, state, revision, created_at) = row?;
            let config: Value = serde_json::from_str(&payload)?;
            let rate_only: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM price_reviews WHERE request_id=?1)",
                [&id],
                |r| r.get(0),
            )?;
            let item = json!({"rate_only":rate_only,"request_id":id,"name":name,"digest":digest,"connection":config,"rationale":rationale,"state":state,"expected_revision":revision,"created_at":created_at});
            let size = serde_json::to_vec(&item)?.len();
            if total + size > 48 * 1024 || items.len() == 20 {
                truncated = true;
                break;
            }
            total += size;
            items.push(item);
        }
        let next = if truncated {
            items.last().and_then(|item| item["request_id"].as_str())
        } else {
            None
        };
        Ok(
            json!({"requests":items,"truncated":truncated,"next_cursor":next,"authority":"owner approval required"}),
        )
    }
    /// Separate owner entrypoint, never routed by scoped_call or MCP.
    /// Hash and current revision protect review-to-approval races. Grant + receipt
    /// commit atomically; replaying an approval cannot rotate credentials again.
    pub fn approve_connection(
        &mut self,
        id: &str,
        reviewed_digest: &str,
        secret: Option<Secret>,
    ) -> Result<Value> {
        self.approve_connection_with_options(id, reviewed_digest, secret, Default::default())
    }
    pub fn approve_connection_with_options(
        &mut self,
        id: &str,
        reviewed_digest: &str,
        secret: Option<Secret>,
        options: crate::connection_library::ApprovalOptions,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        let options_digest = content_digest(&serde_json::to_value(&options)?)?;
        type ApprovalRow = (
            String,
            String,
            String,
            String,
            String,
            u64,
            Option<String>,
            i64,
        );
        let row:Option<ApprovalRow> = self.db.query_row(
            "SELECT assignment_id,name,digest,payload,state,expected_revision,receipt,created_at FROM connection_requests WHERE id=?1",[id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
        let (assignment, name, digest, payload, state, revision, receipt, created) =
            row.ok_or_else(|| RuntimeError::NotFound("connection request not found".into()))?;
        if digest != reviewed_digest {
            return Err(RuntimeError::InvalidInput(
                "proposal changed; review it again".into(),
            ));
        }
        self.ensure_assignment_active(&assignment)?;
        if state == "approved" {
            if secret.is_some() {
                return Err(RuntimeError::InvalidInput(
                    "approval already decided; a new key cannot be supplied on replay".into(),
                ));
            }
            let current:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=?2 AND enabled=1 AND revision=?3)",params![assignment,name,revision+1],|r|r.get(0))?;
            if !current {
                return Err(RuntimeError::State(
                    "previous approval is no longer current; submit a new request".into(),
                ));
            }
            let receipt: Value = serde_json::from_str(
                &receipt
                    .ok_or_else(|| RuntimeError::Integrity("approval receipt missing".into()))?,
            )?;
            if options.all_projects != receipt.get("library").is_some_and(|v| !v.is_null()) {
                return Err(RuntimeError::State("approval scope cannot change on replay; save the existing connection to the library explicitly".into()));
            }
            if receipt
                .get("approval_options_digest")
                .is_some_and(|v| v.as_str() != Some(&options_digest))
                || (receipt.get("approval_options_digest").is_none()
                    && options.reviewed_account_reuse.is_some())
            {
                return Err(RuntimeError::State(
                    "approval options changed on replay".into(),
                ));
            }
            return Ok(receipt);
        }
        if state != "pending" || now_ms() - created > 7 * 24 * 60 * 60 * 1000 {
            return Err(RuntimeError::State(
                "request is rejected or expired; create a new request".into(),
            ));
        }
        let current:u64=self.db.query_row("SELECT revision FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=?2",params![assignment,name],|r|r.get(0)).optional()?.unwrap_or(0);
        if current != revision {
            return Err(RuntimeError::State(
                "connection changed after this proposal; request a fresh review".into(),
            ));
        }
        let mut connection: Connection = serde_json::from_str(&payload)?;
        let saved_count = usize::from(options.saved_credential.is_some())
            + usize::from(options.saved_account.is_some())
            + usize::from(options.reviewed_account_reuse.is_some());
        if (secret.is_some() && saved_count != 0) || saved_count > 1 {
            return Err(RuntimeError::InvalidInput(
                "choose one saved account or a new key".into(),
            ));
        }
        if let Some(saved) = &options.saved_account {
            self.reuse_saved_account(&saved.account_id, saved.revision, &mut connection)?;
        }
        let reviewed_account = if let Some(saved) = &options.reviewed_account_reuse {
            let preview = self.preview_reviewed_account_reuse(
                id,
                reviewed_digest,
                &saved.account_id,
                saved.revision,
            )?;
            if preview["review_digest"].as_str() != Some(&saved.review_digest) {
                return Err(RuntimeError::State(
                    "account review changed; preview it again".into(),
                ));
            }
            let (reference, _) =
                self.reviewed_account_candidate(&saved.account_id, saved.revision, &connection)?;
            match &mut connection.auth {
                Auth::Bearer { credential_ref } | Auth::Header { credential_ref, .. } => {
                    *credential_ref = reference
                }
                _ => unreachable!("candidate checked authentication"),
            }
            Some(json!({"account_id":saved.account_id,"revision":saved.revision}))
        } else {
            None
        };
        if let Some(saved) = &options.saved_credential {
            let source = self.saved_connection(&saved.library_id, saved.revision)?;
            crate::connection_library::reuse_auth(&mut connection, &source)?;
        }
        if let Auth::Oauth { config, .. } = &connection.auth
            && options.saved_credential.is_none()
            && options.saved_account.is_none()
            && options.reviewed_account_reuse.is_none()
        {
            apr_connectors::oauth::validate_bundle(
                config,
                secret.as_ref().ok_or_else(|| {
                    RuntimeError::State("Complete browser authorization first".into())
                })?,
            )
            .map_err(|_| {
                RuntimeError::State(
                    "OAuth authorization is missing, expired or for another connection".into(),
                )
            })?;
        }
        let reference = format!("credential-{}", uuid::Uuid::new_v4().simple());
        match &mut connection.auth {
            Auth::None if secret.is_none() => {}
            Auth::Bearer { .. } | Auth::Header { .. } | Auth::Oauth { .. } if saved_count != 0 => {}
            Auth::Bearer { credential_ref }
            | Auth::Header { credential_ref, .. }
            | Auth::Oauth { credential_ref, .. }
                if secret.is_some() =>
            {
                *credential_ref = reference.clone()
            }
            _ => {
                return Err(RuntimeError::InvalidInput(
                    "secret input does not match requested authentication".into(),
                ));
            }
        }
        connection
            .validate()
            .map_err(|_| RuntimeError::InvalidInput("invalid approved connection".into()))?;
        let has_secret = secret.is_some();
        let vault = if has_secret {
            Some(self.vault()?)
        } else {
            None
        };
        if let Some(secret) = secret {
            vault
                .as_ref()
                .expect("new key has a vault")
                .set(&reference, secret)
                .map_err(|_| {
                    RuntimeError::State("private credential file unavailable; check storage permissions or import saved keys in Settings".into())
                })?;
        }
        let outcome = (|| -> Result<Value> {
            let tx = self.db.transaction()?;
            tx.execute("INSERT INTO execution_grants(assignment_id,kind,name,revision,enabled,payload) VALUES(?1,'connection',?2,?3,1,?4) ON CONFLICT(assignment_id,kind,name) DO UPDATE SET revision=excluded.revision,enabled=1,payload=excluded.payload",params![assignment,name,revision+1,serde_json::to_string(&connection)?])?;
            let account = if let Some(account) = &reviewed_account {
                Some(account.clone())
            } else {
                crate::connection_library::register_account(&tx, &connection)?
            };
            let library = if options.all_projects {
                Some(crate::connection_library::publish(
                    &tx,
                    &assignment,
                    &name,
                    revision + 1,
                    &serde_json::to_string(&connection)?,
                )?)
            } else {
                None
            };
            let receipt = json!({"request_id":id,"digest":digest,"assignment_id":assignment,"name":name,"revision":revision+1,"state":"approved","granted":true,"library":library,"account":account,"approval_options_digest":options_digest,"review_digest":options.reviewed_account_reuse.as_ref().map(|v| &v.review_digest)});
            let changed=tx.execute("UPDATE connection_requests SET state='approved',receipt=?2,decided_at=?3 WHERE id=?1 AND state='pending'",params![id,serde_json::to_string(&receipt)?,now_ms()])?;
            if changed != 1 {
                return Err(RuntimeError::State("approval already decided".into()));
            }
            crate::store::fault_point("approval_before_commit");
            tx.commit()?;
            Ok(receipt)
        })();
        if outcome.is_err() && has_secret {
            let _ = vault
                .as_ref()
                .expect("new key has a vault")
                .remove(&reference);
        }
        outcome
    }
    /// Read only the exact pending owner-reviewed OAuth request. Approval repeats
    /// these checks after login, so a stale browser result cannot authorize changes.
    pub fn oauth_request_config(
        &self,
        id: &str,
        reviewed: &str,
    ) -> Result<apr_connectors::oauth::OAuthConfig> {
        let (assignment,payload,digest,state,created,revision): (String,String,String,String,i64,u64) = self.db.query_row(
            "SELECT assignment_id,payload,digest,state,created_at,expected_revision FROM connection_requests WHERE id=?1", [id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        self.ensure_assignment_active(&assignment)?;
        if digest != reviewed || state != "pending" || now_ms() - created > 7 * 86400 * 1000 {
            return Err(RuntimeError::State(
                "Review a current pending OAuth request first".into(),
            ));
        }
        let current:u64 = self.db.query_row("SELECT COALESCE((SELECT g.revision FROM execution_grants g JOIN connection_requests r ON g.assignment_id=r.assignment_id AND g.name=r.name AND g.kind='connection' WHERE r.id=?1),0)",[id],|r|r.get(0))?;
        if current != revision {
            return Err(RuntimeError::State(
                "Connection changed; review a new request".into(),
            ));
        }
        match serde_json::from_str::<Connection>(&payload)?.auth {
            Auth::Oauth { config, .. } => Ok(config),
            _ => Err(RuntimeError::InvalidInput(
                "This request does not use OAuth".into(),
            )),
        }
    }
    pub fn reject_connection(&mut self, id: &str, reviewed_digest: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        let changed=self.db.execute("UPDATE connection_requests SET state='rejected',decided_at=?3 WHERE id=?1 AND digest=?2 AND state='pending'",params![id,reviewed_digest,now_ms()])?;
        if changed == 0 {
            return Err(RuntimeError::State(
                "request changed or has already been decided".into(),
            ));
        }
        Ok(json!({"request_id":id,"state":"rejected","granted":false}))
    }
    /// User explicitly opted into discovery of this exact endpoint or command.
    /// This does not install a connection or execute a tool. The secret is ephemeral.
    pub fn discover_mcp(&self, connection: &Connection, secret: Option<Secret>) -> Result<Value> {
        self.discover_mcp_kind(connection, secret, "tools")
    }
    pub fn discover_mcp_kind(
        &self,
        connection: &Connection,
        secret: Option<Secret>,
        kind: &str,
    ) -> Result<Value> {
        self.discover_mcp_kind_with_protocol(connection, secret, kind, false)
    }
    pub fn discover_mcp_kind_with_protocol(
        &self,
        connection: &Connection,
        secret: Option<Secret>,
        kind: &str,
        automatic_protocol: bool,
    ) -> Result<Value> {
        struct Ephemeral(Option<Secret>);
        impl CredentialStore for Ephemeral {
            fn get(&self, _: &str) -> std::result::Result<Secret, DispatchError> {
                // Secret is deliberately not clonable/debuggable. One dispatch
                // retrieves a short-lived copy for the broker and zeroizes both.
                self.0
                    .as_ref()
                    .ok_or(DispatchError::CredentialUnavailable)?
                    .duplicate()
            }
        }
        Self::discover_mcp_using(connection, &Ephemeral(secret), kind, automatic_protocol)
    }
    /// Owner-only metadata retry using an exact, current saved account. This
    /// creates no tool grant and never exports a credential to the caller.
    pub fn discover_mcp_with_account(
        &self,
        connection: &Connection,
        account_id: &str,
        revision: u64,
        kind: &str,
    ) -> Result<Value> {
        self.discover_mcp_with_account_protocol(connection, account_id, revision, kind, false)
    }
    pub fn discover_mcp_with_account_protocol(
        &self,
        connection: &Connection,
        account_id: &str,
        revision: u64,
        kind: &str,
        automatic_protocol: bool,
    ) -> Result<Value> {
        let mut bound = connection.clone();
        self.reuse_saved_account(account_id, revision, &mut bound)?;
        Self::discover_mcp_using(&bound, &self.vault()?, kind, automatic_protocol)
    }
    /// Trusted owner API-key setup. Save the account independently of metadata
    /// availability, just as browser authorization does. No tool is approved.
    pub fn connect_mcp_account(
        &mut self,
        connection: &Connection,
        secret: Option<Secret>,
        saved: Option<crate::connection_library::SavedAccount>,
        kind: &str,
    ) -> Result<Value> {
        self.connect_mcp_account_protocol(connection, secret, saved, kind, false)
    }
    pub fn connect_mcp_account_protocol(
        &mut self,
        connection: &Connection,
        secret: Option<Secret>,
        saved: Option<crate::connection_library::SavedAccount>,
        kind: &str,
        automatic_protocol: bool,
    ) -> Result<Value> {
        connection
            .validate()
            .map_err(|_| RuntimeError::InvalidInput("invalid account connection".into()))?;
        if !matches!(kind, "tools" | "resources" | "prompts")
            || connection.mcp.as_ref().is_none_or(|m| m.is_stdio())
            || !matches!(&connection.auth,
                Auth::Bearer { credential_ref } | Auth::Header { credential_ref, .. }
                    if credential_ref == "pending")
        {
            return Err(RuntimeError::InvalidInput(
                "API-key setup requires an unbound remote MCP connection".into(),
            ));
        }
        let account = match (secret, saved) {
            (Some(secret), None) => self.connect_account(connection, secret)?,
            (None, Some(saved)) => {
                // Refuse a stale or incompatible selection before reporting any
                // account as saved. Discovery repeats this check before vault use.
                let mut bound = connection.clone();
                self.reuse_saved_account(&saved.account_id, saved.revision, &mut bound)?;
                json!({"account_id":saved.account_id,"revision":saved.revision})
            }
            _ => {
                return Err(RuntimeError::InvalidInput(
                    "choose one saved account or enter one API key".into(),
                ));
            }
        };
        let id = account["account_id"]
            .as_str()
            .ok_or_else(|| RuntimeError::Integrity("missing account identity".into()))?;
        let revision = account["revision"]
            .as_u64()
            .ok_or_else(|| RuntimeError::Integrity("missing account revision".into()))?;
        Ok(
            match self.discover_mcp_with_account_protocol(
                connection,
                id,
                revision,
                kind,
                automatic_protocol,
            ) {
                Ok(discovery) => json!({"account":account,"discovery":discovery,
                "status":"operation_approval_required","grant_created":false}),
                Err(error) => json!({"account":account,"discovery":null,
                "discovery_error":error.to_string(),
                "status":"account_saved_discovery_failed","grant_created":false}),
            },
        )
    }
    fn discover_mcp_using<V: CredentialStore>(
        connection: &Connection,
        vault: &V,
        kind: &str,
        automatic_protocol: bool,
    ) -> Result<Value> {
        if !matches!(kind, "tools" | "resources" | "prompts") {
            return Err(RuntimeError::InvalidInput("invalid discovery kind".into()));
        }
        // Only owner-controlled metadata discovery negotiates. The selected
        // concrete revision is returned for the separately reviewed tool grant.
        let mut resolved = connection.clone();
        if automatic_protocol {
            let protocol =
                apr_connectors::mcp::negotiate_protocol(connection, vault).map_err(|error| {
                    RuntimeError::State(format!("MCP protocol discovery failed: {error}"))
                })?;
            resolved
                .mcp
                .as_mut()
                .ok_or_else(|| RuntimeError::InvalidInput("MCP connection required".into()))?
                .protocol = protocol;
        }
        let connection = &resolved;
        let result = match kind {
            "tools" => apr_connectors::mcp::discover(connection, vault),
            "resources" | "prompts" => {
                apr_connectors::mcp::discover_features(connection, vault, kind)
            }
            _ => return Err(RuntimeError::InvalidInput("invalid discovery kind".into())),
        };
        // DispatchError contains controlled diagnostics, never response bodies,
        // headers, endpoints or credentials supplied by the provider.
        result
            .map(|mut value| {
                value["protocol"] = json!(connection.mcp.as_ref().map(|m| &m.protocol));
                value["protocol_detected"] = json!(automatic_protocol);
                value
            })
            .map_err(|error| RuntimeError::State(format!("MCP discovery failed: {error}")))
    }
    pub fn owner_assignments(&self) -> Result<Value> {
        let mut query=self.db.prepare("SELECT a.id,a.name,a.state,p.id,p.root FROM assignments a JOIN projects p ON p.id=a.project_id ORDER BY a.created_at DESC LIMIT 100")?;
        let items=query.query_map([],|r|Ok(json!({"assignment_id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"project_id":r.get::<_,String>(3)?,"root":r.get::<_,String>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(json!({"assignments":items}))
    }
    /// Write an opaque attachment into the private state area. The desktop stores
    /// only this path and never returns the bearer token to either WebView.
    pub fn owner_attachment_file(&mut self, assignment: &str) -> Result<PathBuf> {
        let directory = self.root.join("attachments");
        crate::store::private_dir(&directory)?;
        let path = directory.join(format!("desktop-{}.json", uuid::Uuid::new_v4().simple()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut file = options.open(&path)?;
        let attachment = self.create_attachment(assignment, "author")?;
        let result = (|| -> Result<()> {
            file.write_all(&serde_json::to_vec(&attachment)?)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = self.revoke_attachment(attachment["attachment_id"].as_str().unwrap_or(""));
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        Ok(path)
    }
    pub fn owner_project_setup(&mut self, root: &Path, name: &str) -> Result<Value> {
        let project = self.attach_project(root)?;
        self.create_assignment(
            project["project_id"]
                .as_str()
                .ok_or_else(|| RuntimeError::Integrity("project identity missing".into()))?,
            name,
        )
    }
    pub(crate) fn connection_contract(&self, assignment: &str, name: &str) -> Result<Value> {
        let (revision,enabled,payload):(u64,bool,String)=self.db.query_row("SELECT revision,enabled,payload FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=?2",params![assignment,name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(RuntimeError::Forbidden)?;
        let connection: Connection = serde_json::from_str(&payload)?;
        // Neither credential references nor local command/environment arguments are exposed.
        Ok(
            json!({"name":name,"revision":revision,"enabled":enabled,"label":connection.label,"kind":if connection.mcp.is_some(){"mcp"}else{"http.json"},"tool":connection.mcp.as_ref().and_then(|m|m.tool.as_ref()).map(|t|&t.name),"input_schema":connection.input_schema,"output_schema":connection.output_schema,"metering":connection.metering,"credentials":"owner_managed"}),
        )
    }
    pub(crate) fn recipe_readiness(&self, assignment: &str, version: &str) -> Result<Value> {
        let mut stack = vec![version.to_string()];
        let mut seen = std::collections::BTreeSet::new();
        let mut needs = Vec::new();
        let mut visited_steps = 0;
        let mut truncated = false;
        let mut agent_steps = 0;
        let mut connectors = 0;
        let mut unpriced = 0;
        while let Some(version) = stack.pop() {
            if !seen.insert(version.clone()) {
                continue;
            }
            if seen.len() > 4096 {
                return Err(RuntimeError::LimitExceeded(
                    "recipe tree exceeds inspection limit".into(),
                ));
            }
            let manifest:Option<String>=self.db.query_row("SELECT v.manifest FROM process_versions v JOIN catalog c ON c.version=v.version WHERE c.assignment_id=?1 AND v.version=?2",params![assignment,version],|r|r.get(0)).optional()?;
            let manifest = manifest.ok_or(RuntimeError::Forbidden)?;
            let process = apr_core::admit_json(manifest.as_bytes())?;
            if let Err(error) = self.execution_preflight(&process, Some(assignment)) {
                needs.push(json!({"recipe_version":version,"reason":"preflight_blocker","detail":error.code()}));
            }
            for step in process.manifest.steps {
                visited_steps += 1;
                if visited_steps > 10000 || needs.len() >= 20 {
                    truncated = true;
                    break;
                }

                if step.kind == "agent" {
                    agent_steps += 1;
                }
                if let Some(call) = step.call {
                    stack.push(call.process_ref);
                }
                let binding = if step.kind == "script" {
                    Some(("python", "python".to_string()))
                } else {
                    step.connection.map(|name| ("connection", name))
                };
                if let Some((kind, name)) = binding {
                    let payload:Option<String>=self.db.query_row("SELECT payload FROM execution_grants WHERE assignment_id=?1 AND kind=?2 AND name=?3 AND enabled=1",params![assignment,kind,name],|r|r.get(0)).optional()?;
                    if payload.is_none() {
                        needs.push(json!({"recipe_version":version,"step":step.id,"binding":name,"kind":kind,"reason":"needs_owner_approval"}));
                    }
                    if kind == "connection" {
                        connectors += 1;
                        if payload
                            .as_ref()
                            .and_then(|p| serde_json::from_str::<Connection>(p).ok())
                            .is_none_or(|c| c.metering.is_none())
                        {
                            unpriced += 1;
                        }
                    }
                }
            }
            if truncated {
                break;
            }
        }
        Ok(
            json!({"process_ref":version,"can_finish_unattended":!truncated&&needs.is_empty()&&agent_steps==0,"truncated":truncated,"assessment":"static","agent_steps":agent_steps,"connections":connectors,"unpriced_connections":unpriced,"blockers":needs,"conservative":true,"note":"All reachable branches checked; device must be awake. Preflight still validates actual inputs, grants and executor availability."}),
        )
    }
}

pub fn preview_cost(connection: Connection, request: Value, response: Value) -> Result<Value> {
    connection
        .validate()
        .map_err(|_| RuntimeError::InvalidInput("invalid cost contract".into()))?;
    let mapping = connection.metering.as_ref().ok_or_else(|| {
        RuntimeError::InvalidInput(
            "propose a metering contract after analyzing the selected payloads".into(),
        )
    })?;
    let validate = if connection.mcp.is_some() {
        apr_core::validate_service_instance
    } else {
        apr_core::validate_instance
    };
    Ok(
        json!({"observation":mapping.observe_exchange(&request,&response),"request_schema_digest":content_digest(&connection.input_schema)?,"response_schema_digest":content_digest(&connection.output_schema)?,"request_matches_schema":validate(&connection.input_schema,&request).is_ok(),"response_matches_schema":validate(&connection.output_schema,&response).is_ok(),"grants_created":false,"external_calls":0}),
    )
}
