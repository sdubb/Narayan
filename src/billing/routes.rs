//! Billing API routes.
//!
//! POST /billing/checkout              — create a checkout session (redirect URL)
//! GET  /billing/subscription          — current subscription for this tenant
//! POST /billing/subscription/cancel   — cancel current subscription
//! GET  /billing/invoices              — list invoices for this tenant
//! POST /billing/webhooks/:provider    — inbound webhook from PayPal / Stripe / etc.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::{
    api::routes::AppState,
    billing::{BillingPlan, BillingStore},
    tenant::model::AuthenticatedTenant,
};

fn err(code: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

// ── POST /billing/checkout ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CheckoutRequest {
    pub plan: String,
    pub provider: Option<String>,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
}

pub async fn create_checkout(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<CheckoutRequest>,
) -> impl IntoResponse {
    let plan: BillingPlan = match body.plan.parse() {
        Ok(p) => p,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };

    let store = &state.billing;
    let provider = match body.provider.as_deref() {
        Some(name) => store.provider(name),
        None => store.default_provider(),
    };
    let provider = match provider {
        Some(p) => p,
        None => return err(StatusCode::SERVICE_UNAVAILABLE, "no billing provider configured"),
    };

    let base = std::env::var("NARAYAN_BASE_URL").unwrap_or_else(|_| "https://app.narayan.ai".into());
    let success_url = body.success_url.unwrap_or_else(|| format!("{}/billing/success", base));
    let cancel_url = body.cancel_url.unwrap_or_else(|| format!("{}/billing/cancel", base));

    match provider.create_checkout_session(&tenant.tenant_id, &plan, &success_url, &cancel_url).await {
        Ok(session) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id":    session.session_id,
                "provider":      session.provider,
                "redirect_url":  session.redirect_url,
                "plan":          session.plan.as_str(),
                "amount_usd":    session.amount_usd,
                "expires_at":    session.expires_at.to_rfc3339(),
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── GET /billing/subscription ─────────────────────────────────────────────

pub async fn get_subscription(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.billing.get_subscription_by_tenant(&tenant.tenant_id).await {
        Ok(Some(sub)) => Json(serde_json::json!({
            "id":                       sub.id,
            "provider":                 sub.provider,
            "provider_subscription_id": sub.provider_subscription_id,
            "plan":                     sub.plan.as_str(),
            "status":                   sub.status.to_string(),
            "current_period_start":     sub.current_period_start.to_rfc3339(),
            "current_period_end":       sub.current_period_end.to_rfc3339(),
        }))
        .into_response(),
        Ok(None) => Json(serde_json::json!({ "plan": "free", "status": "active" })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── POST /billing/subscription/cancel ────────────────────────────────────

pub async fn cancel_subscription(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    let sub = match state.billing.get_subscription_by_tenant(&tenant.tenant_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return err(StatusCode::NOT_FOUND, "no active subscription"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let provider = match state.billing.provider(&sub.provider) {
        Some(p) => p,
        None => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("provider {} not available", sub.provider)),
    };

    match provider.cancel_subscription(&sub.provider_subscription_id).await {
        Ok(()) => {
            let _ = state.billing.cancel_subscription_in_db(&sub.provider, &sub.provider_subscription_id).await;
            Json(serde_json::json!({ "cancelled": true })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── GET /billing/invoices ─────────────────────────────────────────────────

pub async fn list_invoices(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.billing.list_invoices(&tenant.tenant_id).await {
        Ok(invoices) => {
            let count = invoices.len();
            Json(serde_json::json!({ "invoices": invoices, "count": count })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── POST /billing/webhooks/:provider ─────────────────────────────────────

/// Receives raw webhook payload from PayPal, Stripe, etc.
/// The signature header name varies per provider:
///   PayPal:  PAYPAL-TRANSMISSION-SIG
///   Stripe:  Stripe-Signature
/// We read both and pass whichever is non-empty.
pub async fn handle_webhook(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Extract signature from provider-specific header
    let signature = headers
        .get("paypal-transmission-sig")
        .or_else(|| headers.get("stripe-signature"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    match state.billing.handle_webhook(&provider, &body, signature).await {
        Ok(event) => {
            tracing::info!(provider = %provider, event = ?event, "billing webhook processed");
            (StatusCode::OK, Json(serde_json::json!({ "received": true }))).into_response()
        }
        Err(e) => {
            tracing::error!(provider = %provider, error = %e, "billing webhook failed");
            // Always return 200 to prevent provider retries on our own errors,
            // unless it's a signature failure (400 to signal we reject it)
            if e.to_string().contains("signature") {
                err(StatusCode::BAD_REQUEST, e.to_string())
            } else {
                err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        }
    }
}

// ── POST /billing/credits/purchase ───────────────────────────────────────

/// Purchase a credit top-up pack ($8 = 5,000 steps).
/// Creates a one-time PayPal order (not a subscription).
pub async fn purchase_credits(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Json(body): Json<CheckoutRequest>,
) -> impl IntoResponse {
    let provider = match state.billing.default_provider() {
        Some(p) => p,
        None => return err(StatusCode::SERVICE_UNAVAILABLE, "no billing provider configured"),
    };

    let base = std::env::var("NARAYAN_BASE_URL").unwrap_or_else(|_| "https://app.narayan.ai".into());
    let success_url = format!("{}/billing/credits/success?tenant={}", base, tenant.tenant_id);
    let cancel_url = format!("{}/billing/credits/cancel", base);

    // We reuse BillingPlan::Go as a sentinel for "credit pack" checkout —
    // the provider creates a $8 one-time order. On PaymentSucceeded webhook
    // the CreditsPurchased event adds 5,000 steps.
    use crate::billing::provider::BillingPlan;
    let pseudo_plan = BillingPlan::Go; // provider reads amount from plan.credit_pack_price_usd()

    match provider.create_checkout_session(&tenant.tenant_id, &pseudo_plan, &success_url, &cancel_url).await {
        Ok(session) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id":   session.session_id,
                "redirect_url": session.redirect_url,
                "steps":        BillingPlan::credit_pack_steps(),
                "amount_usd":   BillingPlan::credit_pack_price_usd(),
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── GET /billing/credits ──────────────────────────────────────────────────

pub async fn get_credits(State(state): State<AppState>, tenant: AuthenticatedTenant) -> impl IntoResponse {
    match state.billing.get_extra_steps(&tenant.tenant_id).await {
        Ok(steps) => Json(serde_json::json!({
            "tenant_id":   tenant.tenant_id,
            "extra_steps": steps,
            "pack_price_usd": crate::billing::BillingPlan::credit_pack_price_usd(),
            "pack_steps":     crate::billing::BillingPlan::credit_pack_steps(),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
