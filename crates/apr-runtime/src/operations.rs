//! Owner-only maintenance and public price-source review. Automated checks can
//! propose changed rates, never silently rewrite authorization or historical runs.
use crate::store::{StoreLock, now_ms};
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::{Auth, Connection, CredentialStore};
use apr_core::content_digest;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const MIGRATION_V14: &str = r#"
CREATE TABLE runtime_maintenance(singleton INTEGER PRIMARY KEY CHECK(singleton=1),paused INTEGER NOT NULL,target_version TEXT,created_at INTEGER);
INSERT INTO runtime_maintenance(singleton,paused) VALUES(1,0);
CREATE TABLE desktop_preferences(singleton INTEGER PRIMARY KEY CHECK(singleton=1),check_updates INTEGER NOT NULL DEFAULT 0);
INSERT INTO desktop_preferences(singleton,check_updates) VALUES(1,0);
CREATE TABLE price_sources(assignment_id TEXT NOT NULL,name TEXT NOT NULL,config TEXT NOT NULL,enabled INTEGER NOT NULL,next_check_at INTEGER NOT NULL,last_status TEXT,last_digest TEXT,PRIMARY KEY(assignment_id,name));
CREATE TABLE price_reviews(request_id TEXT PRIMARY KEY,assignment_id TEXT NOT NULL,name TEXT NOT NULL,source_digest TEXT NOT NULL);
"#;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceSource {
    pub endpoint: String,
    pub pointer: String,
    pub interval_seconds: u32,
}
impl PriceSource {
    fn validate(&self) -> Result<()> {
        apr_connectors::oauth::public_url(&self.endpoint, false).map_err(|_| {
            RuntimeError::InvalidInput(
                "Price source must be public HTTPS without credentials".into(),
            )
        })?;
        if !(3600..=2_592_000).contains(&self.interval_seconds)
            || (!self.pointer.is_empty() && !self.pointer.starts_with('/'))
            || self.pointer.len() > 1024
        {
            return Err(RuntimeError::InvalidInput(
                "Invalid price source interval or JSON pointer".into(),
            ));
        }
        Ok(())
    }
}
impl Runtime {
    pub fn update_preferences(&self) -> Result<Value> {
        let enabled: bool = self.db.query_row(
            "SELECT check_updates FROM desktop_preferences WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        Ok(json!({"automatic_checks":enabled,"installation_requires_approval":true}))
    }
    pub fn set_update_preferences(&mut self, enabled: bool) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.db.execute(
            "UPDATE desktop_preferences SET check_updates=?1 WHERE singleton=1",
            [enabled],
        )?;
        self.update_preferences()
    }

    pub fn maintenance_status(&self) -> Result<Value> {
        let (paused, target): (bool, Option<String>) = self.db.query_row(
            "SELECT paused,target_version FROM runtime_maintenance WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(json!({"paused_for_update":paused,"target_version":target}))
    }
    pub(crate) fn ensure_not_maintaining(&self) -> Result<()> {
        if self.db.query_row(
            "SELECT paused FROM runtime_maintenance WHERE singleton=1",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(RuntimeError::State(
                "Runtime paused for a desktop update; finish or cancel maintenance in Setup".into(),
            ));
        }
        Ok(())
    }
    /// A persisted gate closes scheduling and scoped mutation before installer
    /// replacement. An interrupted update stays visibly paused until owner action.
    pub fn prepare_desktop_update(&mut self, target: &str) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        if target.is_empty()
            || target.len() > 64
            || !target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        {
            return Err(RuntimeError::InvalidInput("Invalid update version".into()));
        }
        let active:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE state NOT IN ('succeeded','failed','cancelled'))",[],|r|r.get(0))?;
        if active {
            return Err(RuntimeError::State(
                "Complete or cancel active recipes before installing this update".into(),
            ));
        }
        self.db.execute("UPDATE runtime_maintenance SET paused=1,target_version=?1,created_at=?2 WHERE singleton=1",params![target,now_ms()])?;
        Ok(json!({"paused_for_update":true,"target_version":target}))
    }
    pub fn finish_desktop_update(&mut self) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.db.execute(
            "UPDATE runtime_maintenance SET paused=0,target_version=NULL WHERE singleton=1",
            [],
        )?;
        Ok(json!({"paused_for_update":false}))
    }
    pub fn set_price_source(
        &mut self,
        assignment: &str,
        name: &str,
        source: PriceSource,
        enabled: bool,
    ) -> Result<Value> {
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        self.ensure_assignment_active(assignment)?;
        source.validate()?;
        let payload:String=self.db.query_row("SELECT payload FROM execution_grants WHERE assignment_id=?1 AND name=?2 AND kind='connection' AND enabled=1",params![assignment,name],|r|r.get(0))?;
        let cfg: Connection = serde_json::from_str(&payload)?;
        if cfg.metering.is_none() {
            return Err(RuntimeError::State(
                "Ask your agent to analyze the connection's payloads before adding a price source"
                    .into(),
            ));
        }
        self.db.execute("INSERT INTO price_sources(assignment_id,name,config,enabled,next_check_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(assignment_id,name) DO UPDATE SET config=excluded.config,enabled=excluded.enabled,next_check_at=excluded.next_check_at",params![assignment,name,serde_json::to_string(&source)?,enabled,now_ms()])?;
        Ok(json!({"saved":true,"automatic":"check_and_propose_only","enabled":enabled}))
    }
    pub fn price_sources(&self, assignment: &str) -> Result<Value> {
        let mut q=self.db.prepare("SELECT name,config,enabled,next_check_at,last_status FROM price_sources WHERE assignment_id=?1 ORDER BY name LIMIT 101")?;
        let mut rows = Vec::new();
        for r in q.query_map([assignment], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })? {
            let (name, cfg, enabled, next, status) = r?;
            rows.push(json!({"name":name,"source":serde_json::from_str::<Value>(&cfg)?,"enabled":enabled,"next_check_at":next,"last_status":status}));
        }
        let truncated = rows.len() > 100;
        rows.truncate(100);
        Ok(json!({"items":rows,"truncated":truncated,"rate_changes_require_review":true}))
    }
    pub fn check_price_source(&mut self, assignment: &str, name: &str) -> Result<Value> {
        // Fetch outside the writer lock; then compare the precise prior revision.
        let (source,payload,revision):(String,String,u64)=self.db.query_row("SELECT p.config,g.payload,g.revision FROM price_sources p JOIN execution_grants g ON g.assignment_id=p.assignment_id AND g.name=p.name AND g.kind='connection' WHERE p.assignment_id=?1 AND p.name=?2 AND p.enabled=1 AND g.enabled=1",params![assignment,name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let source_json = source.clone();
        let source: PriceSource = serde_json::from_str(&source)?;
        source.validate()?;
        let fetched = apr_connectors::oauth::read_public_json(&source.endpoint).map_err(|_| {
            RuntimeError::State("Public price source is unavailable or invalid".into())
        })?;
        let mut config: Connection = serde_json::from_str(&payload)?;
        let rates = fetched.pointer(&source.pointer).ok_or_else(|| {
            RuntimeError::InvalidInput(
                "Price source does not contain the reviewed rate object".into(),
            )
        })?;
        let digest = content_digest(rates)?;
        let current = config
            .metering
            .as_ref()
            .ok_or_else(|| RuntimeError::State("Connection has no costing model".into()))?;
        let mut meter = serde_json::to_value(current)?;
        let before = meter.clone();
        for line in meter["meters"]
            .as_array_mut()
            .ok_or_else(|| RuntimeError::Integrity("Invalid saved meters".into()))?
        {
            let key = line["name"]
                .as_str()
                .ok_or_else(|| RuntimeError::Integrity("Invalid meter name".into()))?;
            let rate = rates.get(key).ok_or_else(|| {
                RuntimeError::InvalidInput("Price source is missing a configured meter".into())
            })?;
            line["rate"] = rate.clone();
        }
        let _lock = StoreLock::acquire(&self.root)?;
        self.ensure_not_maintaining()?;
        self.ensure_assignment_active(assignment)?;
        let actual:u64=self.db.query_row("SELECT revision FROM execution_grants WHERE assignment_id=?1 AND name=?2 AND kind='connection' AND enabled=1",params![assignment,name],|r|r.get(0))?;
        let still_current:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM price_sources WHERE assignment_id=?1 AND name=?2 AND enabled=1 AND config=?3)",params![assignment,name,source_json],|r|r.get(0))?;
        if actual != revision || !still_current {
            return Err(RuntimeError::State(
                "Connection changed during price check; check again".into(),
            ));
        }
        if before == meter {
            self.db.execute("UPDATE price_sources SET last_status='unchanged',last_digest=?3,next_check_at=?4 WHERE assignment_id=?1 AND name=?2",params![assignment,name,digest,now_ms()+i64::from(source.interval_seconds)*1000])?;
            return Ok(json!({"changed":false}));
        }
        meter["revision"] = json!(format!("feed-{}", &digest[7..23]));
        let new = serde_json::from_value::<apr_connectors::metering::MeteringConfig>(meter)?;
        new.validate().map_err(|_| {
            RuntimeError::InvalidInput("Price source contains invalid rates".into())
        })?;
        config.metering = Some(new);
        match &mut config.auth {
            Auth::None => {}
            Auth::Bearer { credential_ref }
            | Auth::Header { credential_ref, .. }
            | Auth::Oauth { credential_ref, .. } => *credential_ref = "pending".into(),
        }
        let proposal = crate::setup::ConnectionProposal {
            name: name.into(),
            request_key: format!("price-{revision}-{}", &digest[7..]),
            rationale: format!(
                "Review changed rates from the approved public price source {}. Other permissions and payload rules are unchanged.",
                source.endpoint
            ),
            connection: config,
        };
        let result = self.propose_connection_inner(assignment, proposal)?;
        self.db.execute("INSERT OR IGNORE INTO price_reviews(request_id,assignment_id,name,source_digest) VALUES(?1,?2,?3,?4)",params![result["request_id"].as_str(),assignment,name,digest])?;
        self.db.execute("UPDATE price_sources SET last_status='review_required',last_digest=?3,next_check_at=?4 WHERE assignment_id=?1 AND name=?2",params![assignment,name,digest,now_ms()+i64::from(source.interval_seconds)*1000])?;
        Ok(json!({"changed":true,"proposal":result,"applied":false}))
    }
    pub fn approve_price_review(&mut self, id: &str, digest: &str) -> Result<Value> {
        let secret = {
            let _lock = StoreLock::acquire(&self.root)?;
            let (old,new):(String,String)=self.db.query_row("SELECT g.payload,r.payload FROM price_reviews p JOIN connection_requests r ON r.id=p.request_id JOIN execution_grants g ON g.assignment_id=p.assignment_id AND g.name=p.name AND g.kind='connection' WHERE p.request_id=?1 AND r.digest=?2 AND r.state='pending' AND g.enabled=1 AND g.revision=r.expected_revision",params![id,digest],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let old: Connection = serde_json::from_str(&old)?;
            let mut sanitized = serde_json::to_value(&old)?;
            let mut new: Value = serde_json::from_str(&new)?;
            for v in [&mut sanitized, &mut new] {
                v.as_object_mut().unwrap().remove("metering");
                if v["auth"].get("credential_ref").is_some() {
                    v["auth"]["credential_ref"] = json!("pending");
                }
            }
            if sanitized != new {
                return Err(RuntimeError::State(
                    "This request changes more than rates; use full connection review".into(),
                ));
            }
            let vault = self.vault()?;
            match old.auth {
                Auth::None => None,
                Auth::Bearer { credential_ref }
                | Auth::Header { credential_ref, .. }
                | Auth::Oauth { credential_ref, .. } => {
                    Some(vault.get(&credential_ref).map_err(|_| {
                        RuntimeError::State("Reconnect this account before updating rates".into())
                    })?)
                }
            }
        };
        self.approve_connection(id, digest, secret)
    }
    /// At most one due public metadata read per tick; failures back off for the
    /// configured interval and never dispatch a recipe or approve a proposal.
    pub fn tick_price_sources(&mut self) -> Result<bool> {
        let due: Option<(String, String, String)> = {
            let _lock = StoreLock::acquire(&self.root)?;
            if self.ensure_not_maintaining().is_err() {
                return Ok(false);
            }
            let row=self.db.query_row("SELECT p.assignment_id,p.name,p.config FROM price_sources p JOIN assignments a ON a.id=p.assignment_id WHERE p.enabled=1 AND p.next_check_at<=?1 AND a.state='active' ORDER BY p.next_check_at LIMIT 1",[now_ms()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
            if let Some((assignment, name, source)) = &row {
                let source: PriceSource = serde_json::from_str(source)?;
                self.db.execute("UPDATE price_sources SET next_check_at=?3,last_status='checking' WHERE assignment_id=?1 AND name=?2",params![assignment,name,now_ms()+i64::from(source.interval_seconds)*1000])?;
            }
            row
        };
        let Some((assignment, name, _)) = due else {
            return Ok(false);
        };
        if self.check_price_source(&assignment, &name).is_err() {
            let _lock = StoreLock::acquire(&self.root)?;
            self.db.execute("UPDATE price_sources SET last_status='unavailable_or_invalid' WHERE assignment_id=?1 AND name=?2",params![assignment,name])?;
        }
        Ok(true)
    }
}
