//! Intercom connector — conversation and customer support workflows.
//!
//! Receives Intercom webhook events (new conversations, replies, mentions)
//! and delivers agent responses as replies or notes.
//!
//! Auth: Bearer token (API key from Intercom Developer Hub)
//! Settings: none required
//!
//! Webhook events handled:
//!   conversation.created      → new conversation assigned to support agent
//!   conversation.replied      → customer replied, needs response
//!   conversation_part.created → team member mentioned @narayan

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct IntercomConnector {
    http: reqwest::Client,
}

impl IntercomConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::builder()
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert("Accept", "application/json".parse().unwrap());
                h.insert("Intercom-Version", "2.10".parse().unwrap());
                h
            })
            .build()
            .unwrap_or_default()
        }
    }

    fn token(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("access_token")
            .or_else(|| config.credentials.get("api_key"))
            .or_else(|| config.credentials.get("token"))
            .and_then(|v| v.as_str())
            .map(String::from)
    }
}

#[async_trait]
impl Connector for IntercomConnector {
    fn connector_type(&self) -> &str { "intercom" }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        let payload = &event.payload;

        match event.event_type.as_str() {
            "conversation.created" => {
                let conv_id   = payload["data"]["item"]["id"].as_str().unwrap_or("unknown");
                let subject   = payload["data"]["item"]["source"]["subject"].as_str().unwrap_or("(no subject)");
                let body      = payload["data"]["item"]["source"]["body"].as_str().unwrap_or("");
                let author    = payload["data"]["item"]["source"]["author"]["name"].as_str().unwrap_or("customer");
                let email     = payload["data"]["item"]["source"]["author"]["email"].as_str().unwrap_or("");

                Ok(Some(format!(
                    "New Intercom conversation #{conv_id} from {author} ({email}). \
                     Subject: {subject}. Message: {body}. \
                     Search the knowledge base for relevant help articles. \
                     Draft a helpful, concise reply. \
                     If this is a billing or technical issue that needs escalation, say so clearly.",
                )))
            }

            "conversation.replied" => {
                let conv_id = payload["data"]["item"]["id"].as_str().unwrap_or("unknown");
                let body    = payload["data"]["item"]["conversation_parts"]["conversation_parts"]
                    .as_array()
                    .and_then(|arr| arr.last())
                    .and_then(|p| p["body"].as_str())
                    .unwrap_or("");
                let author  = payload["data"]["item"]["conversation_parts"]["conversation_parts"]
                    .as_array()
                    .and_then(|arr| arr.last())
                    .and_then(|p| p["author"]["name"].as_str())
                    .unwrap_or("customer");

                Ok(Some(format!(
                    "Customer {author} replied to Intercom conversation #{conv_id}: {body}. \
                     Review the full conversation history and provide a helpful follow-up response.",
                )))
            }

            "conversation_part.created" => {
                let body = payload["data"]["item"]["body"].as_str().unwrap_or("");
                if !body.to_lowercase().contains("@narayan") {
                    return Ok(None);
                }
                let conv_id = payload["data"]["item"]["conversation_id"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "Intercom team member requested help on conversation #{conv_id}: {body}. \
                     Review the conversation and provide an expert response.",
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
        let token = Self::token(config)
            .ok_or_else(|| anyhow::anyhow!("missing Intercom access_token"))?;

        let msg_type = metadata.get("message_type").and_then(|v| v.as_str()).unwrap_or("comment");
        // "comment" = internal note, "reply" = customer-visible reply
        let is_note = msg_type == "note" || msg_type == "comment";

        let url = format!("https://api.intercom.io/conversations/{}/reply", external_id);
        let body = serde_json::json!({
            "message_type": if is_note { "note" } else { "reply" },
            "type":         "admin",
            "admin_id":     config.settings.get("admin_id").and_then(|v| v.as_str()).unwrap_or(""),
            "body":         output,
        });

        let resp = self.http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Intercom reply failed {status}: {text}");
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let token = Self::token(config)
            .ok_or_else(|| anyhow::anyhow!("missing 'access_token' or 'api_key' in credentials"))?;

        let resp = self.http
            .get("https://api.intercom.io/me")
            .bearer_auth(&token)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Intercom auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}
