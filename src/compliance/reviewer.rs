//! Reviewer queues — structured human review workflow for agent outputs.
//!
//! Agents can be paused pending review. Reviewers see a queue of items,
//! approve/reject/request-changes, and the agent resumes.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    Approved,
    Rejected,
    ChangesRequested,
}

impl std::fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewStatus::Pending => write!(f, "pending"),
            ReviewStatus::Approved => write!(f, "approved"),
            ReviewStatus::Rejected => write!(f, "rejected"),
            ReviewStatus::ChangesRequested => write!(f, "changes_requested"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: String,
    pub tenant_id: String,
    pub agent_id: String,
    pub step_index: usize,
    /// What needs review (summary of agent's action/output).
    pub summary: String,
    /// Why review is needed (policy rule, risk level, etc.).
    pub reason: String,
    pub status: ReviewStatus,
    /// Reviewer's notes (populated on approve/reject).
    pub reviewer_notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
}

pub struct ReviewQueue {
    pool: PgPool,
}

impl ReviewQueue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS review_queue (
                id              TEXT PRIMARY KEY,
                tenant_id       TEXT NOT NULL,
                agent_id        TEXT NOT NULL,
                step_index      INT NOT NULL,
                summary         TEXT NOT NULL,
                reason          TEXT NOT NULL,
                status          TEXT NOT NULL DEFAULT 'pending',
                reviewer_notes  TEXT,
                created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                reviewed_at     TIMESTAMPTZ
            );
            CREATE INDEX IF NOT EXISTS review_queue_tenant ON review_queue (tenant_id, status);
            CREATE INDEX IF NOT EXISTS review_queue_agent ON review_queue (agent_id);
        "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Submit an item for review — pauses the agent step until reviewed.
    pub async fn submit(
        &self,
        tenant_id: &str,
        agent_id: &str,
        step_index: usize,
        summary: &str,
        reason: &str,
    ) -> Result<String> {
        let id = crate::util::new_id();
        sqlx::query(
            "INSERT INTO review_queue (id, tenant_id, agent_id, step_index, summary, reason)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(agent_id)
        .bind(step_index as i32)
        .bind(summary)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Get all pending reviews for a tenant.
    pub async fn pending(&self, tenant_id: &str) -> Result<Vec<ReviewItem>> {
        let rows = sqlx::query_as::<_, ReviewRow>(
            "SELECT id, tenant_id, agent_id, step_index, summary, reason, status, reviewer_notes,
                    created_at, reviewed_at
             FROM review_queue WHERE tenant_id = $1 AND status = 'pending'
             ORDER BY created_at",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_item).collect())
    }

    /// Approve or reject a review item.
    pub async fn resolve(
        &self,
        review_id: &str,
        status: ReviewStatus,
        notes: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE review_queue SET status = $1, reviewer_notes = $2, reviewed_at = NOW()
             WHERE id = $3",
        )
        .bind(status.to_string())
        .bind(notes)
        .bind(review_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get review status for an agent + step (used by executor to check if it can proceed).
    pub async fn get_for_step(&self, agent_id: &str, step_index: usize) -> Result<Option<ReviewItem>> {
        let row = sqlx::query_as::<_, ReviewRow>(
            "SELECT id, tenant_id, agent_id, step_index, summary, reason, status, reviewer_notes,
                    created_at, reviewed_at
             FROM review_queue WHERE agent_id = $1 AND step_index = $2
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(agent_id)
        .bind(step_index as i32)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_item))
    }
}

#[derive(sqlx::FromRow)]
struct ReviewRow {
    id: String,
    tenant_id: String,
    agent_id: String,
    step_index: i32,
    summary: String,
    reason: String,
    status: String,
    reviewer_notes: Option<String>,
    created_at: DateTime<Utc>,
    reviewed_at: Option<DateTime<Utc>>,
}

fn row_to_item(r: ReviewRow) -> ReviewItem {
    let status = match r.status.as_str() {
        "approved" => ReviewStatus::Approved,
        "rejected" => ReviewStatus::Rejected,
        "changes_requested" => ReviewStatus::ChangesRequested,
        _ => ReviewStatus::Pending,
    };
    ReviewItem {
        id: r.id,
        tenant_id: r.tenant_id,
        agent_id: r.agent_id,
        step_index: r.step_index as usize,
        summary: r.summary,
        reason: r.reason,
        status,
        reviewer_notes: r.reviewer_notes,
        created_at: r.created_at,
        reviewed_at: r.reviewed_at,
    }
}
