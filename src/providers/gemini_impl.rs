use async_trait::async_trait;

use crate::providers::{ChatResponse, Message, Provider, Role, ToolCall, ToolSpec};

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

    async fn chat(&self, messages: Vec<Message>, tools: Vec<ToolSpec>) -> anyhow::Result<ChatResponse> {
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

        // Add tools in Gemini format (functionDeclarations)
        if !tools.is_empty() {
            let function_decls: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    })
                })
                .collect();
            
            payload["tools"] = serde_json::json!([{
                "functionDeclarations": function_decls
            }]);
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;

        let resp = client.post(&url).json(&payload).send().await?.json::<serde_json::Value>().await?;

        // Extract text content from parts array
        let content = resp["candidates"][0]["content"]["parts"]
            .as_array()
            .and_then(|parts| {
                parts.iter()
                    .find(|p| p.get("text").is_some())
                    .and_then(|text_part| text_part["text"].as_str())
                    .map(String::from)
            });

        // Extract tool calls from functionCalls
        let tool_calls = resp["candidates"][0]["content"]["parts"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|part| {
                let func_call = &part["functionCall"];
                if func_call.is_object() {
                    let name = func_call["name"].as_str()?.to_string();
                    let arguments = func_call["args"].clone();
                    // Generate a unique ID for this tool call
                    let id = format!("gemini-fc-{}", std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos());
                    Some(ToolCall { id, name, arguments })
                } else {
                    None
                }
            })
            .collect();

        // Gemini doesn't provide token count directly in response
        Ok(ChatResponse { content, tool_calls, input_tokens: 0, output_tokens: 0 })
    }
}
