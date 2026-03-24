use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Open,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalState {
    pub id: String,
    pub tenant_id: String,
    pub description: String,
    pub status: GoalStatus,
    pub agent_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GoalState {
    pub fn new(id: String, tenant_id: String, description: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            tenant_id,
            description,
            status: GoalStatus::Open,
            agent_ids: vec![],
            created_at: now,
            updated_at: now,
        }
    }
    pub fn add_agent(&mut self, agent_id: String) {
        self.agent_ids.push(agent_id);
        self.updated_at = Utc::now();
    }
    pub fn mark_in_progress(&mut self) {
        self.status = GoalStatus::InProgress;
        self.updated_at = Utc::now();
    }
    pub fn mark_completed(&mut self) {
        self.status = GoalStatus::Completed;
        self.updated_at = Utc::now();
    }
}
