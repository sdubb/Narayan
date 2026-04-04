//! Runtime template variable resolution for tool arguments.
//!
//! Extends the existing `{input.*}` template syntax with:
//!   - `{{$.deps.step-N.output.field}}` — predecessor step outputs (DAG + linear)
//!   - `{{state.field}}` — agent state fields (goal, workspace_path, tenant_id)
//!   - `{{now}}` — current ISO-8601 timestamp
//!   - `{{last_run_at}}` — last successful run timestamp from metadata
//!   - Preserves existing `{input.*}` syntax for backwards compatibility
//!
//! Resolution happens in the executor BEFORE the fast-path check, so steps
//! with templates in their tool_args can still hit the zero-LLM path once
//! all templates resolve to concrete values.

use serde_json::Value;
use std::collections::HashMap;

/// Context for resolving template variables at runtime.
pub struct TemplateContext {
    /// Predecessor step outputs keyed by step reference (e.g., "step-0", "step-1")
    pub predecessor_outputs: HashMap<String, Value>,
    /// Agent state metadata (flat key-value from state.metadata)
    pub state_metadata: Value,
    /// Agent goal
    pub goal: String,
    /// Agent workspace path
    pub workspace_path: String,
    /// Agent tenant ID
    pub tenant_id: String,
    /// Input data from trigger/role
    pub input_data: Value,
}

impl TemplateContext {
    /// Build a TemplateContext from AgentState for linear execution.
    /// Predecessor outputs are extracted from `state.metadata.step_N_output`.
    pub fn from_agent_state(state: &crate::state::AgentState) -> Self {
        let mut predecessor_outputs = HashMap::new();

        // Extract step outputs from metadata (linear path stores them as step_N_output)
        if let Some(obj) = state.metadata.as_object() {
            for (key, value) in obj {
                if let Some(rest) = key.strip_prefix("step_") {
                    if rest.ends_with("_output") {
                        let step_num = &rest[..rest.len() - 7]; // strip "_output"
                        if step_num.chars().all(|c| c.is_ascii_digit()) {
                            let step_key = format!("step-{}", step_num);
                            predecessor_outputs.insert(step_key, value.clone());
                        }
                    }
                }
            }
        }

        let input_data = state.metadata.get("input_data").cloned().unwrap_or_else(|| serde_json::json!({}));

        Self {
            predecessor_outputs,
            state_metadata: state.metadata.clone(),
            goal: state.goal.clone(),
            workspace_path: state.workspace_path.clone(),
            tenant_id: state.tenant_id.clone(),
            input_data,
        }
    }

    /// Build a TemplateContext from DAG step input (predecessor outputs already resolved).
    pub fn from_dag_input(input: &crate::agent::dag_engine::StepInput, state: &crate::state::AgentState) -> Self {
        let input_data = state.metadata.get("input_data").cloned().unwrap_or_else(|| serde_json::json!({}));

        Self {
            predecessor_outputs: input.predecessor_outputs.clone(),
            state_metadata: state.metadata.clone(),
            goal: state.goal.clone(),
            workspace_path: state.workspace_path.clone(),
            tenant_id: state.tenant_id.clone(),
            input_data,
        }
    }
}

/// Check if a JSON value contains any `{{...}}` template patterns.
pub fn has_templates(value: &Option<Value>) -> bool {
    match value {
        None => false,
        Some(v) => value_has_templates(v),
    }
}

fn value_has_templates(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains("{{") && s.contains("}}"),
        Value::Object(map) => map.values().any(value_has_templates),
        Value::Array(arr) => arr.iter().any(value_has_templates),
        _ => false,
    }
}

/// Resolve all `{{...}}` template variables in a JSON value.
///
/// Template syntax:
///   `{{$.deps.step-N.output.field}}` — predecessor output
///   `{{state.goal}}` — agent state field
///   `{{now}}` — current ISO-8601 timestamp
///   `{{last_run_at}}` — last successful run from metadata
///   `{{input.field}}` — trigger input data
///
/// Unresolved templates are left as-is (executor LLM will handle them).
pub fn resolve_templates(value: &Value, ctx: &TemplateContext) -> Value {
    match value {
        Value::String(s) => {
            if !s.contains("{{") {
                return value.clone();
            }
            let resolved = resolve_string_templates(s, ctx);
            Value::String(resolved)
        }
        Value::Object(map) => {
            let resolved: serde_json::Map<String, Value> =
                map.iter().map(|(k, v)| (k.clone(), resolve_templates(v, ctx))).collect();
            Value::Object(resolved)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| resolve_templates(v, ctx)).collect()),
        other => other.clone(),
    }
}

/// Resolve template patterns in a single string.
fn resolve_string_templates(s: &str, ctx: &TemplateContext) -> String {
    let mut result = s.to_string();

    // Use a loop to handle all {{...}} patterns
    loop {
        let Some(start) = result.find("{{") else {
            break;
        };
        let rest = &result[start + 2..];
        let Some(end) = rest.find("}}") else {
            break;
        };

        let var_name = rest[..end].trim();
        let replacement = resolve_single_var(var_name, ctx);

        match replacement {
            Some(val) => {
                // If the entire string is just one template, return the raw value type
                // But since we're in resolve_string_templates, we always return a string
                let val_str = match &val {
                    Value::String(s) => s.clone(),
                    Value::Null => "null".to_string(),
                    other => other.to_string(),
                };
                result = format!("{}{}{}", &result[..start], val_str, &rest[end + 2..]);
            }
            None => {
                // Leave unresolved — skip past this template to avoid infinite loop
                let skip_to = start + 2 + end + 2;
                if skip_to >= result.len() {
                    break;
                }
                // Check for more templates after this one
                let remaining = &result[skip_to..];
                if !remaining.contains("{{") {
                    break;
                }
                continue;
            }
        }
    }

    result
}

/// Resolve a single template variable name to its value.
fn resolve_single_var(var: &str, ctx: &TemplateContext) -> Option<Value> {
    // Built-in variables
    match var {
        "now" => return Some(Value::String(chrono::Utc::now().to_rfc3339())),
        "last_run_at" => {
            return ctx
                .state_metadata
                .get("last_successful_run_at")
                .cloned()
                .or_else(|| Some(Value::String("1970-01-01T00:00:00Z".to_string())));
        }
        _ => {}
    }

    // State variables: {{state.goal}}, {{state.workspace_path}}, etc.
    if let Some(field) = var.strip_prefix("state.") {
        return match field {
            "goal" => Some(Value::String(ctx.goal.clone())),
            "workspace_path" => Some(Value::String(ctx.workspace_path.clone())),
            "tenant_id" => Some(Value::String(ctx.tenant_id.clone())),
            _ => ctx.state_metadata.get(field).cloned(),
        };
    }

    // Input variables: {{input.field}} — from trigger input data
    if let Some(field) = var.strip_prefix("input.") {
        return resolve_nested_path(field, &ctx.input_data);
    }

    // Predecessor outputs: {{$.deps.step-N.output.field}} or {{step-N.field}}
    let normalized = var.strip_prefix("$.deps.").or_else(|| var.strip_prefix("deps.")).unwrap_or(var);

    resolve_predecessor_path(normalized, &ctx.predecessor_outputs)
}

/// Resolve a dot-notation path against predecessor outputs.
/// Handles: "step-0.output.count", "result_of_step_0.output.count"
fn resolve_predecessor_path(path: &str, predecessors: &HashMap<String, Value>) -> Option<Value> {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        return None;
    }

    // Resolve step reference from first segment
    let step_key = if let Some(rest) = segments[0].strip_prefix("result_of_step_") {
        let digit_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digit_len == 0 {
            return None;
        }
        format!("step-{}", &rest[..digit_len])
    } else {
        segments[0].to_string()
    };

    let mut current = predecessors.get(&step_key)?.clone();

    // Walk remaining path segments
    for segment in &segments[1..] {
        if segment.is_empty() {
            continue;
        }
        if let Some(val) = current.get(*segment) {
            current = val.clone();
        } else if let Ok(idx) = segment.parse::<usize>() {
            current = current.get(idx)?.clone();
        } else {
            return None;
        }
    }

    Some(current)
}

/// Resolve nested dot-path in a JSON value (e.g., "user.name" in input_data).
fn resolve_nested_path(path: &str, value: &Value) -> Option<Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current.clone())
}

/// Resolve templates in a PlannedStep's tool_args, returning a modified step.
/// This is the main entry point called from the executor.
pub fn resolve_step_templates(
    step: &crate::agent::planner::PlannedStep,
    ctx: &TemplateContext,
) -> crate::agent::planner::PlannedStep {
    let mut resolved = step.clone();

    if let Some(ref args) = step.tool_args {
        if value_has_templates(args) {
            let resolved_args = resolve_templates(args, ctx);
            tracing::debug!(
                step_index = step.index,
                original = ?args,
                resolved = ?resolved_args,
                "template variables resolved in tool_args"
            );
            resolved.tool_args = Some(resolved_args);
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> TemplateContext {
        let mut predecessors = HashMap::new();
        predecessors.insert(
            "step-0".to_string(),
            serde_json::json!({
                "output": {
                    "count": 42,
                    "users": [{"name": "Alice"}, {"name": "Bob"}]
                }
            }),
        );
        predecessors.insert(
            "step-1".to_string(),
            serde_json::json!({
                "output": "filtered results"
            }),
        );

        TemplateContext {
            predecessor_outputs: predecessors,
            state_metadata: serde_json::json!({
                "last_successful_run_at": "2026-04-01T00:00:00Z",
                "role_id": "test-role"
            }),
            goal: "monitor database".to_string(),
            workspace_path: "/tmp/ws".to_string(),
            tenant_id: "tenant-1".to_string(),
            input_data: serde_json::json!({
                "topic": "security vulnerabilities",
                "db_name": "production"
            }),
        }
    }

    #[test]
    fn test_resolve_predecessor_output() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "query": "SELECT * FROM users WHERE count > {{$.deps.step-0.output.count}}"
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(result["query"].as_str().unwrap(), "SELECT * FROM users WHERE count > 42");
    }

    #[test]
    fn test_resolve_input_var() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "query": "{{input.topic}}"
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(result["query"].as_str().unwrap(), "security vulnerabilities");
    }

    #[test]
    fn test_resolve_state_var() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "goal": "{{state.goal}}",
            "workspace": "{{state.workspace_path}}"
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(result["goal"].as_str().unwrap(), "monitor database");
        assert_eq!(result["workspace"].as_str().unwrap(), "/tmp/ws");
    }

    #[test]
    fn test_resolve_now() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "timestamp": "{{now}}"
        });
        let result = resolve_templates(&input, &ctx);
        // Should be a valid ISO timestamp
        assert!(result["timestamp"].as_str().unwrap().contains("T"));
    }

    #[test]
    fn test_resolve_last_run_at() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "since": "{{last_run_at}}"
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(result["since"].as_str().unwrap(), "2026-04-01T00:00:00Z");
    }

    #[test]
    fn test_unresolved_left_as_is() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "unknown": "{{nonexistent.path}}"
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(result["unknown"].as_str().unwrap(), "{{nonexistent.path}}");
    }

    #[test]
    fn test_has_templates() {
        assert!(!has_templates(&None));
        assert!(!has_templates(&Some(serde_json::json!({"key": "value"}))));
        assert!(has_templates(&Some(serde_json::json!({"key": "{{now}}"}))));
        assert!(has_templates(&Some(serde_json::json!({"nested": {"key": "{{input.x}}"}}))));
    }

    #[test]
    fn test_no_templates_passthrough() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "query": "SELECT * FROM users",
            "limit": 10
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(result, input);
    }

    #[test]
    fn test_mixed_templates_and_static() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "query": "SELECT * FROM {{input.db_name}} WHERE created_at > '{{last_run_at}}'"
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(
            result["query"].as_str().unwrap(),
            "SELECT * FROM production WHERE created_at > '2026-04-01T00:00:00Z'"
        );
    }

    #[test]
    fn test_result_of_step_format() {
        let ctx = test_ctx();
        let input = serde_json::json!({
            "data": "{{result_of_step_0.output.count}}"
        });
        let result = resolve_templates(&input, &ctx);
        assert_eq!(result["data"].as_str().unwrap(), "42");
    }
}
