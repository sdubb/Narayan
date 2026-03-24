use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    agent::prompts::PreflightPrompt,
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::AgentState,
};

/// Result of the pre-flight check.
#[derive(Debug, Clone)]
pub enum PreflightResult {
    /// Goal is achievable — proceed to clarification then planning.
    Feasible,
    /// Goal cannot be achieved with available tools.
    Infeasible { reason: String, missing_tools: Vec<String> },
}

#[async_trait]
pub trait Preflight: Send + Sync {
    async fn check(&self, state: &AgentState, available_tools: &[&str]) -> Result<PreflightResult>;
}

// Preflight system prompt is in prompts::PreflightPrompt::system()
// (Legacy PREFLIGHT_SYSTEM prompt removed)

pub struct LlmPreflight {
    gateway: Arc<dyn LlmGateway>,
}

impl LlmPreflight {
    pub fn new(gateway: Arc<dyn LlmGateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl Preflight for LlmPreflight {
    async fn check(&self, state: &AgentState, available_tools: &[&str]) -> Result<PreflightResult> {
        // Build grouped tool manifest instead of flat list
        let manifest = crate::tools::selector::tool_manifest_from_names(available_tools);
        let user = PreflightPrompt::user(&state.goal, &manifest);
        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Simple,
            vec![Message::system(PreflightPrompt::system().to_string()), Message::user(user)],
        );

        let resp = self.gateway.chat(request).await?;
        let raw = resp.content.unwrap_or_default();
        let clean = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();

        #[derive(serde::Deserialize)]
        struct PreflightResponse {
            feasible: bool,
            missing_tools: Vec<String>,
            reason: String,
        }

        match serde_json::from_str::<PreflightResponse>(clean) {
            Ok(r) if r.feasible => {
                tracing::info!(agent_id = %state.id, "pre-flight passed");
                Ok(PreflightResult::Feasible)
            }
            Ok(r) => {
                tracing::warn!(
                    agent_id      = %state.id,
                    reason        = %r.reason,
                    missing_tools = ?r.missing_tools,
                    "pre-flight failed"
                );
                Ok(PreflightResult::Infeasible { reason: r.reason, missing_tools: r.missing_tools })
            }
            Err(e) => {
                // Parse failure → assume feasible, log warning
                tracing::warn!(
                    agent_id = %state.id,
                    error    = %e,
                    "pre-flight parse failed, assuming feasible"
                );
                Ok(PreflightResult::Feasible)
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
            Ok(self.responses.lock().expect("responses lock should succeed").remove(0))
        }
    }

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "fix CI pipeline".into(), "/tmp/ws".into())
    }

    #[tokio::test]
    async fn test_preflight_returns_feasible_for_valid_response() {
        let preflight = LlmPreflight::new(Arc::new(MockGateway::from_contents(vec![
            r#"{
            "feasible": true,
            "missing_tools": [],
            "reason": ""
        }"#,
        ])));

        let result = preflight.check(&make_state(), &["shell", "file_read"]).await.expect("check should succeed");
        assert!(matches!(result, PreflightResult::Feasible));
    }

    #[tokio::test]
    async fn test_preflight_returns_infeasible_with_missing_tools() {
        let preflight = LlmPreflight::new(Arc::new(MockGateway::from_contents(vec![
            r#"{
            "feasible": false,
            "missing_tools": ["browser"],
            "reason": "needs browser automation"
        }"#,
        ])));

        let result = preflight.check(&make_state(), &["shell"]).await.expect("check should succeed");
        match result {
            PreflightResult::Infeasible { reason, missing_tools } => {
                assert_eq!(reason, "needs browser automation");
                assert_eq!(missing_tools, vec!["browser".to_string()]);
            }
            other => panic!("expected infeasible result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_preflight_parse_failure_defaults_to_feasible() {
        let preflight = LlmPreflight::new(Arc::new(MockGateway::from_contents(vec!["not json"])));

        let result = preflight.check(&make_state(), &["shell"]).await.expect("check should succeed");
        assert!(matches!(result, PreflightResult::Feasible));
    }
}
