//! Freshdesk connector — helpdesk ticket workflows.
//!
//! Auth: API key (HTTP Basic: api_key:X)
//! Settings: domain (e.g. "acme" → acme.freshdesk.com)
//!
//! Webhook events handled:
//!   ticket_created   → new ticket needs triage and response
//!   ticket_updated   → status/priority changed
//!   note_created     → agent note added, may need action

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct FreshdeskConnector {
    http: reqwest::Client,
}

impl FreshdeskConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn domain(config: &ConnectorConfig) -> &str {
        config.settings.get("domain").and_then(|v| v.as_str()).unwrap_or("")
    }

    fn auth(config: &ConnectorConfig) -> Option<reqwest::header::HeaderValue> {
        let api_key = config
            .credentials
            .get("api_key")
            .or_else(|| config.credentials.get("access_token"))
            .or_else(|| config.credentials.get("token"))
            .and_then(|v| v.as_str())?;
        // Freshdesk uses HTTP Basic: api_key:X
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{}:X", api_key));
        format!("Basic {}", encoded).parse().ok()
    }

    fn base_url(config: &ConnectorConfig) -> String {
        format!("https://{}.freshdesk.com/api/v2", Self::domain(config))
    }
}

#[async_trait]
impl Connector for FreshdeskConnector {
    fn connector_type(&self) -> &str {
        "freshdesk"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        let payload = &event.payload;

        match event.event_type.as_str() {
            "ticket_created" => {
                let id = payload["id"].as_u64().unwrap_or(0);
                let subject = payload["subject"].as_str().unwrap_or("ticket");
                let desc =
                    payload["description_text"].as_str().or_else(|| payload["description"].as_str()).unwrap_or("");
                let priority = match payload["priority"].as_u64().unwrap_or(1) {
                    1 => "low",
                    2 => "medium",
                    3 => "high",
                    4 => "urgent",
                    _ => "normal",
                };
                let requester = payload["requester"]["name"].as_str().unwrap_or("customer");
                let email = payload["requester"]["email"].as_str().unwrap_or("");

                Ok(Some(format!(
                    "New Freshdesk ticket #{id} ({priority} priority) from {requester} ({email}). \
                     Subject: {subject}. Description: {desc}. \
                     Search the knowledge base for similar resolved tickets. \
                     Draft a response following support guidelines. \
                     Flag for escalation if this requires engineering attention.",
                )))
            }

            "ticket_updated" => {
                let id = payload["id"].as_u64().unwrap_or(0);
                let subject = payload["subject"].as_str().unwrap_or("ticket");
                let changes = payload.get("changes").cloned().unwrap_or_default();

                // Only act if status changed to waiting-on-us or escalated
                let status_changed = changes.get("status").is_some();
                if !status_changed {
                    return Ok(None);
                }

                let new_status = changes["status"]
                    .as_array()
                    .and_then(|arr| arr.get(1))
                    .and_then(|v| v.as_u64())
                    .map(|s| match s {
                        2 => "open",
                        3 => "pending",
                        4 => "resolved",
                        5 => "closed",
                        _ => "updated",
                    })
                    .unwrap_or("updated");

                Ok(Some(format!(
                    "Freshdesk ticket #{id} '{subject}' status changed to {new_status}. \
                     Review the ticket and determine if any action is needed.",
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
        let auth = Self::auth(config).ok_or_else(|| anyhow::anyhow!("missing Freshdesk api_key"))?;
        let base = Self::base_url(config);

        let is_private = metadata.get("private").and_then(|v| v.as_bool()).unwrap_or(true);

        let url = format!("{}/tickets/{}/notes", base, external_id);
        let body = serde_json::json!({
            "body":    output,
            "private": is_private,
        });

        let resp = self
            .http
            .post(&url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Freshdesk note creation failed {status}: {text}");
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let auth = Self::auth(config).ok_or_else(|| anyhow::anyhow!("missing 'api_key' in credentials"))?;
        let domain = Self::domain(config);
        if domain.is_empty() {
            anyhow::bail!("missing 'domain' in settings (e.g. 'acme' for acme.freshdesk.com)");
        }

        let url = format!("https://{}.freshdesk.com/api/v2/tickets?per_page=1", domain);
        let resp = self.http.get(&url).header("Authorization", auth).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Freshdesk auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}
