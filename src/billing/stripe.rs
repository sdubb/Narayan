//! Stripe billing provider stub.
//!
//! Ready to complete — all method signatures match the trait.
//! To activate: set STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SECRET env vars.
//!
//! Docs: https://stripe.com/docs/api

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::billing::provider::{BillingEvent, BillingPlan, BillingProvider, CheckoutSession, ProviderSubscription};

const STRIPE_BASE: &str = "https://api.stripe.com/v1";

pub struct StripeProvider {
    secret_key:      String,
    webhook_secret:  String,
    http:            Client,
}

impl StripeProvider {
    pub fn new(secret_key: String, webhook_secret: String) -> Self {
        Self {
            secret_key,
            webhook_secret,
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(
            std::env::var("STRIPE_SECRET_KEY").ok()?,
            std::env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default(),
        ))
    }

    async fn post_form(&self, path: &str, params: Vec<(&str, String)>) -> Result<Value> {
        let res = self.http
            .post(format!("{}{}", STRIPE_BASE, path))
            .bearer_auth(&self.secret_key)
            .form(&params)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        Ok(res)
    }

    async fn get_json(&self, path: &str) -> Result<Value> {
        let res = self.http
            .get(format!("{}{}", STRIPE_BASE, path))
            .bearer_auth(&self.secret_key)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        Ok(res)
    }

    /// Stripe price IDs — set these to your actual Stripe Price IDs in production.
    fn price_id(plan: &BillingPlan) -> &'static str {
        match plan {
            BillingPlan::Free       => "",
            BillingPlan::Go         => "price_go_monthly",          // set STRIPE_PRICE_ID_GO
            BillingPlan::Pro        => "price_pro_monthly",         // set STRIPE_PRICE_ID_PRO
            BillingPlan::Enterprise => "price_enterprise_monthly",  // set STRIPE_PRICE_ID_ENTERPRISE
            BillingPlan::Credits    => "price_credits_pack",        // set STRIPE_PRICE_ID_CREDITS
        }
    }

    /// Verify Stripe webhook signature using HMAC-SHA256.
    fn verify_stripe_signature(payload: &[u8], signature: &str, secret: &str) -> Result<()> {
        // Stripe signature format: t=timestamp,v1=sig1,v1=sig2
        let ts = signature.split(',')
            .find(|p| p.starts_with("t="))
            .and_then(|p| p.strip_prefix("t="))
            .ok_or_else(|| anyhow!("no timestamp in Stripe signature"))?;

        let expected_payload = format!("{}.{}", ts, std::str::from_utf8(payload)?);

        let provided_sigs: Vec<&str> = signature.split(',')
            .filter(|p| p.starts_with("v1="))
            .filter_map(|p| p.strip_prefix("v1="))
            .collect();

        use ring::hmac;
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let computed = hmac::sign(&key, expected_payload.as_bytes());
        let computed_hex = hex::encode(computed.as_ref());

        if !provided_sigs.iter().any(|s| *s == computed_hex.as_str()) {
            bail!("Stripe webhook signature mismatch");
        }

        // Check timestamp is recent (within 5 minutes)
        let ts_secs: i64 = ts.parse().map_err(|_| anyhow!("invalid timestamp"))?;
        let now = chrono::Utc::now().timestamp();
        if (now - ts_secs).abs() > 300 {
            bail!("Stripe webhook timestamp too old");
        }

        Ok(())
    }
}

#[async_trait]
impl BillingProvider for StripeProvider {
    fn name(&self) -> &'static str { "stripe" }

    async fn create_checkout_session(
        &self,
        tenant_id:   &str,
        plan:        &BillingPlan,
        success_url: &str,
        cancel_url:  &str,
    ) -> Result<CheckoutSession> {
        if plan == &BillingPlan::Free {
            return Ok(CheckoutSession {
                session_id:   format!("free-{}", tenant_id),
                provider:     "stripe".into(),
                redirect_url: success_url.to_string(),
                plan:         plan.clone(),
                amount_usd:   0.0,
                expires_at:   chrono::Utc::now() + chrono::Duration::hours(1),
            });
        }

        // Credits are a one-time payment, not a subscription
        let mode = if plan == &BillingPlan::Credits { "payment" } else { "subscription" };

        let price_id = Self::price_id(plan);
        let params = vec![
            ("mode",                           mode.to_string()),
            ("line_items[0][price]",            price_id.to_string()),
            ("line_items[0][quantity]",         "1".to_string()),
            ("success_url",                     success_url.to_string()),
            ("cancel_url",                      cancel_url.to_string()),
            ("client_reference_id",             tenant_id.to_string()),
            ("metadata[tenant_id]",             tenant_id.to_string()),
            ("metadata[plan]",                  plan.as_str().to_string()),
        ];

        let res = self.post_form("/checkout/sessions", params).await?;
        let session_id   = res["id"].as_str().ok_or_else(|| anyhow!("no session id"))?.to_string();
        let redirect_url = res["url"].as_str().ok_or_else(|| anyhow!("no checkout URL"))?.to_string();
        let expires_at   = res["expires_at"].as_i64()
            .map(|ts| chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(chrono::Utc::now))
            .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(24));

        Ok(CheckoutSession {
            session_id,
            provider: "stripe".into(),
            redirect_url,
            plan:      plan.clone(),
            amount_usd: plan.monthly_price_usd(),
            expires_at,
        })
    }

    async fn verify_webhook(&self, payload: &[u8], signature: &str) -> anyhow::Result<BillingEvent> {
        if !self.webhook_secret.is_empty() {
            Self::verify_stripe_signature(payload, signature, &self.webhook_secret)?;
        }

        let raw: Value = serde_json::from_slice(payload)?;
        let event_type = raw["type"].as_str().unwrap_or("").to_string();
        let data       = &raw["data"]["object"];

        let event = match event_type.as_str() {
            "customer.subscription.created" | "customer.subscription.updated" => {
                let sub_id    = data["id"].as_str().unwrap_or("").to_string();
                let tenant_id = data["metadata"]["tenant_id"].as_str().map(String::from);
                let plan_str  = data["metadata"]["plan"].as_str().unwrap_or("pro");
                let plan      = plan_str.parse().unwrap_or(BillingPlan::Pro);
                let ps        = data["current_period_start"].as_i64().unwrap_or(0);
                let pe        = data["current_period_end"].as_i64().unwrap_or(0);
                BillingEvent::SubscriptionActivated {
                    provider_subscription_id: sub_id,
                    tenant_id,
                    plan,
                    period_start: chrono::DateTime::from_timestamp(ps, 0).unwrap_or_else(chrono::Utc::now),
                    period_end:   chrono::DateTime::from_timestamp(pe, 0).unwrap_or_else(chrono::Utc::now),
                }
            }
            "invoice.payment_succeeded" => {
                let sub_id    = data["subscription"].as_str().unwrap_or("").to_string();
                let tenant_id = data["metadata"]["tenant_id"].as_str().map(String::from);
                let amount    = data["amount_paid"].as_i64().unwrap_or(0) as f64 / 100.0;
                BillingEvent::PaymentSucceeded {
                    provider_subscription_id: sub_id,
                    tenant_id,
                    amount_usd:  amount,
                    invoice_id:  data["id"].as_str().map(String::from),
                }
            }
            "invoice.payment_failed" => {
                let sub_id    = data["subscription"].as_str().unwrap_or("").to_string();
                let tenant_id = data["metadata"]["tenant_id"].as_str().map(String::from);
                BillingEvent::PaymentFailed {
                    provider_subscription_id: sub_id,
                    tenant_id,
                    reason: data["last_finalization_error"]["message"]
                        .as_str().unwrap_or("payment failed").to_string(),
                }
            }
            "customer.subscription.deleted" => {
                let sub_id    = data["id"].as_str().unwrap_or("").to_string();
                let tenant_id = data["metadata"]["tenant_id"].as_str().map(String::from);
                BillingEvent::SubscriptionCancelled { provider_subscription_id: sub_id, tenant_id }
            }
            other => BillingEvent::Unknown { raw_type: other.to_string() },
        };

        Ok(event)
    }

    async fn cancel_subscription(&self, provider_subscription_id: &str) -> Result<()> {
        self.post_form(
            &format!("/subscriptions/{}", provider_subscription_id),
            vec![("cancel_at_period_end", "true".to_string())],
        ).await?;
        Ok(())
    }

    async fn get_subscription(&self, provider_subscription_id: &str) -> Result<ProviderSubscription> {
        let res = self.get_json(&format!("/subscriptions/{}", provider_subscription_id)).await?;
        let plan_str = res["metadata"]["plan"].as_str().unwrap_or("pro");
        let plan     = plan_str.parse().unwrap_or(BillingPlan::Pro);
        let status   = res["status"].as_str().unwrap_or("unknown").to_string();
        let ps       = res["current_period_start"].as_i64().unwrap_or(0);
        let pe       = res["current_period_end"].as_i64().unwrap_or(0);
        Ok(ProviderSubscription {
            provider_subscription_id: provider_subscription_id.to_string(),
            provider: "stripe".into(),
            plan,
            status,
            current_period_start: chrono::DateTime::from_timestamp(ps, 0).unwrap_or_else(chrono::Utc::now),
            current_period_end:   chrono::DateTime::from_timestamp(pe, 0).unwrap_or_else(chrono::Utc::now),
        })
    }
}
