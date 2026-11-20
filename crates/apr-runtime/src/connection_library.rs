//! Owner-approved reusable tools. Agents may bind an exact published contract,
//! but cannot publish tools, widen contracts, or read stored credentials.
use crate::store::{StoreLock, now_ms};
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::{Auth, Connection, Secret};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const MIGRATION_V15: &str = r#"
CREATE TABLE connection_library (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, revision INTEGER NOT NULL,
 enabled INTEGER NOT NULL, payload TEXT NOT NULL,
 source_assignment TEXT NOT NULL REFERENCES assignments(id), source_name TEXT NOT NULL,
 source_revision INTEGER NOT NULL, created_at INTEGER NOT NULL,
 UNIQUE(source_assignment,source_name,source_revision)
);
CREATE TABLE connection_library_uses (
 assignment_id TEXT NOT NULL REFERENCES assignments(id), name TEXT NOT NULL,
 library_id TEXT NOT NULL REFERENCES connection_library(id), binding_revision INTEGER NOT NULL,
 PRIMARY KEY(assignment_id,name)
);
CREATE TABLE connection_library_blocks (
 assignment_id TEXT NOT NULL REFERENCES assignments(id), library_id TEXT NOT NULL REFERENCES connection_library(id),
 PRIMARY KEY(assignment_id,library_id)
);
"#;

pub(crate) const MIGRATION_V17: &str = r#"
CREATE TABLE saved_accounts (
 id TEXT PRIMARY KEY, revision INTEGER NOT NULL, enabled INTEGER NOT NULL,
 label TEXT NOT NULL, credential_ref TEXT NOT NULL UNIQUE,
 contract TEXT NOT NULL, created_at INTEGER NOT NULL
);
"#;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalOptions {
    #[serde(default)]
    pub all_projects: bool,
    #[serde(default)]
    pub saved_credential: Option<SavedCredential>,
    #[serde(default)]
    pub saved_account: Option<SavedAccount>,
    #[serde(default)]
    pub reviewed_account_reuse: Option<ReviewedAccountReuse>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedAccountReuse {
    pub account_id: String,
    pub revision: u64,
    pub review_digest: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedCredential {
    pub library_id: String,
    pub revision: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedAccount {
    pub account_id: String,
    pub revision: u64,
}

fn credential_ref(connection: &Connection) -> Option<&str> {
    match &connection.auth {
        Auth::Bearer { credential_ref }
        | Auth::Header { credential_ref, .. }
        | Auth::Oauth { credential_ref, .. } => Some(credential_ref),
        Auth::None => None,
    }
}

pub(crate) fn account_contract(connection: &Connection) -> Result<Value> {
    let auth = match &connection.auth {
        Auth::Bearer { .. } => json!({"kind":"bearer"}),
        Auth::Header { name, .. } => json!({"kind":"header","name":name.to_ascii_lowercase()}),
        Auth::Oauth { config, .. } => json!({"kind":"oauth","config":config}),
        Auth::None => {
            return Err(RuntimeError::InvalidInput(
                "connection has no account".into(),
            ));
        }
    };
    Ok(json!({"endpoint":connection.endpoint,
        "transport":connection.mcp.as_ref().map(|m| &m.transport),"auth":auth}))
}

pub(crate) fn register_account(
    tx: &Transaction<'_>,
    connection: &Connection,
) -> Result<Option<Value>> {
    let Some(reference) = credential_ref(connection) else {
        return Ok(None);
    };
    let contract = account_contract(connection)?;
    let prior: Option<(String, u64, bool, String)> = tx
        .query_row(
            "SELECT id,revision,enabled,contract FROM saved_accounts WHERE credential_ref=?1",
            [reference],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if let Some((id, revision, enabled, existing)) = prior {
        if !enabled || serde_json::from_str::<Value>(&existing)? != contract {
            return Err(RuntimeError::State(
                "saved account is revoked or has a conflicting contract".into(),
            ));
        }
        return Ok(Some(json!({"account_id":id,"revision":revision})));
    }
    let id = format!("account-{}", uuid::Uuid::new_v4().simple());
    tx.execute("INSERT INTO saved_accounts(id,revision,enabled,label,credential_ref,contract,created_at) VALUES(?1,1,1,?2,?3,?4,?5)",
        params![id,connection.label,reference,serde_json::to_string(&contract)?,now_ms()])?;
    Ok(Some(json!({"account_id":id,"revision":1})))
}

pub(crate) fn backfill_accounts(tx: &Transaction<'_>) -> Result<()> {
    let payloads: Vec<String> = {
        let mut statement=tx.prepare("SELECT payload FROM execution_grants WHERE kind='connection' AND enabled=1 UNION SELECT payload FROM connection_library WHERE enabled=1")?;
        statement
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?
    };
    for payload in payloads {
        register_account(tx, &serde_json::from_str::<Connection>(&payload)?)?;
    }
    Ok(())
}

pub(crate) fn public_contract(connection: &Connection) -> Value {
    json!({"label":connection.label,"kind":if connection.mcp.is_some(){"mcp"}else{"http.json"},
        "tool":connection.mcp.as_ref().and_then(|m|m.tool.as_ref()).map(|t|&t.name),
        "input_schema":connection.input_schema,"output_schema":connection.output_schema,
        "metering":connection.metering,"credentials":"owner_managed"})
}

fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(RuntimeError::InvalidInput("invalid connection name".into()));
    }
    Ok(())
}

pub(crate) fn publish(
    tx: &Transaction<'_>,
    assignment: &str,
    name: &str,
    revision: u64,
    payload: &str,
) -> Result<Value> {
    // Publishing a derived project binding must preserve its original identity.
    // Otherwise a duplicate entry would escape a library/project revocation.
    let inherited: Option<(String,u64,bool)> = tx.query_row(
        "SELECT l.id,l.revision,l.enabled FROM connection_library_uses u JOIN connection_library l ON l.id=u.library_id WHERE u.assignment_id=?1 AND u.name=?2 AND u.binding_revision=?3",
        params![assignment,name,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if let Some((id, library_revision, enabled)) = inherited {
        if !enabled {
            return Err(RuntimeError::State(
                "saved tool was disabled; review a new connection".into(),
            ));
        }
        return Ok(
            json!({"library_id":id,"revision":library_revision,"availability":"all_projects"}),
        );
    }
    let prior: Option<(String,u64,bool)> = tx.query_row(
        "SELECT id,revision,enabled FROM connection_library WHERE source_assignment=?1 AND source_name=?2 AND source_revision=?3",
        params![assignment,name,revision], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if let Some((id, library_revision, enabled)) = prior {
        if !enabled {
            return Err(RuntimeError::State(
                "saved tool was disabled; review a new connection".into(),
            ));
        }
        return Ok(
            json!({"library_id":id,"revision":library_revision,"availability":"all_projects"}),
        );
    }
    let id = format!("tool-{}", uuid::Uuid::new_v4().simple());
    tx.execute("INSERT INTO connection_library(id,name,revision,enabled,payload,source_assignment,source_name,source_revision,created_at) VALUES(?1,?2,1,1,?3,?4,?2,?5,?6)",params![id,name,payload,assignment,revision,now_ms()])?;
    tx.execute("INSERT INTO connection_library_uses(assignment_id,name,library_id,binding_revision) VALUES(?1,?2,?3,?4) ON CONFLICT(assignment_id,name) DO UPDATE SET library_id=excluded.library_id,binding_revision=excluded.binding_revision",params![assignment,name,id,revision])?;
    Ok(json!({"library_id":id,"revision":1,"availability":"all_projects"}))
}

impl Runtime {
    /// An exception candidate is deliberately narrower than a generic account
    /// match. The owner still has to approve the exact old/new contract digest.
    pub(crate) fn reviewed_account_candidate(
        &self,
        account_id: &str,
        revision: u64,
        target: &Connection,
    ) -> Result<(String, Value)> {
        use apr_connectors::mcp::Transport;
        let row: Option<(String, String)> = self.db.query_row(
            "SELECT credential_ref,contract FROM saved_accounts WHERE id=?1 AND revision=?2 AND enabled=1",
            params![account_id, revision],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        let (reference, encoded) = row
            .ok_or_else(|| RuntimeError::State("saved account changed or is unavailable".into()))?;
        let old: Value = serde_json::from_str(&encoded)?;
        let new = account_contract(target)?;
        let Some(Transport::Stdio {
            executable,
            credential_env: Some(env),
            ..
        }) = target.mcp.as_ref().map(|m| &m.transport)
        else {
            return Err(RuntimeError::InvalidInput(
                "reviewed reuse requires a credentialed native stdio connection".into(),
            ));
        };
        if !matches!(target.auth, Auth::Bearer { .. } | Auth::Header { .. })
            || old["endpoint"] != new["endpoint"]
            || old["auth"] != new["auth"]
            || old["transport"]["kind"] != "stdio"
            || old["transport"]["executable"] != executable.as_str()
            || old["transport"]["credential_env"] != env.as_str()
        {
            return Err(RuntimeError::InvalidInput(
                "saved account provider, command path or credential contract differs".into(),
            ));
        }
        Ok((reference, old))
    }
    /// Trusted owner sign-in may save an account before approving any operation.
    /// This does not create an execution grant or publish a tool.
    pub fn connect_account(&mut self, connection: &Connection, secret: Secret) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        connection
            .validate()
            .map_err(|_| RuntimeError::InvalidInput("invalid account connection".into()))?;
        match &connection.auth {
            Auth::Oauth {
                credential_ref,
                config,
            } if credential_ref == "pending" => {
                apr_connectors::oauth::validate_bundle(config, &secret).map_err(|_| {
                    RuntimeError::State(
                        "OAuth authorization is missing, expired or for another account".into(),
                    )
                })?;
            }
            Auth::Bearer { credential_ref } | Auth::Header { credential_ref, .. }
                if credential_ref == "pending" => {}
            _ => {
                return Err(RuntimeError::InvalidInput(
                    "account setup requires pending authentication".into(),
                ));
            }
        }
        let reference = format!("credential-{}", uuid::Uuid::new_v4().simple());
        let mut bound = connection.clone();
        match &mut bound.auth {
            Auth::Bearer { credential_ref }
            | Auth::Header { credential_ref, .. }
            | Auth::Oauth { credential_ref, .. } => *credential_ref = reference.clone(),
            Auth::None => unreachable!("checked above"),
        }
        let vault = self.vault()?;
        vault.set(&reference, secret).map_err(|_| {
            RuntimeError::State("private credential file unavailable; check storage permissions or import saved keys in Settings".into())
        })?;
        let result = (|| -> Result<Value> {
            let tx = self.db.transaction()?;
            let account = register_account(&tx, &bound)?.ok_or_else(|| {
                RuntimeError::Integrity("account registration lost authentication".into())
            })?;
            tx.commit()?;
            Ok(account)
        })();
        if result.is_err() {
            let _ = vault.remove(&reference);
        }
        result
    }

    /// Owner-only, bounded account inventory. Credential references and OAuth
    /// tokens never cross this boundary.
    pub fn saved_accounts(&self, after: Option<&str>) -> Result<Value> {
        self.saved_accounts_for(after, None)
    }
    /// Compatibility is computed by the same contract used for approval; the UI
    /// never guesses whether two account configurations are interchangeable.
    pub fn saved_accounts_for(
        &self,
        after: Option<&str>,
        connection: Option<&Connection>,
    ) -> Result<Value> {
        let expected = connection.map(account_contract).transpose()?;
        let mut stmt=self.db.prepare("SELECT id,revision,enabled,label,contract FROM saved_accounts WHERE id>?1 ORDER BY id LIMIT 21")?;
        let mut accounts = Vec::new();
        let mut next = None;
        for row in stmt.query_map([after.unwrap_or("")], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })? {
            let (id, revision, enabled, label, contract) = row?;
            if accounts.len() == 20 {
                next = accounts
                    .last()
                    .and_then(|v: &Value| v["account_id"].as_str())
                    .map(str::to_owned);
                break;
            }
            let contract: Value = serde_json::from_str(&contract)?;
            let compatible = expected.as_ref().map(|c| c == &contract);
            let review_required = enabled
                && compatible == Some(false)
                && connection
                    .is_some_and(|c| self.reviewed_account_candidate(&id, revision, c).is_ok());
            accounts.push(json!({"account_id":id,"revision":revision,"enabled":enabled,"label":label,
                "endpoint":contract["endpoint"],"kind":if contract["transport"].is_null(){"http.json"}else{"mcp"},
                "auth_kind":contract["auth"]["kind"],"compatible":compatible,"review_required":review_required}));
        }
        Ok(json!({"accounts":accounts,"next_cursor":next}))
    }

    pub(crate) fn reuse_saved_account(
        &self,
        account_id: &str,
        revision: u64,
        target: &mut Connection,
    ) -> Result<Value> {
        let row:Option<(String,String)>=self.db.query_row(
            "SELECT credential_ref,contract FROM saved_accounts WHERE id=?1 AND revision=?2 AND enabled=1",
            params![account_id,revision],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (reference, encoded) = row
            .ok_or_else(|| RuntimeError::State("saved account changed or is unavailable".into()))?;
        let contract: Value = serde_json::from_str(&encoded)?;
        if account_contract(target)? != contract {
            return Err(RuntimeError::InvalidInput(
                "saved account destination, transport or authentication does not match".into(),
            ));
        }
        match &mut target.auth {
            Auth::Bearer { credential_ref }
            | Auth::Header { credential_ref, .. }
            | Auth::Oauth { credential_ref, .. } => *credential_ref = reference,
            Auth::None => {
                return Err(RuntimeError::InvalidInput(
                    "connection has no account".into(),
                ));
            }
        }
        Ok(json!({"account_id":account_id,"revision":revision}))
    }

    pub fn revoke_saved_account(
        &mut self,
        account_id: &str,
        reviewed_revision: u64,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        let tx = self.db.transaction()?;
        let reference:Option<String>=tx.query_row("SELECT credential_ref FROM saved_accounts WHERE id=?1 AND revision=?2 AND enabled=1",params![account_id,reviewed_revision],|r|r.get(0)).optional()?;
        let reference = reference
            .ok_or_else(|| RuntimeError::State("saved account changed or is unavailable".into()))?;
        tx.execute(
            "UPDATE saved_accounts SET enabled=0,revision=revision+1 WHERE id=?1",
            [account_id],
        )?;
        let disabled_tools=tx.execute("UPDATE connection_library SET enabled=0,revision=revision+1 WHERE enabled=1 AND json_extract(payload,'$.auth.credential_ref')=?1",[&reference])?;
        let disabled_bindings=tx.execute("UPDATE execution_grants SET enabled=0,revision=revision+1 WHERE kind='connection' AND enabled=1 AND json_extract(payload,'$.auth.credential_ref')=?1",[&reference])?;
        tx.commit()?;
        // Revocation stands even if local secret cleanup fails. Report cleanup
        // separately so an unavailable/corrupt file does not imply deletion.
        let credential_removed = self
            .vault()
            .is_ok_and(|vault| vault.remove(&reference).is_ok());
        Ok(
            json!({"account_id":account_id,"enabled":false,"disabled_bindings":disabled_bindings,"disabled_tools":disabled_tools,"credential_removed":credential_removed}),
        )
    }
    /// Bounded owner-only review of current workspace connections. Stored key
    /// references are replaced; credential contents are never retrieved.
    pub fn execution_bindings(&self, assignment: &str) -> Result<Value> {
        self.ensure_assignment_active(assignment)?;
        let mut stmt=self.db.prepare("SELECT name,revision,payload FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND enabled=1 ORDER BY name LIMIT 21")?;
        let mut items = Vec::new();
        let mut size = 0;
        let mut truncated = false;
        for row in stmt.query_map([assignment], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (name, revision, payload) = row?;
            let mut c: Connection = serde_json::from_str(&payload)?;
            match &mut c.auth {
                Auth::Bearer { credential_ref }
                | Auth::Header { credential_ref, .. }
                | Auth::Oauth { credential_ref, .. } => *credential_ref = "stored".into(),
                Auth::None => {}
            }
            let item = json!({"name":name,"revision":revision,"connection":c});
            let bytes = serde_json::to_vec(&item)?.len();
            if items.len() == 20 || size + bytes > 48 * 1024 {
                truncated = true;
                break;
            }
            size += bytes;
            items.push(item);
        }
        Ok(json!({"bindings":items,"truncated":truncated}))
    }
    /// Promote an already approved tool without retrieving or duplicating its key.
    /// Existing workspace grants are never promoted automatically on upgrade.
    pub fn save_connection_to_library(
        &mut self,
        assignment: &str,
        name: &str,
        reviewed_revision: u64,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        self.ensure_assignment_active(assignment)?;
        let payload: Option<String> = self.db.query_row("SELECT payload FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=?2 AND revision=?3 AND enabled=1",params![assignment,name,reviewed_revision],|r|r.get(0)).optional()?;
        let payload = payload.ok_or_else(|| {
            RuntimeError::State("connection changed; review its current revision".into())
        })?;
        let tx = self.db.transaction()?;
        let result = publish(&tx, assignment, name, reviewed_revision, &payload)?;
        tx.commit()?;
        Ok(result)
    }

    /// Bounded public catalog. Exact contracts are fetched individually on demand.
    pub fn connection_library(&self, after: Option<&str>, owner: bool) -> Result<Value> {
        let mut stmt=self.db.prepare("SELECT id,name,revision,enabled,payload FROM connection_library WHERE id>?1 AND (?2 OR enabled=1) ORDER BY id LIMIT 21")?;
        let mut items = Vec::new();
        let mut next = None;
        for row in stmt.query_map(params![after.unwrap_or(""), owner], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, String>(4)?,
            ))
        })? {
            let (id, name, revision, enabled, payload) = row?;
            if items.len() == 20 {
                next = items
                    .last()
                    .and_then(|x: &Value| x["library_id"].as_str())
                    .map(str::to_owned);
                break;
            }
            let c: Connection = serde_json::from_str(&payload)?;
            let mut item = json!({"library_id":id,"name":name,"label":c.label,"revision":revision,"enabled":enabled,"availability":"all_projects","kind":if c.mcp.is_some(){"mcp"}else{"http.json"},"tool":c.mcp.as_ref().and_then(|m|m.tool.as_ref()).map(|t|&t.name),"authenticated":!matches!(c.auth,Auth::None)});
            if owner {
                item["endpoint"] = json!(c.endpoint);
                item["auth_kind"] = json!(match c.auth {
                    Auth::None => "none",
                    Auth::Bearer { .. } => "bearer",
                    Auth::Header { .. } => "header",
                    Auth::Oauth { .. } => "oauth",
                });
            }
            items.push(item);
        }
        Ok(json!({"connections":items,"next_cursor":next,"credentials":"owner_managed"}))
    }

    pub(crate) fn saved_connection(&self, id: &str, revision: u64) -> Result<Connection> {
        let payload: Option<String> = self
            .db
            .query_row(
                "SELECT payload FROM connection_library WHERE id=?1 AND revision=?2 AND enabled=1",
                params![id, revision],
                |r| r.get(0),
            )
            .optional()?;
        let connection: Connection = serde_json::from_str(&payload.ok_or_else(|| {
            RuntimeError::State("saved tool changed or is unavailable; refresh the library".into())
        })?)?;
        if let Some(reference) = credential_ref(&connection) {
            let active: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM saved_accounts WHERE credential_ref=?1 AND enabled=1)",
                [reference],
                |r| r.get(0),
            )?;
            if !active {
                return Err(RuntimeError::State("saved account is unavailable".into()));
            }
        }
        Ok(connection)
    }

    pub(crate) fn library_contract(&self, id: &str, revision: u64) -> Result<Value> {
        let c = self.saved_connection(id, revision)?;
        let mut result = public_contract(&c);
        result["library_id"] = json!(id);
        result["revision"] = json!(revision);
        result["availability"] = json!("all_projects");
        Ok(result)
    }

    /// Called under scoped_call's store lock. The owner already authorized this
    /// exact tool for all projects. No caller-supplied endpoint or auth is accepted.
    pub(crate) fn use_library_connection(
        &mut self,
        assignment: &str,
        id: &str,
        revision: u64,
        name: &str,
    ) -> Result<Value> {
        valid_name(name)?;
        self.ensure_assignment_active(assignment)?;
        let blocked:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM connection_library_blocks WHERE assignment_id=?1 AND library_id=?2)",params![assignment,id],|r|r.get(0))?;
        if blocked {
            return Err(RuntimeError::Forbidden);
        }
        let c = self.saved_connection(id, revision)?;
        let existing:Option<(u64,bool,String)>=self.db.query_row("SELECT revision,enabled,payload FROM execution_grants WHERE assignment_id=?1 AND kind='connection' AND name=?2",params![assignment,name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((binding_revision, enabled, payload)) = existing {
            let matches:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM connection_library_uses WHERE assignment_id=?1 AND name=?2 AND library_id=?3 AND binding_revision=?4)",params![assignment,name,id,binding_revision],|r|r.get(0))?;
            if enabled
                && matches
                && serde_json::from_str::<Value>(&payload)? == serde_json::to_value(&c)?
            {
                return Ok(
                    json!({"name":name,"library_id":id,"revision":binding_revision,"reused":true}),
                );
            }
            return Err(RuntimeError::State(
                "binding name already exists; use a new name or ask the owner to review it".into(),
            ));
        }
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO execution_grants(assignment_id,kind,name,revision,enabled,payload) VALUES(?1,'connection',?2,1,1,?3)",params![assignment,name,serde_json::to_string(&c)?])?;
        tx.execute("INSERT INTO connection_library_uses(assignment_id,name,library_id,binding_revision) VALUES(?1,?2,?3,1)",params![assignment,name,id])?;
        tx.commit()?;
        Ok(json!({"name":name,"library_id":id,"revision":1,"reused":false}))
    }

    /// Disabling a saved tool invalidates every unchanged derived grant, including
    /// frozen runs at their next dispatch. Unrelated owner replacements survive.
    pub fn disable_library_connection(
        &mut self,
        id: &str,
        reviewed_revision: u64,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        self.saved_connection(id, reviewed_revision)?;
        let tx = self.db.transaction()?;
        tx.execute("UPDATE connection_library SET enabled=0,revision=revision+1 WHERE id=?1 AND revision=?2",params![id,reviewed_revision])?;
        let disabled=tx.execute("UPDATE execution_grants SET enabled=0,revision=revision+1 WHERE kind='connection' AND enabled=1 AND EXISTS(SELECT 1 FROM connection_library_uses u WHERE u.assignment_id=execution_grants.assignment_id AND u.name=execution_grants.name AND u.binding_revision=execution_grants.revision AND u.library_id=?1)",[id])?;
        tx.commit()?;
        Ok(json!({"library_id":id,"enabled":false,"disabled_bindings":disabled}))
    }
}

/// A saved credential can authorize another reviewed operation only at the exact
/// same destination/transport and authentication contract. An API key is never a
/// generic credential for arbitrary endpoints, commands, or OAuth audiences.
pub(crate) fn reuse_auth(target: &mut Connection, source: &Connection) -> Result<()> {
    let transport = |c: &Connection| {
        c.mcp
            .as_ref()
            .map(|m| serde_json::to_value(&m.transport))
            .transpose()
    };
    if target.endpoint != source.endpoint || transport(target)? != transport(source)? {
        return Err(RuntimeError::InvalidInput(
            "saved credential belongs to another destination or command".into(),
        ));
    }
    let compatible = match (&target.auth, &source.auth) {
        (Auth::Bearer { .. }, Auth::Bearer { .. }) => true,
        (Auth::Header { name: a, .. }, Auth::Header { name: b, .. }) => a.eq_ignore_ascii_case(b),
        (Auth::Oauth { config: a, .. }, Auth::Oauth { config: b, .. }) => {
            serde_json::to_value(a)? == serde_json::to_value(b)?
        }
        _ => false,
    };
    if !compatible {
        return Err(RuntimeError::InvalidInput(
            "saved credential authentication does not match this request".into(),
        ));
    }
    target.auth = source.auth.clone();
    Ok(())
}
