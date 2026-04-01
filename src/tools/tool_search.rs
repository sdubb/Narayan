use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub const TOOL_NAME: &str = "tool_search";

pub struct ToolSearchTool;

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Search deferred tools by name or capability and load exact schemas only when needed."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("query", "string", "Search query describing the capability or tool name."),
            ParameterSchema::optional(
                "tool_names",
                "array",
                "Optional exact tool names to load after searching. When omitted, returns matching candidates only.",
            ),
            ParameterSchema::optional("limit", "integer", "Maximum number of matches to return (default 8)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args.get("query").and_then(|value| value.as_str()).map(str::trim).unwrap_or_default();
        if query.is_empty() {
            return Ok(ToolResult::err("'query' is required"));
        }

        Ok(ToolResult::ok(serde_json::json!({
            "status": "searching",
            "query": query,
            "message": "Executor will search the deferred tool catalogue and optionally inject exact schemas.",
        })))
    }
}
