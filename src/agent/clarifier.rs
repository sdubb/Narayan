use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::AgentState,
};

fn default_required() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClarificationQuestion {
    #[serde(default)]
    pub id: String,
    #[serde(default, alias = "type")]
    pub question_type: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default, alias = "helperText")]
    pub helper_text: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default, alias = "multiSelect")]
    pub multi_select: bool,
    #[serde(default)]
    pub recommended: Vec<String>,
    #[serde(default)]
    pub preview: Option<serde_json::Value>,
    #[serde(default = "default_required")]
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
    #[serde(default, alias = "storeAsCredential")]
    pub store_as_credential: Option<String>,
    #[serde(default, alias = "connectorType")]
    pub connector_type: Option<String>,
    #[serde(default, alias = "actionLabel")]
    pub action_label: Option<String>,
    #[serde(default)]
    pub card_type: Option<String>,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default)]
    pub binding_target: Option<String>,
    #[serde(default)]
    pub resume_token: Option<String>,
}

impl ClarificationQuestion {
    pub fn new(prompt: impl Into<String>) -> Self {
        let prompt = prompt.into();
        Self {
            id: question_id_from_prompt(&prompt, 0),
            question_type: Some("clarification".into()),
            prompt,
            placeholder: None,
            helper_text: None,
            options: Vec::new(),
            multi_select: false,
            recommended: Vec::new(),
            preview: None,
            required: true,
            secret: false,
            store_as_credential: None,
            connector_type: None,
            action_label: None,
            card_type: None,
            required_fields: Vec::new(),
            binding_target: None,
            resume_token: None,
        }
    }

    pub fn normalized(mut self, index: usize) -> Self {
        if self.id.trim().is_empty() {
            self.id = question_id_from_prompt(&self.prompt, index);
        }
        self
    }
}

fn question_id_from_prompt(prompt: &str, index: usize) -> String {
    let slug: String = prompt
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("_");
    if slug.is_empty() {
        format!("question_{}", index + 1)
    } else {
        slug
    }
}

pub fn parse_clarification_questions(value: &serde_json::Value) -> Vec<ClarificationQuestion> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, item)| match item {
            serde_json::Value::String(prompt) => Some(ClarificationQuestion::new(prompt.clone()).normalized(index)),
            serde_json::Value::Object(map) => serde_json::from_value::<ClarificationQuestion>(item.clone())
                .ok()
                .map(|question| question.normalized(index))
                .or_else(|| {
                    map.get("question").and_then(|value| value.as_str()).map(|prompt| {
                        ClarificationQuestion {
                            id: map.get("id").and_then(|value| value.as_str()).unwrap_or_default().to_string(),
                            question_type: map
                                .get("question_type")
                                .or_else(|| map.get("type"))
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            prompt: prompt.to_string(),
                            placeholder: map.get("placeholder").and_then(|value| value.as_str()).map(str::to_string),
                            helper_text: map
                                .get("helper_text")
                                .or_else(|| map.get("helperText"))
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            options: map
                                .get("options")
                                .and_then(|value| value.as_array())
                                .map(|items| {
                                    items
                                        .iter()
                                        .filter_map(|value| value.as_str().map(str::to_string))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default(),
                            multi_select: map
                                .get("multi_select")
                                .or_else(|| map.get("multiSelect"))
                                .and_then(|value| value.as_bool())
                                .unwrap_or(false),
                            recommended: map
                                .get("recommended")
                                .and_then(|value| value.as_array())
                                .map(|items| {
                                    items
                                        .iter()
                                        .filter_map(|value| value.as_str().map(str::to_string))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default(),
                            preview: map.get("preview").cloned(),
                            required: map.get("required").and_then(|value| value.as_bool()).unwrap_or(true),
                            secret: map.get("secret").and_then(|value| value.as_bool()).unwrap_or(false),
                            store_as_credential: map
                                .get("store_as_credential")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            connector_type: map
                                .get("connector_type")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            action_label: map.get("action_label").and_then(|value| value.as_str()).map(str::to_string),
                            card_type: map.get("card_type").and_then(|value| value.as_str()).map(str::to_string),
                            required_fields: map
                                .get("required_fields")
                                .and_then(|value| value.as_array())
                                .map(|items| {
                                    items
                                        .iter()
                                        .filter_map(|value| value.as_str().map(str::to_string))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default(),
                            binding_target: map
                                .get("binding_target")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            resume_token: map.get("resume_token").and_then(|value| value.as_str()).map(str::to_string),
                        }
                        .normalized(index)
                    })
                }),
            _ => None,
        })
        .collect()
}

/// Outcome of the clarification check.
#[derive(Debug, Clone)]
pub enum ClarificationResult {
    /// Goal is clear enough — proceed to planning immediately.
    Clear,
    /// Ambiguities found — user must answer before planning.
    NeedsInput { questions: Vec<ClarificationQuestion> },
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
    {
      "id": "database_setup",
      "question_type": "card_open",
      "prompt": "Connect the database before continuing.",
      "card_type": "database",
      "binding_target": "db_main",
      "required_fields": ["host", "port", "db_name"],
      "resume_token": "bind_db_main"
    },
    {
      "id": "trigger_mode",
      "question_type": "mcq",
      "prompt": "How should this workflow start?",
      "options": ["Manual", "Schedule", "Webhook"]
    }
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
            #[serde(default)]
            questions: Vec<serde_json::Value>,
        }

        match serde_json::from_str::<ClarifierResponse>(clean) {
            Ok(r) if r.clear || r.questions.is_empty() => {
                tracing::info!(agent_id = %state.id, "clarifier: goal is clear");
                Ok(ClarificationResult::Clear)
            }
            Ok(r) => {
                let questions = parse_clarification_questions(&serde_json::Value::Array(r.questions));
                tracing::info!(
                    agent_id  = %state.id,
                    questions = ?questions,
                    "clarifier: needs input"
                );
                if questions.is_empty() {
                    Ok(ClarificationResult::Clear)
                } else {
                    Ok(ClarificationResult::NeedsInput { questions })
                }
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
                assert_eq!(questions, vec![ClarificationQuestion::new("Which repository should be fixed?")]);
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
