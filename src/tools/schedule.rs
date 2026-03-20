use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct ScheduleTool;
#[async_trait]
impl Tool for ScheduleTool {
    fn name(&self) -> &str {
        "schedule"
    }
    fn description(&self) -> &str {
        "Schedule a task or agent goal to run at a future time. Returns a schedule ID."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("goal", "string", "Goal or task description to schedule."),
            ParameterSchema::required("run_at", "string", "ISO 8601 datetime to run at, e.g. '2026-04-01T09:00:00Z'."),
            ParameterSchema::optional("tenant_id", "string", "Tenant ID (injected automatically)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let goal = match args["goal"].as_str() {
            Some(g) => g,
            None => return Ok(ToolResult::err("'goal' required")),
        };
        let run_at = match args["run_at"].as_str() {
            Some(r) => r,
            None => return Ok(ToolResult::err("'run_at' required")),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let key = format!("schedule:{id}");
        crate::tools::memory_store_internal::insert(
            key,
            serde_json::json!({"id": id, "goal": goal, "run_at": run_at, "status": "scheduled"}).to_string(),
        );
        Ok(ToolResult::ok(serde_json::json!({"scheduled": true, "id": id, "goal": goal, "run_at": run_at})))
    }
}
