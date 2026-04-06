use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::{GoalInstance, GoalInstanceStatus},
    storage::PostgresStore,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChatMessage {
    pub role: String, // "user" | "assistant"
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentChatRequest {
    pub message: String,
    #[serde(default)]
    pub conversation: Vec<AgentChatMessage>,
}

pub struct AgentChatManager {
    gateway: Arc<dyn LlmGateway>,
    store: Arc<PostgresStore>,
}

impl AgentChatManager {
    pub fn new(gateway: Arc<dyn LlmGateway>, store: Arc<PostgresStore>) -> Self {
        Self { gateway, store }
    }

    pub async fn respond(
        &self,
        tenant_id: &str,
        agent_id: &str,
        message: &str,
        conversation: &[AgentChatMessage],
    ) -> Result<String> {
        let context = self.build_context(tenant_id, agent_id).await?;

        let system = format!(
            r#"You are the centralized agent chat for Narayan.

You help the user understand one agent and the wider tenant workspace.
You can answer questions about:
- the selected agent
- its roles
- goal instances / task runs
- recent failures and blockers
- related agents in the tenant

Rules:
- Prefer concrete facts from the context.
        - If the user asks about changes, explain the current state and suggest using role chat or the search-first plan mode.
- Be concise but complete.
- If something is not in the context, say so plainly instead of inventing it.

## Workspace Context
{}
"#,
            context
        );

        let mut messages = vec![Message::system(system)];
        for msg in conversation {
            match msg.role.as_str() {
                "assistant" => messages.push(Message::assistant(&msg.content)),
                _ => messages.push(Message::user(&msg.content)),
            }
        }
        messages.push(Message::user(message));

        let req = GatewayRequest::new(
            format!("agent-chat:{}:{}", tenant_id, agent_id),
            tenant_id.to_string(),
            TaskComplexity::Medium,
            messages,
        );

        let resp = self.gateway.chat(req).await?;
        Ok(resp.content.unwrap_or_default().trim().to_string())
    }

    async fn build_context(&self, tenant_id: &str, agent_id: &str) -> Result<String> {
        let agent = self
            .store
            .get_agent_definition(tenant_id, agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent definition '{}' not found", agent_id))?;
        let roles = self.store.list_roles_for_agent(tenant_id, agent_id).await.unwrap_or_default();
        let runs = self.store.list_goal_instances_for_agent(tenant_id, agent_id, 12).await.unwrap_or_default();
        let agents = self.store.list_agent_definitions(tenant_id).await.unwrap_or_default();

        let role_summaries = if roles.is_empty() {
            "No roles yet.".to_string()
        } else {
            roles
                .iter()
                .map(|role| {
                    format!(
                        "- {} [{}] trigger={} connectors={} output={}",
                        role.name,
                        status_label(&role.status),
                        trigger_summary(&role.trigger),
                        if role.connectors.is_empty() { "none".into() } else { role.connectors.join(", ") },
                        output_summary(&role.output_spec)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let role_names: HashMap<String, String> =
            roles.iter().map(|role| (role.id.clone(), role.name.clone())).collect();

        let run_summaries = if runs.is_empty() {
            "No goal instances yet.".to_string()
        } else {
            runs.iter().map(|gi| format_goal_instance(gi, &role_names)).collect::<Vec<_>>().join("\n")
        };

        let mut other_agents = Vec::new();
        for other in agents.iter().filter(|other| other.id != agent_id).take(8) {
            let role_count =
                self.store.list_roles_for_agent(tenant_id, &other.id).await.map(|roles| roles.len()).unwrap_or(0);
            other_agents.push(format!("- {} [{}] roles={}", other.name, status_label(&other.status), role_count));
        }

        let other_agents =
            if other_agents.is_empty() { "No other agents found.".to_string() } else { other_agents.join("\n") };

        Ok(format!(
            r#"Selected agent
- name: {}
- status: {}
- persona: {}
- connectors: {}
- constraints: {}
- memory_ref: {}

Roles
{}

Recent task / goal runs
{}

Other agents in this tenant
{}
"#,
            agent.name,
            status_label(&agent.status),
            maybe_or_dash(&agent.persona),
            list_or_none(&agent.connectors),
            list_or_none(&agent.constraints),
            maybe_or_dash(&agent.memory_ref),
            role_summaries,
            run_summaries,
            other_agents
        ))
    }
}

pub(crate) fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".into()
    } else {
        items.join(", ")
    }
}

pub(crate) fn maybe_or_dash(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "-".into()
    } else {
        trimmed.to_string()
    }
}

fn status_label<T: std::fmt::Debug>(value: &T) -> String {
    format!("{:?}", value).to_lowercase()
}

pub(crate) fn trigger_summary(trigger: &crate::agent::definition::TriggerDef) -> String {
    use crate::agent::definition::TriggerType;

    match trigger.trigger_type {
        TriggerType::Schedule => trigger.cron.clone().unwrap_or_else(|| "schedule".into()),
        TriggerType::Webhook => {
            let source = trigger.source_connector.as_deref().unwrap_or("webhook");
            let event = trigger.event_filter.as_deref().unwrap_or("event");
            format!("{} / {}", source, event)
        }
        TriggerType::UserMessage => "user message".into(),
        TriggerType::Manual => "manual".into(),
        TriggerType::WorkforceEvent => {
            trigger.workforce_event_filter.clone().unwrap_or_else(|| "workforce event".into())
        }
    }
}

fn output_summary(output: &crate::agent::definition::OutputSpec) -> String {
    match &output.destination {
        crate::agent::definition::OutputDestination::Workspace { path } => {
            if let Some(path) = path {
                format!("workspace:{}", path)
            } else {
                "workspace".into()
            }
        }
        crate::agent::definition::OutputDestination::Connector { name, target_field, .. } => {
            format!("connector:{} -> {}", name, target_field)
        }
        crate::agent::definition::OutputDestination::Channel { connector, channel } => {
            format!("channel:{}#{}", connector, channel)
        }
        crate::agent::definition::OutputDestination::Email { connector, draft } => {
            format!("email:{} draft={}", connector, draft)
        }
        crate::agent::definition::OutputDestination::WorkforceEvent { event_name } => {
            format!("workforce event:{}", event_name)
        }
        crate::agent::definition::OutputDestination::ConversationReply => "conversation reply".into(),
    }
}

fn format_goal_instance(gi: &GoalInstance, role_names: &HashMap<String, String>) -> String {
    let trigger = match &gi.trigger_source {
        crate::state::TriggerSource::Webhook { connector, event_type, .. } => {
            format!("webhook:{}:{}", connector, event_type)
        }
        crate::state::TriggerSource::Schedule { cron, .. } => format!("schedule:{}", cron),
        crate::state::TriggerSource::UserMessage { .. } => "user message".into(),
        crate::state::TriggerSource::Manual { .. } => "manual".into(),
        crate::state::TriggerSource::WorkforceEvent { source_role_name, .. } => {
            format!("workforce event from {}", source_role_name)
        }
    };

    let state = match gi.status {
        GoalInstanceStatus::Completed => "completed",
        GoalInstanceStatus::PartiallyComplete => "partial",
        GoalInstanceStatus::Failed => "failed",
        GoalInstanceStatus::Cancelled => "cancelled",
        GoalInstanceStatus::Running => "running",
        GoalInstanceStatus::Pending => "pending",
    };

    let failure = gi.failure_reason.as_deref().unwrap_or("");
    let role_name = role_names.get(&gi.role_id).cloned().unwrap_or_else(|| gi.role_id.clone());

    if failure.is_empty() {
        format!("- goal {} [{}] role={} trigger={} cost=${:.4}", gi.id, state, role_name, trigger, gi.cost_usd)
    } else {
        format!(
            "- goal {} [{}] role={} trigger={} cost=${:.4} note={}",
            gi.id, state, role_name, trigger, gi.cost_usd, failure
        )
    }
}
