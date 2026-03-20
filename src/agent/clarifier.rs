use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::AgentState,
};

/// Outcome of the clarification check.
#[derive(Debug, Clone)]
pub enum ClarificationResult {
    /// Goal is clear enough — proceed to planning immediately.
    Clear,
    /// Ambiguities found — user must answer before planning.
    NeedsInput { questions: Vec<String> },
}

/// User's answers to clarification questions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationAnswers {
    /// Free-form answers to the clarification questions.
    pub answers: Vec<String>,
    /// Or a single combined response if user prefers.
    pub freeform: Option<String>,
}

#[async_trait]
pub trait Clarifier: Send + Sync {
    async fn check(&self, state: &AgentState) -> Result<ClarificationResult>;

    /// Incorporate user answers into the goal context.
    async fn incorporate(&self, state: &AgentState, answers: &ClarificationAnswers) -> Result<String>;
}

const CLARIFIER_SYSTEM: &str = r#"You are a requirements analyst for an autonomous AI agent.
Given a goal, identify ONLY the ambiguities that would cause the agent to take wrong actions
or produce wrong outputs.

OUTPUT FORMAT — valid JSON only:
{
  "clear": true,
  "questions": []
}

or

{
  "clear": false,
  "questions": [
    "Specific question 1?",
    "Specific question 2?"
  ]
}

STRICT RULES:
1. Ask NO MORE than 3 questions.
2. Ask ONLY if the answer would materially change what the agent does.
3. Do NOT ask about things that are obvious from context or can be inferred.
4. Do NOT ask about implementation details — only about the goal itself.
5. Each question must be answerable in one sentence.
6. If in doubt, mark clear=true and let the agent proceed.

BAD questions (do not ask these):
  "What programming language?" (infer from repo)
  "How detailed should the report be?" (produce good quality by default)

GOOD questions (ask only if genuinely ambiguous):
  "Which of these 3 repositories should be fixed?" (cannot proceed without this)
  "Should the report include historical data or just current pricing?" (changes output significantly)"#;

pub struct LlmClarifier {
    gateway: Arc<dyn LlmGateway>,
}

impl LlmClarifier {
    pub fn new(gateway: Arc<dyn LlmGateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl Clarifier for LlmClarifier {
    async fn check(&self, state: &AgentState) -> Result<ClarificationResult> {
        let user = format!(
            "GOAL: {}\nWORKSPACE: {}\n\nAre there critical ambiguities that must be resolved before starting?",
            state.goal, state.workspace_path,
        );

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Simple,
            vec![Message::system(CLARIFIER_SYSTEM.to_string()), Message::user(user)],
        );

        let resp = self.gateway.chat(request).await?;
        let raw = resp.content.unwrap_or_default();
        let clean = strip_fences(raw.trim());

        #[derive(Deserialize)]
        struct ClarifierResponse {
            clear: bool,
            questions: Vec<String>,
        }

        match serde_json::from_str::<ClarifierResponse>(clean) {
            Ok(r) if r.clear || r.questions.is_empty() => {
                tracing::info!(agent_id = %state.id, "clarifier: goal is clear");
                Ok(ClarificationResult::Clear)
            }
            Ok(r) => {
                tracing::info!(
                    agent_id  = %state.id,
                    questions = ?r.questions,
                    "clarifier: needs input"
                );
                Ok(ClarificationResult::NeedsInput { questions: r.questions })
            }
            Err(e) => {
                tracing::warn!(
                    agent_id = %state.id,
                    error    = %e,
                    "clarifier parse failed, assuming clear"
                );
                Ok(ClarificationResult::Clear)
            }
        }
    }

    async fn incorporate(&self, state: &AgentState, answers: &ClarificationAnswers) -> Result<String> {
        let answers_text =
            if let Some(ref free) = answers.freeform { free.clone() } else { answers.answers.join("\n") };

        let system = "You are a goal refinement assistant. Given an original goal and the user's \
                      clarification answers, produce a single refined goal statement that \
                      incorporates all the answers. Output ONLY the refined goal — no explanation.";

        let user = format!("ORIGINAL GOAL: {}\n\nUSER'S ANSWERS:\n{}\n\nRefined goal:", state.goal, answers_text,);

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Simple,
            vec![Message::system(system.to_string()), Message::user(user)],
        );

        let resp = self.gateway.chat(request).await?;
        Ok(resp.content.unwrap_or_else(|| state.goal.clone()))
    }
}

fn strip_fences(s: &str) -> &str {
    let s = if s.starts_with("```") {
        let after = s.trim_start_matches('`').trim_start_matches("json").trim_start_matches('\n');
        if let Some(end) = after.rfind("```") {
            &after[..end]
        } else {
            after
        }
    } else {
        s
    };
    s.trim()
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

    #[test]
    fn test_strip_fences_removes_markdown_code_block() {
        assert_eq!(strip_fences("```json\n{\"clear\":true}\n```"), "{\"clear\":true}");
    }

    #[tokio::test]
    async fn test_clarifier_returns_clear_for_clear_response() {
        let clarifier = LlmClarifier::new(Arc::new(MockGateway::from_contents(vec![
            r#"{
            "clear": true,
            "questions": []
        }"#,
        ])));

        let result = clarifier.check(&make_state()).await.expect("check should succeed");
        assert!(matches!(result, ClarificationResult::Clear));
    }

    #[tokio::test]
    async fn test_clarifier_returns_questions_when_input_is_needed() {
        let clarifier = LlmClarifier::new(Arc::new(MockGateway::from_contents(vec![
            r#"{
            "clear": false,
            "questions": ["Which repository should be fixed?"]
        }"#,
        ])));

        let result = clarifier.check(&make_state()).await.expect("check should succeed");
        match result {
            ClarificationResult::NeedsInput { questions } => {
                assert_eq!(questions, vec!["Which repository should be fixed?".to_string()]);
            }
            other => panic!("expected clarification questions, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_clarifier_parse_failure_defaults_to_clear() {
        let clarifier = LlmClarifier::new(Arc::new(MockGateway::from_contents(vec!["not json"])));
        let result = clarifier.check(&make_state()).await.expect("check should succeed");
        assert!(matches!(result, ClarificationResult::Clear));
    }

    #[tokio::test]
    async fn test_clarifier_incorporate_uses_freeform_or_answer_list() {
        let clarifier = LlmClarifier::new(Arc::new(MockGateway::from_contents(vec![
            "Refined goal from freeform",
            "Refined goal from answers",
        ])));
        let state = make_state();

        let freeform = clarifier
            .incorporate(&state, &ClarificationAnswers { answers: vec![], freeform: Some("Use repo A".into()) })
            .await
            .expect("freeform incorporation should succeed");
        assert_eq!(freeform, "Refined goal from freeform");

        let answers = clarifier
            .incorporate(&state, &ClarificationAnswers { answers: vec!["Use repo B".into()], freeform: None })
            .await
            .expect("answer-list incorporation should succeed");
        assert_eq!(answers, "Refined goal from answers");
    }
}
