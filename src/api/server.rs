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
        .route("/auth/oauth/{provider}/start", get(oauth::oauth_start))
        // OAuth callback is public — provider redirects to it with no auth headers
        .route("/auth/oauth/{provider}/callback", get(oauth::oauth_callback))
        // Billing webhooks are public — signed by provider, verified inside handler
        .route("/billing/webhooks/{provider}", post(billing_routes::handle_webhook));

    // ── Protected routes (tenant JWT) ─────────────────────────────────────
    let protected = Router::new()
        // Metrics & costs
        .route("/metrics", get(get_metrics))
        .route("/costs", get(get_costs))
        // Provider catalog
        .route("/providers", get(list_providers))
        // Credentials & routing
        .route("/credentials", put(set_credential))
        .route("/credentials", get(list_credentials))
        .route("/credentials/{provider}", delete(delete_credential))
        .route("/routing", put(update_routing))
        // Goals & agents
        .route("/goals", post(create_goal))
        // Conversations
        .route("/conversations", get(list_conversations))
        .route("/conversations/{id}", get(get_conversation))
        .route("/agents", get(list_agents))
        .route("/agents/{id}", get(get_agent))
        .route("/agents/{id}/logs", get(get_agent_logs))
        .route("/agents/{id}/workspace/files", get(list_workspace_files))
        .route("/agents/{id}/workspace/tree", get(list_workspace_files))
        .route("/agents/{id}/workspace/files/{*path}", get(read_workspace_file))
        .route("/agents/{id}/workspace/download/{*path}", get(download_workspace_file))
        .route("/agents/{id}/workspace/files.tar.zst", get(download_workspace_bundle))
        .route("/agents/{id}/children", get(list_agent_children))
        .route("/agents/{id}/messages", get(list_agent_messages))
        .route("/agents/{id}/messages/{message_id}", get(get_agent_message))
        .route("/agents/{id}/messages/{message_id}/ack", post(ack_agent_message))
        .route("/agents/{id}/children/{child_id}/continue", post(continue_agent_child))
        .route("/agents/{id}/approve-plan", post(approve_plan))
        .route("/agents/{id}/pause", post(pause_agent))
        .route("/agents/{id}/resume", post(resume_agent))
        .route("/agents/{id}/cancel", post(cancel_agent))
        .route("/agents/{id}/clarify", post(submit_clarification))
        .route("/agents/{id}/replay", get(replay_agent))
        .route("/agents/{id}/citations", get(list_agent_citations))
        // ── Agent definitions (multi-role agents) ────────────────────────
        .route("/savings", get(get_savings_summary))
        .route("/goal-instances/{id}", get(get_goal_instance_detail))
        .route("/agent-definitions", get(list_agent_definitions))
        .route("/agent-definitions/{id}", get(get_agent_definition))
        .route("/agent-definitions/{id}/summary", get(agent_definition_summary))
        .route("/agent-definitions/{id}", put(update_agent_definition))
        .route("/agent-definitions/{id}", delete(delete_agent_definition))
        .route("/agent-definitions/{id}/chat", post(agent_chat))
        .route("/agent-definitions/{id}/summary.pdf", get(export_agent_summary_pdf))
        // Roles
        .route("/agent-definitions/{id}/roles", get(list_agent_roles))
        .route("/agent-definitions/{id}/roles", post(create_agent_role))
        .route("/agent-definitions/{agent_id}/roles/{role_id}", put(update_agent_role))
        .route("/agent-definitions/{agent_id}/roles/{role_id}", delete(delete_agent_role))
        // Role chat — discuss and modify a role conversationally
        .route("/roles/{role_id}/chat", post(start_role_chat))
        .route("/roles/{role_id}/chat/{session_id}/turn", post(role_chat_turn))
        .route("/roles/{role_id}/chat/{session_id}/apply", post(role_chat_apply))
        // Goal instances
        .route("/agent-definitions/{id}/goal-instances", get(list_goal_instances))
        .route("/agent-definitions/{agent_id}/roles/{role_id}/goal-instances", get(list_role_goal_instances))
        .route("/agent-definitions/{agent_id}/roles/{role_id}/trigger", post(trigger_role))
        // Plan mode
        .route("/plan-mode/templates", get(list_plan_mode_templates))
        .route("/plan-mode/sessions", post(start_plan_mode_session))
        .route("/plan-mode/sessions/{id}", get(get_plan_mode_session))
        .route("/plan-mode/sessions/{id}/turn", post(plan_mode_turn))
        .route("/plan-mode/sessions/{id}/test", post(test_plan_mode_session))
        .route("/plan-mode/sessions/{id}/revise", post(revise_plan_mode_session))
        .route("/plan-mode/sessions/{id}/save", post(save_plan_mode_session))
        // Tenant connectors (custom)
        .route("/tenant-connectors", get(list_tenant_connectors))
        .route("/tenant-connectors/{name}", delete(delete_tenant_connector))
        // Tenant-specific WASM tools
        .route("/tenant-wasm-tools", get(list_tenant_wasm_tools))
        .route("/tenant-wasm-tools", post(register_tenant_wasm_tool))
        .route("/tenant-wasm-tools/runs", get(list_tenant_wasm_tool_runs))
        .route("/tenant-wasm-tools/{name}/enabled", put(set_tenant_wasm_tool_enabled))
        .route("/tenant-wasm-tools/{name}", delete(delete_tenant_wasm_tool))
        // Custom connections — MCP server, REST API, database
        .route("/connections/mcp/test", post(test_mcp_connection))
        .route("/connections/mcp", post(register_mcp_connection))
        .route("/connections/api/test", post(test_api_connection))
        .route("/connections/api", post(register_api_connection))
        .route("/connections/db/test", post(test_db_connection))
        .route("/connections/db", post(register_db_connection))
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
        .route("/webhooks/{id}", delete(delete_webhook))
        // Audit log
        .route("/audit", get(query_audit_log))
        // Reviews
        .route("/reviews", get(list_reviews))
        .route("/reviews/resolve-all", post(resolve_all_reviews))
        .route("/reviews/{id}/resolve", post(resolve_review))
        // Auto-approvals
        .route("/auto-approvals", get(list_auto_approvals))
        .route("/auto-approvals", post(create_auto_approval))
        .route("/auto-approvals/{rule_id}", delete(delete_auto_approval))
        // Swarm
        .route("/swarm/status", get(swarm_status))
        // ── Connector install & management ────────────────────────────────
        // Install API-key connectors
        .route("/connectors", get(oauth::list_connectors))
        .route("/connectors/{type}/install", post(oauth::install_connector))
        .route("/connectors/{type}/webhook-install", post(oauth::install_webhook_connector))
        .route("/connectors/{type}/validate", post(oauth::validate_connector))
        .route("/connectors/{type}", delete(oauth::uninstall_connector))
        // Inbound connector webhooks (kept for users who prefer push over poll)
        .route("/connectors/{type}/webhook", post(connector_inbound))
        // ── Billing ───────────────────────────────────────────────────────
        .route("/billing/checkout", post(billing_routes::create_checkout))
        .route("/billing/subscription", get(billing_routes::get_subscription))
        .route("/billing/subscription/cancel", post(billing_routes::cancel_subscription))
        .route("/billing/invoices", get(billing_routes::list_invoices))
        .route("/billing/credits", get(billing_routes::get_credits))
        .route("/billing/credits/purchase", post(billing_routes::purchase_credits))
        .route_layer(middleware::from_fn_with_state(auth_state.clone(), auth_middleware));

    // SSE stream — auth extracted inside handler
    let stream_routes = Router::new()
        .route("/agents/{id}/stream", get(agent_stream))
        .route_layer(middleware::from_fn_with_state(auth_state.clone(), auth_middleware))
        .with_state(stream_state);

    // Admin routes — separate token
    let admin_state = AdminState { store: store.clone(), tenant_store, cost_tracker, audit_log, metrics };
    let admin_token = std::env::var("NARAYAN_ADMIN_TOKEN").unwrap_or_default();
    let admin = Router::new()
        .route("/admin/info", get(admin_routes::system_info))
        .route("/admin/health/ready", get(admin_routes::readiness))
        .route("/admin/health/live", get(admin_routes::liveness))
        .route("/admin/metrics", get(admin_routes::admin_metrics))
        .route("/admin/tenants", get(admin_routes::list_tenants))
        .route("/admin/tenants/{id}/suspend", post(admin_routes::suspend_tenant))
        .route("/admin/tenants/{id}/activate", post(admin_routes::activate_tenant))
        .route("/admin/tenants/{id}/plan", put(admin_routes::change_plan))
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
