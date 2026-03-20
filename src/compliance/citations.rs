//! Citation tracking — records source attribution for every claim/action an agent makes.
//!
//! Each citation links an agent's output to the source material it was derived from.
//! This is critical for compliance ops where audit trails must show provenance.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub id: String,
    pub agent_id: String,
    pub tenant_id: String,
    pub step_index: usize,
    /// The claim or output that needs attribution.
    pub claim: String,
    /// Source type: "document", "url", "tool_output", "memory", "user_input".
    pub source_type: String,
    /// Reference to the source (document ID, URL, tool name, etc.).
    pub source_ref: String,
    /// Relevant excerpt from the source.
    pub excerpt: String,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
}

pub struct CitationTracker {
    pool: PgPool,
}

impl CitationTracker {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS citations (
                id          TEXT PRIMARY KEY,
                agent_id    TEXT NOT NULL,
                tenant_id   TEXT NOT NULL,
                step_index  INT NOT NULL,
                claim       TEXT NOT NULL,
                source_type TEXT NOT NULL,
                source_ref  TEXT NOT NULL,
                excerpt     TEXT NOT NULL DEFAULT '',
                confidence  DOUBLE PRECISION NOT NULL DEFAULT 1.0,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS citations_agent_id ON citations (agent_id)")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS citations_tenant_id ON citations (tenant_id)")
            .execute(&self.pool).await?;
        Ok(())
    }

    /// Record a citation linking an agent's claim to its source.
    pub async fn record(
        &self,
        agent_id: &str,
        tenant_id: &str,
        step_index: usize,
        claim: &str,
        source_type: &str,
        source_ref: &str,
        excerpt: &str,
        confidence: f64,
    ) -> Result<String> {
        let id = crate::util::new_id();
        sqlx::query(
            "INSERT INTO citations (id, agent_id, tenant_id, step_index, claim, source_type, source_ref, excerpt, confidence)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&id)
        .bind(agent_id)
        .bind(tenant_id)
        .bind(step_index as i32)
        .bind(claim)
        .bind(source_type)
        .bind(source_ref)
        .bind(excerpt)
        .bind(confidence)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get all citations for an agent (for evidence packaging / audit).
    pub async fn get_for_agent(&self, agent_id: &str) -> Result<Vec<Citation>> {
        let rows = sqlx::query_as::<_, CitationRow>(
            "SELECT id, agent_id, tenant_id, step_index, claim, source_type, source_ref, excerpt, confidence, created_at
             FROM citations WHERE agent_id = $1 ORDER BY step_index, created_at",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_citation).collect())
    }

    /// Get citations for a tenant (cross-agent).
    pub async fn get_for_tenant(&self, tenant_id: &str, limit: i64) -> Result<Vec<Citation>> {
        let rows = sqlx::query_as::<_, CitationRow>(
            "SELECT id, agent_id, tenant_id, step_index, claim, source_type, source_ref, excerpt, confidence, created_at
             FROM citations WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_citation).collect())
    }
}

#[derive(sqlx::FromRow)]
struct CitationRow {
    id: String,
    agent_id: String,
    tenant_id: String,
    step_index: i32,
    claim: String,
    source_type: String,
    source_ref: String,
    excerpt: String,
    confidence: f64,
    created_at: DateTime<Utc>,
}

fn row_to_citation(r: CitationRow) -> Citation {
    Citation {
        id: r.id,
        agent_id: r.agent_id,
        tenant_id: r.tenant_id,
        step_index: r.step_index as usize,
        claim: r.claim,
        source_type: r.source_type,
        source_ref: r.source_ref,
        excerpt: r.excerpt,
        confidence: r.confidence,
        created_at: r.created_at,
    }
}
