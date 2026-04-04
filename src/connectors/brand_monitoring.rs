//! Brand Protection & Monitoring connector — website surveillance and change detection.
//!
//! Triggers: website content changed, defacement detected, competitor announcement,
//!          social mention detected, uptime issue.
//! Delivers: escalation alerts, incident creation, notification delivery.

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};
use anyhow::Result;
use async_trait::async_trait;

pub struct BrandMonitoringConnector {
    http: reqwest::Client,
}

impl BrandMonitoringConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn get_webhook_secret(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("webhook_secret").and_then(|v| v.as_str()).map(String::from)
    }

    fn get_api_token(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("api_token").and_then(|v| v.as_str()).map(String::from)
    }
}

#[async_trait]
impl Connector for BrandMonitoringConnector {
    fn connector_type(&self) -> &str {
        "brand_monitoring"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "website_defacement_detected" => {
                let url = event.payload["url"].as_str().unwrap_or("unknown");
                let change_summary = event.payload["change_summary"].as_str().unwrap_or("modification");
                let severity = event.payload["severity"].as_str().unwrap_or("medium");
                Ok(Some(format!(
                    "⚠️ BRAND ALERT: Website defacement detected at {url}\n\
                     Severity: {severity}\n\
                     Change detected: {change_summary}\n\
                     ACTION REQUIRED:\n\
                     1. Fetch the current website using web_fetch\n\
                     2. Take a screenshot to document the unauthorized change\n\
                     3. Compare with known legitimate content (search for cached versions)\n\
                     4. Generate incident report with exact differences\n\
                     5. Create alert notification"
                )))
            }
            "website_content_changed" => {
                let url = event.payload["url"].as_str().unwrap_or("unknown");
                let page_section = event.payload["page_section"].as_str().unwrap_or("homepage");
                let timestamp = event.payload["timestamp"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "📝 BRAND TRACKING: Content change detected on {url} at {timestamp}\n\
                     Section: {page_section}\n\
                     ACTION REQUIRED:\n\
                     1. Fetch the current page content\n\
                     2. Take a screenshot for documentation\n\
                     3. Extract and summarize the key changes\n\
                     4. Compare with previous version to identify what changed\n\
                     5. Log change to brand monitoring record with citations"
                )))
            }
            "competitor_announcement" => {
                let competitor_name = event.payload["competitor_name"].as_str().unwrap_or("competitor");
                let announcement = event.payload["announcement"].as_str().unwrap_or("update");
                Ok(Some(format!(
                    "🔍 COMPETITOR INTEL: {competitor_name} made an announcement\n\
                     Announcement: {announcement}\n\
                     ACTION REQUIRED:\n\
                     1. Fetch competitor's announcement page/post\n\
                     2. Extract full announcement details\n\
                     3. Identify key differentiators from our offering\n\
                     4. Generate competitive analysis with sources\n\
                     5. Flag for marketing/product team review"
                )))
            }
            "website_uptime_issue" => {
                let url = event.payload["url"].as_str().unwrap_or("unknown");
                let status_code = event.payload["status_code"].as_i64().unwrap_or(0);
                let downtime_minutes = event.payload["downtime_minutes"].as_i64().unwrap_or(0);
                Ok(Some(format!(
                    "🚨 CRITICAL: Website outage detected for {url}\n\
                     Status: {status_code} | Downtime: {downtime_minutes} minutes\n\
                     ACTION REQUIRED:\n\
                     1. Verify current website status using web_fetch\n\
                     2. Check for DNS/server issues\n\
                     3. Escalate to incident management\n\
                     4. Notify ops team immediately\n\
                     5. Create critical incident"
                )))
            }
            "social_mention_detected" => {
                let platform = event.payload["platform"].as_str().unwrap_or("social");
                let mention = event.payload["mention"].as_str().unwrap_or("mention");
                let sentiment = event.payload["sentiment"].as_str().unwrap_or("neutral");
                Ok(Some(format!(
                    "💬 SOCIAL ALERT: Mention detected on {platform}\n\
                     Sentiment: {sentiment}\n\
                     Content: {mention}\n\
                     ACTION REQUIRED:\n\
                     1. Search for full context of the mention\n\
                     2. Analyze sentiment and impact\n\
                     3. If negative: assess response strategy\n\
                     4. Generate summary for reputation team\n\
                     5. Archive with source links for compliance"
                )))
            }
            "trademark_violation" => {
                let violation_source = event.payload["violation_source"].as_str().unwrap_or("unknown");
                let violation_details = event.payload["violation_details"].as_str().unwrap_or("details");
                Ok(Some(format!(
                    "⚖️ LEGAL ALERT: Potential trademark violation detected\n\
                     Source: {violation_source}\n\
                     Details: {violation_details}\n\
                     ACTION REQUIRED:\n\
                     1. Document the violation with screenshots\n\
                     2. Preserve evidence and URLs\n\
                     3. Escalate to legal team\n\
                     4. Prepare takedown notice if needed\n\
                     5. File formal report"
                )))
            }
            _ => Ok(None),
        }
    }

    async fn deliver_output(
        &self,
        config: &ConnectorConfig,
        _external_id: &str,
        output: &str,
        _metadata: &serde_json::Value,
    ) -> Result<()> {
        // Deliver alert outputs to configured channels
        // This could be: Slack, email, PagerDuty, ticketing system, etc.
        let notification_channels = config.settings.get("notification_channels").and_then(|v| v.as_array()).cloned();

        if let Some(channels) = notification_channels {
            for channel in channels {
                let channel_type = channel.get("type").and_then(|v| v.as_str()).unwrap_or("email");
                let channel_target = channel.get("target").and_then(|v| v.as_str()).unwrap_or("default");

                match channel_type {
                    "slack" => {
                        // Slack webhook delivery
                        let webhook_url = channel.get("webhook_url").and_then(|v| v.as_str());
                        if let Some(url) = webhook_url {
                            self.http
                                .post(url)
                                .json(&serde_json::json!({
                                    "text": output,
                                    "blocks": [{
                                        "type": "section",
                                        "text": { "type": "mrkdwn", "text": output }
                                    }]
                                }))
                                .send()
                                .await?;
                        }
                    }
                    "email" => {
                        // Email delivery
                        tracing::info!("Email alert would be sent to: {} | {}", channel_target, output);
                    }
                    "webhook" => {
                        // Generic webhook delivery
                        if let Some(url) = channel.get("url").and_then(|v| v.as_str()) {
                            self.http
                                .post(url)
                                .json(&serde_json::json!({
                                    "alert": output,
                                    "timestamp": chrono::Utc::now().to_rfc3339()
                                }))
                                .send()
                                .await?;
                        }
                    }
                    _ => {
                        tracing::warn!("Unknown notification channel type: {}", channel_type);
                    }
                }
            }
        }

        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        // Validate that monitored URLs are accessible
        let urls = config.settings.get("monitored_urls").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        if urls.is_empty() {
            anyhow::bail!("No monitored URLs configured");
        }

        // Test connectivity to at least one URL
        let test_url =
            urls.first().and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("Invalid monitored URL format"))?;

        let resp = self.http.head(test_url).send().await?;
        if !resp.status().is_success() && resp.status().as_u16() != 401 && resp.status().as_u16() != 403 {
            anyhow::bail!("Cannot reach monitored URL: {} ({})", test_url, resp.status());
        }

        Ok(())
    }
}

impl Default for BrandMonitoringConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_website_defacement_event() {
        let connector = BrandMonitoringConnector::new();
        let event = ConnectorEvent {
            connector_type: "brand_monitoring".to_string(),
            event_type: "website_defacement_detected".to_string(),
            payload: serde_json::json!({
                "url": "https://example.com",
                "change_summary": "Homepage logo replaced with unauthorized content",
                "severity": "critical"
            }),
            tenant_id: "test-tenant".to_string(),
            external_id: Some("alert-001".to_string()),
        };

        let config = ConnectorConfig {
            id: "brand1".to_string(),
            tenant_id: "test-tenant".to_string(),
            connector_type: "brand_monitoring".to_string(),
            credentials: serde_json::json!({}),
            settings: serde_json::json!({}),
            enabled: true,
        };

        let result = connector.handle_inbound(&event, &config).await;
        assert!(result.is_ok());
        let goal = result.unwrap();
        assert!(goal.is_some());
        assert!(goal.unwrap().contains("BRAND ALERT"));
    }

    #[tokio::test]
    async fn test_competitor_announcement_event() {
        let connector = BrandMonitoringConnector::new();
        let event = ConnectorEvent {
            connector_type: "brand_monitoring".to_string(),
            event_type: "competitor_announcement".to_string(),
            payload: serde_json::json!({
                "competitor_name": "RivalCorp",
                "announcement": "Launched new AI-powered feature"
            }),
            tenant_id: "test-tenant".to_string(),
            external_id: None,
        };

        let config = ConnectorConfig {
            id: "brand1".to_string(),
            tenant_id: "test-tenant".to_string(),
            connector_type: "brand_monitoring".to_string(),
            credentials: serde_json::json!({}),
            settings: serde_json::json!({}),
            enabled: true,
        };

        let result = connector.handle_inbound(&event, &config).await;
        assert!(result.is_ok());
        let goal = result.unwrap();
        assert!(goal.is_some());
        assert!(goal.unwrap().contains("COMPETITOR INTEL"));
    }
}
