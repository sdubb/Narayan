use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    state::{SessionTask, SessionTaskOutput, SessionTaskResultStatus, SessionTaskStatus},
    storage::PostgresStore,
    tools::{ParameterSchema, Tool, ToolResult},
};

fn required_string(args: &serde_json::Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("'{}' is required", key))
}

fn optional_string(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key).and_then(|value| value.as_str()).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn parse_string_array(args: &serde_json::Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn parse_status(value: Option<&str>) -> Option<SessionTaskStatus> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "pending" => Some(SessionTaskStatus::Pending),
        "in_progress" => Some(SessionTaskStatus::InProgress),
        "blocked" => Some(SessionTaskStatus::Blocked),
        "completed" => Some(SessionTaskStatus::Completed),
        "failed" => Some(SessionTaskStatus::Failed),
        "stopped" => Some(SessionTaskStatus::Stopped),
        _ => None,
    }
}

fn task_to_json(task: &SessionTask) -> serde_json::Value {
    serde_json::to_value(task).unwrap_or_else(|_| serde_json::json!({}))
}

#[derive(Clone)]
pub struct TaskCreateTool {
    store: Arc<PostgresStore>,
}

impl TaskCreateTool {
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskCreateTool {
    fn name(&self) -> &str {
        "task_create"
    }

    fn description(&self) -> &str {
        "Create a durable session task that tracks planning or execution scaffolding for the current agent."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("subject", "string", "Short task title."),
            ParameterSchema::optional("description", "string", "Detailed task description."),
            ParameterSchema::optional("owner", "string", "Assigned worker or role name."),
            ParameterSchema::optional("blocked_by", "array", "Task IDs that block this task."),
            ParameterSchema::optional("blocks", "array", "Task IDs this task blocks."),
            ParameterSchema::optional("metadata", "object", "Structured task metadata."),
            ParameterSchema::required("tenant_id", "string", "Injected automatically."),
            ParameterSchema::required("agent_id", "string", "Injected automatically."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "type": "object", "additionalProperties": true }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let agent_id = match required_string(&args, "agent_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let subject = match required_string(&args, "subject") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };

        let mut task = SessionTask::new(
            uuid::Uuid::new_v4().to_string(),
            tenant_id,
            agent_id,
            subject,
            optional_string(&args, "description").unwrap_or_default(),
        );
        task.owner = optional_string(&args, "owner");
        task.blocked_by = parse_string_array(&args, "blocked_by");
        task.blocks = parse_string_array(&args, "blocks");
        task.metadata = args.get("metadata").cloned().unwrap_or_else(|| serde_json::json!({}));

        self.store.upsert_session_task(&task).await?;
        Ok(ToolResult::ok(serde_json::json!({
            "status": "created",
            "task": task_to_json(&task),
        })))
    }
}

#[derive(Clone)]
pub struct TaskGetTool {
    store: Arc<PostgresStore>,
}

impl TaskGetTool {
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskGetTool {
    fn name(&self) -> &str {
        "task_get"
    }

    fn description(&self) -> &str {
        "Fetch one durable session task by ID."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("task_id", "string", "Task ID."),
            ParameterSchema::required("tenant_id", "string", "Injected automatically."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let task_id = match required_string(&args, "task_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let Some(task) = self.store.get_session_task(&tenant_id, &task_id).await? else {
            return Ok(ToolResult::err(format!("task '{}' not found", task_id)));
        };
        Ok(ToolResult::ok(serde_json::json!({
            "task": task_to_json(&task),
        })))
    }
}

#[derive(Clone)]
pub struct TaskListTool {
    store: Arc<PostgresStore>,
}

impl TaskListTool {
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskListTool {
    fn name(&self) -> &str {
        "task_list"
    }

    fn description(&self) -> &str {
        "List durable session tasks for the current agent."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("tenant_id", "string", "Injected automatically."),
            ParameterSchema::required("agent_id", "string", "Injected automatically."),
            ParameterSchema::optional("status", "string", "Optional status filter."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let agent_id = match required_string(&args, "agent_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let status_filter = parse_status(args.get("status").and_then(|value| value.as_str()));

        let mut tasks = self.store.list_session_tasks_for_agent(&tenant_id, &agent_id).await?;
        if let Some(status) = status_filter {
            tasks.retain(|task| task.status == status);
        }

        Ok(ToolResult::ok(serde_json::json!({
            "tasks": tasks.into_iter().map(|task| task_to_json(&task)).collect::<Vec<_>>(),
        })))
    }
}

#[derive(Clone)]
pub struct TaskUpdateTool {
    store: Arc<PostgresStore>,
}

impl TaskUpdateTool {
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskUpdateTool {
    fn name(&self) -> &str {
        "task_update"
    }

    fn description(&self) -> &str {
        "Update task state, ownership, dependencies, or metadata."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("task_id", "string", "Task ID."),
            ParameterSchema::required("tenant_id", "string", "Injected automatically."),
            ParameterSchema::optional("subject", "string", "Updated task title."),
            ParameterSchema::optional("description", "string", "Updated task description."),
            ParameterSchema::optional("status", "string", "pending|in_progress|blocked|completed|failed|stopped"),
            ParameterSchema::optional("owner", "string", "Updated owner."),
            ParameterSchema::optional("blocked_by", "array", "Task IDs that block this task."),
            ParameterSchema::optional("blocks", "array", "Task IDs this task blocks."),
            ParameterSchema::optional("metadata", "object", "Structured task metadata."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let task_id = match required_string(&args, "task_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let Some(mut task) = self.store.get_session_task(&tenant_id, &task_id).await? else {
            return Ok(ToolResult::err(format!("task '{}' not found", task_id)));
        };

        if let Some(subject) = optional_string(&args, "subject") {
            task.subject = subject;
        }
        if let Some(description) = optional_string(&args, "description") {
            task.description = description;
        }
        if let Some(owner) = optional_string(&args, "owner") {
            task.owner = Some(owner);
        }
        if args.get("blocked_by").is_some() {
            task.blocked_by = parse_string_array(&args, "blocked_by");
        }
        if args.get("blocks").is_some() {
            task.blocks = parse_string_array(&args, "blocks");
        }
        if let Some(metadata) = args.get("metadata") {
            task.metadata = metadata.clone();
        }
        if let Some(status) = parse_status(args.get("status").and_then(|value| value.as_str())) {
            task.set_status(status);
        } else {
            task.updated_at = chrono::Utc::now();
        }

        self.store.upsert_session_task(&task).await?;
        Ok(ToolResult::ok(serde_json::json!({
            "status": "updated",
            "task": task_to_json(&task),
        })))
    }
}

#[derive(Clone)]
pub struct TaskStopTool {
    store: Arc<PostgresStore>,
}

impl TaskStopTool {
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskStopTool {
    fn name(&self) -> &str {
        "task_stop"
    }

    fn description(&self) -> &str {
        "Stop a task cleanly without deleting its history."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("task_id", "string", "Task ID."),
            ParameterSchema::required("tenant_id", "string", "Injected automatically."),
            ParameterSchema::optional("reason", "string", "Why the task was stopped."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let task_id = match required_string(&args, "task_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let Some(mut task) = self.store.get_session_task(&tenant_id, &task_id).await? else {
            return Ok(ToolResult::err(format!("task '{}' not found", task_id)));
        };

        task.set_status(SessionTaskStatus::Stopped);
        if let Some(reason) = optional_string(&args, "reason") {
            task.metadata["stop_reason"] = serde_json::json!(reason);
        }
        self.store.upsert_session_task(&task).await?;
        Ok(ToolResult::ok(serde_json::json!({
            "status": "stopped",
            "task": task_to_json(&task),
        })))
    }
}

#[derive(Clone)]
pub struct TaskOutputTool {
    store: Arc<PostgresStore>,
}

impl TaskOutputTool {
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "task_output"
    }

    fn description(&self) -> &str {
        "Attach a structured result contract to a task so coordinator-style synthesis has durable findings and artifacts."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("task_id", "string", "Task ID."),
            ParameterSchema::required("tenant_id", "string", "Injected automatically."),
            ParameterSchema::required("status", "string", "complete|partial|failed"),
            ParameterSchema::optional("artifacts", "array", "Artifact paths or identifiers."),
            ParameterSchema::optional("findings", "array", "Concrete findings."),
            ParameterSchema::optional("confidence", "number", "Confidence between 0 and 1."),
            ParameterSchema::optional("note", "string", "Optional summary note."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match required_string(&args, "tenant_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let task_id = match required_string(&args, "task_id") {
            Ok(value) => value,
            Err(message) => return Ok(ToolResult::err(message)),
        };
        let Some(mut task) = self.store.get_session_task(&tenant_id, &task_id).await? else {
            return Ok(ToolResult::err(format!("task '{}' not found", task_id)));
        };

        let status =
            match args.get("status").and_then(|value| value.as_str()).map(|value| value.trim().to_ascii_lowercase()) {
                Some(value) if value == "complete" => SessionTaskResultStatus::Complete,
                Some(value) if value == "partial" => SessionTaskResultStatus::Partial,
                Some(value) if value == "failed" => SessionTaskResultStatus::Failed,
                _ => return Ok(ToolResult::err("'status' must be complete, partial, or failed")),
            };
        let output = SessionTaskOutput {
            status: status.clone(),
            artifacts: parse_string_array(&args, "artifacts"),
            findings: parse_string_array(&args, "findings"),
            confidence: args.get("confidence").and_then(|value| value.as_f64()).unwrap_or(1.0).clamp(0.0, 1.0),
            note: optional_string(&args, "note"),
        };
        task.set_output(output);
        match status {
            SessionTaskResultStatus::Complete => task.set_status(SessionTaskStatus::Completed),
            SessionTaskResultStatus::Partial => task.set_status(SessionTaskStatus::Blocked),
            SessionTaskResultStatus::Failed => task.set_status(SessionTaskStatus::Failed),
        }
        self.store.upsert_session_task(&task).await?;
        Ok(ToolResult::ok(serde_json::json!({
            "status": "recorded",
            "task": task_to_json(&task),
        })))
    }
}
