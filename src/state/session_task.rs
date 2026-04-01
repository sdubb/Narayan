use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_confidence() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionTaskStatus {
    #[default]
    Pending,
    InProgress,
    Blocked,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionTaskResultStatus {
    #[default]
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionTaskOutput {
    #[serde(default)]
    pub status: SessionTaskResultStatus,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub findings: Vec<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTask {
    pub id: String,
    pub tenant_id: String,
    pub agent_id: String,
    pub subject: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: SessionTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<SessionTaskOutput>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl SessionTask {
    pub fn new(
        id: String,
        tenant_id: String,
        agent_id: String,
        subject: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            tenant_id,
            agent_id,
            subject: subject.into(),
            description: description.into(),
            status: SessionTaskStatus::Pending,
            owner: None,
            blocked_by: Vec::new(),
            blocks: Vec::new(),
            metadata: serde_json::json!({}),
            output: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    pub fn set_status(&mut self, status: SessionTaskStatus) {
        self.status = status.clone();
        self.updated_at = Utc::now();
        self.completed_at = match status {
            SessionTaskStatus::Completed | SessionTaskStatus::Failed | SessionTaskStatus::Stopped => {
                Some(self.updated_at)
            }
            _ => None,
        };
    }

    pub fn set_output(&mut self, output: SessionTaskOutput) {
        self.output = Some(output);
        self.updated_at = Utc::now();
    }
}
