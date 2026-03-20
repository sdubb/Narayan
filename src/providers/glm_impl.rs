use async_trait::async_trait;

use crate::providers::{ChatResponse, Message, Provider, Role, ToolSpec};

pub struct GlmProviderAdapter {
    api_key: String,
    model: String,
}

impl GlmProviderAdapter {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Provider for GlmProviderAdapter {
    fn name(&self) -> &str {
        "glm"
    }

    async fn chat(&self, messages: Vec<Message>, _tools: Vec<ToolSpec>) -> anyhow::Result<ChatResponse> {
        let system = messages.iter().find(|m| matches!(m.role, Role::System)).map(|m| m.content.as_str());
        let history: Vec<serde_json::Value> = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(|m| serde_json::json!({"role": format!("{:?}", m.role).to_lowercase(), "content": m.content}))
            .collect();

        let mut payload = serde_json::json!({
            "model":    self.model,
            "messages": history,
        });
        if let Some(sys) = system {
            payload["system"] = serde_json::json!(sys);
        }

        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;

        let resp = client
            .post("https://open.bigmodel.cn/api/paas/v4/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Try OpenAI-compatible response format
        let content = resp["choices"][0]["message"]["content"]
            .as_str()
            .or_else(|| resp["content"][0]["text"].as_str())
            .or_else(|| resp["response"].as_str())
            .map(String::from);

        Ok(ChatResponse { content, tool_calls: vec![], input_tokens: 0, output_tokens: 0 })
    }
}
