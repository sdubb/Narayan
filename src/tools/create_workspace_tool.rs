//! `create_workspace_tool` - meta-tool to define a workspace-scoped custom tool.
//!
//! The executor intercepts this call, writes code into the role workspace, and
//! injects a callable tool spec immediately in the same step.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub const TOOL_NAME: &str = "create_workspace_tool";

pub struct CreateWorkspaceToolTool;

#[async_trait]
impl Tool for CreateWorkspaceToolTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Create a workspace-scoped custom tool for this role by providing code and language. \
         The runtime saves it under the role workspace and injects it as a callable tool \
         immediately. Use when no existing tool matches the required capability. \
         Runtime enforces strict size/timeout limits."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("name", "string", "Short tool name, e.g. 'lead_score'."),
            ParameterSchema::required("language", "string", "python | node | deno | ruby | bash | bun"),
            ParameterSchema::required("code", "string", "Source code for the tool implementation."),
            ParameterSchema::optional("description", "string", "What the custom tool does."),
            ParameterSchema::optional(
                "input_schema",
                "object",
                "Optional JSON schema-like object describing expected input fields.",
            ),
            ParameterSchema::optional(
                "timeout_secs",
                "integer",
                "Execution timeout for this custom tool (default: 20, max: 30).",
            ),
        ]
    }

    // Fallback path; normal runtime handles this via executor interception.
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = args["name"].as_str().unwrap_or("").trim();
        let language = args["language"].as_str().unwrap_or("").trim();
        let code = args["code"].as_str().unwrap_or("").trim();
        if name.is_empty() || language.is_empty() || code.is_empty() {
            return Ok(ToolResult::err(
                "name, language, and code are required for create_workspace_tool",
            ));
        }

        Ok(ToolResult::ok(serde_json::json!({
            "status": "pending_intercept",
            "message": "Executor will persist the workspace tool and inject it into this step.",
        })))
    }
}
