use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_boolean};
pub struct RequestCredentialTool;
#[async_trait]
impl Tool for RequestCredentialTool {
    fn name(&self) -> &str {
        "request_credential"
    }
    fn description(&self) -> &str {
        "Store a credential (API key, password, token) in agent credential memory for use by other tools. \
         The value is not echoed back in normal agent prompts or UI cards and is referenced by name."
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

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["stored", "name", "hint"],
            "properties": {
                "stored": schema_boolean(),
                "name": schema_string(),
                "hint": schema_string(),
            },
            "additionalProperties": true,
        }))
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
        Ok(ToolResult::ok(serde_json::json!({
            "stored": true,
            "name": name,
            "hint": "[stored securely in credential memory]"
        })))
    }
}
