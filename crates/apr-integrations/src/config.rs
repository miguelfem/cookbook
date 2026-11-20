use crate::{Client, Error, InstallRequest, Result, digest};
use serde_json::{Value, json};
use toml_edit::{Array, DocumentMut, Item, Table, value};

pub(crate) struct Edited {
    pub bytes: Vec<u8>,
    pub entry_hash: String,
}

pub(crate) fn entry(request: &InstallRequest) -> Value {
    let mut entry = json!({"command":request.executable,"args":["mcp","--attachment-file",request.attachment_file]});
    if request.client == Client::Claude {
        entry["type"] = json!("stdio");
    } else {
        entry["startup_timeout_sec"] = json!(10);
        entry["tool_timeout_sec"] = json!(30);
    }
    entry
}

pub(crate) fn install(
    request: &InstallRequest,
    before: Option<&[u8]>,
    owned: Option<&str>,
) -> Result<Edited> {
    if request.client == Client::Claude {
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
        let existing = servers
            .get("apr")
            .map(|value| digest(value.to_string().as_bytes()));
        if existing.as_deref() != owned {
            return Err(Error::Conflict(if owned.is_some() {
                "owned_entry_edited"
            } else {
                "unowned_entry"
            }));
        }
        let entry = entry(request);
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
            request
                .executable
                .to_str()
                .ok_or(Error::InvalidInput("non_utf8_path"))?,
        ),
    );
    let mut args = Array::new();
    for arg in [
        "mcp",
        "--attachment-file",
        request
            .attachment_file
            .to_str()
            .ok_or(Error::InvalidInput("non_utf8_path"))?,
    ] {
        args.push(arg);
    }
    table.insert("args", value(args));
    table.insert("startup_timeout_sec", value(10));
    table.insert("tool_timeout_sec", value(30));
    servers.insert("apr", Item::Table(table));
    let bytes = document.to_string().into_bytes();
    let parsed = parse_toml(Some(&bytes))?;
    let entry_hash = codex_entry_digest(&parsed["mcp_servers"]["apr"]);
    Ok(Edited { bytes, entry_hash })
}

pub(crate) fn codex_entry_digest(item: &Item) -> String {
    // Keep existing flat-entry hashes valid. Item::to_string omits nested
    // tables, which would otherwise hide user-added tool policies from drift
    // checks. Hash the complete subtree whenever a nested table exists.
    let nested = item.as_table_like().is_some_and(|table| {
        table
            .iter()
            .any(|(_, child)| child.is_table() || child.is_array_of_tables())
    });
    if nested {
        let mut document = DocumentMut::new();
        document.insert("apr", item.clone());
        digest(document.to_string().as_bytes())
    } else {
        digest(item.to_string().as_bytes())
    }
}

fn parse_toml(bytes: Option<&[u8]>) -> Result<DocumentMut> {
    std::str::from_utf8(bytes.unwrap_or(b""))
        .map_err(|_| Error::InvalidInput("config_not_utf8"))?
        .parse()
        .map_err(|_| Error::InvalidInput("invalid_toml"))
}

pub(crate) fn remove(
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
            .map(|value| digest(value.to_string().as_bytes()))
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
