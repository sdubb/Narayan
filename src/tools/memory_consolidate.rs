use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    memory::{apply_consolidation_metadata, MemoryConsolidator},
    storage::PostgresStore,
    tools::{ParameterSchema, Tool, ToolResult},
};

#[derive(Clone)]
pub struct MemoryConsolidateTool {
    store: Arc<PostgresStore>,
    consolidator: Arc<MemoryConsolidator>,
}

impl MemoryConsolidateTool {
    pub fn new(store: Arc<PostgresStore>, consolidator: Arc<MemoryConsolidator>) -> Self {
        Self { store, consolidator }
    }
}

#[async_trait]
impl Tool for MemoryConsolidateTool {
    fn name(&self) -> &str {
        "memory_consolidate"
    }

    fn description(&self) -> &str {
        "Run a durable memory consolidation pass for a successfully completed agent. This merges recent successful signal into topic memories, updates the memory index, and prunes stale topics."
    }

    fn category(&self) -> &'static str {
        "memory"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("tenant_id", "string", "Tenant ID injected automatically."),
            ParameterSchema::required("agent_id", "string", "Agent ID injected automatically."),
            ParameterSchema::optional("force", "boolean", "Bypass the no-new-signal gate while still requiring a completed successful run."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tenant_id = match args.get("tenant_id").and_then(|value| value.as_str()) {
            Some(value) if !value.trim().is_empty() => value,
            _ => return Ok(ToolResult::err("'tenant_id' is required")),
        };
        let agent_id = match args.get("agent_id").and_then(|value| value.as_str()) {
            Some(value) if !value.trim().is_empty() => value,
            _ => return Ok(ToolResult::err("'agent_id' is required")),
        };
        let force = args.get("force").and_then(|value| value.as_bool()).unwrap_or(false);

        let Some(mut state) = self.store.get_agent(tenant_id, agent_id).await? else {
            return Ok(ToolResult::err(format!("agent '{}' was not found", agent_id)));
        };

        let result = self.consolidator.consolidate_agent(&state, force).await?;
        if !result.skipped {
            apply_consolidation_metadata(&mut state, &result);
            self.store.upsert_agent(&state).await?;
        }

        Ok(ToolResult::ok(serde_json::json!({
            "changed": result.changed,
            "skipped": result.skipped,
            "summary": result.summary,
            "topics_saved": result.topics_saved,
            "pruned_topics": result.pruned_topics,
            "topic_keys": result.topic_keys,
            "index_key": result.index_key,
        })))
    }
}
