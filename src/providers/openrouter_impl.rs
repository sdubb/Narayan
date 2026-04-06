use async_trait::async_trait;

use crate::{
    gateway::llm_controls::LlmGenerationConfig,
    providers::{ChatResponse, Message, Provider, Role, ToolCall, ToolSpec},
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

pub struct OpenRouterProviderAdapter {
    api_key: String,
    model: String,
}

impl OpenRouterProviderAdapter {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Provider for OpenRouterProviderAdapter {
    fn name(&self) -> &str {
        "openrouter"
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

        // OpenRouter accepts OpenAI-compatible tool format and transforms for providers
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
            payload["provider"] = serde_json::json!({
                "require_parameters": true,
                "allow_fallbacks": false,
            });
            if generation.response_format.is_some() {
                tracing::info!(
                    "provider request note provider=openrouter model={} dropping response_format because tools are present",
                    self.model
                );
            }
        } else if let Some(response_format) = generation.response_format.clone() {
            payload["response_format"] = response_format;
            payload["plugins"] = serde_json::json!([
                { "id": "response-healing" }
            ]);
        }

        tracing::info!(
            "provider request payload provider=openrouter model={} payload={}",
            self.model,
            truncate_for_log(&payload.to_string(), 4000)
        );

        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;

        let resp = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        tracing::info!(
            "provider response payload provider=openrouter model={} response={}",
            self.model,
            truncate_for_log(&resp.to_string(), 4000)
        );

        if let Some(error) = resp.get("error") {
            let code = error["code"].as_i64().unwrap_or_default();
            let message = error["message"].as_str().unwrap_or("OpenRouter returned an error");
            let raw = error["metadata"]["raw"].as_str().unwrap_or("");
            let detail = if raw.is_empty() {
                format!("openrouter error code={} message={}", code, message)
            } else {
                format!("openrouter error code={} message={} raw={}", code, message, raw)
            };
            return Err(anyhow::anyhow!(detail));
        }

        // Parse OpenAI-compatible response from OpenRouter
        let choice = &resp["choices"][0]["message"];
        let content = choice["content"]
            .as_str()
            .map(String::from)
            .or_else(|| {
                choice["content"].as_array().and_then(|parts| {
                    let mut out = String::new();
                    for part in parts {
                        if let Some(text) = part["text"].as_str() {
                            out.push_str(text);
                        } else if let Some(text) = part.as_str() {
                            out.push_str(text);
                        }
                    }
                    if out.is_empty() { None } else { Some(out) }
                })
            })
            .or_else(|| choice["parsed"].as_str().map(String::from))
            .or_else(|| {
                if choice["parsed"].is_object() || choice["parsed"].is_array() {
                    Some(choice["parsed"].to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                resp["content"]
                    .as_array()
                    .and_then(|parts| parts.first())
                    .and_then(|first| first["text"].as_str().map(String::from))
            })
            .or_else(|| resp["response"].as_str().map(String::from));

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
