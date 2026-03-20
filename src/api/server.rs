use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    api::{
        admin::{
            middleware::{admin_auth_middleware, AdminAuthState},
            routes::{self as admin_routes, AdminState},
        },
        routes::*,
        stream::{agent_stream, StreamState},
    },
    audit::AuditLog,
    auth::middleware::{auth_middleware, AuthState},
    billing::routes as billing_routes,
    connectors::oauth,
    events::EventBus,
    gateway::CostTracker,
    metrics::Metrics,
    storage::PostgresStore,
    tenant::TenantStore,
};

pub fn build_router(
    state: AppState,
    tenant_store: Arc<TenantStore>,
    event_bus: Arc<EventBus>,
    store: Arc<PostgresStore>,
    audit_log: Arc<AuditLog>,
    cost_tracker: Arc<CostTracker>,
    metrics: Arc<Metrics>,
) -> Router {
    let auth_state = AuthState { jwt_secret: state.jwt_secret.clone() };
    let stream_state = StreamState { event_bus, store: store.clone() };

    // ── Public routes (no auth) ───────────────────────────────────────────
    let public = Router::new()
        .route("/health", get(health))
        .route("/auth/login", post(issue_session_token))
        .route("/auth/token", post(issue_session_token))
        .route("/auth/register", post(register))
        // OAuth callback is public — receives the code redirect from the provider
        // OAuth start is public — browser can't send Authorization header during a redirect.
        // Token is passed as ?token= query param and validated inside the handler.
        .route("/auth/oauth/:provider/start", get(oauth::oauth_start))
        // OAuth callback is public — provider redirects to it with no auth headers
        .route("/auth/oauth/:provider/callback", get(oauth::oauth_callback))
        // Billing webhooks are public — signed by provider, verified inside handler
        .route("/billing/webhooks/:provider", post(billing_routes::handle_webhook));

    // ── Protected routes (tenant JWT) ─────────────────────────────────────
    let protected = Router::new()
        // Metrics & costs
        .route("/metrics", get(get_metrics))
        .route("/costs", get(get_costs))
        // Credentials & routing
        .route("/credentials", put(set_credential))
        .route("/credentials", get(list_credentials))
        .route("/credentials/:provider", delete(delete_credential))
        .route("/routing", put(update_routing))
        // Goals & agents
        .route("/goals", post(create_goal))
        // Conversations
        .route("/conversations", get(list_conversations))
        .route("/conversations/:id", get(get_conversation))
        .route("/agents", get(list_agents))
        .route("/agents/:id", get(get_agent))
        .route("/agents/:id/logs", get(get_agent_logs))
        .route("/agents/:id/pause", post(pause_agent))
        .route("/agents/:id/resume", post(resume_agent))
        .route("/agents/:id/clarify", post(submit_clarification))
        .route("/agents/:id/replay", get(replay_agent))
        .route("/agents/:id/citations", get(list_agent_citations))
        // Citations (cross-agent)
        .route("/citations", get(list_tenant_citations))
        // Skills
        .route("/skills/upload", post(upload_skill))
        .route("/skills", get(list_skills))
        .route("/skills/install", post(install_skill))
        .route("/skills/registry", get(list_installed_skills))
        // Outbound webhooks
        .route("/webhooks", post(create_webhook))
        .route("/webhooks", get(list_webhooks))
        .route("/webhooks/:id", delete(delete_webhook))
        // Audit log
        .route("/audit", get(query_audit_log))
        // Reviews
        .route("/reviews", get(list_reviews))
        .route("/reviews/resolve-all", post(resolve_all_reviews))
        .route("/reviews/:id/resolve", post(resolve_review))
        // Auto-approvals
        .route("/auto-approvals", get(list_auto_approvals))
        .route("/auto-approvals", post(create_auto_approval))
        .route("/auto-approvals/:rule_id", delete(delete_auto_approval))
        // Swarm
        .route("/swarm/status", get(swarm_status))
        // ── Connector install & management ────────────────────────────────
        // Install API-key connectors
        .route("/connectors", get(oauth::list_connectors))
        .route("/connectors/:type/install", post(oauth::install_connector))
        .route("/connectors/:type/webhook-install", post(oauth::install_webhook_connector))
        .route("/connectors/:type", delete(oauth::uninstall_connector))
        // Inbound connector webhooks (kept for users who prefer push over poll)
        .route("/connectors/:type/webhook", post(connector_inbound))
        // ── Billing ───────────────────────────────────────────────────────
        .route("/billing/checkout", post(billing_routes::create_checkout))
        .route("/billing/subscription", get(billing_routes::get_subscription))
        .route("/billing/subscription/cancel", post(billing_routes::cancel_subscription))
        .route("/billing/invoices", get(billing_routes::list_invoices))
        .route("/billing/credits", get(billing_routes::get_credits))
        .route("/billing/credits/purchase", post(billing_routes::purchase_credits))
        .route_layer(middleware::from_fn_with_state(auth_state, auth_middleware));

    // SSE stream — auth extracted inside handler
    let stream_routes = Router::new().route("/agents/:id/stream", get(agent_stream)).with_state(stream_state);

    // Admin routes — separate token
    let admin_state = AdminState { store: store.clone(), tenant_store, cost_tracker, audit_log, metrics };
    let admin_token = std::env::var("NARAYAN_ADMIN_TOKEN").unwrap_or_default();
    let admin = Router::new()
        .route("/admin/info", get(admin_routes::system_info))
        .route("/admin/health/ready", get(admin_routes::readiness))
        .route("/admin/health/live", get(admin_routes::liveness))
        .route("/admin/metrics", get(admin_routes::admin_metrics))
        .route("/admin/tenants", get(admin_routes::list_tenants))
        .route("/admin/tenants/:id/suspend", post(admin_routes::suspend_tenant))
        .route("/admin/tenants/:id/activate", post(admin_routes::activate_tenant))
        .route("/admin/tenants/:id/plan", put(admin_routes::change_plan))
        .route("/admin/spend", get(admin_routes::spend_report))
        .route("/admin/audit", get(admin_routes::admin_audit))
        .route_layer(middleware::from_fn_with_state(AdminAuthState { admin_token }, admin_auth_middleware))
        .with_state(admin_state);

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(stream_routes)
        .merge(admin)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn serve(
    state: AppState,
    tenant_store: Arc<TenantStore>,
    event_bus: Arc<EventBus>,
    store: Arc<PostgresStore>,
    audit_log: Arc<AuditLog>,
    cost_tracker: Arc<CostTracker>,
    metrics: Arc<Metrics>,
    host: &str,
    port: u16,
) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    let router = build_router(state, tenant_store, event_bus, store, audit_log, cost_tracker, metrics);
    tracing::info!("API listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
