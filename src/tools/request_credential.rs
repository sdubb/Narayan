use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct RequestCredentialTool;
#[async_trait]
impl Tool for RequestCredentialTool {
    fn name(&self) -> &str {
        "request_credential"
    }
    fn description(&self) -> &str {
        "Store a credential (API key, password, token) securely in agent memory for use by other tools. \
         The credential is stored encrypted and referenced by name."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "name",
                "string",
                "Credential name (used to retrieve it later, e.g. 'github_token').",
            ),
            ParameterSchema::required("value", "string", "The credential value to store."),
            ParameterSchema::optional(
                "description",
                "string",
                "Human-readable description of what this credential is for.",
            ),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = match args["name"].as_str() {
            Some(n) => n,
            None => return Ok(ToolResult::err("'name' required")),
        };
        let value = match args["value"].as_str() {
            Some(v) => v,
            None => return Ok(ToolResult::err("'value' required")),
        };
        let key = format!("credential:{name}");
        crate::tools::memory_store_internal::insert(key, value.to_string());
        Ok(ToolResult::ok(
            serde_json::json!({"stored": true, "name": name, "hint": format!("{}***", &value[..value.len().min(4)])}),
        ))
    }
}
