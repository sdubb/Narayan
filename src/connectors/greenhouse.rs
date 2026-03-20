//! Greenhouse ATS connector — HR & People Ops workflows.
//!
//! Triggers: application created, interview scheduled, offer sent.
//! Delivers: candidate notes, scorecard updates, rejection reasons.

use anyhow::Result;
use async_trait::async_trait;
use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct GreenhouseConnector { http: reqwest::Client }

impl GreenhouseConnector {
    pub fn new() -> Self { Self { http: reqwest::Client::new() } }

    fn api_key(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("api_key").and_then(|v| v.as_str()).map(String::from)
    }

    fn base_url() -> &'static str { "https://harvest.greenhouse.io/v1" }
}

#[async_trait]
impl Connector for GreenhouseConnector {
    fn connector_type(&self) -> &str { "greenhouse" }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "application" => {
                let candidate = event.payload["candidate"]["name"].as_str().unwrap_or("candidate");
                let job_name  = event.payload["job"]["name"].as_str().unwrap_or("role");
                let app_id    = event.payload["id"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "New Greenhouse application {app_id}: {candidate} for '{job_name}'. \
                     Review the resume against the job requirements. \
                     Score the candidate on: required skills, experience level, culture fit indicators. \
                     Flag any gaps or standout strengths. \
                     Produce a structured screening summary for the hiring manager. \
                     Do NOT make a hire/reject decision — surface findings for human review only."
                )))
            }
            "interview" => {
                let candidate   = event.payload["candidate"]["name"].as_str().unwrap_or("candidate");
                let interviewer = event.payload["interviewer"]["name"].as_str().unwrap_or("interviewer");
                let job_name    = event.payload["job"]["name"].as_str().unwrap_or("role");
                Ok(Some(format!(
                    "Interview scheduled: {candidate} with {interviewer} for '{job_name}'. \
                     Prepare a briefing pack for the interviewer: \
                     candidate background summary, suggested questions per competency, \
                     role-specific technical scenarios, previous interview notes if any. \
                     Format as a one-page document ready to use in the interview."
                )))
            }
            "offer" => {
                let candidate = event.payload["candidate"]["name"].as_str().unwrap_or("candidate");
                let job_name  = event.payload["job"]["name"].as_str().unwrap_or("role");
                Ok(Some(format!(
                    "Offer sent to {candidate} for '{job_name}'. \
                     Prepare onboarding checklist: \
                     IT equipment requests, system access list, first-week schedule, \
                     buddy assignment, mandatory training modules. \
                     Draft a personalised welcome email from the hiring manager."
                )))
            }
            _ => Ok(None),
        }
    }

    async fn deliver_output(&self, config: &ConnectorConfig, external_id: &str, output: &str, metadata: &serde_json::Value) -> Result<()> {
        let key      = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing Greenhouse api_key"))?;
        let delivery = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("note");
        match delivery {
            "note" => {
                let url = format!("{}/candidates/{external_id}/activity_feed/notes", Self::base_url());
                self.http.post(&url)
                    .basic_auth(&key, Some(""))
                    .json(&serde_json::json!({ "body": output, "visibility": "private" }))
                    .send().await?;
            }
            _ => { tracing::info!(external_id, "Greenhouse output logged"); }
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let key  = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing api_key"))?;
        let url  = format!("{}/users?per_page=1", Self::base_url());
        let resp = self.http.get(&url).basic_auth(&key, Some("")).send().await?;
        if !resp.status().is_success() { anyhow::bail!("Greenhouse auth failed: {}", resp.status()); }
        Ok(())
    }
}

impl Default for GreenhouseConnector { fn default() -> Self { Self::new() } }
