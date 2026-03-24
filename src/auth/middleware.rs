use axum::{
    extract::{FromRequestParts, Request},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

use crate::tenant::model::{AuthenticatedTenant, TenantPlan};

// ── Shared auth state injected into Axum ──────────────────────────────────

#[derive(Clone)]
pub struct AuthState {
    pub jwt_secret: String,
}

// ── Axum extractor — use in handlers as `Extension<AuthenticatedTenant>` ──

#[axum::async_trait]
impl<S> FromRequestParts<S> for AuthenticatedTenant
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<AuthenticatedTenant>().cloned().ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "not authenticated" }))).into_response()
        })
    }
}

// ── Axum middleware function ───────────────────────────────────────────────

/// Validates every incoming request.
/// Accepts:
///   - `Authorization: Bearer nar_<key>` — API key auth
///   - `Authorization: Bearer <jwt>`     — JWT session auth
///
/// Injects `AuthenticatedTenant` into request extensions on success.
pub async fn auth_middleware(
    axum::extract::State(auth): axum::extract::State<AuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    // Extract Bearer token
    let token = match request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        Some(t) => t.to_string(),
        None => {
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "missing Authorization header" })))
                .into_response();
        }
    };

    let authenticated = validate_jwt(&token, &auth.jwt_secret);

    match authenticated {
        Ok(tenant) => {
            request.extensions_mut().insert(tenant);
            next.run(request).await
        }
        Err(e) => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

fn validate_jwt(token: &str, secret: &str) -> Result<AuthenticatedTenant, anyhow::Error> {
    let claims = crate::auth::jwt::validate_token(token, secret)?;

    let plan = match claims.plan.as_str() {
        "go" => TenantPlan::Go,
        "pro" => TenantPlan::Pro,
        "enterprise" => TenantPlan::Enterprise,
        _ => TenantPlan::Free,
    };

    Ok(AuthenticatedTenant { tenant_id: claims.sub, plan })
}
