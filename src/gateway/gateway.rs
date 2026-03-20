use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    gateway::{
        cache::{hash_str, make_cache_key, ResponseCache},
        cost::{CostTracker, SpendCheck},
        limiter::RateLimiter,
        router::TaskComplexity,
    },
    providers::{ChatResponse, Message, Provider, ToolSpec},
    tenant::{
        config::{decrypt_secret, TenantConfig},
        TenantStore,
    },
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

/// A request flowing into the LLM Gateway from an agent.
#[derive(Debug, Clone)]
pub struct GatewayRequest {
    pub agent_id: String,
    pub tenant_id: String,
    pub complexity: TaskComplexity,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub bypass_cache: bool,
}

impl GatewayRequest {
    pub fn new(agent_id: String, tenant_id: String, complexity: TaskComplexity, messages: Vec<Message>) -> Self {
        Self { agent_id, tenant_id, complexity, messages, tools: vec![], bypass_cache: false }
    }

    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }
    pub fn no_cache(mut self) -> Self {
        self.bypass_cache = true;
        self
    }

    fn cache_key(&self) -> String {
        let content: String = self.messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>().join("|");
        make_cache_key(&self.agent_id, &hash_str(&content))
    }
}

/// Core LLM Gateway trait — agents call this, never providers directly.
#[async_trait]
pub trait LlmGateway: Send + Sync {
    async fn chat(&self, request: GatewayRequest) -> Result<ChatResponse>;
}

/// Full BYOK gateway:
/// 1. Loads tenant's own provider credentials from DB (decrypted at request time)
/// 2. Uses tenant's routing config to select provider by complexity
/// 3. Caches, rate-limits, and tracks cost
/// 4. Falls back to platform providers if tenant has not configured their own key
pub struct NarayanGateway {
    tenant_store: Arc<TenantStore>,
    encrypt_key: String,
    cache: Arc<ResponseCache>,
    cost_tracker: Arc<CostTracker>,
    rate_limiter: Arc<RateLimiter>,
    /// Platform-level fallback providers — used when tenant has no key configured.
    /// In pure BYOK deployments this map is empty.
    fallback_providers: HashMap<String, Arc<dyn Provider>>,
}

impl NarayanGateway {
    pub fn new(
        tenant_store: Arc<TenantStore>,
        encrypt_key: String,
        cache: Arc<ResponseCache>,
        cost_tracker: Arc<CostTracker>,
        rate_limiter: Arc<RateLimiter>,
        fallback_providers: HashMap<String, Arc<dyn Provider>>,
    ) -> Self {
        Self { tenant_store, encrypt_key, cache, cost_tracker, rate_limiter, fallback_providers }
    }

    /// Build a live provider from a tenant credential by decrypting the key.
    fn build_provider(&self, config: &TenantConfig, provider: &str) -> Result<Arc<dyn Provider>> {
        let cred = config
            .get_credential(provider)
            .ok_or_else(|| anyhow::anyhow!("no credential for provider '{}'. Add it via PUT /credentials", provider))?;

        if !cred.enabled {
            anyhow::bail!("provider '{}' is disabled for this tenant", provider);
        }

        let api_key = decrypt_secret(&cred.secret_enc, &self.encrypt_key)?;
        let model = cred.model.clone();

        crate::providers::build_provider(provider, api_key, model)
            .ok_or_else(|| anyhow::anyhow!(
                "unknown provider type '{}'. Supported: anthropic, openai, gemini, ollama, openrouter,                  copilot, glm, novita, sglang, compatible",
                provider
            ))
    }

    /// Resolve the correct provider for this tenant + complexity.
    /// Order: tenant key → fallback platform key → error.
    async fn resolve_provider(
        &self,
        tenant_id: &str,
        complexity: &TaskComplexity,
    ) -> Result<(Arc<dyn Provider>, String)> {
        // Load tenant config
        let config = self.tenant_store.get_config(tenant_id).await?;

        // Pick preferred provider name from tenant's routing config
        let preferred = match complexity {
            TaskComplexity::Simple => &config.routing.simple,
            TaskComplexity::Medium => &config.routing.medium,
            TaskComplexity::Complex => &config.routing.complex,
        }
        .clone();

        // Try tenant's own credential for preferred provider
        if let Ok(p) = self.build_provider(&config, &preferred) {
            tracing::debug!(tenant_id, provider = %preferred, "using tenant BYOK credential");
            return Ok((p, preferred));
        }

        // Try tenant's fallback provider
        if let Ok(p) = self.build_provider(&config, &config.routing.fallback) {
            let fb = config.routing.fallback.clone();
            tracing::debug!(tenant_id, provider = %fb, "using tenant fallback credential");
            return Ok((p, fb));
        }

        // Try any enabled tenant credential
        let enabled: Vec<_> = config.credentials.values().filter(|c| c.enabled).collect();

        if let Some(cred) = enabled.first() {
            if let Ok(p) = self.build_provider(&config, &cred.provider) {
                let name = cred.provider.clone();
                tracing::debug!(tenant_id, provider = %name, "using first available tenant credential");
                return Ok((p, name));
            }
        }

        // Fall back to platform-level providers (dev/testing only)
        if let Some(p) = self.fallback_providers.get(&preferred) {
            tracing::warn!(
                tenant_id,
                provider = %preferred,
                "tenant has no credentials — using platform fallback key"
            );
            return Ok((p.clone(), preferred));
        }

        if let Some((name, p)) = self.fallback_providers.iter().next() {
            tracing::warn!(
                tenant_id,
                provider = %name,
                "tenant has no credentials — using first platform fallback key"
            );
            return Ok((p.clone(), name.clone()));
        }

        anyhow::bail!(
            "tenant '{}' has no provider credentials configured and no platform fallback is available. \
             Call PUT /credentials to add a provider API key.",
            tenant_id
        )
    }
}

#[async_trait]
impl LlmGateway for NarayanGateway {
    async fn chat(&self, request: GatewayRequest) -> Result<ChatResponse> {
        // 1. Cache check
        if !request.bypass_cache {
            if let Some(cached) = self.cache.get(&request.cache_key()).await {
                tracing::debug!(
                    agent_id  = %request.agent_id,
                    tenant_id = %request.tenant_id,
                    "gateway cache hit"
                );
                return Ok(cached);
            }
        }

        // 2. Spend-limit pre-check — block before calling provider
        let tenant = self.tenant_store.get_by_id(&request.tenant_id).await?;
        if let Some(ref t) = tenant {
            let limit = t.plan.spend_limit_usd();
            match self.cost_tracker.check_spend_limit(&request.tenant_id, limit).await {
                SpendCheck::Exceeded { limit_usd, current_usd } => {
                    tracing::warn!(
                        tenant_id   = %request.tenant_id,
                        limit_usd,
                        current_usd,
                        "spend limit exceeded — blocking LLM request"
                    );
                    anyhow::bail!(
                        "tenant '{}' has exceeded spend limit (${:.2} of ${:.2}). \
                         Upgrade your plan or wait for the next billing period.",
                        request.tenant_id, current_usd, limit_usd
                    );
                }
                SpendCheck::Warning { limit_usd, current_usd, pct_used } => {
                    tracing::warn!(
                        tenant_id   = %request.tenant_id,
                        limit_usd,
                        current_usd,
                        pct_used,
                        "tenant approaching spend limit"
                    );
                }
                SpendCheck::Ok => {}
            }
        }

        // 3. Resolve provider from tenant's own credentials
        let (provider, provider_name) = self.resolve_provider(&request.tenant_id, &request.complexity).await?;

        tracing::info!(
            agent_id   = %request.agent_id,
            tenant_id  = %request.tenant_id,
            provider   = %provider_name,
            complexity = ?request.complexity,
            "gateway routing request"
        );

        let message_preview: Vec<String> = request
            .messages
            .iter()
            .enumerate()
            .map(|(idx, msg)| {
                format!(
                    "#{idx} role={:?} content={}",
                    msg.role,
                    truncate_for_log(&msg.content, 800)
                )
            })
            .collect();
        let tool_names: Vec<&str> = request.tools.iter().map(|tool| tool.name.as_str()).collect();

        tracing::info!(
            "gateway request payload agent_id={} tenant_id={} provider={} complexity={:?} message_count={} tool_count={} messages={:?} tools={:?}",
            request.agent_id,
            request.tenant_id,
            provider_name,
            request.complexity,
            request.messages.len(),
            request.tools.len(),
            message_preview,
            tool_names
        );

        // 4. Rate limit (per provider name, shared across tenant)
        self.rate_limiter.acquire(&provider_name).await;

        // 5. Call provider
        let response = provider.chat(request.messages.clone(), request.tools.clone()).await.map_err(|e| {
            tracing::error!(
                tenant_id = %request.tenant_id,
                provider  = %provider_name,
                error     = %e,
                "provider call failed"
            );
            e
        })?;

        // 6. Track cost against tenant + agent
        self.cost_tracker
            .record(&request.tenant_id, &request.agent_id, &provider_name, response.input_tokens, response.output_tokens)
            .await;

        tracing::debug!(
            agent_id       = %request.agent_id,
            tenant_id      = %request.tenant_id,
            input_tokens   = response.input_tokens,
            output_tokens  = response.output_tokens,
            "gateway call complete"
        );

        let tool_summaries: Vec<String> = response
            .tool_calls
            .iter()
            .map(|tool| {
                format!(
                    "{} {}",
                    tool.name,
                    truncate_for_log(&tool.arguments.to_string(), 600)
                )
            })
            .collect();

        tracing::info!(
            "gateway response payload agent_id={} tenant_id={} provider={} input_tokens={} output_tokens={} content={:?} tool_calls={:?}",
            request.agent_id,
            request.tenant_id,
            provider_name,
            response.input_tokens,
            response.output_tokens,
            response.content.as_deref().map(|text| truncate_for_log(text, 1200)),
            tool_summaries
        );

        // 7. Cache response
        if !request.bypass_cache {
            self.cache.set(request.cache_key(), response.clone()).await;
        }

        Ok(response)
    }
}
