//! Gorgias connector - ecommerce support workflows.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct GorgiasConnector {
    http: reqwest::Client,
}

impl GorgiasConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn subdomain(config: &ConnectorConfig) -> Option<String> {
        config
            .settings
            .get("subdomain")
            .or_else(|| config.settings.get("domain"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn email(config: &ConnectorConfig) -> Option<String> {
        config
            .credentials
            .get("email")
            .or_else(|| config.credentials.get("username"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn api_key(config: &ConnectorConfig) -> Option<String> {
        config
            .credentials
            .get("api_key")
            .or_else(|| config.credentials.get("token"))
            .or_else(|| config.credentials.get("access_token"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn api_base(subdomain: &str) -> String {
        format!("https://{subdomain}.gorgias.com/api")
    }
}

#[async_trait]
impl Connector for GorgiasConnector {
    fn connector_type(&self) -> &str {
        "gorgias"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "ticket_created" | "ticket_updated" | "message_received" => {
                let ticket_id = event.payload.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
                let subject = event.payload.get("subject").and_then(|v| v.as_str()).unwrap_or("ticket");
                let _body = event.payload.get("body_text").and_then(|v| v.as_str()).unwrap_or("");
                Ok(Some(format!(
                    "Gorgias ticket {ticket_id} ({subject}) received. Triage urgency, search for prior orders, and draft the best response or escalation."
                )))
            }
            "refund_requested" | "order_issue" => {
                let subject = event.payload.get("subject").and_then(|v| v.as_str()).unwrap_or("order issue");
                Ok(Some(format!(
                    "Gorgias ecommerce support issue: {subject}. Investigate order, payment, shipping, and customer history before replying."
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
        let subdomain = Self::subdomain(config).ok_or_else(|| anyhow::anyhow!("missing Gorgias subdomain"))?;
        let email = Self::email(config).ok_or_else(|| anyhow::anyhow!("missing Gorgias email"))?;
        let api_key = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing Gorgias api_key"))?;

        let public = metadata.get("public").and_then(|v| v.as_bool()).unwrap_or(false);
        let url = format!("{}/tickets/{}/messages", Self::api_base(&subdomain), external_id);
        let resp = self
            .http
            .post(&url)
            .basic_auth(email, Some(api_key))
            .json(&serde_json::json!({
                "body": output,
                "public": public,
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Gorgias delivery failed: {}", resp.status());
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let subdomain = Self::subdomain(config).ok_or_else(|| anyhow::anyhow!("missing subdomain"))?;
        let email = Self::email(config).ok_or_else(|| anyhow::anyhow!("missing email"))?;
        let api_key = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing api_key"))?;

        let resp = self
            .http
            .get(format!("{}/account", Self::api_base(&subdomain)))
            .basic_auth(email, Some(api_key))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Gorgias auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for GorgiasConnector {
    fn default() -> Self {
        Self::new()
    }
}
