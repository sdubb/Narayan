use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct MemoryStoreTool;
#[async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }
    fn description(&self) -> &str {
        "Store a key-value pair in the agent's memory for later recall across steps."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("key", "string", "Memory key."),
            ParameterSchema::required("value", "string", "Value to store."),
            ParameterSchema::optional("agent_id", "string", "Agent ID scope (default: 'global')."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let key = match args["key"].as_str() {
            Some(k) => k,
            None => return Ok(ToolResult::err("'key' required")),
        };
        let value = match args["value"].as_str() {
            Some(v) => v,
            None => return Ok(ToolResult::err("'value' required")),
        };
        let scope = args["agent_id"].as_str().unwrap_or("global");
        crate::tools::memory_store_internal::insert(format!("{scope}:{key}"), value.to_string());
        Ok(ToolResult::ok(serde_json::json!({"stored": true, "key": key, "scope": scope})))
    }
}
