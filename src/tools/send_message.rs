use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    events::{AgentEvent, EventBus},
    state::{AgentMessage, AgentMessageKind, SessionTaskOutput, SessionTaskResultStatus},
    storage::PostgresStore,
    tools::{ParameterSchema, Tool, ToolResult},
};

fn required_string(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("'{}' is required", key))
}

fn optional_string(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key).and_then(|value| value.as_str()).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn parse_string_array(args: &serde_json::Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn parse_message_kind(args: &serde_json::Value) -> Result<AgentMessageKind, String> {
    match args
        .get("message_kind")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") | Some("update") => Ok(AgentMessageKind::Update),
        Some("result") => Ok(AgentMessageKind::Result),
        Some("question") => Ok(AgentMessageKind::Question),
        Some("instruction") => Ok(AgentMessageKind::Instruction),
        Some(other) => Err(format!("unsupported message_kind '{}'", other)),
    }
}

fn parse_result_contract(args: &serde_json::Value) -> Result<Option<SessionTaskOutput>, String> {
    let Some(status) =
        args.get("status").and_then(|value| value.as_str()).map(|value| value.trim().to_ascii_lowercase())
    else {
        return Ok(None);
    };

    let status = match status.as_str() {
        "complete" => SessionTaskResultStatus::Complete,
        "partial" => SessionTaskResultStatus::Partial,
        "failed" => SessionTaskResultStatus::Failed,
        _ => return Err("'status' must be complete, partial, or failed when sending a result contract".into()),
    };

    Ok(Some(SessionTaskOutput {
        status,
        artifacts: parse_string_array(args, "artifacts"),
        findings: parse_string_array(args, "findings"),
        confidence: args.get("confidence").and_then(|value| value.as_f64()).unwrap_or(1.0).clamp(0.0, 1.0),
        note: optional_string(args, "note"),
    }))
}

fn message_to_json(message: &AgentMessage) -> serde_json::Value {
    serde_json::to_value(message).unwrap_or_else(|_| serde_json::json!({}))
}

#[derive(Clone)]
pub struct SendMessageTool {
    store: Arc<PostgresStore>,
    event_bus: Arc<EventBus>,
}

impl SendMessageTool {
    pub fn new(store: Arc<PostgresStore>, event_bus: Arc<EventBus>) -> Self {
        Self { store, event_bus }
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a durable structured message to a parent, child, or teammate agent. Use result contracts for findings, artifacts, and confidence."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional(
                "recipient_agent_id",
                "string",
                "Explicit agent recipient. If omitted, parent_agent_id is used.",
            ),
            ParameterSchema::optional("parent_agent_id", "string", "Injected automatically for child agents."),
            ParameterSchema::optional("message_kind", "string", "update|result|question|instruction"),
            ParameterSchema::optional("subject", "string", "Short subject line."),
            ParameterSchema::optional("body", "string", "Message body or summary."),
            ParameterSchema::optional("task_id", "string", "Associated session task ID."),
            ParameterSchema::optional("status", "string", "complete|partial|failed when sending a result contract."),
            ParameterSchema::optional("artifacts", "array", "Artifact paths or identifiers for result messages."),
            ParameterSchema::optional("findings", "array", "Concrete findings for result messages."),
            ParameterSchema::optional("confidence", "number", "Confidence between 0 and 1."),
            ParameterSchema::optional("note", "string", "Optional note inside the result contract."),
            ParameterSchema::optional("metadata", "object", "Structured message metadata."),
            ParameterSchema::required("tenant_id", "string", "Injected automatically."),
            ParameterSchema::required("agent_id", "string", "Injected automatically."),
            ParameterSchema::optional("step_index", "integer", "Injected automatically."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "type": "object", "additionalProperties": true }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let sender_agent_id = match required_string(&args, "agent_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let recipient_agent_id = optional_string(&args, "recipient_agent_id")
            .or_else(|| optional_string(&args, "parent_agent_id"))
            .ok_or_else(|| "'recipient_agent_id' is required unless parent_agent_id is available".to_string());
        let recipient_agent_id = match recipient_agent_id {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        if self.store.get_agent(&tenant_id, &recipient_agent_id).await?.is_none() {
            return Ok(ToolResult::err(format!("recipient agent '{}' was not found", recipient_agent_id)));
        }

        let result_contract = match parse_result_contract(&args) {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let mut message = AgentMessage::new(
            uuid::Uuid::new_v4().to_string(),
            tenant_id,
            sender_agent_id.clone(),
            recipient_agent_id.clone(),
            match parse_message_kind(&args) {
                Ok(AgentMessageKind::Update) if result_contract.is_some() => AgentMessageKind::Result,
                Ok(kind) => kind,
                Err(message) => return Ok(ToolResult::err(message)),
            },
            optional_string(&args, "subject").unwrap_or_default(),
            optional_string(&args, "body").unwrap_or_default(),
        );
        message.task_id = optional_string(&args, "task_id");
        message.step_index = args.get("step_index").and_then(|value| value.as_u64()).map(|value| value as u32);
        message.result_contract = result_contract;
        message.metadata = args.get("metadata").cloned().unwrap_or_else(|| serde_json::json!({}));

        self.store.create_agent_message(&message).await?;
        self.event_bus.publish(AgentEvent::AgentMessageSent {
            agent_id: sender_agent_id.clone(),
            recipient_agent_id: recipient_agent_id.clone(),
            message_kind: format!("{:?}", message.message_kind).to_ascii_lowercase(),
            task_id: message.task_id.clone(),
            has_result_contract: message.has_result_contract(),
        });
        self.event_bus.publish(AgentEvent::AgentMessageReceived {
            agent_id: recipient_agent_id.clone(),
            sender_agent_id,
            message_kind: format!("{:?}", message.message_kind).to_ascii_lowercase(),
            task_id: message.task_id.clone(),
            has_result_contract: message.has_result_contract(),
        });

        Ok(ToolResult::ok(serde_json::json!({
            "status": "sent",
            "message": message_to_json(&message),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_contract_requires_known_status() {
        let error = parse_result_contract(&serde_json::json!({
            "status": "unknown"
        }))
        .expect_err("unknown status should fail");

        assert!(error.contains("complete, partial, or failed"));
    }

    #[test]
    fn result_contract_defaults_confidence() {
        let contract = parse_result_contract(&serde_json::json!({
            "status": "complete",
            "findings": ["done"]
        }))
        .expect("result contract should parse")
        .expect("contract should exist");

        assert_eq!(contract.status, SessionTaskResultStatus::Complete);
        assert_eq!(contract.confidence, 1.0);
        assert_eq!(contract.findings, vec!["done".to_string()]);
    }
}
