//! Public, owner-managed OAuth client identity. No account credentials live here.
use crate::{Result, Runtime, RuntimeError};
use apr_connectors::oauth::{OAuthConfig, PreRegisteredClient};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;

const FILE: &str = "oauth-client-identity.json";
const MAX_BYTES: u64 = 16 * 1024;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClientIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id_metadata_url: Option<String>,
    #[serde(default)]
    pub pre_registered_clients: Vec<PublicClient>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PublicClient {
    pub issuer: String,
    pub client_id: String,
}

impl ClientIdentity {
    pub fn validate(&self) -> Result<()> {
        let invalid = || RuntimeError::InvalidInput("invalid public OAuth client identity".into());
        if let Some(value) = &self.client_id_metadata_url {
            let url = apr_connectors::oauth::public_url(value, false).map_err(|_| invalid())?;
            if url.path() == "/" || url.path().is_empty() || url.query().is_some() {
                return Err(invalid());
            }
        }
        if self.pre_registered_clients.len() > 32 {
            return Err(invalid());
        }
        let mut issuers = BTreeSet::new();
        for client in &self.pre_registered_clients {
            let issuer =
                apr_connectors::oauth::public_url(&client.issuer, false).map_err(|_| invalid())?;
            if issuer.query().is_some()
                || !issuers.insert(&client.issuer)
                || client.client_id.is_empty()
                || client.client_id.len() > 2048
                || client.client_id.chars().any(char::is_control)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

impl Runtime {
    pub fn oauth_client_identity(&self) -> Result<ClientIdentity> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let bytes = match options.open(self.root.join(FILE)) {
            Ok(file) => {
                if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
                    return Err(RuntimeError::InvalidInput(
                        "invalid client identity file".into(),
                    ));
                }
                let mut bytes = Vec::new();
                file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
                bytes
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                include_bytes!("../../../integrations/oauth/client-identity.json").to_vec()
            }
            Err(error) => return Err(error.into()),
        };
        if bytes.len() as u64 > MAX_BYTES {
            return Err(RuntimeError::InvalidInput(
                "client identity exceeds limit".into(),
            ));
        }
        let identity: ClientIdentity = serde_json::from_slice(&bytes)?;
        identity.validate()?;
        Ok(identity)
    }

    /// Trusted owner setup only; does not connect an account or grant a tool.
    pub fn set_oauth_client_identity(&self, identity: &ClientIdentity) -> Result<()> {
        identity.validate()?;
        let bytes = serde_json::to_vec_pretty(identity)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(RuntimeError::InvalidInput(
                "client identity exceeds limit".into(),
            ));
        }
        let target = self.root.join(FILE);
        if target.symlink_metadata().is_ok_and(|m| !m.is_file()) {
            return Err(RuntimeError::InvalidInput(
                "client identity must be a regular file".into(),
            ));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(target).map_err(|e| e.error)?;
        Ok(())
    }

    pub fn discover_oauth(
        &self,
        resource: &str,
        metadata_url: Option<&str>,
        client_id: &str,
        scopes: Vec<String>,
    ) -> std::result::Result<OAuthConfig, apr_connectors::DispatchError> {
        if !client_id.is_empty() {
            return apr_connectors::oauth::discover(
                resource,
                metadata_url,
                client_id,
                scopes,
                false,
            );
        }
        let identity = self
            .oauth_client_identity()
            .map_err(|_| apr_connectors::DispatchError::InvalidConnection)?;
        let profiles = identity
            .pre_registered_clients
            .into_iter()
            .map(|c| PreRegisteredClient {
                issuer: c.issuer,
                client_id: c.client_id,
            })
            .collect::<Vec<_>>();
        apr_connectors::oauth::discover_with_client_identity(
            resource,
            metadata_url,
            &profiles,
            identity.client_id_metadata_url.as_deref(),
            scopes,
            false,
        )
    }
}
