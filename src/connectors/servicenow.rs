//! ServiceNow connector — IT service management / compliance workflows.
//!
//! Handles incident creation, change requests, and compliance tasks.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct ServiceNowConnector {
    http: reqwest::Client,
}

impl ServiceNowConnector {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    fn instance_url(config: &ConnectorConfig) -> &str {
        config.settings.get("instance_url").and_then(|v| v.as_str()).unwrap_or("")
    }

    fn auth_header(config: &ConnectorConfig) -> Option<String> {
        let user = config.credentials.get("username").and_then(|v| v.as_str())?;
        let pass = config.credentials.get("password").and_then(|v| v.as_str())?;
        let credentials = format!("{}:{}", user, pass);
        Some(format!("Basic {}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, credentials)))
    }
}

#[async_trait]
impl Connector for ServiceNowConnector {
    fn connector_type(&self) -> &str {
        "servicenow"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "incident_created" => {
                let short_desc = event.payload.get("short_description").and_then(|v| v.as_str()).unwrap_or("incident");
                let description = event.payload.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let number = event.payload.get("number").and_then(|v| v.as_str()).unwrap_or("unknown");
                let urgency = event.payload.get("urgency").and_then(|v| v.as_str()).unwrap_or("3");

                Ok(Some(format!(
                    "Investigate ServiceNow incident {} (urgency {}): {}. Details: {}. \
                     Check monitoring tools, logs, and knowledge base. \
                     Provide root cause analysis and recommended resolution.",
                    number, urgency, short_desc, description
                )))
            }
            "change_request" => {
                let short_desc = event.payload.get("short_description").and_then(|v| v.as_str()).unwrap_or("change");
                let description = event.payload.get("description").and_then(|v| v.as_str()).unwrap_or("");

                Ok(Some(format!(
                    "Review change request: {}. Details: {}. \
                     Assess risk, check for conflicts, and provide implementation plan.",
                    short_desc, description
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
        let instance = Self::instance_url(config);
        let auth = Self::auth_header(config).ok_or_else(|| anyhow::anyhow!("missing ServiceNow credentials"))?;

        let url = format!("{}/api/now/table/incident/{}", instance, external_id);
        self.http
            .patch(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "work_notes": output,
            }))
            .send()
            .await?;

        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let instance = Self::instance_url(config);
        let auth = Self::auth_header(config).ok_or_else(|| anyhow::anyhow!("missing ServiceNow credentials"))?;

        let url = format!("{}/api/now/table/sys_user?sysparm_limit=1", instance);
        let resp = self.http.get(&url).header("Authorization", &auth).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("ServiceNow auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}
