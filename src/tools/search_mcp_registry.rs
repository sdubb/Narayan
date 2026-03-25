//! search_mcp_registry — discover MCP servers by capability keyword.
//!
//! Searches a curated registry of known MCP servers plus any servers the
//! tenant has manually registered. Returns matching servers with their
//! URLs, capabilities, and auth requirements.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::tools::{ParameterSchema, Tool, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    pub name: String,
    pub url: String,
    pub description: String,
    pub categories: Vec<String>,
    pub auth_type: String, // "oauth" | "api_key" | "none"
    pub connected: bool,
}

/// Well-known public MCP servers — the same ones available in Claude.ai.
fn known_servers() -> Vec<McpServerEntry> {
    vec![
        McpServerEntry {
            name: "Gmail".into(),
            url: "https://gmail.mcp.claude.ai/mcp".into(),
            description: "Read, send, search Gmail messages and threads".into(),
            categories: vec!["email".into(), "communication".into(), "google".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Google Calendar".into(),
            url: "https://gcal.mcp.claude.ai/mcp".into(),
            description: "Create and manage Google Calendar events".into(),
            categories: vec!["calendar".into(), "scheduling".into(), "google".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Google Drive".into(),
            url: "https://gdrive.mcp.claude.ai/mcp".into(),
            description: "Access and manage Google Drive files and folders".into(),
            categories: vec!["files".into(), "storage".into(), "google".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "GitHub".into(),
            url: "https://api.githubcopilot.com/mcp".into(),
            description: "Repos, issues, PRs, code search, GitHub Actions".into(),
            categories: vec!["git".into(), "code".into(), "devops".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Slack".into(),
            url: "https://slack.mcp.claude.ai/mcp".into(),
            description: "Send messages, read channels, manage Slack workspace".into(),
            categories: vec!["communication".into(), "team".into(), "messaging".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Notion".into(),
            url: "https://notion.mcp.claude.ai/mcp".into(),
            description: "Read and write Notion pages, databases, and blocks".into(),
            categories: vec!["notes".into(), "docs".into(), "database".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Asana".into(),
            url: "https://mcp.asana.com/sse".into(),
            description: "Create tasks, projects, and manage workflows in Asana".into(),
            categories: vec!["tasks".into(), "project".into(), "productivity".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Jira".into(),
            url: "https://mcp.atlassian.com/jira/sse".into(),
            description: "Create and update Jira issues, sprints, and boards".into(),
            categories: vec!["tasks".into(), "devops".into(), "project".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Confluence".into(),
            url: "https://mcp.atlassian.com/confluence/sse".into(),
            description: "Search and edit Confluence pages and spaces".into(),
            categories: vec!["docs".into(), "wiki".into(), "knowledge".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Salesforce".into(),
            url: "https://mcp.salesforce.com/sse".into(),
            description: "CRM records, leads, opportunities, accounts".into(),
            categories: vec!["crm".into(), "sales".into(), "business".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Stripe".into(),
            url: "https://mcp.stripe.com/sse".into(),
            description: "Payments, customers, subscriptions, invoices".into(),
            categories: vec!["payments".into(), "billing".into(), "finance".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Postgres".into(),
            url: "https://mcp.supabase.com/sse".into(),
            description: "Query and manage PostgreSQL databases".into(),
            categories: vec!["database".into(), "sql".into(), "data".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Shopify".into(),
            url: "https://mcp.shopify.com/sse".into(),
            description: "Products, orders, customers, inventory".into(),
            categories: vec!["ecommerce".into(), "store".into(), "orders".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Linear".into(),
            url: "https://mcp.linear.app/sse".into(),
            description: "Issues, projects, cycles for Linear workspaces".into(),
            categories: vec!["tasks".into(), "devops".into(), "project".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "monday.com".into(),
            url: "https://mcp.monday.com/sse".into(),
            description: "Boards, items, columns, and updates for monday.com".into(),
            categories: vec!["tasks".into(), "project".into(), "ops".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "HubSpot".into(),
            url: "https://mcp.hubspot.com/sse".into(),
            description: "Contacts, deals, companies, marketing automation".into(),
            categories: vec!["crm".into(), "marketing".into(), "sales".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Figma".into(),
            url: "https://mcp.figma.com/sse".into(),
            description: "Design files, components, comments, exports".into(),
            categories: vec!["design".into(), "ui".into(), "figma".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Canva".into(),
            url: "https://mcp.canva.com/sse".into(),
            description: "Create and manage Canva designs and assets".into(),
            categories: vec!["design".into(), "graphics".into(), "marketing".into()],
            auth_type: "oauth".into(),
            connected: false,
        },
        McpServerEntry {
            name: "Twilio".into(),
            url: "https://mcp.twilio.com/sse".into(),
            description: "Send SMS, WhatsApp, voice calls via Twilio".into(),
            categories: vec!["sms".into(), "communication".into(), "messaging".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "SendGrid".into(),
            url: "https://mcp.sendgrid.com/sse".into(),
            description: "Transactional and marketing email via SendGrid".into(),
            categories: vec!["email".into(), "marketing".into(), "communication".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
        McpServerEntry {
            name: "AWS".into(),
            url: "https://mcp.aws.amazon.com/sse".into(),
            description: "S3, EC2, Lambda, CloudWatch, and AWS services".into(),
            categories: vec!["cloud".into(), "devops".into(), "infrastructure".into()],
            auth_type: "api_key".into(),
            connected: false,
        },
    ]
}

pub struct SearchMcpRegistryTool;

#[async_trait]
impl Tool for SearchMcpRegistryTool {
    fn name(&self) -> &str {
        "search_mcp_registry"
    }

    fn description(&self) -> &str {
        "Search the registry of available MCP (Model Context Protocol) servers by keyword or category. \
         Returns matching servers with their URLs and auth requirements. \
         Use suggest_connectors to prompt the user to connect one."
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "keywords",
                "string",
                "Search keywords, e.g. 'email', 'github', 'database', 'crm'. Separate multiple with spaces.",
            ),
            ParameterSchema::optional(
                "category",
                "string",
                "Filter by category: email|communication|code|database|crm|files|tasks|design|payments|cloud",
            ),
            ParameterSchema::optional(
                "include_custom",
                "boolean",
                "Also search custom servers registered by this agent (default: true).",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let keywords = match args["keywords"].as_str() {
            Some(k) => k.to_lowercase(),
            None => return Ok(ToolResult::err("'keywords' is required")),
        };
        let category = args["category"].as_str().map(|s| s.to_lowercase());
        let include_custom = args["include_custom"].as_bool().unwrap_or(true);

        let terms: Vec<&str> = keywords.split_whitespace().collect();
        let mut results: Vec<McpServerEntry> = Vec::new();

        // Search known servers
        for server in known_servers() {
            let haystack = format!(
                "{} {} {}",
                server.name.to_lowercase(),
                server.description.to_lowercase(),
                server.categories.join(" ").to_lowercase()
            );

            let keyword_match = terms.iter().any(|t| haystack.contains(t));
            let category_match =
                category.as_ref().map(|c| server.categories.iter().any(|sc| sc.contains(c.as_str()))).unwrap_or(true);

            if keyword_match && category_match {
                results.push(server);
            }
        }

        // Search custom registered connectors in memory
        if include_custom {
            crate::tools::memory_store_internal::with_store(|store| {
                for entry in store.iter() {
                    if entry.key().starts_with("mcp_connector:") {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(entry.value()) {
                            let desc = val["description"].as_str().unwrap_or("").to_lowercase();
                            let name = val["tool_name"].as_str().unwrap_or("").to_lowercase();
                            if terms.iter().any(|t| desc.contains(t) || name.contains(t)) {
                                results.push(McpServerEntry {
                                    name: val["tool_name"].as_str().unwrap_or("custom").to_string(),
                                    url: val["server_url"].as_str().unwrap_or("").to_string(),
                                    description: val["description"].as_str().unwrap_or("").to_string(),
                                    categories: vec!["custom".into()],
                                    auth_type: "unknown".into(),
                                    connected: true,
                                });
                            }
                        }
                    }
                }
            });
        }

        Ok(ToolResult::ok(serde_json::json!({
            "query":   keywords,
            "count":   results.len(),
            "servers": results,
            "tip": "Use suggest_connectors with the server URL to prompt connection, then mcp_session to use it.",
        })))
    }
}
