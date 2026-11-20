//! Protected, bounded, one-attempt JSON HTTP transport. Runtime owns permission,
//! durable intent, budgets, retry and reconciliation.

mod file_vault;
pub mod mcp;
pub mod metering;
pub mod oauth;
pub use file_vault::FileVault;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use ureq::http::{self, Uri};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum DispatchError {
    #[error("invalid connection configuration")]
    InvalidConnection,
    #[error("request does not match the connection contract")]
    InvalidInput,
    #[error("protected credential storage is unavailable or the binding is missing")]
    CredentialUnavailable,
    #[error(
        "Browser sign-in for this service needs a CookBook provider integration; no account was connected"
    )]
    OAuthRegistrationUnsupported,
    #[error(
        "OAuth client registration failed or returned incompatible client metadata; no account was connected"
    )]
    OAuthRegistrationFailed,
    #[error(
        "OAuth client authentication method differs from the reviewed configuration; rediscover the exact endpoint in Browser sign-in and retry. No account was connected"
    )]
    OAuthRegistrationMethodMismatch,
    #[error("browser sign-in expired after five minutes; start a new sign-in in CookBook")]
    OAuthAuthorizationExpired,
    #[error("browser sign-in was cancelled; no account was connected")]
    OAuthAuthorizationCancelled,
    #[error("OAuth authorization is unavailable or expired; reconnect this account in CookBook")]
    OAuthReconnectRequired,
    #[error("OAuth provider is unavailable; retry later or inspect the account in CookBook")]
    OAuthTokenUnavailable,
    #[error(
        "MCP server returned HTTP {0}; check the selected protocol and account access before retrying discovery"
    )]
    McpHttpStatus(u16),
    #[error("request may have executed; acknowledgement requires reconciliation")]
    Uncertain,
}
impl DispatchError {
    pub fn may_have_executed(&self) -> bool {
        matches!(self, Self::Uncertain)
    }
}

/// No Debug or Serialize implementation: this value is confined to transport.
pub struct Secret(Zeroizing<Vec<u8>>);
impl Secret {
    pub fn new(bytes: Vec<u8>) -> Result<Self, DispatchError> {
        let bytes = Zeroizing::new(bytes);
        if bytes.is_empty()
            || bytes.len() > 8192
            || !bytes.iter().all(|b| (0x21..=0x7e).contains(b))
        {
            return Err(DispatchError::CredentialUnavailable);
        }
        Ok(Self(bytes))
    }
    /// Explicit short-lived broker copy; both buffers zeroize on drop.
    pub fn duplicate(&self) -> Result<Self, DispatchError> {
        Self::new(self.0.to_vec())
    }
    fn text(&self) -> &str {
        std::str::from_utf8(&self.0).expect("validated ASCII secret")
    }
}

/// A refresh guard is held only during token read/rotate/persist, never API work.
pub trait CredentialRefreshGuard {}
impl<T> CredentialRefreshGuard for T {}

/// Credential backend boundary for protected transports.
pub trait CredentialStore: Send + Sync {
    fn get(&self, reference: &str) -> Result<Secret, DispatchError>;
    fn replace(&self, _reference: &str, _secret: Secret) -> Result<(), DispatchError> {
        Err(DispatchError::CredentialUnavailable)
    }
    fn lock_refresh(
        &self,
        _reference: &str,
    ) -> Result<Box<dyn CredentialRefreshGuard + '_>, DispatchError> {
        // Non-file backends keep a conservative process-wide fallback. The
        // production FileVault overrides this with a per-reference file lock.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        Ok(Box::new(
            LOCK.lock()
                .map_err(|_| DispatchError::CredentialUnavailable)?,
        ))
    }
    /// Atomic compare-and-replace. Unsupported stores fail closed on rotation.
    fn replace_if_matches(
        &self,
        _reference: &str,
        _expected: &Secret,
        _next: Secret,
    ) -> Result<(), DispatchError> {
        Err(DispatchError::CredentialUnavailable)
    }
}

/// Native credential backend retained only for an explicit owner-led migration.
pub struct PlatformVault {
    namespace: String,
}
impl PlatformVault {
    pub fn new(namespace: &str) -> Result<Self, DispatchError> {
        if !identifier(namespace) {
            return Err(DispatchError::InvalidConnection);
        }
        Ok(Self {
            namespace: format!("ai.apr.{namespace}"),
        })
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn entry(&self, reference: &str) -> Result<keyring::Entry, DispatchError> {
        if !identifier(reference) {
            return Err(DispatchError::InvalidConnection);
        }
        keyring::Entry::new(&self.namespace, reference)
            .map_err(|_| DispatchError::CredentialUnavailable)
    }

    pub fn set(&self, reference: &str, secret: Secret) -> Result<(), DispatchError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.entry(reference)?
                .set_secret(&secret.0)
                .map_err(|_| DispatchError::CredentialUnavailable)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (reference, secret);
            Err(DispatchError::CredentialUnavailable)
        }
    }

    pub fn remove(&self, reference: &str) -> Result<(), DispatchError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.entry(reference)?
                .delete_credential()
                .map_err(|_| DispatchError::CredentialUnavailable)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = reference;
            Err(DispatchError::CredentialUnavailable)
        }
    }
}
impl CredentialStore for PlatformVault {
    fn replace(&self, reference: &str, secret: Secret) -> Result<(), DispatchError> {
        self.set(reference, secret)
    }
    fn get(&self, reference: &str) -> Result<Secret, DispatchError> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            Secret::new(
                self.entry(reference)?
                    .get_secret()
                    .map_err(|_| DispatchError::CredentialUnavailable)?,
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = reference;
            Err(DispatchError::CredentialUnavailable)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Auth {
    Oauth {
        credential_ref: String,
        config: oauth::OAuthConfig,
    },
    None,
    Bearer {
        credential_ref: String,
    },
    Header {
        credential_ref: String,
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// Selected MCP tool and exact transport. Absent retains direct JSON HTTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<mcp::McpBinding>,
    /// Optional owner-approved mapping and frozen rate card. No credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metering: Option<metering::MeteringConfig>,
    pub label: String,
    /// Optional nonsecret description of the credential kind, shown to owners and agents.
    /// It is valid only for bearer or API-key header authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_label: Option<String>,
    /// Exact owner-approved destination, never substituted from workflow input.
    pub endpoint: String,
    pub method: String,
    pub auth: Auth,
    pub input_schema: Value,
    pub output_schema: Value,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
    #[serde(default)]
    pub loopback_test: bool,
}

impl Connection {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if let Some(metering) = &self.metering {
            metering
                .validate()
                .map_err(|_| DispatchError::InvalidConnection)?;
            if let Some(analysis) = &metering.analysis
                && (apr_core::content_digest(&self.input_schema)
                    .map_err(|_| DispatchError::InvalidConnection)?
                    != analysis.request_schema_digest
                    || apr_core::content_digest(&self.output_schema)
                        .map_err(|_| DispatchError::InvalidConnection)?
                        != analysis.response_schema_digest)
            {
                return Err(DispatchError::InvalidConnection);
            }
        }
        let stdio = self.mcp.as_ref().is_some_and(mcp::McpBinding::is_stdio);
        if let Some(mcp) = &self.mcp {
            if self.method != "POST" {
                return Err(DispatchError::InvalidConnection);
            }
            mcp.validate()?;
            if self.method != "POST" {
                return Err(DispatchError::InvalidConnection);
            }
            if stdio {
                if self.endpoint != "stdio:local" || self.loopback_test {
                    return Err(DispatchError::InvalidConnection);
                }
                if let mcp::Transport::Stdio { credential_env, .. } = &mcp.transport
                    && credential_env.is_some() != !matches!(self.auth, Auth::None)
                {
                    return Err(DispatchError::InvalidConnection);
                }
            }
        }
        let endpoint = if stdio {
            "https://stdio.invalid"
        } else {
            &self.endpoint
        };
        let uri: Uri = endpoint
            .parse()
            .map_err(|_| DispatchError::InvalidConnection)?;
        let host = uri.host().ok_or(DispatchError::InvalidConnection)?;
        if self.label.is_empty()
            || self.label.len() > 128
            || self.endpoint.len() > 2048
            || self.endpoint.contains(['@', '#'])
            || uri.authority().is_none()
            || !(1..=120_000).contains(&self.timeout_ms)
            || !(1..=64 * 1024).contains(&self.max_response_bytes)
            || !matches!(
                self.method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
            )
        {
            return Err(DispatchError::InvalidConnection);
        }
        if self.credential_label.as_deref().is_some_and(|label| {
            label.trim().is_empty()
                || label.len() > 160
                || label.chars().any(char::is_control)
                || !matches!(self.auth, Auth::Bearer { .. } | Auth::Header { .. })
        }) {
            return Err(DispatchError::InvalidConnection);
        }
        let ip = host.trim_matches(['[', ']']).parse::<IpAddr>().ok();
        if self.loopback_test {
            if !cfg!(feature = "test-http")
                || uri.scheme_str() != Some("http")
                || !ip.is_some_and(|ip| ip.is_loopback())
            {
                return Err(DispatchError::InvalidConnection);
            }
        } else if uri.scheme_str() != Some("https")
            || ip.is_some_and(|ip| !public_ip(ip))
            || host.eq_ignore_ascii_case("localhost")
        {
            return Err(DispatchError::InvalidConnection);
        }
        match &self.auth {
            Auth::None => {}
            Auth::Oauth {
                credential_ref,
                config,
            } if identifier(credential_ref) => {
                config.validate()?;
                if stdio
                    || config.resource != self.endpoint
                    || config.loopback_test != self.loopback_test
                {
                    return Err(DispatchError::InvalidConnection);
                }
            }
            Auth::Bearer { credential_ref } if identifier(credential_ref) => {}
            Auth::Header {
                credential_ref,
                name,
            } if identifier(credential_ref) => {
                let header = http::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| DispatchError::InvalidConnection)?;
                if !matches!(header.as_str(), "x-api-key" | "api-key") {
                    return Err(DispatchError::InvalidConnection);
                }
            }
            _ => return Err(DispatchError::InvalidConnection),
        }
        for schema in [&self.input_schema, &self.output_schema] {
            if serde_json::to_vec(schema)
                .map_err(|_| DispatchError::InvalidConnection)?
                .len()
                > 16 * 1024
            {
                return Err(DispatchError::InvalidConnection);
            }
            if self.mcp.is_some() {
                apr_core::validate_service_schema(schema)
            } else {
                apr_core::validate_schema(schema)
            }
            .map_err(|_| DispatchError::InvalidConnection)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DispatchOutput {
    pub output: Value,
    pub status: u16,
    pub response_bytes: usize,
    pub elapsed_ms: u64,
}

pub struct HttpBroker<V> {
    vault: V,
}
impl<V: CredentialStore> HttpBroker<V> {
    pub fn new(vault: V) -> Self {
        Self { vault }
    }
    pub fn dispatch(
        &self,
        connection: &Connection,
        input: &Value,
    ) -> Result<DispatchOutput, DispatchError> {
        self.dispatch_cancellable(connection, input, None)
    }
    pub fn dispatch_cancellable(
        &self,
        connection: &Connection,
        input: &Value,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<DispatchOutput, DispatchError> {
        connection.validate()?;
        if connection.mcp.is_some() {
            return mcp::dispatch(connection, input, &self.vault, cancel);
        }
        let mut bytes = serde_json::to_vec(input).map_err(|_| DispatchError::InvalidInput)?;
        if bytes.len() > 64 * 1024 {
            return Err(DispatchError::InvalidInput);
        }
        apr_core::validate_instance(&connection.input_schema, input)
            .map_err(|_| DispatchError::InvalidInput)?;
        if connection.method == "GET" {
            if !input.as_object().is_some_and(|object| object.is_empty()) {
                return Err(DispatchError::InvalidInput);
            }
            bytes.clear();
        }
        let secret = oauth::resolve(connection, &self.vault)?;
        let mut request = http::Request::builder()
            .method(connection.method.as_str())
            .uri(&connection.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        if let Some(secret) = &secret {
            match &connection.auth {
                Auth::Bearer { .. } | Auth::Oauth { .. } => {
                    request = request.header("Authorization", format!("Bearer {}", secret.text()))
                }
                Auth::Header { name, .. } => request = request.header(name, secret.text()),
                Auth::None => {}
            }
        }
        let request = request
            .body(bytes)
            .map_err(|_| DispatchError::InvalidConnection)?;
        let deadline = Duration::from_millis(connection.timeout_ms);
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .https_only(!connection.loopback_test)
            .max_redirects(0)
            .http_status_as_error(false)
            .max_idle_connections(0)
            .timeout_global(Some(deadline))
            .timeout_resolve(Some(deadline))
            .timeout_connect(Some(deadline))
            .timeout_recv_body(Some(deadline))
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            DefaultConnector::default(),
            SafeResolver {
                loopback: connection.loopback_test,
            },
        );
        let start = Instant::now();
        // Once handed to transport, conservatively classify every failure as
        // uncertain. No adapter retries, even for read-looking POST operations.
        let mut response = agent.run(request).map_err(|_| DispatchError::Uncertain)?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(DispatchError::Uncertain);
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(connection.max_response_bytes as u64)
            .read_to_vec()
            .map_err(|_| DispatchError::Uncertain)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| DispatchError::Uncertain)?;
        if secret
            .as_ref()
            .is_some_and(|s| contains_secret(&value, s.text()))
        {
            return Err(DispatchError::Uncertain);
        }
        apr_core::validate_instance(&connection.output_schema, &value)
            .map_err(|_| DispatchError::Uncertain)?;
        Ok(DispatchOutput {
            output: value,
            status,
            response_bytes: bytes.len(),
            elapsed_ms: start.elapsed().as_millis().min(u64::MAX as u128) as u64,
        })
    }
}

fn identifier(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}
fn contains_secret(value: &Value, secret: &str) -> bool {
    match value {
        Value::String(text) => text.contains(secret),
        Value::Array(items) => items.iter().any(|v| contains_secret(v, secret)),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| k.contains(secret) || contains_secret(v, secret)),
        _ => false,
    }
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (18..=19).contains(&b)))
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(ip));
            }
            let segments = ip.segments();
            // Permit global unicast only, excluding documentation/transition prefixes.
            (segments[0] & 0xe000) == 0x2000
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && (segments[1] < 0x200 || segments[1] == 0xdb8))
        }
    }
}
#[derive(Debug)]
struct SafeResolver {
    loopback: bool,
}
impl Resolver for SafeResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addresses = DefaultResolver::default().resolve(uri, config, timeout)?;
        // The validated addresses are used directly by this request's connector;
        // no second unchecked DNS resolution can change the destination.
        if addresses.iter().any(|addr| {
            if self.loopback {
                !addr.ip().is_loopback()
            } else {
                !public_ip(addr.ip())
            }
        }) {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(addresses)
    }
}
