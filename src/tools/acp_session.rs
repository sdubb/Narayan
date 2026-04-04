use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_boolean};
pub struct AcpSessionTool;
#[async_trait]
impl Tool for AcpSessionTool {
    fn name(&self) -> &str {
        "acp_session"
    }
    fn description(&self) -> &str {
        "Connect to an ACP (Agent Communication Protocol) server for agent-to-agent communication."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("server_url", "string", "ACP server URL."),
            ParameterSchema::required("action", "string", "Action: 'send_message'|'receive_messages'|'list_agents'."),
            ParameterSchema::optional("message", "string", "Message to send (for send_message)."),
            ParameterSchema::optional("target_agent", "string", "Target agent ID (for send_message)."),
            ParameterSchema::optional("auth_token", "string", "Bearer auth token."),
        ]
    }


    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["agents"],
                    "properties": {
                        "agents": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["sent", "to"],
                    "properties": {
                        "sent": schema_boolean(),
                        "to": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let server = match args["server_url"].as_str() {
            Some(s) => s,
            None => return Ok(ToolResult::err("'server_url' required")),
        };
        let action = match args["action"].as_str() {
            Some(a) => a,
            None => return Ok(ToolResult::err("'action' required")),
        };
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?;
        let auth = args["auth_token"].as_str();
        match action {
            "list_agents" => {
                let mut r = client.get(format!("{server}/agents"));
                if let Some(t) = auth {
                    r = r.bearer_auth(t);
                }
                match r.send().await {
                    Ok(resp) => {
                        Ok(ToolResult::ok(serde_json::json!({"agents": resp.text().await.unwrap_or_default()})))
                    }
                    Err(e) => Ok(ToolResult::err(format!("ACP list_agents failed: {e}"))),
                }
            }
            "send_message" => {
                let msg = match args["message"].as_str() {
                    Some(m) => m,
                    None => return Ok(ToolResult::err("'message' required")),
                };
                let target = match args["target_agent"].as_str() {
                    Some(t) => t,
                    None => return Ok(ToolResult::err("'target_agent' required")),
                };
                let payload = serde_json::json!({"to": target, "content": msg});
                let mut r = client.post(format!("{server}/messages")).json(&payload);
                if let Some(t) = auth {
                    r = r.bearer_auth(t);
                }
                match r.send().await {
                    Ok(resp) => {
                        Ok(ToolResult::ok(serde_json::json!({"sent": resp.status().is_success(), "to": target})))
                    }
                    Err(e) => Ok(ToolResult::err(format!("ACP send failed: {e}"))),
                }
            }
            other => Ok(ToolResult::err(format!("Unknown ACP action: '{other}'"))),
        }
    }
}
