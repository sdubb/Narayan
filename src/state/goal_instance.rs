//! GoalInstance — one execution of an AgentRole.
//!
//! ## Relationship to existing types
//!
//! The old `AgentState` represented both "what is this agent doing" and
//! "what goal is it working on" in one struct. With multi-role agents those
//! concerns separate cleanly:
//!
//!   AgentDefinition — the employee identity (static)
//!   AgentRole       — a role template (versioned, semi-static)
//!   GoalInstance    — one concrete piece of work (dynamic, per-run)
//!   AgentState      — runtime execution state (ephemeral, per-run)
//!
//! AgentState.goal_instance_id links the runtime state to this record.
//! When a GoalInstance completes it emits a WorkforceEventPayload.
//!
//! ## Testing mode
//!
//! GoalInstances created while the parent role has RoleStatus::Testing are
//! flagged with `is_test = true`. The executor checks this flag and:
//!   - Skips real writes to external connectors
//!   - Writes to a sandboxed workspace path
//!   - Marks outputs clearly as test data

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalInstanceStatus {
    /// Created, waiting for scheduler to pick up.
    Pending,
    /// Currently executing.
    Running,
    /// Successfully completed — all completion_criteria satisfied.
    Completed,
    /// Some items processed but not all completion_criteria were met before the run ended.
    /// e.g. 40 of 47 leads processed before a rate limit, or output exists but errors were logged.
    PartiallyComplete,
    /// Failed — see failure_reason.
    Failed,
    /// Manually cancelled.
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalInstance {
    pub id: String,
    pub tenant_id: String,

    /// The agent this goal belongs to.
    pub agent_id: String,

    /// The role template this goal is an instance of.
    pub role_id: String,

    /// Snapshot of the role's version when this goal was created.
    /// The executor always uses this version — role edits never affect
    /// in-flight instances.
    pub role_version: u32,

    /// The specific data for this run.
    /// For webhook triggers: the event payload.
    /// For schedule triggers: { "scheduled_at": "...", "period": "..." }
    /// For user message triggers: { "message": "..." }
    /// For workforce event triggers: the mapped fields from the upstream event.
    pub input_data: serde_json::Value,

    pub status: GoalInstanceStatus,

    /// The final output of this goal instance.
    /// Format matches AgentRole.output_spec.format.
    pub result: Option<serde_json::Value>,

    /// Human-readable failure message when status == Failed.
    pub failure_reason: Option<String>,

    /// How this goal was created.
    pub trigger_source: TriggerSource,

    /// True if the parent role was in Testing status when this was created.
    /// Executor skips real external writes for test instances.
    pub is_test: bool,

    /// Cumulative LLM cost for this run in USD.
    /// Updated after each step completes.
    pub cost_usd: f64,

    /// Estimated human hours this run saved (set on completion).
    #[serde(default)]
    pub human_hours_saved: f64,

    /// Estimated human cost saved in USD (hours × market hourly rate for this category).
    #[serde(default)]
    pub human_cost_saved_usd: f64,

    /// The AgentState ID that is executing this goal instance.
    /// Set when execution starts, used to look up runtime state.
    pub agent_state_id: Option<String>,

    /// If this goal was triggered by a workforce event, the ID of that event's
    /// originating GoalInstance. Used for tracing chains.
    pub triggered_by_goal_instance_id: Option<String>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// How this GoalInstance was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum TriggerSource {
    /// Created by an inbound webhook from a connector.
    Webhook { connector: String, event_type: String, external_id: Option<String> },
    /// Created by the scheduler on a cron schedule.
    Schedule { cron: String, scheduled_at: DateTime<Utc> },
    /// Created by a user message in the chat UI.
    UserMessage { user_id: String, message_id: String },
    /// Created manually via API or UI button.
    Manual { created_by: String },
    /// Created by a workforce event from another goal instance.
    WorkforceEvent { source_goal_instance_id: String, source_role_name: String },
}

impl GoalInstance {
    pub fn new(
        id: String,
        tenant_id: String,
        agent_id: String,
        role_id: String,
        role_version: u32,
        input_data: serde_json::Value,
        trigger_source: TriggerSource,
        is_test: bool,
    ) -> Self {
        let now = Utc::now();
        Self {
            id,
            tenant_id,
            agent_id,
            role_id,
            role_version,
            input_data,
            status: GoalInstanceStatus::Pending,
            result: None,
            failure_reason: None,
            trigger_source,
            is_test,
            cost_usd:             0.0,
            human_hours_saved:    0.0,
            human_cost_saved_usd: 0.0,
            agent_state_id: None,
            triggered_by_goal_instance_id: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    pub fn mark_running(&mut self, agent_state_id: String) {
        self.status = GoalInstanceStatus::Running;
        self.agent_state_id = Some(agent_state_id);
        self.updated_at = Utc::now();
    }

    pub fn mark_completed(&mut self, result: serde_json::Value) {
        self.status = GoalInstanceStatus::Completed;
        self.result = Some(result);
        let now = Utc::now();
        self.updated_at = now;
        self.completed_at = Some(now);
    }

    pub fn mark_failed(&mut self, reason: String) {
        self.status = GoalInstanceStatus::Failed;
        self.failure_reason = Some(reason);
        let now = Utc::now();
        self.updated_at = now;
        self.completed_at = Some(now);
    }

    pub fn mark_partially_complete(&mut self, note: impl Into<String>, partial_result: serde_json::Value) {
        self.status = GoalInstanceStatus::PartiallyComplete;
        self.failure_reason = Some(note.into()); // documents why it's partial
        self.result = Some(partial_result);
        let now = Utc::now();
        self.updated_at = now;
        self.completed_at = Some(now);
    }

    pub fn add_cost(&mut self, delta_usd: f64) {
        self.cost_usd += delta_usd;
        self.updated_at = Utc::now();
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            GoalInstanceStatus::Completed
                | GoalInstanceStatus::Failed
                | GoalInstanceStatus::Cancelled
        )
    }

    /// Build the WorkforceEventPayload emitted when this goal completes or fails.
    /// agent_name and role_name must be passed in (not stored here to avoid duplication).
    pub fn to_workforce_event(
        &self,
        agent_name: &str,
        role_name: &str,
    ) -> crate::agent::definition::WorkforceEventPayload {
        crate::agent::definition::WorkforceEventPayload {
            tenant_id: self.tenant_id.clone(),
            agent_id: self.agent_id.clone(),
            agent_name: agent_name.to_string(),
            role_id: self.role_id.clone(),
            role_name: role_name.to_string(),
            goal_instance_id: self.id.clone(),
            status: match self.status {
                GoalInstanceStatus::Completed => "completed".into(),
                GoalInstanceStatus::Failed    => "failed".into(),
                GoalInstanceStatus::Cancelled => "cancelled".into(),
                _                             => "unknown".into(),
            },
            output_data: self.result.clone().unwrap_or(serde_json::Value::Null),
            failure_reason: self.failure_reason.clone(),
            emitted_at: Utc::now(),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_instance() -> GoalInstance {
        GoalInstance::new(
            "gi-1".into(),
            "t-1".into(),
            "ag-1".into(),
            "role-1".into(),
            3,
            serde_json::json!({ "lead_id": "L-1234" }),
            TriggerSource::Manual { created_by: "user-1".into() },
            false,
        )
    }

    #[test]
    fn test_new_instance_is_pending() {
        let gi = make_instance();
        assert_eq!(gi.status, GoalInstanceStatus::Pending);
        assert_eq!(gi.cost_usd, 0.0);
        assert!(!gi.is_test);
        assert!(gi.agent_state_id.is_none());
        assert!(gi.completed_at.is_none());
    }

    #[test]
    fn test_mark_running() {
        let mut gi = make_instance();
        gi.mark_running("as-99".into());
        assert_eq!(gi.status, GoalInstanceStatus::Running);
        assert_eq!(gi.agent_state_id.as_deref(), Some("as-99"));
    }

    #[test]
    fn test_mark_completed() {
        let mut gi = make_instance();
        gi.mark_completed(serde_json::json!({ "summary": "done" }));
        assert_eq!(gi.status, GoalInstanceStatus::Completed);
        assert!(gi.result.is_some());
        assert!(gi.completed_at.is_some());
        assert!(gi.is_terminal());
    }

    #[test]
    fn test_mark_failed() {
        let mut gi = make_instance();
        gi.mark_failed("Salesforce timeout".into());
        assert_eq!(gi.status, GoalInstanceStatus::Failed);
        assert_eq!(gi.failure_reason.as_deref(), Some("Salesforce timeout"));
        assert!(gi.is_terminal());
    }

    #[test]
    fn test_add_cost_accumulates() {
        let mut gi = make_instance();
        gi.add_cost(0.05);
        gi.add_cost(0.03);
        assert!((gi.cost_usd - 0.08).abs() < f64::EPSILON);
    }

    #[test]
    fn test_pending_not_terminal() {
        let gi = make_instance();
        assert!(!gi.is_terminal());
    }

    #[test]
    fn test_to_workforce_event_completed() {
        let mut gi = make_instance();
        gi.mark_completed(serde_json::json!({ "lead_id": "L-1234", "enriched": true }));
        let ev = gi.to_workforce_event("Sales Ops Agent", "Lead Enrichment");
        assert_eq!(ev.status, "completed");
        assert_eq!(ev.role_name, "Lead Enrichment");
        assert_eq!(ev.agent_name, "Sales Ops Agent");
        assert_eq!(ev.output_data["lead_id"], "L-1234");
        assert!(ev.failure_reason.is_none());
    }

    #[test]
    fn test_to_workforce_event_failed() {
        let mut gi = make_instance();
        gi.mark_failed("Timeout after 600s".into());
        let ev = gi.to_workforce_event("Sales Ops Agent", "Lead Enrichment");
        assert_eq!(ev.status, "failed");
        assert_eq!(ev.failure_reason.as_deref(), Some("Timeout after 600s"));
    }

    #[test]
    fn test_role_version_snapshotted() {
        let gi = make_instance();
        // Version is 3 — even if the role is later updated to v4, this instance
        // keeps v3 and will execute against the v3 guidelines.
        assert_eq!(gi.role_version, 3);
    }

    #[test]
    fn test_serialisation_roundtrip() {
        let gi = make_instance();
        let json = serde_json::to_value(&gi).unwrap();
        let back: GoalInstance = serde_json::from_value(json).unwrap();
        assert_eq!(back.id, gi.id);
        assert_eq!(back.role_version, gi.role_version);
    }

    #[test]
    fn test_trigger_source_webhook_serialises() {
        let src = TriggerSource::Webhook {
            connector: "salesforce".into(),
            event_type: "lead_created".into(),
            external_id: Some("L-1234".into()),
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["source"], "webhook");
        assert_eq!(json["connector"], "salesforce");
    }

    #[test]
    fn test_trigger_source_workforce_event_serialises() {
        let src = TriggerSource::WorkforceEvent {
            source_goal_instance_id: "gi-99".into(),
            source_role_name: "Lead Enrichment".into(),
        };
        let json = serde_json::to_value(&src).unwrap();
        assert_eq!(json["source"], "workforce_event");
        assert_eq!(json["source_role_name"], "Lead Enrichment");
    }
}
