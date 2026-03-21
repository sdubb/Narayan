//! Notion connector — research and knowledge management workflows.
//!
//! Triggers: database item created, page updated, mention.
//! Delivers: page content, database updates, comments.

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};
use anyhow::Result;
use async_trait::async_trait;

pub struct NotionConnector {
    http: reqwest::Client,
}

impl NotionConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn token(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("api_key").and_then(|v| v.as_str()).map(String::from)
    }
}

#[async_trait]
impl Connector for NotionConnector {
    fn connector_type(&self) -> &str {
        "notion"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "database_item_created" => {
                let title = event.payload["title"].as_str().unwrap_or("item");
                let db_name = event.payload["database_name"].as_str().unwrap_or("database");
                Ok(Some(format!(
                    "New Notion item '{title}' created in '{db_name}'. \
                     Research the topic thoroughly using web sources. \
                     Enrich the page with: key facts, related concepts, source links, and a summary. \
                     Store all findings back to the Notion page."
                )))
            }
            "research_request" => {
                let topic = event.payload["topic"].as_str().unwrap_or("topic");
                let page_id = event.payload["page_id"].as_str().unwrap_or("");
                Ok(Some(format!(
                    "Research request for Notion page {page_id}: '{topic}'. \
                     Conduct thorough multi-source research. \
                     Produce a structured report: executive summary, key findings, \
                     data points with citations, conflicting viewpoints, conclusion. \
                     Write the report back to the Notion page in blocks."
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
        let token = Self::token(config).ok_or_else(|| anyhow::anyhow!("missing Notion api_key"))?;
        let url = format!("https://api.notion.com/v1/blocks/{external_id}/children");
        self.http
            .patch(&url)
            .bearer_auth(&token)
            .header("Notion-Version", "2022-06-28")
            .json(&serde_json::json!({
                "children": [{
                    "object": "block",
                    "type": "paragraph",
                    "paragraph": { "rich_text": [{ "type": "text", "text": { "content": output } }] }
                }]
            }))
            .send()
            .await?;
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let token = Self::token(config).ok_or_else(|| anyhow::anyhow!("missing api_key"))?;
        let resp = self
            .http
            .get("https://api.notion.com/v1/users/me")
            .bearer_auth(&token)
            .header("Notion-Version", "2022-06-28")
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Notion auth failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for NotionConnector {
    fn default() -> Self {
        Self::new()
    }
}
