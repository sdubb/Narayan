//! Zendesk connector — customer support ticket workflows.
//!
//! Receives Zendesk webhook triggers and creates support agent goals.
//! Delivers agent responses as internal notes or public replies.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct ZendeskConnector {
    http: reqwest::Client,
}

impl ZendeskConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn subdomain(config: &ConnectorConfig) -> &str {
        config.settings.get("subdomain").and_then(|v| v.as_str()).unwrap_or("")
    }

    fn auth_header(config: &ConnectorConfig) -> Option<String> {
        let email = config.credentials.get("email").and_then(|v| v.as_str())?;
        let token = config.credentials.get("api_token").and_then(|v| v.as_str())?;
        let credentials = format!("{}/token:{}", email, token);
        Some(format!("Basic {}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, credentials)))
    }
}

#[async_trait]
impl Connector for ZendeskConnector {
    fn connector_type(&self) -> &str {
        "zendesk"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "ticket_created" | "ticket_updated" => {
                let subject = event.payload.get("subject").and_then(|v| v.as_str()).unwrap_or("ticket");
                let description = event.payload.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let ticket_id = event.payload.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                let priority = event.payload.get("priority").and_then(|v| v.as_str()).unwrap_or("normal");

                Ok(Some(format!(
                    "Handle Zendesk ticket #{} (priority: {}): {}. Customer message: {}. \
                     Use vector_search to find similar resolved cases first. \
                     Draft a response following the support policy. \
                     If the issue requires escalation, flag it.",
                    ticket_id, priority, subject, description
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
        let subdomain = Self::subdomain(config);
        let auth = Self::auth_header(config).ok_or_else(|| anyhow::anyhow!("missing Zendesk credentials"))?;

        let is_public = metadata.get("public").and_then(|v| v.as_bool()).unwrap_or(false);

        let url = format!("https://{}.zendesk.com/api/v2/tickets/{}.json", subdomain, external_id);
        self.http
            .put(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "ticket": {
                    "comment": {
                        "body": output,
                        "public": is_public,
                    }
                }
            }))
            .send()
            .await?;

        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let subdomain = Self::subdomain(config);
        let auth = Self::auth_header(config).ok_or_else(|| anyhow::anyhow!("missing Zendesk credentials"))?;

        let url = format!("https://{}.zendesk.com/api/v2/users/me.json", subdomain);
        let resp = self.http.get(&url).header("Authorization", &auth).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Zendesk auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}
