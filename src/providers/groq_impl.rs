use async_trait::async_trait;

use crate::providers::{ChatResponse, Message, Provider, ToolCall, ToolSpec};

pub struct GroqProviderAdapter {
    api_key: String,
    model: String,
}

impl GroqProviderAdapter {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Provider for GroqProviderAdapter {
    fn name(&self) -> &str {
        "groq"
    }

    async fn chat(&self, messages: Vec<Message>, tools: Vec<ToolSpec>) -> anyhow::Result<ChatResponse> {
        let client = reqwest::Client::new();

        // Build OpenAI-compatible tool format for Groq
        let groq_tools: Vec<serde_json::Value> = tools
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

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(|m| serde_json::json!({ "role": m.role, "content": m.content })).collect::<Vec<_>>(),
        });

        if !groq_tools.is_empty() {
            payload["tools"] = serde_json::json!(groq_tools);
        }

        tracing::info!(
            "provider request payload provider=groq model={} payload={}",
            self.model,
            truncate_for_log(&payload.to_string(), 4000)
        );

        let resp = client
            .post("https://api.groq.com/openai/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?;

        let status = resp.status();
        let resp_text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!(
                "provider request failed: status={} model={} body={}",
                status,
                self.model,
                truncate_for_log(&resp_text, 2000)
            );
        }

        let resp: serde_json::Value = serde_json::from_str(&resp_text)?;

        tracing::info!(
            "provider response payload provider=groq model={} response={}",
            self.model,
            truncate_for_log(&resp.to_string(), 4000)
        );

        let choice = &resp["choices"][0]["message"];
        let content = choice["content"].as_str().map(String::from);

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
