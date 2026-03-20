use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct MemoryRecallTool;
#[async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }
    fn description(&self) -> &str {
        "Retrieve a stored value from agent memory by key."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("key", "string", "Memory key to retrieve."),
            ParameterSchema::optional("agent_id", "string", "Agent ID scope (default: 'global')."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let key = match args["key"].as_str() {
            Some(k) => k,
            None => return Ok(ToolResult::err("'key' required")),
        };
        let scope = args["agent_id"].as_str().unwrap_or("global");
        let value = crate::tools::memory_store_internal::get(&format!("{scope}:{key}"));
        Ok(ToolResult::ok(serde_json::json!({"key": key, "value": value, "found": value.is_some()})))
    }
}
