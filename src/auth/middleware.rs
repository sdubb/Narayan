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
/// Team context (optional):
///   - Embedded in JWT `team_id` claim, OR
///   - Provided via `X-Narayan-Team-Id` header (for API key auth)
///
/// Note: team membership is NOT validated here to avoid a DB hit on every
/// request. Route handlers that require team access call TeamStore::assert_member_role.
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

    // Extract optional team context from header (used when token has no team claim)
    let header_team_id = request
        .headers()
        .get("X-Narayan-Team-Id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let authenticated = validate_jwt(&token, &auth.jwt_secret, header_team_id);

    match authenticated {
        Ok(tenant) => {
            request.extensions_mut().insert(tenant);
            next.run(request).await
        }
        Err(e) => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

fn validate_jwt(
    token: &str,
    secret: &str,
    header_team_id: Option<String>,
) -> Result<AuthenticatedTenant, anyhow::Error> {
    let claims = crate::auth::jwt::validate_token(token, secret)?;

    let plan = match claims.plan.as_str() {
        "go" => TenantPlan::Go,
        "pro" => TenantPlan::Pro,
        "enterprise" => TenantPlan::Enterprise,
        _ => TenantPlan::Free,
    };

    // Team context: prefer JWT claim (signed) over header
    let team_id = claims.team_id.or(header_team_id);

    Ok(AuthenticatedTenant {
        tenant_id: claims.sub,
        plan,
        team_id,
        team_role: None, // populated lazily by handlers that require team access
    })
}
