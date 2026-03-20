use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct PushoverTool;
#[async_trait]
impl Tool for PushoverTool {
    fn name(&self) -> &str {
        "pushover"
    }
    fn description(&self) -> &str {
        "Send a push notification via Pushover. Requires PUSHOVER_TOKEN and PUSHOVER_USER env vars or stored credentials."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("message", "string", "Notification message."),
            ParameterSchema::optional("title", "string", "Notification title."),
            ParameterSchema::optional("priority", "integer", "Priority: -2 (lowest) to 2 (emergency). Default: 0."),
            ParameterSchema::optional("url", "string", "Supplementary URL to include."),
            ParameterSchema::optional("url_title", "string", "Title for the supplementary URL."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let message = match args["message"].as_str() {
            Some(m) => m,
            None => return Ok(ToolResult::err("'message' required")),
        };
        let token = std::env::var("PUSHOVER_TOKEN")
            .or_else(|_| {
                crate::tools::memory_store_internal::get("credential:pushover_token")
                    .map(|v| v.clone())
                    .ok_or_else(|| std::env::VarError::NotPresent)
            })
            .map_err(|_| anyhow::anyhow!("PUSHOVER_TOKEN not set"))?;
        let user = std::env::var("PUSHOVER_USER")
            .or_else(|_| {
                crate::tools::memory_store_internal::get("credential:pushover_user")
                    .map(|v| v.clone())
                    .ok_or_else(|| std::env::VarError::NotPresent)
            })
            .map_err(|_| anyhow::anyhow!("PUSHOVER_USER not set"))?;
        let mut body = std::collections::HashMap::new();
        body.insert("token", token.as_str());
        body.insert("user", user.as_str());
        body.insert("message", message);
        let title_str = args["title"].as_str().unwrap_or("Narayan Agent").to_string();
        body.insert("title", &title_str);
        let client = reqwest::Client::new();
        let resp = client.post("https://api.pushover.net/1/messages.json").form(&body).send().await;
        match resp {
            Ok(r) if r.status().is_success() => Ok(ToolResult::ok(serde_json::json!({"sent": true}))),
            Ok(r) => Ok(ToolResult::err(format!("Pushover error: HTTP {}", r.status().as_u16()))),
            Err(e) => Ok(ToolResult::err(format!("Pushover failed: {e}"))),
        }
    }
}
