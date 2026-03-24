//! Policy rules — tenant-configurable action gating.
//!
//! Examples:
//! - "refund > $50 requires human approval"
//! - "no docker/kubernetes for Free plan"
//! - "PII fields must be redacted before external API calls"

use serde::{Deserialize, Serialize};

/// A single policy rule evaluated before tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Which tools this rule applies to (empty = all tools).
    pub tools: Vec<String>,
    /// Condition expression — evaluated against tool args and agent context.
    pub condition: PolicyCondition,
    /// What happens when the condition matches.
    pub action: PolicyAction,
    pub enabled: bool,
}

/// Conditions that trigger a policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyCondition {
    /// Always matches.
    Always,
    /// Matches when a specific tool is called.
    ToolIs { tool: String },
    /// Matches when a JSON field in tool args exceeds a threshold.
    ArgThreshold { field: String, max: f64 },
    /// Matches when the agent's plan is in a specific plan tier.
    PlanIs { plan: String },
    /// Matches when the agent's risk level exceeds a threshold.
    RiskLevel { min_level: String },
    /// Matches a regex pattern against the tool arguments JSON.
    ArgsMatch { pattern: String },
    /// Combines multiple conditions with AND.
    All { conditions: Vec<PolicyCondition> },
    /// Combines multiple conditions with OR.
    Any { conditions: Vec<PolicyCondition> },
}

/// What to do when a policy rule matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyAction {
    /// Allow the action to proceed (log only).
    Allow,
    /// Block the action entirely.
    Block { reason: String },
    /// Require human approval before proceeding.
    RequireApproval { message: String },
    /// Redact specific fields before execution.
    Redact { fields: Vec<String> },
    /// Downgrade to a cheaper model.
    Downgrade { to_model: String },
}

/// A set of policy rules for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyRuleSet {
    pub tenant_id: String,
    pub rules: Vec<PolicyRule>,
}

impl PolicyRuleSet {
    pub fn new(tenant_id: String) -> Self {
        Self { tenant_id, rules: Vec::new() }
    }

    /// Default platform-wide safety policies.
    pub fn platform_defaults() -> Vec<PolicyRule> {
        vec![
            PolicyRule {
                id: "platform-no-critical".into(),
                name: "Block critical-risk tools".into(),
                tools: vec![],
                condition: PolicyCondition::RiskLevel { min_level: "critical".into() },
                action: PolicyAction::Block { reason: "critical-risk tools require explicit platform approval".into() },
                enabled: true,
            },
            PolicyRule {
                id: "platform-free-no-infra".into(),
                name: "Free plan cannot use infra tools".into(),
                tools: vec!["docker".into(), "kubernetes".into()],
                condition: PolicyCondition::PlanIs { plan: "free".into() },
                action: PolicyAction::Block { reason: "infrastructure tools require Pro or Enterprise plan".into() },
                enabled: true,
            },
        ]
    }
}
