//! Managed project client configuration, with explicit previews and ownership.
mod config;
mod diagnostics;
mod files;
pub mod global;

use files::Root;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const SKILL: &str = include_str!("../../../integrations/skills/apr/SKILL.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Client {
    Codex,
    Claude,
}

impl Client {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
    pub fn config_path(self) -> &'static Path {
        Path::new(match self {
            Self::Codex => ".codex/config.toml",
            Self::Claude => ".mcp.json",
        })
    }
    pub fn skill_path(self) -> &'static Path {
        Path::new(match self {
            Self::Codex => ".agents/skills/apr/SKILL.md",
            Self::Claude => ".claude/skills/apr/SKILL.md",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRequest {
    pub project_dir: PathBuf,
    pub attachment_file: PathBuf,
    pub executable: PathBuf,
    pub client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub format_version: u32,
    pub state_dir: PathBuf,
    pub request: InstallRequest,
    pub integration_id: String,
    pub plan_id: String,
    pub config_path: PathBuf,
    pub skill_path: PathBuf,
    pub server_entry: Value,
    pub expected_config: Option<String>,
    pub expected_skill: Option<String>,
    pub expected_record: Option<String>,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub integration_id: String,
    pub state: String,
    pub changed: bool,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub configuration: String,
    pub client: ClientStatus,
    pub service: String,
    pub service_code: Option<String>,
    pub client_trust: String,
    pub client_connection: String,
}

#[derive(Debug, Serialize)]
pub struct ClientStatus {
    pub status: String,
    pub executable: Option<PathBuf>,
    pub version: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("integration input is invalid: {0}")]
    InvalidInput(&'static str),
    #[error("integration conflicts with existing changes: {0}")]
    Conflict(&'static str),
    #[error("integration path is not a safe owned regular file or directory")]
    UnsafePath,
    #[error("integration data exceeds the byte limit")]
    LimitExceeded,
    #[error("another integration operation is in progress")]
    Busy,
    #[error("integration is not managed")]
    NotManaged,
    #[error("local integration storage failed")]
    Io(#[from] std::io::Error),
}
impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Conflict(_) => "INTEGRATION_CONFLICT",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::UnsafePath => "UNSAFE_PATH",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::Busy => "INTEGRATION_BUSY",
            Self::NotManaged => "NOT_MANAGED",
            Self::Io(_) => "INTERNAL_ERROR",
        }
    }
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Conflict(reason) | Self::InvalidInput(reason) => reason,
            Self::UnsafePath => "unsafe_path",
            Self::LimitExceeded => "file_too_large",
            Self::Busy => "integration_busy",
            Self::NotManaged => "not_managed",
            Self::Io(_) => "storage_error",
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format_version: u32,
    request: InstallRequest,
    config_created: bool,
    entry_hash: String,
    skill_hash: String,
    last_plan_id: String,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
    integration_id: String,
    project: PathBuf,
    client: Client,
    operation: String,
    plan_id: String,
    files: Vec<Replacement>,
}

pub struct Manager {
    state: Root,
}

impl Manager {
    pub fn open(state_dir: &Path) -> Result<Self> {
        Ok(Self {
            state: Root::open(state_dir, true)?,
        })
    }

    pub fn installed_request(&self, project: &Path, client: Client) -> Result<InstallRequest> {
        let project = Root::open(project, false)?;
        let id = identity(&project.path, client);
        let record = parse_record(self.state.read(&record_path(&id))?.as_deref())?
            .ok_or(Error::NotManaged)?;
        if record.request.project_dir != project.path || record.request.client != client {
            return Err(Error::Conflict("invalid_ownership_record"));
        }
        Ok(record.request)
    }

    pub fn doctor(&self, request: &InstallRequest) -> Result<DoctorReport> {
        self.doctor_with_client(request, None)
    }

    pub fn doctor_with_client(
        &self,
        request: &InstallRequest,
        executable: Option<&Path>,
    ) -> Result<DoctorReport> {
        let project = Root::open(&request.project_dir, false)?;
        let id = identity(&project.path, request.client);
        let record = parse_record(self.state.read(&record_path(&id))?.as_deref())?
            .ok_or(Error::NotManaged)?;
        if record.request.project_dir != project.path || record.request.client != request.client {
            return Err(Error::Conflict("invalid_ownership_record"));
        }
        let configuration = if self.pending(&id)?.is_some() {
            "operation_pending"
        } else {
            let config = project.read(request.client.config_path())?;
            let skill = project.read(request.client.skill_path())?;
            if config::install(&record.request, config.as_deref(), Some(&record.entry_hash)).is_ok()
                && skill.as_deref().map(digest).as_deref() == Some(&record.skill_hash)
            {
                "configured"
            } else {
                "changed"
            }
        };
        let ready = apr_runtime::service::ServiceClient::from_file(&record.request.attachment_file)
            .and_then(|client| {
                if client.state_dir().canonicalize()? != self.state.path {
                    return Err(apr_runtime::RuntimeError::InvalidInput(
                        "attachment state does not match integration".into(),
                    ));
                }
                client.with_launcher(&record.request.executable).call(
                    "service.info",
                    serde_json::json!({}),
                    8192,
                )
            });
        let service_code = ready.as_ref().err().map(|error| error.code().to_string());
        Ok(DoctorReport {
            configuration: configuration.into(),
            client: diagnostics::client(request.client, executable),
            service: if ready.is_ok() {
                "ready"
            } else {
                "unavailable"
            }
            .into(),
            service_code,
            client_trust: "unknown".into(),
            client_connection: "unverified".into(),
        })
    }

    pub fn update(&self, request: &InstallRequest) -> Result<Report> {
        let (request, _) = self.validate(request)?;
        let id = identity(&request.project_dir, request.client);
        {
            let _lock = self
                .state
                .lock(&PathBuf::from(format!("integrations/{id}/lock")))?;
            if let Some(journal) = self.pending(&id)? {
                let after = journal
                    .files
                    .iter()
                    .find(|file| file.slot == Slot::Record)
                    .and_then(|file| file.after.as_deref());
                let next = after
                    .map(|hash| self.snapshot_bytes(&id, hash))
                    .transpose()?;
                let next = parse_record(next.as_deref())?;
                if journal.operation != "install"
                    || !next.is_some_and(|record| record.request == request)
                {
                    return Err(Error::Conflict("operation_pending"));
                }
                return self.finish(&journal);
            }
        }
        let plan = self.preview(&request)?;
        if plan.expected_record.is_none() {
            return Err(Error::NotManaged);
        }
        self.apply(&plan)
    }

    pub fn uninstall(&self, project: &Path, client: Client) -> Result<Report> {
        let project = Root::open(project, false)?;
        let id = identity(&project.path, client);
        let _lock = self
            .state
            .lock(&PathBuf::from(format!("integrations/{id}/lock")))?;
        if let Some(journal) = self.pending(&id)? {
            if journal.operation != "uninstall" {
                return Err(Error::Conflict("operation_pending"));
            }
            return self.finish(&journal);
        }
        let record_bytes = self.state.read(&record_path(&id))?;
        let Some(record) = parse_record(record_bytes.as_deref())? else {
            return Ok(Report {
                integration_id: id,
                state: "uninstalled".into(),
                changed: false,
            });
        };
        let before = project
            .read(client.config_path())?
            .ok_or(Error::Conflict("owned_entry_edited"))?;
        let skill = project
            .read(client.skill_path())?
            .ok_or(Error::Conflict("owned_skill_edited"))?;
        if digest(&skill) != record.skill_hash {
            return Err(Error::Conflict("owned_skill_edited"));
        }
        let after = config::remove(client, &before, &record.entry_hash, record.config_created)?;
        let files = vec![
            self.replacement(&id, Slot::Config, Some(&before), after.as_deref())?,
            self.replacement(&id, Slot::Skill, Some(&skill), None)?,
            self.replacement(&id, Slot::Record, record_bytes.as_deref(), None)?,
        ];
        self.commit(Journal {
            format_version: 1,
            integration_id: id,
            project: project.path,
            client,
            operation: "uninstall".into(),
            plan_id: String::new(),
            files,
        })
    }

    pub fn preview(&self, request: &InstallRequest) -> Result<Plan> {
        let (request, project) = self.validate(request)?;
        let integration_id = identity(&request.project_dir, request.client);
        if self.pending(&integration_id)?.is_some() {
            return Err(Error::Conflict("operation_pending"));
        }
        let record_bytes = self.state.read(&record_path(&integration_id))?;
        let record = parse_record(record_bytes.as_deref())?;
        let before = project.read(request.client.config_path())?;
        let skill = project.read(request.client.skill_path())?;
        let edited = config::install(
            &request,
            before.as_deref(),
            record.as_ref().map(|r| r.entry_hash.as_str()),
        )?;
        if skill.as_deref().map(digest).as_deref()
            != record.as_ref().map(|record| record.skill_hash.as_str())
        {
            return Err(Error::Conflict(if record.is_some() {
                "owned_skill_edited"
            } else {
                "unowned_skill"
            }));
        }
        let mut changes = Vec::new();
        if before.as_deref() != Some(edited.bytes.as_slice()) {
            changes.push("write_config".into());
        }
        if skill.as_deref() != Some(SKILL.as_bytes()) {
            changes.push("write_skill".into());
        }
        let mut plan = Plan {
            format_version: 1,
            state_dir: self.state.path.clone(),
            config_path: project.path.join(request.client.config_path()),
            skill_path: project.path.join(request.client.skill_path()),
            server_entry: config::entry(&request),
            request,
            integration_id,
            plan_id: String::new(),
            expected_config: before.as_deref().map(digest),
            expected_skill: skill.as_deref().map(digest),
            expected_record: record_bytes.as_deref().map(digest),
            changes,
        };
        plan.plan_id = plan_digest(&plan)?;
        Ok(plan)
    }

    pub fn apply(&self, plan: &Plan) -> Result<Report> {
        if plan.format_version != 1
            || plan.state_dir != self.state.path
            || plan.plan_id != plan_digest(plan)?
            || plan.integration_id != identity(&plan.request.project_dir, plan.request.client)
        {
            return Err(Error::InvalidInput("invalid_plan"));
        }
        let _lock = self.state.lock(&PathBuf::from(format!(
            "integrations/{}/lock",
            plan.integration_id
        )))?;
        if let Some(journal) = self.pending(&plan.integration_id)? {
            if journal.operation != "install" || journal.plan_id != plan.plan_id {
                return Err(Error::Conflict("operation_pending"));
            }
            return self.finish(&journal);
        }
        let fresh = self.preview(&plan.request)?;
        let existing = parse_record(
            self.state
                .read(&record_path(&plan.integration_id))?
                .as_deref(),
        )?;
        if existing
            .as_ref()
            .is_some_and(|record| record.last_plan_id == plan.plan_id)
            || (fresh.plan_id == plan.plan_id && fresh.changes.is_empty() && existing.is_some())
        {
            return Ok(Report {
                integration_id: plan.integration_id.clone(),
                state: "installed".into(),
                changed: false,
            });
        }
        if fresh.plan_id != plan.plan_id {
            return Err(Error::Conflict("stale_preview"));
        }
        let project = Root::open(&plan.request.project_dir, false)?;
        let before = project.read(plan.request.client.config_path())?;
        let record = parse_record(
            self.state
                .read(&record_path(&plan.integration_id))?
                .as_deref(),
        )?;
        let edited = config::install(
            &plan.request,
            before.as_deref(),
            record.as_ref().map(|r| r.entry_hash.as_str()),
        )?;
        let record = Record {
            format_version: 1,
            request: plan.request.clone(),
            config_created: record.map(|r| r.config_created).unwrap_or(before.is_none()),
            entry_hash: edited.entry_hash,
            skill_hash: digest(SKILL.as_bytes()),
            last_plan_id: plan.plan_id.clone(),
        };
        let skill = project.read(plan.request.client.skill_path())?;
        let record_before = self.state.read(&record_path(&plan.integration_id))?;
        let files = vec![
            self.replacement(
                &plan.integration_id,
                Slot::Config,
                before.as_deref(),
                Some(&edited.bytes),
            )?,
            self.replacement(
                &plan.integration_id,
                Slot::Skill,
                skill.as_deref(),
                Some(SKILL.as_bytes()),
            )?,
            self.replacement(
                &plan.integration_id,
                Slot::Record,
                record_before.as_deref(),
                Some(&json_bytes(&record)?),
            )?,
        ];
        self.commit(Journal {
            format_version: 1,
            integration_id: plan.integration_id.clone(),
            project: project.path,
            client: plan.request.client,
            operation: "install".into(),
            plan_id: plan.plan_id.clone(),
            files,
        })
    }

    fn validate(&self, request: &InstallRequest) -> Result<(InstallRequest, Root)> {
        let project = Root::open(&request.project_dir, false)?;
        diagnostics::ensure_untracked(&project.path, request.client.config_path())?;
        let executable = request.executable.canonicalize()?;
        let metadata = fs::metadata(&executable)?;
        if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
            return Err(Error::InvalidInput("executable_unavailable"));
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&request.attachment_file)?;
        let metadata = file.metadata()?;
        files::check_regular(&metadata, true)?;
        if metadata.len() > 64 * 1024 {
            return Err(Error::LimitExceeded);
        }
        let attachment_file = request.attachment_file.canonicalize()?;
        if !attachment_file.starts_with(&self.state.path) {
            return Err(Error::InvalidInput("attachment_outside_state"));
        }
        if executable.to_str().is_none() || attachment_file.to_str().is_none() {
            return Err(Error::InvalidInput("non_utf8_path"));
        }
        Ok((
            InstallRequest {
                project_dir: project.path.clone(),
                executable,
                attachment_file,
                client: request.client,
            },
            project,
        ))
    }

    fn snapshot(&self, id: &str, bytes: &[u8]) -> Result<String> {
        let hash = digest(bytes);
        let path = PathBuf::from(format!("integrations/{id}/snapshots/{hash}"));
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
            before: before.map(|bytes| self.snapshot(id, bytes)).transpose()?,
            after: after.map(|bytes| self.snapshot(id, bytes)).transpose()?,
        })
    }

    fn pending(&self, id: &str) -> Result<Option<Journal>> {
        self.state
            .read(&journal_path(id))?
            .map(|bytes| {
                let journal: Journal = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::Conflict("invalid_journal"))?;
                if journal.format_version != 1
                    || journal.integration_id != id
                    || identity(&journal.project, journal.client) != id
                    || !["install", "uninstall"].contains(&journal.operation.as_str())
                    || journal
                        .files
                        .iter()
                        .map(|file| file.slot)
                        .collect::<Vec<_>>()
                        != [Slot::Config, Slot::Skill, Slot::Record]
                    || journal
                        .files
                        .iter()
                        .flat_map(|file| [&file.before, &file.after])
                        .flatten()
                        .any(|hash| !valid_hash(hash))
                {
                    return Err(Error::Conflict("invalid_journal"));
                }
                Ok(journal)
            })
            .transpose()
    }

    fn commit(&self, journal: Journal) -> Result<Report> {
        self.state.write(
            &journal_path(&journal.integration_id),
            None,
            Some(&json_bytes(&journal)?),
        )?;
        self.finish(&journal)
    }

    fn finish(&self, journal: &Journal) -> Result<Report> {
        let project = Root::open(&journal.project, false)?;
        if journal.operation == "install" {
            // Recovery may happen after the user changes repository policy.
            // Do not publish remaining device-local setup into tracked config.
            diagnostics::ensure_untracked(&project.path, journal.client.config_path())?;
        }
        // Check every target before changing any of them. A third state belongs
        // to an external edit and is never overwritten during recovery.
        for file in &journal.files {
            let (root, path) = self.target(&project, journal, file.slot);
            let current = root.read(&path)?.as_deref().map(digest);
            if current != file.before && current != file.after {
                return Err(Error::Conflict("recovery_target_edited"));
            }
            if let Some(hash) = &file.after {
                self.snapshot_bytes(&journal.integration_id, hash)?;
            }
        }
        for file in &journal.files {
            let (root, path) = self.target(&project, journal, file.slot);
            let current = root.read(&path)?.as_deref().map(digest);
            if current == file.after {
                continue;
            }
            let after = file
                .after
                .as_deref()
                .map(|hash| self.snapshot_bytes(&journal.integration_id, hash))
                .transpose()?;
            root.write(&path, file.before.as_deref(), after.as_deref())?;
        }
        let path = journal_path(&journal.integration_id);
        let current = self
            .state
            .read(&path)?
            .ok_or(Error::Conflict("journal_changed"))?;
        self.state.write(&path, Some(&digest(&current)), None)?;
        Ok(Report {
            integration_id: journal.integration_id.clone(),
            state: if journal.operation == "install" {
                "installed"
            } else {
                "uninstalled"
            }
            .into(),
            changed: true,
        })
    }

    fn target<'a>(
        &'a self,
        project: &'a Root,
        journal: &Journal,
        slot: Slot,
    ) -> (&'a Root, PathBuf) {
        match slot {
            Slot::Config => (project, journal.client.config_path().into()),
            Slot::Skill => (project, journal.client.skill_path().into()),
            Slot::Record => (&self.state, record_path(&journal.integration_id)),
        }
    }

    fn snapshot_bytes(&self, id: &str, hash: &str) -> Result<Vec<u8>> {
        if !valid_hash(hash) {
            return Err(Error::Conflict("invalid_backup_reference"));
        }
        let bytes = self
            .state
            .read(&PathBuf::from(format!(
                "integrations/{id}/snapshots/{hash}"
            )))?
            .ok_or(Error::Conflict("missing_backup"))?;
        if digest(&bytes) != hash {
            return Err(Error::Conflict("backup_changed"));
        }
        Ok(bytes)
    }
}

fn record_path(id: &str) -> PathBuf {
    PathBuf::from(format!("integrations/{id}/record.json"))
}
fn journal_path(id: &str) -> PathBuf {
    PathBuf::from(format!("integrations/{id}/pending.json"))
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn identity(project: &Path, client: Client) -> String {
    digest(format!("{}\0{}", project.display(), client.name()).as_bytes())
}
fn plan_digest(plan: &Plan) -> Result<String> {
    let mut value = plan.clone();
    value.plan_id.clear();
    Ok(digest(&json_bytes(&value)?))
}
fn parse_record(bytes: Option<&[u8]>) -> Result<Option<Record>> {
    bytes
        .map(|bytes| {
            let record: Record = serde_json::from_slice(bytes)
                .map_err(|_| Error::Conflict("invalid_ownership_record"))?;
            if record.format_version != 1
                || !valid_hash(&record.entry_hash)
                || !valid_hash(&record.skill_hash)
                || !valid_hash(&record.last_plan_id)
                || !record.request.project_dir.is_absolute()
                || !record.request.attachment_file.is_absolute()
                || !record.request.executable.is_absolute()
            {
                return Err(Error::Conflict("invalid_ownership_record"));
            }
            Ok(record)
        })
        .transpose()
}
fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| Error::InvalidInput("invalid_json"))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
