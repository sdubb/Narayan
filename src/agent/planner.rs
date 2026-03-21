use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    agent::prompts::{build_conversation_history, is_direct_response_goal, JobType, PlannerPrompt},
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::AgentState,
    storage::PostgresStore,
};

fn truncate_for_log(value: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(value.len().min(max_chars));
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...(truncated)");
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepCondition {
    pub reference: String,
    pub operator: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedStep {
    pub index: usize,
    pub description: String,
    pub tool: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    #[serde(default)]
    pub success_criteria: String,
    #[serde(default)]
    pub condition: Option<StepCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub job_type: Option<String>,
    pub steps: Vec<PlannedStep>,
    #[serde(default)]
    pub rationale: String,
}

impl Plan {
    pub fn next_step(&self, current_step: usize) -> Option<&PlannedStep> {
        self.steps.get(current_step)
    }
    pub fn is_complete(&self, current_step: usize) -> bool {
        current_step >= self.steps.len()
    }
}

#[async_trait]
pub trait Planner: Send + Sync {
    async fn create_plan(&self, state: &AgentState, context: &str, available_tools: &[&str]) -> Result<Plan>;

    async fn revise_plan(&self, plan: &Plan, state: &AgentState, feedback: &str) -> Result<Plan>;
}

pub struct LlmPlanner {
    gateway: Arc<dyn LlmGateway>,
    store: Option<Arc<PostgresStore>>,
}

impl LlmPlanner {
    pub fn new(gateway: Arc<dyn LlmGateway>) -> Self {
        Self { gateway, store: None }
    }

    pub fn with_store(mut self, store: Arc<PostgresStore>) -> Self {
        self.store = Some(store);
        self
    }

    async fn conversation_history(&self, state: &AgentState) -> String {
        let conv_id = match &state.conversation_id {
            Some(id) => id,
            None => return String::new(),
        };
        let store = match &self.store {
            Some(s) => s,
            None => return String::new(),
        };
        match store.list_agents_in_conversation(&state.tenant_id, conv_id).await {
            Ok(agents) => build_conversation_history(&agents, &state.id),
            Err(e) => {
                tracing::warn!(agent_id = %state.id, error = %e, "failed to load conversation history for planner");
                String::new()
            }
        }
    }
}

#[async_trait]
impl Planner for LlmPlanner {
    async fn create_plan(&self, state: &AgentState, context: &str, available_tools: &[&str]) -> Result<Plan> {
        if is_direct_response_goal(&state.goal) {
            tracing::info!(
                agent_id = %state.id,
                goal = %state.goal,
                "planner selected direct-response fast path"
            );
            return Ok(Plan {
                goal: state.goal.clone(),
                job_type: Some("general".into()),
                rationale: "Simple conversational request; answer the user directly without tools.".into(),
                steps: vec![PlannedStep {
                    index: 0,
                    description: "Answer the user's message directly in chat.".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: "User receives a complete direct answer.".into(),
                    condition: None,
                }],
            });
        }

        let job_type = JobType::detect(&state.goal);

        let system = PlannerPrompt::system(&job_type);
        let manifest = crate::tools::selector::tool_manifest_from_names(available_tools);
        let conv_history = self.conversation_history(state).await;
        let user = PlannerPrompt::user_create(state, context, &manifest, &conv_history);

        tracing::debug!(
            agent_id = %state.id,
            job_type = job_type.label(),
            "creating plan"
        );
        tracing::info!(
            agent_id = %state.id,
            goal = %state.goal,
            job_type = job_type.label(),
            context = %truncate_for_log(context, 400),
            manifest = %truncate_for_log(&manifest, 1200),
            "planner request prepared"
        );

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Complex,
            vec![Message::system(system), Message::user(user)],
        );

        let resp = self.gateway.chat(request).await?;
        let raw = resp.content.unwrap_or_default();
        tracing::info!(
            agent_id = %state.id,
            response = %truncate_for_log(&raw, 1200),
            "planner response received"
        );

        // Strip markdown code fences if model wrapped the JSON
        let cleaned = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();

        match serde_json::from_str::<Plan>(cleaned) {
            Ok(mut plan) => {
                normalize_plan(&mut plan);
                tracing::info!(
                    agent_id = %state.id,
                    steps    = plan.steps.len(),
                    job_type = job_type.label(),
                    "plan created"
                );
                Ok(plan)
            }
            Err(e) => {
                tracing::warn!(
                    agent_id = %state.id,
                    error    = %e,
                    raw      = %&raw[..raw.len().min(200)],
                    "planner returned unparseable JSON — using single-step fallback"
                );
                Ok(Plan {
                    goal: state.goal.clone(),
                    job_type: Some(job_type.label().to_string()),
                    rationale: String::new(),
                    steps: vec![PlannedStep {
                        index: 0,
                        description: state.goal.clone(),
                        tool: None,
                        tool_args: None,
                        success_criteria: String::new(),
                        condition: None,
                    }],
                })
            }
        }
    }

    async fn revise_plan(&self, plan: &Plan, state: &AgentState, feedback: &str) -> Result<Plan> {
        let user = PlannerPrompt::user_revise(plan, feedback, state);
        let job_type = JobType::detect(&state.goal);
        let system = PlannerPrompt::system(&job_type);

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Medium,
            vec![Message::system(system), Message::user(user)],
        );

        let resp = self.gateway.chat(request).await?;
        let raw = resp.content.unwrap_or_default();
        let cleaned = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();

        match serde_json::from_str::<Plan>(cleaned) {
            Ok(mut revised) => {
                normalize_plan(&mut revised);
                tracing::info!(
                    agent_id  = %state.id,
                    new_steps = revised.steps.len(),
                    "plan revised"
                );
                Ok(revised)
            }
            Err(_) => {
                tracing::warn!(agent_id = %state.id, "plan revision failed to parse, keeping original");
                Ok(plan.clone())
            }
        }
    }
}

fn normalize_plan(plan: &mut Plan) {
    for step in &mut plan.steps {
        let normalized = step.tool.as_deref().map(str::trim).map(str::to_lowercase);
        if matches!(normalized.as_deref(), Some("") | Some("null") | Some("none")) {
            step.tool = None;
        }
        if let Some(condition) = step.condition.as_mut() {
            condition.reference = condition.reference.trim().to_string();
            condition.operator = condition.operator.trim().to_ascii_lowercase();
            if condition.reference.is_empty() || condition.operator.is_empty() {
                step.condition = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::providers::ChatResponse;

    struct MockGateway {
        responses: Mutex<Vec<ChatResponse>>,
    }

    impl MockGateway {
        fn from_contents(contents: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(
                    contents
                        .into_iter()
                        .map(|content| ChatResponse {
                            content: Some(content.to_string()),
                            tool_calls: vec![],
                            input_tokens: 0,
                            output_tokens: 0,
                        })
                        .collect(),
                ),
            }
        }
    }

    #[async_trait]
    impl LlmGateway for MockGateway {
        async fn chat(&self, _request: GatewayRequest) -> Result<ChatResponse> {
            let mut responses = self.responses.lock().expect("responses lock should succeed");
            Ok(responses.remove(0))
        }
    }

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "fix CI pipeline".into(), "/tmp/ws".into())
    }

    #[tokio::test]
    async fn test_create_plan_parses_valid_json_response() {
        let planner = LlmPlanner::new(Arc::new(MockGateway::from_contents(vec![
            r#"{
            "goal":"fix CI pipeline",
            "job_type":"software_engineer",
            "steps":[
                {"index":0,"description":"Inspect failing workflow","tool":"file_read","tool_args":{"path":".github/workflows/ci.yml"},"success_criteria":"workflow reviewed"}
            ],
            "rationale":"understand the failure before changing code"
        }"#,
        ])));

        let plan = planner
            .create_plan(&make_state(), "previous failure in CI", &["file_read", "shell"])
            .await
            .expect("plan should parse");

        assert_eq!(plan.goal, "fix CI pipeline");
        assert_eq!(plan.job_type.as_deref(), Some("software_engineer"));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].tool.as_deref(), Some("file_read"));
    }

    #[tokio::test]
    async fn test_create_plan_falls_back_to_single_step_when_json_is_invalid() {
        let planner = LlmPlanner::new(Arc::new(MockGateway::from_contents(vec!["not valid json"])));
        let state = make_state();

        let plan = planner.create_plan(&state, "", &["shell"]).await.expect("fallback plan should be returned");

        assert_eq!(plan.goal, state.goal);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].description, state.goal);
        assert!(plan.steps[0].tool.is_none());
    }

    #[tokio::test]
    async fn test_revise_plan_returns_original_when_revision_json_is_invalid() {
        let planner = LlmPlanner::new(Arc::new(MockGateway::from_contents(vec!["{bad json"])));
        let state = make_state();
        let original = Plan {
            goal: state.goal.clone(),
            job_type: Some("software_engineer".into()),
            steps: vec![PlannedStep {
                index: 0,
                description: "Inspect failing workflow".into(),
                tool: Some("file_read".into()),
                tool_args: None,
                success_criteria: "workflow reviewed".into(),
                condition: None,
            }],
            rationale: "inspect first".into(),
        };

        let revised = planner
            .revise_plan(&original, &state, "change remaining work")
            .await
            .expect("original plan should be retained");

        assert_eq!(revised.goal, original.goal);
        assert_eq!(revised.steps.len(), original.steps.len());
        assert_eq!(revised.steps[0].description, original.steps[0].description);
    }
}
