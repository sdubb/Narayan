//! Stripe connector — payment and billing workflows.
//!
//! Auth: API key (secret key, sk_live_... or sk_test_...)
//! Settings: none required
//!
//! Webhook events handled (via Stripe webhook endpoint):
//!   payment_intent.payment_failed    → investigate and notify customer
//!   customer.subscription.deleted    → churn analysis and win-back
//!   invoice.payment_failed           → dunning workflow
//!   charge.dispute.created           → dispute response preparation
//!   customer.created                 → new customer onboarding
//!
//! Webhook signature verified via Stripe-Signature header (HMAC SHA-256)

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct StripeConnector {
    http: reqwest::Client,
}

impl StripeConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn secret_key(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("api_key")
            .or_else(|| config.credentials.get("secret_key"))
            .or_else(|| config.credentials.get("access_token"))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    fn amount_str(cents: i64, currency: &str) -> String {
        let major = cents as f64 / 100.0;
        format!("{:.2} {}", major, currency.to_uppercase())
    }
}

#[async_trait]
impl Connector for StripeConnector {
    fn connector_type(&self) -> &str { "stripe" }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        let payload = &event.payload;
        // Stripe webhook structure: { type: "...", data: { object: {...} } }
        let obj = &payload["data"]["object"];
        let event_type = event.event_type.as_str();

        match event_type {
            "payment_intent.payment_failed" => {
                let amount      = obj["amount"].as_i64().unwrap_or(0);
                let currency    = obj["currency"].as_str().unwrap_or("usd");
                let customer_id = obj["customer"].as_str().unwrap_or("unknown");
                let reason      = obj["last_payment_error"]["message"].as_str().unwrap_or("unknown reason");
                let pi_id       = obj["id"].as_str().unwrap_or("");

                Ok(Some(format!(
                    "Stripe payment failed for {pi_id}: {} for customer {customer_id}. \
                     Failure reason: {reason}. \
                     Look up the customer, check if this is a recurring issue, \
                     draft a recovery email with a payment retry link, \
                     and log the failure to the CRM.",
                    Self::amount_str(amount, currency),
                )))
            }

            "customer.subscription.deleted" => {
                let customer_id  = obj["customer"].as_str().unwrap_or("unknown");
                let plan_id      = obj["items"]["data"][0]["price"]["id"].as_str().unwrap_or("unknown");
                let cancel_at    = obj["canceled_at"].as_i64().unwrap_or(0);

                Ok(Some(format!(
                    "Stripe subscription cancelled for customer {customer_id} (plan: {plan_id}, \
                     cancelled at: {cancel_at}). \
                     Look up the customer's usage history and NPS scores. \
                     Identify the most likely reason for churn. \
                     Draft a personalised win-back email with an appropriate offer.",
                )))
            }

            "invoice.payment_failed" => {
                let invoice_id  = obj["id"].as_str().unwrap_or("unknown");
                let customer_id = obj["customer"].as_str().unwrap_or("unknown");
                let amount      = obj["amount_due"].as_i64().unwrap_or(0);
                let currency    = obj["currency"].as_str().unwrap_or("usd");
                let attempt     = obj["attempt_count"].as_i64().unwrap_or(1);

                Ok(Some(format!(
                    "Stripe invoice {invoice_id} payment failed for customer {customer_id}. \
                     Amount due: {}. Attempt #{attempt}. \
                     Check if the customer has updated payment methods recently. \
                     Draft a dunning email appropriate for attempt #{attempt}. \
                     If this is attempt 3+, flag for account review.",
                    Self::amount_str(amount, currency),
                )))
            }

            "charge.dispute.created" => {
                let dispute_id  = obj["id"].as_str().unwrap_or("unknown");
                let charge_id   = obj["charge"].as_str().unwrap_or("unknown");
                let amount      = obj["amount"].as_i64().unwrap_or(0);
                let currency    = obj["currency"].as_str().unwrap_or("usd");
                let reason      = obj["reason"].as_str().unwrap_or("unknown");
                let due_by      = obj["evidence_details"]["due_by"].as_i64().unwrap_or(0);

                Ok(Some(format!(
                    "Stripe dispute {dispute_id} created for charge {charge_id}: {} — reason: {reason}. \
                     Evidence submission due by: {due_by}. \
                     Gather all transaction evidence: receipts, communication logs, delivery confirmation. \
                     Draft the dispute response and evidence package.",
                    Self::amount_str(amount, currency),
                )))
            }

            "customer.created" => {
                let customer_id = obj["id"].as_str().unwrap_or("unknown");
                let email       = obj["email"].as_str().unwrap_or("");
                let name        = obj["name"].as_str().unwrap_or("customer");

                Ok(Some(format!(
                    "New Stripe customer created: {name} ({email}, id: {customer_id}). \
                     Look up company information if this is a business account. \
                     Send a personalised welcome email with getting-started resources.",
                )))
            }

            _ => Ok(None),
        }
    }

    async fn deliver_output(
        &self,
        config: &ConnectorConfig,
        external_id: &str,
        output: &str,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        let key = Self::secret_key(config)
            .ok_or_else(|| anyhow::anyhow!("missing Stripe api_key"))?;

        // Delivery for Stripe means adding metadata to a customer/charge record
        let delivery_type = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("customer_note");

        match delivery_type {
            "customer_note" => {
                // Add a metadata note to the customer record
                let url = format!("https://api.stripe.com/v1/customers/{}", external_id);
                self.http
                    .post(&url)
                    .basic_auth(&key, Option::<&str>::None)
                    .form(&[("metadata[narayan_note]", output)])
                    .send()
                    .await?;
            }
            "dispute_evidence" => {
                // Submit dispute evidence
                let url = format!("https://api.stripe.com/v1/disputes/{}/close", external_id);
                self.http
                    .post(&url)
                    .basic_auth(&key, Option::<&str>::None)
                    .form(&[("evidence[uncategorized_text]", output)])
                    .send()
                    .await?;
            }
            _ => {
                tracing::warn!(delivery_type, "Stripe: unknown delivery_type, skipping");
            }
        }

        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let key = Self::secret_key(config)
            .ok_or_else(|| anyhow::anyhow!("missing 'api_key' in credentials"))?;

        if !key.starts_with("sk_") {
            anyhow::bail!("Stripe api_key must start with 'sk_live_' or 'sk_test_'");
        }

        let resp = self.http
            .get("https://api.stripe.com/v1/customers?limit=1")
            .basic_auth(&key, Option::<&str>::None)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Stripe auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}
