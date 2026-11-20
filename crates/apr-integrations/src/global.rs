//! One-time, user-scoped APR client installation.
//!
//! The state directory owns records, snapshots and an operation journal.  The
//! home directory is always opened through `Root`, so configuration traversal
//! is anchored and never follows a user-controlled symlink.

use crate::{Client, Error, Result, config::codex_entry_digest, digest, files::Root};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Table, value};

const SKILL: &str = include_str!("../../../integrations/skills/apr/SKILL.md");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalPlan {
    pub format_version: u32,
    pub state_dir: PathBuf,
    pub home: PathBuf,
    pub executable: PathBuf,
    pub client: Client,
    /// Explicit owner-reviewed Codex policy for the APR run tool only.
    #[serde(default)]
    pub trust_apr_run: bool,
    pub installation_id: String,
    pub plan_id: String,
    pub config_path: PathBuf,
    pub skill_path: PathBuf,
    pub server_entry: Value,
    pub expected_config: Option<String>,
    pub expected_skill: Option<String>,
    pub expected_record: Option<String>,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalReport {
    pub installation_id: String,
    pub state: String,
    pub changed: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format_version: u32,
    home: PathBuf,
    executable: PathBuf,
    client: Client,
    config_created: bool,
    entry_hash: String,
    skill_hash: String,
    last_plan_id: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Slot {
    Config,
    Skill,
    Record,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacement {
    slot: Slot,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format_version: u32,
    installation_id: String,
    home: PathBuf,
    client: Client,
    operation: String,
    plan_id: String,
    files: Vec<Replacement>,
}

pub struct GlobalManager {
    state: Root,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalInstallOptions {
    pub trust_apr_run: bool,
}

impl GlobalManager {
    pub fn open(state_dir: &Path) -> Result<Self> {
        Ok(Self {
            state: Root::open(state_dir, true)?,
        })
    }

    pub fn preview(&self, home: &Path, executable: &Path, client: Client) -> Result<GlobalPlan> {
        self.preview_with_options(home, executable, client, GlobalInstallOptions::default())
    }

    pub fn preview_with_options(
        &self,
        home: &Path,
        executable: &Path,
        client: Client,
        options: GlobalInstallOptions,
    ) -> Result<GlobalPlan> {
        if options.trust_apr_run && client != Client::Codex {
            return Err(Error::InvalidInput("codex_trust_requires_codex"));
        }
        let home = Root::open(home, false)?;
        let executable = validate_executable(executable)?;
        let installation_id = identity(&home.path, client);
        if self.pending(&installation_id)?.is_some() {
            return Err(Error::Conflict("operation_pending"));
        }
        let record_bytes = self.state.read(&record_path(&installation_id))?;
        let record = parse_record(record_bytes.as_deref())?;
        if let Some(record) = &record
            && (record.home != home.path || record.client != client)
        {
            return Err(Error::Conflict("invalid_ownership_record"));
        }
        let config = home.read(config_path(client))?;
        let skill = home.read(skill_path(client))?;
        let edited = install_config(
            client,
            &executable,
            &self.state.path,
            config.as_deref(),
            record.as_ref().map(|r| r.entry_hash.as_str()),
            options,
        )?;
        if skill.as_deref().map(digest).as_deref() != record.as_ref().map(|r| r.skill_hash.as_str())
        {
            return Err(Error::Conflict(if record.is_some() {
                "owned_skill_edited"
            } else {
                "unowned_skill"
            }));
        }
        let mut plan = GlobalPlan {
            format_version: 1,
            state_dir: self.state.path.clone(),
            home: home.path.clone(),
            executable: executable.clone(),
            client,
            trust_apr_run: options.trust_apr_run,
            installation_id,
            plan_id: String::new(),
            config_path: home.path.join(config_path(client)),
            skill_path: home.path.join(skill_path(client)),
            server_entry: installed_entry(client, &executable, &self.state.path, options),
            expected_config: config.as_deref().map(digest),
            expected_skill: skill.as_deref().map(digest),
            expected_record: record_bytes.as_deref().map(digest),
            changes: Vec::new(),
        };
        if config.as_deref() != Some(edited.bytes.as_slice()) {
            plan.changes.push("write_config".into());
        }
        if skill.as_deref() != Some(SKILL.as_bytes()) {
            plan.changes.push("write_skill".into());
        }
        plan.plan_id = plan_digest(&plan)?;
        Ok(plan)
    }

    pub fn apply(&self, plan: &GlobalPlan) -> Result<GlobalReport> {
        if plan.format_version != 1
            || plan.state_dir != self.state.path
            || plan.plan_id != plan_digest(plan)?
            || plan.installation_id != identity(&plan.home, plan.client)
        {
            return Err(Error::InvalidInput("invalid_plan"));
        }
        let _lock = self.state.lock(&lock_path(&plan.installation_id))?;
        if let Some(journal) = self.pending(&plan.installation_id)? {
            if journal.operation != "install" || journal.plan_id != plan.plan_id {
                return Err(Error::Conflict("operation_pending"));
            }
            return self.finish(&journal);
        }
        let options = GlobalInstallOptions {
            trust_apr_run: plan.trust_apr_run,
        };
        let fresh =
            self.preview_with_options(&plan.home, &plan.executable, plan.client, options)?;
        let existing = parse_record(
            self.state
                .read(&record_path(&plan.installation_id))?
                .as_deref(),
        )?;
        if existing
            .as_ref()
            .is_some_and(|r| r.last_plan_id == plan.plan_id)
            || (fresh.plan_id == plan.plan_id && fresh.changes.is_empty() && existing.is_some())
        {
            return Ok(GlobalReport {
                installation_id: plan.installation_id.clone(),
                state: "installed".into(),
                changed: false,
            });
        }
        if fresh.plan_id != plan.plan_id {
            return Err(Error::Conflict("stale_preview"));
        }
        let home = Root::open(&plan.home, false)?;
        let before = home.read(config_path(plan.client))?;
        let edited = install_config(
            plan.client,
            &plan.executable,
            &self.state.path,
            before.as_deref(),
            existing.as_ref().map(|r| r.entry_hash.as_str()),
            options,
        )?;
        let skill = home.read(skill_path(plan.client))?;
        let record_before = self.state.read(&record_path(&plan.installation_id))?;
        let record = Record {
            format_version: 1,
            home: home.path.clone(),
            executable: plan.executable.clone(),
            client: plan.client,
            config_created: existing
                .map(|r| r.config_created)
                .unwrap_or(before.is_none()),
            entry_hash: edited.entry_hash,
            skill_hash: digest(SKILL.as_bytes()),
            last_plan_id: plan.plan_id.clone(),
        };
        let files = vec![
            self.replacement(
                &plan.installation_id,
                Slot::Config,
                before.as_deref(),
                Some(&edited.bytes),
            )?,
            self.replacement(
                &plan.installation_id,
                Slot::Skill,
                skill.as_deref(),
                Some(SKILL.as_bytes()),
            )?,
            self.replacement(
                &plan.installation_id,
                Slot::Record,
                record_before.as_deref(),
                Some(&json_bytes(&record)?),
            )?,
        ];
        self.commit(Journal {
            format_version: 1,
            installation_id: plan.installation_id.clone(),
            home: home.path,
            client: plan.client,
            operation: "install".into(),
            plan_id: plan.plan_id.clone(),
            files,
        })
    }

    pub fn uninstall(&self, home: &Path, client: Client) -> Result<GlobalReport> {
        let home = Root::open(home, false)?;
        let id = identity(&home.path, client);
        let _lock = self.state.lock(&lock_path(&id))?;
        if let Some(journal) = self.pending(&id)? {
            if journal.operation != "uninstall" {
                return Err(Error::Conflict("operation_pending"));
            }
            return self.finish(&journal);
        }
        let record_bytes = self.state.read(&record_path(&id))?;
        let Some(record) = parse_record(record_bytes.as_deref())? else {
            return Ok(GlobalReport {
                installation_id: id,
                state: "uninstalled".into(),
                changed: false,
            });
        };
        if record.home != home.path || record.client != client {
            return Err(Error::Conflict("invalid_ownership_record"));
        }
        let config = home
            .read(config_path(client))?
            .ok_or(Error::Conflict("owned_entry_edited"))?;
        let skill = home
            .read(skill_path(client))?
            .ok_or(Error::Conflict("owned_skill_edited"))?;
        if digest(&skill) != record.skill_hash {
            return Err(Error::Conflict("owned_skill_edited"));
        }
        let after = remove_config(client, &config, &record.entry_hash, record.config_created)?;
        let files = vec![
            self.replacement(&id, Slot::Config, Some(&config), after.as_deref())?,
            self.replacement(&id, Slot::Skill, Some(&skill), None)?,
            self.replacement(&id, Slot::Record, record_bytes.as_deref(), None)?,
        ];
        self.commit(Journal {
            format_version: 1,
            installation_id: id,
            home: home.path,
            client,
            operation: "uninstall".into(),
            plan_id: String::new(),
            files,
        })
    }

    fn snapshot(&self, id: &str, bytes: &[u8]) -> Result<String> {
        let hash = digest(bytes);
        let path = snapshot_path(id, &hash);
        if let Some(saved) = self.state.read(&path)? {
            if saved != bytes {
                return Err(Error::Conflict("backup_changed"));
            }
        } else {
            self.state.write(&path, None, Some(bytes))?;
        }
        Ok(hash)
    }
    fn replacement(
        &self,
        id: &str,
        slot: Slot,
        before: Option<&[u8]>,
        after: Option<&[u8]>,
    ) -> Result<Replacement> {
        Ok(Replacement {
            slot,
            before: before.map(|b| self.snapshot(id, b)).transpose()?,
            after: after.map(|b| self.snapshot(id, b)).transpose()?,
        })
    }
    fn pending(&self, id: &str) -> Result<Option<Journal>> {
        self.state
            .read(&journal_path(id))?
            .map(|bytes| {
                let j: Journal = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::Conflict("invalid_journal"))?;
                if j.format_version != 1
                    || j.installation_id != id
                    || identity(&j.home, j.client) != id
                    || !["install", "uninstall"].contains(&j.operation.as_str())
                    || j.files.iter().map(|f| f.slot).collect::<Vec<_>>()
                        != [Slot::Config, Slot::Skill, Slot::Record]
                    || j.files
                        .iter()
                        .flat_map(|f| [&f.before, &f.after])
                        .flatten()
                        .any(|h| !valid_hash(h))
                {
                    return Err(Error::Conflict("invalid_journal"));
                }
                Ok(j)
            })
            .transpose()
    }
    fn commit(&self, journal: Journal) -> Result<GlobalReport> {
        self.state.write(
            &journal_path(&journal.installation_id),
            None,
            Some(&json_bytes(&journal)?),
        )?;
        self.finish(&journal)
    }
    fn finish(&self, journal: &Journal) -> Result<GlobalReport> {
        let home = Root::open(&journal.home, false)?;
        for file in &journal.files {
            let (root, path) = self.target(&home, journal, file.slot);
            let current = root.read(&path)?.as_deref().map(digest);
            if current != file.before && current != file.after {
                return Err(Error::Conflict("recovery_target_edited"));
            }
            if let Some(hash) = &file.after {
                self.snapshot_bytes(&journal.installation_id, hash)?;
            }
        }
        for file in &journal.files {
            let (root, path) = self.target(&home, journal, file.slot);
            if root.read(&path)?.as_deref().map(digest) == file.after {
                continue;
            }
            let after = file
                .after
                .as_deref()
                .map(|h| self.snapshot_bytes(&journal.installation_id, h))
                .transpose()?;
            root.write(&path, file.before.as_deref(), after.as_deref())?;
        }
        let path = journal_path(&journal.installation_id);
        let current = self
            .state
            .read(&path)?
            .ok_or(Error::Conflict("journal_changed"))?;
        self.state.write(&path, Some(&digest(&current)), None)?;
        Ok(GlobalReport {
            installation_id: journal.installation_id.clone(),
            state: if journal.operation == "install" {
                "installed"
            } else {
                "uninstalled"
            }
            .into(),
            changed: true,
        })
    }
    fn target<'a>(&'a self, home: &'a Root, journal: &Journal, slot: Slot) -> (&'a Root, PathBuf) {
        match slot {
            Slot::Config => (home, config_path(journal.client).into()),
            Slot::Skill => (home, skill_path(journal.client).into()),
            Slot::Record => (&self.state, record_path(&journal.installation_id)),
        }
    }
    fn snapshot_bytes(&self, id: &str, hash: &str) -> Result<Vec<u8>> {
        if !valid_hash(hash) {
            return Err(Error::Conflict("invalid_backup_reference"));
        }
        let bytes = self
            .state
            .read(&snapshot_path(id, hash))?
            .ok_or(Error::Conflict("missing_backup"))?;
        if digest(&bytes) != hash {
            return Err(Error::Conflict("backup_changed"));
        }
        Ok(bytes)
    }
}

fn config_path(client: Client) -> &'static Path {
    Path::new(match client {
        Client::Codex => ".codex/config.toml",
        Client::Claude => ".claude.json",
    })
}
fn skill_path(client: Client) -> &'static Path {
    Path::new(match client {
        Client::Codex => ".agents/skills/apr/SKILL.md",
        Client::Claude => ".claude/skills/apr/SKILL.md",
    })
}
fn validate_executable(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    let meta = fs::metadata(&path)?;
    if !meta.is_file() || meta.mode() & 0o111 == 0 || path.to_str().is_none() {
        return Err(Error::InvalidInput("executable_unavailable"));
    }
    Ok(path)
}

struct Edited {
    bytes: Vec<u8>,
    entry_hash: String,
}
fn installed_entry(
    client: Client,
    executable: &Path,
    state: &Path,
    options: GlobalInstallOptions,
) -> Value {
    let mut v = json!({"command": executable, "args": ["--state-dir", state, "mcp", "--projects"]});
    if client == Client::Claude {
        v["type"] = json!("stdio");
    } else {
        v["startup_timeout_sec"] = json!(10);
        v["tool_timeout_sec"] = json!(30);
        if options.trust_apr_run {
            v["tools"]["apr_run"]["approval_mode"] = json!("approve");
        }
    }
    v
}
fn install_config(
    client: Client,
    executable: &Path,
    state: &Path,
    before: Option<&[u8]>,
    owned: Option<&str>,
    options: GlobalInstallOptions,
) -> Result<Edited> {
    let entry = installed_entry(client, executable, state, options);
    if client == Client::Claude {
        let mut document: Value = serde_json::from_slice(before.unwrap_or(b"{}"))
            .map_err(|_| Error::InvalidInput("invalid_json_config"))?;
        let root = document
            .as_object_mut()
            .ok_or(Error::InvalidInput("invalid_config_shape"))?;
        let servers = root
            .entry("mcpServers")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or(Error::InvalidInput("invalid_config_shape"))?;
        let existing = servers.get("apr").map(|v| digest(v.to_string().as_bytes()));
        if existing.as_deref() != owned {
            return Err(Error::Conflict(if owned.is_some() {
                "owned_entry_edited"
            } else {
                "unowned_entry"
            }));
        }
        let entry_hash = digest(entry.to_string().as_bytes());
        servers.insert("apr".into(), entry);
        let mut bytes = serde_json::to_vec_pretty(&document)
            .map_err(|_| Error::InvalidInput("invalid_json_config"))?;
        bytes.push(b'\n');
        return Ok(Edited { bytes, entry_hash });
    }
    let mut document = parse_toml(before)?;
    if !document.contains_key("mcp_servers") {
        let mut table = Table::new();
        table.set_implicit(true);
        document.insert("mcp_servers", Item::Table(table));
    }
    let servers = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .ok_or(Error::InvalidInput("invalid_config_shape"))?;
    let existing = servers.get("apr").map(codex_entry_digest);
    if existing.as_deref() != owned {
        return Err(Error::Conflict(if owned.is_some() {
            "owned_entry_edited"
        } else {
            "unowned_entry"
        }));
    }
    let mut table = Table::new();
    table.insert(
        "command",
        value(
            executable
                .to_str()
                .ok_or(Error::InvalidInput("non_utf8_path"))?,
        ),
    );
    let mut args = Array::new();
    for arg in [
        "--state-dir",
        state.to_str().ok_or(Error::InvalidInput("non_utf8_path"))?,
        "mcp",
        "--projects",
    ] {
        args.push(arg);
    }
    table.insert("args", value(args));
    table.insert("startup_timeout_sec", value(10));
    table.insert("tool_timeout_sec", value(30));
    if options.trust_apr_run {
        let mut run = Table::new();
        run.insert("approval_mode", value("approve"));
        let mut tools = Table::new();
        tools.insert("apr_run", Item::Table(run));
        table.insert("tools", Item::Table(tools));
    }
    servers.insert("apr", Item::Table(table));
    let bytes = document.to_string().into_bytes();
    let parsed = parse_toml(Some(&bytes))?;
    Ok(Edited {
        entry_hash: codex_entry_digest(&parsed["mcp_servers"]["apr"]),
        bytes,
    })
}

fn remove_config(
    client: Client,
    before: &[u8],
    owned: &str,
    created: bool,
) -> Result<Option<Vec<u8>>> {
    if client == Client::Claude {
        let mut document: Value = serde_json::from_slice(before)
            .map_err(|_| Error::InvalidInput("invalid_json_config"))?;
        let root = document
            .as_object_mut()
            .ok_or(Error::InvalidInput("invalid_config_shape"))?;
        let servers = root
            .get_mut("mcpServers")
            .and_then(Value::as_object_mut)
            .ok_or(Error::Conflict("owned_entry_edited"))?;
        if servers
            .get("apr")
            .map(|v| digest(v.to_string().as_bytes()))
            .as_deref()
            != Some(owned)
        {
            return Err(Error::Conflict("owned_entry_edited"));
        }
        servers.remove("apr");
        if created && servers.is_empty() {
            root.remove("mcpServers");
        }
        if created && root.is_empty() {
            return Ok(None);
        }
        let mut bytes = serde_json::to_vec_pretty(&document)
            .map_err(|_| Error::InvalidInput("invalid_json_config"))?;
        bytes.push(b'\n');
        return Ok(Some(bytes));
    }
    let mut document = parse_toml(Some(before))?;
    let servers = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .ok_or(Error::Conflict("owned_entry_edited"))?;
    if servers.get("apr").map(codex_entry_digest).as_deref() != Some(owned) {
        return Err(Error::Conflict("owned_entry_edited"));
    }
    servers.remove("apr");
    let bytes = document.to_string().into_bytes();
    if created && bytes.iter().all(u8::is_ascii_whitespace) {
        Ok(None)
    } else {
        Ok(Some(bytes))
    }
}
fn parse_toml(bytes: Option<&[u8]>) -> Result<DocumentMut> {
    std::str::from_utf8(bytes.unwrap_or(b""))
        .map_err(|_| Error::InvalidInput("config_not_utf8"))?
        .parse()
        .map_err(|_| Error::InvalidInput("invalid_toml"))
}
fn identity(home: &Path, client: Client) -> String {
    digest(format!("global\0{}\0{}", home.display(), client.name()).as_bytes())
}
fn record_path(id: &str) -> PathBuf {
    PathBuf::from(format!("global-integrations/{id}/record.json"))
}
fn journal_path(id: &str) -> PathBuf {
    PathBuf::from(format!("global-integrations/{id}/pending.json"))
}
fn lock_path(id: &str) -> PathBuf {
    PathBuf::from(format!("global-integrations/{id}/lock"))
}
fn snapshot_path(id: &str, hash: &str) -> PathBuf {
    PathBuf::from(format!("global-integrations/{id}/snapshots/{hash}"))
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn plan_digest(plan: &GlobalPlan) -> Result<String> {
    let mut value = plan.clone();
    value.plan_id.clear();
    Ok(digest(&json_bytes(&value)?))
}
fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| Error::InvalidInput("invalid_json"))
}
fn parse_record(bytes: Option<&[u8]>) -> Result<Option<Record>> {
    bytes
        .map(|b| {
            let r: Record = serde_json::from_slice(b)
                .map_err(|_| Error::Conflict("invalid_ownership_record"))?;
            if r.format_version != 1
                || !r.home.is_absolute()
                || !r.executable.is_absolute()
                || !valid_hash(&r.entry_hash)
                || !valid_hash(&r.skill_hash)
                || !valid_hash(&r.last_plan_id)
            {
                return Err(Error::Conflict("invalid_ownership_record"));
            }
            Ok(r)
        })
        .transpose()
}
