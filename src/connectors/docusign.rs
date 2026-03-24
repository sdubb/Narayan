//! DocuSign connector — legal and contract operations.
//!
//! Triggers from DocuSign Connect webhooks:
//! - Envelope sent for signing → pre-signature review agent
//! - Envelope completed → obligation extraction agent
//! - Envelope declined → analysis and re-engagement agent

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct DocuSignConnector {
    http: reqwest::Client,
}

impl DocuSignConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn base_url(config: &ConnectorConfig) -> String {
        let account_id = config.settings.get("account_id").and_then(|v| v.as_str()).unwrap_or("");
        format!("https://na1.docusign.net/restapi/v2.1/accounts/{account_id}")
    }

    fn bearer(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("access_token").and_then(|v| v.as_str()).map(String::from)
    }
}

#[async_trait]
impl Connector for DocuSignConnector {
    fn connector_type(&self) -> &str {
        "docusign"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "envelope_sent" => {
                let envelope_id = event.payload["envelopeId"].as_str().unwrap_or("unknown");
                let subject = event.payload["emailSubject"].as_str().unwrap_or("contract");
                Ok(Some(format!(
                    "Pre-signature review for DocuSign envelope {envelope_id}: '{subject}'. \
                     Download the document, read in full. \
                     Produce a structured issues register: unusual clauses, missing standard terms, \
                     liability exposure, non-standard payment terms, IP ownership. \
                     Cite exact section numbers for each issue. \
                     Route findings to legal review queue before signing proceeds.",
                )))
            }
            "envelope_completed" => {
                let envelope_id = event.payload["envelopeId"].as_str().unwrap_or("unknown");
                let subject = event.payload["emailSubject"].as_str().unwrap_or("contract");
                Ok(Some(format!(
                    "Extract obligations from completed contract envelope {envelope_id}: '{subject}'. \
                     Identify: payment milestones, deliverable deadlines, renewal dates, \
                     termination notice periods, compliance requirements. \
                     Write an obligations tracker spreadsheet with due dates and responsible parties. \
                     Store full contract text to vector_store for future retrieval.",
                )))
            }
            "envelope_declined" => {
                let envelope_id = event.payload["envelopeId"].as_str().unwrap_or("unknown");
                let reason = event.payload["declineReason"].as_str().unwrap_or("not provided");
                Ok(Some(format!(
                    "Analyse declined envelope {envelope_id}. Stated reason: '{reason}'. \
                     Compare the declined version against standard templates. \
                     Identify likely objection points. \
                     Suggest specific redlines that may resolve the decline. \
                     Draft a re-engagement email for the account team.",
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
        let base = Self::base_url(config);
        let token = Self::bearer(config).ok_or_else(|| anyhow::anyhow!("missing DocuSign access_token"))?;

        let delivery = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("note");
        match delivery {
            "envelope_note" => {
                let url = format!("{base}/envelopes/{external_id}/notes");
                self.http.post(&url).bearer_auth(&token).json(&serde_json::json!({ "note": output })).send().await?;
            }
            _ => {
                tracing::info!(envelope_id = external_id, delivery, "DocuSign output logged");
            }
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let base = Self::base_url(config);
        let token = Self::bearer(config).ok_or_else(|| anyhow::anyhow!("missing access_token"))?;
        let url = format!("{base}/users");
        let resp = self.http.get(&url).bearer_auth(&token).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("DocuSign auth failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for DocuSignConnector {
    fn default() -> Self {
        Self::new()
    }
}
