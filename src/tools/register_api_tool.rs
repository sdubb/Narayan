use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_boolean};
pub struct RegisterApiTool;
#[async_trait]
impl Tool for RegisterApiTool {
    fn name(&self) -> &str {
        "register_api_tool"
    }
    fn description(&self) -> &str {
        "Register a new API endpoint as a callable tool. The tool will be available to subsequent steps."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("tool_name", "string", "Name for the new tool."),
            ParameterSchema::required("base_url", "string", "Base URL of the API."),
            ParameterSchema::required("description", "string", "What this API does."),
            ParameterSchema::optional("auth_type", "string", "Auth type: 'bearer'|'api_key'|'none'."),
            ParameterSchema::optional("credential_key", "string", "Credential key name for auth."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["registered", "tool_name", "base_url"],
            "properties": {
                "registered": schema_boolean(),
                "tool_name": schema_string(),
                "base_url": schema_string(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = args["tool_name"].as_str().unwrap_or("unnamed");
        let url = args["base_url"].as_str().unwrap_or("");
        let desc = args["description"].as_str().unwrap_or("");
        // Store registration in memory for agent reference
        let reg_key = format!("api_tool:{name}");
        let reg_val = serde_json::json!({"base_url": url, "description": desc, "auth_type": args["auth_type"], "credential_key": args["credential_key"]});
        crate::tools::memory_store_internal::insert(reg_key, reg_val.to_string());
        Ok(ToolResult::ok(serde_json::json!({"registered": true, "tool_name": name, "base_url": url})))
    }
}
