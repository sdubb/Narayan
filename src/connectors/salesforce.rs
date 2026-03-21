//! Salesforce connector — CRM-driven sales and RevOps workflows.
//!
//! Triggers agent goals from Salesforce flows/webhooks:
//! - New lead created → research + enrichment agent
//! - Opportunity stage changed → next-step recommendation agent
//! - Account flagged for renewal → outreach preparation agent
//!
//! Delivers back to Salesforce via REST API: notes, task creation, field updates.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct SalesforceConnector {
    http: reqwest::Client,
}

impl SalesforceConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn instance_url(config: &ConnectorConfig) -> &str {
        config.settings.get("instance_url").and_then(|v| v.as_str()).unwrap_or("")
    }

    fn bearer(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("access_token").and_then(|v| v.as_str()).map(String::from)
    }
}

#[async_trait]
impl Connector for SalesforceConnector {
    fn connector_type(&self) -> &str {
        "salesforce"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "lead_created" => {
                let name = event.payload["Name"].as_str().unwrap_or("unknown");
                let company = event.payload["Company"].as_str().unwrap_or("unknown company");
                let email = event.payload["Email"].as_str().unwrap_or("");
                let id = event.payload["Id"].as_str().unwrap_or("");
                Ok(Some(format!(
                    "Research and enrich Salesforce lead {id}: {name} at {company} ({email}). \
                     Use web_search and data_extractor to find company size, funding, tech stack, \
                     and relevant news. Draft a personalised outreach email based on findings. \
                     Store enrichment data back to the lead record.",
                )))
            }
            "opportunity_stage_changed" => {
                let name = event.payload["Name"].as_str().unwrap_or("opportunity");
                let stage = event.payload["StageName"].as_str().unwrap_or("unknown");
                let amount = event.payload["Amount"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "Opportunity '{name}' moved to stage '{stage}' (${amount}). \
                     Research the account for recent news, competitors, and decision-maker changes. \
                     Recommend the 3 most impactful next actions for this stage. \
                     Draft a follow-up email appropriate for '{stage}'.",
                )))
            }
            "renewal_alert" => {
                let account = event.payload["AccountName"].as_str().unwrap_or("account");
                let days = event.payload["DaysUntilRenewal"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "Account '{account}' has renewal in {days} days. \
                     Analyse their usage data and support history. \
                     Identify expansion opportunities and risk signals. \
                     Prepare a renewal briefing document for the account executive.",
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
        let instance = Self::instance_url(config);
        let token = Self::bearer(config).ok_or_else(|| anyhow::anyhow!("missing Salesforce access_token"))?;

        let object_type = metadata.get("object_type").and_then(|v| v.as_str()).unwrap_or("Lead");
        let delivery = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("note");

        match delivery {
            "note" => {
                // Create a Chatter note or activity on the record
                let url = format!("{instance}/services/data/v58.0/sobjects/Note");
                self.http
                    .post(&url)
                    .bearer_auth(&token)
                    .json(&serde_json::json!({
                        "ParentId": external_id,
                        "Title":    "Narayan Agent Output",
                        "Body":     output,
                    }))
                    .send()
                    .await?;
            }
            "field_update" => {
                // Update a field on the record (e.g. enriched description)
                let field = metadata.get("field").and_then(|v| v.as_str()).unwrap_or("Description");
                let url = format!("{instance}/services/data/v58.0/sobjects/{object_type}/{external_id}");
                self.http.patch(&url).bearer_auth(&token).json(&serde_json::json!({ field: output })).send().await?;
            }
            "task" => {
                // Create a follow-up task
                let url = format!("{instance}/services/data/v58.0/sobjects/Task");
                self.http
                    .post(&url)
                    .bearer_auth(&token)
                    .json(&serde_json::json!({
                        "WhatId":   external_id,
                        "Subject":  "Narayan: follow-up required",
                        "Description": output,
                        "Status":   "Not Started",
                        "Priority": "Normal",
                    }))
                    .send()
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let instance = Self::instance_url(config);
        let token = Self::bearer(config).ok_or_else(|| anyhow::anyhow!("missing access_token"))?;
        let url = format!("{instance}/services/data/v58.0/limits");
        let resp = self.http.get(&url).bearer_auth(&token).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Salesforce auth failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for SalesforceConnector {
    fn default() -> Self {
        Self::new()
    }
}
