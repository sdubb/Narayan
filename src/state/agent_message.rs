use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::state::SessionTaskOutput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    #[default]
    Update,
    Result,
    Question,
    Instruction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub tenant_id: String,
    pub sender_agent_id: String,
    pub recipient_agent_id: String,
    #[serde(default)]
    pub message_kind: AgentMessageKind,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_contract: Option<SessionTaskOutput>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<DateTime<Utc>>,
}

impl AgentMessage {
    pub fn new(
        id: String,
        tenant_id: String,
        sender_agent_id: String,
        recipient_agent_id: String,
        message_kind: AgentMessageKind,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            id,
            tenant_id,
            sender_agent_id,
            recipient_agent_id,
            message_kind,
            subject: subject.into(),
            body: body.into(),
            task_id: None,
            step_index: None,
            result_contract: None,
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
            delivered_at: None,
        }
    }

    pub fn has_result_contract(&self) -> bool {
        self.result_contract.is_some()
    }

    pub fn mark_delivered(&mut self) {
        self.delivered_at = Some(Utc::now());
    }
}
