//! SLA tracking and escalation logic for customer support agents.
//!
//! Tracks response time targets, resolution deadlines, and auto-escalates
//! when SLA thresholds are breached.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// SLA policy for a tenant or ticket category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaPolicy {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    /// Maximum time to first response (in minutes).
    pub first_response_mins: i64,
    /// Maximum time to resolution (in minutes).
    pub resolution_mins: i64,
    /// Priority level this policy applies to.
    pub priority: SlaPriority,
    /// What happens when the SLA is breached.
    pub escalation_rules: Vec<EscalationRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlaPriority {
    Low,
    Normal,
    High,
    Urgent,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationRule {
    /// Trigger after this percentage of SLA time has elapsed (e.g., 80 = 80%).
    pub trigger_pct: f64,
    /// Action to take.
    pub action: EscalationAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EscalationAction {
    /// Notify via webhook.
    Notify { message: String },
    /// Reassign to a different agent or team.
    Reassign { target: String },
    /// Pause the agent and escalate to human.
    EscalateToHuman { reason: String },
    /// Increase priority.
    IncreasePriority,
}

/// Tracks SLA status for a single ticket/goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaStatus {
    pub agent_id: String,
    pub policy_id: String,
    pub started_at: DateTime<Utc>,
    pub first_response_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub first_response_deadline: DateTime<Utc>,
    pub resolution_deadline: DateTime<Utc>,
    pub breached: bool,
    pub escalation_triggered: bool,
}

pub struct SlaTracker {
    policies: Vec<SlaPolicy>,
}

impl SlaTracker {
    pub fn new(policies: Vec<SlaPolicy>) -> Self {
        Self { policies }
    }

    /// Start SLA tracking for a new agent/ticket.
    pub fn start(&self, agent_id: &str, tenant_id: &str, priority: &SlaPriority) -> Option<SlaStatus> {
        let policy = self.policies.iter().find(|p| p.tenant_id == tenant_id && p.priority == *priority)?;

        let now = Utc::now();
        Some(SlaStatus {
            agent_id: agent_id.to_string(),
            policy_id: policy.id.clone(),
            started_at: now,
            first_response_at: None,
            resolved_at: None,
            first_response_deadline: now + Duration::minutes(policy.first_response_mins),
            resolution_deadline: now + Duration::minutes(policy.resolution_mins),
            breached: false,
            escalation_triggered: false,
        })
    }

    /// Check SLA status and determine if any escalation rules should fire.
    pub fn check(&self, status: &SlaStatus) -> Vec<EscalationAction> {
        let policy = match self.policies.iter().find(|p| p.id == status.policy_id) {
            Some(p) => p,
            None => return vec![],
        };

        let now = Utc::now();
        let mut actions = Vec::new();

        // Check first response SLA
        if status.first_response_at.is_none() && now > status.first_response_deadline {
            actions.push(EscalationAction::EscalateToHuman {
                reason: "first response SLA breached".into(),
            });
        }

        // Check resolution SLA
        if status.resolved_at.is_none() {
            let total_mins = policy.resolution_mins as f64;
            let elapsed_mins = (now - status.started_at).num_minutes() as f64;
            let pct_elapsed = (elapsed_mins / total_mins) * 100.0;

            for rule in &policy.escalation_rules {
                if pct_elapsed >= rule.trigger_pct {
                    actions.push(rule.action.clone());
                }
            }
        }

        actions
    }

    /// Record that the agent has made its first response.
    pub fn record_first_response(status: &mut SlaStatus) {
        if status.first_response_at.is_none() {
            status.first_response_at = Some(Utc::now());
        }
    }

    /// Record that the ticket/goal is resolved.
    pub fn record_resolution(status: &mut SlaStatus) {
        status.resolved_at = Some(Utc::now());
        if let (Some(_first), Some(resolved)) = (status.first_response_at, status.resolved_at) {
            status.breached = resolved > status.resolution_deadline;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_policy() -> SlaPolicy {
        SlaPolicy {
            id: "sla-1".into(),
            tenant_id: "t1".into(),
            name: "Standard SLA".into(),
            first_response_mins: 30,
            resolution_mins: 240,
            priority: SlaPriority::Normal,
            escalation_rules: vec![
                EscalationRule {
                    trigger_pct: 80.0,
                    action: EscalationAction::Notify { message: "SLA at 80%".into() },
                },
                EscalationRule {
                    trigger_pct: 100.0,
                    action: EscalationAction::EscalateToHuman { reason: "SLA breached".into() },
                },
            ],
        }
    }

    #[test]
    fn test_start_tracking() {
        let tracker = SlaTracker::new(vec![test_policy()]);
        let status = tracker.start("agent-1", "t1", &SlaPriority::Normal);
        assert!(status.is_some());
        let s = status.unwrap();
        assert!(!s.breached);
        assert!(s.first_response_at.is_none());
    }

    #[test]
    fn test_no_policy_returns_none() {
        let tracker = SlaTracker::new(vec![test_policy()]);
        let status = tracker.start("agent-1", "t1", &SlaPriority::Critical);
        assert!(status.is_none());
    }

    #[test]
    fn test_record_first_response() {
        let tracker = SlaTracker::new(vec![test_policy()]);
        let mut status = tracker.start("agent-1", "t1", &SlaPriority::Normal).unwrap();
        assert!(status.first_response_at.is_none());
        SlaTracker::record_first_response(&mut status);
        assert!(status.first_response_at.is_some());
    }
}
