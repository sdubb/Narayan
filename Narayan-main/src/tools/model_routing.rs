use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct ModelRoutingTool;
#[async_trait]
impl Tool for ModelRoutingTool {
    fn name(&self) -> &str {
        "model_routing"
    }
    fn description(&self) -> &str {
        "Override LLM model routing for this agent. Set preferred model for simple, medium, or complex tasks."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional("simple_model", "string", "Model for simple tasks (e.g. 'gpt-4o-mini')."),
            ParameterSchema::optional("medium_model", "string", "Model for medium tasks (e.g. 'gpt-4o')."),
            ParameterSchema::optional("complex_model", "string", "Model for complex tasks (e.g. 'claude-sonnet-4-6')."),
            ParameterSchema::optional("provider", "string", "Provider name override."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let mut updated = Vec::new();
        if let Some(s) = args["simple_model"].as_str() {
            crate::tools::memory_store_internal::insert("routing:simple".into(), s.to_string());
            updated.push(format!("simple={s}"));
        }
        if let Some(m) = args["medium_model"].as_str() {
            crate::tools::memory_store_internal::insert("routing:medium".into(), m.to_string());
            updated.push(format!("medium={m}"));
        }
        if let Some(c) = args["complex_model"].as_str() {
            crate::tools::memory_store_internal::insert("routing:complex".into(), c.to_string());
            updated.push(format!("complex={c}"));
        }
        if let Some(p) = args["provider"].as_str() {
            crate::tools::memory_store_internal::insert("routing:provider".into(), p.to_string());
            updated.push(format!("provider={p}"));
        }
        Ok(ToolResult::ok(serde_json::json!({"updated": updated})))
    }
}
