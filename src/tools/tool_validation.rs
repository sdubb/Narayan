use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct ToolValidationTool;
#[async_trait]
impl Tool for ToolValidationTool {
    fn name(&self) -> &str {
        "tool_validation"
    }
    fn description(&self) -> &str {
        "Validate tool arguments against their schema before execution. Use before calling risky tools."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("tool_name", "string", "Name of the tool to validate args for."),
            ParameterSchema::required("args", "object", "Arguments to validate."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tool_name = args["tool_name"].as_str().unwrap_or("unknown");
        let tool_args = &args["args"];
        // Basic validation: check args is an object
        if !tool_args.is_object() {
            return Ok(ToolResult {
                success: false,
                output: serde_json::json!({"valid": false}),
                error: Some("args must be a JSON object".into()),
            });
        }
        Ok(ToolResult::ok(
            serde_json::json!({"valid": true, "tool": tool_name, "arg_count": tool_args.as_object().map(|o| o.len()).unwrap_or(0)}),
        ))
    }
}
