use std::sync::Arc;

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_array};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DelegateArgs {
    tenant_id: String,
    parent_id: String,
    sub_goals: Vec<String>,
    worker_type: Option<String>,
    task_id: Option<String>,
    write_scope: Vec<String>,
    continue_child_id: Option<String>,
}

fn parse_delegate_args(args: &serde_json::Value) -> Result<DelegateArgs, String> {
    let tenant_id = args["tenant_id"].as_str().unwrap_or("").to_string();
    let parent_id = args["parent_agent_id"].as_str().unwrap_or("").to_string();
    let sub_goals: Vec<String> =
        args["sub_goals"].as_array().unwrap_or(&vec![]).iter().filter_map(|v| v.as_str().map(String::from)).collect();
    let worker_type = args["worker_type"].as_str().map(String::from);
    let task_id = args["task_id"].as_str().map(String::from);
    let write_scope: Vec<String> = args["write_scope"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|value| value.as_str().map(String::from))
        .collect();
    let continue_child_id = args["continue_child_id"].as_str().map(String::from);

    if sub_goals.is_empty() && continue_child_id.is_none() {
        return Err("sub_goals must not be empty unless continue_child_id is provided".into());
    }
    if tenant_id.is_empty() || parent_id.is_empty() {
        return Err("tenant_id and parent_agent_id are required".into());
    }

    Ok(DelegateArgs { tenant_id, parent_id, sub_goals, worker_type, task_id, write_scope, continue_child_id })
}

fn delegate_result(child_ids: Vec<String>, continued: Option<String>) -> ToolResult {
    ToolResult::ok(serde_json::json!({
        "child_agent_ids": child_ids,
        "continued_child_id": continued,
        "message": format!("{} sub-agents spawned and scheduled", child_ids.len()),
    }))
}

pub struct DelegateTool {
    pub store: Arc<crate::storage::PostgresStore>,
    pub workspace_manager: Arc<crate::workspace::manager::WorkspaceManager>,
    pub swarm: Arc<crate::swarm::Swarm>,
}

impl DelegateTool {
    pub fn new(
        store: Arc<crate::storage::PostgresStore>,
        workspace_manager: Arc<crate::workspace::manager::WorkspaceManager>,
        swarm: Arc<crate::swarm::Swarm>,
    ) -> Self {
        Self { store, workspace_manager, swarm }
    }
}

#[async_trait]
impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }
    fn description(&self) -> &str {
        "Spawn one or more parallel sub-agents to work on independent sub-tasks simultaneously. \
         The current agent pauses until all children complete, then resumes with their combined results."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "sub_goals",
                "array",
                "List of independent sub-goal strings to execute in parallel.",
            ),
            ParameterSchema::required("tenant_id", "string", "Tenant ID — injected automatically by the executor."),
            ParameterSchema::required(
                "parent_agent_id",
                "string",
                "Parent agent ID — injected automatically by the executor.",
            ),
            ParameterSchema::optional(
                "worker_type",
                "string",
                "Worker role such as research, implementation, or verification.",
            ),
            ParameterSchema::optional("task_id", "string", "Associated session task ID."),
            ParameterSchema::optional("write_scope", "array", "Explicit file/module ownership for the worker."),
            ParameterSchema::optional(
                "continue_child_id",
                "string",
                "Existing child agent ID to resume instead of spawning a fresh worker.",
            ),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["child_agent_ids", "message"],
            "properties": {
                "child_agent_ids": schema_array(schema_string()),
                "message": schema_string(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let DelegateArgs { tenant_id, parent_id, sub_goals, worker_type, task_id, write_scope, continue_child_id } =
            match parse_delegate_args(&args) {
                Ok(parsed) => parsed,
                Err(message) => return Ok(ToolResult::err(message)),
            };

        let mut child_ids = Vec::new();
        let mut continued = None;
        if let Some(existing_child_id) = continue_child_id {
            if let Some(mut child) = self.store.get_agent(&tenant_id, &existing_child_id).await? {
                child.metadata["delegation_context"] = serde_json::json!({
                    "worker_type": worker_type,
                    "task_id": task_id,
                    "write_scope": write_scope,
                    "continued_at": chrono::Utc::now().to_rfc3339(),
                });
                self.store.upsert_agent(&child).await?;
                if let Err(e) = self.swarm.push(existing_child_id.clone()).await {
                    tracing::warn!(child = %existing_child_id, error = %e, "swarm enqueue failed for continued child");
                }
                child_ids.push(existing_child_id.clone());
                continued = Some(existing_child_id);
            }
        }

        for sub_goal in &sub_goals {
            let child_id = crate::util::new_id();
            // Use WorkspaceManager so child agents respect hybrid/remote storage mode
            let handle = self.workspace_manager.create(&tenant_id, &child_id).await?;
            let workspace = handle.local_path_str();
            let mut child =
                crate::state::AgentState::new(child_id.clone(), tenant_id.clone(), sub_goal.clone(), workspace);
            child.parent_agent_id = Some(parent_id.clone());
            child.metadata["delegation_context"] = serde_json::json!({
                "worker_type": worker_type,
                "task_id": task_id,
                "write_scope": write_scope,
                "spawned_at": chrono::Utc::now().to_rfc3339(),
            });
            self.store.upsert_agent(&child).await?;
            tracing::info!(parent = %parent_id, child = %child_id, sub_goal = %sub_goal, "child agent spawned");
            // Enqueue child via the shared queue-backed Swarm — no global Mutex.
            if let Err(e) = self.swarm.push(child_id.clone()).await {
                tracing::warn!(child = %child_id, error = %e, "swarm enqueue failed");
            }
            child_ids.push(child_id);
        }
        Ok(delegate_result(child_ids, continued))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_delegate_args_requires_sub_goals() {
        let error = parse_delegate_args(&serde_json::json!({
            "tenant_id": "tenant-1",
            "parent_agent_id": "agent-1",
            "sub_goals": []
        }))
        .expect_err("empty sub-goals should fail");

        assert_eq!(error, "sub_goals must not be empty unless continue_child_id is provided");
    }

    #[test]
    fn test_parse_delegate_args_requires_tenant_and_parent_ids() {
        let error = parse_delegate_args(&serde_json::json!({
            "sub_goals": ["inspect logs"]
        }))
        .expect_err("missing context IDs should fail");

        assert_eq!(error, "tenant_id and parent_agent_id are required");
    }

    #[test]
    fn test_parse_delegate_args_extracts_parallel_sub_goals() {
        let parsed = parse_delegate_args(&serde_json::json!({
            "tenant_id": "tenant-1",
            "parent_agent_id": "agent-1",
            "sub_goals": ["inspect logs", "review CI workflow"]
        }))
        .expect("valid args should parse");

        assert_eq!(
            parsed,
            DelegateArgs {
                tenant_id: "tenant-1".into(),
                parent_id: "agent-1".into(),
                sub_goals: vec!["inspect logs".into(), "review CI workflow".into()],
                worker_type: None,
                task_id: None,
                write_scope: vec![],
                continue_child_id: None,
            }
        );
    }

    #[test]
    fn test_delegate_result_reports_child_count_and_ids() {
        let result = delegate_result(vec!["child-1".into(), "child-2".into()], None);

        assert!(result.success);
        assert_eq!(result.output["child_agent_ids"][0], "child-1");
        assert_eq!(result.output["child_agent_ids"][1], "child-2");
        assert_eq!(result.output["message"], "2 sub-agents spawned and scheduled");
    }
}
