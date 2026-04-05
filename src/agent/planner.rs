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
