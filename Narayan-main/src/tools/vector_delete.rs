//! vector_delete — Remove documents from semantic memory.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    memory::{PgVectorStore, VectorStore},
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct VectorDeleteTool {
    pub store: Arc<PgVectorStore>,
}

#[async_trait]
impl Tool for VectorDeleteTool {
    fn name(&self) -> &str {
        "vector_delete"
    }
    fn description(&self) -> &str {
        "Delete documents from semantic memory. Either delete a specific document by ID, \
         or clear all memory for an agent."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("tenant_id", "string", "Tenant ID — injected automatically."),
            ParameterSchema::optional("doc_id", "string", "Specific document ID to delete."),
            ParameterSchema::optional("agent_id", "string", "Delete ALL documents for this agent. Use with caution."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match args["tenant_id"].as_str() {
            Some(t) => t,
            None => return Ok(ToolResult::err("'tenant_id' required")),
        };

        if let Some(doc_id) = args["doc_id"].as_str() {
            match self.store.delete(tenant_id, doc_id).await {
                Ok(()) => Ok(ToolResult::ok(serde_json::json!({"deleted": true, "doc_id": doc_id}))),
                Err(e) => Ok(ToolResult::err(format!("delete failed: {}", e))),
            }
        } else if let Some(agent_id) = args["agent_id"].as_str() {
            match self.store.delete_by_agent(tenant_id, agent_id).await {
                Ok(n) => Ok(ToolResult::ok(serde_json::json!({"deleted": n, "agent_id": agent_id}))),
                Err(e) => Ok(ToolResult::err(format!("delete_by_agent failed: {}", e))),
            }
        } else {
            Ok(ToolResult::err("'doc_id' or 'agent_id' is required"))
        }
    }
}
