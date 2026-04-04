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
    pub permission_mode: String,
    pub tool_pool: String,
    pub workspace_root: Option<String>,
}

/// Result of policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Allowed - proceed with execution.
    Allow,
    /// Blocked - do not execute.
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
        if let Some(decision) = self.evaluate_builtin_guards(ctx) {
            return decision;
        }

        for rule in &self.platform_rules {
            if !rule.enabled {
                continue;
            }
            if let Some(decision) = self.evaluate_rule(rule, ctx) {
                return decision;
            }
        }

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

    fn evaluate_builtin_guards(&self, ctx: &PolicyContext) -> Option<PolicyDecision> {
        let paths = target_paths(&ctx.tool_args);
        let writes_outside_workspace =
            paths.iter().any(|path| is_write_target(ctx, path) && path_leaves_workspace(path, ctx));
        let protected_path = paths.iter().find(|path| targets_protected_path(path)).cloned();
        let mutating_tool = is_mutating_tool(ctx);
        let external_side_effect = has_external_side_effect(ctx);

        if matches!(ctx.tool_name.as_str(), "enter_worktree" | "exit_worktree")
            && ctx.tool_args.get("explicit_user_request").and_then(|value| value.as_bool()) != Some(true)
        {
            return Some(PolicyDecision::Block {
                rule_id: "builtin-worktree-explicit-only".into(),
                reason: "worktree tools are explicit-use only and require explicit_user_request=true".into(),
            });
        }

        if let Some(command) = command_text(ctx) {
            if let Some(reason) = dangerous_shell_reason(&command) {
                return Some(PolicyDecision::Block { rule_id: "builtin-dangerous-shell".into(), reason });
            }
            if looks_destructive_shell_command(&command)
                && permission_mode_ord(&ctx.permission_mode) < permission_mode_ord("trusted_auto")
            {
                return Some(PolicyDecision::RequireApproval {
                    rule_id: "builtin-destructive-shell".into(),
                    message: "destructive shell or git operations require explicit approval in this permission mode"
                        .into(),
                });
            }
        }

        if ctx.permission_mode == "plan_only" && mutating_tool {
            return Some(PolicyDecision::RequireApproval {
                rule_id: "builtin-plan-only-mutation".into(),
                message: "this role is in plan_only mode, so mutating tools require explicit approval".into(),
            });
        }

        if ctx.tool_pool == "coordinator" && mutating_tool {
            return Some(PolicyDecision::RequireApproval {
                rule_id: "builtin-coordinator-mutation".into(),
                message: "coordinator-scoped roles should orchestrate rather than produce final artifacts directly"
                    .into(),
            });
        }

        if let Some(path) = protected_path {
            return Some(PolicyDecision::RequireApproval {
                rule_id: "builtin-protected-path".into(),
                message: format!("tool call touches protected path '{}' and requires approval", path),
            });
        }

        if writes_outside_workspace && permission_mode_ord(&ctx.permission_mode) < permission_mode_ord("trusted_auto") {
            return Some(PolicyDecision::RequireApproval {
                rule_id: "builtin-workspace-boundary".into(),
                message: "write targets outside the current workspace require approval".into(),
            });
        }

        if external_side_effect && permission_mode_ord(&ctx.permission_mode) < permission_mode_ord("trusted_auto") {
            return Some(PolicyDecision::RequireApproval {
                rule_id: "builtin-external-side-effect".into(),
                message: "external side effects require approval in the current permission mode".into(),
            });
        }

        None
    }

    fn evaluate_rule(&self, rule: &PolicyRule, ctx: &PolicyContext) -> Option<PolicyDecision> {
        if !rule.tools.is_empty() && !rule.tools.iter().any(|t| t == &ctx.tool_name) {
            return None;
        }

        if !self.matches_condition(&rule.condition, ctx) {
            return None;
        }

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
            PolicyCondition::PermissionModeIs { mode } => ctx.permission_mode == *mode,
            PolicyCondition::ToolPoolIs { pool } => ctx.tool_pool == *pool,
            PolicyCondition::RiskLevel { min_level } => risk_ord(&ctx.risk_level) >= risk_ord(min_level),
            PolicyCondition::ExternalSideEffect => has_external_side_effect(ctx),
            PolicyCondition::ProtectedPathTouched => {
                target_paths(&ctx.tool_args).iter().any(|path| targets_protected_path(path))
            }
            PolicyCondition::WritesOutsideWorkspace => target_paths(&ctx.tool_args)
                .iter()
                .any(|path| is_write_target(ctx, path) && path_leaves_workspace(path, ctx)),
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

fn permission_mode_ord(mode: &str) -> u8 {
    match mode {
        "plan_only" => 0,
        "safe_auto" => 1,
        "workspace_write" => 2,
        "trusted_auto" => 3,
        _ => 1,
    }
}

fn command_text(ctx: &PolicyContext) -> Option<String> {
    ["command", "cmd", "script"]
        .iter()
        .find_map(|key| ctx.tool_args.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn has_external_side_effect(ctx: &PolicyContext) -> bool {
    match ctx.tool_name.as_str() {
        "email" | "notification" | "pushover" | "api_call" | "register_api_tool" | "send_message" => true,
        "http_request" => ctx
            .tool_args
            .get("method")
            .and_then(|value| value.as_str())
            .map(|method| !matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD"))
            .unwrap_or(false),
        "mcp_session" => matches!(ctx.tool_args.get("action").and_then(|value| value.as_str()), Some("call_tool")),
        _ => false,
    }
}

fn is_mutating_tool(ctx: &PolicyContext) -> bool {
    match ctx.tool_name.as_str() {
        "file_write"
        | "file_edit"
        | "memory_store"
        | "memory_forget"
        | "git_operations"
        | "api_call"
        | "pushover"
        | "schedule"
        | "cron_add"
        | "cron_update"
        | "cron_remove"
        | "run_registered_wasm"
        | "code_run"
        | "compress"
        | "decompress"
        | "image_process"
        | "pdf_create"
        | "spreadsheet_write"
        | "email"
        | "notification"
        | "vector_store"
        | "vector_delete"
        | "delegate"
        | "send_message"
        | "enter_worktree"
        | "exit_worktree"
        | "create_workspace_tool"
        | "task_create"
        | "task_update"
        | "task_stop"
        | "task_output" => true,
        "message_inbox" => matches!(
            ctx.tool_args.get("action").and_then(|value| value.as_str()),
            Some("ack") | Some("continue_worker")
        ),
        "mcp_session" => matches!(ctx.tool_args.get("action").and_then(|value| value.as_str()), Some("call_tool")),
        "http_request" => ctx
            .tool_args
            .get("method")
            .and_then(|value| value.as_str())
            .map(|method| !matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD"))
            .unwrap_or(false),
        _ => false,
    }
}

fn is_write_target(ctx: &PolicyContext, path: &str) -> bool {
    if !is_mutating_tool(ctx) {
        return false;
    }
    !path.trim().is_empty()
}

fn target_paths(args: &serde_json::Value) -> Vec<String> {
    const PATH_KEYS: &[&str] =
        &["path", "paths", "output", "repo_path", "worktree_path", "workspace", "directory", "cwd", "root"];

    fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if PATH_KEYS.contains(&key.as_str()) {
                        match value {
                            serde_json::Value::String(path) => out.push(path.trim().to_string()),
                            serde_json::Value::Array(items) => {
                                for item in items {
                                    if let Some(path) = item.as_str() {
                                        out.push(path.trim().to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    collect(value, out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    collect(args, &mut out);
    out.retain(|value| !value.is_empty());
    out.sort();
    out.dedup();
    out
}

fn path_leaves_workspace(path: &str, ctx: &PolicyContext) -> bool {
    let normalized = normalize_path(path);
    if normalized.is_empty() {
        return false;
    }
    if contains_path_traversal(&normalized) {
        return true;
    }
    if !looks_absolute_path(&normalized) {
        return false;
    }
    let Some(workspace_root) = ctx.workspace_root.as_ref().map(|value| normalize_path(value)) else {
        return false;
    };
    !workspace_root.is_empty() && !normalized.starts_with(&workspace_root)
}

fn contains_path_traversal(path: &str) -> bool {
    path.split('/').any(|segment| segment == "..")
}

fn targets_protected_path(path: &str) -> bool {
    let normalized = normalize_path(path);
    if normalized.is_empty() {
        return false;
    }
    let protected_segments = [
        ".git",
        ".env",
        ".ssh",
        ".aws",
        ".kube",
        ".claude",
        ".narayan",
        "id_rsa",
        "id_ed25519",
        "authorized_keys",
        "known_hosts",
        "credentials",
    ];
    normalized
        .split('/')
        .any(|segment| protected_segments.iter().any(|candidate| segment.eq_ignore_ascii_case(candidate)))
}

fn normalize_path(path: &str) -> String {
    path.trim().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()
}

fn looks_absolute_path(path: &str) -> bool {
    path.starts_with('/') || path.as_bytes().get(1) == Some(&b':')
}

fn dangerous_shell_reason(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    let patterns = [
        ("git reset --hard", "git reset --hard is blocked by policy"),
        ("git checkout --", "git checkout -- is blocked by policy"),
        ("git clean -fd", "git clean -fd is blocked by policy"),
        ("rm -rf /", "rm -rf / is blocked by policy"),
        ("rm -rf ~", "rm -rf ~ is blocked by policy"),
        ("curl ", ""),
        ("wget ", ""),
    ];

    for (pattern, reason) in patterns {
        if lower.contains(pattern) {
            if pattern == "curl " || pattern == "wget " {
                if (lower.contains("| sh") || lower.contains("| bash")) && lower.contains(pattern) {
                    return Some("remote shell bootstrap commands are blocked by policy".into());
                }
            } else {
                return Some(reason.into());
            }
        }
    }

    if lower.contains("sudo ") || lower.starts_with("sudo") {
        return Some("sudo commands are blocked by policy".into());
    }

    None
}

fn looks_destructive_shell_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    ["rm -rf", "git reset", "git checkout --", "git clean", "chmod 777", "mv ", "del ", "remove-item"]
        .iter()
        .any(|pattern| lower.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::rules::{PolicyAction, PolicyCondition, PolicyRule};

    fn ctx(tool: &str, plan: &str, risk: &str) -> PolicyContext {
        PolicyContext {
            tenant_id: "t1".into(),
            agent_id: "a1".into(),
            tool_name: tool.into(),
            tool_args: serde_json::json!({}),
            plan: plan.into(),
            risk_level: risk.into(),
            permission_mode: "safe_auto".into(),
            tool_pool: "worker".into(),
            workspace_root: Some("/workspace/project".into()),
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
            action: PolicyAction::RequireApproval { message: "Refund exceeds $50 - requires human approval".into() },
            enabled: true,
        });

        let mut c = ctx("api_call", "pro", "medium");
        c.tool_args = serde_json::json!({ "amount": 75.0 });
        let decision = engine.evaluate(&c, &rules);
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));

        c.tool_args = serde_json::json!({ "amount": 25.0 });
        let decision = engine.evaluate(&c, &rules);
        assert!(
            matches!(decision, PolicyDecision::RequireApproval { .. }),
            "builtin external-side-effect guard should still escalate api_call"
        );
    }

    #[test]
    fn test_plan_only_mode_requires_approval_for_mutation() {
        let engine = PolicyEngine::new();
        let rules = PolicyRuleSet::new("t1".into());
        let mut c = ctx("file_write", "pro", "medium");
        c.permission_mode = "plan_only".into();
        c.tool_args = serde_json::json!({ "path": "notes.txt" });
        let decision = engine.evaluate(&c, &rules);
        assert!(
            matches!(decision, PolicyDecision::RequireApproval { rule_id, .. } if rule_id == "builtin-plan-only-mutation")
        );
    }

    #[test]
    fn test_workspace_boundary_requires_approval_for_writes() {
        let engine = PolicyEngine::new();
        let rules = PolicyRuleSet::new("t1".into());
        let mut c = ctx("file_write", "pro", "medium");
        c.permission_mode = "workspace_write".into();
        c.tool_args = serde_json::json!({ "path": "/tmp/outside.txt" });
        let decision = engine.evaluate(&c, &rules);
        assert!(
            matches!(decision, PolicyDecision::RequireApproval { rule_id, .. } if rule_id == "builtin-workspace-boundary")
        );
    }

    #[test]
    fn test_protected_path_requires_approval() {
        let engine = PolicyEngine::new();
        let rules = PolicyRuleSet::new("t1".into());
        let mut c = ctx("file_edit", "pro", "medium");
        c.permission_mode = "workspace_write".into();
        c.tool_args = serde_json::json!({ "path": "/workspace/project/.env" });
        let decision = engine.evaluate(&c, &rules);
        assert!(
            matches!(decision, PolicyDecision::RequireApproval { rule_id, .. } if rule_id == "builtin-protected-path")
        );
    }

    #[test]
    fn test_dangerous_shell_is_blocked() {
        let engine = PolicyEngine::new();
        let rules = PolicyRuleSet::new("t1".into());
        let mut c = ctx("shell", "pro", "medium");
        c.tool_args = serde_json::json!({ "command": "git reset --hard HEAD~1" });
        let decision = engine.evaluate(&c, &rules);
        assert!(matches!(decision, PolicyDecision::Block { rule_id, .. } if rule_id == "builtin-dangerous-shell"));
    }

    #[test]
    fn test_external_side_effect_condition_matches() {
        let engine = PolicyEngine::new();
        let mut c = ctx("http_request", "pro", "medium");
        c.tool_args = serde_json::json!({ "method": "POST", "url": "https://example.com" });
        assert!(engine.matches_condition(&PolicyCondition::ExternalSideEffect, &c));
    }
}
