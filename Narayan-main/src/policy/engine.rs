//! Policy evaluation engine.
//!
//! Evaluates policy rules against a tool call context and returns a decision.
//! Called by the executor before each tool execution (replaces/extends plane_guard).

use serde::{Deserialize, Serialize};

use crate::policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet};

/// Context passed to the policy engine for evaluation.
#[derive(Debug, Clone)]
pub struct PolicyContext {
    pub tenant_id: String,
    pub agent_id: String,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub plan: String,
    pub risk_level: String,
}

/// Result of policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Allowed — proceed with execution.
    Allow,
    /// Blocked — do not execute.
    Block { rule_id: String, reason: String },
    /// Needs human approval.
    RequireApproval { rule_id: String, message: String },
    /// Proceed but with redacted fields.
    Redact { rule_id: String, fields: Vec<String> },
    /// Proceed but downgrade the model.
    Downgrade { rule_id: String, to_model: String },
}

pub struct PolicyEngine {
    platform_rules: Vec<PolicyRule>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self { platform_rules: PolicyRuleSet::platform_defaults() }
    }

    /// Evaluate all applicable rules against the context.
    /// Returns the first blocking/escalating decision, or Allow if all pass.
    pub fn evaluate(&self, ctx: &PolicyContext, tenant_rules: &PolicyRuleSet) -> PolicyDecision {
        // Platform rules take priority
        for rule in &self.platform_rules {
            if !rule.enabled {
                continue;
            }
            if let Some(decision) = self.evaluate_rule(rule, ctx) {
                return decision;
            }
        }

        // Then tenant-specific rules
        for rule in &tenant_rules.rules {
            if !rule.enabled {
                continue;
            }
            if let Some(decision) = self.evaluate_rule(rule, ctx) {
                return decision;
            }
        }

        PolicyDecision::Allow
    }

    fn evaluate_rule(&self, rule: &PolicyRule, ctx: &PolicyContext) -> Option<PolicyDecision> {
        // Check if rule applies to this tool
        if !rule.tools.is_empty() && !rule.tools.iter().any(|t| t == &ctx.tool_name) {
            return None;
        }

        // Evaluate condition
        if !self.matches_condition(&rule.condition, ctx) {
            return None;
        }

        // Condition matched — return the action as a decision
        Some(match &rule.action {
            PolicyAction::Allow => PolicyDecision::Allow,
            PolicyAction::Block { reason } => {
                PolicyDecision::Block { rule_id: rule.id.clone(), reason: reason.clone() }
            }
            PolicyAction::RequireApproval { message } => {
                PolicyDecision::RequireApproval { rule_id: rule.id.clone(), message: message.clone() }
            }
            PolicyAction::Redact { fields } => {
                PolicyDecision::Redact { rule_id: rule.id.clone(), fields: fields.clone() }
            }
            PolicyAction::Downgrade { to_model } => {
                PolicyDecision::Downgrade { rule_id: rule.id.clone(), to_model: to_model.clone() }
            }
        })
    }

    fn matches_condition(&self, condition: &PolicyCondition, ctx: &PolicyContext) -> bool {
        match condition {
            PolicyCondition::Always => true,
            PolicyCondition::ToolIs { tool } => ctx.tool_name == *tool,
            PolicyCondition::PlanIs { plan } => ctx.plan == *plan,
            PolicyCondition::RiskLevel { min_level } => risk_ord(&ctx.risk_level) >= risk_ord(min_level),
            PolicyCondition::ArgThreshold { field, max } => {
                ctx.tool_args.get(field).and_then(|v| v.as_f64()).map(|v| v > *max).unwrap_or(false)
            }
            PolicyCondition::ArgsMatch { pattern } => {
                regex::Regex::new(pattern).ok().map(|re| re.is_match(&ctx.tool_args.to_string())).unwrap_or(false)
            }
            PolicyCondition::All { conditions } => conditions.iter().all(|c| self.matches_condition(c, ctx)),
            PolicyCondition::Any { conditions } => conditions.iter().any(|c| self.matches_condition(c, ctx)),
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn risk_ord(level: &str) -> u8 {
    match level {
        "low" => 1,
        "medium" => 2,
        "high" => 3,
        "critical" => 4,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(tool: &str, plan: &str, risk: &str) -> PolicyContext {
        PolicyContext {
            tenant_id: "t1".into(),
            agent_id: "a1".into(),
            tool_name: tool.into(),
            tool_args: serde_json::json!({}),
            plan: plan.into(),
            risk_level: risk.into(),
        }
    }

    #[test]
    fn test_platform_blocks_critical() {
        let engine = PolicyEngine::new();
        let rules = PolicyRuleSet::new("t1".into());
        let decision = engine.evaluate(&ctx("dangerous_tool", "pro", "critical"), &rules);
        assert!(matches!(decision, PolicyDecision::Block { .. }));
    }

    #[test]
    fn test_platform_blocks_free_infra() {
        let engine = PolicyEngine::new();
        let rules = PolicyRuleSet::new("t1".into());
        let decision = engine.evaluate(&ctx("docker", "free", "high"), &rules);
        assert!(matches!(decision, PolicyDecision::Block { .. }));
    }

    #[test]
    fn test_pro_plan_allows_docker() {
        let engine = PolicyEngine::new();
        let rules = PolicyRuleSet::new("t1".into());
        let decision = engine.evaluate(&ctx("docker", "pro", "high"), &rules);
        assert!(matches!(decision, PolicyDecision::Allow));
    }

    #[test]
    fn test_tenant_rule_threshold() {
        let engine = PolicyEngine::new();
        let mut rules = PolicyRuleSet::new("t1".into());
        rules.rules.push(PolicyRule {
            id: "refund-limit".into(),
            name: "Refunds over $50 need approval".into(),
            tools: vec!["api_call".into()],
            condition: PolicyCondition::ArgThreshold { field: "amount".into(), max: 50.0 },
            action: PolicyAction::RequireApproval { message: "Refund exceeds $50 — requires human approval".into() },
            enabled: true,
        });

        let mut c = ctx("api_call", "pro", "medium");
        c.tool_args = serde_json::json!({ "amount": 75.0 });
        let decision = engine.evaluate(&c, &rules);
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));

        c.tool_args = serde_json::json!({ "amount": 25.0 });
        let decision = engine.evaluate(&c, &rules);
        assert!(matches!(decision, PolicyDecision::Allow));
    }
}
