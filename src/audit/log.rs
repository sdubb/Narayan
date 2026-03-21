//! Immutable, append-only audit log backed by PostgreSQL.
//!
//! Every action that mutates state — tool execution, API call, auth event,
//! agent state transition, credential change — gets an audit entry.
//! Entries are INSERT-only; there is no UPDATE or DELETE.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Categories of auditable actions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    // ── Auth ──────────────────────────────────────────────────────
    TenantRegistered,
    TokenIssued,
    // ── Credentials ──────────────────────────────────────────────
    CredentialSet,
    CredentialDeleted,
    RoutingUpdated,
    // ── Agent lifecycle ──────────────────────────────────────────
    GoalCreated,
    AgentPaused,
    AgentResumed,
    AgentClarified,
    // ── Execution ────────────────────────────────────────────────
    StepStarted,
    StepCompleted,
    ToolExecuted,
    ToolBlocked,
    // ── Gateway ──────────────────────────────────────────────────
    LlmCallCompleted,
    SpendLimitExceeded,
    SpendLimitWarning,
    // ── Admin ────────────────────────────────────────────────────
    TenantSuspended,
    TenantPlanChanged,
    // ── Webhooks ─────────────────────────────────────────────────
    WebhookRegistered,
    WebhookDelivered,
    WebhookFailed,
    // ── Catch-all ────────────────────────────────────────────────
    Custom,
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{:?}", self));
        f.write_str(&s)
    }
}

/// A single immutable audit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub tenant_id: String,
    pub agent_id: Option<String>,
    pub action: AuditAction,
    pub detail: serde_json::Value,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Query parameters for retrieving audit entries.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuditQuery {
    pub tenant_id: Option<String>,
    pub agent_id: Option<String>,
    pub action: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Append-only audit log backed by PostgreSQL.
pub struct AuditLog {
    pool: PgPool,
}

impl AuditLog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id          TEXT PRIMARY KEY,
                tenant_id   TEXT NOT NULL,
                agent_id    TEXT,
                action      TEXT NOT NULL,
                detail      JSONB NOT NULL DEFAULT '{}',
                ip_address  TEXT,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS audit_log_tenant_id ON audit_log (tenant_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS audit_log_agent_id ON audit_log (agent_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS audit_log_action ON audit_log (action)").execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS audit_log_created_at ON audit_log (created_at)")
            .execute(&self.pool)
            .await?;

        // Immutable audit log — prevent UPDATE/DELETE via trigger
        sqlx::query(
            "CREATE OR REPLACE FUNCTION audit_log_immutable() RETURNS TRIGGER AS $$
            BEGIN
                RAISE EXCEPTION 'audit_log is append-only — UPDATE and DELETE are forbidden';
            END;
            $$ LANGUAGE plpgsql",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("DROP TRIGGER IF EXISTS enforce_audit_immutability ON audit_log").execute(&self.pool).await?;
        sqlx::query(
            "CREATE TRIGGER enforce_audit_immutability
                BEFORE UPDATE OR DELETE ON audit_log
                FOR EACH ROW EXECUTE FUNCTION audit_log_immutable()",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Append a new audit entry. This is the only write operation.
    pub async fn append(
        &self,
        tenant_id: &str,
        agent_id: Option<&str>,
        action: AuditAction,
        detail: serde_json::Value,
        ip_address: Option<&str>,
    ) -> Result<String> {
        let id = crate::util::new_id();
        let action_str = action.to_string();

        sqlx::query(
            "INSERT INTO audit_log (id, tenant_id, agent_id, action, detail, ip_address, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(agent_id)
        .bind(&action_str)
        .bind(&detail)
        .bind(ip_address)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Query audit entries with filters.
    pub async fn query(&self, q: &AuditQuery) -> Result<Vec<AuditEntry>> {
        let mut sql = String::from(
            "SELECT id, tenant_id, agent_id, action, detail, ip_address, created_at
             FROM audit_log WHERE 1=1",
        );
        let mut bind_idx = 1u32;
        let mut binds: Vec<String> = Vec::new();

        if let Some(ref tid) = q.tenant_id {
            sql.push_str(&format!(" AND tenant_id = ${}", bind_idx));
            bind_idx += 1;
            binds.push(tid.clone());
        }
        if let Some(ref aid) = q.agent_id {
            sql.push_str(&format!(" AND agent_id = ${}", bind_idx));
            bind_idx += 1;
            binds.push(aid.clone());
        }
        if let Some(ref action) = q.action {
            sql.push_str(&format!(" AND action = ${}", bind_idx));
            bind_idx += 1;
            binds.push(action.clone());
        }
        if let Some(ref from) = q.from {
            sql.push_str(&format!(" AND created_at >= ${}", bind_idx));
            bind_idx += 1;
            binds.push(from.to_rfc3339());
        }
        if let Some(ref to) = q.to {
            sql.push_str(&format!(" AND created_at <= ${}", bind_idx));
            bind_idx += 1;
            binds.push(to.to_rfc3339());
        }
        let _ = bind_idx; // suppress unused warning

        sql.push_str(" ORDER BY created_at DESC");
        let limit = q.limit.unwrap_or(100).min(1000);
        let offset = q.offset.unwrap_or(0);
        sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

        // Build the query dynamically
        let mut query = sqlx::query_as::<_, AuditRow>(&sql);
        for b in &binds {
            query = query.bind(b);
        }

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(row_to_entry).collect())
    }

    /// Count entries matching a filter (for pagination).
    pub async fn count(&self, tenant_id: &str) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_log WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: String,
    tenant_id: String,
    agent_id: Option<String>,
    action: String,
    detail: serde_json::Value,
    ip_address: Option<String>,
    created_at: DateTime<Utc>,
}

fn row_to_entry(r: AuditRow) -> AuditEntry {
    let action = serde_json::from_value(serde_json::Value::String(r.action.clone())).unwrap_or(AuditAction::Custom);
    AuditEntry {
        id: r.id,
        tenant_id: r.tenant_id,
        agent_id: r.agent_id,
        action,
        detail: r.detail,
        ip_address: r.ip_address,
        created_at: r.created_at,
    }
}
