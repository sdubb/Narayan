use async_trait::async_trait;

use crate::{
    gateway::llm_controls::LlmGenerationConfig,
    providers::{ChatResponse, Message, Provider, Role, ToolCall, ToolSpec},
};

pub struct CompatibleProviderAdapter {
    api_key: String,
    model: String,
}

impl CompatibleProviderAdapter {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Provider for CompatibleProviderAdapter {
    fn name(&self) -> &str {
        "compatible"
    }

    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        generation: Option<&LlmGenerationConfig>,
    ) -> anyhow::Result<ChatResponse> {
        let system = messages.iter().find(|m| matches!(m.role, Role::System)).map(|m| m.content.as_str());
        let history: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(|m| {
                let mut msg = serde_json::json!({"role": format!("{:?}", m.role).to_lowercase(), "content": m.content});
                if let Some(tool_call_id) = &m.tool_call_id {
                    msg["tool_call_id"] = serde_json::json!(tool_call_id);
                }
                msg
            })
            .collect();

        let generation = generation.cloned().unwrap_or_else(|| {
            LlmGenerationConfig::new(
                crate::gateway::llm_controls::LlmRole::Drafter,
                crate::gateway::llm_controls::LlmExecutionIntent::Balanced,
                crate::gateway::llm_controls::LlmBudgetTier::High,
            )
            .with_limits(4096, 0.2)
        });

        let mut payload = serde_json::json!({
            "model":    self.model,
            "messages": history,
            "max_tokens": generation.max_tokens,
            "temperature": generation.temperature,
        });
        if let Some(sys) = system {
            payload["system"] = serde_json::json!(sys);
        }
        if let Some(response_format) = generation.response_format.clone() {
            payload["response_format"] = response_format;
        }

        // Add tools in OpenAI-compatible format for compatible providers
        if !tools.is_empty() {
            let oai_tools: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            payload["tools"] = serde_json::json!(oai_tools);
        }

        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;

        let resp = client
            .post("http://localhost:8000/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Parse responses from compatible providers
        let choice = &resp["choices"][0]["message"];
        let content = choice["content"]
            .as_str()
            .or_else(|| resp["content"][0]["text"].as_str())
            .or_else(|| resp["response"].as_str())
            .map(String::from);

        // Parse tool calls if present
        let tool_calls = choice["tool_calls"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|tc| {
                let id = tc["id"].as_str()?.to_string();
                let name = tc["function"]["name"].as_str()?.to_string();
                let arguments: serde_json::Value =
                    serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}")).unwrap_or_default();
                Some(ToolCall { id, name, arguments })
            })
            .collect();

        let input_tokens = resp["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = resp["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(ChatResponse { content, tool_calls, input_tokens, output_tokens })
    }
}
