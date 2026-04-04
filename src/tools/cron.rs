use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_boolean};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub schedule: String,
    pub command: String,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub runs: Vec<String>,
}

static CRON_STORE: LazyLock<Arc<DashMap<String, CronJob>>> = LazyLock::new(|| Arc::new(DashMap::new()));

pub struct CronAddTool;
pub struct CronListTool;
pub struct CronRemoveTool;
pub struct CronRunTool;
pub struct CronRunsTool;
pub struct CronUpdateTool;

#[async_trait]
impl Tool for CronAddTool {
    fn name(&self) -> &str {
        "cron_add"
    }
    fn description(&self) -> &str {
        "Add a new cron job. Stores the schedule and command; actual execution requires the cron runner service."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("schedule", "string", "Cron schedule expression (e.g. '0 * * * *' for hourly)."),
            ParameterSchema::required("command", "string", "Shell command or agent goal to run on schedule."),
            ParameterSchema::optional("id", "string", "Optional job ID (auto-generated if omitted)."),
        ]
    }


    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["added", "id", "schedule"],
            "properties": {
                "added": schema_boolean(),
                "id": schema_string(),
                "schedule": schema_string(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let schedule = match args["schedule"].as_str() {
            Some(s) => s,
            None => return Ok(ToolResult::err("'schedule' required")),
        };
        let command = match args["command"].as_str() {
            Some(c) => c,
            None => return Ok(ToolResult::err("'command' required")),
        };
        let id = args["id"].as_str().map(String::from).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let job = CronJob {
            id: id.clone(),
            schedule: schedule.to_string(),
            command: command.to_string(),
            enabled: true,
            last_run: None,
            runs: vec![],
        };
        CRON_STORE.insert(id.clone(), job);
        Ok(ToolResult::ok(serde_json::json!({"added": true, "id": id, "schedule": schedule})))
    }
}

#[async_trait]
impl Tool for CronListTool {
    fn name(&self) -> &str {
        "cron_list"
    }
    fn description(&self) -> &str {
        "List all registered cron jobs."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![]
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let jobs: Vec<serde_json::Value> = CRON_STORE.iter().map(|e| {
            let j = e.value();
            serde_json::json!({"id": j.id, "schedule": j.schedule, "command": j.command, "enabled": j.enabled, "last_run": j.last_run})
        }).collect();
        Ok(ToolResult::ok(serde_json::json!({"jobs": jobs, "count": jobs.len()})))
    }
}

#[async_trait]
impl Tool for CronRemoveTool {
    fn name(&self) -> &str {
        "cron_remove"
    }
    fn description(&self) -> &str {
        "Remove a cron job by ID."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![ParameterSchema::required("id", "string", "Job ID to remove.")]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = match args["id"].as_str() {
            Some(i) => i,
            None => return Ok(ToolResult::err("'id' required")),
        };
        let removed = CRON_STORE.remove(id).is_some();
        Ok(ToolResult::ok(serde_json::json!({"removed": removed, "id": id})))
    }
}

#[async_trait]
impl Tool for CronRunTool {
    fn name(&self) -> &str {
        "cron_run"
    }
    fn description(&self) -> &str {
        "Manually trigger a cron job immediately by ID."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![ParameterSchema::required("id", "string", "Job ID to run.")]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = match args["id"].as_str() {
            Some(i) => i,
            None => return Ok(ToolResult::err("'id' required")),
        };
        let job = match CRON_STORE.get(id) {
            Some(j) => j.clone(),
            None => return Ok(ToolResult::err(format!("Job '{}' not found", id))),
        };
        let out = tokio::process::Command::new("sh").arg("-c").arg(&job.command).output().await;
        let (success, output) = match out {
            Ok(o) => (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()),
            Err(e) => (false, e.to_string()),
        };
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(mut j) = CRON_STORE.get_mut(id) {
            j.last_run = Some(now.clone());
            j.runs.push(now.clone());
        }
        Ok(ToolResult::ok(
            serde_json::json!({"ran": true, "id": id, "success": success, "output": crate::util::truncate(&output, 1000), "ran_at": now}),
        ))
    }
}

#[async_trait]
impl Tool for CronRunsTool {
    fn name(&self) -> &str {
        "cron_runs"
    }
    fn description(&self) -> &str {
        "Get the run history of a cron job."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![ParameterSchema::required("id", "string", "Job ID.")]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = match args["id"].as_str() {
            Some(i) => i,
            None => return Ok(ToolResult::err("'id' required")),
        };
        match CRON_STORE.get(id) {
            Some(j) => Ok(ToolResult::ok(serde_json::json!({"id": id, "runs": j.runs, "last_run": j.last_run}))),
            None => Ok(ToolResult::err(format!("Job '{}' not found", id))),
        }
    }
}

#[async_trait]
impl Tool for CronUpdateTool {
    fn name(&self) -> &str {
        "cron_update"
    }
    fn description(&self) -> &str {
        "Update an existing cron job's schedule or command."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("id", "string", "Job ID to update."),
            ParameterSchema::optional("schedule", "string", "New schedule expression."),
            ParameterSchema::optional("command", "string", "New command."),
            ParameterSchema::optional("enabled", "boolean", "Enable or disable the job."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let id = match args["id"].as_str() {
            Some(i) => i,
            None => return Ok(ToolResult::err("'id' required")),
        };
        match CRON_STORE.get_mut(id) {
            Some(mut j) => {
                if let Some(s) = args["schedule"].as_str() {
                    j.schedule = s.to_string();
                }
                if let Some(c) = args["command"].as_str() {
                    j.command = c.to_string();
                }
                if let Some(e) = args["enabled"].as_bool() {
                    j.enabled = e;
                }
                Ok(ToolResult::ok(serde_json::json!({"updated": true, "id": id})))
            }
            None => Ok(ToolResult::err(format!("Job '{}' not found", id))),
        }
    }
}
