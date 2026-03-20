//! Connector polling scheduler.
//!
//! Replaces inbound webhooks for tenants who prefer OAuth over webhook setup.
//! Runs alongside the main agent scheduler — one poll loop per installed connector.
//!
//! Poll intervals:
//!   GitHub, Jira, Linear  — 2 minutes  (fast-moving, code events)
//!   Slack, Gmail, Outlook  — 5 minutes  (messages)
//!   Salesforce, HubSpot    — 5 minutes  (CRM changes)
//!   Zendesk, ServiceNow    — 3 minutes  (support tickets)
//!   QuickBooks, DocuSign   — 15 minutes (financial documents, slow-moving)
//!   Notion, dbt Cloud      — 10 minutes

use std::sync::Arc;
use chrono::Utc;
use anyhow::Result;
use sqlx::PgPool;

use crate::{
    agent::AgentManager,
    connectors::installs::{ConnectorInstall, ConnectorInstallStore},
};

pub struct ConnectorPoller {
    installs: Arc<ConnectorInstallStore>,
    manager:  Arc<AgentManager>,
}

impl ConnectorPoller {
    pub fn new(installs: Arc<ConnectorInstallStore>, manager: Arc<AgentManager>) -> Self {
        Self { installs, manager }
    }

    /// Run forever — poll all enabled connectors on their schedule.
    pub async fn run(&self) {
        tracing::info!("connector poller starting");
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = self.poll_all().await {
                tracing::error!(error = %e, "connector poll cycle failed");
            }
        }
    }

    async fn poll_all(&self) -> Result<()> {
        // Fetch all enabled installs that have an access token (OAuth or API key).
        // webhook_only installs don't need polling — they receive pushes.
        let all = sqlx::query_as!(
            ConnectorInstall,
            "SELECT id, tenant_id, connector_type, auth_type, token_enc, refresh_enc,
                    token_expires_at, settings, webhook_secret_enc, enabled, last_polled_at,
                    created_at, updated_at
               FROM connector_installs
              WHERE enabled = true AND token_enc IS NOT NULL
              ORDER BY last_polled_at ASC NULLS FIRST
              LIMIT 200"
        )
        .fetch_all(self.installs.pool())
        .await
        .unwrap_or_default();

        let now = chrono::Utc::now();

        for install in all {
            let interval_secs = Self::poll_interval_secs(&install.connector_type);
            let due = match install.last_polled_at {
                None       => true,
                Some(last) => (now - last).num_seconds() >= interval_secs as i64,
            };
            if !due { continue; }

            let token = match self.installs.decrypt_token(&install) {
                Some(t) => t,
                None    => continue,
            };

            match self.poll_connector(&install, &token).await {
                Ok(goals) if !goals.is_empty() => {
                    tracing::info!(
                        tenant_id  = %install.tenant_id,
                        connector  = %install.connector_type,
                        goal_count = goals.len(),
                        "connector poll found new events"
                    );
                    self.process_goals(&install.tenant_id, goals).await;
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(
                    tenant_id = %install.tenant_id,
                    connector = %install.connector_type,
                    error     = %e,
                    "connector poll failed"
                ),
            }

            // Always update last_polled_at regardless of result
            let _ = self.installs.update_last_polled(&install.tenant_id, &install.connector_type).await;
        }
        Ok(())
    }

    fn poll_interval_secs(connector_type: &str) -> u64 {
        match connector_type {
            "github" | "jira" | "atlassian" | "linear" | "zendesk" => 120,
            "slack"  | "gmail" | "microsoft" | "salesforce" | "hubspot" | "pagerduty" => 300,
            "servicenow" | "greenhouse"  => 180,
            "notion" | "dbt_cloud"       => 600,
            "quickbooks" | "docusign"    => 900,
            _                            => 300,
        }
    }

    async fn poll_connector(&self, install: &ConnectorInstall, token: &str) -> Result<Vec<String>> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let since = install.last_polled_at.unwrap_or_else(|| Utc::now() - chrono::Duration::minutes(30));

        match install.connector_type.as_str() {
            "github" => self.poll_github(&http, token, &install.settings, since).await,
            "slack"  => self.poll_slack(&http, token, &install.settings, since).await,
            "gmail"  => self.poll_gmail(&http, token, since).await,
            "jira" | "atlassian" => self.poll_jira(&http, token, &install.settings, since).await,
            "zendesk"  => self.poll_zendesk(&http, token, &install.settings, since).await,
            "salesforce" => self.poll_salesforce(&http, token, &install.settings, since).await,
            "hubspot"  => self.poll_hubspot(&http, token, &install.settings, since).await,
            "pagerduty" => self.poll_pagerduty(&http, token, since).await,
            "notion"   => self.poll_notion(&http, token, since).await,
            "linear"   => self.poll_linear(&http, token, since).await,
            "dbt_cloud" => self.poll_dbt_cloud(&http, token, &install.settings, since).await,
            "greenhouse" => self.poll_greenhouse(&http, token, since).await,
            _ => Ok(vec![]),
        }
    }

    // ── Per-connector pollers ─────────────────────────────────────────────

    async fn poll_github(&self, http: &reqwest::Client, token: &str, settings: &serde_json::Value, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let repo = settings["repo"].as_str().unwrap_or("");
        if repo.is_empty() { return Ok(vec![]); }
        let res = http.get(format!("https://api.github.com/repos/{}/issues?since={}&state=open", repo, since.to_rfc3339()))
            .bearer_auth(token).header("User-Agent", "Narayan/1.0").send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res.as_array().unwrap_or(&vec![]).iter().filter_map(|issue| {
            let title  = issue["title"].as_str()?;
            let number = issue["number"].as_u64()?;
            let url    = issue["html_url"].as_str()?;
            Some(format!("GitHub issue #{number} opened in {repo}: \"{title}\" — {url}"))
        }).collect();
        Ok(goals)
    }

    async fn poll_slack(&self, http: &reqwest::Client, token: &str, settings: &serde_json::Value, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let channel = settings["channel_id"].as_str().unwrap_or("");
        if channel.is_empty() { return Ok(vec![]); }
        let oldest = since.timestamp().to_string();
        let res = http.get(format!("https://slack.com/api/conversations.history?channel={}&oldest={}", channel, oldest))
            .bearer_auth(token).send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["messages"].as_array().unwrap_or(&vec![]).iter().filter_map(|msg| {
            let text = msg["text"].as_str()?.trim();
            // Only trigger on messages that @mention the bot or contain trigger keywords
            let user = msg["user"].as_str().unwrap_or("unknown");
            if text.contains("@narayan") || text.to_lowercase().contains("help me") {
                Some(format!("Slack message from {user} in channel: \"{text}\""))
            } else { None }
        }).collect();
        Ok(goals)
    }

    async fn poll_gmail(&self, http: &reqwest::Client, token: &str, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let after = since.timestamp();
        let res = http.get(format!("https://gmail.googleapis.com/gmail/v1/users/me/messages?q=after:{}&maxResults=10", after))
            .bearer_auth(token).send().await?.json::<serde_json::Value>().await?;
        let count = res["messages"].as_array().map(|a| a.len()).unwrap_or(0);
        if count > 0 {
            Ok(vec![format!("{count} new Gmail messages — review and respond to the most urgent ones")])
        } else { Ok(vec![]) }
    }

    async fn poll_jira(&self, http: &reqwest::Client, token: &str, settings: &serde_json::Value, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let cloud_id = settings["cloud_id"].as_str().unwrap_or("");
        if cloud_id.is_empty() { return Ok(vec![]); }
        let jql = format!("created > \"{}\" ORDER BY created DESC", since.format("%Y-%m-%d %H:%M"));
        let res = http.get(format!("https://api.atlassian.com/ex/jira/{}/rest/api/3/search?jql={}&maxResults=10", cloud_id, urlencoding::encode(&jql)))
            .bearer_auth(token).send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["issues"].as_array().unwrap_or(&vec![]).iter().filter_map(|issue| {
            let key     = issue["key"].as_str()?;
            let summary = issue["fields"]["summary"].as_str()?;
            Some(format!("Jira issue {key} created: \"{summary}\""))
        }).collect();
        Ok(goals)
    }

    async fn poll_zendesk(&self, http: &reqwest::Client, token: &str, settings: &serde_json::Value, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let subdomain = settings["subdomain"].as_str().unwrap_or("");
        if subdomain.is_empty() { return Ok(vec![]); }
        let res = http.get(format!("https://{}.zendesk.com/api/v2/tickets/recent.json", subdomain))
            .bearer_auth(token).send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["tickets"].as_array().unwrap_or(&vec![]).iter().filter_map(|t| {
            let id      = t["id"].as_u64()?;
            let subject = t["subject"].as_str()?;
            let status  = t["status"].as_str().unwrap_or("open");
            if status == "new" || status == "open" {
                Some(format!("Zendesk ticket #{id}: \"{subject}\" — draft a response and update the ticket"))
            } else { None }
        }).collect();
        Ok(goals)
    }

    async fn poll_salesforce(&self, http: &reqwest::Client, token: &str, settings: &serde_json::Value, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let instance = settings["instance_url"].as_str().unwrap_or("https://login.salesforce.com");
        let soql     = format!("SELECT Id,Name,StageName FROM Opportunity WHERE CreatedDate > {} ORDER BY CreatedDate DESC LIMIT 10", since.format("%Y-%m-%dT%H:%M:%SZ"));
        let res = http.get(format!("{}/services/data/v58.0/query?q={}", instance, urlencoding::encode(&soql)))
            .bearer_auth(token).send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["records"].as_array().unwrap_or(&vec![]).iter().filter_map(|opp| {
            let name  = opp["Name"].as_str()?;
            let stage = opp["StageName"].as_str()?;
            Some(format!("Salesforce opportunity \"{name}\" moved to stage \"{stage}\" — research and prepare talking points"))
        }).collect();
        Ok(goals)
    }

    async fn poll_hubspot(&self, http: &reqwest::Client, token: &str, _settings: &serde_json::Value, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let since_ms = since.timestamp_millis();
        let res = http.get(format!("https://api.hubapi.com/crm/v3/objects/deals?createdAfter={}&limit=10", since_ms))
            .bearer_auth(token).send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["results"].as_array().unwrap_or(&vec![]).iter().filter_map(|deal| {
            let name = deal["properties"]["dealname"].as_str()?;
            Some(format!("HubSpot deal created: \"{name}\" — research the company and prepare outreach"))
        }).collect();
        Ok(goals)
    }

    async fn poll_pagerduty(&self, http: &reqwest::Client, token: &str, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let res = http.get(format!("https://api.pagerduty.com/incidents?statuses[]=triggered&since={}", since.to_rfc3339()))
            .header("Authorization", format!("Token token={token}"))
            .header("Accept", "application/vnd.pagerduty+json;version=2")
            .send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["incidents"].as_array().unwrap_or(&vec![]).iter().filter_map(|inc| {
            let title = inc["title"].as_str()?;
            let id    = inc["id"].as_str()?;
            Some(format!("PagerDuty incident {id} triggered: \"{title}\" — run the runbook and post status update"))
        }).collect();
        Ok(goals)
    }

    async fn poll_notion(&self, http: &reqwest::Client, token: &str, _since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let res = http.post("https://api.notion.com/v1/search")
            .bearer_auth(token)
            .header("Notion-Version", "2022-06-28")
            .json(&serde_json::json!({"filter": {"property": "object", "value": "page"}, "page_size": 5}))
            .send().await?.json::<serde_json::Value>().await?;
        let _ = res; // Notion polling is best done via MCP session — just return empty
        Ok(vec![])
    }

    async fn poll_linear(&self, http: &reqwest::Client, token: &str, since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let query = format!(r#"{{"query": "{{ issues(filter: {{ createdAt: {{ gt: \"{}\" }} }}) {{ nodes {{ id title priority state {{ name }} }} }} }}"}}"#, since.to_rfc3339());
        let res = http.post("https://api.linear.app/graphql")
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .body(query)
            .send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["data"]["issues"]["nodes"].as_array().unwrap_or(&vec![]).iter().filter_map(|issue| {
            let title = issue["title"].as_str()?;
            let id    = issue["id"].as_str()?;
            Some(format!("Linear issue {id} created: \"{title}\""))
        }).collect();
        Ok(goals)
    }

    async fn poll_dbt_cloud(&self, http: &reqwest::Client, token: &str, settings: &serde_json::Value, _since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let account_id = settings["account_id"].as_str().unwrap_or("");
        if account_id.is_empty() { return Ok(vec![]); }
        let res = http.get(format!("https://cloud.getdbt.com/api/v2/accounts/{}/runs/?status=20&limit=5", account_id))
            .header("Authorization", format!("Token {token}"))
            .send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res["data"].as_array().unwrap_or(&vec![]).iter().filter_map(|run| {
            let id   = run["id"].as_u64()?;
            let name = run["job_definition_id"].to_string();
            Some(format!("dbt Cloud run #{id} (job {name}) failed — investigate logs and identify the failing model"))
        }).collect();
        Ok(goals)
    }

    async fn poll_greenhouse(&self, http: &reqwest::Client, token: &str, _since: chrono::DateTime<Utc>) -> Result<Vec<String>> {
        let res = http.get("https://harvest.greenhouse.io/v1/applications?status=active&per_page=10")
            .basic_auth(token, Some(""))
            .send().await?.json::<serde_json::Value>().await?;
        let goals: Vec<String> = res.as_array().unwrap_or(&vec![]).iter().filter_map(|app| {
            let id       = app["id"].as_u64()?;
            let job_name = app["jobs"][0]["name"].as_str()?;
            Some(format!("Greenhouse application #{id} for \"{job_name}\" — review resume and draft initial screening notes"))
        }).collect();
        Ok(goals)
    }

    /// Create agent goals for new events, skipping duplicates.
    pub async fn process_goals(&self, tenant_id: &str, goals: Vec<String>) {
        for goal in goals {
            match self.manager.create_goal(tenant_id.to_string(), goal.clone()).await {
                Ok((_, agent)) => tracing::info!(tenant_id, agent_id = %agent.id, "connector poll created agent"),
                Err(e)         => tracing::warn!(tenant_id, error = %e, goal = %goal, "connector poll failed to create agent"),
            }
        }
    }
}
