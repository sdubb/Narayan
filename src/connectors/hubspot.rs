//! HubSpot connector — marketing and growth workflows.
//!
//! Triggers: contact property change, deal stage change, form submission.
//! Delivers: contact notes, deal updates, task creation.

use anyhow::Result;
use async_trait::async_trait;
use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct HubSpotConnector { http: reqwest::Client }

impl HubSpotConnector {
    pub fn new() -> Self { Self { http: reqwest::Client::new() } }

    fn token(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("access_token").and_then(|v| v.as_str()).map(String::from)
    }
}

#[async_trait]
impl Connector for HubSpotConnector {
    fn connector_type(&self) -> &str { "hubspot" }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "contact.propertyChange" => {
                let email    = event.payload["email"].as_str().unwrap_or("unknown");
                let property = event.payload["propertyName"].as_str().unwrap_or("property");
                let value    = event.payload["propertyValue"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "HubSpot contact {email} updated {property} to '{value}'. \
                     Research the contact's company and recent activity. \
                     Determine if this change signals buying intent. \
                     Draft a personalised follow-up email if appropriate. \
                     Save enriched contact notes back to HubSpot."
                )))
            }
            "deal.stageChange" => {
                let deal_name  = event.payload["dealName"].as_str().unwrap_or("deal");
                let new_stage  = event.payload["newStage"].as_str().unwrap_or("unknown");
                let deal_value = event.payload["amount"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "HubSpot deal '{deal_name}' (${deal_value}) moved to stage '{new_stage}'. \
                     Research the company for recent news and competitive landscape. \
                     Recommend the 3 highest-impact next actions for this stage. \
                     Draft a stage-appropriate follow-up message."
                )))
            }
            "form.submission" => {
                let form_name = event.payload["formName"].as_str().unwrap_or("form");
                let email     = event.payload["email"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "New HubSpot form submission '{form_name}' from {email}. \
                     Research the company and person. Qualify the lead. \
                     Draft a personalised welcome email with relevant content. \
                     Assign a lead score based on fit and intent signals."
                )))
            }
            _ => Ok(None),
        }
    }

    async fn deliver_output(&self, config: &ConnectorConfig, external_id: &str, output: &str, metadata: &serde_json::Value) -> Result<()> {
        let token    = Self::token(config).ok_or_else(|| anyhow::anyhow!("missing HubSpot access_token"))?;
        let delivery = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("note");
        match delivery {
            "note" => {
                self.http.post("https://api.hubapi.com/crm/v3/objects/notes")
                    .bearer_auth(&token)
                    .json(&serde_json::json!({ "properties": { "hs_note_body": output, "hs_attachment_ids": external_id } }))
                    .send().await?;
            }
            "task" => {
                self.http.post("https://api.hubapi.com/crm/v3/objects/tasks")
                    .bearer_auth(&token)
                    .json(&serde_json::json!({ "properties": { "hs_task_body": output, "hs_task_subject": "Narayan follow-up" } }))
                    .send().await?;
            }
            _ => { tracing::info!(external_id, "HubSpot output logged"); }
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let token = Self::token(config).ok_or_else(|| anyhow::anyhow!("missing access_token"))?;
        let resp  = self.http.get("https://api.hubapi.com/crm/v3/objects/contacts?limit=1")
            .bearer_auth(&token).send().await?;
        if !resp.status().is_success() { anyhow::bail!("HubSpot auth failed: {}", resp.status()); }
        Ok(())
    }
}

impl Default for HubSpotConnector { fn default() -> Self { Self::new() } }
