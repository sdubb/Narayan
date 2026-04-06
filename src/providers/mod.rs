use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::gateway::llm_controls::LlmGenerationConfig;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), tool_call_id: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), tool_call_id: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_call_id: None }
    }
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self { role: Role::Tool, content: content.into(), tool_call_id: Some(tool_call_id.into()) }
    }
}

// ── Tool spec passed to providers ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
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

#[derive(Debug, Clone, Serialize)]
pub struct ProviderCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub models: &'static [&'static str],
}

// ── Core provider trait ────────────────────────────────────────────────────

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        generation: Option<&LlmGenerationConfig>,
    ) -> Result<ChatResponse>;
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

    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        generation: Option<&LlmGenerationConfig>,
    ) -> Result<ChatResponse> {
        let client = reqwest::Client::new();

        // Convert tools to Anthropic format
        let anthropic_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();

        let generation = generation.cloned().unwrap_or_else(|| {
            crate::gateway::llm_controls::LlmGenerationConfig::new(
                crate::gateway::llm_controls::LlmRole::Drafter,
                crate::gateway::llm_controls::LlmExecutionIntent::Balanced,
                crate::gateway::llm_controls::LlmBudgetTier::High,
            )
            .with_limits(4096, 0.2)
        });

        let mut payload = serde_json::json!({
            "model": self.model,
            "max_tokens": generation.max_tokens,
            "temperature": generation.temperature,
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

        if !anthropic_tools.is_empty() {
            payload["tools"] = serde_json::json!(anthropic_tools);
        }

        tracing::info!(
            "provider request payload provider=anthropic model={} payload={}",
            self.model,
            truncate_for_log(&payload.to_string(), 4000)
        );

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

        tracing::info!(
            "provider response payload provider=anthropic model={} response={}",
            self.model,
            truncate_for_log(&resp.to_string(), 4000)
        );

        // Parse tool calls from Anthropic's tool_use content blocks
        let tool_calls = resp["content"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|content| {
                if content["type"].as_str() == Some("tool_use") {
                    let id = content["id"].as_str()?.to_string();
                    let name = content["name"].as_str()?.to_string();
                    let input = content["input"].clone();
                    Some(ToolCall { id, name, arguments: input })
                } else {
                    None
                }
            })
            .collect();

        // Extract text content, skip tool_use blocks
        let content = resp["content"].as_array().and_then(|arr| {
            arr.iter()
                .find(|item| item["type"].as_str() == Some("text"))
                .and_then(|text_block| text_block["text"].as_str())
                .map(String::from)
        });

        let input_tokens = resp["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = resp["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;

        Ok(ChatResponse { content, tool_calls, input_tokens, output_tokens })
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

    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        generation: Option<&LlmGenerationConfig>,
    ) -> Result<ChatResponse> {
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

        let generation = generation.cloned().unwrap_or_else(|| {
            crate::gateway::llm_controls::LlmGenerationConfig::new(
                crate::gateway::llm_controls::LlmRole::Drafter,
                crate::gateway::llm_controls::LlmExecutionIntent::Balanced,
                crate::gateway::llm_controls::LlmBudgetTier::High,
            )
            .with_limits(4096, 0.2)
        });

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(|m| {
                let mut msg = serde_json::json!({ "role": m.role, "content": m.content });
                if let Some(tool_call_id) = &m.tool_call_id {
                    msg["tool_call_id"] = serde_json::json!(tool_call_id);
                }
                msg
            }).collect::<Vec<_>>(),
            "max_tokens": generation.max_tokens,
            "temperature": generation.temperature,
        });

        if self.model.contains("gpt-oss") {
            payload["include_reasoning"] = serde_json::json!(false);
        }

        if let Some(response_format) = generation.response_format.clone() {
            payload["response_format"] = response_format;
        }

        if !oai_tools.is_empty() {
            payload["tools"] = serde_json::json!(oai_tools);
        }

        tracing::info!(
            "provider request payload provider={} model={} base_url={} payload={}",
            self.name(),
            self.model,
            self.base_url,
            truncate_for_log(&payload.to_string(), 4000)
        );

        let resp = client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?;

        let status = resp.status();
        let resp_text = resp.text().await?;
        if !status.is_success() {
            anyhow::bail!(
                "provider request failed: status={} base_url={} model={} body={}",
                status,
                self.base_url,
                self.model,
                truncate_for_log(&resp_text, 2000)
            );
        }

        let resp: serde_json::Value = serde_json::from_str(&resp_text)?;

        tracing::info!(
            "provider response payload provider={} model={} base_url={} response={}",
            self.name(),
            self.model,
            self.base_url,
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

    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSpec>,
        generation: Option<&LlmGenerationConfig>,
    ) -> Result<ChatResponse> {
        self.inner.chat(messages, tools, generation).await
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
mod groq_impl;
mod novita_impl;
mod openrouter_impl;
mod sglang_impl;

pub use compatible_impl::CompatibleProviderAdapter;
pub use copilot_impl::CopilotProviderAdapter;
pub use gemini_impl::GeminiProviderAdapter;
pub use glm_impl::GlmProviderAdapter;
pub use groq_impl::GroqProviderAdapter;
pub use novita_impl::NovitaProviderAdapter;
pub use openrouter_impl::OpenRouterProviderAdapter;
pub use sglang_impl::SglangProviderAdapter;

pub fn provider_catalog() -> Vec<ProviderCatalogEntry> {
    vec![
        ProviderCatalogEntry {
            id: "anthropic",
            label: "Anthropic",
            models: &["claude-sonnet-4-20250514", "claude-opus-4-20250514", "claude-haiku-4-5-20251001"],
        },
        ProviderCatalogEntry { id: "openai", label: "OpenAI", models: &["gpt-4o", "gpt-4o-mini", "o1", "o3-mini"] },
        ProviderCatalogEntry {
            id: "openrouter",
            label: "OpenRouter",
            models: &[
                "openai/gpt-4o",
                "openai/gpt-4o-mini",
                "anthropic/claude-3.5-sonnet",
                "anthropic/claude-3.5-haiku",
                "meta-llama/llama-3.3-70b-instruct",
                "meta-llama/llama-3.1-8b-instruct",
                "qwen/qwen-2.5-72b-instruct",
                "deepseek/deepseek-chat",
                "google/gemma-2-9b-it",
                "mistralai/mistral-small-3.1",
                "openrouter/free",
            ],
        },
        ProviderCatalogEntry {
            id: "groq",
            label: "Groq",
            models: &["openai/gpt-oss-120b", "llama-3.3-70b-versatile", "llama-3.1-8b-instant", "mixtral-8x7b-32768"],
        },
        ProviderCatalogEntry {
            id: "gemini",
            label: "Gemini",
            models: &["gemini-2.0-flash", "gemini-2.0-pro", "gemini-1.5-pro"],
        },
        ProviderCatalogEntry {
            id: "nvidia",
            label: "NVIDIA",
            models: &[
                "openai/gpt-oss-120b",
                "nvidia/nemotron-3-super-120b-a12b",
                "nvidia/nemotron-3-nano-30b-a3b",
                "meta/llama-3.1-70b-instruct",
                "meta/llama-3.1-8b-instruct",
                "nvidia/llama-3.1-nemotron-70b-instruct",
            ],
        },
        ProviderCatalogEntry { id: "ollama", label: "Ollama", models: &["llama3.3", "qwen2.5-coder", "deepseek-r1"] },
        ProviderCatalogEntry { id: "compatible", label: "Compatible", models: &["custom-model"] },
    ]
}

pub fn supports_provider(name: &str) -> bool {
    provider_catalog().iter().any(|provider| provider.id == name)
}

/// Build a provider instance from a name, api_key, and model string.
/// Used by the BYOK gateway to construct providers from tenant credentials.
pub fn build_provider(name: &str, api_key: String, model: String) -> Option<Arc<dyn Provider>> {
    match name {
        "anthropic" => Some(Arc::new(AnthropicProvider::new(api_key, model))),
        "openai" => Some(Arc::new(OpenAiProvider::new(api_key, model))),
        "groq" => Some(Arc::new(GroqProviderAdapter::new(api_key, model))),
        "ollama" => Some(Arc::new(OllamaProvider::new(api_key, model))),
        "gemini" => Some(Arc::new(GeminiProviderAdapter::new(api_key, model))),
        "nvidia" => {
            Some(Arc::new(OpenAiProvider::new(api_key, model).with_base_url("https://integrate.api.nvidia.com".into())))
        }
        "openrouter" => Some(Arc::new(OpenRouterProviderAdapter::new(api_key, model))),
        "copilot" => Some(Arc::new(CopilotProviderAdapter::new(api_key, model))),
        "glm" => Some(Arc::new(GlmProviderAdapter::new(api_key, model))),
        "novita" => Some(Arc::new(NovitaProviderAdapter::new(api_key, model))),
        "sglang" => Some(Arc::new(SglangProviderAdapter::new(api_key, model))),
        "compatible" => Some(Arc::new(CompatibleProviderAdapter::new(api_key, model))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{provider_catalog, supports_provider};

    #[test]
    fn test_provider_catalog_includes_latest_groq_nvidia_and_openrouter_models() {
        let catalog = provider_catalog();

        let groq = catalog.iter().find(|provider| provider.id == "groq").expect("groq provider should exist");
        assert!(groq.models.contains(&"openai/gpt-oss-120b"));

        let nvidia = catalog.iter().find(|provider| provider.id == "nvidia").expect("nvidia provider should exist");
        assert!(nvidia.models.contains(&"openai/gpt-oss-120b"));
        assert!(nvidia.models.contains(&"nvidia/nemotron-3-super-120b-a12b"));
        assert!(nvidia.models.contains(&"nvidia/nemotron-3-nano-30b-a3b"));

        let openrouter = catalog.iter().find(|provider| provider.id == "openrouter").expect("openrouter provider should exist");
        assert!(openrouter.models.contains(&"openrouter/free"));
        assert!(openrouter.models.contains(&"openai/gpt-4o-mini"));
    }

    #[test]
    fn test_supports_provider_matches_catalog_entries() {
        assert!(supports_provider("groq"));
        assert!(supports_provider("nvidia"));
        assert!(!supports_provider("totally-unknown-provider"));
    }
}
