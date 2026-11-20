//! Explicitly approved MCP multi-round requests. No automatic model invocation,
//! browser navigation, roots disclosure, or acceptance of user-input requests.
use crate::DispatchError;
use apr_core::{validate_instance, validate_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionPolicy {
    #[serde(default)]
    pub sampling: bool,
    #[serde(default)]
    pub form: bool,
    #[serde(default)]
    pub url: bool,
    pub max_rounds: u32,
}
impl InteractionPolicy {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if self.max_rounds == 0 || self.max_rounds > 16 {
            return Err(DispatchError::InvalidConnection);
        }
        Ok(())
    }
    pub fn capabilities(&self) -> Value {
        let mut c = json!({});
        if self.sampling {
            c["sampling"] = json!({});
        }
        if self.form {
            c["elicitation"]["form"] = json!({});
        }
        if self.url {
            c["elicitation"]["url"] = json!({});
        }
        c
    }
    pub fn validate_request(&self, result: &Value) -> Result<(), DispatchError> {
        self.validate()?;
        let err = DispatchError::Uncertain;
        if result["resultType"] != "input_required"
            || serde_json::to_vec(result).map_err(|_| err)?.len() > 32768
        {
            return Err(err);
        }
        let requests = result.get("inputRequests").and_then(Value::as_object);
        if result.get("inputRequests").is_some() && requests.is_none() {
            return Err(err);
        }
        let state = result.get("requestState");
        if state.is_some_and(|s| !s.is_string() || s.as_str().unwrap_or("").len() > 16384)
            || (requests.is_none() && state.is_none())
        {
            return Err(err);
        }
        if let Some(requests) = requests {
            if requests.len() > 8 {
                return Err(err);
            }
            for (id, request) in requests {
                if id.is_empty() || id.len() > 128 {
                    return Err(err);
                }
                let p = &request["params"];
                match request["method"].as_str() {
                    Some("sampling/createMessage") if self.sampling => {
                        if !p["maxTokens"]
                            .as_u64()
                            .is_some_and(|n| (1..=8192).contains(&n))
                            || !p["messages"]
                                .as_array()
                                .is_some_and(|a| !a.is_empty() && a.len() <= 32)
                            || p.get("tools").is_some()
                            || p.get("toolChoice").is_some()
                            || p.get("includeContext").is_some_and(|v| v != "none")
                        {
                            return Err(err);
                        }
                    }
                    Some("elicitation/create") if p["mode"] == "url" && self.url => {
                        crate::oauth::public_url(p["url"].as_str().ok_or(err)?, false)
                            .map_err(|_| err)?;
                        if !p["elicitationId"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty() && s.len() <= 256)
                        {
                            return Err(err);
                        }
                    }
                    Some("elicitation/create")
                        if (p.get("mode").is_none() || p["mode"] == "form") && self.form =>
                    {
                        let schema = &p["requestedSchema"];
                        validate_schema(schema).map_err(|_| err)?;
                        let props = schema["properties"].as_object().ok_or(err)?;
                        if schema["type"] != "object"
                            || props.len() > 32
                            || serde_json::to_vec(schema).map_err(|_| err)?.len() > 8192
                        {
                            return Err(err);
                        }
                        for (name, s) in props {
                            let lower = name.to_ascii_lowercase();
                            if [
                                "password",
                                "secret",
                                "token",
                                "api_key",
                                "credit_card",
                                "ssn",
                            ]
                            .iter()
                            .any(|v| lower.contains(v))
                                || s["format"] == "password"
                            {
                                return Err(err);
                            }
                            match s["type"].as_str() {
                                Some("string" | "number" | "integer" | "boolean") => {}
                                Some("array")
                                    if s["items"]["type"] == "string"
                                        && s["items"]["enum"].is_array() => {}
                                _ => return Err(err),
                            }
                            if s.get("$ref").is_some() {
                                return Err(err);
                            }
                        }
                    }
                    _ => return Err(err),
                }
            }
        }
        Ok(())
    }
    /// Validate a subset without borrowing another participant's authority.
    pub fn validate_partial_response(
        &self,
        request: &Value,
        responses: &Value,
        owner: bool,
    ) -> Result<(), DispatchError> {
        self.validate_request(request)?;
        let got = responses.as_object().ok_or(DispatchError::InvalidInput)?;
        let expected = request.get("inputRequests").and_then(Value::as_object);
        let mut subset = serde_json::Map::new();
        for key in got.keys() {
            subset.insert(
                key.clone(),
                expected
                    .and_then(|v| v.get(key))
                    .ok_or(DispatchError::InvalidInput)?
                    .clone(),
            );
        }
        if got.is_empty() && expected.is_some_and(|v| !v.is_empty()) {
            return Err(DispatchError::InvalidInput);
        }
        let mut limited = request.clone();
        limited["inputRequests"] = Value::Object(subset);
        self.validate_response(&limited, responses, owner)
    }
    pub fn validate_response(
        &self,
        request: &Value,
        responses: &Value,
        owner: bool,
    ) -> Result<(), DispatchError> {
        self.validate_request(request)?;
        let expected = request.get("inputRequests").and_then(Value::as_object);
        let got = responses.as_object().ok_or(DispatchError::InvalidInput)?;
        if got.len() != expected.map_or(0, |e| e.len())
            || serde_json::to_vec(responses)
                .map_err(|_| DispatchError::InvalidInput)?
                .len()
                > 32768
        {
            return Err(DispatchError::InvalidInput);
        }
        if let Some(expected) = expected {
            for (id, r) in expected {
                let answer = got.get(id).ok_or(DispatchError::InvalidInput)?;
                if r["method"] == "elicitation/create" {
                    // Only an owner action may assert user consent. A caller model
                    // cannot turn {action:accept} into owner approval.
                    if !owner {
                        return Err(DispatchError::InvalidInput);
                    }
                    match answer["action"].as_str() {
                        Some("accept") if r["params"]["mode"] != "url" => {
                            validate_instance(&r["params"]["requestedSchema"], &answer["content"])
                                .map_err(|_| DispatchError::InvalidInput)?;
                            let props = r["params"]["requestedSchema"]["properties"]
                                .as_object()
                                .ok_or(DispatchError::InvalidInput)?;
                            if answer["content"]
                                .as_object()
                                .is_none_or(|m| m.keys().any(|k| !props.contains_key(k)))
                            {
                                return Err(DispatchError::InvalidInput);
                            }
                        }
                        Some("accept" | "decline" | "cancel") => {}
                        _ => return Err(DispatchError::InvalidInput),
                    }
                    if answer.as_object().is_none_or(|m| {
                        m.keys()
                            .any(|k| !matches!(k.as_str(), "action" | "content"))
                    }) {
                        return Err(DispatchError::InvalidInput);
                    }
                    if (answer["action"] != "accept" || r["params"]["mode"] == "url")
                        && answer.get("content").is_some()
                    {
                        return Err(DispatchError::InvalidInput);
                    }
                } else {
                    if !matches!(answer["role"].as_str(), Some("assistant"))
                        || answer["content"]["type"] != "text"
                        || !answer["content"]["text"].is_string()
                        || !answer["model"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty() && s.len() <= 256)
                        || !matches!(
                            answer["stopReason"].as_str(),
                            Some("endTurn" | "maxTokens" | "stopSequence")
                        )
                    {
                        return Err(DispatchError::InvalidInput);
                    }
                    if answer.as_object().is_none_or(|m| {
                        m.keys().any(|k| {
                            !matches!(k.as_str(), "role" | "content" | "model" | "stopReason")
                        })
                    }) {
                        return Err(DispatchError::InvalidInput);
                    }
                }
            }
        }
        Ok(())
    }
}
/// Constructed only from a saved, validated runtime interaction, not from recipe
/// input fields. The state is opaque and copied exactly without interpretation.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Continuation {
    pub input_responses: Value,
    pub request_state: Option<String>,
}
