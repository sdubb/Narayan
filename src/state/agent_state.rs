use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::planner::Plan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Newly created — pre-flight not yet run.
    Pending,
    /// Pre-flight running.
    Preflight,
    /// Waiting for user to answer clarification questions.
    Clarifying,
    /// Plan created — waiting for user to approve before execution begins.
    PlanApprovalNeeded,
    /// Actively executing a step.
    Running,
    /// Step complete — waiting for scheduler to wake for next step.
    Waiting,
    /// Waiting for child agents to complete before proceeding.
    Delegating,
    /// Goal successfully completed.
    Completed,
    /// Unrecoverable failure.
    Failed,
    /// Manually paused by user.
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: String,
    pub tenant_id: String,
    pub goal: String,
    pub status: AgentStatus,
    pub current_task: Option<String>,
    pub current_step: u32,
    pub workspace_path: String,
    pub memory_ref: Option<String>,
    pub next_run: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Wall-clock time when this agent first started executing (set on first
    /// non-Pending step).  Persisted to DB so the timeout survives restarts.
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// The active execution plan — stored in its own DB column so it can be
    /// read/updated without deserializing the entire metadata JSONB blob.
    #[serde(default)]
    pub plan: Option<Plan>,
    #[serde(default)]
    pub final_answer: Option<String>,
    pub metadata: serde_json::Value,
    /// If this is a child agent, the parent's agent_id.
    pub parent_agent_id: Option<String>,
    /// Child agent IDs spawned by this agent (for delegation).
    pub pending_children: Vec<String>,
    /// Conversation thread this agent belongs to.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Number of times the user has rejected this agent's plan.
    /// Persisted via metadata so it survives server restarts.
    /// Agent fails gracefully after 3 rejections.
    #[serde(default)]
    pub plan_rejection_count: u32,
}

impl AgentState {
    pub fn new(id: String, tenant_id: String, goal: String, workspace_path: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            tenant_id,
            goal,
            status: AgentStatus::Pending,
            current_task: None,
            current_step: 0,
            workspace_path,
            memory_ref: None,
            next_run: now,
            created_at: now,
            updated_at: now,
            started_at: None,
            plan: None,
            final_answer: None,
            metadata: serde_json::Value::Object(Default::default()),
            parent_agent_id: None,
            pending_children: Vec::new(),
            conversation_id: None,
            plan_rejection_count: 0,
        }
    }

    pub fn advance_step(&mut self) {
        self.current_step += 1;
        self.updated_at = Utc::now();
    }

    pub fn mark_preflight(&mut self) {
        self.status = AgentStatus::Preflight;
        // Stamp the wall-clock start time exactly once — survives restarts.
        if self.started_at.is_none() {
            self.started_at = Some(Utc::now());
        }
        self.updated_at = Utc::now();
    }

    pub fn mark_clarifying(&mut self) {
        self.status = AgentStatus::Clarifying;
        self.updated_at = Utc::now();
    }

    pub fn mark_plan_approval_needed(&mut self) {
        self.status = AgentStatus::PlanApprovalNeeded;
        self.updated_at = Utc::now();
    }

    pub fn mark_running(&mut self) {
        self.status = AgentStatus::Running;
        self.updated_at = Utc::now();
    }

    pub fn mark_waiting(&mut self, next_run: DateTime<Utc>) {
        self.status = AgentStatus::Waiting;
        self.next_run = next_run;
        self.updated_at = Utc::now();
    }

    pub fn mark_delegating(&mut self, child_ids: Vec<String>) {
        self.status = AgentStatus::Delegating;
        self.pending_children = child_ids;
        self.updated_at = Utc::now();
    }

    pub fn mark_completed(&mut self) {
        self.status = AgentStatus::Completed;
        self.updated_at = Utc::now();
    }

    pub fn mark_partially_complete(&mut self, note: String, result: serde_json::Value) {
        self.status = AgentStatus::Completed;
        self.metadata["partial_completion_note"] = serde_json::json!(note);
        self.metadata["partial_completion_result"] = result;
        self.updated_at = Utc::now();
    }

    pub fn mark_failed(&mut self) {
        self.status = AgentStatus::Failed;
        self.updated_at = Utc::now();
    }

    pub fn final_answer(&self) -> Option<&str> {
        self.final_answer.as_deref().or_else(|| self.metadata.get("final_answer").and_then(|value| value.as_str()))
    }

    pub fn set_final_answer(&mut self, answer: impl Into<String>) {
        let answer = answer.into().trim().to_string();
        if answer.is_empty() {
            return;
        }
        self.final_answer = Some(answer.clone());
        self.metadata["final_answer"] = serde_json::Value::String(answer);
        self.updated_at = Utc::now();
    }

    pub fn clear_final_answer(&mut self) {
        self.final_answer = None;
        if let Some(metadata) = self.metadata.as_object_mut() {
            metadata.remove("final_answer");
        }
        self.updated_at = Utc::now();
    }

    /// True if this agent is a sub-agent spawned by delegation.
    pub fn is_child(&self) -> bool {
        self.parent_agent_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "deploy service".into(), "/tmp/ws".into())
    }

    #[test]
    fn test_new_state_defaults() {
        let state = make_state();
        assert_eq!(state.status, AgentStatus::Pending);
        assert_eq!(state.current_step, 0);
        assert!(state.parent_agent_id.is_none());
    }

    #[test]
    fn test_advance_step() {
        let mut state = make_state();
        assert_eq!(state.current_step, 0);
        state.advance_step();
        assert_eq!(state.current_step, 1);
        state.advance_step();
        assert_eq!(state.current_step, 2);
    }

    #[test]
    fn test_mark_running() {
        let mut state = make_state();
        state.mark_running();
        assert_eq!(state.status, AgentStatus::Running);
    }

    #[test]
    fn test_mark_completed() {
        let mut state = make_state();
        state.mark_completed();
        assert_eq!(state.status, AgentStatus::Completed);
    }

    #[test]
    fn test_mark_failed() {
        let mut state = make_state();
        state.mark_failed();
        assert_eq!(state.status, AgentStatus::Failed);
    }

    #[test]
    fn test_is_child() {
        let mut state = make_state();
        assert!(!state.is_child());
        state.parent_agent_id = Some("parent-1".into());
        assert!(state.is_child());
    }
}
