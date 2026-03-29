//! Twilio connector - SMS and call-center workflows.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct TwilioConnector {
    http: reqwest::Client,
}

impl TwilioConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn account_sid(config: &ConnectorConfig) -> Option<String> {
        config
            .credentials
            .get("account_sid")
            .or_else(|| config.credentials.get("sid"))
            .or_else(|| config.settings.get("account_sid"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn auth_token(config: &ConnectorConfig) -> Option<String> {
        config
            .credentials
            .get("auth_token")
            .or_else(|| config.credentials.get("api_key"))
            .or_else(|| config.credentials.get("token"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn from_number(config: &ConnectorConfig) -> Option<String> {
        config
            .settings
            .get("from_number")
            .or_else(|| config.credentials.get("from_number"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn api_base(account_sid: &str) -> String {
        format!("https://api.twilio.com/2010-04-01/Accounts/{account_sid}")
    }
}

#[async_trait]
impl Connector for TwilioConnector {
    fn connector_type(&self) -> &str {
        "twilio"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "sms.received" | "message.received" => {
                let from = event.payload.get("From").and_then(|v| v.as_str()).unwrap_or("unknown");
                let body = event.payload.get("Body").and_then(|v| v.as_str()).unwrap_or("");
                Ok(Some(format!(
                    "Incoming SMS from {from}: {body}. Triage the request, identify urgency, and draft a concise response."
                )))
            }
            "call.received" | "voice.call_received" | "voicemail.received" => {
                let from = event.payload.get("From").and_then(|v| v.as_str()).unwrap_or("unknown");
                let call_sid = event.payload.get("CallSid").and_then(|v| v.as_str()).unwrap_or("unknown");
                Ok(Some(format!(
                    "Incoming Twilio call {call_sid} from {from}. Review caller context, route to the correct queue, and prepare call notes."
                )))
            }
            "call.completed" => {
                let from = event.payload.get("From").and_then(|v| v.as_str()).unwrap_or("unknown");
                let duration = event.payload.get("CallDuration").and_then(|v| v.as_str()).unwrap_or("0");
                Ok(Some(format!(
                    "Completed Twilio call from {from} lasting {duration} seconds. Summarize disposition, next action, and any promised follow-up."
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
        _metadata: &serde_json::Value,
    ) -> Result<()> {
        let account_sid = Self::account_sid(config).ok_or_else(|| anyhow::anyhow!("missing Twilio account_sid"))?;
        let auth_token = Self::auth_token(config).ok_or_else(|| anyhow::anyhow!("missing Twilio auth_token"))?;
        let from = Self::from_number(config).ok_or_else(|| anyhow::anyhow!("missing Twilio from_number"))?;

        let url = format!("{}/Messages.json", Self::api_base(&account_sid));
        let resp = self
            .http
            .post(&url)
            .basic_auth(&account_sid, Some(auth_token))
            .form(&[("To", external_id), ("From", &from), ("Body", output)])
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Twilio message delivery failed: {}", resp.status());
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let account_sid = Self::account_sid(config).ok_or_else(|| anyhow::anyhow!("missing account_sid"))?;
        let auth_token = Self::auth_token(config).ok_or_else(|| anyhow::anyhow!("missing auth_token"))?;

        let url = format!("{}.json", Self::api_base(&account_sid));
        let resp = self.http.get(&url).basic_auth(&account_sid, Some(auth_token)).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Twilio auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for TwilioConnector {
    fn default() -> Self {
        Self::new()
    }
}
