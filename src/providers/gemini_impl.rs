use async_trait::async_trait;

use crate::providers::{ChatResponse, Message, Provider, Role, ToolSpec};

pub struct GeminiProviderAdapter {
    api_key: String,
    model: String,
}

impl GeminiProviderAdapter {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Provider for GeminiProviderAdapter {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn chat(&self, messages: Vec<Message>, _tools: Vec<ToolSpec>) -> anyhow::Result<ChatResponse> {
        let system = messages.iter().find(|m| matches!(m.role, Role::System)).map(|m| m.content.clone());

        let mut contents: Vec<serde_json::Value> = Vec::new();
        for msg in &messages {
            if matches!(msg.role, Role::System) {
                continue;
            }
            let role = if matches!(msg.role, Role::User) { "user" } else { "model" };
            contents.push(serde_json::json!({
                "role": role,
                "parts": [{"text": msg.content}]
            }));
        }

        let mut payload = serde_json::json!({"contents": contents});
        if let Some(sys) = system {
            payload["systemInstruction"] = serde_json::json!({
                "parts": [{"text": sys}]
            });
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;

        let resp = client.post(&url).json(&payload).send().await?.json::<serde_json::Value>().await?;

        let content = resp["candidates"][0]["content"]["parts"][0]["text"].as_str().map(String::from);

        Ok(ChatResponse { content, tool_calls: vec![], input_tokens: 0, output_tokens: 0 })
    }
}
