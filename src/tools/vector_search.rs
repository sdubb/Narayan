//! vector_search — Semantic search over agent memory using pgvector.
//!
//! Embeds the query, then returns the most semantically similar stored documents.
//! Supports cross-agent search (search all docs for this tenant)
//! or scoped to a specific agent.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    memory::{EmbeddingModel, PgVectorStore, VectorStore},
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct VectorSearchTool {
    pub store: Arc<PgVectorStore>,
    pub embedder: Arc<dyn EmbeddingModel>,
}

#[async_trait]
impl Tool for VectorSearchTool {
    fn name(&self) -> &str {
        "vector_search"
    }
    fn description(&self) -> &str {
        "Search the agent's semantic memory using a natural language query. \
         Returns the most relevant stored documents ranked by cosine similarity. \
         Use after vector_store to retrieve previously stored knowledge."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("query", "string", "Natural language search query."),
            ParameterSchema::required("tenant_id", "string", "Tenant ID — injected automatically."),
            ParameterSchema::optional(
                "agent_id",
                "string",
                "Scope search to this agent only. Omit to search across all agents.",
            ),
            ParameterSchema::optional("top_k", "integer", "Max results to return (default: 5, max: 20)."),
            ParameterSchema::optional(
                "min_score",
                "number",
                "Minimum similarity score 0.0–1.0 (default: 0.3). Higher = stricter.",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = match args["query"].as_str() {
            Some(q) => q,
            None => return Ok(ToolResult::err("'query' required")),
        };
        let tenant_id = match args["tenant_id"].as_str() {
            Some(t) => t,
            None => return Ok(ToolResult::err("'tenant_id' required")),
        };
        let agent_id = args["agent_id"].as_str();
        let top_k = args["top_k"].as_u64().unwrap_or(5).min(20) as usize;
        let min_score = args["min_score"].as_f64().unwrap_or(0.3) as f32;

        // Embed the query
        let query_embedding = match self.embedder.embed(query).await {
            Ok(e) => e,
            Err(e) => return Ok(ToolResult::err(format!("embedding failed: {}", e))),
        };

        match self.store.search(tenant_id, agent_id, query_embedding, top_k, min_score).await {
            Ok(results) => {
                let docs: Vec<serde_json::Value> = results
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "doc_id":   r.document.id,
                            "agent_id": r.document.agent_id,
                            "score":    (r.score * 1000.0).round() / 1000.0,
                            "content":  crate::util::truncate(&r.document.content, 2000),
                            "metadata": r.document.metadata,
                            "stored_at": r.document.created_at,
                        })
                    })
                    .collect();

                Ok(ToolResult::ok(serde_json::json!({
                    "query":      query,
                    "count":      docs.len(),
                    "results":    docs,
                    "model":      self.embedder.model_name(),
                    "scope":      agent_id.unwrap_or("all agents"),
                })))
            }
            Err(e) => Ok(ToolResult::err(format!("search failed: {}", e))),
        }
    }
}
