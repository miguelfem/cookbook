//! An owner-reviewed text-only API worker for MCP sampling requests. It cannot
//! select tools, expand context, choose a different model, or grant permissions.
use crate::DispatchError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SamplingWorker {
    pub connection: String,
    pub model: String,
    pub max_tokens: u32,
    pub request_template: Value,
    pub messages_key: String,
    pub max_tokens_key: String,
    pub text_pointer: String,
    pub model_pointer: String,
    pub stop_reason_pointer: String,
    pub stop_reason_map: std::collections::BTreeMap<String, String>,
}
impl SamplingWorker {
    pub fn validate(&self) -> Result<(), DispatchError> {
        let e = DispatchError::InvalidConnection;
        if self.stop_reason_map.is_empty()
            || self.stop_reason_map.len() > 16
            || self.stop_reason_map.iter().any(|(k, v)| {
                k.is_empty()
                    || k.len() > 128
                    || !matches!(v.as_str(), "endTurn" | "maxTokens" | "stopSequence")
            })
            || !self.stop_reason_pointer.starts_with('/')
            || self.stop_reason_pointer.len() > 1024
            || self.connection.is_empty()
            || self.connection.len() > 80
            || !self
                .connection
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            || self.model.is_empty()
            || self.model.len() > 128
            || self.max_tokens == 0
            || self.max_tokens > 8192
            || !self.request_template.is_object()
            || serde_json::to_vec(&self.request_template)
                .map_err(|_| e)?
                .len()
                > 8192
            || self.request_template.get("tools").is_some()
            || self.request_template.get("tool_choice").is_some()
            || [&self.messages_key, &self.max_tokens_key].iter().any(|k| {
                k.is_empty()
                    || k.len() > 80
                    || !k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            })
            || self.messages_key == self.max_tokens_key
            || self.messages_key == "model"
            || self.max_tokens_key == "model"
            || [&self.text_pointer, &self.model_pointer]
                .iter()
                .any(|p| !p.starts_with('/') || p.len() > 1024)
        {
            return Err(e);
        }
        Ok(())
    }
    pub fn request(&self, input: &Value) -> Result<Value, DispatchError> {
        self.validate()?;
        let e = DispatchError::InvalidInput;
        if input["method"] != "sampling/createMessage" {
            return Err(e);
        }
        let p = &input["params"];
        let mut messages = Vec::new();
        if let Some(system) = p.get("systemPrompt") {
            let text = system.as_str().filter(|t| t.len() <= 8192).ok_or(e)?;
            messages.push(json!({"role":"system","content":text}));
        }
        for m in p["messages"]
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 32)
            .ok_or(e)?
        {
            let role = m["role"]
                .as_str()
                .filter(|s| matches!(*s, "user" | "assistant"))
                .ok_or(e)?;
            let blocks: Vec<&Value> = match &m["content"] {
                Value::Array(a) => a.iter().collect(),
                v => vec![v],
            };
            let mut text = String::new();
            for b in blocks {
                if b["type"] != "text" {
                    return Err(e);
                }
                text.push_str(b["text"].as_str().ok_or(e)?);
                if text.len() > 16384 {
                    return Err(e);
                }
            }
            messages.push(json!({"role":role,"content":text}));
        }
        let max = p["maxTokens"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or(e)?
            .min(self.max_tokens as u64);
        let mut body = self.request_template.clone();
        body["model"] = json!(self.model);
        body[&self.messages_key] = json!(messages);
        body[&self.max_tokens_key] = json!(max);
        if serde_json::to_vec(&body).map_err(|_| e)?.len() > 32768 {
            return Err(e);
        }
        Ok(body)
    }
    pub fn result(&self, output: &Value) -> Result<Value, DispatchError> {
        let e = DispatchError::Uncertain;
        let text = output
            .pointer(&self.text_pointer)
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 16000)
            .ok_or(e)?;
        let model = output
            .pointer(&self.model_pointer)
            .and_then(Value::as_str)
            .filter(|s| *s == self.model)
            .ok_or(e)?;
        let reason = output
            .pointer(&self.stop_reason_pointer)
            .and_then(Value::as_str)
            .and_then(|s| self.stop_reason_map.get(s))
            .ok_or(e)?;
        Ok(
            json!({"role":"assistant","model":model,"content":{"type":"text","text":text},"stopReason":reason}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> SamplingWorker {
        serde_json::from_value(json!({"connection":"model","model":"exact-model","max_tokens":50,"request_template":{},"messages_key":"messages","max_tokens_key":"max_tokens","text_pointer":"/text","model_pointer":"/model","stop_reason_pointer":"/finish","stop_reason_map":{"stop":"endTurn","length":"maxTokens"}})).unwrap()
    }
    #[test]
    fn text_worker_pins_model_limits_and_observed_stop_reason() {
        let p = profile();
        let body=p.request(&json!({"method":"sampling/createMessage","params":{"maxTokens":300,"messages":[{"role":"user","content":{"type":"text","text":"data"}}]}})).unwrap();
        assert_eq!(body["max_tokens"], 50);
        assert_eq!(body["model"], "exact-model");
        assert_eq!(
            p.result(&json!({"model":"exact-model","text":"partial","finish":"length"}))
                .unwrap()["stopReason"],
            "maxTokens"
        );
        assert!(
            p.result(&json!({"model":"other-model","text":"body","finish":"stop"}))
                .is_err()
        );
        assert!(
            p.result(&json!({"model":"exact-model","text":"body"}))
                .is_err()
        );
        assert!(p.request(&json!({"method":"elicitation/create"})).is_err());
        let mut p = p;
        p.request_template["tools"] = json!([]);
        assert!(p.validate().is_err());
    }
}
