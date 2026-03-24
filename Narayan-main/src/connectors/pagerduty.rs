//! PagerDuty connector — IT Operations and ITSM workflows.
//!
//! Triggers from PagerDuty webhooks:
//! - Incident triggered → runbook execution agent
//! - Incident acknowledged → postmortem preparation agent
//! - Service degraded → health check and triage agent

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct PagerDutyConnector {
    http: reqwest::Client,
}

impl PagerDutyConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn api_key(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("api_key").and_then(|v| v.as_str()).map(String::from)
    }
}

#[async_trait]
impl Connector for PagerDutyConnector {
    fn connector_type(&self) -> &str {
        "pagerduty"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "incident.triggered" => {
                let id = event.payload["id"].as_str().unwrap_or("unknown");
                let title = event.payload["title"].as_str().unwrap_or("incident");
                let service = event.payload["service"]["summary"].as_str().unwrap_or("unknown service");
                let urgency = event.payload["urgency"].as_str().unwrap_or("high");
                Ok(Some(format!(
                    "PagerDuty incident {id} triggered on {service} (urgency: {urgency}): '{title}'. \
                     Execute the runbook: \
                     1. Check service health endpoints and recent logs \
                     2. Identify the failure scope (single host vs cluster-wide) \
                     3. Check recent deployments and config changes \
                     4. Attempt automated mitigation if runbook permits \
                     5. Post a status update to the incident within 5 minutes \
                     Log every action taken with timestamp.",
                )))
            }
            "incident.resolved" => {
                let id = event.payload["id"].as_str().unwrap_or("unknown");
                let title = event.payload["title"].as_str().unwrap_or("incident");
                let dur = event.payload["duration_seconds"].as_u64().unwrap_or(0);
                Ok(Some(format!(
                    "Prepare postmortem for resolved PagerDuty incident {id}: '{title}' ({dur}s duration). \
                     Gather: timeline of events, commands run, metrics during impact window, \
                     contributing factors, affected customers/systems. \
                     Write a structured postmortem document with: \
                     summary, timeline, root cause, contributing factors, \
                     action items with owners and due dates.",
                )))
            }
            "service.degraded" => {
                let service = event.payload["service"]["summary"].as_str().unwrap_or("service");
                Ok(Some(format!(
                    "Service '{service}' is degraded. \
                     Run health checks across all instances. \
                     Compare current metrics against baseline from the past 7 days. \
                     Identify the degraded component. \
                     Recommend immediate mitigation steps. \
                     Do NOT make any changes — produce a triage report only.",
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
        let key = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing PagerDuty api_key"))?;

        let delivery = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("note");
        match delivery {
            "incident_note" => {
                let url = format!("https://api.pagerduty.com/incidents/{external_id}/notes");
                self.http
                    .post(&url)
                    .header("Authorization", format!("Token token={key}"))
                    .header("Accept", "application/vnd.pagerduty+json;version=2")
                    .json(&serde_json::json!({ "note": { "content": output } }))
                    .send()
                    .await?;
            }
            "status_update" => {
                let url = format!("https://api.pagerduty.com/incidents/{external_id}");
                self.http
                    .put(&url)
                    .header("Authorization", format!("Token token={key}"))
                    .header("Accept", "application/vnd.pagerduty+json;version=2")
                    .json(&serde_json::json!({
                        "incident": {
                            "type": "incident_reference",
                            "status_update": output,
                        }
                    }))
                    .send()
                    .await?;
            }
            _ => {
                tracing::info!(incident_id = external_id, delivery, "PagerDuty output logged");
            }
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let key = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing api_key"))?;
        let resp = self
            .http
            .get("https://api.pagerduty.com/abilities")
            .header("Authorization", format!("Token token={key}"))
            .header("Accept", "application/vnd.pagerduty+json;version=2")
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("PagerDuty auth failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for PagerDutyConnector {
    fn default() -> Self {
        Self::new()
    }
}
