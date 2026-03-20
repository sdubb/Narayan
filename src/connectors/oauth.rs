//! OAuth flow for connector authentication.
//!
//! Routes:
//!   GET  /auth/oauth/:provider/start    — redirect user to provider consent page
//!   GET  /auth/oauth/:provider/callback — exchange code, store token, redirect to UI
//!
//! Supported OAuth providers and their scopes:
//!   slack      — channels:read, chat:write, users:read
//!   gmail      — gmail.readonly, gmail.send
//!   outlook    — Mail.Read, Mail.Send (Microsoft Graph)
//!   google     — drive.readonly, spreadsheets, documents (Google)
//!   salesforce — api, refresh_token
//!   hubspot    — crm.objects.contacts.read, oauth
//!   jira       — read:jira-work, write:jira-work
//!   notion     — (Notion's own OAuth)
//!   github     — repo, issues, pull_requests

use std::sync::Arc;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    Json,
};
use serde::Deserialize;

use crate::api::routes::AppState;
use crate::tenant::model::AuthenticatedTenant;

fn err(code: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

/// OAuth provider configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id:     String,
    pub client_secret: String,
    pub auth_url:      String,
    pub token_url:     String,
    pub scopes:        Vec<String>,
    pub connector_type: String,
}

impl OAuthConfig {
    pub fn from_env(provider: &str) -> Option<Self> {
        let prefix = provider.to_uppercase().replace('-', "_");
        let client_id     = std::env::var(format!("{prefix}_CLIENT_ID")).ok()?;
        let client_secret = std::env::var(format!("{prefix}_CLIENT_SECRET")).ok()?;

        let (auth_url, token_url, scopes, connector_type): (String, String, Vec<&str>, &str) = match provider {
            "slack" => (
                "https://slack.com/oauth/v2/authorize".into(),
                "https://slack.com/api/oauth.v2.access".into(),
                vec!["channels:read", "chat:write", "users:read", "files:read"],
                "slack",
            ),
            "gmail" | "google" => (
                "https://accounts.google.com/o/oauth2/v2/auth".into(),
                "https://oauth2.googleapis.com/token".into(),
                vec!["https://www.googleapis.com/auth/gmail.readonly",
                     "https://www.googleapis.com/auth/gmail.send",
                     "https://www.googleapis.com/auth/drive.readonly",
                     "https://www.googleapis.com/auth/spreadsheets",
                     "https://www.googleapis.com/auth/documents"],
                "google",
            ),
            "outlook" | "microsoft" => (
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
                "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
                vec!["Mail.Read", "Mail.Send", "Calendars.Read", "User.Read", "offline_access",
                     "https://graph.microsoft.com/Chat.Read",
                     "https://graph.microsoft.com/ChannelMessage.Send"],
                "microsoft",
            ),
            "salesforce" => (
                "https://login.salesforce.com/services/oauth2/authorize".into(),
                "https://login.salesforce.com/services/oauth2/token".into(),
                vec!["api", "refresh_token", "offline_access"],
                "salesforce",
            ),
            "hubspot" => (
                "https://app.hubspot.com/oauth/authorize".into(),
                "https://api.hubapi.com/oauth/v1/token".into(),
                vec!["crm.objects.contacts.read", "crm.objects.deals.read", "oauth"],
                "hubspot",
            ),
            "jira" | "atlassian" => (
                "https://auth.atlassian.com/authorize".into(),
                "https://auth.atlassian.com/oauth/token".into(),
                vec!["read:jira-work", "write:jira-work", "read:confluence-content.all",
                     "write:confluence-content", "offline_access"],
                "atlassian",
            ),
            "notion" => (
                "https://api.notion.com/v1/oauth/authorize".into(),
                "https://api.notion.com/v1/oauth/token".into(),
                vec![],  // Notion doesn't use scope params
                "notion",
            ),
            "github" => (
                "https://github.com/login/oauth/authorize".into(),
                "https://github.com/login/oauth/access_token".into(),
                vec!["repo", "issues", "pull_requests", "read:user"],
                "github",
            ),
            "quickbooks" => (
                "https://appcenter.intuit.com/connect/oauth2".into(),
                "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer".into(),
                vec!["com.intuit.quickbooks.accounting", "offline_access"],
                "quickbooks",
            ),
            "docusign" => (
                "https://account.docusign.com/oauth/auth".into(),
                "https://account.docusign.com/oauth/token".into(),
                vec!["signature", "extended", "impersonation"],
                "docusign",
            ),
            _ => return None,
        };

        Some(OAuthConfig {
            client_id,
            client_secret,
            auth_url,
            token_url,
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            connector_type: connector_type.to_string(),
        })
    }
}

/// GET /auth/oauth/:provider/start?token=<jwt>
/// Public route — browser redirect can't send Authorization header.
/// Validates the JWT from the `token` query param, then redirects to provider consent.
pub async fn oauth_start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(qparams): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    // Validate JWT from query param
    let token = match qparams.get("token") {
        Some(t) => t.clone(),
        None    => return err(StatusCode::UNAUTHORIZED, "missing ?token= query param").into_response(),
    };
    let tenant_id = match crate::auth::jwt::validate_token(&token, &state.jwt_secret) {
        Ok(claims) => claims.sub,
        Err(_)     => return err(StatusCode::UNAUTHORIZED, "invalid or expired token").into_response(),
    };

    let cfg = match OAuthConfig::from_env(&provider) {
        Some(c) => c,
        None    => return err(StatusCode::NOT_FOUND,
            format!("OAuth not configured for '{provider}'. Set {}_CLIENT_ID and {}_CLIENT_SECRET env vars.",
                provider.to_uppercase(), provider.to_uppercase())).into_response(),
    };

    let csrf_state = crate::util::new_id();
    if let Err(e) = state.connector_installs.save_oauth_state(&csrf_state, &tenant_id, &provider).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to save OAuth state: {e}")).into_response();
    }

    let base         = std::env::var("NARAYAN_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
    let redirect_uri = format!("{}/auth/oauth/{}/callback", base, provider);
    let scope_str    = cfg.scopes.join(" ");

    let mut params = vec![
        ("client_id",     cfg.client_id.clone()),
        ("redirect_uri",  redirect_uri.clone()),
        ("response_type", "code".to_string()),
        ("state",         csrf_state.clone()),
    ];
    if !scope_str.is_empty() {
        params.push(("scope", scope_str));
    }
    match provider.as_str() {
        "google" | "gmail"   => { params.push(("access_type", "offline".into())); params.push(("prompt", "consent".into())); }
        "jira" | "atlassian" => { params.push(("audience", "api.atlassian.com".into())); params.push(("prompt", "consent".into())); }
        _ => {}
    }

    let url = format!("{}?{}", cfg.auth_url,
        params.iter().map(|(k,v)| format!("{}={}", k, urlencoding::encode(v))).collect::<Vec<_>>().join("&")
    );
    Redirect::temporary(&url).into_response()
}

#[derive(Deserialize)]
pub struct OAuthCallbackParams {
    pub code:  Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// GET /auth/oauth/:provider/callback
/// Receives the authorization code, exchanges it for tokens, stores them.
pub async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(params): Query<OAuthCallbackParams>,
) -> impl IntoResponse {
    let ui_base = std::env::var("NARAYAN_UI_URL").unwrap_or_else(|_| "http://localhost:5173".into());

    // Handle provider error
    if let Some(err_msg) = params.error {
        let url = format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&err_msg));
        return Redirect::temporary(&url).into_response();
    }

    let code  = match params.code  { Some(c) => c, None => return Redirect::temporary(&format!("{}/settings/connectors?error=no_code", ui_base)).into_response() };
    let state_token = match params.state { Some(s) => s, None => return Redirect::temporary(&format!("{}/settings/connectors?error=no_state", ui_base)).into_response() };

    // Validate CSRF state
    let (tenant_id, _) = match state.connector_installs.consume_oauth_state(&state_token).await {
        Ok(Some(r)) => r,
        _ => return Redirect::temporary(&format!("{}/settings/connectors?error=invalid_state", ui_base)).into_response(),
    };

    let cfg = match OAuthConfig::from_env(&provider) {
        Some(c) => c,
        None    => return Redirect::temporary(&format!("{}/settings/connectors?error=provider_not_configured", ui_base)).into_response(),
    };

    let narayan_base = std::env::var("NARAYAN_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
    let redirect_uri = format!("{}/auth/oauth/{}/callback", narayan_base, provider);

    // Exchange code for tokens.
    // Each provider has quirks:
    //   GitHub    — returns form-encoded by default; needs Accept: application/json
    //   Atlassian — needs JSON body not form-encoded
    //   Notion    — needs JSON body not form-encoded
    //   Others    — standard form + Basic auth
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().unwrap();

    let token_body: serde_json::Value = match provider.as_str() {
        "github" => {
            // GitHub returns form-encoded unless Accept: application/json is set
            let res = client
                .post(&cfg.token_url)
                .header("Accept", "application/json")
                .basic_auth(&cfg.client_id, Some(&cfg.client_secret))
                .form(&[
                    ("grant_type",   "authorization_code"),
                    ("code",         &code),
                    ("redirect_uri", &redirect_uri),
                ])
                .send().await;
            match res {
                Ok(r) => match r.json().await {
                    Ok(j) => j,
                    Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
                },
                Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
            }
        }
        "jira" | "atlassian" => {
            // Atlassian requires JSON body, not form-encoded
            let body = serde_json::json!({
                "grant_type":   "authorization_code",
                "client_id":    &cfg.client_id,
                "client_secret": &cfg.client_secret,
                "code":         &code,
                "redirect_uri": &redirect_uri,
            });
            let res = client.post(&cfg.token_url).json(&body).send().await;
            match res {
                Ok(r) => match r.json().await {
                    Ok(j) => j,
                    Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
                },
                Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
            }
        }
        "notion" => {
            // Notion requires JSON body with Basic auth header
            let body = serde_json::json!({
                "grant_type":   "authorization_code",
                "code":         &code,
                "redirect_uri": &redirect_uri,
            });
            let res = client
                .post(&cfg.token_url)
                .basic_auth(&cfg.client_id, Some(&cfg.client_secret))
                .json(&body)
                .send().await;
            match res {
                Ok(r) => match r.json().await {
                    Ok(j) => j,
                    Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
                },
                Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
            }
        }
        _ => {
            // Standard: form-encoded body with Basic auth (Slack, Google, Microsoft, Salesforce, HubSpot, etc.)
            let res = client
                .post(&cfg.token_url)
                .basic_auth(&cfg.client_id, Some(&cfg.client_secret))
                .form(&[
                    ("grant_type",   "authorization_code"),
                    ("code",         &code),
                    ("redirect_uri", &redirect_uri),
                ])
                .send().await;
            match res {
                Ok(r) => match r.json().await {
                    Ok(j) => j,
                    Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
                },
                Err(e) => return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response(),
            }
        }
    };

    let access_token  = match token_body["access_token"].as_str() {
        Some(t) => t.to_string(),
        None    => {
            let msg = token_body["error_description"].as_str().unwrap_or("no access_token").to_string();
            return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&msg))).into_response();
        }
    };
    let refresh_token = token_body["refresh_token"].as_str().map(String::from);
    let expires_in    = token_body["expires_in"].as_u64();
    let expires_at    = expires_in.map(|s| chrono::Utc::now() + chrono::Duration::seconds(s as i64));

    // Build settings from token response (e.g. store Slack team_id, Salesforce instance_url)
    let mut settings = serde_json::json!({});
    if let Some(tid) = token_body["team"]["id"].as_str() { settings["team_id"] = tid.into(); }
    if let Some(iurl) = token_body["instance_url"].as_str() { settings["instance_url"] = iurl.into(); }
    if let Some(bot) = token_body["bot_user_id"].as_str() { settings["bot_user_id"] = bot.into(); }

    if let Err(e) = state.connector_installs.upsert_oauth_token(
        &tenant_id, &cfg.connector_type,
        &access_token, refresh_token.as_deref(), expires_at, settings
    ).await {
        return Redirect::temporary(&format!("{}/settings/connectors?error={}", ui_base, urlencoding::encode(&e.to_string()))).into_response();
    }

    tracing::info!(tenant_id, connector = cfg.connector_type, "OAuth connector connected");
    Redirect::temporary(&format!("{}/settings/connectors?connected={}", ui_base, cfg.connector_type)).into_response()
}

// ── Install API routes ────────────────────────────────────────────────────

/// POST /connectors/:type/install — install an API-key connector.
pub async fn install_connector(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let api_key = match body["api_key"].as_str().or_else(|| body["token"].as_str()) {
        Some(k) => k.to_string(),
        None    => return err(StatusCode::BAD_REQUEST, "'api_key' or 'token' required"),
    };

    // Extract optional settings from body (e.g. zendesk_subdomain, servicenow_instance_url)
    let settings = body.get("settings").cloned().unwrap_or(serde_json::json!({}));

    match state.connector_installs.upsert_api_key(&tenant.tenant_id, &connector_type, &api_key, settings).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({
            "installed": true,
            "id":        id,
            "connector": connector_type,
        }))).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// POST /connectors/:type/webhook-install — generate webhook secret for inbound-only connectors.
pub async fn install_webhook_connector(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let base            = std::env::var("NARAYAN_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
    let webhook_url     = format!("{}/connectors/{}/webhook", base, connector_type);
    // Use provided secret or generate one
    let webhook_secret  = body["webhook_secret"].as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("nar_whsec_{}", &crate::util::new_id().replace('-', "")[..24]));
    let settings = body.get("settings").cloned().unwrap_or(serde_json::json!({}));

    match state.connector_installs.upsert_webhook_only(&tenant.tenant_id, &connector_type, &webhook_secret, settings).await {
        Ok((id, secret)) => (StatusCode::CREATED, Json(serde_json::json!({
            "installed":      true,
            "id":             id,
            "connector":      connector_type,
            "webhook_url":    webhook_url,
            "webhook_secret": secret,
            "note":           "Paste the webhook_url and webhook_secret into the external system's webhook settings.",
        }))).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// GET /connectors — list all installed connectors for this tenant.
pub async fn list_connectors(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
) -> impl IntoResponse {
    match state.connector_installs.list_for_tenant(&tenant.tenant_id).await {
        Ok(installs) => {
            let count = installs.len();
            Json(serde_json::json!({ "connectors": installs, "count": count })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// DELETE /connectors/:type — uninstall a connector.
pub async fn uninstall_connector(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
) -> impl IntoResponse {
    match state.connector_installs.delete(&tenant.tenant_id, &connector_type).await {
        Ok(true)  => Json(serde_json::json!({ "uninstalled": true })).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "connector not installed"),
        Err(e)    => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
