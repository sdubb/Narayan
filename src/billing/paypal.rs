//! PayPal billing provider — subscriptions + one-time credit purchases.
//!
//! For subscriptions, you must pre-create PayPal Billing Plans in the dashboard
//! (or via API) and set the plan IDs in env vars:
//!   PAYPAL_PLAN_ID_GO         — PayPal Plan ID for the Go ($15/mo) plan
//!   PAYPAL_PLAN_ID_PRO        — PayPal Plan ID for the Pro ($79/mo) plan
//!
//! For credit packs, a one-time order is created.
//!
//! Required env vars:
//!   PAYPAL_CLIENT_ID
//!   PAYPAL_CLIENT_SECRET
//!   PAYPAL_WEBHOOK_ID         — from PayPal dashboard (for signature verification)
//!   PAYPAL_SANDBOX            — "true" for sandbox

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::billing::{BillingEvent, BillingPlan, BillingProvider, CheckoutSession, ProviderSubscription};

const SANDBOX_BASE: &str = "https://api-m.sandbox.paypal.com";
const LIVE_BASE:    &str = "https://api-m.paypal.com";

pub struct PayPalProvider {
    client_id:     String,
    client_secret: String,
    webhook_id:    String,
    base_url:      &'static str,
    plan_id_go:    Option<String>,
    plan_id_pro:   Option<String>,
    http:          Client,
    token_cache:   RwLock<Option<(String, u64)>>,
}

impl PayPalProvider {
    pub fn new(client_id: String, client_secret: String, webhook_id: String, sandbox: bool) -> Self {
        Self {
            client_id,
            client_secret,
            webhook_id,
            base_url:     if sandbox { SANDBOX_BASE } else { LIVE_BASE },
            plan_id_go:   std::env::var("PAYPAL_PLAN_ID_GO").ok(),
            plan_id_pro:  std::env::var("PAYPAL_PLAN_ID_PRO").ok(),
            http: Client::builder().timeout(std::time::Duration::from_secs(30)).build().expect("reqwest"),
            token_cache: RwLock::new(None),
        }
    }

    pub fn from_env() -> Option<Self> {
        Some(Self::new(
            std::env::var("PAYPAL_CLIENT_ID").ok()?,
            std::env::var("PAYPAL_CLIENT_SECRET").ok()?,
            std::env::var("PAYPAL_WEBHOOK_ID").unwrap_or_default(),
            std::env::var("PAYPAL_SANDBOX").map(|v| v == "true").unwrap_or(false),
        ))
    }

    async fn access_token(&self) -> Result<String> {
        {
            let cache = self.token_cache.read().await;
            if let Some((token, exp)) = cache.as_ref() {
                let now = unix_now();
                if *exp > now + 60 { return Ok(token.clone()); }
            }
        }
        let res = self.http
            .post(format!("{}/v1/oauth2/token", self.base_url))
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[("grant_type", "client_credentials")])
            .send().await?.error_for_status()?.json::<Value>().await?;

        let token      = res["access_token"].as_str().ok_or_else(|| anyhow!("no access_token"))?.to_string();
        let expires_in = res["expires_in"].as_u64().unwrap_or(3600);
        *self.token_cache.write().await = Some((token.clone(), unix_now() + expires_in));
        Ok(token)
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        let token = self.access_token().await?;
        Ok(self.http.post(format!("{}{}", self.base_url, path))
            .bearer_auth(token).json(&body)
            .send().await?.error_for_status()?.json::<Value>().await?)
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let token = self.access_token().await?;
        Ok(self.http.get(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .send().await?.error_for_status()?.json::<Value>().await?)
    }
}

#[async_trait]
impl BillingProvider for PayPalProvider {
    fn name(&self) -> &'static str { "paypal" }

    async fn create_checkout_session(
        &self,
        tenant_id:   &str,
        plan:        &BillingPlan,
        success_url: &str,
        cancel_url:  &str,
    ) -> Result<CheckoutSession> {
        // Free plan — no payment
        if plan == &BillingPlan::Free {
            return Ok(CheckoutSession {
                session_id:   format!("free-{}", tenant_id),
                provider:     "paypal".into(),
                redirect_url: success_url.to_string(),
                plan:         plan.clone(),
                amount_usd:   0.0,
                expires_at:   chrono::Utc::now() + chrono::Duration::hours(1),
            });
        }

        // Credit pack — one-time order (not a subscription)
        if plan == &BillingPlan::Credits {
            return self.create_credit_order(tenant_id, success_url, cancel_url).await;
        }

        // Subscription plans — requires pre-created PayPal Billing Plan ID
        let paypal_plan_id = match plan {
            BillingPlan::Go  => self.plan_id_go.as_deref().ok_or_else(|| anyhow!(
                "PAYPAL_PLAN_ID_GO not set. Create a PayPal Billing Plan and set this env var."
            ))?,
            BillingPlan::Pro => self.plan_id_pro.as_deref().ok_or_else(|| anyhow!(
                "PAYPAL_PLAN_ID_PRO not set. Create a PayPal Billing Plan and set this env var."
            ))?,
            BillingPlan::Enterprise => bail!("Enterprise plan uses manual billing — contact sales"),
            _ => bail!("unexpected plan: {}", plan),
        };

        // Create a PayPal Subscription (recurring billing)
        // tenant_id is stored in custom_id so we can identify the tenant in webhooks
        let body = json!({
            "plan_id": paypal_plan_id,
            "custom_id": tenant_id,
            "application_context": {
                "brand_name":   "Narayan",
                "return_url":   success_url,
                "cancel_url":   cancel_url,
                "user_action":  "SUBSCRIBE_NOW",
                "payment_method": {
                    "payer_selected":  "PAYPAL",
                    "payee_preferred": "IMMEDIATE_PAYMENT_REQUIRED"
                }
            }
        });

        let res = self.post("/v1/billing/subscriptions", body).await?;
        let sub_id = res["id"].as_str().ok_or_else(|| anyhow!("no subscription id in PayPal response"))?.to_string();

        let redirect_url = res["links"]
            .as_array()
            .and_then(|links| links.iter().find(|l| l["rel"] == "approve"))
            .and_then(|l| l["href"].as_str())
            .ok_or_else(|| anyhow!("no approval URL in PayPal subscription response"))?
            .to_string();

        Ok(CheckoutSession {
            session_id:   sub_id,
            provider:     "paypal".into(),
            redirect_url,
            plan:         plan.clone(),
            amount_usd:   plan.monthly_price_usd(),
            expires_at:   chrono::Utc::now() + chrono::Duration::hours(3),
        })
    }

    async fn verify_webhook(&self, payload: &[u8], signature: &str) -> anyhow::Result<BillingEvent> {
        let body_str = std::str::from_utf8(payload)?;
        let raw: Value = serde_json::from_str(body_str)?;

        // Verify with PayPal API if webhook_id is configured
        if !self.webhook_id.is_empty() {
            let verify_body = json!({
                "webhook_id":    self.webhook_id,
                "webhook_event": raw,
                "transmission_sig": signature
            });
            let result = self.post("/v1/notifications/verify-webhook-signature", verify_body).await?;
            if result["verification_status"].as_str() != Some("SUCCESS") {
                bail!("PayPal webhook signature verification failed");
            }
        }

        let event_type = raw["event_type"].as_str().unwrap_or("").to_string();
        let resource   = &raw["resource"];

        // tenant_id is stored in custom_id on subscriptions (set when creating the subscription)
        let tenant_id = resource["custom_id"].as_str().map(String::from);
        let sub_id    = resource["id"].as_str().unwrap_or("").to_string();

        // Derive plan from the PayPal plan_id → env var mapping
        let plan = {
            let pid = resource["plan_id"].as_str().unwrap_or("");
            if self.plan_id_go.as_deref() == Some(pid)  { BillingPlan::Go }
            else if self.plan_id_pro.as_deref() == Some(pid) { BillingPlan::Pro }
            else { BillingPlan::Go } // safe fallback
        };

        let event = match event_type.as_str() {
            "BILLING.SUBSCRIPTION.ACTIVATED" | "BILLING.SUBSCRIPTION.RENEWED" => {
                let billing_info = &resource["billing_info"];
                let now = chrono::Utc::now();
                let period_end = billing_info["next_billing_time"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or(now + chrono::Duration::days(30));
                BillingEvent::SubscriptionActivated {
                    provider_subscription_id: sub_id,
                    tenant_id,
                    plan,
                    period_start: now,
                    period_end,
                }
            }
            "PAYMENT.SALE.COMPLETED" | "PAYMENT.CAPTURE.COMPLETED" => {
                let amount = resource["amount"]["total"].as_str()
                    .or_else(|| resource["amount"]["value"].as_str())
                    .unwrap_or("0").parse::<f64>().unwrap_or(0.0);
                let sid = resource["billing_agreement_id"].as_str()
                    .or_else(|| resource["id"].as_str()).unwrap_or("").to_string();
                BillingEvent::PaymentSucceeded {
                    provider_subscription_id: sid,
                    tenant_id,
                    amount_usd:  amount,
                    invoice_id:  resource["id"].as_str().map(String::from),
                }
            }
            "PAYMENT.SALE.DENIED" | "BILLING.SUBSCRIPTION.PAYMENT.FAILED" => {
                BillingEvent::PaymentFailed {
                    provider_subscription_id: sub_id,
                    tenant_id,
                    reason: event_type.clone(),
                }
            }
            "BILLING.SUBSCRIPTION.CANCELLED" | "BILLING.SUBSCRIPTION.EXPIRED" => {
                BillingEvent::SubscriptionCancelled { provider_subscription_id: sub_id, tenant_id }
            }
            other => BillingEvent::Unknown { raw_type: other.to_string() },
        };

        Ok(event)
    }

    async fn cancel_subscription(&self, provider_subscription_id: &str) -> Result<()> {
        let body = json!({ "reason": "Cancelled by customer via Narayan dashboard" });
        self.post(&format!("/v1/billing/subscriptions/{}/cancel", provider_subscription_id), body).await?;
        Ok(())
    }

    async fn get_subscription(&self, provider_subscription_id: &str) -> Result<ProviderSubscription> {
        let res    = self.get(&format!("/v1/billing/subscriptions/{}", provider_subscription_id)).await?;
        let status = res["status"].as_str().unwrap_or("unknown").to_lowercase();
        let pid    = res["plan_id"].as_str().unwrap_or("");
        let plan   = if self.plan_id_go.as_deref() == Some(pid)  { BillingPlan::Go }
                     else if self.plan_id_pro.as_deref() == Some(pid) { BillingPlan::Pro }
                     else { BillingPlan::Go };
        let now    = chrono::Utc::now();
        let period_end = res["billing_info"]["next_billing_time"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or(now + chrono::Duration::days(30));
        Ok(ProviderSubscription {
            provider_subscription_id: provider_subscription_id.to_string(),
            provider:    "paypal".into(),
            plan,
            status,
            current_period_start: now,
            current_period_end:   period_end,
        })
    }
}

impl PayPalProvider {
    async fn create_credit_order(&self, tenant_id: &str, success_url: &str, cancel_url: &str) -> Result<CheckoutSession> {
        let body = json!({
            "intent": "CAPTURE",
            "purchase_units": [{
                "amount": {
                    "currency_code": "USD",
                    "value": format!("{:.2}", BillingPlan::credit_pack_price_usd())
                },
                "description": "Narayan Credit Pack — 5,000 extra steps",
                "custom_id": tenant_id
            }],
            "application_context": {
                "return_url": success_url,
                "cancel_url": cancel_url,
                "brand_name": "Narayan",
                "user_action": "PAY_NOW"
            }
        });

        let res = self.post("/v2/checkout/orders", body).await?;
        let order_id = res["id"].as_str().ok_or_else(|| anyhow!("no order id"))?.to_string();
        let redirect_url = res["links"]
            .as_array()
            .and_then(|links| links.iter().find(|l| l["rel"] == "approve"))
            .and_then(|l| l["href"].as_str())
            .ok_or_else(|| anyhow!("no approval URL"))?
            .to_string();

        Ok(CheckoutSession {
            session_id:   order_id,
            provider:     "paypal".into(),
            redirect_url,
            plan:         BillingPlan::Credits,
            amount_usd:   BillingPlan::credit_pack_price_usd(),
            expires_at:   chrono::Utc::now() + chrono::Duration::hours(3),
        })
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs()
}
