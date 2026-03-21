//! GitHub connector — webhook-driven PR/issue workflows.
//!
//! Receives GitHub webhook events and creates agent goals:
//! - PR opened → code review agent
//! - Issue created → triage/fix agent
//! - Check run requested → CI agent
//! - Comment with @narayan → response agent

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct GitHubConnector {
    http: reqwest::Client,
}

impl GitHubConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn api_token(config: &ConnectorConfig) -> Option<String> {
        config.credentials.get("token").and_then(|v| v.as_str()).map(String::from)
    }
}

#[async_trait]
impl Connector for GitHubConnector {
    fn connector_type(&self) -> &str {
        "github"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "pull_request" => {
                let action = event.payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if action == "opened" || action == "synchronize" {
                    let title = event.payload["pull_request"]["title"].as_str().unwrap_or("PR");
                    let url = event.payload["pull_request"]["html_url"].as_str().unwrap_or("");
                    let body = event.payload["pull_request"]["body"].as_str().unwrap_or("");
                    return Ok(Some(format!(
                        "Review pull request: {} ({}). Description: {}. \
                         Review the code changes, check for bugs, style issues, and security concerns. \
                         Post a review comment with your findings.",
                        title, url, body
                    )));
                }
                Ok(None)
            }
            "issues" => {
                let action = event.payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if action == "opened" {
                    let title = event.payload["issue"]["title"].as_str().unwrap_or("Issue");
                    let body = event.payload["issue"]["body"].as_str().unwrap_or("");
                    let url = event.payload["issue"]["html_url"].as_str().unwrap_or("");
                    return Ok(Some(format!(
                        "Triage and investigate issue: {} ({}). Description: {}. \
                         Analyze the issue, identify root cause, and propose a fix.",
                        title, url, body
                    )));
                }
                Ok(None)
            }
            "issue_comment" => {
                let body = event.payload["comment"]["body"].as_str().unwrap_or("");
                if body.contains("@narayan") {
                    let issue_title = event.payload["issue"]["title"].as_str().unwrap_or("issue");
                    return Ok(Some(format!("Respond to comment on '{}': {}", issue_title, body)));
                }
                Ok(None)
            }
            "check_run" => {
                let action = event.payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if action == "requested_action" {
                    let name = event.payload["check_run"]["name"].as_str().unwrap_or("check");
                    return Ok(Some(format!("Run CI check '{}' and report results.", name)));
                }
                Ok(None)
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
        let token = Self::api_token(config).ok_or_else(|| anyhow::anyhow!("no GitHub token configured"))?;
        let repo = config.settings.get("repo").and_then(|v| v.as_str()).unwrap_or("");

        // Determine delivery target: PR review, issue comment, or check run
        let delivery_type = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("comment");

        match delivery_type {
            "pr_review" => {
                let url = format!("https://api.github.com/repos/{}/pulls/{}/reviews", repo, external_id);
                self.http
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .header("Accept", "application/vnd.github.v3+json")
                    .header("User-Agent", "narayan-agent")
                    .json(&serde_json::json!({
                        "body": output,
                        "event": "COMMENT",
                    }))
                    .send()
                    .await?;
            }
            _ => {
                // Default: post as issue/PR comment
                let url = format!("https://api.github.com/repos/{}/issues/{}/comments", repo, external_id);
                self.http
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", token))
                    .header("Accept", "application/vnd.github.v3+json")
                    .header("User-Agent", "narayan-agent")
                    .json(&serde_json::json!({ "body": output }))
                    .send()
                    .await?;
            }
        }

        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let token = Self::api_token(config).ok_or_else(|| anyhow::anyhow!("missing 'token' in credentials"))?;
        let resp = self
            .http
            .get("https://api.github.com/user")
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "narayan-agent")
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("GitHub token validation failed: {}", resp.status());
        }
        Ok(())
    }
}
