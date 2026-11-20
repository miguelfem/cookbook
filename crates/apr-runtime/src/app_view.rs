//! Read-only MCP App data from already approved and completed steps. Rendering
//! never fetches a new resource, executes a tool, or gains a connection grant.
use crate::{Result, Runtime, RuntimeError};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
impl Runtime {
    pub(crate) fn mcp_app_view(&self, run: &str, resource: &str, tool: &str) -> Result<Value> {
        if [run, resource, tool]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256)
        {
            return Err(RuntimeError::Forbidden);
        }
        let (html, resource_binding) = self.completed_app_step(run, resource)?;
        let (data, tool_binding) = self.completed_app_step(run, tool)?;
        let uri = resource_binding
            .pointer("/mcp/feature/uri")
            .and_then(Value::as_str)
            .ok_or(RuntimeError::Forbidden)?;
        if !uri.starts_with("ui://")
            || tool_binding
                .pointer("/mcp/tool/ui_resource_uri")
                .and_then(Value::as_str)
                != Some(uri)
            || resource_binding["endpoint"] != tool_binding["endpoint"]
            || resource_binding.pointer("/mcp/transport") != tool_binding.pointer("/mcp/transport")
        {
            return Err(RuntimeError::Forbidden);
        }
        let contents = html["contents"]
            .as_array()
            .filter(|a| a.len() == 1)
            .ok_or(RuntimeError::Forbidden)?;
        let item = &contents[0];
        if item["uri"] != uri || item["mimeType"] != "text/html;profile=mcp-app" {
            return Err(RuntimeError::Forbidden);
        }
        let text = item["text"].as_str().ok_or(RuntimeError::Forbidden)?;
        if text.len() > 24000 || serde_json::to_vec(&data)?.len() > 12000 {
            return Err(RuntimeError::LimitExceeded(
                "MCP App exceeds the bounded preview size".into(),
            ));
        }
        Ok(
            json!({"view":"mcp_app","run_id":run,"resource_uri":uri,"html":text,"tool_result":data,"read_only":true,"external_network":false,"host_tools":false,"source":"completed_approved_steps"}),
        )
    }
    fn completed_app_step(&self, run: &str, step: &str) -> Result<(Value, Value)> {
        let row:Option<(String,String)>=self.db.query_row("SELECT s.output,e.binding_snapshot FROM steps s JOIN effect_intents e ON e.run_id=s.run_id AND e.step_id=s.id WHERE s.run_id=?1 AND s.id=?2 AND s.state='succeeded' AND e.status IN ('succeeded','reconciled_success') ORDER BY e.created_at DESC LIMIT 1",params![run,step],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (output, binding) = row.ok_or(RuntimeError::Forbidden)?;
        Ok((
            serde_json::from_str(&output)?,
            serde_json::from_str(&binding)?,
        ))
    }
}
