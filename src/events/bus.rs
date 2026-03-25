use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStepEvent {
    pub step_index: usize,
    pub description: String,
    pub tool: Option<String>,
    pub success_criteria: String,
    pub condition: Option<String>,
}

/// Every observable event an agent can emit during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    // ── Preflight ──────────────────────────────────────────────────────────
    PreflightStarted {
        agent_id: String,
    },
    PreflightPassed {
        agent_id: String,
    },
    PreflightFailed {
        agent_id: String,
        reason: String,
    },

    // ── Clarification ──────────────────────────────────────────────────────
    ClarificationNeeded {
        agent_id: String,
        questions: Vec<crate::agent::clarifier::ClarificationQuestion>,
    },
    ClarificationReceived {
        agent_id: String,
    },

    // ── Planning ───────────────────────────────────────────────────────────
    PlanningStarted {
        agent_id: String,
    },
    PlanCreated {
        agent_id: String,
        step_count: usize,
        rationale: String,
        job_type: Option<String>,
        steps: Vec<PlanStepEvent>,
    },

    // ── Step execution ─────────────────────────────────────────────────────
    StepStarted {
        agent_id: String,
        step_index: usize,
        description: String,
        tool: Option<String>,
        success_criteria: Option<String>,
        condition: Option<String>,
    },
    ToolCalled {
        agent_id: String,
        step_index: usize,
        tool_name: String,
        args_preview: String,
        step_description: Option<String>,
    },
    ToolResult {
        agent_id: String,
        step_index: usize,
        tool_name: String,
        success: bool,
        output_preview: String,
        error: Option<String>,
    },
    StepCompleted {
        agent_id: String,
        step_index: usize,
        success: bool,
        summary: String,
        description: Option<String>,
    },
    StepRetrying {
        agent_id: String,
        step_index: usize,
        delay_secs: i64,
        reason: String,
        retry_count: u32,
    },

    // ── Policy & compliance ────────────────────────────────────────────────
    /// Emitted whenever the policy engine evaluates a tool call.
    PolicyDecision {
        agent_id: String,
        step_index: usize,
        tool: String,
        decision: String, // "allow" | "block" | "require_approval" | "redact"
        rule_id: Option<String>,
        reason: Option<String>,
        risk_level: String,
    },
    /// Emitted when PII is detected and redacted in tool arguments.
    PiiRedacted {
        agent_id: String,
        step_index: usize,
        tool: String,
        fields_redacted: Vec<String>,
    },
    /// Emitted on each SLA threshold check.
    SlaCheck {
        agent_id: String,
        pct_elapsed: f64,
        message: String,
        action: Option<String>,   // "escalate" | "notify" | null
        deadline: Option<String>, // ISO-8601
    },

    // ── Citations & evidence ───────────────────────────────────────────────
    /// Emitted every time a citation is recorded for a step.
    CitationRecorded {
        agent_id: String,
        step_index: usize,
        claim: String,
        source_ref: String,
        source_type: String,
        confidence: f64,
    },
    /// Emitted when the evidence packager bundles the full audit trail.
    EvidencePackaged {
        agent_id: String,
        citations: usize,
        audit_entries: usize,
    },

    // ── Review queue ───────────────────────────────────────────────────────
    /// Emitted when a policy rule triggers a human review, pausing the agent.
    ReviewRequired {
        agent_id: String,
        review_id: String,
        summary: String,
        reason: String,
        rule_id: Option<String>,
    },

    // ── Judgement ─────────────────────────────────────────────────────────────
    /// Emitted when the judgment layer sees a step worth surfacing to the UI.
    JudgementSignal {
        agent_id: String,
        step_index: usize,
        step_description: String,
        job_type: Option<String>,
        profile: String,
        score: f64,
        confidence: f64,
        recommendation: String,
        summary: String,
        reasons: Vec<String>,
        timestamp: String,
    },

    // ── Connector triggers ─────────────────────────────────────────────────
    /// Emitted when an inbound connector webhook creates this agent.
    ConnectorTrigger {
        agent_id: String,
        connector_type: String,
        event_type: String,
        external_id: Option<String>,
    },

    // ── Delegation ─────────────────────────────────────────────────────────
    ChildSpawned {
        agent_id: String,
        child_agent_id: String,
        sub_goal: String,
    },
    ChildrenComplete {
        agent_id: String,
        child_ids: Vec<String>,
    },

    // ── Cost tracking ────────────────────────────────────────────────────
    /// Emitted after each LLM call with cost deltas for live cost counter.
    LlmCostUpdate {
        agent_id: String,
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        cost_delta_usd: f64,
        total_cost_usd: f64,
        total_requests: u64,
    },

    // ── Plan approval ────────────────────────────────────────────────────
    /// Emitted when the plan is ready and needs user sign-off before execution.
    PlanApprovalNeeded {
        agent_id: String,
        step_count: usize,
        rationale: String,
        steps: Vec<serde_json::Value>,
        job_type: Option<String>,
        rejection_count: u32,
        missing_credentials: Vec<String>,
        /// Per-step confidence colour: "green" | "amber" | "red"
        step_confidence: Vec<String>,
    },
    /// Emitted when user approves the agent's plan.
    PlanApproved {
        agent_id: String,
    },
    /// Emitted immediately when the user rejects the plan, before any replan starts.
    PlanRejected {
        agent_id: String,
        rejection_count: u32,
        max_rejections: u32,
        feedback: String,
        /// true  → agent will replan with feedback
        /// false → final rejection, agent stops
        will_replan: bool,
    },
    /// Emitted when user edits plan steps before approving.
    PlanEdited {
        agent_id: String,
        step_count: usize,
    },

    // ── Terminal ───────────────────────────────────────────────────────────
    GoalComplete {
        agent_id: String,
        summary: String,
    },
    GoalFailed {
        agent_id: String,
        reason: String,
    },
}

impl AgentEvent {
    /// Serialize to SSE wire format: `data: <json>\n\n`
    pub fn to_sse(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        format!("data: {}\n\n", json)
    }

    pub fn agent_id(&self) -> &str {
        match self {
            AgentEvent::PreflightStarted { agent_id, .. } => agent_id,
            AgentEvent::PreflightPassed { agent_id, .. } => agent_id,
            AgentEvent::PreflightFailed { agent_id, .. } => agent_id,
            AgentEvent::ClarificationNeeded { agent_id, .. } => agent_id,
            AgentEvent::ClarificationReceived { agent_id } => agent_id,
            AgentEvent::PlanningStarted { agent_id, .. } => agent_id,
            AgentEvent::PlanCreated { agent_id, .. } => agent_id,
            AgentEvent::StepStarted { agent_id, .. } => agent_id,
            AgentEvent::ToolCalled { agent_id, .. } => agent_id,
            AgentEvent::ToolResult { agent_id, .. } => agent_id,
            AgentEvent::StepCompleted { agent_id, .. } => agent_id,
            AgentEvent::StepRetrying { agent_id, .. } => agent_id,
            AgentEvent::PolicyDecision { agent_id, .. } => agent_id,
            AgentEvent::PiiRedacted { agent_id, .. } => agent_id,
            AgentEvent::SlaCheck { agent_id, .. } => agent_id,
            AgentEvent::CitationRecorded { agent_id, .. } => agent_id,
            AgentEvent::EvidencePackaged { agent_id, .. } => agent_id,
            AgentEvent::ReviewRequired { agent_id, .. } => agent_id,
            AgentEvent::JudgementSignal { agent_id, .. } => agent_id,
            AgentEvent::ConnectorTrigger { agent_id, .. } => agent_id,
            AgentEvent::ChildSpawned { agent_id, .. } => agent_id,
            AgentEvent::ChildrenComplete { agent_id, .. } => agent_id,
            AgentEvent::LlmCostUpdate { agent_id, .. } => agent_id,
            AgentEvent::PlanApprovalNeeded { agent_id, .. } => agent_id,
            AgentEvent::PlanApproved { agent_id, .. } => agent_id,
            AgentEvent::PlanRejected { agent_id, .. } => agent_id,
            AgentEvent::PlanEdited { agent_id, .. } => agent_id,
            AgentEvent::GoalComplete { agent_id, .. } => agent_id,
            AgentEvent::GoalFailed { agent_id, .. } => agent_id,
        }
    }
}

const CHANNEL_CAPACITY: usize = 256;

/// Shared event bus — one broadcast channel per active agent.
pub struct EventBus {
    channels: Arc<DashMap<String, broadcast::Sender<AgentEvent>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self { channels: Arc::new(DashMap::new()) }
    }

    pub fn publish(&self, event: AgentEvent) {
        let agent_id = event.agent_id().to_string();
        let sender =
            self.channels.entry(agent_id.clone()).or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0).clone();
        let _ = sender.send(event);
    }

    pub fn subscribe(&self, agent_id: &str) -> broadcast::Receiver<AgentEvent> {
        self.channels.entry(agent_id.to_string()).or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0).subscribe()
    }

    pub fn close(&self, agent_id: &str) {
        self.channels.remove(agent_id);
    }

    pub fn active_count(&self) -> usize {
        self.channels.len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(agent_id: &str) -> AgentEvent {
        AgentEvent::GoalComplete { agent_id: agent_id.into(), summary: "done".into() }
    }

    #[tokio::test]
    async fn test_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe("a1");
        bus.publish(sample_event("a1"));
        let event = rx.recv().await.expect("should receive event");
        assert_eq!(event.agent_id(), "a1");
    }

    #[tokio::test]
    async fn test_close_removes_channel() {
        let bus = EventBus::new();
        bus.publish(sample_event("a1"));
        assert_eq!(bus.active_count(), 1);
        bus.close("a1");
        assert_eq!(bus.active_count(), 0);
    }

    #[tokio::test]
    async fn test_sse_format() {
        let event = sample_event("a1");
        let sse = event.to_sse();
        assert!(sse.starts_with("data: "));
        assert!(sse.ends_with("\n\n"));
    }

    #[test]
    fn test_policy_decision_serialises_decision_tag() {
        let ev = AgentEvent::PolicyDecision {
            agent_id: "a1".into(),
            step_index: 0,
            tool: "shell".into(),
            decision: "block".into(),
            rule_id: Some("no_shell".into()),
            reason: Some("shell not permitted".into()),
            risk_level: "high".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["event"], "policy_decision");
        assert_eq!(json["decision"], "block");
        assert_eq!(json["risk_level"], "high");
    }

    #[test]
    fn test_citation_recorded_serialises_correctly() {
        let ev = AgentEvent::CitationRecorded {
            agent_id: "a1".into(),
            step_index: 2,
            claim: "Stripe charges 2.9%".into(),
            source_ref: "web_search_tool".into(),
            source_type: "tool_output".into(),
            confidence: 0.95,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["event"], "citation_recorded");
        assert_eq!(json["confidence"], 0.95);
    }

    #[test]
    fn test_review_required_serialises_correctly() {
        let ev = AgentEvent::ReviewRequired {
            agent_id: "a1".into(),
            review_id: "rev-123".into(),
            summary: "needs review".into(),
            reason: "policy rule triggered".into(),
            rule_id: Some("external_api_call".into()),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["event"], "review_required");
        assert_eq!(json["review_id"], "rev-123");
    }

    #[test]
    fn test_judgement_signal_serialises_correctly() {
        let ev = AgentEvent::JudgementSignal {
            agent_id: "a1".into(),
            step_index: 2,
            step_description: "Review the output".into(),
            job_type: Some("finance_accounting".into()),
            profile: "finance".into(),
            score: 0.81,
            confidence: 0.78,
            recommendation: "watch".into(),
            summary: "Judgement: watch closely".into(),
            reasons: vec!["retry count is 1".into()],
            timestamp: "2026-03-25T00:00:00Z".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["event"], "judgement_signal");
        assert_eq!(json["recommendation"], "watch");
        assert_eq!(json["step_index"], 2);
        assert_eq!(json["profile"], "finance");
    }
}
