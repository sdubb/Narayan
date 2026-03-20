use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Message types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

// ── Tool spec passed to providers ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ── Tool call returned by provider ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

// ── Provider response ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ── Core provider trait ────────────────────────────────────────────────────

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, messages: Vec<Message>, tools: Vec<ToolSpec>) -> Result<ChatResponse>;
}

// ── Anthropic provider ─────────────────────────────────────────────────────

pub struct AnthropicProvider {
    pub api_key: String,
    pub model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, messages: Vec<Message>, _tools: Vec<ToolSpec>) -> Result<ChatResponse> {
        let client = reqwest::Client::new();

        let payload = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": messages
                .iter()
                .filter(|m| !matches!(m.role, Role::System))
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect::<Vec<_>>(),
            "system": messages
                .iter()
                .find(|m| matches!(m.role, Role::System))
                .map(|m| m.content.clone())
                .unwrap_or_default(),
        });

        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2025-01-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let content = resp["content"][0]["text"].as_str().map(String::from);

        let input_tokens = resp["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = resp["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(ChatResponse { content, tool_calls: vec![], input_tokens, output_tokens })
    }
}

// ── OpenAI provider ────────────────────────────────────────────────────────

pub struct OpenAiProvider {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model, base_url: "https://api.openai.com".into() }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn chat(&self, messages: Vec<Message>, tools: Vec<ToolSpec>) -> Result<ChatResponse> {
        let client = reqwest::Client::new();

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

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(|m| serde_json::json!({ "role": m.role, "content": m.content })).collect::<Vec<_>>(),
        });

        if !oai_tools.is_empty() {
            payload["tools"] = serde_json::json!(oai_tools);
        }

        let resp = client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

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

// ── Ollama provider (compatible with OpenAI API) ───────────────────────────

pub struct OllamaProvider {
    inner: OpenAiProvider,
}

impl OllamaProvider {
    pub fn new(base_url: String, model: String) -> Self {
        Self { inner: OpenAiProvider::new(String::new(), model).with_base_url(base_url) }
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn chat(&self, messages: Vec<Message>, tools: Vec<ToolSpec>) -> Result<ChatResponse> {
        self.inner.chat(messages, tools).await
    }
}

// ── Provider registry ──────────────────────────────────────────────────────

pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
    default: String,
}

impl ProviderRegistry {
    pub fn new(default: impl Into<String>) -> Self {
        Self { providers: HashMap::new(), default: default.into() }
    }

    pub fn register(&mut self, provider: Arc<dyn Provider>) {
        self.providers.insert(provider.name().to_string(), provider);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(name).cloned()
    }

    pub fn default_provider(&self) -> Option<Arc<dyn Provider>> {
        self.providers.get(&self.default).cloned()
    }
}

// ── New provider implementations (from uploaded project) ───────────────────
// Each is wrapped with an adapter implementing our Provider trait.
// Their internal chat_with_system(system, message, model, temp) -> String
// is bridged to our chat(messages, tools) -> ChatResponse.

mod compatible_impl;
mod copilot_impl;
mod gemini_impl;
mod glm_impl;
mod novita_impl;
mod openrouter_impl;
mod sglang_impl;

pub use compatible_impl::CompatibleProviderAdapter;
pub use copilot_impl::CopilotProviderAdapter;
pub use gemini_impl::GeminiProviderAdapter;
pub use glm_impl::GlmProviderAdapter;
pub use novita_impl::NovitaProviderAdapter;
pub use openrouter_impl::OpenRouterProviderAdapter;
pub use sglang_impl::SglangProviderAdapter;

/// Build a provider instance from a name, api_key, and model string.
/// Used by the BYOK gateway to construct providers from tenant credentials.
pub fn build_provider(name: &str, api_key: String, model: String) -> Option<Arc<dyn Provider>> {
    match name {
        "anthropic" => Some(Arc::new(AnthropicProvider::new(api_key, model))),
        "openai" => Some(Arc::new(OpenAiProvider::new(api_key, model))),
        "ollama" => Some(Arc::new(OllamaProvider::new(api_key, model))),
        "gemini" => Some(Arc::new(GeminiProviderAdapter::new(api_key, model))),
        "openrouter" => Some(Arc::new(OpenRouterProviderAdapter::new(api_key, model))),
        "copilot" => Some(Arc::new(CopilotProviderAdapter::new(api_key, model))),
        "glm" => Some(Arc::new(GlmProviderAdapter::new(api_key, model))),
        "novita" => Some(Arc::new(NovitaProviderAdapter::new(api_key, model))),
        "sglang" => Some(Arc::new(SglangProviderAdapter::new(api_key, model))),
        "compatible" => Some(Arc::new(CompatibleProviderAdapter::new(api_key, model))),
        _ => None,
    }
}
