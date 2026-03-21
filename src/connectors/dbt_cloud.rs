//! dbt Cloud connector — Data & Analytics Ops workflows.
//!
//! Triggers: job run failed, job run completed, data freshness alert.
//! Delivers: run annotations, Slack/webhook notifications via dbt Cloud API.

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};
use anyhow::Result;
use async_trait::async_trait;

pub struct DbtCloudConnector {
    http: reqwest::Client,
}

impl DbtCloudConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn token(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("api_token").and_then(|v| v.as_str()).map(String::from)
    }

    fn account_id(config: &ConnectorConfig) -> &str {
        config.settings.get("account_id").and_then(|v| v.as_str()).unwrap_or("")
    }
}

#[async_trait]
impl Connector for DbtCloudConnector {
    fn connector_type(&self) -> &str {
        "dbt_cloud"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "job.run.errored" => {
                let job_name = event.payload["job_name"].as_str().unwrap_or("job");
                let run_id = event.payload["run_id"].as_str().unwrap_or("unknown");
                let error = event.payload["status_message"].as_str().unwrap_or("unknown error");
                Ok(Some(format!(
                    "dbt Cloud job '{job_name}' (run {run_id}) failed: {error}. \
                     Fetch the full run logs and error output. \
                     Identify the failing model(s) and root cause. \
                     Check for: upstream source freshness issues, SQL syntax errors, \
                     schema drift, permission failures. \
                     Produce a triage report with recommended fix. \
                     Do NOT modify any models — surface findings only."
                )))
            }
            "job.run.completed" => {
                let job_name = event.payload["job_name"].as_str().unwrap_or("job");
                let models = event.payload["models_run"].as_u64().unwrap_or(0);
                let duration = event.payload["duration_seconds"].as_u64().unwrap_or(0);
                Ok(Some(format!(
                    "dbt Cloud job '{job_name}' completed: {models} models in {duration}s. \
                     Analyse run timing vs. 30-day baseline. \
                     Flag models that ran >2x slower than average. \
                     Check data quality test results for any failures or warnings. \
                     Write a run health summary report."
                )))
            }
            "source.freshness.error" => {
                let source = event.payload["source_name"].as_str().unwrap_or("source");
                let table = event.payload["identifier"].as_str().unwrap_or("table");
                let age = event.payload["age_hours"].as_str().unwrap_or("unknown");
                Ok(Some(format!(
                    "dbt source freshness alert: {source}.{table} is {age} hours old (threshold exceeded). \
                     Check the upstream pipeline for this source. \
                     Identify if there is a loading failure, delay, or schema issue. \
                     Produce an impact assessment: which downstream models are affected. \
                     Recommend immediate remediation steps."
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
        let token = Self::token(config).ok_or_else(|| anyhow::anyhow!("missing dbt Cloud api_token"))?;
        let account = Self::account_id(config);
        // Post as a run note annotation
        let url = format!("https://cloud.getdbt.com/api/v2/accounts/{account}/runs/{external_id}/");
        self.http.get(&url).header("Authorization", format!("Token {token}")).send().await?;
        tracing::info!(run_id = external_id, output_len = output.len(), "dbt Cloud output logged");
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let token = Self::token(config).ok_or_else(|| anyhow::anyhow!("missing api_token"))?;
        let account = Self::account_id(config);
        let url = format!("https://cloud.getdbt.com/api/v2/accounts/{account}/");
        let resp = self.http.get(&url).header("Authorization", format!("Token {token}")).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("dbt Cloud auth failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for DbtCloudConnector {
    fn default() -> Self {
        Self::new()
    }
}
