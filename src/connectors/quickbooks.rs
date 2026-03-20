//! QuickBooks connector — finance and accounting automation.
//!
//! Triggers from QuickBooks webhooks:
//! - Invoice overdue → collections outreach agent
//! - New expense batch → categorisation and coding agent
//! - Month-end signal → close checklist agent
//!
//! Delivers back: categorised expenses, reconciliation notes, journal entries.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct QuickBooksConnector {
    http: reqwest::Client,
}

impl QuickBooksConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn realm_id(config: &ConnectorConfig) -> &str {
        config.settings.get("realm_id").and_then(|v| v.as_str()).unwrap_or("")
    }

    fn bearer(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("access_token").and_then(|v| v.as_str()).map(String::from)
    }

    fn api_base(config: &ConnectorConfig) -> String {
        let realm = Self::realm_id(config);
        format!("https://quickbooks.api.intuit.com/v3/company/{realm}")
    }
}

#[async_trait]
impl Connector for QuickBooksConnector {
    fn connector_type(&self) -> &str {
        "quickbooks"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "invoice_overdue" => {
                let invoice_num = event.payload["DocNumber"].as_str().unwrap_or("unknown");
                let amount      = event.payload["Balance"].as_str().unwrap_or("0");
                let customer    = event.payload["CustomerRef"]["name"].as_str().unwrap_or("customer");
                let days        = event.payload["DaysOverdue"].as_str().unwrap_or("0");
                Ok(Some(format!(
                    "Invoice {invoice_num} for {customer} is {days} days overdue (balance: ${amount}). \
                     Research the customer's payment history and current status. \
                     Draft a professional collections email scaled to the days overdue. \
                     Log outreach attempt with timestamp.",
                )))
            }
            "expense_batch_ready" => {
                let count  = event.payload["count"].as_u64().unwrap_or(0);
                let period = event.payload["period"].as_str().unwrap_or("unknown period");
                Ok(Some(format!(
                    "Categorise {count} expenses for {period}. \
                     For each expense: read the receipt or description, assign GL account code, \
                     flag any that require manager approval (>$500 or unusual categories). \
                     Write categorisation results to spreadsheet with confidence scores.",
                )))
            }
            "month_end_close" => {
                let month = event.payload["month"].as_str().unwrap_or("current month");
                Ok(Some(format!(
                    "Run month-end close checklist for {month}. \
                     Verify all invoices are recorded. \
                     Reconcile bank accounts against ledger. \
                     Check for uncategorised transactions. \
                     Produce a close status report listing any open items requiring human resolution.",
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
        let base  = Self::api_base(config);
        let token = Self::bearer(config).ok_or_else(|| anyhow::anyhow!("missing QB access_token"))?;

        let delivery = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("note");
        match delivery {
            "invoice_note" => {
                // Attach a memo to the invoice
                let url = format!("{base}/invoice/{external_id}");
                let resp = self.http.get(&url).bearer_auth(&token)
                    .query(&[("minorversion", "65")]).send().await?;
                if let Ok(mut invoice) = resp.json::<serde_json::Value>().await {
                    if let Some(obj) = invoice["Invoice"].as_object_mut() {
                        obj.insert("PrivateNote".into(), serde_json::json!(output));
                    }
                    self.http.post(&format!("{base}/invoice"))
                        .bearer_auth(&token)
                        .query(&[("minorversion", "65")])
                        .json(&invoice)
                        .send().await?;
                }
            }
            _ => {
                tracing::info!(external_id, delivery, output_len = output.len(), "QuickBooks output logged");
            }
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let base  = Self::api_base(config);
        let token = Self::bearer(config).ok_or_else(|| anyhow::anyhow!("missing access_token"))?;
        let url   = format!("{base}/companyinfo/{}", Self::realm_id(config));
        let resp  = self.http.get(&url).bearer_auth(&token)
            .query(&[("minorversion", "65")]).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("QuickBooks auth failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for QuickBooksConnector {
    fn default() -> Self { Self::new() }
}
