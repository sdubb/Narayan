use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct MemoryForgetTool;
#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }
    fn description(&self) -> &str {
        "Delete a key from agent memory."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("key", "string", "Memory key to delete."),
            ParameterSchema::optional("agent_id", "string", "Agent ID scope (default: 'global')."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let key = match args["key"].as_str() {
            Some(k) => k,
            None => return Ok(ToolResult::err("'key' required")),
        };
        let scope = args["agent_id"].as_str().unwrap_or("global");
        let removed = crate::tools::memory_store_internal::remove(&format!("{scope}:{key}"));
        Ok(ToolResult::ok(serde_json::json!({"deleted": removed, "key": key})))
    }
}
