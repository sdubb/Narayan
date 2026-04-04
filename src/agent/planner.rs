use serde::{Deserialize, Serialize};

use crate::{
    agent::workflow_compiler::{legacy_args_template_from_compiled_step, CompiledWorkflow, TypedExpression},
    gateway::LlmRole,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StepCondition {
    Deterministic(StructuredCondition),
    Expression(TypedExpression),
    // Semantic(String), // Future expansion
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredCondition {
    pub left: String,
    pub operator: ConditionOp,
    #[serde(default)]
    pub right: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConditionOp {
    Exists,
    NotExists,
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    GreaterThanEquals,
    LessThanEquals,
    NotEmpty,
    Empty,
    IsTruthy,
    IsFalsy,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkipReason {
    ConditionFalse,
    UpstreamSkipped,
    NoInput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlannedStep {
    pub index: usize,
    pub description: String,
    pub tool: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    #[serde(default)]
    pub success_criteria: String,
    #[serde(default)]
    pub condition: Option<StepCondition>,
    #[serde(default)]
    pub foreach: Option<String>,
    /// DAG dependency edges — indices of predecessor steps that must
    /// complete before this step can execute. Empty = no dependencies
    /// (or linear execution where the engine infers sequential deps).
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub job_type: Option<String>,
    pub steps: Vec<PlannedStep>,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AdaptiveResearchMemo {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub workflow_hints: Vec<String>,
}

impl Plan {
    pub fn next_step(&self, current_step: usize) -> Option<&PlannedStep> {
        self.steps.get(current_step)
    }
    pub fn is_complete(&self, current_step: usize) -> bool {
        current_step >= self.steps.len()
    }

    /// Build a deterministic Plan from a role's enriched workflow outline.
    /// No LLM call — templates are rendered against the trigger's input_data.
    pub fn from_workflow_outline(role: &crate::agent::definition::AgentRole, input_data: &serde_json::Value) -> Self {
        let steps = role
            .execution_guidelines
            .workflow_outline
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                let is_llm_worker = ws.tool.as_deref() == Some("llm_worker")
                    || (ws.tool.is_none() && ws.condition.is_none() && ws.foreach.is_none());
                let tool = ws.tool.clone().or_else(|| {
                    if ws.condition.is_some() || ws.foreach.is_some() {
                        None
                    } else {
                        Some(crate::agent::workflow_compiler::LLM_WORKER_TOOL_NAME.into())
                    }
                });
                let tool_args = ws.args_template.as_ref().map(|t| render_template(t, input_data)).or_else(|| {
                    if ws.condition.is_some() || ws.foreach.is_some() {
                        None
                    } else if is_llm_worker {
                        let role = crate::agent::workflow_compiler::infer_llm_role(&ws.description);
                        let generation =
                            crate::agent::workflow_compiler::llm_generation_for_hint(&ws.description, &role);
                        Some(serde_json::json!({
                            "instruction": ws.description,
                            "response_format": "json",
                            "llm_role": role,
                            "execution_intent": generation.execution_intent,
                            "budget_tier": generation.budget_tier,
                            "temperature": generation.temperature,
                            "max_tokens": generation.max_tokens,
                            "cost_budget_usd": generation.cost_budget_usd,
                            "output_schema": crate::agent::workflow_compiler::llm_output_schema(&role),
                        }))
                    } else {
                        Some(serde_json::json!({
                            "instruction": ws.description,
                            "response_format": "text",
                        }))
                    }
                });

                PlannedStep {
                    index: i,
                    description: ws.description.clone(),
                    tool,
                    tool_args,
                    success_criteria: if ws.success_criteria.trim().is_empty() {
                        format!("step {} complete", i + 1)
                    } else {
                        ws.success_criteria.clone()
                    },
                    condition: ws.condition.clone(),
                    foreach: ws.foreach.clone(),
                    depends_on: ws.depends_on.clone(),
                }
            })
            .collect();
        Plan {
            goal: role.purpose.clone(),
            job_type: Some(role.role_category.as_str().into()),
            steps,
            rationale: "deterministic plan from workflow outline".into(),
        }
    }

    /// Build a deterministic Plan directly from the compiled workflow artifact.
    /// This is the runtime path for the new compiler-first execution model.
    pub fn from_compiled_workflow(workflow: &CompiledWorkflow, role: &crate::agent::definition::AgentRole) -> Self {
        let steps = workflow
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let mut tool_args = legacy_args_template_from_compiled_step(step);
                if step.tool.as_deref() == Some(crate::agent::workflow_compiler::LLM_WORKER_TOOL_NAME) {
                    let role = step.llm_role.clone().unwrap_or(LlmRole::Drafter);
                    let generation = step
                        .llm_generation
                        .clone()
                        .unwrap_or_else(|| crate::agent::workflow_compiler::llm_generation_for_hint(&step.id, &role));
                    if let Some(map) = tool_args.as_object_mut() {
                        map.insert("llm_role".into(), serde_json::json!(role));
                        map.insert("execution_intent".into(), serde_json::json!(generation.execution_intent));
                        map.insert("budget_tier".into(), serde_json::json!(generation.budget_tier));
                        map.insert("temperature".into(), serde_json::json!(generation.temperature));
                        map.insert("max_tokens".into(), serde_json::json!(generation.max_tokens));
                        map.insert("cost_budget_usd".into(), serde_json::json!(generation.cost_budget_usd));
                        map.insert("response_format".into(), serde_json::json!("json"));
                        map.insert("output_schema".into(), step.output_schema.clone());
                    }
                }

                PlannedStep {
                    index,
                    description: format!("{}: {:?}", step.id, step.dsl_type),
                    tool: step.tool.clone(),
                    tool_args: Some(tool_args),
                    success_criteria: if step.success_criteria.is_empty() {
                        format!("step {} complete", index + 1)
                    } else {
                        step.success_criteria.join("; ")
                    },
                    condition: step.condition.clone(),
                    foreach: None,
                    depends_on: step
                        .depends_on
                        .iter()
                        .filter_map(|dep| dep.strip_prefix("step_").and_then(|value| value.parse::<usize>().ok()))
                        .map(|value| value.saturating_sub(1))
                        .collect(),
                }
            })
            .collect();

        Plan {
            goal: role.purpose.clone(),
            job_type: Some(role.role_category.as_str().into()),
            steps,
            rationale: "deterministic plan from compiled workflow".into(),
        }
    }
}

/// Recursively render `{input.*}` template placeholders in a JSON value.
/// Unresolved placeholders are left as-is so the executor LLM can handle them.
fn render_template(template: &serde_json::Value, input_data: &serde_json::Value) -> serde_json::Value {
    match template {
        serde_json::Value::String(s) => {
            let mut result = s.clone();
            // Find all {input.X} patterns and replace with values from input_data
            while let Some(start) = result.find("{input.") {
                let rest = &result[start + 7..];
                if let Some(end) = rest.find('}') {
                    let key = &rest[..end];
                    let replacement = input_data
                        .get(key)
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_else(|| format!("{{input.{}}}", key));
                    result = format!("{}{}{}", &result[..start], replacement, &rest[end + 1..]);
                } else {
                    break;
                }
            }
            serde_json::Value::String(result)
        }
        serde_json::Value::Object(map) => {
            let rendered: serde_json::Map<String, serde_json::Value> =
                map.iter().map(|(k, v)| (k.clone(), render_template(v, input_data))).collect();
            serde_json::Value::Object(rendered)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| render_template(v, input_data)).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_role() -> crate::agent::definition::AgentRole {
        let mut role = crate::agent::definition::AgentRole::new(
            "role-1".into(),
            "agent-1".into(),
            "tenant-1".into(),
            "Planner role".into(),
        );
        role.purpose = "Run a deterministic workflow".into();
        role.execution_guidelines.workflow_outline = vec![crate::agent::definition::WorkflowStep {
            description: "Inspect the source file".into(),
            tool: Some("file_read".into()),
            args_template: Some(serde_json::json!({ "path": "{input.file_path}" })),
            success_criteria: "source file inspected".into(),
            condition: None,
            ..Default::default()
        }];
        role
    }

    #[test]
    fn test_from_workflow_outline_preserves_success_criteria() {
        let role = make_role();
        let plan = Plan::from_workflow_outline(
            &role,
            &serde_json::json!({
                "file_path": "/tmp/ws/input.txt"
            }),
        );

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].description, "Inspect the source file");
        assert_eq!(plan.steps[0].tool.as_deref(), Some("file_read"));
        assert_eq!(plan.steps[0].success_criteria, "source file inspected");
        assert_eq!(
            plan.steps[0].tool_args.as_ref().and_then(|v| v.get("path")).and_then(|v| v.as_str()),
            Some("/tmp/ws/input.txt")
        );
    }
}
