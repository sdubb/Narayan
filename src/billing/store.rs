//! BillingStore — wraps DB + all registered providers.
//! Single entry point for all billing operations.

use std::{collections::HashMap, sync::Arc};

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::Row;

use crate::billing::provider::{BillingEvent, BillingPlan, BillingProvider};

// ── DB row types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SubscriptionRow {
    pub id: String,
    pub tenant_id: String,
    pub provider: String,
    pub provider_subscription_id: String,
    pub plan: String,
    pub status: String,
    pub current_period_start: DateTime<Utc>,
    pub current_period_end: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct InvoiceRow {
    pub id: String,
    pub tenant_id: String,
    pub provider: String,
    pub provider_inv_id: String,
    pub amount_usd: f64,
    pub status: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub pdf_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CreditRow {
    pub tenant_id: String,
    pub extra_steps: i64,
    pub updated_at: DateTime<Utc>,
}

// ── BillingStore ──────────────────────────────────────────────────────────

pub struct BillingStore {
    pool: PgPool,
    providers: HashMap<String, Arc<dyn BillingProvider>>,
    default: Option<String>,
}

impl BillingStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool, providers: HashMap::new(), default: None }
    }

    pub fn register(mut self, provider: Arc<dyn BillingProvider>) -> Self {
        let name = provider.name().to_string();
        if self.default.is_none() {
            self.default = Some(name.clone());
        }
        self.providers.insert(name, provider);
        self
    }

    pub fn provider(&self, name: &str) -> Option<Arc<dyn BillingProvider>> {
        self.providers.get(name).cloned()
    }

    pub fn default_provider(&self) -> Option<Arc<dyn BillingProvider>> {
        self.default.as_ref().and_then(|n| self.providers.get(n)).cloned()
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS subscriptions (
                id                       TEXT PRIMARY KEY,
                tenant_id                TEXT NOT NULL UNIQUE,
                provider                 TEXT NOT NULL,
                provider_subscription_id TEXT NOT NULL,
                plan                     TEXT NOT NULL DEFAULT 'free',
                status                   TEXT NOT NULL DEFAULT 'active',
                current_period_start     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                current_period_end       TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '30 days',
                created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS subscriptions_tenant ON subscriptions (tenant_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS subscriptions_provider_sub ON subscriptions (provider_subscription_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS invoices (
                id              TEXT PRIMARY KEY,
                tenant_id       TEXT NOT NULL,
                provider        TEXT NOT NULL,
                provider_inv_id TEXT NOT NULL,
                amount_usd      DOUBLE PRECISION NOT NULL DEFAULT 0,
                status          TEXT NOT NULL DEFAULT 'open',
                period_start    TIMESTAMPTZ NOT NULL,
                period_end      TIMESTAMPTZ NOT NULL,
                pdf_url         TEXT,
                created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS invoices_tenant ON invoices (tenant_id)").execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS billing_events (
                id           TEXT PRIMARY KEY,
                tenant_id    TEXT,
                provider     TEXT NOT NULL,
                event_type   TEXT NOT NULL,
                payload      JSONB NOT NULL DEFAULT '{}',
                processed    BOOLEAN NOT NULL DEFAULT false,
                error        TEXT,
                received_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                processed_at TIMESTAMPTZ
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS billing_events_unprocessed ON billing_events (processed) WHERE NOT processed",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_credits (
                tenant_id   TEXT PRIMARY KEY,
                extra_steps BIGINT NOT NULL DEFAULT 0,
                updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ── Subscriptions ─────────────────────────────────────────────────────

    pub async fn get_subscription_by_tenant(&self, tenant_id: &str) -> Result<Option<SubscriptionRow>> {
        let row = sqlx::query_as::<_, SubscriptionRow>(
            "SELECT id, tenant_id, provider, provider_subscription_id, plan, status,
                    current_period_start, current_period_end, created_at, updated_at
               FROM subscriptions WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn upsert_subscription(
        &self,
        tenant_id: &str,
        provider: &str,
        provider_subscription_id: &str,
        plan: &BillingPlan,
        status: &str,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<()> {
        let id = crate::util::new_id();
        sqlx::query(
            r#"INSERT INTO subscriptions
                   (id, tenant_id, provider, provider_subscription_id, plan, status,
                    current_period_start, current_period_end)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
               ON CONFLICT (tenant_id) DO UPDATE SET
                   provider=EXCLUDED.provider,
                   provider_subscription_id=EXCLUDED.provider_subscription_id,
                   plan=EXCLUDED.plan, status=EXCLUDED.status,
                   current_period_start=EXCLUDED.current_period_start,
                   current_period_end=EXCLUDED.current_period_end,
                   updated_at=NOW()"#,
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(provider)
        .bind(provider_subscription_id)
        .bind(plan.as_str())
        .bind(status)
        .bind(period_start)
        .bind(period_end)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn cancel_subscription_in_db(&self, provider: &str, provider_sub_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE subscriptions SET status='cancelled', updated_at=NOW()
              WHERE provider=$1 AND provider_subscription_id=$2",
        )
        .bind(provider)
        .bind(provider_sub_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Invoices ──────────────────────────────────────────────────────────

    pub async fn create_invoice(
        &self,
        tenant_id: &str,
        provider: &str,
        provider_inv_id: &str,
        amount_usd: f64,
        status: &str,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        pdf_url: Option<&str>,
    ) -> Result<()> {
        let id = crate::util::new_id();
        sqlx::query(
            "INSERT INTO invoices (id, tenant_id, provider, provider_inv_id, amount_usd, status, period_start, period_end, pdf_url)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)"
        ).bind(&id).bind(tenant_id).bind(provider).bind(provider_inv_id)
        .bind(amount_usd).bind(status).bind(period_start).bind(period_end).bind(pdf_url)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_invoices(&self, tenant_id: &str) -> Result<Vec<InvoiceRow>> {
        let rows = sqlx::query_as::<_, InvoiceRow>(
            "SELECT id, tenant_id, provider, provider_inv_id, amount_usd, status,
                    period_start, period_end, pdf_url, created_at
               FROM invoices WHERE tenant_id=$1 ORDER BY created_at DESC LIMIT 50",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ── Credits ───────────────────────────────────────────────────────────

    pub async fn get_extra_steps(&self, tenant_id: &str) -> Result<u64> {
        let row = sqlx::query("SELECT extra_steps FROM tenant_credits WHERE tenant_id=$1")
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .map(|r| {
                let extra_steps: i64 = r.get::<i64, _>("extra_steps");
                extra_steps.max(0) as u64
            })
            .unwrap_or(0))
    }

    pub async fn add_credits(&self, tenant_id: &str, steps: u64) -> Result<()> {
        let steps_i64 = steps as i64;
        sqlx::query(
            r#"INSERT INTO tenant_credits (tenant_id, extra_steps)
               VALUES ($1, $2)
               ON CONFLICT (tenant_id) DO UPDATE SET
                   extra_steps = tenant_credits.extra_steps + EXCLUDED.extra_steps,
                   updated_at  = NOW()"#,
        )
        .bind(tenant_id)
        .bind(steps_i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn deduct_credit_step(&self, tenant_id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE tenant_credits SET extra_steps = extra_steps - 1, updated_at=NOW()
              WHERE tenant_id=$1 AND extra_steps > 0",
        )
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Event log ─────────────────────────────────────────────────────────

    pub async fn record_event(
        &self,
        provider: &str,
        event_type: &str,
        tenant_id: Option<&str>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let id = crate::util::new_id();
        sqlx::query(
            "INSERT INTO billing_events (id, tenant_id, provider, event_type, payload) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(provider)
        .bind(event_type)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn mark_event_processed(&self, id: &str, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE billing_events SET processed=true, error=$1, processed_at=NOW() WHERE id=$2")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Webhook dispatch ──────────────────────────────────────────────────

    pub async fn handle_webhook(&self, provider_name: &str, payload: &[u8], signature: &str) -> Result<BillingEvent> {
        let provider = self
            .providers
            .get(provider_name)
            .ok_or_else(|| anyhow::anyhow!("unknown billing provider: {provider_name}"))?;

        let event = provider.verify_webhook(payload, signature).await?;

        // Log event
        let raw: serde_json::Value = serde_json::from_slice(payload).unwrap_or_default();
        let event_type = format!("{:?}", event).split('{').next().unwrap_or("unknown").trim().to_lowercase();
        let tenant_id = match &event {
            BillingEvent::SubscriptionActivated { tenant_id, .. } => tenant_id.as_deref(),
            BillingEvent::PaymentSucceeded { tenant_id, .. } => tenant_id.as_deref(),
            BillingEvent::PaymentFailed { tenant_id, .. } => tenant_id.as_deref(),
            BillingEvent::SubscriptionCancelled { tenant_id, .. } => tenant_id.as_deref(),
            BillingEvent::CreditsPurchased { tenant_id, .. } => Some(tenant_id.as_str()),
            BillingEvent::Unknown { .. } => None,
        };

        let ev_id = self.record_event(provider_name, &event_type, tenant_id, &raw).await?;

        // Act on the event
        if let Err(e) = self.process_event(&event).await {
            let _ = self.mark_event_processed(&ev_id, Some(&e.to_string())).await;
            bail!("billing event processing failed: {e}");
        }
        let _ = self.mark_event_processed(&ev_id, None).await;
        Ok(event)
    }

    async fn process_event(&self, event: &BillingEvent) -> Result<()> {
        match event {
            BillingEvent::SubscriptionActivated {
                tenant_id: Some(tid),
                plan,
                period_start,
                period_end,
                provider_subscription_id,
            } => {
                self.upsert_subscription(
                    tid,
                    "paypal",
                    provider_subscription_id,
                    plan,
                    "active",
                    *period_start,
                    *period_end,
                )
                .await?;
                // Update the tenant's plan in tenant_configs
                self.update_tenant_plan_in_db(tid, plan.as_str()).await.unwrap_or_else(|e| {
                    tracing::warn!(error=%e, "failed to sync tenant plan after subscription activation");
                });
            }
            BillingEvent::PaymentSucceeded {
                tenant_id: Some(tid),
                amount_usd,
                invoice_id,
                provider_subscription_id,
            } => {
                let now = Utc::now();
                let end = now + chrono::Duration::days(30);
                if let Some(inv_id) = invoice_id {
                    let _ = self.create_invoice(tid, "paypal", inv_id, *amount_usd, "paid", now, end, None).await;
                }
                // Renew subscription
                sqlx::query(
                    "UPDATE subscriptions SET status='active', current_period_start=$1, current_period_end=$2, updated_at=NOW()
                      WHERE provider_subscription_id=$3"
                ).bind(now).bind(end).bind(provider_subscription_id)
                .execute(&self.pool).await?;
            }
            BillingEvent::PaymentFailed { provider_subscription_id, .. } => {
                sqlx::query(
                    "UPDATE subscriptions SET status='past_due', updated_at=NOW()
                      WHERE provider_subscription_id=$1",
                )
                .bind(provider_subscription_id)
                .execute(&self.pool)
                .await?;
            }
            BillingEvent::SubscriptionCancelled { tenant_id: Some(tid), provider_subscription_id } => {
                self.cancel_subscription_in_db("paypal", provider_subscription_id).await?;
                self.update_tenant_plan_in_db(tid, "free").await.unwrap_or_default();
            }
            BillingEvent::CreditsPurchased { tenant_id, steps, .. } => {
                self.add_credits(tenant_id, *steps).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn update_tenant_plan_in_db(&self, tenant_id: &str, plan: &str) -> Result<()> {
        sqlx::query("UPDATE tenants SET plan=$1, updated_at=NOW() WHERE id=$2")
            .bind(plan)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn billing_store_no_default_without_providers() {
        let store = BillingStore::new(sqlx::PgPool::connect_lazy("postgres://localhost/test").unwrap());
        assert!(store.default_provider().is_none());
    }
}
