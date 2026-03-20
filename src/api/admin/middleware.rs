//! Admin authentication middleware — validates the NARAYAN_ADMIN_TOKEN header.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

#[derive(Clone)]
pub struct AdminAuthState {
    pub admin_token: String,
}

pub async fn admin_auth_middleware(
    State(state): State<AdminAuthState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get("X-Admin-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    if token.is_empty() || token != state.admin_token {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid or missing admin token" })),
        )
            .into_response();
    }

    next.run(req).await
}
