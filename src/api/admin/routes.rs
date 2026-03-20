//! Admin API endpoints — management plane for platform operators.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::{
    audit::{AuditAction, AuditLog, AuditQuery},
    gateway::CostTracker,
    metrics::Metrics,
    storage::PostgresStore,
    tenant::TenantStore,
};

#[derive(Clone)]
pub struct AdminState {
    pub store: Arc<PostgresStore>,
    pub tenant_store: Arc<TenantStore>,
    pub cost_tracker: Arc<CostTracker>,
    pub audit_log: Arc<AuditLog>,
    pub metrics: Arc<Metrics>,
}

fn err(code: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

// ── System info ──────────────────────────────────────────────────────────

/// GET /admin/info — system version and build info.
pub async fn system_info() -> impl IntoResponse {
    Json(serde_json::json!({
        "service": "narayan",
        "version": env!("CARGO_PKG_VERSION"),
        "edition": "2021",
    }))
}

/// GET /admin/health/ready — readiness probe (checks DB connectivity).
pub async fn readiness(State(state): State<AdminState>) -> impl IntoResponse {
    match state.store.health_check().await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "ready": true }))).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "ready": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /admin/health/live — liveness probe (always returns OK if the process is running).
pub async fn liveness() -> impl IntoResponse {
    Json(serde_json::json!({ "live": true }))
}

// ── Metrics ──────────────────────────────────────────────────────────────

/// GET /admin/metrics — full system metrics snapshot.
pub async fn admin_metrics(State(state): State<AdminState>) -> impl IntoResponse {
    let total_cost = state.cost_tracker.total_cost_usd().await;
    let metrics = state.metrics.snapshot();
    Json(serde_json::json!({
        "metrics": metrics,
        "total_cost_usd": total_cost,
    }))
}

// ── Tenant management ────────────────────────────────────────────────────

/// GET /admin/tenants — list all tenants.
pub async fn list_tenants(State(state): State<AdminState>) -> impl IntoResponse {
    match state.tenant_store.list_all().await {
        Ok(tenants) => {
            let body: Vec<serde_json::Value> = tenants.iter().map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "name": t.name,
                    "email": t.email,
                    "status": format!("{:?}", t.status).to_lowercase(),
                    "plan": format!("{:?}", t.plan).to_lowercase(),
                    "created_at": t.created_at.to_rfc3339(),
                })
            }).collect();
            Json(serde_json::json!({ "tenants": body, "count": body.len() })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /admin/tenants/:id/suspend — suspend a tenant.
pub async fn suspend_tenant(
    State(state): State<AdminState>,
    Path(tenant_id): Path<String>,
) -> impl IntoResponse {
    match state.tenant_store.suspend(&tenant_id).await {
        Ok(_) => {
            let _ = state.audit_log.append(
                &tenant_id, None,
                AuditAction::TenantSuspended,
                serde_json::json!({ "suspended_by": "admin" }),
                None,
            ).await;
            Json(serde_json::json!({ "suspended": true })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /admin/tenants/:id/activate — reactivate a suspended tenant.
pub async fn activate_tenant(
    State(state): State<AdminState>,
    Path(tenant_id): Path<String>,
) -> impl IntoResponse {
    match state.tenant_store.activate(&tenant_id).await {
        Ok(_) => Json(serde_json::json!({ "activated": true })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct ChangePlanRequest {
    pub plan: String,
}

/// PUT /admin/tenants/:id/plan — change a tenant's plan.
pub async fn change_plan(
    State(state): State<AdminState>,
    Path(tenant_id): Path<String>,
    Json(body): Json<ChangePlanRequest>,
) -> impl IntoResponse {
    let valid = matches!(body.plan.as_str(), "free" | "go" | "pro" | "enterprise");
    if !valid {
        return err(StatusCode::BAD_REQUEST, "plan must be one of: free, go, pro, enterprise");
    }

    match state.tenant_store.update_plan(&tenant_id, &body.plan).await {
        Ok(_) => {
            let _ = state.audit_log.append(
                &tenant_id, None,
                AuditAction::TenantPlanChanged,
                serde_json::json!({ "new_plan": body.plan }),
                None,
            ).await;
            Json(serde_json::json!({ "updated": true, "plan": body.plan })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── Spend reports ────────────────────────────────────────────────────────

/// GET /admin/spend — spend report across all tenants.
pub async fn spend_report(State(state): State<AdminState>) -> impl IntoResponse {
    let all = state.cost_tracker.all_usage().await;
    let total: f64 = all.iter().map(|u| u.total_cost_usd).sum();
    Json(serde_json::json!({
        "total_cost_usd": total,
        "agent_count": all.len(),
        "agents": all,
    }))
}

// ── Audit (cross-tenant) ─────────────────────────────────────────────────

/// GET /admin/audit — query audit log across all tenants.
pub async fn admin_audit(
    State(state): State<AdminState>,
    Query(params): Query<AuditQuery>,
) -> impl IntoResponse {
    match state.audit_log.query(&params).await {
        Ok(entries) => {
            let count = entries.len();
            Json(serde_json::json!({ "entries": entries, "count": count })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
