//! Webhook registration and storage.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// A registered webhook endpoint for a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    pub tenant_id: String,
    pub url: String,
    /// HMAC-SHA256 signing secret — used to sign payloads so the receiver can verify authenticity.
    pub secret: String,
    /// Which event types to deliver (empty = all events).
    pub events: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Number of consecutive delivery failures.
    pub failure_count: i32,
    /// Automatically disabled after this many consecutive failures.
    pub max_failures: i32,
}

/// Request body for creating/updating a webhook.
#[derive(Debug, Deserialize)]
pub struct WebhookCreateRequest {
    pub url: String,
    pub events: Vec<String>,
    pub secret: Option<String>,
}

pub struct WebhookStore {
    pool: PgPool,
}

impl WebhookStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS webhooks (
                id            TEXT PRIMARY KEY,
                tenant_id     TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                url           TEXT NOT NULL,
                secret        TEXT NOT NULL,
                events        JSONB NOT NULL DEFAULT '[]',
                enabled       BOOLEAN NOT NULL DEFAULT true,
                failure_count INT NOT NULL DEFAULT 0,
                max_failures  INT NOT NULL DEFAULT 10,
                created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            CREATE INDEX IF NOT EXISTS webhooks_tenant_id ON webhooks (tenant_id);

            CREATE TABLE IF NOT EXISTS webhook_deliveries (
                id          TEXT PRIMARY KEY,
                webhook_id  TEXT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
                event_type  TEXT NOT NULL,
                payload     JSONB NOT NULL,
                status_code INT,
                response    TEXT,
                attempt     INT NOT NULL DEFAULT 1,
                success     BOOLEAN NOT NULL DEFAULT false,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            CREATE INDEX IF NOT EXISTS webhook_deliveries_webhook_id ON webhook_deliveries (webhook_id);
        "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create(
        &self,
        tenant_id: &str,
        url: &str,
        secret: &str,
        events: &[String],
    ) -> Result<WebhookConfig> {
        let id = crate::util::new_id();
        let now = Utc::now();
        let events_json = serde_json::to_value(events)?;

        sqlx::query(
            "INSERT INTO webhooks (id, tenant_id, url, secret, events, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(url)
        .bind(secret)
        .bind(&events_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(WebhookConfig {
            id,
            tenant_id: tenant_id.to_string(),
            url: url.to_string(),
            secret: secret.to_string(),
            events: events.to_vec(),
            enabled: true,
            created_at: now,
            updated_at: now,
            failure_count: 0,
            max_failures: 10,
        })
    }

    pub async fn list_for_tenant(&self, tenant_id: &str) -> Result<Vec<WebhookConfig>> {
        let rows = sqlx::query_as::<_, WebhookRow>(
            "SELECT id, tenant_id, url, secret, events, enabled, failure_count, max_failures,
                    created_at, updated_at
             FROM webhooks WHERE tenant_id = $1 ORDER BY created_at",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_config).collect())
    }

    /// Get all enabled webhooks for a tenant that subscribe to a given event type.
    pub async fn get_matching(&self, tenant_id: &str, event_type: &str) -> Result<Vec<WebhookConfig>> {
        let all = self.list_for_tenant(tenant_id).await?;
        Ok(all
            .into_iter()
            .filter(|w| {
                w.enabled && (w.events.is_empty() || w.events.iter().any(|e| e == event_type))
            })
            .collect())
    }

    pub async fn delete(&self, tenant_id: &str, webhook_id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM webhooks WHERE id = $1 AND tenant_id = $2")
            .bind(webhook_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn increment_failure(&self, webhook_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE webhooks SET failure_count = failure_count + 1, updated_at = NOW()
             WHERE id = $1"
        )
        .bind(webhook_id)
        .execute(&self.pool)
        .await?;

        // Auto-disable if over max failures
        sqlx::query(
            "UPDATE webhooks SET enabled = false WHERE id = $1 AND failure_count >= max_failures"
        )
        .bind(webhook_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn reset_failure(&self, webhook_id: &str) -> Result<()> {
        sqlx::query("UPDATE webhooks SET failure_count = 0, updated_at = NOW() WHERE id = $1")
            .bind(webhook_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Record a delivery attempt.
    pub async fn record_delivery(
        &self,
        webhook_id: &str,
        event_type: &str,
        payload: &serde_json::Value,
        status_code: Option<i32>,
        response: Option<&str>,
        attempt: i32,
        success: bool,
    ) -> Result<()> {
        let id = crate::util::new_id();
        sqlx::query(
            "INSERT INTO webhook_deliveries (id, webhook_id, event_type, payload, status_code, response, attempt, success)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&id)
        .bind(webhook_id)
        .bind(event_type)
        .bind(payload)
        .bind(status_code)
        .bind(response)
        .bind(attempt)
        .bind(success)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct WebhookRow {
    id: String,
    tenant_id: String,
    url: String,
    secret: String,
    events: serde_json::Value,
    enabled: bool,
    failure_count: i32,
    max_failures: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn row_to_config(r: WebhookRow) -> WebhookConfig {
    let events: Vec<String> = serde_json::from_value(r.events).unwrap_or_default();
    WebhookConfig {
        id: r.id,
        tenant_id: r.tenant_id,
        url: r.url,
        secret: r.secret,
        events,
        enabled: r.enabled,
        failure_count: r.failure_count,
        max_failures: r.max_failures,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}
