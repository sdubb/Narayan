//! suggest_connectors — recommend MCP servers and prompt for connection.
//!
//! This is the agent-side equivalent of Claude.ai's "Connect" button.
//! When an agent discovers it needs a capability it doesn't have access to,
//! it uses this tool to surface a connection request to the human operator,
//! then pauses and waits for the credential to be provided via PUT /credentials.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct SuggestConnectorsTool;

#[async_trait]
impl Tool for SuggestConnectorsTool {
    fn name(&self) -> &str {
        "suggest_connectors"
    }

    fn description(&self) -> &str {
        "Suggest one or more MCP servers for the operator to connect. \
         Stores the suggestion in agent state and pauses execution until \
         the operator connects the server and provides credentials via PUT /credentials. \
         Use after search_mcp_registry when a needed capability is not yet connected."
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "servers",
                "array",
                "List of server names or URLs to suggest, e.g. ['Gmail', 'GitHub'].",
            ),
            ParameterSchema::required("reason", "string", "Why this connector is needed — shown to the operator."),
            ParameterSchema::optional(
                "blocking",
                "boolean",
                "If true, agent pauses until connector is available (default: true).",
            ),
            ParameterSchema::optional(
                "credential_keys",
                "array",
                "Credential key names the operator should provide, e.g. ['gmail_token', 'github_key'].",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let servers: Vec<String> =
            args["servers"].as_array().unwrap_or(&vec![]).iter().filter_map(|v| v.as_str().map(String::from)).collect();

        let reason = args["reason"].as_str().unwrap_or("required for this task").to_string();
        let blocking = args["blocking"].as_bool().unwrap_or(true);
        let cred_keys: Vec<String> = args["credential_keys"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if servers.is_empty() {
            return Ok(ToolResult::err("'servers' must not be empty"));
        }

        // Store the pending connector request so the operator can see it
        let request = serde_json::json!({
            "servers":         servers,
            "reason":          reason,
            "credential_keys": cred_keys,
            "blocking":        blocking,
            "requested_at":    chrono::Utc::now().to_rfc3339(),
            "status":          "pending",
        });

        crate::tools::memory_store_internal::insert("suggest_connectors:pending".into(), request.to_string());

        tracing::info!(
            servers  = ?servers,
            reason   = %reason,
            blocking = blocking,
            "connector suggestion raised"
        );

        // Build human-readable instructions for the operator
        let cred_instructions = if cred_keys.is_empty() {
            String::new()
        } else {
            format!(
                " Provide credentials via PUT /credentials for: {}.",
                cred_keys.iter().map(|k| format!("'{}'", k)).collect::<Vec<_>>().join(", ")
            )
        };

        let message = if blocking {
            format!(
                "Agent requires connection to: {}. Reason: {}.{} \
                 After connecting, resume via POST /agents/:id/resume.",
                servers.join(", "),
                reason,
                cred_instructions
            )
        } else {
            format!(
                "Suggested connectors: {}. Reason: {}.{} Agent will continue without them.",
                servers.join(", "),
                reason,
                cred_instructions
            )
        };

        Ok(ToolResult::ok(serde_json::json!({
            "suggested":          servers,
            "reason":             reason,
            "blocking":           blocking,
            "credential_keys":    cred_keys,
            "operator_message":   message,
            "status":             if blocking { "awaiting_connection" } else { "suggested" },
            "resume_endpoint":    "POST /agents/:id/resume",
        })))
    }
}
