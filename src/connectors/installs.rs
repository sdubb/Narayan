//! Connector install store — per-tenant OAuth tokens and API keys.
//!
//! Two auth paths:
//!   OAuth:    GET /auth/oauth/:provider/start → consent → /callback → token stored
//!   API key:  POST /connectors/:type/install  { "api_key": "..." }
//!
//! Stored tokens are AES-256-GCM encrypted (same key as credential store).
//! The mcp_session tool auto-injects the stored token when server_url matches.
//! The connector_inbound handler loads the real ConnectorConfig from this store.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::tenant::encrypt_secret;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConnectorInstall {
    pub id:              String,
    pub tenant_id:       String,
    pub connector_type:  String,  // "github", "slack", "gmail", ...
    pub auth_type:       String,  // "oauth" | "api_key" | "webhook_only"
    /// Encrypted access token (OAuth) or API key.
    pub token_enc:       Option<String>,
    /// Encrypted refresh token (OAuth only).
    pub refresh_enc:     Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    /// JSON: connector-specific settings (e.g. GitHub org, Zendesk subdomain).
    pub settings:        serde_json::Value,
    /// Webhook secret for inbound connectors (encrypted).
    pub webhook_secret_enc: Option<String>,
    pub enabled:         bool,
    pub last_polled_at:  Option<DateTime<Utc>>,
    pub created_at:      DateTime<Utc>,
    pub updated_at:      DateTime<Utc>,
}

/// What we return to the frontend — no encrypted values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorInstallView {
    pub id:              String,
    pub connector_type:  String,
    pub auth_type:       String,
    pub connected:       bool,
    pub settings:        serde_json::Value,
    pub last_polled_at:  Option<DateTime<Utc>>,
    pub created_at:      DateTime<Utc>,
}

impl From<&ConnectorInstall> for ConnectorInstallView {
    fn from(c: &ConnectorInstall) -> Self {
        Self {
            id:             c.id.clone(),
            connector_type: c.connector_type.clone(),
            auth_type:      c.auth_type.clone(),
            connected:      c.token_enc.is_some() || c.webhook_secret_enc.is_some(),
            settings:       c.settings.clone(),
            last_polled_at: c.last_polled_at,
            created_at:     c.created_at,
        }
    }
}

pub struct ConnectorInstallStore {
    pool:        PgPool,
    encrypt_key: String,
}

impl ConnectorInstallStore {
    pub fn new(pool: PgPool, encrypt_key: String) -> Self {
        Self { pool, encrypt_key }
    }

    /// Expose the raw pool for the poller (read-only queries).
    pub fn pool(&self) -> &PgPool { &self.pool }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS connector_installs (
                id                  TEXT PRIMARY KEY,
                tenant_id           TEXT NOT NULL,
                connector_type      TEXT NOT NULL,
                auth_type           TEXT NOT NULL DEFAULT 'api_key',
                token_enc           TEXT,
                refresh_enc         TEXT,
                token_expires_at    TIMESTAMPTZ,
                settings            JSONB NOT NULL DEFAULT '{}',
                webhook_secret_enc  TEXT,
                enabled             BOOLEAN NOT NULL DEFAULT true,
                last_polled_at      TIMESTAMPTZ,
                created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE (tenant_id, connector_type)
            );
            CREATE INDEX IF NOT EXISTS connector_installs_tenant ON connector_installs (tenant_id);
            CREATE INDEX IF NOT EXISTS connector_installs_poll
                ON connector_installs (last_polled_at)
                WHERE enabled = true;

            -- OAuth state table: prevents CSRF during OAuth flow
            CREATE TABLE IF NOT EXISTS oauth_states (
                state      TEXT PRIMARY KEY,
                tenant_id  TEXT NOT NULL,
                provider   TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
        "#).execute(&self.pool).await?;
        Ok(())
    }

    // ── Install / update ──────────────────────────────────────────────────

    pub async fn upsert_api_key(
        &self,
        tenant_id:      &str,
        connector_type: &str,
        api_key:        &str,
        settings:       serde_json::Value,
    ) -> Result<String> {
        let id        = crate::util::new_id();
        let token_enc = encrypt_secret(api_key, &self.encrypt_key);
        sqlx::query!(
            r#"INSERT INTO connector_installs
                   (id, tenant_id, connector_type, auth_type, token_enc, settings)
               VALUES ($1,$2,$3,'api_key',$4,$5)
               ON CONFLICT (tenant_id, connector_type) DO UPDATE SET
                   token_enc  = EXCLUDED.token_enc,
                   settings   = EXCLUDED.settings,
                   enabled    = true,
                   updated_at = NOW()"#,
            id, tenant_id, connector_type, token_enc, settings
        ).execute(&self.pool).await?;
        Ok(id)
    }

    pub async fn upsert_oauth_token(
        &self,
        tenant_id:       &str,
        connector_type:  &str,
        access_token:    &str,
        refresh_token:   Option<&str>,
        expires_at:      Option<DateTime<Utc>>,
        settings:        serde_json::Value,
    ) -> Result<String> {
        let id          = crate::util::new_id();
        let token_enc   = encrypt_secret(access_token, &self.encrypt_key);
        let refresh_enc = refresh_token.map(|t| encrypt_secret(t, &self.encrypt_key));
        sqlx::query!(
            r#"INSERT INTO connector_installs
                   (id, tenant_id, connector_type, auth_type, token_enc, refresh_enc, token_expires_at, settings)
               VALUES ($1,$2,$3,'oauth',$4,$5,$6,$7)
               ON CONFLICT (tenant_id, connector_type) DO UPDATE SET
                   token_enc        = EXCLUDED.token_enc,
                   refresh_enc      = EXCLUDED.refresh_enc,
                   token_expires_at = EXCLUDED.token_expires_at,
                   settings         = EXCLUDED.settings,
                   enabled          = true,
                   updated_at       = NOW()"#,
            id, tenant_id, connector_type, token_enc, refresh_enc, expires_at, settings
        ).execute(&self.pool).await?;
        Ok(id)
    }

    pub async fn upsert_webhook_only(
        &self,
        tenant_id:       &str,
        connector_type:  &str,
        webhook_secret:  &str,
        settings:        serde_json::Value,
    ) -> Result<(String, String)> {
        let id              = crate::util::new_id();
        let webhook_enc     = encrypt_secret(webhook_secret, &self.encrypt_key);
        sqlx::query!(
            r#"INSERT INTO connector_installs
                   (id, tenant_id, connector_type, auth_type, webhook_secret_enc, settings)
               VALUES ($1,$2,$3,'webhook_only',$4,$5)
               ON CONFLICT (tenant_id, connector_type) DO UPDATE SET
                   webhook_secret_enc = EXCLUDED.webhook_secret_enc,
                   settings           = EXCLUDED.settings,
                   enabled            = true,
                   updated_at         = NOW()"#,
            id, tenant_id, connector_type, webhook_enc, settings
        ).execute(&self.pool).await?;
        Ok((id, webhook_secret.to_string()))
    }

    // ── Read ──────────────────────────────────────────────────────────────

    pub async fn get(&self, tenant_id: &str, connector_type: &str) -> Result<Option<ConnectorInstall>> {
        let row = sqlx::query_as!(ConnectorInstall,
            "SELECT id, tenant_id, connector_type, auth_type, token_enc, refresh_enc,
                    token_expires_at, settings, webhook_secret_enc, enabled, last_polled_at,
                    created_at, updated_at
               FROM connector_installs
              WHERE tenant_id=$1 AND connector_type=$2 AND enabled=true",
            tenant_id, connector_type
        ).fetch_optional(&self.pool).await?;
        Ok(row)
    }

    pub async fn list_for_tenant(&self, tenant_id: &str) -> Result<Vec<ConnectorInstallView>> {
        let rows = sqlx::query_as!(ConnectorInstall,
            "SELECT id, tenant_id, connector_type, auth_type, token_enc, refresh_enc,
                    token_expires_at, settings, webhook_secret_enc, enabled, last_polled_at,
                    created_at, updated_at
               FROM connector_installs WHERE tenant_id=$1 ORDER BY connector_type",
            tenant_id
        ).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(ConnectorInstallView::from).collect())
    }

    /// Decrypt and return the access token / API key. None if not installed.
    pub fn decrypt_token(&self, install: &ConnectorInstall) -> Option<String> {
        let enc = install.token_enc.as_ref()?;
        crate::tenant::decrypt_secret(enc, &self.encrypt_key).ok()
    }

    pub fn decrypt_webhook_secret(&self, install: &ConnectorInstall) -> Option<String> {
        let enc = install.webhook_secret_enc.as_ref()?;
        crate::tenant::decrypt_secret(enc, &self.encrypt_key).ok()
    }

    pub async fn delete(&self, tenant_id: &str, connector_type: &str) -> Result<bool> {
        let r = sqlx::query!(
            "UPDATE connector_installs SET enabled=false, updated_at=NOW()
              WHERE tenant_id=$1 AND connector_type=$2",
            tenant_id, connector_type
        ).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn update_last_polled(&self, tenant_id: &str, connector_type: &str) -> Result<()> {
        sqlx::query!(
            "UPDATE connector_installs SET last_polled_at=NOW()
              WHERE tenant_id=$1 AND connector_type=$2",
            tenant_id, connector_type
        ).execute(&self.pool).await?;
        Ok(())
    }

    // ── OAuth state (CSRF protection) ─────────────────────────────────────

    pub async fn save_oauth_state(&self, state: &str, tenant_id: &str, provider: &str) -> Result<()> {
        sqlx::query!(
            "INSERT INTO oauth_states (state, tenant_id, provider) VALUES ($1,$2,$3)
             ON CONFLICT (state) DO NOTHING",
            state, tenant_id, provider
        ).execute(&self.pool).await?;
        // Expire old states (>10 min)
        let _ = sqlx::query!(
            "DELETE FROM oauth_states WHERE created_at < NOW() - INTERVAL '10 minutes'"
        ).execute(&self.pool).await;
        Ok(())
    }

    pub async fn consume_oauth_state(&self, state: &str) -> Result<Option<(String, String)>> {
        let row = sqlx::query!(
            "DELETE FROM oauth_states WHERE state=$1 AND created_at > NOW() - INTERVAL '10 minutes'
             RETURNING tenant_id, provider",
            state
        ).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| (r.tenant_id, r.provider)))
    }
}
