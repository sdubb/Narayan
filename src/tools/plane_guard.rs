use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct PlaneGuardTool;
#[async_trait]
impl Tool for PlaneGuardTool {
    fn name(&self) -> &str {
        "plane_guard"
    }
    fn description(&self) -> &str {
        "Validate whether an action is permitted by the agent's security policy before execution. Returns approved/denied with reason."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("action", "string", "Action to validate, e.g. 'shell:rm -rf'"),
            ParameterSchema::required("risk_level", "string", "Risk level: 'low'|'medium'|'high'|'critical'"),
            ParameterSchema::optional("description", "string", "Human-readable description of what the action does."),
            ParameterSchema::optional("reversible", "boolean", "Whether the action can be undone (default: false)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args["action"].as_str().unwrap_or("");
        let risk = args["risk_level"].as_str().unwrap_or("medium");
        let reversible = args["reversible"].as_bool().unwrap_or(false);
        // Block critical irreversible actions
        let blocked = risk == "critical" && !reversible;
        let reason = if blocked {
            "Critical irreversible actions require manual approval. Set reversible=true or lower risk_level."
        } else if risk == "high" && !reversible {
            "High-risk irreversible action requires explicit approval (set approved=true in the tool call)."
        } else {
            "Action permitted by current policy."
        };
        Ok(ToolResult::ok(serde_json::json!({
            "approved":   !blocked,
            "action":     action,
            "risk_level": risk,
            "reversible": reversible,
            "reason":     reason,
        })))
    }
}
