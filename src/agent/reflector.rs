use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    agent::{
        executor::StepResult,
        planner::{Plan, Planner},
        prompts::ReflectorPrompt,
    },
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::AgentState,
};

#[derive(Debug, Clone)]
pub struct Reflection {
    pub summary: String,
    pub key_findings: Vec<String>, // ← concrete facts discovered this step
    pub revised_plan: Option<Plan>,
}

#[async_trait]
pub trait Reflector: Send + Sync {
    async fn reflect(&self, state: &AgentState, plan: &Plan, result: &StepResult) -> Result<Reflection>;

    /// Revise the remaining plan based on feedback from evaluate_and_reflect.
    async fn revise_plan(&self, plan: &Plan, state: &AgentState, feedback: &str) -> Result<Plan>;
}

pub struct LlmReflector {
    gateway: Arc<dyn LlmGateway>,
    planner: Arc<dyn Planner>,
}

impl LlmReflector {
    pub fn new(gateway: Arc<dyn LlmGateway>, planner: Arc<dyn Planner>) -> Self {
        Self { gateway, planner }
    }
}

#[async_trait]
impl Reflector for LlmReflector {
    async fn reflect(&self, state: &AgentState, plan: &Plan, result: &StepResult) -> Result<Reflection> {
        if result.success && plan.is_complete(state.current_step as usize + 1) {
            return Ok(Reflection { summary: "goal complete".into(), key_findings: vec![], revised_plan: None });
        }

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Simple,
            vec![
                Message::system(ReflectorPrompt::system().to_string()),
                Message::user(ReflectorPrompt::user(state, plan, result)),
            ],
        );

        let resp = self.gateway.chat(request).await?;
        let raw = resp.content.unwrap_or_default();
        let cleaned = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();

        let parsed: serde_json::Value = serde_json::from_str(cleaned).unwrap_or_else(|_| {
            serde_json::json!({
                "summary":      &raw[..raw.len().min(140)],
                "key_findings": [],
                "revise":       false,
                "feedback":     ""
            })
        });

        let summary = parsed["summary"].as_str().unwrap_or("step processed").to_string();
        let key_findings: Vec<String> = parsed["key_findings"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let should_revise = parsed["revise"].as_bool().unwrap_or(false);
        let feedback = parsed["feedback"].as_str().unwrap_or("").to_string();

        tracing::debug!(
            agent_id      = %state.id,
            step          = result.step_index,
            summary       = %summary,
            findings      = key_findings.len(),
            should_revise,
            "reflection complete"
        );

        let revised_plan = if should_revise && !feedback.is_empty() {
            match self.planner.revise_plan(plan, state, &feedback).await {
                Ok(p) => {
                    tracing::info!(agent_id = %state.id, "plan revised");
                    Some(p)
                }
                Err(e) => {
                    tracing::warn!(agent_id = %state.id, error = %e, "plan revision failed");
                    None
                }
            }
        } else {
            None
        };

        Ok(Reflection { summary, key_findings, revised_plan })
    }

    async fn revise_plan(&self, plan: &Plan, state: &AgentState, feedback: &str) -> Result<Plan> {
        self.planner.revise_plan(plan, state, feedback).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::{
        agent::{planner::PlannedStep, test_helpers::MockPlanner},
        providers::ChatResponse,
    };

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
            Ok(self.responses.lock().expect("responses lock should succeed").remove(0))
        }
    }

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "fix CI pipeline".into(), "/tmp/ws".into())
    }

    fn make_plan() -> Plan {
        Plan {
            goal: "fix CI pipeline".into(),
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
        }
    }

    fn make_result(success: bool, output: &str) -> StepResult {
        StepResult {
            step_index: 0,
            success,
            output: output.into(),
            final_answer_candidate: Some(output.into()),
            tool_results: vec![],
            tools_called: vec![],
            items_processed: 0,
            connector_writes: vec![],
        }
    }

    #[tokio::test]
    async fn test_reflect_short_circuits_to_goal_complete_for_final_successful_step() {
        let mut state = make_state();
        state.current_step = 0;
        let plan = make_plan();
        let reflector = LlmReflector::new(Arc::new(MockGateway::from_contents(vec![])), Arc::new(MockPlanner::new()));

        let reflection = reflector
            .reflect(&state, &plan, &make_result(true, "STEP COMPLETE"))
            .await
            .expect("reflection should succeed");

        assert_eq!(reflection.summary, "goal complete");
        assert!(reflection.key_findings.is_empty());
        assert!(reflection.revised_plan.is_none());
    }

    #[tokio::test]
    async fn test_reflect_parses_json_and_requests_plan_revision_when_needed() {
        let revised = Plan {
            goal: "fix CI pipeline".into(),
            job_type: Some("software_engineer".into()),
            steps: vec![
                PlannedStep {
                    index: 0,
                    description: "Inspect failing workflow".into(),
                    tool: Some("file_read".into()),
                    tool_args: None,
                    success_criteria: "workflow reviewed".into(),
                    condition: None,
                },
                PlannedStep {
                    index: 1,
                    description: "Patch the workflow".into(),
                    tool: Some("file_edit".into()),
                    tool_args: None,
                    success_criteria: "workflow fixed".into(),
                    condition: None,
                },
            ],
            rationale: "adapt remaining work".into(),
        };
        let reflector = LlmReflector::new(
            Arc::new(MockGateway::from_contents(vec![
                r#"{
                "summary":"workflow path was wrong",
                "key_findings":["ci file moved to another directory"],
                "revise":true,
                "feedback":"update the remaining plan to target the new workflow path"
            }"#,
            ])),
            Arc::new(MockPlanner::from_revise_responses(vec![revised.clone()])),
        );
        let state = make_state();
        let plan = Plan {
            goal: "fix CI pipeline".into(),
            job_type: Some("software_engineer".into()),
            steps: vec![
                PlannedStep {
                    index: 0,
                    description: "Inspect failing workflow".into(),
                    tool: Some("file_read".into()),
                    tool_args: None,
                    success_criteria: "workflow reviewed".into(),
                    condition: None,
                },
                PlannedStep {
                    index: 1,
                    description: "Patch old workflow path".into(),
                    tool: Some("file_edit".into()),
                    tool_args: None,
                    success_criteria: "workflow fixed".into(),
                    condition: None,
                },
            ],
            rationale: "inspect then patch".into(),
        };

        let reflection = reflector
            .reflect(&state, &plan, &make_result(false, "STEP FAILED: wrong file path"))
            .await
            .expect("reflection should succeed");

        assert_eq!(reflection.summary, "workflow path was wrong");
        assert_eq!(reflection.key_findings, vec!["ci file moved to another directory"]);
        assert!(reflection.revised_plan.is_some());
        assert_eq!(reflection.revised_plan.expect("revised plan expected").steps[1].description, "Patch the workflow");
    }

    #[tokio::test]
    async fn test_reflect_falls_back_to_raw_output_summary_on_invalid_json() {
        let reflector = LlmReflector::new(
            Arc::new(MockGateway::from_contents(vec!["plain text reflection without json"])),
            Arc::new(MockPlanner::new()),
        );
        let mut state = make_state();
        state.current_step = 1;
        let plan = Plan {
            goal: "fix CI pipeline".into(),
            job_type: Some("software_engineer".into()),
            steps: vec![
                PlannedStep {
                    index: 0,
                    description: "Inspect failing workflow".into(),
                    tool: Some("file_read".into()),
                    tool_args: None,
                    success_criteria: "workflow reviewed".into(),
                    condition: None,
                },
                PlannedStep {
                    index: 1,
                    description: "Patch workflow".into(),
                    tool: Some("file_edit".into()),
                    tool_args: None,
                    success_criteria: "workflow fixed".into(),
                    condition: None,
                },
                PlannedStep {
                    index: 2,
                    description: "Run tests".into(),
                    tool: Some("shell".into()),
                    tool_args: None,
                    success_criteria: "tests green".into(),
                    condition: None,
                },
            ],
            rationale: "full repair flow".into(),
        };

        let reflection = reflector
            .reflect(&state, &plan, &make_result(false, "STEP FAILED: syntax error"))
            .await
            .expect("reflection should succeed");

        assert!(reflection.summary.contains("plain text reflection without json"));
        assert!(reflection.key_findings.is_empty());
        assert!(reflection.revised_plan.is_none());
    }
}
