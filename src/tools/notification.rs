//! notification — Send messages to Slack, Discord, Telegram, and MS Teams.
//! All via webhook URLs — no bot tokens, no OAuth setup needed.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct NotificationTool;

#[async_trait]
impl Tool for NotificationTool {
    fn name(&self) -> &str {
        "notification"
    }
    fn description(&self) -> &str {
        "Send a notification to Slack, Discord, Telegram, or MS Teams. \
         Store the webhook URL or bot token with request_credential first."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("channel", "string", "Target: slack | discord | telegram | teams"),
            ParameterSchema::required("message", "string", "Message text (supports markdown for Slack/Discord)."),
            ParameterSchema::optional("title", "string", "Optional title / header."),
            ParameterSchema::optional(
                "credential_key",
                "string",
                "Credential key for webhook URL or bot token (default: '{channel}_webhook').",
            ),
            ParameterSchema::optional("chat_id", "string", "Telegram chat ID (required for telegram channel)."),
            ParameterSchema::optional("color", "string", "Sidebar colour hex for Slack attachments (e.g. '#10b981')."),
            ParameterSchema::optional("urgent", "boolean", "Mark as urgent / high-priority (default: false)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let channel = match args["channel"].as_str() {
            Some(c) => c,
            None => return Ok(ToolResult::err("'channel' required")),
        };
        let message = match args["message"].as_str() {
            Some(m) => m,
            None => return Ok(ToolResult::err("'message' required")),
        };
        let title = args["title"].as_str().unwrap_or("Narayan Agent");
        let urgent = args["urgent"].as_bool().unwrap_or(false);
        let color = args["color"].as_str().unwrap_or(if urgent { "#ef4444" } else { "#10b981" });

        let cred_key =
            args["credential_key"].as_str().map(String::from).unwrap_or_else(|| format!("{}_webhook", channel));

        let webhook = match crate::tools::memory_store_internal::get(&format!("credential:{}", cred_key)) {
            Some(w) => w,
            None => {
                return Ok(ToolResult::err(format!(
                    "credential '{}' not found — store the webhook URL with request_credential",
                    cred_key
                )))
            }
        };

        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build()?;

        let (status, provider) = match channel.to_lowercase().as_str() {
            "slack" => {
                let payload = serde_json::json!({
                    "text": format!("*{}*\n{}", title, message),
                    "attachments": [{
                        "color": color,
                        "text":  message,
                        "title": title,
                        "footer": "Narayan",
                        "ts": chrono::Utc::now().timestamp(),
                    }],
                });
                let r =
                    client.post(&webhook).json(&payload).send().await.map_err(|e| anyhow::anyhow!("Slack: {}", e))?;
                (r.status().as_u16(), "slack")
            }
            "discord" => {
                let emoji = if urgent { "🚨" } else { "🤖" };
                let payload = serde_json::json!({
                    "embeds": [{
                        "title":       format!("{} {}", emoji, title),
                        "description": message,
                        "color":       hex_to_decimal(color),
                        "footer":      { "text": "Narayan Agent" },
                        "timestamp":   chrono::Utc::now().to_rfc3339(),
                    }]
                });
                let r =
                    client.post(&webhook).json(&payload).send().await.map_err(|e| anyhow::anyhow!("Discord: {}", e))?;
                (r.status().as_u16(), "discord")
            }
            "telegram" => {
                let chat_id = match args["chat_id"].as_str() {
                    Some(id) => id,
                    None => return Ok(ToolResult::err("'chat_id' required for telegram")),
                };
                let text = format!("*{}*\n{}", title, message);
                let url = format!("https://api.telegram.org/bot{}/sendMessage", webhook);
                let payload = serde_json::json!({
                    "chat_id":    chat_id,
                    "text":       text,
                    "parse_mode": "Markdown",
                });
                let r =
                    client.post(&url).json(&payload).send().await.map_err(|e| anyhow::anyhow!("Telegram: {}", e))?;
                (r.status().as_u16(), "telegram")
            }
            "teams" => {
                let payload = serde_json::json!({
                    "@type":      "MessageCard",
                    "@context":   "http://schema.org/extensions",
                    "summary":    title,
                    "themeColor": color.trim_start_matches('#'),
                    "sections":   [{ "activityTitle": title, "activityText": message }],
                });
                let r =
                    client.post(&webhook).json(&payload).send().await.map_err(|e| anyhow::anyhow!("Teams: {}", e))?;
                (r.status().as_u16(), "teams")
            }
            other => {
                return Ok(ToolResult::err(format!(
                    "unknown channel '{}' — use: slack | discord | telegram | teams",
                    other
                )))
            }
        };

        let ok = (200..300).contains(&status) || status == 204;
        if ok {
            Ok(ToolResult::ok(serde_json::json!({"sent": true, "provider": provider, "status": status})))
        } else {
            Ok(ToolResult {
                success: false,
                output: serde_json::json!({"provider": provider, "status": status}),
                error: Some(format!("{} returned HTTP {}", provider, status)),
            })
        }
    }
}

fn hex_to_decimal(hex: &str) -> u32 {
    u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x10b981)
}
