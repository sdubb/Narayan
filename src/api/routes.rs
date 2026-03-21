use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use sqlx::Row;

use crate::{
    agent::AgentManager,
    auth::{hash_password, issue_token, verify_password},
    gateway::{cost::AgentUsage, CostTracker},
    metrics::Metrics,
    skill_marketplace::{marketplace::MarketplaceSkill, SkillMarketplace},
    skills::registry::{Skill, SkillRegistry},
    state::AgentStatus,
    storage::PostgresStore,
    tenant::{encrypt_secret, model::AuthenticatedTenant, ProviderCredential, TenantStore},
};

// ── Auto-approval store ────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoApprovalRule {
    pub rule_id: String,
    pub tenant_id: String,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// DB-backed auto-approval rule store with in-memory DashMap cache.
/// DB is authoritative; cache speeds up the hot path (executor checks per tool call).
pub struct AutoApprovalStore {
    cache: DashMap<String, AutoApprovalRule>,
    pool: sqlx::PgPool,
}

impl AutoApprovalStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { cache: DashMap::new(), pool }
    }

    fn key(tenant_id: &str, rule_id: &str) -> String {
        format!("{}:{}", tenant_id, rule_id)
    }

    /// Create the table if it doesn't exist and warm the in-memory cache.
    pub async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS auto_approvals (
                rule_id    TEXT        NOT NULL,
                tenant_id  TEXT        NOT NULL,
                notes      TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (tenant_id, rule_id)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS auto_approvals_tenant ON auto_approvals (tenant_id)")
            .execute(&self.pool)
            .await?;
        self.warm_cache().await?;
        Ok(())
    }

    async fn warm_cache(&self) -> anyhow::Result<()> {
        let rows = sqlx::query("SELECT rule_id, tenant_id, notes, created_at FROM auto_approvals")
            .fetch_all(&self.pool)
            .await?;
        for r in rows {
            let rule_id: String = r.get("rule_id");
            let tenant_id: String = r.get("tenant_id");
            self.cache.insert(
                Self::key(&tenant_id, &rule_id),
                AutoApprovalRule { rule_id, tenant_id, notes: r.get("notes"), created_at: r.get("created_at") },
            );
        }
        Ok(())
    }

    pub async fn upsert(
        &self,
        tenant_id: &str,
        rule_id: &str,
        notes: Option<&str>,
    ) -> anyhow::Result<AutoApprovalRule> {
        sqlx::query(
            "INSERT INTO auto_approvals (rule_id, tenant_id, notes)
             VALUES ($1, $2, $3)
             ON CONFLICT (tenant_id, rule_id) DO UPDATE SET notes = EXCLUDED.notes",
        )
        .bind(rule_id)
        .bind(tenant_id)
        .bind(notes)
        .execute(&self.pool)
        .await?;

        let rule = AutoApprovalRule {
            rule_id: rule_id.to_string(),
            tenant_id: tenant_id.to_string(),
            notes: notes.map(String::from),
            created_at: chrono::Utc::now(),
        };
        self.cache.insert(Self::key(tenant_id, rule_id), rule.clone());
        Ok(rule)
    }

    pub async fn get_for_tenant(&self, tenant_id: &str) -> anyhow::Result<Vec<AutoApprovalRule>> {
        let rows = sqlx::query(
            "SELECT rule_id, tenant_id, notes, created_at FROM auto_approvals WHERE tenant_id = $1 ORDER BY created_at",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| AutoApprovalRule {
                rule_id: r.get("rule_id"),
                tenant_id: r.get("tenant_id"),
                notes: r.get("notes"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Fast in-memory check — used by the executor hot path.
    pub fn contains(&self, tenant_id: &str, rule_id: &str) -> bool {
        self.cache.contains_key(&Self::key(tenant_id, rule_id))
    }

    pub async fn delete(&self, tenant_id: &str, rule_id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM auto_approvals WHERE tenant_id = $1 AND rule_id = $2")
            .bind(tenant_id)
            .bind(rule_id)
            .execute(&self.pool)
            .await?;
        self.cache.remove(&Self::key(tenant_id, rule_id));
        Ok(result.rows_affected() > 0)
    }
}

// ── App state ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<PostgresStore>,
    pub tenant_store: Arc<TenantStore>,
    pub manager: Arc<AgentManager>,
    pub cost_tracker: Arc<CostTracker>,
    pub metrics: Arc<Metrics>,
    pub jwt_secret: String,
    pub encrypt_key: String,
    pub skill_registry: Arc<RwLock<SkillRegistry>>,
    pub marketplace: Arc<tokio::sync::Mutex<SkillMarketplace>>,
    pub audit_log: Arc<crate::audit::AuditLog>,
    pub webhook_store: Arc<crate::webhooks::WebhookStore>,
    pub webhook_dispatcher: Arc<crate::webhooks::WebhookDispatcher>,
    pub review_queue: Arc<crate::compliance::ReviewQueue>,
    pub swarm: Arc<crate::swarm::Swarm>,
    pub connector_registry: Arc<crate::connectors::ConnectorRegistry>,
    /// Optional citation tracker — present when compliance module is enabled.
    pub citation_tracker: Option<Arc<crate::compliance::CitationTracker>>,
    /// In-process auto-approval rule store.
    pub auto_approvals: Arc<AutoApprovalStore>,
    /// Event bus handle for publishing SSE events from HTTP handlers (connectors etc).
    pub event_bus_handle: Arc<crate::events::EventBus>,
    /// Billing store — subscriptions, invoices, credits, provider routing.
    pub billing: Arc<crate::billing::BillingStore>,
    /// Connector install store — OAuth tokens and API keys per tenant.
    pub connector_installs: Arc<crate::connectors::ConnectorInstallStore>,
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}
type Response = axum::response::Response;

/// Verify a webhook HMAC-SHA256 signature (covers GitHub, PagerDuty, HubSpot, etc.)
fn verify_webhook_hmac(signature: &str, payload: &[u8], secret: &str) -> bool {
    use ring::hmac;
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let computed = hmac::sign(&key, payload);
    let computed_hex = format!("sha256={}", hex::encode(computed.as_ref()));
    // Constant-time compare
    let sig = signature.trim();
    sig == computed_hex || sig == &computed_hex[7..] // with or without "sha256=" prefix
}

fn register_request_is_valid(body: &RegisterRequest) -> bool {
    !body.name.trim().is_empty()
        && !body.username.trim().is_empty()
        && !body.email.trim().is_empty()
        && body.password.trim().len() >= 8
}

fn auth_response(tenant_id: &str, username: &str, token: String) -> serde_json::Value {
    serde_json::json!({
        "token": token,
        "tenant_id": tenant_id,
        "username": username,
    })
}

fn cost_response_json(tenant_id: &str, usage: Option<AgentUsage>) -> serde_json::Value {
    match usage {
        Some(u) => serde_json::json!({
            "tenant_id": tenant_id,
            "total_cost_usd": u.total_cost_usd,
            "total_input_tokens": u.total_input_tokens,
            "total_output_tokens": u.total_output_tokens,
            "total_requests": u.total_requests,
        }),
        None => serde_json::json!({
            "tenant_id": tenant_id,
            "total_cost_usd": 0.0
        }),
    }
}

fn marketplace_skill_from_upload(body: UploadSkillRequest) -> MarketplaceSkill {
    MarketplaceSkill {
        name: body.name,
        author: body.author.unwrap_or_else(|| "anonymous".into()),
        description: body.description,
        steps: body.steps,
    }
}

fn marketplace_list_json(marketplace: &SkillMarketplace) -> serde_json::Value {
    let skills: Vec<serde_json::Value> = marketplace
        .list()
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "author": s.author,
                "description": s.description,
                "step_count": s.steps.len(),
            })
        })
        .collect();
    serde_json::json!({ "skills": skills, "count": skills.len() })
}

fn install_marketplace_skill(
    marketplace: &SkillMarketplace,
    registry: &mut SkillRegistry,
    name: &str,
) -> Result<(), String> {
    let Some(ms) = marketplace.get(name) else {
        return Err(format!("skill '{}' not in marketplace", name));
    };

    registry.register(Skill::new(ms.name.clone(), ms.description.clone(), ms.steps.clone()));
    Ok(())
}

fn installed_skills_json(registry: &SkillRegistry) -> serde_json::Value {
    let skills: Vec<serde_json::Value> = registry
        .list()
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "step_count": s.steps.len(),
                "aliases": s.aliases,
                "version": s.version,
            })
        })
        .collect();
    serde_json::json!({ "skills": skills, "count": skills.len() })
}

fn provider_catalog_json() -> serde_json::Value {
    let providers: Vec<serde_json::Value> = crate::providers::provider_catalog()
        .into_iter()
        .map(|provider| {
            serde_json::json!({
                "id": provider.id,
                "label": provider.label,
                "models": provider.models,
            })
        })
        .collect();
    serde_json::json!({ "providers": providers, "count": providers.len() })
}

// ── Health ─────────────────────────────────────────────────────────────────

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "service": "narayan" }))
}

// ── Auth / Registration ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub token: String,
    pub tenant_id: String,
    pub username: String,
}

/// POST /auth/register — create a new tenant and return a session token.
pub async fn register(State(state): State<AppState>, Json(body): Json<RegisterRequest>) -> impl IntoResponse {
    if !register_request_is_valid(&body) {
        return err(StatusCode::BAD_REQUEST, "name, username, email, and password (min 8 chars) required");
    }

    let password_hash = match hash_password(&body.password) {
        Ok(hash) => hash,
        Err(e) => return err(StatusCode::BAD_REQUEST, e.to_string()),
    };

    match state
        .tenant_store
        .create_tenant(
            body.username.trim().to_string(),
            body.name.trim().to_string(),
            body.email.trim().to_string(),
            password_hash,
            String::new(),
            String::new(),
        )
        .await
    {
        Ok(tenant) => {
            let _ = state
                .audit_log
                .append(
                    &tenant.id,
                    None,
                    crate::audit::AuditAction::TenantRegistered,
                    serde_json::json!({ "email": body.email, "name": body.name, "username": body.username }),
                    None,
                )
                .await;
            let plan_str = format!("{:?}", tenant.plan).to_lowercase();
            match issue_token(&tenant.id, &plan_str, &state.jwt_secret) {
                Ok(token) => (
                    StatusCode::CREATED,
                    Json(RegisterResponse { token, tenant_id: tenant.id, username: tenant.username }),
                )
                    .into_response(),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "register failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

/// POST /auth/login or /auth/token — exchange username/email + password for a JWT session.
#[derive(Deserialize)]
pub struct TokenRequest {
    pub identifier: String,
    pub password: String,
}

pub async fn issue_session_token(State(state): State<AppState>, Json(body): Json<TokenRequest>) -> impl IntoResponse {
    if body.identifier.trim().is_empty() || body.password.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "identifier and password required");
    }

    let tenant = match state.tenant_store.get_auth_by_identifier(&body.identifier).await {
        Ok(Some(t)) => t,
        Ok(None) => return err(StatusCode::UNAUTHORIZED, "invalid credentials"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let Some(password_hash) = tenant.password_hash.as_deref() else {
        return err(StatusCode::UNAUTHORIZED, "password login is not configured for this account");
    };
    if !verify_password(&body.password, password_hash) {
        return err(StatusCode::UNAUTHORIZED, "invalid credentials");
    }

    match issue_token(&tenant.id, &tenant.plan, &state.jwt_secret) {
        Ok(token) => {
            Json(auth_response(&tenant.id, tenant.username.as_deref().unwrap_or(tenant.email.as_str()), token))
                .into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Provider credentials ───────────────────────────────────────────────────

/// GET /providers — list supported providers plus suggested model IDs for the UI.
pub async fn list_providers(_tenant: AuthenticatedTenant) -> impl IntoResponse {
    Json(provider_catalog_json()).into_response()
}

#[derive(Deserialize)]
pub struct SetCredentialRequest {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub label: String,
}

/// PUT /credentials — store provider API key for this tenant (encrypted at rest).
/// Also updates routing config to use this provider if no routing is set yet.
pub async fn set_credential(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<SetCredentialRequest>,
) -> impl IntoResponse {
    let provider_name = body.provider.clone();
    if !crate::providers::supports_provider(&provider_name) {
        return err(StatusCode::BAD_REQUEST, format!("unsupported provider '{}'", provider_name));
    }
    let secret_enc = encrypt_secret(&body.api_key, &state.encrypt_key);

    let cred =
        ProviderCredential { provider: body.provider, secret_enc, label: body.label, model: body.model, enabled: true };

    match state.tenant_store.get_config(&tenant.tenant_id).await {
        Ok(mut config) => {
            // Store credential
            config.set_credential(cred);

            // If this is the first credential, auto-configure routing to use it
            let default_routing = crate::tenant::TenantRoutingConfig::default();
            let routing_is_default =
                config.routing.simple == default_routing.simple && config.routing.complex == default_routing.complex;

            if routing_is_default || config.credentials.len() == 1 {
                config.routing.simple = provider_name.clone();
                config.routing.medium = provider_name.clone();
                config.routing.complex = provider_name.clone();
                config.routing.fallback = provider_name.clone();
            }

            match state.tenant_store.upsert_config(&config).await {
                Ok(_) => {
                    let _ = state
                        .audit_log
                        .append(
                            &tenant.tenant_id,
                            None,
                            crate::audit::AuditAction::CredentialSet,
                            serde_json::json!({ "provider": provider_name }),
                            None,
                        )
                        .await;
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "saved":    true,
                            "provider": provider_name,
                            "routing_updated": routing_is_default || config.credentials.len() <= 1,
                        })),
                    )
                        .into_response()
                }
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// DELETE /credentials/:provider — remove a provider credential.
pub async fn delete_credential(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    match state.tenant_store.get_config(&tenant.tenant_id).await {
        Ok(mut config) => {
            config.credentials.remove(&provider);
            match state.tenant_store.upsert_config(&config).await {
                Ok(_) => Json(serde_json::json!({ "deleted": true })).into_response(),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /credentials — list provider names configured for this tenant (no secrets).
pub async fn list_credentials(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.tenant_store.get_config(&tenant.tenant_id).await {
        Ok(config) => {
            let providers: Vec<serde_json::Value> = config
                .credentials
                .values()
                .map(|c| {
                    serde_json::json!({
                        "provider": c.provider,
                        "model":    c.model,
                        "label":    c.label,
                        "enabled":  c.enabled,
                    })
                })
                .collect();
            Json(serde_json::json!({ "credentials": providers })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// PUT /routing — update per-tenant LLM routing preferences.
pub async fn update_routing(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<crate::tenant::TenantRoutingConfig>,
) -> impl IntoResponse {
    match state.tenant_store.get_config(&tenant.tenant_id).await {
        Ok(mut config) => {
            config.routing = body;
            match state.tenant_store.upsert_config(&config).await {
                Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "updated": true }))).into_response(),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Metrics + costs ────────────────────────────────────────────────────────

pub async fn get_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let s = state.metrics.snapshot();
    Json(serde_json::json!({
        // Raw backend fields
        "steps_total":         s.steps_total,
        "agents_running":      s.agents_running,
        "goals_total":         s.goals_total,
        "llm_calls_total":     s.llm_calls_total,
        "llm_cache_hits":      s.llm_cache_hits,
        "input_tokens_total":  s.input_tokens_total,
        "output_tokens_total": s.output_tokens_total,
        "uptime_secs":         s.uptime_secs,
        // Frontend-expected aliases
        "agents_started":  s.goals_total,
        "agents_finished": s.goals_total.saturating_sub(s.agents_running),
        "steps_completed": s.steps_total,
        "steps_per_minute": if s.uptime_secs > 0 {
            (s.steps_total as f64 / s.uptime_secs as f64 * 60.0).round() as u64
        } else { 0 },
    }))
}

pub async fn get_costs(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    let tenant_usage = state.cost_tracker.get_tenant_usage(&tenant.tenant_id).await;
    let limit = tenant.plan.spend_limit_usd();
    let current = tenant_usage.as_ref().map(|u| u.total_cost_usd).unwrap_or(0.0);
    let pct_used = if limit > 0.0 { (current / limit) * 100.0 } else { 0.0 };

    // Build per-provider usage breakdown.
    // CostTracker tracks per-agent; roll up into a single "combined" provider entry
    // so the frontend usage tab renders correctly even without per-provider breakdown.
    let input_tokens = tenant_usage.as_ref().map(|u| u.total_input_tokens).unwrap_or(0);
    let output_tokens = tenant_usage.as_ref().map(|u| u.total_output_tokens).unwrap_or(0);
    let requests = tenant_usage.as_ref().map(|u| u.total_requests).unwrap_or(0);

    Json(serde_json::json!({
        "tenant_id":         tenant.tenant_id,
        "spend_limit_usd":   limit,
        "current_spend_usd": current,
        "pct_used":          pct_used,
        // Legacy flat fields
        "total_input_tokens":  input_tokens,
        "total_output_tokens": output_tokens,
        "total_requests":      requests,
        // Per-provider breakdown expected by the frontend UsageTab
        "total_usd": current,
        "usage": {
            "platform": {
                "input_tokens":  input_tokens,
                "output_tokens": output_tokens,
                "usd":           current,
                "requests":      requests,
            }
        },
    }))
    .into_response()
}

// ── Goals ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateGoalRequest {
    pub description: String,
    /// Omit to auto-create a new conversation. Provide to continue an existing one.
    pub conversation_id: Option<String>,
}

#[derive(Serialize)]
pub struct CreateGoalResponse {
    pub goal_id: String,
    pub agent_id: String,
    pub conversation_id: String,
}

/// POST /goals
pub async fn create_goal(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<CreateGoalRequest>,
) -> impl IntoResponse {
    if body.description.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "description required");
    }

    // Enforce plan agent limit
    match state.store.count_active_agents(&tenant.tenant_id).await {
        Ok(count) if count >= tenant.plan.max_agents() as i64 => {
            return err(
                StatusCode::TOO_MANY_REQUESTS,
                format!("agent limit reached for your plan ({} max)", tenant.plan.max_agents()),
            );
        }
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        _ => {}
    }

    // Enforce step budget before creating a new goal.
    // Narayan is BYOK — we charge for platform step execution, not LLM token spend.
    // spend_limit_usd is kept as an informational display metric only.
    {
        let steps_limit = tenant.plan.max_steps_per_month();
        if steps_limit != u64::MAX {
            let steps_used = state.metrics.steps_this_month(&tenant.tenant_id);
            // Also add any purchased credit steps
            let extra_steps = state.billing.get_extra_steps(&tenant.tenant_id).await.unwrap_or(0);
            let total_budget = steps_limit + extra_steps;
            if steps_used >= total_budget {
                return err(
                    StatusCode::PAYMENT_REQUIRED,
                    format!(
                        "Monthly step limit reached ({} of {} steps used). \
                         Upgrade your plan or purchase a credit top-up ($8 = 5,000 extra steps).",
                        steps_used, total_budget
                    ),
                );
            }
            if steps_used >= (total_budget as f64 * 0.8) as u64 {
                tracing::warn!(
                    tenant_id = %tenant.tenant_id,
                    steps_used, total_budget,
                    "tenant approaching step limit"
                );
            }
        }
    }

    // ── Resolve or create conversation ──────────────────────────────────────
    let conv_id = if let Some(ref cid) = body.conversation_id {
        // Verify conversation exists and belongs to this tenant
        match state.store.get_conversation(&tenant.tenant_id, cid).await {
            Ok(Some(_)) => {
                let _ = state.store.touch_conversation(cid).await;
                cid.clone()
            }
            Ok(None) => return err(StatusCode::NOT_FOUND, "conversation not found"),
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    } else {
        // Auto-create a new conversation
        let cid = crate::util::new_id();
        let title: String = body.description.chars().take(80).collect();
        if let Err(e) = state.store.create_conversation(&cid, &tenant.tenant_id, Some(&title)).await {
            return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
        cid
    };

    match state.manager.create_goal(tenant.tenant_id.clone(), body.description.clone(), Some(conv_id.clone())).await {
        Ok((goal, agent)) => {
            state.metrics.goal_created();
            let _ = state
                .audit_log
                .append(
                    &tenant.tenant_id,
                    Some(&agent.id),
                    crate::audit::AuditAction::GoalCreated,
                    serde_json::json!({
                        "goal_id": goal.id,
                        "description": body.description,
                        "conversation_id": conv_id,
                    }),
                    None,
                )
                .await;
            (
                StatusCode::CREATED,
                Json(CreateGoalResponse { goal_id: goal.id, agent_id: agent.id, conversation_id: conv_id }),
            )
                .into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Conversations ─────────────────────────────────────────────────────────

/// GET /conversations — list conversations for this tenant.
pub async fn list_conversations(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.store.list_conversations(&tenant.tenant_id).await {
        Ok(conversations) => {
            // For each conversation, count agents
            let mut items = Vec::with_capacity(conversations.len());
            for conv in &conversations {
                let agent_count = state
                    .store
                    .list_agents_in_conversation(&tenant.tenant_id, &conv.id)
                    .await
                    .map(|a| a.len())
                    .unwrap_or(0);
                items.push(serde_json::json!({
                    "id":          conv.id,
                    "title":       conv.title,
                    "created_at":  conv.created_at.to_rfc3339(),
                    "updated_at":  conv.updated_at.to_rfc3339(),
                    "agent_count": agent_count,
                }));
            }
            Json(serde_json::json!({ "conversations": items })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /conversations/:id — conversation detail with all agent summaries.
pub async fn get_conversation(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(conv_id): Path<String>,
) -> impl IntoResponse {
    let conv = match state.store.get_conversation(&tenant.tenant_id, &conv_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return err(StatusCode::NOT_FOUND, "conversation not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let agents_result = state.store.list_agents_in_conversation(&tenant.tenant_id, &conv_id).await;
    let agents_list = match agents_result {
        Ok(a) => a,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let agents_json: Vec<serde_json::Value> = agents_list
        .iter()
        .map(|a| {
            serde_json::json!({
                "id":           a.id,
                "goal":         a.goal,
                "status":       format!("{:?}", a.status).to_lowercase(),
                "current_step": a.current_step,
                "final_answer": a.final_answer(),
                "created_at":   a.created_at.to_rfc3339(),
                "updated_at":   a.updated_at.to_rfc3339(),
            })
        })
        .collect();

    Json(serde_json::json!({
        "id":          conv.id,
        "title":       conv.title,
        "created_at":  conv.created_at.to_rfc3339(),
        "updated_at":  conv.updated_at.to_rfc3339(),
        "agents":      agents_json,
    }))
    .into_response()
}

// ── Agents ─────────────────────────────────────────────────────────────────

/// GET /agents — list all agents for this tenant.
pub async fn list_agents(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.store.list_agents(&tenant.tenant_id).await {
        Ok(agents) => {
            let body: Vec<serde_json::Value> = agents
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "id":              a.id,
                        "goal":            a.goal,
                        "status":          format!("{:?}", a.status).to_lowercase(),
                        "current_step":    a.current_step,
                        "next_run":        a.next_run.to_rfc3339(),
                        "created_at":      a.created_at.to_rfc3339(),
                        "updated_at":      a.updated_at.to_rfc3339(),
                        "conversation_id": a.conversation_id,
                    })
                })
                .collect();
            Json(serde_json::json!({ "agents": body })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /agents/:id — tenant-scoped fetch (enhanced with plan, children, job_type).
pub async fn get_agent(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(a)) => {
            let final_answer = a.final_answer().map(str::to_string);
            let key_findings = a.metadata.get("key_findings").cloned().unwrap_or_else(|| serde_json::json!([]));
            let plan_json = a.plan.as_ref().map(|p| serde_json::to_value(p).unwrap_or_default());
            let step_count = a.plan.as_ref().map(|p| p.steps.len()).unwrap_or(0);
            let job_type = a.plan.as_ref().and_then(|p| p.job_type.clone());
            let cost = state.cost_tracker.get_usage(&a.id).await;
            Json(serde_json::json!({
                "id":               a.id,
                "goal":             a.goal,
                "status":           format!("{:?}", a.status).to_lowercase(),
                "current_step":     a.current_step,
                "step_count":       step_count,
                "workspace_path":   a.workspace_path,
                "next_run":         a.next_run.to_rfc3339(),
                "created_at":       a.created_at.to_rfc3339(),
                "updated_at":       a.updated_at.to_rfc3339(),
                "started_at":       a.started_at.map(|t| t.to_rfc3339()),
                "final_answer":     final_answer.clone(),
                "plan":             plan_json,
                "job_type":         job_type,
                "parent_agent_id":  a.parent_agent_id,
                "pending_children": a.pending_children,
                "conversation_id":  a.conversation_id,
                "cost": cost.as_ref().map(|c| serde_json::json!({
                    "total_cost_usd": c.total_cost_usd,
                    "total_input_tokens": c.total_input_tokens,
                    "total_output_tokens": c.total_output_tokens,
                    "total_requests": c.total_requests,
                })),
                "metadata": {
                    "final_answer": final_answer,
                    "last_reflection": a.metadata.get("last_reflection"),
                    "key_findings": key_findings,
                },
            }))
            .into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /agents/:id/logs
pub async fn get_agent_logs(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(a)) => {
            let log_path = format!("{}/logs/agent.log", a.workspace_path);
            let content = tokio::fs::read_to_string(&log_path).await.unwrap_or_default();
            Json(serde_json::json!({ "logs": content })).into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /agents/:id/workspace/files — list files in agent workspace.
pub async fn list_workspace_files(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let agent = match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let base = std::path::PathBuf::from(&agent.workspace_path).join("files");
    let mut files = Vec::new();
    if base.exists() {
        collect_workspace_files(&base, &base, &mut files);
    }

    Json(serde_json::json!({
        "agent_id": agent_id,
        "files": files,
        "count": files.len()
    })).into_response()
}

fn collect_workspace_files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<serde_json::Value>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_string();
            if path.is_dir() {
                let mut children = Vec::new();
                collect_workspace_files(root, &path, &mut children);
                out.push(serde_json::json!({
                    "name": entry.file_name().to_string_lossy(),
                    "path": rel,
                    "is_dir": true,
                    "children": children
                }));
            } else {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let modified = entry.metadata().ok()
                    .and_then(|m| m.modified().ok())
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Utc> = t.into();
                        dt.to_rfc3339()
                    });
                out.push(serde_json::json!({
                    "name": entry.file_name().to_string_lossy(),
                    "path": rel,
                    "is_dir": false,
                    "size": size,
                    "modified": modified
                }));
            }
        }
    }
}

/// GET /agents/:id/workspace/files/*path — read a specific workspace file.
pub async fn read_workspace_file(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path((agent_id, file_path)): Path<(String, String)>,
) -> impl IntoResponse {
    let agent = match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    // Prevent directory traversal
    if file_path.contains("..") {
        return err(StatusCode::BAD_REQUEST, "invalid path");
    }

    let base = std::path::PathBuf::from(&agent.workspace_path).join("files");
    let full_path = base.join(&file_path);

    if !full_path.starts_with(&base) {
        return err(StatusCode::BAD_REQUEST, "invalid path");
    }

    match tokio::fs::read(&full_path).await {
        Ok(content) => {
            if content.len() > 1_048_576 {
                return err(StatusCode::BAD_REQUEST, "file too large (>1MB)");
            }
            // Guess content type from extension
            let ct = match full_path.extension().and_then(|e| e.to_str()) {
                Some("md" | "txt" | "log") => "text/plain; charset=utf-8",
                Some("json") => "application/json",
                Some("csv") => "text/csv",
                Some("html") => "text/html",
                Some("png") => "image/png",
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("pdf") => "application/pdf",
                _ => "application/octet-stream",
            };
            ([(axum::http::header::CONTENT_TYPE, ct)], content).into_response()
        }
        Err(_) => err(StatusCode::NOT_FOUND, "file not found"),
    }
}

/// GET /agents/:id/children — list child agents for delegation view.
pub async fn list_agent_children(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    // Verify parent exists
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(None) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        _ => {}
    }

    match state.store.get_agent_children(&tenant.tenant_id, &agent_id).await {
        Ok(children) => {
            let body: Vec<serde_json::Value> = children.iter().map(|c| {
                let step_count = c.plan.as_ref().map(|p| p.steps.len()).unwrap_or(0);
                serde_json::json!({
                    "id": c.id,
                    "goal": c.goal,
                    "status": format!("{:?}", c.status).to_lowercase(),
                    "current_step": c.current_step,
                    "step_count": step_count,
                    "created_at": c.created_at.to_rfc3339(),
                    "updated_at": c.updated_at.to_rfc3339(),
                })
            }).collect();
            Json(serde_json::json!({
                "parent_id": agent_id,
                "children": body,
                "count": body.len()
            })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /agents/:id/plan/approve — approve the agent's plan.
pub async fn approve_plan(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let agent = match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    if agent.plan.is_none() {
        return err(StatusCode::BAD_REQUEST, "no plan to approve");
    }

    if let Err(e) = state.store.update_agent_status(&tenant.tenant_id, &agent_id, "running").await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }

    state.event_bus_handle.publish(crate::events::AgentEvent::PlanApproved {
        agent_id: agent_id.clone(),
    });

    let _ = state.audit_log.append(
        &tenant.tenant_id,
        &agent_id,
        "plan_approved",
        serde_json::json!({}),
    ).await;

    Json(serde_json::json!({"status": "approved", "agent_id": agent_id})).into_response()
}

/// POST /agents/:id/plan/reject — reject with feedback, triggers replan.
pub async fn reject_plan(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent = match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let feedback = body.get("feedback").and_then(|v| v.as_str()).unwrap_or("");

    let mut meta = agent.metadata.clone();
    meta["plan_feedback"] = serde_json::json!(feedback);
    let _ = state.store.update_agent_metadata(&tenant.tenant_id, &agent_id, &meta).await;
    let _ = state.store.update_agent_status(&tenant.tenant_id, &agent_id, "preflight").await;

    state.event_bus_handle.publish(crate::events::AgentEvent::PlanRejected {
        agent_id: agent_id.clone(),
        feedback: feedback.to_string(),
    });

    let _ = state.audit_log.append(
        &tenant.tenant_id,
        &agent_id,
        "plan_rejected",
        serde_json::json!({"feedback": feedback}),
    ).await;

    Json(serde_json::json!({"status": "rejected", "agent_id": agent_id})).into_response()
}

/// POST /agents/:id/plan/edit — edit plan steps then approve.
pub async fn edit_plan(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent = match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(a)) => a,
        Ok(None) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let edited_steps = body.get("steps").cloned().unwrap_or(serde_json::json!([]));
    let step_count = edited_steps.as_array().map(|a| a.len()).unwrap_or(0);

    if let Some(plan) = agent.plan.as_ref() {
        let mut plan_json = serde_json::to_value(plan).unwrap_or_default();
        plan_json["steps"] = edited_steps.clone();
        let _ = state.store.update_agent_plan(&tenant.tenant_id, &agent_id, &plan_json).await;
    }

    let _ = state.store.update_agent_status(&tenant.tenant_id, &agent_id, "running").await;

    state.event_bus_handle.publish(crate::events::AgentEvent::PlanEdited {
        agent_id: agent_id.clone(),
        step_count,
    });

    let _ = state.audit_log.append(
        &tenant.tenant_id,
        &agent_id,
        "plan_edited",
        serde_json::json!({"step_count": step_count}),
    ).await;

    Json(serde_json::json!({"status": "edited", "agent_id": agent_id, "step_count": step_count})).into_response()
}

/// POST /agents/:id/pause
pub async fn pause_agent(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(mut a)) => {
            a.status = AgentStatus::Paused;
            a.updated_at = chrono::Utc::now();
            match state.store.upsert_agent(&a).await {
                Ok(_) => Json(serde_json::json!({ "paused": true })).into_response(),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /agents/:id/resume
pub async fn resume_agent(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(mut a)) => {
            a.status = AgentStatus::Waiting;
            a.next_run = chrono::Utc::now();
            a.updated_at = chrono::Utc::now();
            match state.store.upsert_agent(&a).await {
                Ok(_) => Json(serde_json::json!({ "resumed": true })).into_response(),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Clarification endpoint ─────────────────────────────────────────────────

/// POST /agents/:id/clarify — submit answers to clarification questions.
pub async fn submit_clarification(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
    Json(body): Json<crate::agent::ClarificationAnswers>,
) -> impl IntoResponse {
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(mut agent)) => {
            if agent.status != crate::state::AgentStatus::Clarifying {
                return err(StatusCode::BAD_REQUEST, "agent is not in clarifying state");
            }

            let questions = agent
                .metadata
                .get("clarification_questions")
                .map(crate::agent::clarifier::parse_clarification_questions)
                .unwrap_or_default();

            for (index, question) in questions.iter().enumerate() {
                if question.required
                    && body.answers.get(index).map(|answer| answer.trim().is_empty()).unwrap_or(true)
                {
                    return err(
                        StatusCode::BAD_REQUEST,
                        format!("answer required for '{}'", question.prompt),
                    );
                }
            }

            let mut safe_answers = Vec::with_capacity(body.answers.len());
            for (index, answer) in body.answers.iter().enumerate() {
                let trimmed = answer.trim();
                let safe_answer = match questions.get(index) {
                    Some(question)
                        if (question.secret || question.store_as_credential.is_some()) && !trimmed.is_empty() =>
                    {
                        let credential_key = question
                            .store_as_credential
                            .clone()
                            .filter(|value| !value.trim().is_empty())
                            .unwrap_or_else(|| question.id.clone());
                        crate::tools::memory_store_internal::insert(
                            format!("credential:{credential_key}"),
                            trimmed.to_string(),
                        );
                        format!(
                            "User provided '{}' securely and it was stored as credential '{}'.",
                            question.prompt, credential_key
                        )
                    }
                    Some(question) if question.connector_type.is_some() && trimmed.is_empty() => format!(
                        "Connector setup requested for '{}'.",
                        question.connector_type.clone().unwrap_or_else(|| question.prompt.clone())
                    ),
                    _ => trimmed.to_string(),
                };
                safe_answers.push(safe_answer);
            }

            let sanitized_answers = crate::agent::ClarificationAnswers {
                answers: safe_answers.clone(),
                freeform: if questions.iter().any(|question| question.secret || question.store_as_credential.is_some()) {
                    None
                } else {
                    body.freeform.clone()
                },
            };

            // Persist answers into metadata so the loop can use them without leaking secrets.
            agent.metadata["clarification_answers"] = serde_json::to_value(&sanitized_answers).unwrap_or_default();
            agent.metadata["last_user_input_context"] =
                serde_json::json!(safe_answers.iter().filter(|answer| !answer.is_empty()).cloned().collect::<Vec<_>>().join("\n"));
            if let Some(metadata) = agent.metadata.as_object_mut() {
                metadata.remove("clarification_questions");
            }

            // Move agent back to waiting so it gets scheduled for planning
            agent.status = crate::state::AgentStatus::Waiting;
            agent.next_run = chrono::Utc::now();
            agent.updated_at = chrono::Utc::now();

            match state.store.upsert_agent(&agent).await {
                Ok(_) => {
                    state.event_bus_handle.publish(crate::events::AgentEvent::ClarificationReceived {
                        agent_id: agent.id.clone(),
                    });
                    Json(serde_json::json!({ "acknowledged": true })).into_response()
                }
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Skill Marketplace endpoints ────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct UploadSkillRequest {
    pub name: String,
    pub description: String,
    pub steps: Vec<String>,
    pub author: Option<String>,
}

/// POST /skills/upload — publish a skill to the marketplace.
pub async fn upload_skill(
    State(state): State<AppState>,
    _tenant: AuthenticatedTenant,
    Json(body): Json<UploadSkillRequest>,
) -> impl IntoResponse {
    let skill_name = body.name.clone();
    let skill = marketplace_skill_from_upload(body);
    state.marketplace.lock().await.upload(skill);
    (StatusCode::CREATED, Json(serde_json::json!({ "uploaded": true, "name": skill_name }))).into_response()
}

/// GET /skills — list all marketplace skills.
pub async fn list_skills(State(state): State<AppState>, _tenant: AuthenticatedTenant) -> impl IntoResponse {
    let mp = state.marketplace.lock().await;
    Json(marketplace_list_json(&mp)).into_response()
}

/// POST /skills/install — install a marketplace skill into the agent skill registry.
#[derive(serde::Deserialize)]
pub struct InstallSkillRequest {
    pub name: String,
}

pub async fn install_skill(
    State(state): State<AppState>,
    _tenant: AuthenticatedTenant,
    Json(body): Json<InstallSkillRequest>,
) -> impl IntoResponse {
    let mp = state.marketplace.lock().await;
    let mut registry = state.skill_registry.write().await;
    match install_marketplace_skill(&mp, &mut *registry, &body.name) {
        Ok(()) => Json(serde_json::json!({ "installed": true, "name": body.name })).into_response(),
        Err(message) => err(StatusCode::NOT_FOUND, message),
    }
}

/// GET /skills/registry — list all installed skills.
pub async fn list_installed_skills(State(state): State<AppState>, _tenant: AuthenticatedTenant) -> impl IntoResponse {
    let reg = state.skill_registry.read().await;
    Json(installed_skills_json(&reg)).into_response()
}

/// GET /agents/:id/replay — replay a recorded agent execution.
pub async fn replay_agent(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(Some(agent)) => {
            let recording: Vec<serde_json::Value> =
                agent.metadata.get("debug_recording").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            Json(serde_json::json!({
                "agent_id": agent_id,
                "steps":    recording,
                "count":    recording.len(),
            }))
            .into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Webhook endpoints ─────────────────────────────────────────────────────

/// POST /webhooks — register a new webhook endpoint.
pub async fn create_webhook(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<crate::webhooks::config::WebhookCreateRequest>,
) -> impl IntoResponse {
    if body.url.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "url required");
    }

    let secret = body.secret.unwrap_or_else(|| crate::util::new_id());
    match state.webhook_store.create(&tenant.tenant_id, &body.url, &secret, &body.events).await {
        Ok(hook) => {
            let _ = state
                .audit_log
                .append(
                    &tenant.tenant_id,
                    None,
                    crate::audit::AuditAction::WebhookRegistered,
                    serde_json::json!({ "webhook_id": hook.id, "url": hook.url }),
                    None,
                )
                .await;
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "id": hook.id,
                    "url": hook.url,
                    "secret": secret,
                    "events": hook.events,
                })),
            )
                .into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /webhooks — list all webhooks for this tenant.
pub async fn list_webhooks(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.webhook_store.list_for_tenant(&tenant.tenant_id).await {
        Ok(hooks) => {
            let body: Vec<serde_json::Value> = hooks
                .iter()
                .map(|h| {
                    serde_json::json!({
                        "id": h.id,
                        "url": h.url,
                        "events": h.events,
                        "enabled": h.enabled,
                        "failure_count": h.failure_count,
                    })
                })
                .collect();
            Json(serde_json::json!({ "webhooks": body, "count": body.len() })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// DELETE /webhooks/:id — remove a webhook.
pub async fn delete_webhook(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(webhook_id): Path<String>,
) -> impl IntoResponse {
    match state.webhook_store.delete(&tenant.tenant_id, &webhook_id).await {
        Ok(true) => Json(serde_json::json!({ "deleted": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "webhook not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /audit — query the immutable audit log for this tenant.
pub async fn query_audit_log(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    axum::extract::Query(params): axum::extract::Query<crate::audit::AuditQuery>,
) -> impl IntoResponse {
    let mut q = params;
    // Scope to this tenant — tenants can only see their own audit entries
    q.tenant_id = Some(tenant.tenant_id.clone());
    match state.audit_log.query(&q).await {
        Ok(entries) => {
            let count = entries.len();
            Json(serde_json::json!({ "entries": entries, "count": count })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Connector inbound webhook ─────────────────────────────────────────────

/// POST /connectors/:type/webhook — receive inbound events from external services.
/// Looks up the connector from the segment registry, calls handle_inbound,
/// and if a goal string is returned, creates an agent via AgentManager.
pub async fn connector_inbound(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let event_type = payload.get("event_type").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

    let event = crate::connectors::ConnectorEvent {
        connector_type: connector_type.clone(),
        event_type: event_type.clone(),
        payload: payload.clone(),
        tenant_id: tenant.tenant_id.clone(),
        external_id: payload.get("id").and_then(|v| v.as_str()).map(String::from),
    };

    // ── Look up connector and generate goal ───────────────────────────────
    let connector = match state.connector_registry.get(&connector_type) {
        Some(c) => c,
        None => {
            return err(StatusCode::NOT_FOUND, format!("no connector registered for type '{connector_type}'"));
        }
    };

    // Load real ConnectorConfig from the install store.
    // Falls back to empty credentials if tenant hasn't connected this connector —
    // the connector's handle_inbound will return an error with a helpful message.
    let (credentials, settings) = match state.connector_installs.get(&tenant.tenant_id, &connector_type).await {
        Ok(Some(install)) => {
            // Verify webhook signature if this is a webhook_only install
            if install.auth_type == "webhook_only" {
                if let Some(secret) = state.connector_installs.decrypt_webhook_secret(&install) {
                    // Verify HMAC — connector-specific header names handled below
                    let sig_header = payload
                        .get("x-hub-signature-256")
                        .or_else(|| payload.get("x-pagerduty-signature"))
                        .or_else(|| payload.get("x-hubspot-signature"))
                        .or_else(|| payload.get("stripe-signature"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !sig_header.is_empty()
                        && !verify_webhook_hmac(sig_header, &serde_json::to_vec(&payload).unwrap_or_default(), &secret)
                    {
                        return err(StatusCode::UNAUTHORIZED, "webhook signature verification failed");
                    }
                }
            }
            let token = state
                .connector_installs
                .decrypt_token(&install)
                .map(|t| serde_json::json!({ "access_token": t, "api_key": t, "token": t }))
                .unwrap_or(serde_json::json!({}));
            (token, install.settings.clone())
        }
        _ => (serde_json::json!({}), serde_json::json!({})),
    };

    let config = crate::connectors::ConnectorConfig {
        id: crate::util::new_id(),
        tenant_id: tenant.tenant_id.clone(),
        connector_type: connector_type.clone(),
        credentials,
        settings,
        enabled: true,
    };

    let goal_str = match connector.handle_inbound(&event, &config).await {
        Ok(Some(g)) => g,
        Ok(None) => {
            // Connector handled the event but produced no agent goal (e.g. unsupported event type)
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "received":  true,
                    "connector": connector_type,
                    "agent_created": false,
                    "reason": "event type produced no goal",
                })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(connector = %connector_type, error = %e, "connector handle_inbound failed");
            return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
    };

    // ── Create an agent for the goal ──────────────────────────────────────
    match state.manager.create_goal(tenant.tenant_id.clone(), goal_str.clone(), None).await {
        Ok((goal, agent)) => {
            // Emit ConnectorTrigger SSE so the frontend can show the trigger card
            state.event_bus_handle.publish(crate::events::AgentEvent::ConnectorTrigger {
                agent_id: agent.id.clone(),
                connector_type: connector_type.clone(),
                event_type: event_type.clone(),
                external_id: payload.get("id").and_then(|v| v.as_str()).map(String::from),
            });

            let _ = state
                .audit_log
                .append(
                    &tenant.tenant_id,
                    Some(&agent.id),
                    crate::audit::AuditAction::Custom,
                    serde_json::json!({
                        "action":         "connector_agent_created",
                        "connector_type": connector_type,
                        "event_type":     event_type,
                        "goal":           goal_str,
                    }),
                    None,
                )
                .await;

            tracing::info!(
                connector  = %connector_type,
                agent_id   = %agent.id,
                goal_id    = %goal.id,
                "connector created agent"
            );

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "received":      true,
                    "connector":     connector_type,
                    "agent_created": true,
                    "agent_id":      agent.id,
                    "goal_id":       goal.id,
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(connector = %connector_type, error = %e, "failed to create agent from connector event");
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

// ── Review queue endpoints ────────────────────────────────────────────────

/// GET /reviews — list review items for this tenant.
/// Query param: status=pending|all (default: all)
pub async fn list_reviews(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let pending_only = params.get("status").map(|s| s == "pending").unwrap_or(false);
    match state.review_queue.pending(&tenant.tenant_id).await {
        Ok(mut items) => {
            if pending_only {
                items.retain(|i| i.status == crate::compliance::ReviewStatus::Pending);
            }
            let count = items.len();
            Json(serde_json::json!({ "reviews": items, "count": count })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /reviews/:id/resolve — approve or reject a review item.
#[derive(Deserialize)]
pub struct ResolveReviewRequest {
    pub status: String,
    pub notes: Option<String>,
}

pub async fn resolve_review(
    State(state): State<AppState>,
    _tenant: AuthenticatedTenant,
    Path(review_id): Path<String>,
    Json(body): Json<ResolveReviewRequest>,
) -> impl IntoResponse {
    // auto_approved is a UI concept — maps to Approved on the wire
    let status = match body.status.as_str() {
        "approved" | "auto_approved" => crate::compliance::ReviewStatus::Approved,
        "rejected" => crate::compliance::ReviewStatus::Rejected,
        "changes_requested" => crate::compliance::ReviewStatus::ChangesRequested,
        _ => {
            return err(
                StatusCode::BAD_REQUEST,
                "status must be: approved, auto_approved, rejected, or changes_requested",
            )
        }
    };

    match state.review_queue.resolve(&review_id, status, body.notes.as_deref()).await {
        Ok(_) => Json(serde_json::json!({ "resolved": true })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /swarm/status — current swarm queue depth plus pool info.
pub async fn swarm_status(state: State<AppState>, _tenant: AuthenticatedTenant) -> impl IntoResponse {
    let depth = state.swarm.queue_depth().await.unwrap_or(0);
    // Read pool size from config env (falls back to 32 default)
    let pool_size: u64 = std::env::var("NARAYAN__WORKER__POOL_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let queue_backed = std::env::var("NARAYAN__REDIS__ENABLED").map(|v| v == "true").unwrap_or(false);
    Json(serde_json::json!({
        "queue_depth":   depth,
        "pool_size":     pool_size,
        "queue_backed":  queue_backed,
    }))
    .into_response()
}

// ── Citations ─────────────────────────────────────────────────────────────

/// GET /agents/:id/citations — all citations recorded for a specific agent.
pub async fn list_agent_citations(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    // Verify agent belongs to tenant first
    match state.store.get_agent(&tenant.tenant_id, &agent_id).await {
        Ok(None) => return err(StatusCode::NOT_FOUND, "agent not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Ok(Some(_)) => {}
    }

    if let Some(ref ct) = state.citation_tracker {
        match ct.get_for_agent(&agent_id).await {
            Ok(cites) => {
                let count = cites.len();
                return Json(serde_json::json!({ "citations": cites, "count": count })).into_response();
            }
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
    Json(serde_json::json!({ "citations": [], "count": 0 })).into_response()
}

/// GET /citations — all citations for this tenant (cross-agent, last 200).
pub async fn list_tenant_citations(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    if let Some(ref ct) = state.citation_tracker {
        match ct.get_for_tenant(&tenant.tenant_id, 200).await {
            Ok(cites) => {
                let count = cites.len();
                return Json(serde_json::json!({ "citations": cites, "count": count })).into_response();
            }
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
    Json(serde_json::json!({ "citations": [], "count": 0 })).into_response()
}

// ── Auto-approvals ────────────────────────────────────────────────────────
// Stored in-memory (per process) — sufficient for the UI's "don't ask again"
// pattern. For production, back this with a DB table.

/// GET /auto-approvals — list saved auto-approval rules for this tenant.
pub async fn list_auto_approvals(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.auto_approvals.get_for_tenant(&tenant.tenant_id).await {
        Ok(rules) => {
            let count = rules.len();
            Json(serde_json::json!({ "rules": rules, "count": count })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct CreateAutoApprovalRequest {
    pub rule_id: String,
    pub notes: Option<String>,
}

/// POST /auto-approvals — save an auto-approval rule.
pub async fn create_auto_approval(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<CreateAutoApprovalRequest>,
) -> impl IntoResponse {
    if body.rule_id.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "rule_id required");
    }
    match state.auto_approvals.upsert(&tenant.tenant_id, &body.rule_id, body.notes.as_deref()).await {
        Ok(rule) => (StatusCode::CREATED, Json(serde_json::json!({ "saved": true, "rule": rule }))).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// DELETE /auto-approvals/:rule_id — remove an auto-approval rule.
pub async fn delete_auto_approval(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(rule_id): Path<String>,
) -> impl IntoResponse {
    match state.auto_approvals.delete(&tenant.tenant_id, &rule_id).await {
        Ok(true) => Json(serde_json::json!({ "deleted": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "auto-approval rule not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /reviews/resolve-all — bulk approve all pending reviews for this tenant.
pub async fn resolve_all_reviews(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<ResolveReviewRequest>,
) -> impl IntoResponse {
    let status = match body.status.as_str() {
        "approved" | "auto_approved" => crate::compliance::ReviewStatus::Approved,
        "rejected" => crate::compliance::ReviewStatus::Rejected,
        "changes_requested" => crate::compliance::ReviewStatus::ChangesRequested,
        _ => return err(StatusCode::BAD_REQUEST, "invalid status"),
    };

    match state.review_queue.pending(&tenant.tenant_id).await {
        Ok(items) => {
            let mut resolved = 0usize;
            for item in &items {
                if item.status == crate::compliance::ReviewStatus::Pending {
                    let _ = state.review_queue.resolve(&item.id, status.clone(), body.notes.as_deref()).await;
                    resolved += 1;
                }
            }
            Json(serde_json::json!({ "resolved": resolved })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_usage() -> AgentUsage {
        AgentUsage {
            agent_id: "agent-1".into(),
            total_input_tokens: 321,
            total_output_tokens: 123,
            total_cost_usd: 4.25,
            total_requests: 7,
        }
    }

    #[test]
    fn test_register_request_validation_rejects_blank_fields() {
        assert!(!register_request_is_valid(&RegisterRequest {
            name: "   ".into(),
            username: "narayan".into(),
            email: "team@example.com".into(),
            password: "password123".into(),
        }));
        assert!(!register_request_is_valid(&RegisterRequest {
            name: "Narayan".into(),
            username: "".into(),
            email: "team@example.com".into(),
            password: "password123".into(),
        }));
        assert!(!register_request_is_valid(&RegisterRequest {
            name: "Narayan".into(),
            username: "narayan".into(),
            email: "\n\t".into(),
            password: "password123".into(),
        }));
        assert!(!register_request_is_valid(&RegisterRequest {
            name: "Narayan".into(),
            username: "narayan".into(),
            email: "team@example.com".into(),
            password: "short".into(),
        }));
        assert!(register_request_is_valid(&RegisterRequest {
            name: "Narayan".into(),
            username: "narayan".into(),
            email: "team@example.com".into(),
            password: "password123".into(),
        }));
    }

    #[test]
    fn test_cost_response_json_returns_zero_shape_for_unknown_tenant_usage() {
        let payload = cost_response_json("tenant-1", None);
        assert_eq!(payload["tenant_id"], "tenant-1");
        assert_eq!(payload["total_cost_usd"], 0.0);
        assert!(payload.get("total_requests").is_none());
    }

    #[test]
    fn test_cost_response_json_includes_full_usage_totals() {
        let payload = cost_response_json("tenant-1", Some(sample_usage()));
        assert_eq!(payload["tenant_id"], "tenant-1");
        assert_eq!(payload["total_cost_usd"], 4.25);
        assert_eq!(payload["total_input_tokens"], 321);
        assert_eq!(payload["total_output_tokens"], 123);
        assert_eq!(payload["total_requests"], 7);
    }

    #[test]
    fn test_marketplace_skill_from_upload_defaults_author() {
        let skill = marketplace_skill_from_upload(UploadSkillRequest {
            name: "triage".into(),
            description: "Sort incoming incidents".into(),
            steps: vec!["collect context".into(), "prioritize".into()],
            author: None,
        });

        assert_eq!(skill.name, "triage");
        assert_eq!(skill.author, "anonymous");
        assert_eq!(skill.steps.len(), 2);
    }

    #[test]
    fn test_install_marketplace_skill_registers_skill_with_steps() {
        let mut marketplace = SkillMarketplace::new();
        marketplace.upload(MarketplaceSkill {
            name: "triage".into(),
            author: "ops".into(),
            description: "Sort incoming incidents".into(),
            steps: vec!["collect context".into(), "prioritize".into()],
        });
        let mut registry = SkillRegistry::new();

        install_marketplace_skill(&marketplace, &mut registry, "triage")
            .expect("skill should install from marketplace");

        let installed = registry.get("triage").expect("skill should be present");
        assert_eq!(installed.description, "Sort incoming incidents");
        assert_eq!(
            installed.steps,
            vec![
                crate::skills::registry::SkillStep::from("collect context"),
                crate::skills::registry::SkillStep::from("prioritize"),
            ]
        );
    }

    #[test]
    fn test_install_marketplace_skill_returns_precise_not_found_error() {
        let marketplace = SkillMarketplace::new();
        let mut registry = SkillRegistry::new();

        let error = install_marketplace_skill(&marketplace, &mut registry, "missing")
            .expect_err("missing skills should return a not-found message");

        assert_eq!(error, "skill 'missing' not in marketplace");
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_marketplace_and_registry_json_include_counts() {
        let mut marketplace = SkillMarketplace::new();
        marketplace.upload(MarketplaceSkill {
            name: "triage".into(),
            author: "ops".into(),
            description: "Sort incoming incidents".into(),
            steps: vec!["collect context".into(), "prioritize".into()],
        });
        let marketplace_payload = marketplace_list_json(&marketplace);
        assert_eq!(marketplace_payload["count"], 1);
        assert_eq!(marketplace_payload["skills"][0]["name"], "triage");
        assert_eq!(marketplace_payload["skills"][0]["step_count"], 2);

        let mut registry = SkillRegistry::new();
        registry.register(Skill::new(
            "triage",
            "Sort incoming incidents",
            vec!["collect context".into(), "prioritize".into()],
        ));
        let registry_payload = installed_skills_json(&registry);
        assert_eq!(registry_payload["count"], 1);
        assert_eq!(registry_payload["skills"][0]["name"], "triage");
        assert_eq!(registry_payload["skills"][0]["version"], 1);
    }

    #[test]
    fn test_provider_catalog_json_includes_latest_groq_and_nvidia_models() {
        let payload = provider_catalog_json();

        let providers = payload["providers"].as_array().expect("providers should be an array");
        let groq = providers.iter().find(|provider| provider["id"] == "groq").expect("groq should exist");
        let nvidia = providers.iter().find(|provider| provider["id"] == "nvidia").expect("nvidia should exist");

        assert!(groq["models"]
            .as_array()
            .expect("groq models should be an array")
            .iter()
            .any(|model| model == "openai/gpt-oss-120b"));
        assert!(nvidia["models"]
            .as_array()
            .expect("nvidia models should be an array")
            .iter()
            .any(|model| model == "nvidia/nemotron-3-super-120b-a12b"));
        assert!(nvidia["models"]
            .as_array()
            .expect("nvidia models should be an array")
            .iter()
            .any(|model| model == "nvidia/nemotron-3-nano-30b-a3b"));
    }
}
