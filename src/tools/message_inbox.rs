use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    events::{AgentEvent, EventBus},
    state::{AgentMessage, AgentMessageKind, AgentStatus},
    storage::PostgresStore,
    swarm::Swarm,
    tools::{ParameterSchema, Tool, ToolResult},
    util::next_run_after,
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

fn optional_bool(args: &serde_json::Value, key: &str) -> bool {
    args.get(key).and_then(|value| value.as_bool()).unwrap_or(false)
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

fn message_to_json(message: &AgentMessage) -> serde_json::Value {
    serde_json::to_value(message).unwrap_or_else(|_| serde_json::json!({}))
}

#[derive(Debug, Clone)]
pub struct ContinueWorkerRequest {
    pub tenant_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    pub subject: Option<String>,
    pub body: String,
    pub task_id: Option<String>,
    pub worker_type: Option<String>,
    pub write_scope: Vec<String>,
    pub ack_message_ids: Vec<String>,
    pub metadata: serde_json::Value,
}

pub async fn continue_worker_from_parent(
    store: &PostgresStore,
    swarm: &Swarm,
    event_bus: &EventBus,
    request: ContinueWorkerRequest,
) -> anyhow::Result<ToolResult> {
    let Some(parent) = store.get_agent(&request.tenant_id, &request.parent_agent_id).await? else {
        return Ok(ToolResult::err(format!("parent agent '{}' was not found", request.parent_agent_id)));
    };
    let Some(mut child) = store.get_agent(&request.tenant_id, &request.child_agent_id).await? else {
        return Ok(ToolResult::err(format!("child agent '{}' was not found", request.child_agent_id)));
    };
    if child.parent_agent_id.as_deref() != Some(request.parent_agent_id.as_str()) {
        return Ok(ToolResult::err("target child does not belong to this parent agent"));
    }
    if matches!(child.status, AgentStatus::Delegating) && !child.pending_children.is_empty() {
        return Ok(ToolResult::err("child is still waiting on its own pending children"));
    }

    let mut instruction = AgentMessage::new(
        uuid::Uuid::new_v4().to_string(),
        request.tenant_id.clone(),
        parent.id.clone(),
        child.id.clone(),
        AgentMessageKind::Instruction,
        request.subject.clone().unwrap_or_else(|| "continue_worker".into()),
        request.body.clone(),
    );
    instruction.task_id = request.task_id.clone().or_else(|| child.current_task.clone());
    instruction.metadata = serde_json::json!({
        "continued_via": "message_inbox",
        "worker_type": request.worker_type,
        "write_scope": request.write_scope,
        "parent_status": format!("{:?}", parent.status).to_ascii_lowercase(),
        "extra": request.metadata,
    });
    store.create_agent_message(&instruction).await?;

    child.metadata["last_user_input_context"] = serde_json::json!(request.body);
    child.metadata["continue_worker_instruction"] = serde_json::json!({
        "subject": request.subject,
        "body": instruction.body,
        "task_id": instruction.task_id,
        "worker_type": request.worker_type,
        "write_scope": request.write_scope,
        "continued_at": chrono::Utc::now().to_rfc3339(),
    });

    if let Some(ctx) = child.metadata.get_mut("delegation_context").and_then(|value| value.as_object_mut()) {
        if let Some(task_id) = instruction.task_id.as_ref() {
            ctx.insert("task_id".into(), serde_json::json!(task_id));
        }
        if let Some(worker_type) = request.worker_type.as_ref() {
            ctx.insert("worker_type".into(), serde_json::json!(worker_type));
        }
        ctx.insert("write_scope".into(), serde_json::json!(request.write_scope));
        ctx.insert("continued_at".into(), serde_json::json!(chrono::Utc::now().to_rfc3339()));
    } else {
        child.metadata["delegation_context"] = serde_json::json!({
            "task_id": instruction.task_id,
            "worker_type": request.worker_type,
            "write_scope": request.write_scope,
            "continued_at": chrono::Utc::now().to_rfc3339(),
        });
    }

    if let Some(task_id) = instruction.task_id.clone() {
        child.current_task = Some(task_id);
    }
    child.clear_final_answer();
    child.mark_waiting(next_run_after(0));
    child.updated_at = chrono::Utc::now();
    store.upsert_agent(&child).await?;

    for message_id in &request.ack_message_ids {
        let _ = store
            .mark_agent_message_delivered_for_recipient(&request.tenant_id, &request.parent_agent_id, message_id)
            .await;
    }

    swarm.push(child.id.clone()).await?;
    event_bus.publish(AgentEvent::AgentMessageSent {
        agent_id: parent.id.clone(),
        recipient_agent_id: child.id.clone(),
        message_kind: "instruction".into(),
        task_id: instruction.task_id.clone(),
        has_result_contract: false,
    });
    event_bus.publish(AgentEvent::AgentMessageReceived {
        agent_id: child.id.clone(),
        sender_agent_id: parent.id.clone(),
        message_kind: "instruction".into(),
        task_id: instruction.task_id.clone(),
        has_result_contract: false,
    });
    event_bus.publish(AgentEvent::WorkerContinued {
        agent_id: parent.id,
        child_agent_id: child.id.clone(),
        task_id: instruction.task_id.clone(),
        worker_type: request.worker_type.clone(),
    });

    Ok(ToolResult::ok(serde_json::json!({
        "status": "continued",
        "child_agent_id": child.id,
        "task_id": instruction.task_id,
        "message": message_to_json(&instruction),
    })))
}

#[derive(Clone)]
pub struct MessageInboxTool {
    store: Arc<PostgresStore>,
    swarm: Arc<Swarm>,
    event_bus: Arc<EventBus>,
}

impl MessageInboxTool {
    pub fn new(store: Arc<PostgresStore>, swarm: Arc<Swarm>, event_bus: Arc<EventBus>) -> Self {
        Self { store, swarm, event_bus }
    }
}

#[async_trait]
impl Tool for MessageInboxTool {
    fn name(&self) -> &str {
        "message_inbox"
    }

    fn description(&self) -> &str {
        "Read the durable agent inbox, acknowledge messages, and continue an existing child worker with follow-up instructions."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("action", "string", "list|get|ack|continue_worker"),
            ParameterSchema::required("tenant_id", "string", "Tenant ID injected automatically."),
            ParameterSchema::required("agent_id", "string", "Current agent ID injected automatically."),
            ParameterSchema::optional("direction", "string", "inbox|sent|all for list."),
            ParameterSchema::optional("undelivered_only", "boolean", "Only return undelivered inbox messages."),
            ParameterSchema::optional("limit", "integer", "Maximum messages to return, default 20."),
            ParameterSchema::optional("message_id", "string", "Message ID for get or ack."),
            ParameterSchema::optional("child_agent_id", "string", "Child agent to continue."),
            ParameterSchema::optional("subject", "string", "Follow-up subject for continue_worker."),
            ParameterSchema::optional("body", "string", "Follow-up instruction for continue_worker."),
            ParameterSchema::optional("task_id", "string", "Associated session task ID."),
            ParameterSchema::optional("worker_type", "string", "Worker type for continue_worker."),
            ParameterSchema::optional("write_scope", "array", "Write scope ownership for continue_worker."),
            ParameterSchema::optional(
                "ack_message_ids",
                "array",
                "Parent inbox message IDs to mark delivered during continue_worker.",
            ),
            ParameterSchema::optional("metadata", "object", "Structured metadata for continue_worker."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "type": "object", "additionalProperties": true }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = match required_string(&args, "action") {
            Ok(value) => value.to_ascii_lowercase(),
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let agent_id = match required_string(&args, "agent_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };

        match action.as_str() {
            "list" => {
                let limit = args.get("limit").and_then(|value| value.as_i64()).unwrap_or(20).clamp(1, 100);
                let direction = optional_string(&args, "direction").unwrap_or_else(|| "inbox".into());
                let messages = match direction.as_str() {
                    "inbox" => {
                        self.store
                            .list_agent_inbox_messages(
                                &tenant_id,
                                &agent_id,
                                optional_bool(&args, "undelivered_only"),
                                limit,
                            )
                            .await?
                    }
                    "sent" => self.store.list_agent_sent_messages(&tenant_id, &agent_id, limit).await?,
                    "all" => self.store.list_agent_messages_for_agent(&tenant_id, &agent_id, limit).await?,
                    other => return Ok(ToolResult::err(format!("unsupported direction '{}'", other))),
                };
                let unread = self.store.count_undelivered_agent_messages(&tenant_id, &agent_id).await.unwrap_or(0);
                Ok(ToolResult::ok(serde_json::json!({
                    "direction": direction,
                    "messages": messages.iter().map(message_to_json).collect::<Vec<_>>(),
                    "count": messages.len(),
                    "unread_count": unread,
                })))
            }
            "get" => {
                let message_id = match required_string(&args, "message_id") {
                    Ok(value) => value,
                    Err(message) => return Ok(ToolResult::err(message)),
                };
                let Some(message) = self.store.get_agent_message_for_agent(&tenant_id, &agent_id, &message_id).await?
                else {
                    return Ok(ToolResult::err(format!("message '{}' not found", message_id)));
                };
                Ok(ToolResult::ok(serde_json::json!({
                    "message": message_to_json(&message),
                })))
            }
            "ack" => {
                let message_id = match required_string(&args, "message_id") {
                    Ok(value) => value,
                    Err(message) => return Ok(ToolResult::err(message)),
                };
                let acknowledged =
                    self.store.mark_agent_message_delivered_for_recipient(&tenant_id, &agent_id, &message_id).await?;
                if acknowledged {
                    self.event_bus.publish(AgentEvent::AgentMessageDelivered {
                        agent_id: agent_id.clone(),
                        message_id: message_id.clone(),
                    });
                }
                Ok(ToolResult::ok(serde_json::json!({
                    "acknowledged": acknowledged,
                    "message_id": message_id,
                })))
            }
            "continue_worker" => {
                let child_agent_id = match required_string(&args, "child_agent_id") {
                    Ok(value) => value,
                    Err(message) => return Ok(ToolResult::err(message)),
                };
                let body = match required_string(&args, "body") {
                    Ok(value) => value,
                    Err(message) => return Ok(ToolResult::err(message)),
                };
                continue_worker_from_parent(
                    &self.store,
                    &self.swarm,
                    &self.event_bus,
                    ContinueWorkerRequest {
                        tenant_id,
                        parent_agent_id: agent_id,
                        child_agent_id,
                        subject: optional_string(&args, "subject"),
                        body,
                        task_id: optional_string(&args, "task_id"),
                        worker_type: optional_string(&args, "worker_type"),
                        write_scope: parse_string_array(&args, "write_scope"),
                        ack_message_ids: parse_string_array(&args, "ack_message_ids"),
                        metadata: args.get("metadata").cloned().unwrap_or_else(|| serde_json::json!({})),
                    },
                )
                .await
            }
            other => Ok(ToolResult::err(format!("unsupported action '{}'", other))),
        }
    }
}
