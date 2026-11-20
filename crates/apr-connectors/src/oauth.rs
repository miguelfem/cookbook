//! Native OAuth with PKCE S256. No tokens, codes or provider bodies are
//! returned to the inspector. Metadata and token transport share SSRF controls.
use crate::{Auth, Connection, CredentialStore, DispatchError, SafeResolver, Secret};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use ureq::unversioned::transport::DefaultConnector;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OAuthConfig {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: String,
    /// Public registration policy only. Issued client credentials stay in the vault.
    /// Omission preserves the digest of existing static-client token bundles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration: Option<ClientRegistration>,
    /// A configured, hosted public client identity. Omitted for legacy static
    /// clients and DCR so their saved account digests remain unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id_metadata_url: Option<String>,
    pub resource: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub require_issuer_response: bool,
    #[serde(default)]
    pub loopback_test: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClientRegistration {
    pub endpoint: String,
    pub token_endpoint_auth_method: ClientAuthMethod,
}
/// A shipped public client ID, bound to the exact authorization-server issuer.
/// Client secrets are never part of a public profile.
#[derive(Debug, Clone, PartialEq)]
pub struct PreRegisteredClient {
    pub issuer: String,
    pub client_id: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuthMethod {
    None,
    ClientSecretPost,
    ClientSecretBasic,
}
impl ClientAuthMethod {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ClientSecretPost => "client_secret_post",
            Self::ClientSecretBasic => "client_secret_basic",
        }
    }
}
impl OAuthConfig {
    pub fn validate(&self) -> Result<(), DispatchError> {
        for endpoint in [
            &self.issuer,
            &self.authorization_endpoint,
            &self.token_endpoint,
            &self.resource,
        ] {
            public_url(endpoint, self.loopback_test)?;
        }
        let issuer = Url::parse(&self.issuer).map_err(|_| DispatchError::InvalidConnection)?;
        if let Some(registration) = &self.registration {
            public_url(&registration.endpoint, self.loopback_test)?;
            if !self.client_id.is_empty() {
                return Err(DispatchError::InvalidConnection);
            }
        }
        if let Some(metadata_url) = &self.client_id_metadata_url {
            validate_metadata_url(metadata_url, self.loopback_test)?;
            if self.registration.is_some() || self.client_id != *metadata_url {
                return Err(DispatchError::InvalidConnection);
            }
        }
        if issuer.query().is_some()
            || (self.client_id.is_empty() && self.registration.is_none())
            || self.client_id.len() > 2048
            || self.client_id.chars().any(char::is_control)
            || self.scopes.len() > 64
            || self.scopes.iter().any(|s| {
                s.is_empty()
                    || s.len() > 128
                    || !s
                        .bytes()
                        .all(|b| (0x21..=0x7e).contains(&b) && b != b'"' && b != b'\\')
            })
        {
            return Err(DispatchError::InvalidConnection);
        }
        Ok(())
    }
}
pub fn public_url(value: &str, test: bool) -> Result<Url, DispatchError> {
    let url = Url::parse(value).map_err(|_| DispatchError::InvalidConnection)?;
    let ip = url
        .host_str()
        .and_then(|h| h.trim_matches(['[', ']']).parse().ok());
    if value.len() > 4096
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || url.host_str() == Some("localhost")
        || if test {
            !cfg!(feature = "test-http")
                || url.scheme() != "http"
                || !ip.is_some_and(|i: std::net::IpAddr| i.is_loopback())
        } else {
            url.scheme() != "https" || ip.is_some_and(|i| !crate::public_ip(i))
        }
    {
        return Err(DispatchError::InvalidConnection);
    }
    Ok(url)
}
fn validate_metadata_url(value: &str, test: bool) -> Result<(), DispatchError> {
    let url = public_url(value, test)?;
    if value.len() > 2048 || url.path().is_empty() || url.path() == "/" || url.query().is_some() {
        return Err(DispatchError::InvalidConnection);
    }
    Ok(())
}
/// Only an exact, fixed IPv4 loopback callback published by the client may be
/// used. CIMD redirects require string equality; a dynamic port is not a match.
fn fixed_loopback_port(value: &str) -> Option<u16> {
    let url = Url::parse(value).ok()?;
    let port = url.port()?;
    (url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.path() == "/oauth/callback"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && value == format!("http://127.0.0.1:{port}/oauth/callback"))
    .then_some(port)
}
/// Fetching this public document grants no account or operation authority.
/// It verifies that the identity we send can actually use our native callback.
fn client_metadata_redirects(client_id: &str, test: bool) -> Result<Vec<String>, DispatchError> {
    validate_metadata_url(client_id, test)?;
    let metadata = exchange_authenticated(client_id, None, test, None, true)?;
    let name = metadata["client_name"]
        .as_str()
        .ok_or(DispatchError::InvalidConnection)?;
    let redirects = metadata["redirect_uris"]
        .as_array()
        .ok_or(DispatchError::InvalidConnection)?;
    if metadata["client_id"].as_str() != Some(client_id)
        || name.is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
        || redirects.is_empty()
        || redirects.len() > 16
        || metadata.get("client_secret").is_some()
        || metadata.get("client_secret_expires_at").is_some()
        || metadata["token_endpoint_auth_method"] != "none"
        || metadata.get("grant_types").is_some_and(|v| {
            !v.as_array()
                .is_some_and(|a| a.iter().any(|v| v == "authorization_code"))
        })
        || metadata
            .get("response_types")
            .is_some_and(|v| !v.as_array().is_some_and(|a| a.iter().any(|v| v == "code")))
    {
        return Err(DispatchError::InvalidConnection);
    }
    let redirects = redirects
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or(DispatchError::InvalidConnection)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !redirects.iter().any(|v| fixed_loopback_port(v).is_some()) {
        return Err(DispatchError::InvalidConnection);
    }
    Ok(redirects)
}
/// Bounded JSON/form exchange, no redirects/proxy/ambient cookies or retries.
fn exchange(url: &str, body: Option<(&str, Vec<u8>)>, test: bool) -> Result<Value, DispatchError> {
    exchange_authenticated(url, body, test, None, false)
}
fn exchange_authenticated(
    url: &str,
    body: Option<(&str, Vec<u8>)>,
    test: bool,
    authorization: Option<&str>,
    exact_ok: bool,
) -> Result<Value, DispatchError> {
    public_url(url, test)?;
    let mut req = ureq::http::Request::builder()
        .uri(url)
        .header("Accept", "application/json");
    if let Some(value) = authorization {
        req = req.header("Authorization", value);
    }
    let bytes = Zeroizing::new(if let Some((kind, bytes)) = body {
        req = req.method("POST").header("Content-Type", kind);
        bytes
    } else {
        req = req.method("GET");
        Vec::new()
    });
    let config = ureq::Agent::config_builder()
        .proxy(None)
        .https_only(!test)
        .max_redirects(0)
        .http_status_as_error(false)
        .max_idle_connections(0)
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        DefaultConnector::default(),
        SafeResolver { loopback: test },
    );
    let mut response = agent
        .run(
            req.body(bytes.as_slice())
                .map_err(|_| DispatchError::InvalidConnection)?,
        )
        .map_err(|_| DispatchError::CredentialUnavailable)?;
    if response.status().as_u16() != 200 && (exact_ok || response.status().as_u16() != 201) {
        if matches!(response.status().as_u16(), 400 | 401 | 403) {
            return Err(DispatchError::OAuthReconnectRequired);
        }
        return Err(DispatchError::CredentialUnavailable);
    }
    let bytes = Zeroizing::new(
        response
            .body_mut()
            .with_config()
            .limit(65536)
            .read_to_vec()
            .map_err(|_| DispatchError::CredentialUnavailable)?,
    );
    serde_json::from_slice(&bytes).map_err(|_| DispatchError::CredentialUnavailable)
}
fn token_form(
    cfg: &OAuthConfig,
    mut fields: Vec<(&str, String)>,
    client: Option<&RegisteredClient>,
) -> Result<Value, DispatchError> {
    validate_client(cfg, client)?;
    fields.push((
        "client_id",
        client.map_or(&cfg.client_id, |c| &c.id).clone(),
    ));
    let mut authorization = None;
    if let Some(client) = client {
        match client.method {
            ClientAuthMethod::None => {}
            ClientAuthMethod::ClientSecretPost => fields.push((
                "client_secret",
                client
                    .secret
                    .as_ref()
                    .ok_or(DispatchError::OAuthReconnectRequired)?
                    .clone(),
            )),
            ClientAuthMethod::ClientSecretBasic => {
                let encode = |s: &str| {
                    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>()
                };
                let raw = Zeroizing::new(format!(
                    "{}:{}",
                    encode(&client.id),
                    encode(
                        client
                            .secret
                            .as_deref()
                            .ok_or(DispatchError::OAuthReconnectRequired)?
                    )
                ));
                authorization = Some(Zeroizing::new(format!(
                    "Basic {}",
                    STANDARD.encode(raw.as_bytes())
                )));
            }
        }
    }
    let body = Zeroizing::new(
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&fields)
            .finish(),
    );
    for (_, value) in &mut fields {
        value.zeroize();
    }
    exchange_authenticated(
        &cfg.token_endpoint,
        Some((
            "application/x-www-form-urlencoded",
            body.as_bytes().to_vec(),
        )),
        cfg.loopback_test,
        authorization.as_ref().map(|s| s.as_str()),
        false,
    )
    .map_err(|error| match error {
        DispatchError::OAuthReconnectRequired => error,
        _ => DispatchError::OAuthTokenUnavailable,
    })
}
/// Only a 401 challenge from the exact public resource can provide metadata.
/// The challenge is never a redirect and never receives account credentials.
fn challenge_metadata(resource: &str, test: bool) -> Option<String> {
    public_url(resource, test).ok()?;
    let config = ureq::Agent::config_builder()
        .proxy(None)
        .https_only(!test)
        .max_redirects(0)
        .http_status_as_error(false)
        .max_idle_connections(0)
        .timeout_global(Some(Duration::from_secs(10)))
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        DefaultConnector::default(),
        SafeResolver { loopback: test },
    );
    let response = agent
        .run(
            ureq::http::Request::builder()
                .method("GET")
                .uri(resource)
                .header("Accept", "application/json, text/event-stream")
                .body(Vec::<u8>::new())
                .ok()?,
        )
        .ok()?;
    if response.status().as_u16() != 401 {
        return None;
    }
    for h in response.headers().get_all("www-authenticate") {
        let h = h.to_str().ok()?;
        if h.len() > 8192 {
            continue;
        }
        // Quoted URLs cannot contain a comma; escaped values are rejected.
        for part in h.split(',') {
            let part = part.trim().strip_prefix("Bearer ").unwrap_or(part.trim());
            if let Some(("resource_metadata", value)) = part.split_once('=') {
                let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
                if value.contains('\\') || public_url(value, test).is_err() {
                    return None;
                }
                return Some(value.to_string());
            }
        }
    }
    None
}
/// Public metadata only. An empty client ID selects advertised dynamic
/// registration; registration itself happens only during owner authorization.
pub fn discover(
    resource: &str,
    metadata_url: Option<&str>,
    client_id: &str,
    scopes: Vec<String>,
    test: bool,
) -> Result<OAuthConfig, DispatchError> {
    discover_selected(resource, metadata_url, client_id, &[], None, scopes, test)
}
/// Select only configured public identities. A profile is tied to the exact
/// discovered issuer; a hosted identity is used only when the server advertises
/// CIMD. Without either, advertised DCR remains the fallback.
pub fn discover_with_client_identity(
    resource: &str,
    metadata_url: Option<&str>,
    pre_registered_clients: &[PreRegisteredClient],
    hosted_client_id_metadata_url: Option<&str>,
    scopes: Vec<String>,
    test: bool,
) -> Result<OAuthConfig, DispatchError> {
    discover_selected(
        resource,
        metadata_url,
        "",
        pre_registered_clients,
        hosted_client_id_metadata_url,
        scopes,
        test,
    )
}
fn discover_selected(
    resource: &str,
    metadata_url: Option<&str>,
    explicit_client_id: &str,
    pre_registered_clients: &[PreRegisteredClient],
    hosted_client_id_metadata_url: Option<&str>,
    scopes: Vec<String>,
    test: bool,
) -> Result<OAuthConfig, DispatchError> {
    let resource_url = public_url(resource, test)?;
    let origin = resource_url.origin().ascii_serialization();
    let path = resource_url.path().trim_end_matches('/');
    let candidates = if let Some(uri) = metadata_url {
        vec![uri.to_owned()]
    } else {
        let mut urls = Vec::new();
        if let Some(uri) = challenge_metadata(resource, test) {
            urls.push(uri);
        }
        urls.push(format!(
            "{origin}/.well-known/oauth-protected-resource{path}"
        ));
        urls.push(format!("{origin}/.well-known/oauth-protected-resource"));
        urls
    };
    let metadata = candidates
        .iter()
        .find_map(|u| exchange(u, None, test).ok())
        .ok_or(DispatchError::CredentialUnavailable)?;
    if metadata["resource"]
        .as_str()
        .and_then(|r| Url::parse(r).ok())
        != Some(resource_url)
    {
        return Err(DispatchError::InvalidConnection);
    }
    let issuer = metadata["authorization_servers"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .ok_or(DispatchError::InvalidConnection)?;
    let issuer_url = public_url(issuer, test)?;
    let base = issuer_url.origin().ascii_serialization();
    let path = issuer_url.path().trim_end_matches('/');
    let endpoints = [
        format!("{base}/.well-known/oauth-authorization-server{path}"),
        format!("{base}/.well-known/openid-configuration{path}"),
        format!("{issuer}/.well-known/openid-configuration"),
    ];
    let server = endpoints
        .iter()
        .find_map(|u| exchange(u, None, test).ok())
        .ok_or(DispatchError::CredentialUnavailable)?;
    if server["issuer"].as_str() != Some(issuer)
        || !server["code_challenge_methods_supported"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "S256"))
    {
        return Err(DispatchError::InvalidConnection);
    }
    let selected_profile = if explicit_client_id.is_empty() {
        pre_registered_clients.iter().find(|p| p.issuer == issuer)
    } else {
        None
    };
    if selected_profile.is_some_and(|p| p.client_id.is_empty()) {
        return Err(DispatchError::InvalidConnection);
    }
    let client_id = if explicit_client_id.is_empty() {
        selected_profile.map_or("", |p| p.client_id.as_str())
    } else {
        explicit_client_id
    };
    let client_id_metadata_url =
        if client_id.is_empty() && server["client_id_metadata_document_supported"] == true {
            if let Some(url) = hosted_client_id_metadata_url {
                client_metadata_redirects(url, test)?;
                Some(url.to_owned())
            } else {
                None
            }
        } else {
            None
        };
    let client_id = client_id_metadata_url.as_deref().unwrap_or(client_id);
    let registration = if client_id.is_empty() {
        let endpoint = server["registration_endpoint"]
            .as_str()
            .ok_or(DispatchError::OAuthRegistrationUnsupported)?;
        public_url(endpoint, test)?;
        let methods = server["token_endpoint_auth_methods_supported"].as_array();
        let method = if methods.is_some_and(|a| a.iter().any(|v| v == "none")) {
            ClientAuthMethod::None
        } else if methods.is_none_or(|a| a.iter().any(|v| v == "client_secret_basic")) {
            // Prefer RFC 7591's default when supported: some providers omit
            // this field from their successful registration response.
            ClientAuthMethod::ClientSecretBasic
        } else if methods.is_some_and(|a| a.iter().any(|v| v == "client_secret_post")) {
            ClientAuthMethod::ClientSecretPost
        } else {
            return Err(DispatchError::OAuthRegistrationUnsupported);
        };
        Some(ClientRegistration {
            endpoint: endpoint.into(),
            token_endpoint_auth_method: method,
        })
    } else {
        None
    };
    let scopes = if scopes.is_empty() {
        match metadata.get("scopes_supported") {
            None => scopes,
            Some(Value::Array(values)) => values
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or(DispatchError::InvalidConnection)
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(DispatchError::InvalidConnection),
        }
    } else {
        scopes
    };
    let cfg = OAuthConfig {
        issuer: issuer.into(),
        authorization_endpoint: server["authorization_endpoint"]
            .as_str()
            .ok_or(DispatchError::InvalidConnection)?
            .into(),
        token_endpoint: server["token_endpoint"]
            .as_str()
            .ok_or(DispatchError::InvalidConnection)?
            .into(),
        client_id: client_id.into(),
        registration,
        client_id_metadata_url,
        resource: resource.into(),
        scopes,
        require_issuer_response: server["authorization_response_iss_parameter_supported"]
            .as_bool()
            .unwrap_or(false),
        loopback_test: test,
    };
    cfg.validate()?;
    Ok(cfg)
}
fn random() -> Result<String, DispatchError> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| DispatchError::CredentialUnavailable)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tokens {
    access: String,
    refresh: Option<String>,
    expires: u64,
    config_digest: String,
    scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client: Option<RegisteredClient>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisteredClient {
    id: String,
    secret: Option<String>,
    method: ClientAuthMethod,
    secret_expires_at: u64,
}
impl Drop for RegisteredClient {
    fn drop(&mut self) {
        if let Some(secret) = &mut self.secret {
            secret.zeroize();
        }
    }
}
fn validate_client(
    cfg: &OAuthConfig,
    client: Option<&RegisteredClient>,
) -> Result<(), DispatchError> {
    match (&cfg.registration, client) {
        (None, None) if !cfg.client_id.is_empty() => Ok(()),
        (Some(registration), Some(client))
            if registration.token_endpoint_auth_method == client.method =>
        {
            if client.id.is_empty()
                || client.id.len() > 2048
                || client.id.chars().any(char::is_control)
                || (client.secret_expires_at != 0 && client.secret_expires_at <= now())
            {
                return Err(DispatchError::OAuthReconnectRequired);
            }
            match (client.method, &client.secret) {
                (ClientAuthMethod::None, None) => Ok(()),
                (
                    ClientAuthMethod::ClientSecretPost | ClientAuthMethod::ClientSecretBasic,
                    Some(secret),
                ) => Secret::new(secret.as_bytes().to_vec()).map(|_| ()),
                _ => Err(DispatchError::OAuthReconnectRequired),
            }
        }
        _ => Err(DispatchError::OAuthReconnectRequired),
    }
}
fn register_client(
    cfg: &OAuthConfig,
    redirect: &str,
) -> Result<Option<RegisteredClient>, DispatchError> {
    let Some(registration) = &cfg.registration else {
        return Ok(None);
    };
    let body = json!({"client_name":"CookBook APR", "application_type":"native", "redirect_uris":[redirect], "grant_types":["authorization_code","refresh_token"], "response_types":["code"], "token_endpoint_auth_method":registration.token_endpoint_auth_method.name(), "scope":cfg.scopes.join(" ")});
    let mut response = exchange(
        &registration.endpoint,
        Some((
            "application/json",
            serde_json::to_vec(&body).map_err(|_| DispatchError::OAuthRegistrationFailed)?,
        )),
        cfg.loopback_test,
    )
    .map_err(|_| DispatchError::OAuthRegistrationFailed)?;
    let result = (|| {
        if response["redirect_uris"] != json!([redirect])
            || response
                .get("application_type")
                .is_some_and(|v| v != "native")
            || response
                .get("response_types")
                .is_some_and(|v| *v != json!(["code"]))
            || response.get("grant_types").is_some_and(|v| {
                !v.as_array().is_some_and(|a| {
                    a.iter().any(|v| v == "authorization_code")
                        && a.iter()
                            .all(|v| v == "authorization_code" || v == "refresh_token")
                })
            })
            || response.get("scope").is_some_and(|v| {
                v.as_str()
                    .map(|s| s.split_ascii_whitespace().collect::<BTreeSet<_>>())
                    != Some(cfg.scopes.iter().map(String::as_str).collect())
            })
        {
            return Err(DispatchError::OAuthRegistrationFailed);
        }
        let method: ClientAuthMethod = serde_json::from_value(
            response
                .get("token_endpoint_auth_method")
                .cloned()
                .unwrap_or(json!("client_secret_basic")),
        )
        .map_err(|_| DispatchError::OAuthRegistrationUnsupported)?;
        if method != registration.token_endpoint_auth_method {
            return Err(DispatchError::OAuthRegistrationMethodMismatch);
        }
        let client = RegisteredClient {
            id: response["client_id"]
                .as_str()
                .ok_or(DispatchError::OAuthRegistrationFailed)?
                .to_owned(),
            secret: match response.get("client_secret") {
                Some(Value::String(s)) => Some(s.clone()),
                None => None,
                _ => return Err(DispatchError::OAuthRegistrationFailed),
            },
            method,
            secret_expires_at: match response.get("client_secret_expires_at") {
                None => 0,
                Some(v) => v.as_u64().ok_or(DispatchError::OAuthRegistrationFailed)?,
            },
        };
        validate_client(cfg, Some(&client)).map_err(|_| DispatchError::OAuthRegistrationFailed)?;
        Ok(Some(client))
    })();
    scrub(&mut response);
    result
}
impl Drop for Tokens {
    fn drop(&mut self) {
        self.access.zeroize();
        if let Some(s) = &mut self.refresh {
            s.zeroize();
        }
    }
}
fn digest(cfg: &OAuthConfig) -> Result<String, DispatchError> {
    apr_core::content_digest(
        &serde_json::to_value(cfg).map_err(|_| DispatchError::InvalidConnection)?,
    )
    .map_err(|_| DispatchError::InvalidConnection)
}
fn tokens(
    cfg: &OAuthConfig,
    value: &Value,
    prior: Option<&Tokens>,
) -> Result<Tokens, DispatchError> {
    let access = value["access_token"]
        .as_str()
        .ok_or(DispatchError::CredentialUnavailable)?
        .to_owned();
    Secret::new(access.as_bytes().to_vec())?;
    if !value["token_type"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("bearer"))
    {
        return Err(DispatchError::CredentialUnavailable);
    }
    let expires = value["expires_in"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 366 * 86400)
        .ok_or(DispatchError::CredentialUnavailable)?;
    let scopes = value["scope"]
        .as_str()
        .map(|s| {
            s.split_ascii_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            prior
                .map(|p| p.scope.clone())
                .unwrap_or_else(|| cfg.scopes.clone())
        });
    let expected: BTreeSet<_> = cfg.scopes.iter().collect();
    if scopes.iter().any(|s| !expected.contains(s))
        || cfg.scopes.iter().any(|s| !scopes.contains(s))
    {
        return Err(DispatchError::CredentialUnavailable);
    }
    let refresh = match value.get("refresh_token") {
        Some(Value::String(s)) => {
            Secret::new(s.as_bytes().to_vec())?;
            Some(s.clone())
        }
        Some(_) => return Err(DispatchError::CredentialUnavailable),
        None => prior.and_then(|t| t.refresh.clone()),
    };
    Ok(Tokens {
        access,
        refresh,
        expires: now().saturating_add(expires),
        config_digest: digest(cfg)?,
        scope: scopes,
        client: prior.and_then(|p| p.client.clone()),
    })
}
fn seal(tokens: &Tokens) -> Result<Secret, DispatchError> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(tokens).map_err(|_| DispatchError::CredentialUnavailable)?,
    );
    Secret::new(URL_SAFE_NO_PAD.encode(&*bytes).into_bytes())
}
/// Verify an owner-provided vault blob before storing it. Never interpret an API
/// key as an OAuth token bundle or permit identity changes on refresh.
pub fn validate_bundle(cfg: &OAuthConfig, secret: &Secret) -> Result<(), DispatchError> {
    let bytes = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(secret.text())
            .map_err(|_| DispatchError::CredentialUnavailable)?,
    );
    let saved: Tokens =
        serde_json::from_slice(&bytes).map_err(|_| DispatchError::CredentialUnavailable)?;
    if saved.config_digest != digest(cfg)?
        || saved.expires <= now()
        || saved.scope.iter().collect::<BTreeSet<_>>() != cfg.scopes.iter().collect::<BTreeSet<_>>()
    {
        return Err(DispatchError::CredentialUnavailable);
    }
    Secret::new(saved.access.as_bytes().to_vec())?;
    validate_client(cfg, saved.client.as_ref())?;
    Ok(())
}
/// Public metadata only: destination and response bounds apply without any
/// account credential or inherited proxy. Call only after owner source approval.
pub fn read_public_json(endpoint: &str) -> Result<Value, DispatchError> {
    exchange(endpoint, None, false)
}
/// Credential blobs contain both tokens and identity, exclusively inside the owner-managed credential store.
pub fn access<V: CredentialStore>(
    cfg: &OAuthConfig,
    reference: &str,
    vault: &V,
) -> Result<Secret, DispatchError> {
    cfg.validate()?;
    let sealed = vault.get(reference)?;
    let saved = open_tokens(cfg, &sealed)?;
    if saved.expires > now().saturating_add(60) {
        return Secret::new(saved.access.as_bytes().to_vec());
    }
    let _refresh = vault.lock_refresh(reference)?;
    // Another thread/process may already have refreshed, or the owner may have
    // replaced/revoked this account while we waited. Never reuse the stale read.
    let sealed = vault.get(reference)?;
    let mut saved = open_tokens(cfg, &sealed)?;
    if saved.expires <= now().saturating_add(60) {
        let refresh = saved
            .refresh
            .as_ref()
            .ok_or(DispatchError::CredentialUnavailable)?;
        let mut response = token_form(
            cfg,
            vec![
                ("grant_type", "refresh_token".into()),
                ("refresh_token", refresh.clone()),
                ("resource", cfg.resource.clone()),
            ],
            saved.client.as_ref(),
        )?;
        let next = tokens(cfg, &response, Some(&saved));
        scrub(&mut response);
        saved = next?;
        // A concurrent owner replacement/revocation must never be overwritten.
        vault.replace_if_matches(reference, &sealed, seal(&saved)?)?;
    }
    Secret::new(saved.access.as_bytes().to_vec())
}

fn open_tokens(cfg: &OAuthConfig, sealed: &Secret) -> Result<Tokens, DispatchError> {
    let bytes = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(sealed.text())
            .map_err(|_| DispatchError::CredentialUnavailable)?,
    );
    let saved: Tokens =
        serde_json::from_slice(&bytes).map_err(|_| DispatchError::CredentialUnavailable)?;
    if saved.config_digest != digest(cfg)?
        || saved.scope.iter().collect::<BTreeSet<_>>() != cfg.scopes.iter().collect::<BTreeSet<_>>()
    {
        return Err(DispatchError::CredentialUnavailable);
    }
    validate_client(cfg, saved.client.as_ref())?;
    Ok(saved)
}
fn scrub(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(a) => a.iter_mut().for_each(scrub),
        Value::Object(o) => o.values_mut().for_each(scrub),
        _ => {}
    }
}
pub fn resolve<V: CredentialStore>(
    connection: &Connection,
    vault: &V,
) -> Result<Option<Secret>, DispatchError> {
    match &connection.auth {
        Auth::None => Ok(None),
        Auth::Bearer { credential_ref } | Auth::Header { credential_ref, .. } => {
            vault.get(credential_ref).map(Some)
        }
        Auth::Oauth {
            credential_ref,
            config,
        } => access(config, credential_ref, vault).map(Some),
    }
}
/// Native-only pending authorization. The listener binds before opening a browser
/// and never exposes the verifier/code/token to frontend JavaScript.
pub struct PendingAuthorization {
    config: OAuthConfig,
    listener: TcpListener,
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
    redirect: String,
    deadline: Instant,
    client: Option<RegisteredClient>,
}
impl PendingAuthorization {
    pub fn begin(config: OAuthConfig) -> Result<(Self, String), DispatchError> {
        config.validate()?;
        let listener = if let Some(client_id) = &config.client_id_metadata_url {
            let redirects = client_metadata_redirects(client_id, config.loopback_test)?;
            redirects
                .iter()
                .filter_map(|url| fixed_loopback_port(url))
                .find_map(|port| TcpListener::bind(("127.0.0.1", port)).ok())
                .ok_or(DispatchError::CredentialUnavailable)?
        } else {
            TcpListener::bind("127.0.0.1:0").map_err(|_| DispatchError::CredentialUnavailable)?
        };
        listener
            .set_nonblocking(true)
            .map_err(|_| DispatchError::CredentialUnavailable)?;
        let state = Zeroizing::new(random()?);
        let verifier = Zeroizing::new(random()?);
        let redirect = format!(
            "http://127.0.0.1:{}/oauth/callback",
            listener
                .local_addr()
                .map_err(|_| DispatchError::CredentialUnavailable)?
                .port()
        );
        let mut url = Url::parse(&config.authorization_endpoint)
            .map_err(|_| DispatchError::InvalidConnection)?;
        if url.query().is_some() {
            return Err(DispatchError::InvalidConnection);
        }
        let client = register_client(&config, &redirect)?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            (
                "client_id",
                client.as_ref().map_or(&config.client_id, |c| &c.id),
            ),
            ("redirect_uri", &redirect),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            ),
            ("state", &state),
            ("resource", &config.resource),
            ("scope", &config.scopes.join(" ")),
        ]);
        let pending = Self {
            config,
            listener,
            state,
            verifier,
            redirect,
            deadline: Instant::now() + Duration::from_secs(300),
            client,
        };
        Ok((pending, url.into()))
    }
    pub fn finish(self, cancel: &AtomicBool) -> Result<Secret, DispatchError> {
        while Instant::now() < self.deadline && !cancel.load(Ordering::Acquire) {
            let (mut stream, peer) = match self.listener.accept() {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => return Err(DispatchError::CredentialUnavailable),
            };
            if !peer.ip().is_loopback() {
                continue;
            }
            // macOS can inherit the listener's nonblocking flag. A gap between
            // valid header fragments is not an invalid authorization response.
            // Keep the bounded read timeout, header limit and full validation.
            stream
                .set_nonblocking(false)
                .map_err(|_| DispatchError::CredentialUnavailable)?;
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(|_| DispatchError::CredentialUnavailable)?;
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(|_| DispatchError::CredentialUnavailable)?;
            let mut bytes = Zeroizing::new(Vec::new());
            let mut one = [0u8; 1];
            let read_deadline = Instant::now() + Duration::from_secs(2);
            while bytes.len() < 8192
                && Instant::now() < read_deadline
                && !bytes.ends_with(b"\r\n\r\n")
            {
                match stream.read(&mut one) {
                    Ok(1) => bytes.push(one[0]),
                    _ => break,
                }
            }
            let request = std::str::from_utf8(&bytes).unwrap_or("");
            let line = request.lines().next().unwrap_or("");
            let mut parts = line.split_whitespace();
            let method = parts.next();
            let target = parts.next().unwrap_or("");
            let parsed = Url::parse(&format!("http://127.0.0.1{target}"));
            let expected_host = self
                .redirect
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("");
            let hosts: Vec<_> = request
                .lines()
                .skip(1)
                .filter_map(|l| l.split_once(':'))
                .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
                .map(|(_, v)| v.trim())
                .collect();
            let valid = parsed.ok().filter(|u| {
                method == Some("GET")
                    && target.starts_with("/oauth/callback?")
                    && u.path() == "/oauth/callback"
                    && hosts == [expected_host]
                    && bytes.ends_with(b"\r\n\r\n")
            });
            let code = valid.and_then(|u| self.callback(&u).ok());
            let ok = code.is_some();
            let text = if ok {
                "Authorization received. Return to CookBook."
            } else {
                "Invalid authorization response."
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {}\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                if ok { "200 OK" } else { "400 Bad Request" },
                text.len(),
                text
            );
            let Some(code) = code else { continue };
            let mut response = token_form(
                &self.config,
                vec![
                    ("grant_type", "authorization_code".into()),
                    ("code", code.to_string()),
                    ("redirect_uri", self.redirect.clone()),
                    ("code_verifier", self.verifier.to_string()),
                    ("resource", self.config.resource.clone()),
                ],
                self.client.as_ref(),
            )?;
            let result = tokens(&self.config, &response, None).and_then(|mut t| {
                t.client = self.client.clone();
                seal(&t)
            });
            scrub(&mut response);
            return result;
        }
        Err(if cancel.load(Ordering::Acquire) {
            DispatchError::OAuthAuthorizationCancelled
        } else {
            DispatchError::OAuthAuthorizationExpired
        })
    }
    fn callback(&self, url: &Url) -> Result<Zeroizing<String>, DispatchError> {
        let mut fields = BTreeMap::new();
        for (k, v) in url.query_pairs() {
            if fields.insert(k.into_owned(), v.into_owned()).is_some() {
                return Err(DispatchError::InvalidInput);
            }
        }
        if fields.get("state").map(String::as_str) != Some(self.state.as_str())
            || fields.contains_key("error")
            || fields.get("iss").is_some_and(|i| i != &self.config.issuer)
            || (self.config.require_issuer_response && !fields.contains_key("iss"))
        {
            return Err(DispatchError::InvalidInput);
        }
        fields
            .remove("code")
            .filter(|c| !c.is_empty() && c.len() <= 4096)
            .map(Zeroizing::new)
            .ok_or(DispatchError::InvalidInput)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn config() -> OAuthConfig {
        OAuthConfig {
            issuer: "https://auth.example.com".into(),
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "https://auth.example.com/token".into(),
            client_id: "native-client".into(),
            registration: None,
            client_id_metadata_url: None,
            resource: "https://api.example.com/mcp".into(),
            scopes: vec!["read".into()],
            require_issuer_response: true,
            loopback_test: false,
        }
    }
    #[test]
    fn callback_rejects_state_mixup_duplicates_and_missing_issuer() {
        let (p, u) = PendingAuthorization::begin(config()).unwrap();
        assert!(u.contains("code_challenge_method=S256"));
        for query in [
            "code=x&state=no&iss=https://auth.example.com",
            "code=x",
            "code=x&code=y",
        ] {
            assert!(
                p.callback(
                    &Url::parse(&format!("http://127.0.0.1/oauth/callback?{query}")).unwrap()
                )
                .is_err()
            );
        }
        let good = format!(
            "http://127.0.0.1/oauth/callback?code=x&state={}&iss=https%3A%2F%2Fauth.example.com",
            p.state.as_str()
        );
        assert_eq!(
            p.callback(&Url::parse(&good).unwrap()).unwrap().as_str(),
            "x"
        );
    }
    #[test]
    fn metadata_rejects_private_and_cleartext_endpoints() {
        for url in [
            "http://example.com",
            "https://127.0.0.1",
            "https://user:pw@example.com",
            "https://example.com/#x",
        ] {
            assert!(public_url(url, false).is_err());
        }
    }
    #[test]
    fn token_identity_and_scope_cannot_expand() {
        let cfg = config();
        let t = tokens(
            &cfg,
            &json!({"access_token":"abc","token_type":"bearer","expires_in":3600}),
            None,
        )
        .unwrap();
        assert!(seal(&t).is_ok());
        assert!(tokens(&cfg,&json!({"access_token":"abc","token_type":"bearer","expires_in":3600,"scope":"read write"}),None).is_err());
    }
    #[test]
    fn static_client_bundle_digest_stays_compatible() {
        let legacy = json!({"issuer":"https://auth.example.com","authorization_endpoint":"https://auth.example.com/authorize","token_endpoint":"https://auth.example.com/token","client_id":"native-client","resource":"https://api.example.com/mcp","scopes":["read"],"require_issuer_response":true,"loopback_test":false});
        assert_eq!(serde_json::to_value(config()).unwrap(), legacy);
        assert_eq!(
            digest(&config()).unwrap(),
            apr_core::content_digest(&legacy).unwrap()
        );
        let mut dcr = config();
        dcr.client_id.clear();
        dcr.registration = Some(ClientRegistration {
            endpoint: "https://auth.example.com/register".into(),
            token_endpoint_auth_method: ClientAuthMethod::None,
        });
        let mut old_dcr = legacy;
        old_dcr["client_id"] = json!("");
        old_dcr["registration"] = json!({"endpoint":"https://auth.example.com/register","token_endpoint_auth_method":"none"});
        assert_eq!(serde_json::to_value(&dcr).unwrap(), old_dcr);
        assert_eq!(
            digest(&dcr).unwrap(),
            apr_core::content_digest(&old_dcr).unwrap()
        );
    }
    #[test]
    fn hosted_identity_requires_a_public_document_path_and_exact_id() {
        let mut cfg = config();
        cfg.client_id = "https://client.example.com/oauth/client.json".into();
        cfg.client_id_metadata_url = Some(cfg.client_id.clone());
        cfg.validate().unwrap();
        cfg.client_id_metadata_url = Some("https://client.example.com/".into());
        assert!(cfg.validate().is_err());
        cfg.client_id_metadata_url = Some("https://other.example.com/client.json".into());
        assert!(cfg.validate().is_err());
        cfg.client_id_metadata_url = Some("http://127.0.0.1/client.json".into());
        assert!(cfg.validate().is_err());
        assert_eq!(
            fixed_loopback_port("http://127.0.0.1:48557/oauth/callback"),
            Some(48557)
        );
        for value in [
            "http://localhost:48557/oauth/callback",
            "http://127.0.0.1:48557/other",
            "http://127.0.0.1:48557/oauth/callback?next=1",
            "http://127.0.0.1/oauth/callback",
        ] {
            assert_eq!(fixed_loopback_port(value), None);
        }
    }
}

#[cfg(all(test, feature = "test-http"))]
mod roundtrip_tests {
    use super::*;
    use std::sync::{Mutex, mpsc};
    struct MemoryVault(Mutex<Option<Secret>>, bool);
    impl CredentialStore for MemoryVault {
        fn get(&self, _: &str) -> Result<Secret, DispatchError> {
            self.0.lock().unwrap().as_ref().unwrap().duplicate()
        }
        fn replace(&self, _: &str, value: Secret) -> Result<(), DispatchError> {
            if self.1 {
                *self.0.lock().unwrap() = Some(value);
                Ok(())
            } else {
                Err(DispatchError::CredentialUnavailable)
            }
        }
        fn replace_if_matches(
            &self,
            _: &str,
            expected: &Secret,
            next: Secret,
        ) -> Result<(), DispatchError> {
            let mut saved = self.0.lock().unwrap();
            if !self.1 || saved.as_ref().is_none_or(|s| s.text() != expected.text()) {
                return Err(DispatchError::CredentialUnavailable);
            }
            *saved = Some(next);
            Ok(())
        }
    }
    #[test]
    fn concurrent_file_refresh_rechecks_under_reference_lock_and_rotates_once() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let vault = crate::FileVault::new(&root).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let cfg = OAuthConfig {
            issuer: origin.clone(),
            authorization_endpoint: format!("{origin}/authorize"),
            token_endpoint: format!("{origin}/token"),
            client_id: "test-client".into(),
            registration: None,
            client_id_metadata_url: None,
            resource: format!("{origin}/api"),
            scopes: vec!["read".into()],
            require_issuer_response: false,
            loopback_test: true,
        };
        let mut initial = tokens(&cfg, &json!({"access_token":"expired","refresh_token":"one-use-refresh","token_type":"Bearer","scope":"read","expires_in":3600}), None).unwrap();
        initial.expires = now();
        vault.set("shared-oauth", seal(&initial).unwrap()).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let mut calls = 0;
            while !server_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let (head, body) = raw_request(&mut stream);
                        assert!(head.starts_with("POST /token "));
                        let fields: BTreeMap<_, _> =
                            url::form_urlencoded::parse(&body).into_owned().collect();
                        assert_eq!(fields["refresh_token"], "one-use-refresh");
                        calls += 1;
                        std::thread::sleep(Duration::from_millis(100));
                        json_response(
                            &mut stream,
                            json!({"access_token":"fresh","refresh_token":"rotated","token_type":"Bearer","scope":"read","expires_in":3600}),
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            calls
        });
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                let cfg = cfg.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let store = crate::FileVault::new(&root).unwrap();
                    barrier.wait();
                    assert_eq!(
                        access(&cfg, "shared-oauth", &store).unwrap().text(),
                        "fresh"
                    );
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        stop.store(true, Ordering::Release);
        assert_eq!(server.join().unwrap(), 1);
        let saved = open_tokens(&cfg, &vault.get("shared-oauth").unwrap()).unwrap();
        assert_eq!(saved.refresh.as_deref(), Some("rotated"));
    }

    #[test]
    fn owner_changes_during_refresh_are_preserved_and_no_access_is_returned() {
        for remove in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("vault");
            let vault = crate::FileVault::new(&root).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let cfg = OAuthConfig {
                issuer: origin.clone(),
                authorization_endpoint: format!("{origin}/authorize"),
                token_endpoint: format!("{origin}/token"),
                client_id: "test-client".into(),
                registration: None,
                client_id_metadata_url: None,
                resource: format!("{origin}/api"),
                scopes: vec!["read".into()],
                require_issuer_response: false,
                loopback_test: true,
            };
            let mut initial = tokens(&cfg, &json!({"access_token":"expired","refresh_token":"old-refresh","token_type":"Bearer","scope":"read","expires_in":3600}), None).unwrap();
            initial.expires = now();
            vault.set("shared-oauth", seal(&initial).unwrap()).unwrap();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                raw_request(&mut stream);
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                json_response(
                    &mut stream,
                    json!({"access_token":"stale-owner-access","refresh_token":"rotated","token_type":"Bearer","scope":"read","expires_in":3600}),
                );
            });
            let worker_cfg = cfg.clone();
            let worker = std::thread::spawn(move || {
                access(
                    &worker_cfg,
                    "shared-oauth",
                    &crate::FileVault::new(&root).unwrap(),
                )
                .is_err()
            });
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if remove {
                vault.remove("shared-oauth").unwrap();
            } else {
                vault
                    .set(
                        "shared-oauth",
                        Secret::new(b"owner-replacement".to_vec()).unwrap(),
                    )
                    .unwrap();
            }
            release_tx.send(()).unwrap();
            assert!(worker.join().unwrap());
            server.join().unwrap();
            if remove {
                assert!(!vault.contains("shared-oauth").unwrap());
            } else {
                assert_eq!(
                    vault.get("shared-oauth").unwrap().text(),
                    "owner-replacement"
                );
            }
        }
    }

    fn raw_request(stream: &mut std::net::TcpStream) -> (String, Vec<u8>) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut one = [0];
        while !bytes.ends_with(b"\r\n\r\n") {
            assert!(bytes.len() < 8192);
            stream.read_exact(&mut one).unwrap();
            bytes.push(one[0]);
        }
        let h = String::from_utf8(bytes).unwrap();
        let n = h
            .lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0; n];
        stream.read_exact(&mut body).unwrap();
        (h, body)
    }
    fn request(stream: &mut std::net::TcpStream) -> (String, BTreeMap<String, String>) {
        let (headers, body) = raw_request(stream);
        (
            headers.lines().next().unwrap().to_string(),
            url::form_urlencoded::parse(&body)
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        )
    }
    fn json_response(stream: &mut std::net::TcpStream, value: Value) {
        let body = value.to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    }
    #[test]
    fn exact_issuer_public_profile_precedes_cimd_and_registration() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let endpoint = origin.clone();
        let server = std::thread::spawn(move || {
            for value in [
                json!({"resource":format!("{endpoint}/mcp"),"authorization_servers":[endpoint]}),
                json!({"issuer":endpoint,"authorization_endpoint":format!("{endpoint}/authorize"),"token_endpoint":format!("{endpoint}/token"),"registration_endpoint":format!("{endpoint}/register"),"client_id_metadata_document_supported":true,"code_challenge_methods_supported":["S256"]}),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                request(&mut stream);
                json_response(&mut stream, value);
            }
        });
        let cfg = discover_with_client_identity(
            &format!("{origin}/mcp"),
            Some(&format!("{origin}/metadata")),
            &[
                PreRegisteredClient {
                    issuer: "https://other.example".into(),
                    client_id: "wrong".into(),
                },
                PreRegisteredClient {
                    issuer: origin.clone(),
                    client_id: "shipped-public-id".into(),
                },
            ],
            Some("https://client.example.com/client.json"),
            vec!["read".into()],
            true,
        )
        .unwrap();
        assert_eq!(cfg.client_id, "shipped-public-id");
        assert_eq!(cfg.client_id_metadata_url, None);
        assert_eq!(cfg.registration, None);
        let (_, url) = PendingAuthorization::begin(cfg).unwrap();
        assert_eq!(
            Url::parse(&url)
                .unwrap()
                .query_pairs()
                .find(|(k, _)| k == "client_id")
                .unwrap()
                .1,
            "shipped-public-id"
        );
        server.join().unwrap();
    }
    #[test]
    fn hosted_identity_uses_exact_listed_redirect_and_public_token_flow() {
        let reserved_callback = TcpListener::bind("127.0.0.1:0").unwrap();
        let callback_port = reserved_callback.local_addr().unwrap().port();
        let redirect = format!("http://127.0.0.1:{callback_port}/oauth/callback");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let identity = format!("{origin}/client.json");
        let endpoint = origin.clone();
        let doc_id = identity.clone();
        let doc_redirect = redirect.clone();
        let server = std::thread::spawn(move || {
            for index in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let (line, fields) = request(&mut stream);
                match index {
                    0 => {
                        assert!(line.starts_with("GET /metadata "));
                        json_response(
                            &mut stream,
                            json!({"resource":format!("{endpoint}/mcp"),"authorization_servers":[endpoint]}),
                        );
                    }
                    1 => {
                        assert!(line.starts_with("GET /.well-known/oauth-authorization-server "));
                        json_response(
                            &mut stream,
                            json!({"issuer":endpoint,"authorization_endpoint":format!("{endpoint}/authorize"),"token_endpoint":format!("{endpoint}/token"),"registration_endpoint":format!("{endpoint}/register"),"client_id_metadata_document_supported":true,"code_challenge_methods_supported":["S256"]}),
                        );
                    }
                    2 | 3 => {
                        assert!(line.starts_with("GET /client.json "));
                        json_response(
                            &mut stream,
                            json!({"client_id":doc_id,"client_name":"CookBook APR","redirect_uris":[doc_redirect],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"}),
                        );
                    }
                    4 => {
                        assert!(line.starts_with("POST /token "));
                        assert_eq!(fields["client_id"], doc_id);
                        assert_eq!(fields["redirect_uri"], doc_redirect);
                        assert_eq!(fields["resource"], format!("{endpoint}/mcp"));
                        assert!(!fields.contains_key("client_secret"));
                        json_response(
                            &mut stream,
                            json!({"access_token":"first","refresh_token":"refresh","token_type":"Bearer","scope":"read","expires_in":3600}),
                        );
                    }
                    5 => {
                        assert!(line.starts_with("POST /token "));
                        assert_eq!(fields["client_id"], doc_id);
                        assert_eq!(fields["refresh_token"], "refresh");
                        assert_eq!(fields["resource"], format!("{endpoint}/mcp"));
                        assert!(!fields.contains_key("client_secret"));
                        json_response(
                            &mut stream,
                            json!({"access_token":"rotated","token_type":"Bearer","scope":"read","expires_in":3600}),
                        );
                    }
                    _ => unreachable!(),
                }
            }
        });
        let cfg = discover_with_client_identity(
            &format!("{origin}/mcp"),
            Some(&format!("{origin}/metadata")),
            &[],
            Some(&identity),
            vec!["read".into()],
            true,
        )
        .unwrap();
        assert_eq!(cfg.client_id, identity);
        assert_eq!(cfg.client_id_metadata_url, Some(identity.clone()));
        assert_eq!(cfg.registration, None);
        drop(reserved_callback);
        let (pending, url) = PendingAuthorization::begin(cfg.clone()).unwrap();
        let pairs: BTreeMap<String, String> = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(pairs["redirect_uri"], redirect);
        assert_eq!(pairs["client_id"], identity);
        assert_eq!(pairs["code_challenge_method"], "S256");
        let runner = std::thread::spawn(move || pending.finish(&AtomicBool::new(false)).unwrap());
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([("code", "one-time-code"), ("state", &pairs["state"])])
            .finish();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", callback_port)).unwrap();
        write!(
            stream,
            "GET /oauth/callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:{callback_port}\r\n\r\n"
        )
        .unwrap();
        let mut status = [0; 15];
        stream.read_exact(&mut status).unwrap();
        assert_eq!(&status, b"HTTP/1.1 200 OK");
        let secret = runner.join().unwrap();
        validate_bundle(&cfg, &secret).unwrap();
        let mut saved: Tokens =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(secret.text()).unwrap()).unwrap();
        saved.expires = now();
        let vault = MemoryVault(Mutex::new(Some(seal(&saved).unwrap())), true);
        assert_eq!(
            access(&cfg, "private-ref", &vault).unwrap().text(),
            "rotated"
        );
        server.join().unwrap();
    }
    #[test]
    fn malformed_or_refused_hosted_metadata_never_starts_browser_flow() {
        for mutation in [
            "wrong_id",
            "missing_name",
            "secret",
            "confidential",
            "redirect",
            "dynamic_port",
            "created_status",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let client_id = format!("{origin}/client.json");
            let expected_id = client_id.clone();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                request(&mut stream);
                let mut doc = json!({"client_id":expected_id,"client_name":"CookBook APR","redirect_uris":["http://127.0.0.1:48557/oauth/callback"],"token_endpoint_auth_method":"none"});
                match mutation {
                    "wrong_id" => doc["client_id"] = json!("https://attacker.example/client.json"),
                    "missing_name" => {
                        doc.as_object_mut().unwrap().remove("client_name");
                    }
                    "secret" => doc["client_secret"] = json!("synthetic-secret"),
                    "confidential" => {
                        doc["token_endpoint_auth_method"] = json!("client_secret_basic")
                    }
                    "redirect" => {
                        doc["redirect_uris"] = json!(["https://attacker.example/callback"])
                    }
                    "dynamic_port" => {
                        doc["redirect_uris"] = json!(["http://127.0.0.1/oauth/callback"])
                    }
                    "created_status" => {}
                    _ => unreachable!(),
                }
                if mutation == "created_status" {
                    let body = doc.to_string();
                    write!(stream,"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                } else {
                    json_response(&mut stream, doc);
                }
            });
            assert!(
                client_metadata_redirects(&client_id, true).is_err(),
                "{mutation}"
            );
            server.join().unwrap();
        }
    }
    #[test]
    fn discovery_preserves_scoped_resource_and_selects_registration() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let endpoint = origin.clone();
        let server = std::thread::spawn(move || {
            for value in [
                json!({"resource":format!("{endpoint}/mcp?project=synthetic&read_only=true"),"authorization_servers":[endpoint],"scopes_supported":["read"]}),
                json!({"issuer":endpoint,"authorization_endpoint":format!("{endpoint}/authorize"),"token_endpoint":format!("{endpoint}/token"),"registration_endpoint":format!("{endpoint}/register"),"token_endpoint_auth_methods_supported":["client_secret_post"],"code_challenge_methods_supported":["S256"]}),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                request(&mut stream);
                json_response(&mut stream, value);
            }
        });
        let resource = format!("{origin}/mcp?project=synthetic&read_only=true");
        let result = discover_with_client_identity(
            &resource,
            Some(&format!("{origin}/metadata")),
            &[],
            // The server does not advertise CIMD; this URL must not be fetched.
            Some("https://client.example.com/client.json"),
            vec![],
            true,
        );
        server.join().unwrap();
        let cfg =
            result.expect("advertised dynamic registration should not require a manual client ID");
        assert_eq!(cfg.resource, resource);
        assert_eq!(cfg.scopes, ["read"]);
        let public = serde_json::to_value(cfg).unwrap();
        assert_eq!(
            public["registration"]["endpoint"],
            format!("{origin}/register")
        );
        assert_eq!(
            public["registration"]["token_endpoint_auth_method"],
            "client_secret_post"
        );
        assert!(public.get("client_secret").is_none());
    }
    #[test]
    fn discovery_and_registration_accept_default_basic_metadata() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let endpoint = origin.clone();
        let server = std::thread::spawn(move || {
            for value in [
                json!({"resource":format!("{endpoint}/mcp?read_only=true"),"authorization_servers":[endpoint],"scopes_supported":["read"]}),
                json!({"issuer":endpoint,"authorization_endpoint":format!("{endpoint}/authorize"),"token_endpoint":format!("{endpoint}/token"),"registration_endpoint":format!("{endpoint}/register"),"token_endpoint_auth_methods_supported":["client_secret_post","client_secret_basic"],"code_challenge_methods_supported":["S256"]}),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                request(&mut stream);
                json_response(&mut stream, value);
            }
            let (mut stream, _) = listener.accept().unwrap();
            let (headers, body) = raw_request(&mut stream);
            assert!(headers.starts_with("POST /register "));
            let registration: Value = serde_json::from_slice(&body).unwrap();
            // Minimal response observed from hosted services: absent auth method
            // means Basic under RFC 7591, not whatever the client requested.
            json_response(
                &mut stream,
                json!({
                    "client_id":"issued-client",
                    "client_secret":"synthetic-secret",
                    "client_secret_expires_at":0,
                    "redirect_uris":registration["redirect_uris"]
                }),
            );
            registration
        });
        let resource = format!("{origin}/mcp?read_only=true");
        let cfg = discover(
            &resource,
            Some(&format!("{origin}/metadata")),
            "",
            vec![],
            true,
        )
        .unwrap();
        let result = PendingAuthorization::begin(cfg);
        let registration = server.join().unwrap();
        let (pending, url) =
            result.unwrap_or_else(|e| panic!("Cannot prepare browser sign-in: {e}"));
        assert_eq!(
            registration["token_endpoint_auth_method"],
            "client_secret_basic"
        );
        assert_eq!(pending.config.resource, resource);
        assert_eq!(pending.config.scopes, ["read"]);
        assert!(!url.contains("synthetic-secret"));
    }
    #[test]
    fn dynamic_registration_pkce_and_protected_refresh_for_supported_client_methods() {
        for (method, omitted_method) in [
            (ClientAuthMethod::None, false),
            (ClientAuthMethod::ClientSecretPost, false),
            (ClientAuthMethod::ClientSecretBasic, false),
            (ClientAuthMethod::ClientSecretBasic, true),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let resource = format!("{origin}/mcp?project=synthetic&read_only=true");
            let resource_copy = resource.clone();
            let (challenge_send, challenge_receive) = mpsc::channel::<String>();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let (headers, body) = raw_request(&mut stream);
                assert!(headers.starts_with("POST /register "));
                let registration: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(registration["token_endpoint_auth_method"], method.name());
                assert_eq!(registration["application_type"], "native");
                assert_eq!(registration["scope"], "read");
                let redirect = registration["redirect_uris"][0]
                    .as_str()
                    .unwrap()
                    .to_owned();
                assert!(redirect.starts_with("http://127.0.0.1:"));
                let mut response = registration;
                response["client_id"] = json!("issued-client");
                if method != ClientAuthMethod::None {
                    response["client_secret"] = json!("synthetic-registration-secret");
                    response["client_secret_expires_at"] = json!(0);
                }
                if omitted_method {
                    response
                        .as_object_mut()
                        .unwrap()
                        .remove("token_endpoint_auth_method");
                }
                json_response(&mut stream, response);
                for attempt in 0..3 {
                    let (mut stream, _) = listener.accept().unwrap();
                    let (headers, body) = raw_request(&mut stream);
                    assert!(headers.starts_with("POST /token "));
                    let fields: BTreeMap<_, _> = url::form_urlencoded::parse(&body)
                        .map(|(k, v)| (k.into_owned(), v.into_owned()))
                        .collect();
                    assert_eq!(fields["client_id"], "issued-client");
                    assert_eq!(fields["resource"], resource_copy);
                    match method {
                        ClientAuthMethod::None => {
                            assert!(!fields.contains_key("client_secret"));
                            assert!(!headers.to_lowercase().contains("authorization:"));
                        }
                        ClientAuthMethod::ClientSecretPost => {
                            assert_eq!(fields["client_secret"], "synthetic-registration-secret")
                        }
                        ClientAuthMethod::ClientSecretBasic => {
                            assert!(!fields.contains_key("client_secret"));
                            let auth = headers
                                .lines()
                                .find_map(|l| {
                                    l.split_once(':')
                                        .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                                        .map(|(_, v)| v.trim())
                                })
                                .unwrap();
                            assert_eq!(
                                auth,
                                format!(
                                    "Basic {}",
                                    STANDARD.encode("issued-client:synthetic-registration-secret")
                                )
                            );
                        }
                    }
                    if attempt == 0 {
                        assert_eq!(fields["grant_type"], "authorization_code");
                        assert_eq!(fields["redirect_uri"], redirect);
                        assert_eq!(
                            URL_SAFE_NO_PAD
                                .encode(Sha256::digest(fields["code_verifier"].as_bytes())),
                            challenge_receive.recv().unwrap()
                        );
                    } else {
                        assert_eq!(fields["grant_type"], "refresh_token");
                        assert_eq!(
                            fields["refresh_token"],
                            if attempt == 1 {
                                "first-refresh"
                            } else {
                                "rotated-refresh"
                            }
                        );
                    }
                    if attempt == 2 {
                        write!(stream,"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    } else {
                        json_response(
                            &mut stream,
                            json!({"access_token":if attempt == 0 {"first-token"} else {"rotated-token"},"refresh_token":if attempt == 0 {"first-refresh"} else {"rotated-refresh"},"token_type":"Bearer","scope":"read","expires_in":3600}),
                        );
                    }
                }
            });
            let cfg = OAuthConfig {
                issuer: origin.clone(),
                authorization_endpoint: format!("{origin}/authorize"),
                token_endpoint: format!("{origin}/token"),
                client_id: String::new(),
                registration: Some(ClientRegistration {
                    endpoint: format!("{origin}/register"),
                    token_endpoint_auth_method: method,
                }),
                client_id_metadata_url: None,
                resource,
                scopes: vec!["read".into()],
                require_issuer_response: true,
                loopback_test: true,
            };
            let (pending, url) = PendingAuthorization::begin(cfg.clone()).unwrap();
            assert!(!url.contains("synthetic-registration-secret"));
            let pairs: BTreeMap<String, String> = Url::parse(&url)
                .unwrap()
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            assert_eq!(pairs["client_id"], "issued-client");
            assert_eq!(pairs["resource"], cfg.resource);
            challenge_send
                .send(pairs["code_challenge"].clone())
                .unwrap();
            let runner =
                std::thread::spawn(move || pending.finish(&AtomicBool::new(false)).unwrap());
            let callback = Url::parse(&pairs["redirect_uri"]).unwrap();
            let host = format!("127.0.0.1:{}", callback.port().unwrap());
            let mut stream = std::net::TcpStream::connect(&host).unwrap();
            let query = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs([
                    ("code", "synthetic-code"),
                    ("state", &pairs["state"]),
                    ("iss", &cfg.issuer),
                ])
                .finish();
            write!(
                stream,
                "GET /oauth/callback?{query} HTTP/1.1\r\nHost: {host}\r\n\r\n"
            )
            .unwrap();
            let mut status = [0; 15];
            stream.read_exact(&mut status).unwrap();
            assert_eq!(&status, b"HTTP/1.1 200 OK");
            let secret = runner.join().unwrap();
            validate_bundle(&cfg, &secret).unwrap();
            assert!(
                !serde_json::to_string(&cfg)
                    .unwrap()
                    .contains("synthetic-registration-secret")
            );
            let mut t: Tokens =
                serde_json::from_slice(&URL_SAFE_NO_PAD.decode(secret.text()).unwrap()).unwrap();
            t.expires = now();
            let vault = MemoryVault(Mutex::new(Some(seal(&t).unwrap())), true);
            assert_eq!(access(&cfg, "ref", &vault).unwrap().text(), "rotated-token");
            let saved = vault.get("ref").unwrap();
            let mut refreshed: Tokens =
                serde_json::from_slice(&URL_SAFE_NO_PAD.decode(saved.text()).unwrap()).unwrap();
            assert_eq!(refreshed.client.as_ref().unwrap().id, "issued-client");
            refreshed.expires = now();
            vault.replace("ref", seal(&refreshed).unwrap()).unwrap();
            assert!(matches!(
                access(&cfg, "ref", &vault),
                Err(DispatchError::OAuthReconnectRequired)
            ));
            let mut wrong_resource = cfg.clone();
            wrong_resource.resource.push_str("&another=true");
            assert!(validate_bundle(&wrong_resource, &saved).is_err());
            server.join().unwrap();
        }
    }
    #[test]
    fn registration_rejects_redirect_scope_method_and_secret_changes() {
        for mutation in [
            "redirect",
            "scope",
            "method",
            "missing_method",
            "secret",
            "expiry",
            "application_type",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let (_, body) = raw_request(&mut stream);
                let mut value: Value = serde_json::from_slice(&body).unwrap();
                value["client_id"] = json!("issued-client");
                value["client_secret"] = json!("synthetic-secret");
                match mutation {
                    "redirect" => {
                        value["redirect_uris"] = json!(["https://other.example/callback"])
                    }
                    "scope" => value["scope"] = json!("read write"),
                    "method" => value["token_endpoint_auth_method"] = json!("none"),
                    "missing_method" => {
                        value
                            .as_object_mut()
                            .unwrap()
                            .remove("token_endpoint_auth_method");
                    }
                    "secret" => {
                        value.as_object_mut().unwrap().remove("client_secret");
                    }
                    "expiry" => value["client_secret_expires_at"] = json!(1),
                    "application_type" => value["application_type"] = json!("web"),
                    _ => unreachable!(),
                }
                json_response(&mut stream, value);
            });
            let cfg = OAuthConfig {
                issuer: origin.clone(),
                authorization_endpoint: format!("{origin}/authorize"),
                token_endpoint: format!("{origin}/token"),
                client_id: String::new(),
                registration: Some(ClientRegistration {
                    endpoint: format!("{origin}/register"),
                    token_endpoint_auth_method: ClientAuthMethod::ClientSecretPost,
                }),
                client_id_metadata_url: None,
                resource: format!("{origin}/mcp"),
                scopes: vec!["read".into()],
                require_issuer_response: false,
                loopback_test: true,
            };
            let result = PendingAuthorization::begin(cfg);
            if matches!(mutation, "method" | "missing_method") {
                assert!(
                    matches!(result, Err(DispatchError::OAuthRegistrationMethodMismatch)),
                    "{mutation}"
                );
            } else {
                assert!(
                    matches!(result, Err(DispatchError::OAuthRegistrationFailed)),
                    "{mutation}"
                );
            }
            server.join().unwrap();
        }
    }
    #[test]
    fn temporary_token_failure_does_not_claim_account_revocation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            raw_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let cfg = OAuthConfig {
            issuer: origin.clone(),
            authorization_endpoint: format!("{origin}/authorize"),
            token_endpoint: format!("{origin}/token"),
            resource: format!("{origin}/mcp"),
            loopback_test: true,
            ..super::tests::config()
        };
        assert!(matches!(
            token_form(&cfg, vec![("grant_type", "refresh_token".into())], None),
            Err(DispatchError::OAuthTokenUnavailable)
        ));
        server.join().unwrap();
    }
    #[test]
    fn native_callback_accepts_fragmented_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let (line, fields) = request(&mut stream);
            assert!(line.starts_with("POST /token "));
            assert_eq!(fields["code"], "fragmented-code");
            json_response(
                &mut stream,
                json!({"access_token":"synthetic-token","token_type":"Bearer","scope":"read","expires_in":3600}),
            );
        });
        let cfg = OAuthConfig {
            issuer: origin.clone(),
            authorization_endpoint: format!("{origin}/authorize"),
            token_endpoint: format!("{origin}/token"),
            resource: format!("{origin}/mcp"),
            loopback_test: true,
            ..super::tests::config()
        };
        let (pending, url) = PendingAuthorization::begin(cfg.clone()).unwrap();
        let fields: BTreeMap<String, String> = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let callback = Url::parse(&fields["redirect_uri"]).unwrap();
        let host = format!("127.0.0.1:{}", callback.port().unwrap());
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("code", "fragmented-code"),
                ("state", fields["state"].as_str()),
                ("iss", cfg.issuer.as_str()),
            ])
            .finish();
        let runner = std::thread::spawn(move || pending.finish(&AtomicBool::new(false)));
        let mut stream = std::net::TcpStream::connect(&host).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        write!(stream, "GET /oauth/callback?{query} HTTP/1.1\r\n").unwrap();
        let error = stream
            .read(&mut [0])
            .expect_err("incomplete headers must wait, not return 400");
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        write!(stream, "Host: {host}\r\n\r\n").unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut status = [0; 15];
        stream.read_exact(&mut status).unwrap();
        assert_eq!(&status, b"HTTP/1.1 200 OK");
        validate_bundle(&cfg, &runner.join().unwrap().unwrap()).unwrap();
        server.join().unwrap();
    }
    #[test]
    fn native_callback_token_exchange_refresh_rotation_and_vault_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (challenge_send, challenge_receive) = mpsc::channel::<String>();
        let endpoint = origin.clone();
        let server = std::thread::spawn(move || {
            for index in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let (line, body) = request(&mut stream);
                match index {
                    0 => {
                        assert!(line.starts_with("GET /mcp "));
                        write!(stream,"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer resource_metadata=\"{endpoint}/custom-metadata\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    }
                    1 => {
                        assert!(line.starts_with("GET /custom-metadata "));
                        json_response(
                            &mut stream,
                            json!({"resource":format!("{endpoint}/mcp"),"authorization_servers":[endpoint]}),
                        );
                    }
                    2 => {
                        assert!(line.starts_with("GET /.well-known/oauth-authorization-server "));
                        json_response(
                            &mut stream,
                            json!({"issuer":endpoint,"authorization_endpoint":format!("{endpoint}/authorize"),"token_endpoint":format!("{endpoint}/token"),"code_challenge_methods_supported":["S256"],"authorization_response_iss_parameter_supported":true}),
                        );
                    }
                    3 => {
                        assert!(line.starts_with("POST /token "));
                        assert_eq!(body["grant_type"], "authorization_code");
                        assert_eq!(body["code"], "one-time-code");
                        assert_eq!(body["resource"], format!("{endpoint}/mcp"));
                        assert_eq!(
                            URL_SAFE_NO_PAD
                                .encode(Sha256::digest(body["code_verifier"].as_bytes())),
                            challenge_receive.recv().unwrap()
                        );
                        json_response(
                            &mut stream,
                            json!({"access_token":"first-token","refresh_token":"first-refresh","token_type":"Bearer","scope":"read","expires_in":3600}),
                        );
                    }
                    4 => {
                        assert_eq!(body["refresh_token"], "first-refresh");
                        assert_eq!(body["grant_type"], "refresh_token");
                        assert_eq!(body["resource"], format!("{endpoint}/mcp"));
                        json_response(
                            &mut stream,
                            json!({"access_token":"rotated-token","refresh_token":"rotated-refresh","token_type":"Bearer","scope":"read","expires_in":3600}),
                        );
                    }
                    5 => {
                        assert_eq!(body["refresh_token"], "rotated-refresh");
                        json_response(
                            &mut stream,
                            json!({"access_token":"not-persisted","token_type":"Bearer","scope":"read","expires_in":3600}),
                        );
                    }
                    _ => unreachable!(),
                }
            }
        });
        let cfg = discover(
            &format!("{origin}/mcp"),
            None,
            "native-client",
            vec!["read".into()],
            true,
        )
        .unwrap();
        let (pending, url) = PendingAuthorization::begin(cfg.clone()).unwrap();
        let pairs: BTreeMap<String, String> = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        challenge_send
            .send(pairs["code_challenge"].clone())
            .unwrap();
        let callback = Url::parse(&pairs["redirect_uri"]).unwrap();
        let host = format!("127.0.0.1:{}", callback.port().unwrap());
        let runner = std::thread::spawn(move || pending.finish(&AtomicBool::new(false)).unwrap());
        let mut stream = std::net::TcpStream::connect(&host).unwrap();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("code", "one-time-code"),
                ("state", &pairs["state"]),
                ("iss", &cfg.issuer),
            ])
            .finish();
        write!(
            stream,
            "GET /oauth/callback?{query} HTTP/1.1\r\nHost: {host}\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(!response.contains("one-time-code"));
        let secret = runner.join().unwrap();
        validate_bundle(&cfg, &secret).unwrap();
        let mut t: Tokens =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(secret.text()).unwrap()).unwrap();
        t.expires = now();
        let vault = MemoryVault(Mutex::new(Some(seal(&t).unwrap())), true);
        assert_eq!(
            access(&cfg, "private-ref", &vault).unwrap().text(),
            "rotated-token"
        );
        let saved = vault.get("private-ref").unwrap();
        let mut rotated: Tokens =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(saved.text()).unwrap()).unwrap();
        assert_eq!(rotated.refresh.as_deref(), Some("rotated-refresh"));
        rotated.expires = now();
        let unavailable = MemoryVault(Mutex::new(Some(seal(&rotated).unwrap())), false);
        assert!(access(&cfg, "private-ref", &unavailable).is_err());
        server.join().unwrap();
    }
    #[test]
    fn cancelled_callback_closes_without_token_network_access() {
        let (p, _) = PendingAuthorization::begin(super::tests::config()).unwrap();
        assert!(matches!(
            p.finish(&AtomicBool::new(true)),
            Err(DispatchError::OAuthAuthorizationCancelled)
        ));
    }
    #[test]
    fn expired_callback_reports_timeout_without_token_network_access() {
        let (mut p, _) = PendingAuthorization::begin(super::tests::config()).unwrap();
        p.deadline = Instant::now();
        assert!(matches!(
            p.finish(&AtomicBool::new(false)),
            Err(DispatchError::OAuthAuthorizationExpired)
        ));
    }
}
