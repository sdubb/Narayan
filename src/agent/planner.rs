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
            .map(|(i, ws)| PlannedStep {
                index: i,
                description: ws.description.clone(),
                tool: ws.tool.clone(),
                tool_args: ws.args_template.as_ref().map(|t| render_template(t, input_data)),
                success_criteria: if ws.success_criteria.trim().is_empty() {
                    format!("step {} complete", i + 1)
                } else {
                    ws.success_criteria.clone()
                },
                condition: ws.condition.clone(),
            })
            .collect();
        Plan {
            goal: role.purpose.clone(),
            job_type: Some(role.role_category.as_str().into()),
            steps,
            rationale: "deterministic plan from workflow outline".into(),
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

#[async_trait]
pub trait Planner: Send + Sync {
    async fn create_plan(&self, state: &AgentState, context: &str, available_tools: &[&str]) -> Result<Plan>;

    async fn revise_plan(&self, plan: &Plan, state: &AgentState, feedback: &str) -> Result<Plan>;

    async fn research_for_workflow(
        &self,
        state: &AgentState,
        context: &str,
        available_tools: &[&str],
    ) -> Result<AdaptiveResearchMemo>;
}

pub struct LlmPlanner {
    gateway: Arc<dyn LlmGateway>,
    store: Option<Arc<PostgresStore>>,
}

struct RolePlannerContext {
    prompt_context: String,
    job_type: JobType,
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

    /// Load role context from AgentDefinition + AgentRole if the agent's metadata
    /// carries a role_id.  Returns a formatted string injected into the planner
    /// prompt so it knows the scoped connectors, guidelines, and output spec.
    async fn load_role_context(&self, state: &AgentState) -> Option<RolePlannerContext> {
        let store = self.store.as_ref()?;
        let role_id = state.metadata.get("role_id").and_then(|v| v.as_str())?;

        let role = store.get_agent_role(&state.tenant_id, role_id).await.ok()??;
        let job_type = JobType::from_role_category(&role.role_category);
        let workflow_hints = role.execution_guidelines.workflow_hints();
        let preferred_tool_categories = role.execution_guidelines.preferred_tool_categories();
        let preferred_connector_categories = role.execution_guidelines.preferred_connector_categories();

        let mut parts: Vec<String> = Vec::new();
        parts.push(format!("Role category: {}", role.role_category.as_str()));

        if !role.connectors.is_empty() {
            parts.push(format!("Available connectors for this role: {}", role.connectors.join(", ")));
        }
        if let Ok(tenant_wasm_tools) = store.list_tenant_wasm_tools(&state.tenant_id).await {
            let names: Vec<String> =
                tenant_wasm_tools.into_iter().filter(|tool| tool.enabled).map(|tool| tool.name).collect();
            if !names.is_empty() {
                parts.push(format!("Registered tenant WASM tools (strictly sandboxed): {}", names.join(", ")));
            }
        }
        let mut role_tools = Vec::new();
        let mut allowed_wasm_tools = Vec::new();
        for tool_name in &role.tools {
            if let Some(name) = tool_name.strip_prefix("wasm_tool:") {
                if !name.trim().is_empty() {
                    allowed_wasm_tools.push(name.trim().to_string());
                }
            } else {
                role_tools.push(tool_name.clone());
            }
        }
        allowed_wasm_tools.sort();
        allowed_wasm_tools.dedup();
        if !role_tools.is_empty() {
            parts.push(format!("Specific tools for this role: {}", role_tools.join(", ")));
        }
        if !allowed_wasm_tools.is_empty() {
            parts.push(format!("Allowed registered WASM tools for this role: {}", allowed_wasm_tools.join(", ")));
        }
        if !role.execution_guidelines.is_empty() {
            parts.push(format!("Execution guidelines:\n{}", role.execution_guidelines.to_prompt()));
        }
        if !workflow_hints.is_empty() {
            parts.push(format!("Preferred workflow order for this role:\n- {}", workflow_hints.join("\n- ")));
        }
        if !preferred_tool_categories.is_empty() {
            parts.push(format!("Preferred tool categories for this role: {}", preferred_tool_categories.join(", ")));
        }
        if !preferred_connector_categories.is_empty() {
            parts.push(format!(
                "Preferred connector categories for this role: {}",
                preferred_connector_categories.join(", ")
            ));
        }
        if !role.output_spec.description.is_empty() {
            parts.push(format!("Expected output: {}", role.output_spec.description));
        }
        parts.push(format!("Memory scope: {:?}", role.memory_scope).to_lowercase());
        parts.push(format!(
            "Execution limits: max_steps={}, max_retries={}, timeout_secs={}, max_cost_usd={}",
            role.execution_limits.max_steps,
            role.execution_limits.max_retries,
            role.execution_limits.timeout_secs,
            role.execution_limits.max_cost_usd.map(|value| format!("{value:.2}")).unwrap_or_else(|| "none".into())
        ));
        // Load agent constraints
        if let Ok(Some(agent)) = store.get_agent_definition(&state.tenant_id, &role.agent_id).await {
            if !agent.constraints.is_empty() {
                parts.push(format!("Hard constraints (must follow):\n- {}", agent.constraints.join("\n- ")));
            }
            if !agent.persona.is_empty() {
                parts.push(format!("Persona: {}", agent.persona));
            }
        }

        Some(RolePlannerContext { prompt_context: parts.join("\n\n"), job_type })
    }
}

fn clean_json_response(raw: &str) -> &str {
    raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim()
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

        let role_context = self.load_role_context(state).await;
        let job_type =
            role_context.as_ref().map(|ctx| ctx.job_type.clone()).unwrap_or_else(|| JobType::detect(&state.goal));

        let system = PlannerPrompt::system(&job_type);
        let manifest = crate::tools::selector::tool_manifest_from_names(available_tools);
        let conv_history = self.conversation_history(state).await;

        let user = PlannerPrompt::user_create(
            state,
            context,
            &manifest,
            &conv_history,
            role_context.as_ref().map(|ctx| ctx.prompt_context.as_str()),
        );

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
        let cleaned = clean_json_response(&raw);

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
        let job_type =
            self.load_role_context(state).await.map(|ctx| ctx.job_type).unwrap_or_else(|| JobType::detect(&state.goal));
        let system = PlannerPrompt::system(&job_type);

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Medium,
            vec![Message::system(system), Message::user(user)],
        );

        let resp = self.gateway.chat(request).await?;
        let raw = resp.content.unwrap_or_default();
        let cleaned = clean_json_response(&raw);

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

    async fn research_for_workflow(
        &self,
        state: &AgentState,
        context: &str,
        available_tools: &[&str],
    ) -> Result<AdaptiveResearchMemo> {
        let role_context = self.load_role_context(state).await;
        let job_type =
            role_context.as_ref().map(|ctx| ctx.job_type.clone()).unwrap_or_else(|| JobType::detect(&state.goal));
        let conv_history = self.conversation_history(state).await;
        let manifest = crate::tools::selector::tool_manifest_from_names(available_tools);

        let system = format!(
            "{}\n\nYou are in adaptive planning research mode. Study the task and return a JSON synthesis memo only. \
Do not produce executable steps yet. The memo must capture durable findings, assumptions, risks, and workflow hints \
that can later be compiled into a deterministic workflow outline.",
            PlannerPrompt::system(&job_type)
        );
        let user = format!(
            "Goal:\n{}\n\nResearch context:\n{}\n\nAvailable tools:\n{}\n\nConversation history:\n{}\n\nRole context:\n{}\n\n\
Return strict JSON with this shape:\n{{\n  \"summary\": \"...\",\n  \"findings\": [\"...\"],\n  \"assumptions\": [\"...\"],\n  \"risks\": [\"...\"],\n  \"workflow_hints\": [\"...\"]\n}}\n\
Focus on what must be true for a deterministic workflow to succeed. Keep findings concrete and implementation-facing.",
            state.goal,
            context,
            manifest,
            if conv_history.trim().is_empty() { "none" } else { &conv_history },
            role_context.as_ref().map(|ctx| ctx.prompt_context.as_str()).unwrap_or("none"),
        );

        tracing::debug!(agent_id = %state.id, job_type = job_type.label(), "creating adaptive research memo");
        tracing::info!(
            agent_id = %state.id,
            goal = %state.goal,
            job_type = job_type.label(),
            context = %truncate_for_log(context, 500),
            "adaptive research request prepared"
        );

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Medium,
            vec![Message::system(system), Message::user(user)],
        );

        let resp = self.gateway.chat(request).await?;
        let raw = resp.content.unwrap_or_default();
        tracing::info!(
            agent_id = %state.id,
            response = %truncate_for_log(&raw, 1200),
            "adaptive research memo received"
        );

        match serde_json::from_str::<AdaptiveResearchMemo>(clean_json_response(&raw)) {
            Ok(memo) => Ok(memo),
            Err(error) => {
                tracing::warn!(
                    agent_id = %state.id,
                    error = %error,
                    raw = %truncate_for_log(&raw, 200),
                    "adaptive research memo failed to parse, using fallback synthesis"
                );
                Ok(AdaptiveResearchMemo {
                    summary: format!("Adaptive planning memo for goal: {}", state.goal),
                    findings: vec![context.lines().next().unwrap_or_default().trim().to_string()]
                        .into_iter()
                        .filter(|value| !value.is_empty())
                        .collect(),
                    assumptions: vec![],
                    risks: vec!["Research memo fallback was generated from unstructured model output.".into()],
                    workflow_hints: vec!["Compile the memo into bounded deterministic workflow steps before execution.".into()],
                })
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
        }];
        role
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
