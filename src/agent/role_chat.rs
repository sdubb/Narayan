//! Role chat — conversational interface for an existing AgentRole.
//!
//! Users can:
//!   - Ask why a specific run failed or was skipped
//!   - Understand what the role does and how it's configured
//!   - Request config changes ("change the schedule to daily", "add a Slack notification on failure")
//!   - Review recent run history in plain language
//!
//! ## Session lifecycle
//!
//!   POST /roles/:role_id/chat              → start session, first assistant message
//!   POST /roles/:role_id/chat/:sid/turn    → send a message, get reply + optional pending_change
//!   POST /roles/:role_id/chat/:sid/apply   → apply a pending config change to the live role
//!
//! ## Change safety
//!
//! The LLM produces a `RoleChange` struct alongside its reply text.
//! The frontend renders a confirmation card. The user must explicitly confirm
//! before `apply` is called. The LLM never mutates the role directly.

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    agent::definition::{AgentRole, TriggerType},
    agent::plan_mode::parse_trigger_from_text,
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::GoalInstance,
    storage::PostgresStore,
};

// ── Session ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleChatSession {
    pub id:             String,
    pub tenant_id:      String,
    pub role_id:        String,
    pub agent_id:       String,
    pub conversation:   Vec<RoleChatMessage>,
    /// A structured change the LLM proposed in the last turn, waiting for user confirmation.
    pub pending_change: Option<RoleChange>,
    pub created_at:     chrono::DateTime<Utc>,
    pub updated_at:     chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleChatMessage {
    pub role:    String,  // "user" | "assistant"
    pub content: String,
}

// ── Proposed changes ───────────────────────────────────────────────────────

/// A structured change to an AgentRole proposed by the LLM.
/// The frontend shows this as a confirmation card.
/// Only applied when the user explicitly confirms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleChange {
    pub change_type:    RoleChangeType,
    pub description:    String,  // Human-readable summary shown to the user
    pub new_value:      serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RoleChangeType {
    Schedule,
    AddConstraint,
    RemoveConstraint,
    UpdateGuidelines,
    UpdateOutput,
    UpdateConnectors,
    RenameRole,
    PauseRole,
    ResumeRole,
    /// Add a typed FailureRule — new_value: { text, tool_scope?, action }
    AddFailureRule,
    /// Remove a FailureRule by exact text match — new_value: { text }
    RemoveFailureRule,
    /// Replace all failure rules — new_value: { rules: [{ text, tool_scope?, action }] }
    SetFailureRules,
}

// ── Manager ────────────────────────────────────────────────────────────────

pub struct RoleChatManager {
    gateway: Arc<dyn LlmGateway>,
    store:   Arc<PostgresStore>,
}

impl RoleChatManager {
    pub fn new(gateway: Arc<dyn LlmGateway>, store: Arc<PostgresStore>) -> Self {
        Self { gateway, store }
    }

    // ── Start a session ────────────────────────────────────────────────────

    pub async fn start(
        &self,
        tenant_id: &str,
        role_id:   &str,
    ) -> Result<(RoleChatSession, String)> {
        let role = self.load_role(tenant_id, role_id).await?;
        let recent = self.load_recent_runs(tenant_id, role_id, 5).await;

        let now = Utc::now();
        let session = RoleChatSession {
            id:             Uuid::new_v4().to_string(),
            tenant_id:      tenant_id.to_string(),
            role_id:        role_id.to_string(),
            agent_id:       role.agent_id.clone(),
            conversation:   Vec::new(),
            pending_change: None,
            created_at:     now,
            updated_at:     now,
        };

        let greeting = self.build_greeting(&role, &recent);
        Ok((session, greeting))
    }

    // ── Process one turn ───────────────────────────────────────────────────

    pub async fn turn(
        &self,
        session: &mut RoleChatSession,
        user_message: &str,
    ) -> Result<(String, Option<RoleChange>)> {
        let role = self.load_role(&session.tenant_id, &session.role_id).await?;
        let recent = self.load_recent_runs(&session.tenant_id, &session.role_id, 10).await;

        // Append user message to history
        session.conversation.push(RoleChatMessage {
            role:    "user".into(),
            content: user_message.to_string(),
        });

        let system = self.build_system_prompt(&role, &recent);
        let mut messages = vec![Message::system(system)];
        for msg in &session.conversation {
            if msg.role == "user" {
                messages.push(Message::user(&msg.content));
            } else {
                messages.push(Message::assistant(&msg.content));
            }
        }

        let req = GatewayRequest::new(
            session.id.clone(),
            session.tenant_id.clone(),
            TaskComplexity::Medium,
            messages,
        );

        let resp = self.gateway.chat(req).await?;
        let raw = resp.content.unwrap_or_default();

        // Parse reply — may contain a structured change block
        let (reply_text, proposed_change) = parse_llm_reply(&raw, user_message, &role);

        session.conversation.push(RoleChatMessage {
            role:    "assistant".into(),
            content: reply_text.clone(),
        });
        session.pending_change = proposed_change.clone();
        session.updated_at = Utc::now();

        Ok((reply_text, proposed_change))
    }

    // ── Apply a pending change to the live role ────────────────────────────

    pub async fn apply_change(
        &self,
        tenant_id: &str,
        role_id:   &str,
        change:    &RoleChange,
    ) -> Result<AgentRole> {
        let mut role = self.load_role(tenant_id, role_id).await?;
        role.version += 1;
        role.updated_at = Utc::now();

        match &change.change_type {
            RoleChangeType::Schedule => {
                let cron = change.new_value["cron"].as_str()
                    .unwrap_or("0 9 * * *")
                    .to_string();
                role.trigger = crate::agent::definition::TriggerDef {
                    trigger_type:     TriggerType::Schedule,
                    cron:             Some(cron),
                    source_connector: None,
                    event_filter:     None,
                    input_mapping:    None,
                    ..Default::default()
                };
            }
            RoleChangeType::AddConstraint => {
                let constraint = change.new_value["constraint"].as_str().unwrap_or("").to_string();
                if !constraint.is_empty() {
                    // Constraints live on the AgentDefinition — load and update it
                    if let Ok(Some(mut agent)) = self.store.get_agent_definition(tenant_id, &role.agent_id).await {
                        if !agent.constraints.contains(&constraint) {
                            agent.constraints.push(constraint);
                            agent.updated_at = Utc::now();
                            let _ = self.store.upsert_agent_definition(&agent).await;
                        }
                    }
                }
            }
            RoleChangeType::RemoveConstraint => {
                let constraint = change.new_value["constraint"].as_str().unwrap_or("");
                if let Ok(Some(mut agent)) = self.store.get_agent_definition(tenant_id, &role.agent_id).await {
                    agent.constraints.retain(|c| c != constraint);
                    agent.updated_at = Utc::now();
                    let _ = self.store.upsert_agent_definition(&agent).await;
                }
            }
            RoleChangeType::UpdateGuidelines => {
                if let Some(text) = change.new_value["guidelines"].as_str() {
                    role.execution_guidelines =
                        crate::agent::definition::ExecutionGuidelines::from_skill_text(text);
                }
            }
            RoleChangeType::UpdateOutput => {
                role.output_spec.description = change.new_value["description"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
            }
            RoleChangeType::UpdateConnectors => {
                if let Some(arr) = change.new_value["connectors"].as_array() {
                    role.connectors = arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                }
            }
            RoleChangeType::RenameRole => {
                role.name = change.new_value["name"]
                    .as_str()
                    .unwrap_or(&role.name.clone())
                    .to_string();
            }
            RoleChangeType::PauseRole => {
                role.status = crate::agent::definition::RoleStatus::Paused;
            }
            RoleChangeType::ResumeRole => {
                role.status = crate::agent::definition::RoleStatus::Active;
            }
            RoleChangeType::AddFailureRule => {
                use crate::agent::definition::{FailureRule, infer_failure_action};
                let text       = change.new_value["text"].as_str().unwrap_or("").to_string();
                let tool_scope = change.new_value["tool_scope"].as_str().map(String::from);
                let action_str = change.new_value["action"].as_str().unwrap_or("skip_and_log");
                let action = match action_str {
                    "retry_once"       => crate::agent::definition::FailureAction::RetryOnce,
                    "skip_silently"    => crate::agent::definition::FailureAction::SkipSilently,
                    "abort"            => crate::agent::definition::FailureAction::Abort,
                    "escalate"         => crate::agent::definition::FailureAction::EscalateToHuman {
                        notify_channel: change.new_value["notify_channel"].as_str().map(String::from),
                    },
                    _ => infer_failure_action(&text.to_lowercase()),
                };
                if !text.is_empty() {
                    role.execution_guidelines.add_failure(FailureRule { text, tool_scope, action });
                }
            }
            RoleChangeType::RemoveFailureRule => {
                let text = change.new_value["text"].as_str().unwrap_or("");
                role.execution_guidelines.failure_handling.retain(|r| r.text != text);
            }
            RoleChangeType::SetFailureRules => {
                use crate::agent::definition::{FailureRule, infer_failure_action};
                if let Some(rules) = change.new_value["rules"].as_array() {
                    role.execution_guidelines.failure_handling.clear();
                    for rv in rules {
                        let text       = rv["text"].as_str().unwrap_or("").to_string();
                        let tool_scope = rv["tool_scope"].as_str().map(String::from);
                        let lower      = text.to_lowercase();
                        let action     = infer_failure_action(&lower);
                        if !text.is_empty() {
                            role.execution_guidelines.add_failure(FailureRule { text, tool_scope, action });
                        }
                    }
                }
            }
        }

        self.store.upsert_agent_role(&role).await?;

        // Sync workforce subscriptions in case trigger changed
        let _ = crate::events::workforce::sync_subscriptions_for_role(&role, &self.store).await;

        Ok(role)
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    async fn load_role(&self, tenant_id: &str, role_id: &str) -> Result<AgentRole> {
        self.store.get_agent_role(tenant_id, role_id).await?
            .ok_or_else(|| anyhow::anyhow!("role '{}' not found", role_id))
    }

    async fn load_recent_runs(&self, tenant_id: &str, role_id: &str, limit: i64) -> Vec<GoalInstance> {
        self.store.list_goal_instances_for_role(tenant_id, role_id, limit)
            .await
            .unwrap_or_default()
    }

    fn build_greeting(&self, role: &AgentRole, recent: &[GoalInstance]) -> String {
        let trigger_desc = trigger_description(&role.trigger);
        let run_summary  = runs_summary(recent);

        format!(
            "I'm looking at the **{}** role.\n\n\
             **What it does:** {}\n\
             **Trigger:** {}\n\
             **Connectors:** {}\n\n\
             {}\n\n\
             What would you like to know or change?",
            role.name,
            role.purpose,
            trigger_desc,
            if role.connectors.is_empty() { "none".into() } else { role.connectors.join(", ") },
            run_summary,
        )
    }

    fn build_system_prompt(&self, role: &AgentRole, recent: &[GoalInstance]) -> String {
        let trigger_desc = trigger_description(&role.trigger);
        let runs_text    = recent.iter().map(format_run).collect::<Vec<_>>().join("\n");
        let guidelines   = if role.execution_guidelines.is_empty() {
            "none".to_string()
        } else {
            role.execution_guidelines.to_prompt()
        };
        let constraints  = if role.connectors.is_empty() { "none".into() }
                           else { role.connectors.join(", ") };

        format!(
            r#"You are a helpful assistant that helps users understand and modify an AI agent role.

## Current Role Configuration
Name: {}
Purpose: {}
Status: {:?}
Trigger: {}
Connectors: {}
Execution guidelines: {}
Output: {}

## Recent Runs (last {} runs)
{}

## Your job
Answer questions about this role clearly and concisely.
When the user asks to change something, propose the change in your reply AND output a JSON block
at the very end of your response in this exact format:

```change
{{
  "change_type": "schedule|add_constraint|remove_constraint|update_guidelines|update_output|update_connectors|rename_role|pause_role|resume_role",
  "description": "human-readable summary of what will change",
  "new_value": {{ ... }}
}}
```

For schedule changes, new_value must be: {{"cron": "0 9 * * 1"}}
For constraint changes: {{"constraint": "the constraint text"}}
For guideline updates: {{"guidelines": "full new guidelines text"}}
For output updates: {{"description": "new output description"}}
For connector updates: {{"connectors": ["salesforce", "slack"]}}
For renames: {{"name": "new role name"}}
For pause/resume: {{}}

Only output a change block when the user explicitly asks to change something.
For questions or explanations, just reply normally with no change block.
Always confirm what you're proposing BEFORE showing the change block."#,
            role.name,
            role.purpose,
            role.status,
            trigger_desc,
            constraints,
            guidelines,
            role.output_spec.description,
            recent.len(),
            if runs_text.is_empty() { "No runs yet.".into() } else { runs_text },
        )
    }
}

// ── Parsing LLM reply ──────────────────────────────────────────────────────

fn parse_llm_reply(
    raw:          &str,
    user_message: &str,
    role:         &AgentRole,
) -> (String, Option<RoleChange>) {
    // Split reply text from change block
    const FENCE_START: &str = "```change";
    const FENCE_END:   &str = "```";

    if let Some(start) = raw.find(FENCE_START) {
        let reply_text = raw[..start].trim().to_string();
        let after = &raw[start + FENCE_START.len()..];
        if let Some(end) = after.find(FENCE_END) {
            let json_str = after[..end].trim();
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                let change_type_str = val["change_type"].as_str().unwrap_or("");
                let change_type = match change_type_str {
                    "schedule"           => Some(RoleChangeType::Schedule),
                    "add_constraint"     => Some(RoleChangeType::AddConstraint),
                    "remove_constraint"  => Some(RoleChangeType::RemoveConstraint),
                    "update_guidelines"  => Some(RoleChangeType::UpdateGuidelines),
                    "update_output"      => Some(RoleChangeType::UpdateOutput),
                    "update_connectors"  => Some(RoleChangeType::UpdateConnectors),
                    "rename_role"        => Some(RoleChangeType::RenameRole),
                    "pause_role"         => Some(RoleChangeType::PauseRole),
                    "resume_role"        => Some(RoleChangeType::ResumeRole),
                    "add_failure_rule"   => Some(RoleChangeType::AddFailureRule),
                    "remove_failure_rule"=> Some(RoleChangeType::RemoveFailureRule),
                    "set_failure_rules"  => Some(RoleChangeType::SetFailureRules),
                    _ => None,
                };
                if let Some(ct) = change_type {
                    let description = val["description"].as_str()
                        .unwrap_or("Proposed change")
                        .to_string();
                    let new_value = val["new_value"].clone();
                    return (reply_text, Some(RoleChange {
                        change_type: ct,
                        description,
                        new_value,
                    }));
                }
            }
        }
    }

    // No change block — also try to detect natural-language schedule requests
    // in case the LLM forgot the format, and synthesise a change
    let lower = user_message.to_lowercase();
    if (lower.contains("change") || lower.contains("update") || lower.contains("set"))
        && (lower.contains("schedule") || lower.contains("cron") || lower.contains("daily")
            || lower.contains("weekly") || lower.contains("every"))
    {
        let trigger = parse_trigger_from_text(user_message);
        if trigger.trigger_type == TriggerType::Schedule {
            if let Some(cron) = trigger.cron {
                let change = RoleChange {
                    change_type: RoleChangeType::Schedule,
                    description: format!("Change schedule to: {}", cron),
                    new_value:   serde_json::json!({ "cron": cron }),
                };
                return (raw.to_string(), Some(change));
            }
        }
    }

    (raw.to_string(), None)
}

// ── Formatting helpers ─────────────────────────────────────────────────────

fn trigger_description(trigger: &crate::agent::definition::TriggerDef) -> String {
    match &trigger.trigger_type {
        TriggerType::Schedule => format!(
            "Schedule: {}",
            trigger.cron.as_deref().unwrap_or("unset")
        ),
        TriggerType::Webhook => format!(
            "Webhook from {} {}",
            trigger.source_connector.as_deref().unwrap_or("external"),
            trigger.event_filter.as_deref().unwrap_or(""),
        ),
        TriggerType::Manual       => "Manual (on-demand)".into(),
        TriggerType::UserMessage  => "When you send a message".into(),
        TriggerType::WorkforceEvent => "After another role completes".into(),
    }
}

fn format_run(gi: &GoalInstance) -> String {
    let status = format!("{:?}", gi.status);
    let when   = gi.created_at.format("%d %b %H:%M").to_string();
    let cost   = if gi.cost_usd > 0.0 {
        format!(" — ${:.4}", gi.cost_usd)
    } else {
        String::new()
    };
    let failure = gi.failure_reason.as_deref()
        .map(|r| format!(" — FAILED: {}", &r[..r.len().min(120)]))
        .unwrap_or_default();
    format!("• {} {}{}{}", when, status, cost, failure)
}

fn runs_summary(recent: &[GoalInstance]) -> String {
    if recent.is_empty() {
        return "No runs yet.".into();
    }
    let completed = recent.iter().filter(|r| matches!(r.status, crate::state::GoalInstanceStatus::Completed)).count();
    let failed    = recent.iter().filter(|r| matches!(r.status, crate::state::GoalInstanceStatus::Failed)).count();
    let last      = &recent[0];
    let last_str  = last.created_at.format("%d %b at %H:%M").to_string();
    let last_status = format!("{:?}", last.status).to_lowercase();

    format!(
        "**Recent runs:** {} completed, {} failed (last run: {} — {})",
        completed, failed, last_str, last_status
    )
}
