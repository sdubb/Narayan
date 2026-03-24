//! BillingProvider trait and core types used by all provider implementations.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Plan definitions ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BillingPlan {
    Free,
    Go,
    Pro,
    Enterprise,
    /// Sentinel used internally for credit pack one-time purchases — not a subscription tier.
    Credits,
}

impl BillingPlan {
    pub fn as_str(&self) -> &'static str {
        match self {
            BillingPlan::Free => "free",
            BillingPlan::Go => "go",
            BillingPlan::Pro => "pro",
            BillingPlan::Enterprise => "enterprise",
            BillingPlan::Credits => "credits",
        }
    }

    /// Monthly price in USD. 0 = free or custom.
    pub fn monthly_price_usd(&self) -> f64 {
        match self {
            BillingPlan::Free => 0.0,
            BillingPlan::Go => 15.0,
            BillingPlan::Pro => 79.0,
            BillingPlan::Enterprise => 0.0,
            BillingPlan::Credits => BillingPlan::credit_pack_price_usd(),
        }
    }

    /// Max agent steps per month. u64::MAX = unlimited.
    pub fn max_steps_per_month(&self) -> u64 {
        match self {
            BillingPlan::Free => 1_000,
            BillingPlan::Go => 20_000,
            BillingPlan::Pro => 150_000,
            BillingPlan::Enterprise => u64::MAX,
            BillingPlan::Credits => 0,
        }
    }

    /// Max concurrent agents.
    pub fn max_concurrent_agents(&self) -> usize {
        match self {
            BillingPlan::Free => 3,
            BillingPlan::Go => 20,
            BillingPlan::Pro => 200,
            BillingPlan::Enterprise => usize::MAX,
            BillingPlan::Credits => 0,
        }
    }

    /// Max Narayan API requests per second.
    pub fn api_requests_per_sec(&self) -> f64 {
        match self {
            BillingPlan::Free => 2.0,
            BillingPlan::Go => 10.0,
            BillingPlan::Pro => 50.0,
            BillingPlan::Enterprise => 500.0,
            BillingPlan::Credits => 0.0, // not a real plan
        }
    }

    /// All plans get all connectors and full compliance stack.
    pub fn has_all_connectors(&self) -> bool {
        true
    }
    pub fn has_compliance_stack(&self) -> bool {
        true
    }

    /// Step credit pack price in USD.
    pub fn credit_pack_price_usd() -> f64 {
        8.0
    }
    /// Steps per credit pack.
    pub fn credit_pack_steps() -> u64 {
        5_000
    }
}

impl std::fmt::Display for BillingPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for BillingPlan {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "free" => Ok(BillingPlan::Free),
            "go" => Ok(BillingPlan::Go),
            "pro" => Ok(BillingPlan::Pro),
            "enterprise" => Ok(BillingPlan::Enterprise),
            "credits" => Ok(BillingPlan::Credits),
            other => Err(format!("unknown plan: {other}")),
        }
    }
}

// ── Core types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutSession {
    pub session_id: String,
    pub provider: String,
    pub redirect_url: String,
    pub plan: BillingPlan,
    pub amount_usd: f64,
    pub expires_at: DateTime<Utc>,
}

/// Subscription as returned from the provider (not the DB row).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSubscription {
    pub provider_subscription_id: String,
    pub provider: String,
    pub plan: BillingPlan,
    pub status: String,
    pub current_period_start: DateTime<Utc>,
    pub current_period_end: DateTime<Utc>,
}

// ── Billing events (parsed from provider webhooks) ────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BillingEvent {
    SubscriptionActivated {
        provider_subscription_id: String,
        tenant_id: Option<String>,
        plan: BillingPlan,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    },
    PaymentSucceeded {
        provider_subscription_id: String,
        tenant_id: Option<String>,
        amount_usd: f64,
        invoice_id: Option<String>,
    },
    PaymentFailed {
        provider_subscription_id: String,
        tenant_id: Option<String>,
        reason: String,
    },
    SubscriptionCancelled {
        provider_subscription_id: String,
        tenant_id: Option<String>,
    },
    CreditsPurchased {
        tenant_id: String,
        steps: u64,
        amount_usd: f64,
        order_id: String,
    },
    Unknown {
        raw_type: String,
    },
}

// ── Trait ─────────────────────────────────────────────────────────────────

#[async_trait]
pub trait BillingProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn create_checkout_session(
        &self,
        tenant_id: &str,
        plan: &BillingPlan,
        success_url: &str,
        cancel_url: &str,
    ) -> anyhow::Result<CheckoutSession>;

    /// Verify a raw webhook payload + signature header and return a typed event.
    async fn verify_webhook(&self, payload: &[u8], signature: &str) -> anyhow::Result<BillingEvent>;

    async fn cancel_subscription(&self, provider_subscription_id: &str) -> anyhow::Result<()>;
    async fn get_subscription(&self, provider_subscription_id: &str) -> anyhow::Result<ProviderSubscription>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_roundtrip() {
        assert_eq!("go".parse::<BillingPlan>().unwrap(), BillingPlan::Go);
        assert_eq!(BillingPlan::Go.as_str(), "go");
    }

    #[test]
    fn plan_pricing() {
        assert_eq!(BillingPlan::Go.monthly_price_usd(), 15.0);
        assert_eq!(BillingPlan::Pro.monthly_price_usd(), 79.0);
        assert_eq!(BillingPlan::Free.monthly_price_usd(), 0.0);
    }

    #[test]
    fn plan_steps() {
        assert_eq!(BillingPlan::Free.max_steps_per_month(), 1_000);
        assert_eq!(BillingPlan::Go.max_steps_per_month(), 20_000);
        assert_eq!(BillingPlan::Pro.max_steps_per_month(), 150_000);
    }

    #[test]
    fn credit_pack() {
        assert_eq!(BillingPlan::credit_pack_steps(), 5_000);
        assert_eq!(BillingPlan::credit_pack_price_usd(), 8.0);
    }

    #[test]
    fn everyone_gets_all_features() {
        for plan in [BillingPlan::Free, BillingPlan::Go, BillingPlan::Pro] {
            assert!(plan.has_all_connectors());
            assert!(plan.has_compliance_stack());
        }
    }
}
