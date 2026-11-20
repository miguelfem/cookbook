//! Bounded, explicitly filtered MCP subscription windows. A graceful server end,
//! an intentional window limit and a transport gap are never conflated.
use super::*;
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Filter {
    #[serde(default)]
    pub tools_list_changed: bool,
    #[serde(default)]
    pub prompts_list_changed: bool,
    #[serde(default)]
    pub resources_list_changed: bool,
    #[serde(default)]
    pub resource_subscriptions: Vec<String>,
}
impl Filter {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if (!self.tools_list_changed
            && !self.prompts_list_changed
            && !self.resources_list_changed
            && self.resource_subscriptions.is_empty())
            || self.resource_subscriptions.len() > 32
            || self
                .resource_subscriptions
                .iter()
                .any(|u| u.is_empty() || u.len() > 4096 || u.chars().any(char::is_control))
        {
            return Err(DispatchError::InvalidConnection);
        }
        Ok(())
    }
    fn subset(&self, approved: &Self) -> bool {
        (!self.tools_list_changed || approved.tools_list_changed)
            && (!self.prompts_list_changed || approved.prompts_list_changed)
            && (!self.resources_list_changed || approved.resources_list_changed)
            && self
                .resource_subscriptions
                .iter()
                .all(|u| approved.resource_subscriptions.contains(u))
    }
    fn allows(&self, message: &Value) -> bool {
        match message["method"].as_str() {
            Some("notifications/tools/list_changed") => self.tools_list_changed,
            Some("notifications/prompts/list_changed") => self.prompts_list_changed,
            Some("notifications/resources/list_changed") => self.resources_list_changed,
            Some("notifications/resources/updated") => message["params"]["uri"]
                .as_str()
                .is_some_and(|u| self.resource_subscriptions.iter().any(|r| r == u)),
            _ => false,
        }
    }
}
struct Collector<'a> {
    id: &'a Value,
    approved: &'a Filter,
    ack: Option<Filter>,
    events: Vec<Value>,
    ended: bool,
    max_events: usize,
}
impl<'a> Collector<'a> {
    fn accept(&mut self, message: Value) -> Result<(), DispatchError> {
        if message["jsonrpc"] != "2.0" {
            return Err(DispatchError::Uncertain);
        }
        if let Some(id) = message.get("id") {
            if id != self.id
                || message["result"]["resultType"] != "complete"
                || message["result"]["_meta"]["io.modelcontextprotocol/subscriptionId"] != *self.id
                || self.ack.is_none()
            {
                return Err(DispatchError::Uncertain);
            }
            self.ended = true;
            return Ok(());
        }
        if message["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"] != *self.id {
            return Err(DispatchError::Uncertain);
        }
        if let Some(ack) = &self.ack {
            if !ack.allows(&message) {
                return Err(DispatchError::Uncertain);
            }
            self.events
                .push(json!({"method":message["method"],"uri":message["params"].get("uri")}));
        } else {
            if message["method"] != "notifications/subscriptions/acknowledged" {
                return Err(DispatchError::Uncertain);
            }
            let ack: Filter = serde_json::from_value(message["params"]["notifications"].clone())
                .map_err(|_| DispatchError::Uncertain)?;
            if !ack.subset(self.approved) {
                return Err(DispatchError::Uncertain);
            }
            self.ack = Some(ack);
        }
        Ok(())
    }
    fn complete(&self) -> bool {
        self.ended || self.events.len() >= self.max_events
    }
    fn finish(self, elapsed: u64) -> Result<Value, DispatchError> {
        let ack = self.ack.ok_or(DispatchError::Uncertain)?;
        Ok(
            json!({"resultType":"complete","provenance":"broker_subscription_window","acknowledged_filter":ack,"notifications":self.events,"server_closed_gracefully":self.ended,"resubscribe_requires_refresh":!self.ended,"duration_ms":elapsed,"continuous_delivery_guaranteed":false}),
        )
    }
}
pub(super) fn window(
    session: &mut Session<'_>,
    filter: &Filter,
    input: &Value,
) -> Result<DispatchOutput, DispatchError> {
    filter.validate()?;
    if session.binding.protocol != "2026-07-28" {
        return Err(DispatchError::InvalidConnection);
    }
    let max = input
        .get("max_notifications")
        .and_then(Value::as_u64)
        .unwrap_or(32);
    if max == 0
        || max > 128
        || input
            .as_object()
            .is_none_or(|m| m.keys().any(|k| k != "max_notifications"))
    {
        return Err(DispatchError::InvalidInput);
    }
    let id = json!(uuid::Uuid::new_v4().to_string());
    let params = session.params(json!({"notifications":filter}));
    let message = json!({"jsonrpc":"2.0","id":id,"method":"subscriptions/listen","params":params});
    let mut collector = Collector {
        id: &id,
        approved: filter,
        ack: None,
        events: Vec::new(),
        ended: false,
        max_events: max as usize,
    };
    let start = Instant::now();
    let mut bytes = serde_json::to_vec(&message).map_err(|_| DispatchError::InvalidInput)?;
    if bytes.len() > 65536 {
        return Err(DispatchError::InvalidInput);
    }
    let max_bytes = session.connection.max_response_bytes;
    if let Some(native) = &mut session.native {
        bytes.push(b'\n');
        native.exchange(&bytes, None, session.deadline, session.cancel, max_bytes)?;
        let mut buffer = Vec::new();
        let mut total = 0usize;
        while Instant::now() < session.deadline
            && !session.cancel.is_some_and(|f| f.load(Ordering::Acquire))
            && !collector.complete()
        {
            let mut chunk = [0u8; 4096];
            match native.stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if total > max_bytes {
                        return Err(DispatchError::Uncertain);
                    }
                    buffer.extend_from_slice(&chunk[..n]);
                    while let Some(end) = buffer.iter().position(|c| *c == b'\n') {
                        let line: Vec<_> = buffer.drain(..=end).collect();
                        let v: Value =
                            serde_json::from_slice(&line).map_err(|_| DispatchError::Uncertain)?;
                        if session
                            .secret
                            .as_ref()
                            .is_some_and(|s| crate::contains_secret(&v, s.text()))
                        {
                            return Err(DispatchError::Uncertain);
                        }
                        collector.accept(v)?;
                        if collector.complete() {
                            break;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => break,
            }
        }
        let cancel =
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}});
        let mut bytes = serde_json::to_vec(&cancel).map_err(|_| DispatchError::InvalidInput)?;
        bytes.push(b'\n');
        let _ = native.exchange(
            &bytes,
            None,
            Instant::now() + Duration::from_millis(100),
            None,
            4096,
        );
    } else {
        let mut request = http::Request::builder()
            .method("POST")
            .uri(&session.connection.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "subscriptions/listen");
        if let Some(secret) = &session.secret {
            request = match &session.connection.auth {
                crate::Auth::Header { name, .. } => request.header(name, secret.text()),
                _ => request.header("Authorization", format!("Bearer {}", secret.text())),
            };
        }
        let remaining = session.remaining()?;
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .https_only(!session.connection.loopback_test)
            .max_redirects(0)
            .http_status_as_error(false)
            .max_idle_connections(0)
            .timeout_global(Some(remaining))
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            DefaultConnector::default(),
            SafeResolver {
                loopback: session.connection.loopback_test,
            },
        );
        let mut response = agent
            .run(
                request
                    .body(bytes)
                    .map_err(|_| DispatchError::InvalidInput)?,
            )
            .map_err(|_| DispatchError::Uncertain)?;
        if response.status().as_u16() != 200
            || response
                .headers()
                .get("Content-Type")
                .and_then(|s| s.to_str().ok())
                .is_none_or(|s| s.split(';').next() != Some("text/event-stream"))
        {
            return Err(DispatchError::Uncertain);
        }
        let mut reader = response.body_mut().as_reader();
        let mut line = Vec::new();
        let mut data = String::new();
        let mut total = 0usize;
        while Instant::now() < session.deadline
            && !collector.complete()
            && !session.cancel.is_some_and(|c| c.load(Ordering::Acquire))
        {
            let mut byte = [0u8; 1];
            match reader.read(&mut byte) {
                Ok(1) => {}
                _ => break,
            };
            total += 1;
            if total > max_bytes {
                return Err(DispatchError::Uncertain);
            }
            if byte[0] != b'\n' {
                line.push(byte[0]);
                continue;
            }
            let text = std::str::from_utf8(&line)
                .map_err(|_| DispatchError::Uncertain)?
                .trim_end_matches('\r');
            if text.is_empty() && !data.is_empty() {
                let v: Value = serde_json::from_str(&data).map_err(|_| DispatchError::Uncertain)?;
                data.clear();
                if session
                    .secret
                    .as_ref()
                    .is_some_and(|s| crate::contains_secret(&v, s.text()))
                {
                    return Err(DispatchError::Uncertain);
                }
                collector.accept(v)?;
            } else if let Some(part) = text.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(part.strip_prefix(' ').unwrap_or(part));
            }
            line.clear();
        }
    }
    let elapsed = start.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let output = collector.finish(elapsed)?;
    validate_service_instance(&session.connection.output_schema, &output)
        .map_err(|_| DispatchError::Uncertain)?;
    Ok(DispatchOutput {
        response_bytes: serde_json::to_vec(&output)
            .map_err(|_| DispatchError::Uncertain)?
            .len(),
        output,
        status: 200,
        elapsed_ms: elapsed,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsolicited_notifications_and_wrong_subscription_ids_are_rejected() {
        let f = Filter {
            tools_list_changed: true,
            ..Default::default()
        };
        let id = json!("sub");
        let mut c = Collector {
            id: &id,
            approved: &f,
            ack: None,
            events: Vec::new(),
            ended: false,
            max_events: 4,
        };
        assert!(c.accept(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed","params":{"_meta":{"io.modelcontextprotocol/subscriptionId":"sub"}}})).is_err());
        c.accept(json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{"_meta":{"io.modelcontextprotocol/subscriptionId":"sub"},"notifications":{"toolsListChanged":true}}})).unwrap();
        assert!(c.accept(json!({"jsonrpc":"2.0","method":"notifications/resources/updated","params":{"uri":"file:///secret","_meta":{"io.modelcontextprotocol/subscriptionId":"sub"}}})).is_err());
    }
}
