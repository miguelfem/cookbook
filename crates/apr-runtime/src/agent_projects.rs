//! Project bootstrap for the explicitly installed user-level MCP bridge.
//!
//! A path supplied to an MCP tool is not authority. The client must first write
//! a fresh, session-specific proof using its own authorized filesystem tools.
//! Ordinary fixed-attachment bridges never construct this broker.
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::service::ServiceClient;
use crate::store::{StoreLock, now_ms, private_dir};
use crate::{Result, Runtime, RuntimeError};

const MAX_PROJECTS: usize = 32;
const PROOF_TTL: Duration = Duration::from_secs(300);

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProjectProof {
    challenge: String,
    nonce: String,
    root: String,
}

struct ProjectDirectory {
    path: PathBuf,
    file: File,
    device: u64,
    inode: u64,
}

impl ProjectDirectory {
    fn open(path: &Path, state: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(RuntimeError::InvalidInput(
                "project root must be absolute".into(),
            ));
        }
        let path = path.canonicalize()?;
        if path.parent().is_none()
            || path.starts_with(state)
            || state.starts_with(&path)
            || std::env::var_os("HOME")
                .and_then(|home| PathBuf::from(home).canonicalize().ok())
                .is_some_and(|home| home.starts_with(&path))
            || path
                .to_str()
                .is_none_or(|p| p.chars().any(char::is_control))
        {
            return Err(RuntimeError::Forbidden);
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let metadata = file.metadata()?;
        Ok(Self {
            path,
            file,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn verify(&self) -> Result<()> {
        let metadata =
            std::fs::symlink_metadata(&self.path).map_err(|_| RuntimeError::Forbidden)?;
        if !metadata.is_dir()
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
            || self.path.canonicalize()? != self.path
        {
            return Err(RuntimeError::Forbidden);
        }
        Ok(())
    }

    fn proof(&self, filename: &str, expected: &[u8]) -> Result<()> {
        self.verify()?;
        let name = CString::new(filename).map_err(|_| RuntimeError::Forbidden)?;
        // The generated single-component filename is resolved from the pinned
        // directory descriptor. Symlinks, devices, pipes and hardlinks fail.
        let descriptor = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(RuntimeError::Forbidden);
        }
        // SAFETY: openat returned a new owned descriptor.
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
            || metadata.len() > 4096
        {
            return Err(RuntimeError::Forbidden);
        }
        let mut contents = Vec::new();
        Read::by_ref(&mut file)
            .take(4097)
            .read_to_end(&mut contents)?;
        // File editors may change JSON whitespace or key order. Authority is
        // the exact session nonce, challenge and pinned root, not formatting.
        // Typed deserialization rejects missing, unknown and duplicate fields.
        if contents.len() > 4096 {
            return Err(RuntimeError::Forbidden);
        }
        let provided: ProjectProof =
            serde_json::from_slice(&contents).map_err(|_| RuntimeError::Forbidden)?;
        let expected: ProjectProof =
            serde_json::from_slice(expected).map_err(|_| RuntimeError::Forbidden)?;
        if provided != expected {
            return Err(RuntimeError::Forbidden);
        }
        self.verify()
    }
}

struct Challenge {
    directory: ProjectDirectory,
    name: String,
    filename: String,
    contents: String,
    created: Instant,
}

struct Workspace {
    directory: ProjectDirectory,
    client: ServiceClient,
}

/// Only created by `apr mcp --projects`, after the user installs that connection.
pub struct ProjectBroker {
    state: PathBuf,
    pending: HashMap<String, Challenge>,
    workspaces: HashMap<String, Workspace>,
    completed: HashMap<String, (String, Value)>,
}

impl ProjectBroker {
    pub fn open(state: &Path) -> Result<Self> {
        let runtime = Runtime::open(state)?;
        Ok(Self {
            state: runtime.root.clone(),
            pending: HashMap::new(),
            workspaces: HashMap::new(),
            completed: HashMap::new(),
        })
    }

    pub fn prepare(&mut self, root: &Path, name: Option<&str>) -> Result<Value> {
        self.pending
            .retain(|_, challenge| challenge.created.elapsed() < PROOF_TTL);
        if self.pending.len() >= MAX_PROJECTS {
            return Err(RuntimeError::LimitExceeded(
                "too many project connections in this session".into(),
            ));
        }
        let directory = ProjectDirectory::open(root, &self.state)?;
        let name = name.unwrap_or_else(|| {
            directory
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Recipes")
        });
        if name.trim().is_empty() || name.len() > 160 || name.chars().any(char::is_control) {
            return Err(RuntimeError::InvalidInput(
                "project name must contain 1-160 bytes without controls".into(),
            ));
        }
        let name = name.to_string();
        let id = Uuid::new_v4().simple().to_string();
        let filename = format!(".cookbook-connect-{id}.json");
        let contents = format!(
            "{}\n",
            json!({"challenge":id,"nonce":Uuid::new_v4().simple().to_string(),"root":directory.path})
        );
        let response = json!({"challenge_id":id,"root":directory.path,
            "verification_file":directory.path.join(&filename),"verification_contents":contents,
            "expires_in_seconds":PROOF_TTL.as_secs(),
            "next":"Using your client's authorized filesystem tool, create verification_file with the JSON in verification_contents. Preserve every field and string value; JSON whitespace and key order may differ. Then call apr_project action=connect with challenge_id. Do not ask the user to perform these steps. Only use the project the user is working in. No credentials are involved."});
        self.pending.insert(
            id,
            Challenge {
                directory,
                name,
                filename,
                contents,
                created: Instant::now(),
            },
        );
        Ok(response)
    }

    pub fn connect(&mut self, challenge_id: &str) -> Result<Value> {
        if let Some((handle, response)) = self.completed.get(challenge_id) {
            let workspace = self
                .workspaces
                .get(handle)
                .ok_or(RuntimeError::Unauthorized)?;
            workspace.directory.verify()?;
            // Replay must still respect owner revocation, closure and expiry.
            Runtime::open(&self.state)?.validate_agent_attachment(&workspace.client, None)?;
            return Ok(response.clone());
        }
        let challenge = self
            .pending
            .get(challenge_id)
            .ok_or(RuntimeError::Forbidden)?;
        if challenge.created.elapsed() >= PROOF_TTL {
            return Err(RuntimeError::Forbidden);
        }
        challenge
            .directory
            .proof(&challenge.filename, challenge.contents.as_bytes())?;
        let existing = self
            .workspaces
            .iter()
            .find(|(_, workspace)| {
                workspace.directory.path == challenge.directory.path
                    && workspace.directory.device == challenge.directory.device
                    && workspace.directory.inode == challenge.directory.inode
            })
            .map(|(handle, _)| handle.clone());
        if existing.is_none() && self.workspaces.len() >= MAX_PROJECTS {
            return Err(RuntimeError::LimitExceeded(
                "too many projects in this session".into(),
            ));
        }
        let mut runtime = Runtime::open(&self.state)?;
        let (client, project_id) =
            runtime.agent_attachment(&challenge.directory, &challenge.name)?;
        challenge.directory.verify()?;
        let handle = existing.unwrap_or_else(|| format!("workspace_{}", Uuid::new_v4().simple()));
        let response = json!({"workspace":handle,"project_id":project_id,"assignment_id":client.assignment_id(),
            "root":challenge.directory.path,"mode":"author", "created_or_connected":true,
            "cleanup_file":challenge.directory.path.join(&challenge.filename),
            "next":"Remove cleanup_file with your filesystem tool. Include workspace in every APR tool call. Run a known prepared recipe directly. Discover only when selecting a recipe or reading needed guidance. Connection credentials, native workers and access grants still require owner approval."});
        let challenge = self
            .pending
            .remove(challenge_id)
            .ok_or(RuntimeError::Forbidden)?;
        self.workspaces.insert(
            handle.clone(),
            Workspace {
                directory: challenge.directory,
                client,
            },
        );
        // Keep bounded lost-response replay without consuming project slots.
        if self.completed.len() >= MAX_PROJECTS * 4
            && let Some(evicted) = self.completed.keys().next().cloned()
        {
            self.completed.remove(&evicted);
        }
        self.completed
            .insert(challenge_id.into(), (handle, response.clone()));
        Ok(response)
    }

    /// Resolve only handles returned by this bridge session. Never select a
    /// mutable global 'current project' for concurrent client conversations.
    pub fn client(&self, workspace: &str) -> Result<ServiceClient> {
        let workspace = self
            .workspaces
            .get(workspace)
            .ok_or(RuntimeError::Unauthorized)?;
        workspace.directory.verify()?;
        Ok(workspace.client.clone())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    attachment_file: PathBuf,
}

fn private_file(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 64 * 1024
    {
        return Err(RuntimeError::Forbidden);
    }
    Ok(file)
}

impl Runtime {
    fn agent_attachment(
        &mut self,
        directory: &ProjectDirectory,
        name: &str,
    ) -> Result<(ServiceClient, String)> {
        if self.maintenance_status()?["paused_for_update"] == true {
            return Err(RuntimeError::State("runtime is paused for update".into()));
        }
        let records = self.root.join("agent-projects");
        private_dir(&records)?;
        let lock_path = records.join(".lock");
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&lock_path)?;
        private_file(&lock_path)?;
        lock.lock_exclusive()?;
        directory.verify()?;
        let key = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                directory.path.clone(),
                directory.device,
                directory.inode
            ))?)
        );
        let record_path = records.join(format!("{key}.json"));
        if record_path.try_exists()? {
            let record: Record =
                serde_json::from_reader(private_file(&record_path)?.take(64 * 1024 + 1))?;
            if record.attachment_file.parent() != Some(self.root.join("attachments").as_path()) {
                return Err(RuntimeError::Forbidden);
            }
            let client = ServiceClient::from_file(&record.attachment_file)?;
            let project = self.validate_agent_attachment(&client, Some(directory))?;
            if project.1 != directory.path
                || project.2 != directory.device.to_string()
                || project.3 != directory.inode.to_string()
            {
                return Err(RuntimeError::Forbidden);
            }
            return Ok((client, project.0));
        }
        let assignment = self.owner_project_setup(&directory.path, name)?;
        let assignment_id = assignment["assignment_id"]
            .as_str()
            .ok_or(RuntimeError::Unauthorized)?;
        let result = (|| {
            let attachment = self.owner_attachment_file(assignment_id)?;
            let client = ServiceClient::from_file(&attachment)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&record_path)?;
            file.write_all(&serde_json::to_vec(&Record {
                attachment_file: attachment,
            })?)?;
            file.sync_all()?;
            File::open(&records)?.sync_all()?;
            Ok((
                client,
                assignment["project_id"]
                    .as_str()
                    .ok_or(RuntimeError::Unauthorized)?
                    .into(),
            ))
        })();
        if result.is_err() {
            let _ = self.close_assignment(assignment_id);
        }
        result
    }

    fn validate_agent_attachment(
        &self,
        client: &ServiceClient,
        renew: Option<&ProjectDirectory>,
    ) -> Result<(String, PathBuf, String, String)> {
        let _lock = StoreLock::acquire(&self.root)?;
        let attachment = client.attachment_for_project_broker();
        if attachment.state_dir != self.root || attachment.mode != "author" {
            return Err(RuntimeError::Unauthorized);
        }
        let project = self.db.query_row(
            "SELECT p.id,p.root,p.device,p.inode FROM attachments t JOIN assignments a ON a.id=t.assignment_id JOIN projects p ON p.id=a.project_id WHERE t.id=?1 AND t.assignment_id=?2 AND t.token_hash=?3 AND t.mode='author' AND t.revoked_at IS NULL AND a.state='active' AND t.expires_at>?4",
            params![attachment.attachment_id,attachment.assignment_id,format!("{:x}",Sha256::digest(attachment.token.as_bytes())),now_ms()],
            |row| Ok((row.get::<_,String>(0)?,PathBuf::from(row.get::<_,String>(1)?),row.get::<_,String>(2)?,row.get::<_,String>(3)?)))
            .map_err(|_| RuntimeError::Unauthorized)?;
        if let Some(directory) = renew {
            if project.1 != directory.path
                || project.2 != directory.device.to_string()
                || project.3 != directory.inode.to_string()
            {
                return Err(RuntimeError::Forbidden);
            }
            self.db.execute(
                "UPDATE attachments SET expires_at=?2 WHERE id=?1",
                params![
                    attachment.attachment_id,
                    now_ms() + 30 * 24 * 60 * 60 * 1000
                ],
            )?;
        }
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn fixture() -> (tempfile::TempDir, ProjectBroker, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let broker = ProjectBroker::open(&temp.path().join("state")).unwrap();
        (temp, broker, root)
    }

    fn proof(broker: &mut ProjectBroker, root: &Path) -> (String, PathBuf) {
        let value = broker.prepare(root, None).unwrap();
        let file = PathBuf::from(value["verification_file"].as_str().unwrap());
        std::fs::write(&file, value["verification_contents"].as_str().unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        (value["challenge_id"].as_str().unwrap().into(), file)
    }

    #[test]
    fn bootstrap_requires_fresh_authorized_file_and_excludes_private_state() {
        let (_temp, mut broker, root) = fixture();
        for forbidden in [
            &broker.state.clone(),
            &root.parent().unwrap().to_path_buf(),
            &PathBuf::from("/"),
        ] {
            assert!(broker.prepare(forbidden, None).is_err());
        }
        assert!(broker.prepare(Path::new("relative"), None).is_err());
        let value = broker.prepare(&root, None).unwrap();
        let id = value["challenge_id"].as_str().unwrap();
        assert!(broker.connect(id).is_err());
        std::fs::write(value["verification_file"].as_str().unwrap(), "wrong").unwrap();
        assert!(broker.connect(id).is_err());
        let (id, _) = proof(&mut broker, &root);
        broker.pending.get_mut(&id).unwrap().created = Instant::now() - PROOF_TTL;
        assert!(broker.connect(&id).is_err());
        let runtime = Runtime::open(&broker.state).unwrap();
        assert_eq!(
            runtime
                .db
                .query_row("SELECT count(*) FROM assignments", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn proof_rejects_links_unsafe_modes_and_replaced_directory() {
        let (temp, mut broker, root) = fixture();
        let (id, file) = proof(&mut broker, &root);
        let stored = temp.path().join("proof");
        std::fs::rename(&file, &stored).unwrap();
        symlink(&stored, &file).unwrap();
        assert!(broker.connect(&id).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::hard_link(&stored, &file).unwrap();
        assert!(broker.connect(&id).is_err());
        std::fs::remove_file(&stored).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(broker.connect(&id).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::rename(&root, temp.path().join("moved")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(broker.connect(&id).is_err());
    }

    #[test]
    fn proof_formatting_preserves_exact_identity_and_rejects_ambiguous_payloads() {
        let (_temp, mut broker, root) = fixture();
        let prepared = broker.prepare(&root, None).unwrap();
        let id = prepared["challenge_id"].as_str().unwrap();
        let file = PathBuf::from(prepared["verification_file"].as_str().unwrap());
        let expected: Value =
            serde_json::from_str(prepared["verification_contents"].as_str().unwrap()).unwrap();
        for field in ["challenge", "nonce", "root"] {
            let mut changed = expected.clone();
            changed[field] = json!("different identity");
            std::fs::write(&file, serde_json::to_vec(&changed).unwrap()).unwrap();
            assert!(broker.connect(id).is_err(), "changed {field}");
            let mut missing = expected.clone();
            missing.as_object_mut().unwrap().remove(field);
            std::fs::write(&file, serde_json::to_vec(&missing).unwrap()).unwrap();
            assert!(broker.connect(id).is_err(), "missing {field}");
            let duplicate = format!(
                "{{\"{field}\":{},{}",
                expected[field],
                serde_json::to_string(&expected)
                    .unwrap()
                    .trim_start_matches('{')
            );
            std::fs::write(&file, duplicate).unwrap();
            assert!(broker.connect(id).is_err(), "duplicate {field}");
        }
        let mut extra = expected.clone();
        extra["extra"] = json!(true);
        for invalid in [
            serde_json::to_string(&extra).unwrap(),
            format!("{expected} null"),
            format!("{expected}{}", " ".repeat(4097)),
        ] {
            std::fs::write(&file, invalid).unwrap();
            assert!(broker.connect(id).is_err());
        }
        let reordered = format!(
            "{{\n  \"root\": {},\n  \"nonce\": {},\n  \"challenge\": {}\n}}\n\n",
            expected["root"], expected["nonce"], expected["challenge"]
        );
        std::fs::write(&file, reordered).unwrap();
        let connected = broker.connect(id).unwrap();
        assert!(connected["workspace"].is_string());
        assert_eq!(broker.connect(id).unwrap(), connected);
    }

    #[test]
    fn fresh_proof_and_response_replay_never_revive_revoked_expired_or_closed_access() {
        for condition in ["revoked", "expired", "closed"] {
            let (_temp, mut broker, root) = fixture();
            let (id, _) = proof(&mut broker, &root);
            let connected = broker.connect(&id).unwrap();
            let client = broker
                .client(connected["workspace"].as_str().unwrap())
                .unwrap();
            let attachment = client.attachment_for_project_broker();
            let mut runtime = Runtime::open(&broker.state).unwrap();
            match condition {
                "revoked" => {
                    runtime
                        .revoke_attachment(&attachment.attachment_id)
                        .unwrap();
                }
                "closed" => {
                    runtime.close_assignment(client.assignment_id()).unwrap();
                }
                _ => {
                    runtime
                        .db
                        .execute(
                            "UPDATE attachments SET expires_at=0 WHERE id=?1",
                            [&attachment.attachment_id],
                        )
                        .unwrap();
                }
            }
            assert!(broker.connect(&id).is_err(), "{condition} replay");
            let (fresh, _) = proof(&mut broker, &root);
            assert!(broker.connect(&fresh).is_err(), "{condition} fresh proof");
            let mut reopened = ProjectBroker::open(&broker.state).unwrap();
            let (fresh, _) = proof(&mut reopened, &root);
            assert!(reopened.connect(&fresh).is_err(), "{condition} new bridge");
        }
    }

    #[test]
    fn repeat_connect_reuses_workspace_and_never_copies_grants_or_tokens() {
        let (_temp, mut broker, root) = fixture();
        let mut previous = Value::Null;
        for _ in 0..MAX_PROJECTS + 1 {
            let (id, file) = proof(&mut broker, &root);
            let connected = broker.connect(&id).unwrap();
            if previous != Value::Null {
                assert_eq!(previous["workspace"], connected["workspace"]);
            }
            assert_eq!(broker.connect(&id).unwrap(), connected);
            assert!(connected.get("token").is_none());
            assert!(connected.get("attachment_file").is_none());
            previous = connected;
            std::fs::remove_file(file).unwrap();
        }
        let runtime = Runtime::open(&broker.state).unwrap();
        assert_eq!(
            runtime
                .db
                .query_row("SELECT count(*) FROM assignments", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            runtime
                .db
                .query_row("SELECT count(*) FROM execution_grants", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(broker.workspaces.len(), 1);
    }

    #[test]
    fn concurrent_bridges_reuse_one_persistent_assignment() {
        let (_temp, broker, root) = fixture();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let state = broker.state.clone();
                let root = root.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut broker = ProjectBroker::open(&state).unwrap();
                    let (id, _) = proof(&mut broker, &root);
                    barrier.wait();
                    broker.connect(&id).unwrap()["assignment_id"].clone()
                })
            })
            .collect();
        let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(ids[0], ids[1]);
    }
}
