use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct AskUserTool;
#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }
    fn description(&self) -> &str {
        "Pause the agent and request input from the user. The agent enters 'clarifying' state. \
         Use sparingly — only when blocking ambiguity cannot be resolved without human input."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("question", "string", "The question to ask the user."),
            ParameterSchema::optional("options", "array", "Optional list of answer options to present."),
            ParameterSchema::optional(
                "required",
                "boolean",
                "Whether an answer is required before proceeding (default: true).",
            ),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let question = match args["question"].as_str() {
            Some(q) => q,
            None => return Ok(ToolResult::err("'question' required")),
        };
        let options = args["options"].as_array().cloned().unwrap_or_default();
        // Store the pending question so the clarification API can surface it
        let pending =
            serde_json::json!({"question": question, "options": options, "asked_at": chrono::Utc::now().to_rfc3339()});
        crate::tools::memory_store_internal::insert("ask_user:pending".into(), pending.to_string());
        Ok(ToolResult::ok(serde_json::json!({
            "status":    "awaiting_user_input",
            "question":  question,
            "options":   options,
            "note":      "Agent is paused. User must respond via POST /agents/:id/clarify",
        })))
    }
}
