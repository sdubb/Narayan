//! vector_store — Embed text and store it in pgvector semantic memory.
//!
//! Agents use this to persist knowledge across steps and across runs.
//! Stored documents are retrievable via vector_search using semantic similarity.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    memory::{EmbeddingModel, PgVectorStore, VectorDocument, VectorStore},
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct VectorStoreTool {
    pub store: Arc<PgVectorStore>,
    pub embedder: Arc<dyn EmbeddingModel>,
}

#[async_trait]
impl Tool for VectorStoreTool {
    fn name(&self) -> &str {
        "vector_store"
    }
    fn description(&self) -> &str {
        "Embed text and store it in the agent's semantic memory (pgvector). \
         Content is retrievable later via vector_search using natural language queries. \
         Use for facts, findings, summaries, code snippets — anything worth remembering."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("content", "string", "Text content to embed and store."),
            ParameterSchema::required("agent_id", "string", "Agent ID — injected automatically."),
            ParameterSchema::required("tenant_id", "string", "Tenant ID — injected automatically."),
            ParameterSchema::optional(
                "doc_id",
                "string",
                "Explicit document ID (for idempotent upserts). Auto-generated if omitted.",
            ),
            ParameterSchema::optional(
                "metadata",
                "object",
                "Arbitrary metadata to attach: {source, url, step, type, ...}.",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let content = match args["content"].as_str() {
            Some(c) => c,
            None => return Ok(ToolResult::err("'content' required")),
        };
        let agent_id = match args["agent_id"].as_str() {
            Some(a) => a,
            None => return Ok(ToolResult::err("'agent_id' required")),
        };
        let tenant_id = match args["tenant_id"].as_str() {
            Some(t) => t,
            None => return Ok(ToolResult::err("'tenant_id' required")),
        };
        let metadata = args["metadata"].clone();

        // Embed the content
        let embedding = match self.embedder.embed(content).await {
            Ok(e) => e,
            Err(e) => return Ok(ToolResult::err(format!("embedding failed: {}", e))),
        };

        let mut doc = VectorDocument::new(tenant_id.to_string(), agent_id.to_string(), content.to_string(), embedding);

        // Allow caller to specify doc_id for idempotent upserts
        if let Some(id) = args["doc_id"].as_str() {
            doc.id = id.to_string();
        }
        if !metadata.is_null() {
            doc.metadata = metadata;
        }

        let doc_id = doc.id.clone();

        match self.store.upsert(doc).await {
            Ok(()) => Ok(ToolResult::ok(serde_json::json!({
                "stored":     true,
                "doc_id":     doc_id,
                "model":      self.embedder.model_name(),
                "dimensions": self.embedder.dimension(),
                "chars":      content.len(),
            }))),
            Err(e) => Ok(ToolResult::err(format!("store failed: {}", e))),
        }
    }
}
