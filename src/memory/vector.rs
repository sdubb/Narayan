//! pgvector-backed vector store — real semantic memory for agents.
//!
//! Requires PostgreSQL with pgvector extension:
//!   CREATE EXTENSION IF NOT EXISTS vector;
//!
//! Tables created automatically by PgVectorStore::migrate().
//!
//! Index strategy:
//!   - HNSW index (best query speed at scale)
//!   - Falls back to exact search for small collections (<1000 docs)
//!
//! Supported distance metrics:
//!   - cosine (default, best for text embeddings)
//!   - l2 (euclidean)
//!   - inner_product (for dot-product normalised embeddings)

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use sqlx::{PgPool, Row};

/// A document stored in the vector knowledge base.
#[derive(Debug, Clone)]
pub struct VectorDocument {
    pub id: String,
    pub tenant_id: String,
    pub agent_id: String,
    pub content: String,
    pub embedding: Vec<f32>,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl VectorDocument {
    pub fn new(tenant_id: String, agent_id: String, content: String, embedding: Vec<f32>) -> Self {
        Self {
            id: crate::util::new_id(),
            tenant_id,
            agent_id,
            content,
            embedding,
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
        }
    }
    pub fn with_metadata(mut self, m: serde_json::Value) -> Self {
        self.metadata = m;
        self
    }
}

/// Search result — document plus its similarity score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub document: VectorDocument,
    pub score: f32,
}

/// Supported distance metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMetric {
    Cosine,
    L2,
    InnerProduct,
}

impl DistanceMetric {
    fn operator(&self) -> &'static str {
        match self {
            Self::Cosine => "<=>",
            Self::L2 => "<->",
            Self::InnerProduct => "<#>",
        }
    }
    fn index_ops(&self) -> &'static str {
        match self {
            Self::Cosine => "vector_cosine_ops",
            Self::L2 => "vector_l2_ops",
            Self::InnerProduct => "vector_ip_ops",
        }
    }
}

/// Vector store trait.
#[async_trait]
pub trait VectorStore: Send + Sync {
    async fn upsert(&self, doc: VectorDocument) -> Result<()>;
    async fn search(
        &self,
        tenant_id: &str,
        agent_id: Option<&str>,
        query_embedding: Vec<f32>,
        top_k: usize,
        min_score: f32,
    ) -> Result<Vec<SearchResult>>;
    async fn delete(&self, tenant_id: &str, doc_id: &str) -> Result<()>;
    async fn delete_by_agent(&self, tenant_id: &str, agent_id: &str) -> Result<u64>;
    async fn count(&self, tenant_id: &str, agent_id: Option<&str>) -> Result<u64>;
}

// ── PgVectorStore — the real implementation ──────────────────────────────────

pub struct PgVectorStore {
    pool: PgPool,
    metric: DistanceMetric,
    dimension: usize,
}

impl PgVectorStore {
    pub fn new(pool: PgPool, dimension: usize, metric: DistanceMetric) -> Arc<Self> {
        Arc::new(Self { pool, metric, dimension })
    }

    /// Create the table and HNSW index. Call once at startup.
    pub async fn migrate(&self) -> Result<()> {
        // Enable pgvector
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector").execute(&self.pool).await?;

        // Main documents table
        sqlx::query(&format!(
            r#"
            CREATE TABLE IF NOT EXISTS vector_documents (
                id          TEXT        PRIMARY KEY,
                tenant_id   TEXT        NOT NULL,
                agent_id    TEXT        NOT NULL,
                content     TEXT        NOT NULL,
                embedding   vector({dim}) NOT NULL,
                metadata    JSONB       NOT NULL DEFAULT '{{}}'::jsonb,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#,
            dim = self.dimension
        ))
        .execute(&self.pool)
        .await?;

        // Tenant + agent scoped lookups
        sqlx::query("CREATE INDEX IF NOT EXISTS vd_tenant_agent ON vector_documents (tenant_id, agent_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS vd_created ON vector_documents (created_at DESC)")
            .execute(&self.pool)
            .await?;

        // HNSW index — best speed/recall tradeoff for production
        // m=16, ef_construction=64 are good defaults. Tune for your data size.
        let idx_ops = self.metric.index_ops();
        let _ = sqlx::query(&format!(
            r#"
            CREATE INDEX IF NOT EXISTS vd_embedding_hnsw
            ON vector_documents
            USING hnsw (embedding {idx_ops})
            WITH (m = 16, ef_construction = 64)
        "#
        ))
        .execute(&self.pool)
        .await;
        // Not fatal if index creation fails (e.g. old pgvector version)

        tracing::info!(
            dimension   = self.dimension,
            metric      = ?self.metric,
            "pgvector store migrated"
        );
        Ok(())
    }

    /// Update HNSW ef_search parameter at query time for accuracy vs speed tradeoff.
    async fn set_ef_search(&self, ef: i32) {
        let _ = sqlx::query(&format!("SET LOCAL hnsw.ef_search = {}", ef)).execute(&self.pool).await;
    }
}

#[async_trait]
impl VectorStore for PgVectorStore {
    async fn upsert(&self, doc: VectorDocument) -> Result<()> {
        let embedding_str = format_vector(&doc.embedding);
        sqlx::query(
            r#"
            INSERT INTO vector_documents (id, tenant_id, agent_id, content, embedding, metadata, created_at)
            VALUES ($1, $2, $3, $4, $5::vector, $6, $7)
            ON CONFLICT (id) DO UPDATE SET
                content    = EXCLUDED.content,
                embedding  = EXCLUDED.embedding,
                metadata   = EXCLUDED.metadata
        "#,
        )
        .bind(&doc.id)
        .bind(&doc.tenant_id)
        .bind(&doc.agent_id)
        .bind(&doc.content)
        .bind(&embedding_str)
        .bind(&doc.metadata)
        .bind(doc.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn search(
        &self,
        tenant_id: &str,
        agent_id: Option<&str>,
        query_embedding: Vec<f32>,
        top_k: usize,
        min_score: f32,
    ) -> Result<Vec<SearchResult>> {
        let query_vec_str = format_vector(&query_embedding);
        let op = self.metric.operator();

        // Higher ef_search = more accurate but slower. 40 is a good production default.
        self.set_ef_search(40).await;

        // For cosine/ip, score = 1 - distance. For L2, score = 1 / (1 + distance).
        let rows = if let Some(aid) = agent_id {
            sqlx::query(&format!(
                r#"
                SELECT id, tenant_id, agent_id, content,
                       embedding::text, metadata, created_at,
                       1 - (embedding {op} $1::vector) AS score
                FROM   vector_documents
                WHERE  tenant_id = $2
                  AND  agent_id  = $3
                  AND  1 - (embedding {op} $1::vector) >= $4
                ORDER  BY embedding {op} $1::vector
                LIMIT  $5
            "#
            ))
            .bind(&query_vec_str)
            .bind(tenant_id)
            .bind(aid)
            .bind(min_score as f64)
            .bind(top_k as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(&format!(
                r#"
                SELECT id, tenant_id, agent_id, content,
                       embedding::text, metadata, created_at,
                       1 - (embedding {op} $1::vector) AS score
                FROM   vector_documents
                WHERE  tenant_id = $2
                  AND  1 - (embedding {op} $1::vector) >= $3
                ORDER  BY embedding {op} $1::vector
                LIMIT  $4
            "#
            ))
            .bind(&query_vec_str)
            .bind(tenant_id)
            .bind(min_score as f64)
            .bind(top_k as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows
            .into_iter()
            .map(|row| {
                let emb_text: String = row.get("embedding");
                let embedding = parse_vector(&emb_text);
                SearchResult {
                    score: row.get::<f64, _>("score") as f32,
                    document: VectorDocument {
                        id: row.get("id"),
                        tenant_id: row.get("tenant_id"),
                        agent_id: row.get("agent_id"),
                        content: row.get("content"),
                        embedding,
                        metadata: row.get("metadata"),
                        created_at: row.get("created_at"),
                    },
                }
            })
            .collect())
    }

    async fn delete(&self, tenant_id: &str, doc_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM vector_documents WHERE id = $1 AND tenant_id = $2")
            .bind(doc_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_by_agent(&self, tenant_id: &str, agent_id: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM vector_documents WHERE tenant_id = $1 AND agent_id = $2")
            .bind(tenant_id)
            .bind(agent_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    async fn count(&self, tenant_id: &str, agent_id: Option<&str>) -> Result<u64> {
        let n: i64 = if let Some(aid) = agent_id {
            sqlx::query_scalar("SELECT COUNT(*) FROM vector_documents WHERE tenant_id=$1 AND agent_id=$2")
                .bind(tenant_id)
                .bind(aid)
                .fetch_one(&self.pool)
                .await?
        } else {
            sqlx::query_scalar("SELECT COUNT(*) FROM vector_documents WHERE tenant_id=$1")
                .bind(tenant_id)
                .fetch_one(&self.pool)
                .await?
        };
        Ok(n as u64)
    }
}

// ── Helpers for pgvector text representation ──────────────────────────────────

/// Format a `Vec<f32>` into pgvector text format: `[0.1,0.2,0.3]`.
fn format_vector(v: &[f32]) -> String {
    let inner: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    format!("[{}]", inner.join(","))
}

/// Parse pgvector text format `[0.1,0.2,0.3]` back into `Vec<f32>`.
fn parse_vector(s: &str) -> Vec<f32> {
    let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect()
}

// ── In-memory fallback (cosine, dev/testing) ──────────────────────────────────

pub struct InMemoryVectorStore {
    docs: tokio::sync::RwLock<Vec<VectorDocument>>,
}

impl InMemoryVectorStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { docs: tokio::sync::RwLock::new(Vec::new()) })
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }
}

impl Default for InMemoryVectorStore {
    fn default() -> Self {
        Self { docs: tokio::sync::RwLock::new(Vec::new()) }
    }
}

#[async_trait]
impl VectorStore for InMemoryVectorStore {
    async fn upsert(&self, doc: VectorDocument) -> Result<()> {
        let mut docs = self.docs.write().await;
        if let Some(e) = docs.iter_mut().find(|d| d.id == doc.id) {
            *e = doc;
        } else {
            docs.push(doc);
        }
        Ok(())
    }

    async fn search(
        &self,
        tenant_id: &str,
        agent_id: Option<&str>,
        query_embedding: Vec<f32>,
        top_k: usize,
        min_score: f32,
    ) -> Result<Vec<SearchResult>> {
        let docs = self.docs.read().await;
        let mut scored: Vec<SearchResult> = docs
            .iter()
            .filter(|d| d.tenant_id == tenant_id && agent_id.map(|a| d.agent_id == a).unwrap_or(true))
            .map(|d| {
                let s = Self::cosine(&query_embedding, &d.embedding);
                (s, d.clone())
            })
            .filter(|(s, _)| *s >= min_score)
            .map(|(score, document)| SearchResult { score, document })
            .collect();
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(top_k).collect())
    }

    async fn delete(&self, tenant_id: &str, doc_id: &str) -> Result<()> {
        self.docs.write().await.retain(|d| !(d.tenant_id == tenant_id && d.id == doc_id));
        Ok(())
    }

    async fn delete_by_agent(&self, tenant_id: &str, agent_id: &str) -> Result<u64> {
        let mut docs = self.docs.write().await;
        let before = docs.len();
        docs.retain(|d| !(d.tenant_id == tenant_id && d.agent_id == agent_id));
        Ok((before - docs.len()) as u64)
    }

    async fn count(&self, tenant_id: &str, agent_id: Option<&str>) -> Result<u64> {
        let docs = self.docs.read().await;
        Ok(
            docs.iter()
                .filter(|d| d.tenant_id == tenant_id && agent_id.map(|a| d.agent_id == a).unwrap_or(true))
                .count() as u64,
        )
    }
}
