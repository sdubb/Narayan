use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Row};

use crate::{
    state::{AgentState, AgentStatus, GoalState, GoalStatus},
    workspace::{manager::WorkspaceInfo, resolver::WorkspaceMode},
};

pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub async fn new(database_url: &str, max_connections: u32) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(max_connections).connect(database_url).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }

    /// Simple connectivity check for readiness probes.
    pub async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn migrate(&self) -> Result<()> {
        // pgvector extension — optional, warn if unavailable
        let _ = sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(&self.pool)
            .await;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS agents (
                id               TEXT PRIMARY KEY,
                tenant_id        TEXT NOT NULL,
                goal             TEXT NOT NULL,
                status           TEXT NOT NULL DEFAULT 'pending',
                current_task     TEXT,
                current_step     INTEGER NOT NULL DEFAULT 0,
                workspace_path   TEXT NOT NULL,
                memory_ref       TEXT,
                next_run         TIMESTAMPTZ NOT NULL,
                created_at       TIMESTAMPTZ NOT NULL,
                updated_at       TIMESTAMPTZ NOT NULL,
                started_at       TIMESTAMPTZ,
                plan             JSONB,
                metadata         JSONB NOT NULL DEFAULT '{}',
                parent_agent_id  TEXT,
                pending_children JSONB NOT NULL DEFAULT '[]'
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS agents_next_run ON agents (next_run) WHERE status IN ('pending', 'waiting')")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agents_tenant ON agents (tenant_id, status)")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agents_parent ON agents (parent_agent_id) WHERE parent_agent_id IS NOT NULL")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agents_delegating ON agents (status) WHERE status = 'delegating'")
            .execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS goals (
                id          TEXT PRIMARY KEY,
                tenant_id   TEXT NOT NULL,
                description TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'open',
                agent_ids   JSONB NOT NULL DEFAULT '[]',
                created_at  TIMESTAMPTZ NOT NULL,
                updated_at  TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS goals_tenant ON goals (tenant_id)")
            .execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS workspaces (
                id           TEXT PRIMARY KEY,
                tenant_id    TEXT NOT NULL,
                agent_id     TEXT NOT NULL,
                mode         TEXT NOT NULL DEFAULT 'hybrid',
                local_path   TEXT,
                storage_key  TEXT,
                created_at   TIMESTAMPTZ NOT NULL,
                archived     BOOLEAN NOT NULL DEFAULT FALSE
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS workspaces_tenant ON workspaces (tenant_id)")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS workspaces_created ON workspaces (created_at) WHERE archived = FALSE")
            .execute(&self.pool).await?;

        Ok(())
    }

    // ── Agent CRUD ──────────────────────────────────────────────────────────

    pub async fn upsert_agent(&self, state: &AgentState) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agents (
                id, tenant_id, goal, status, current_task, current_step,
                workspace_path, memory_ref, next_run, created_at, updated_at,
                started_at, plan, metadata, parent_agent_id, pending_children
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
            ON CONFLICT (id) DO UPDATE SET
                goal             = EXCLUDED.goal,
                status           = EXCLUDED.status,
                current_task     = EXCLUDED.current_task,
                current_step     = EXCLUDED.current_step,
                workspace_path   = EXCLUDED.workspace_path,
                memory_ref       = EXCLUDED.memory_ref,
                next_run         = EXCLUDED.next_run,
                updated_at       = EXCLUDED.updated_at,
                started_at       = COALESCE(agents.started_at, EXCLUDED.started_at),
                plan             = EXCLUDED.plan,
                metadata         = EXCLUDED.metadata,
                pending_children = EXCLUDED.pending_children
        "#,
        )
        .bind(&state.id)
        .bind(&state.tenant_id)
        .bind(&state.goal)
        .bind(status_to_str(&state.status))
        .bind(&state.current_task)
        .bind(state.current_step as i32)
        .bind(&state.workspace_path)
        .bind(&state.memory_ref)
        .bind(state.next_run)
        .bind(state.created_at)
        .bind(state.updated_at)
        .bind(state.started_at)
        .bind(state.plan.as_ref().and_then(|p| serde_json::to_value(p).ok()))
        .bind(&state.metadata)
        .bind(&state.parent_agent_id)
        .bind(serde_json::json!(state.pending_children))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_agent(&self, tenant_id: &str, id: &str) -> Result<Option<AgentState>> {
        let row = sqlx::query("SELECT * FROM agents WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| row_to_agent_state(&r)))
    }

    pub async fn get_agent_internal(&self, id: &str) -> Result<Option<AgentState>> {
        let row = sqlx::query("SELECT * FROM agents WHERE id = $1").bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| row_to_agent_state(&r)))
    }

    pub async fn list_agents(&self, tenant_id: &str) -> Result<Vec<AgentState>> {
        let rows = sqlx::query("SELECT * FROM agents WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT 100")
            .bind(tenant_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|r| row_to_agent_state(r)).collect())
    }

    /// Claim due agents (pending/waiting) using FOR UPDATE SKIP LOCKED.
    pub async fn claim_due_agents(&self, limit: i64) -> Result<Vec<AgentState>> {
        let rows = sqlx::query(
            r#"
            UPDATE agents
            SET    status = 'running', updated_at = NOW()
            WHERE  id IN (
                SELECT id FROM agents
                WHERE  next_run <= NOW()
                  AND  status   IN ('pending', 'waiting')
                ORDER  BY next_run ASC
                LIMIT  $1
                FOR UPDATE SKIP LOCKED
            )
            RETURNING *
        "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| row_to_agent_state(r)).collect())
    }

    /// Check delegating agents whose all children have finished.
    /// Returns agents that should be woken up.
    pub async fn resolve_delegating_agents(&self, limit: i64) -> Result<Vec<AgentState>> {
        // Find delegating agents where every child is completed or failed
        let rows = sqlx::query(
            r#"
            SELECT a.*
            FROM   agents a
            WHERE  a.status = 'delegating'
              AND  jsonb_array_length(a.pending_children) > 0
              AND  NOT EXISTS (
                  SELECT 1
                  FROM   agents child
                  WHERE  child.id = ANY(
                      SELECT jsonb_array_elements_text(a.pending_children)
                  )
                  AND child.status NOT IN ('completed', 'failed')
              )
            LIMIT $1
        "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| row_to_agent_state(r)).collect())
    }

    pub async fn count_active_agents(&self, tenant_id: &str) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM agents
             WHERE tenant_id = $1 AND status IN ('pending','waiting','running','clarifying','delegating')",
        )
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("cnt"))
    }

    // ── Goal CRUD ───────────────────────────────────────────────────────────

    pub async fn upsert_goal(&self, goal: &GoalState) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO goals (id, tenant_id, description, status, agent_ids, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7)
            ON CONFLICT (id) DO UPDATE SET
                description = EXCLUDED.description,
                status      = EXCLUDED.status,
                agent_ids   = EXCLUDED.agent_ids,
                updated_at  = EXCLUDED.updated_at
        "#,
        )
        .bind(&goal.id)
        .bind(&goal.tenant_id)
        .bind(&goal.description)
        .bind(goal_status_to_str(&goal.status))
        .bind(serde_json::json!(goal.agent_ids))
        .bind(goal.created_at)
        .bind(goal.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Workspace CRUD ──────────────────────────────────────────────────────

    pub async fn upsert_workspace(&self, ws: &WorkspaceInfo) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO workspaces (id, tenant_id, agent_id, mode, local_path, storage_key, created_at, archived)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            ON CONFLICT (id) DO UPDATE SET
                mode        = EXCLUDED.mode,
                local_path  = EXCLUDED.local_path,
                storage_key = EXCLUDED.storage_key,
                archived    = EXCLUDED.archived
        "#,
        )
        .bind(&ws.id)
        .bind(&ws.tenant_id)
        .bind(&ws.agent_id)
        .bind(mode_to_str(&ws.mode))
        .bind(&ws.local_path)
        .bind(&ws.storage_key)
        .bind(ws.created_at)
        .bind(ws.archived)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_workspace_archived(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE workspaces SET archived = TRUE WHERE id = $1").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_workspaces_older_than(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<WorkspaceInfo>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, agent_id, mode, local_path, storage_key, created_at, archived
             FROM workspaces WHERE created_at < $1 ORDER BY created_at ASC LIMIT 500",
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| WorkspaceInfo {
                id: r.get("id"),
                tenant_id: r.get("tenant_id"),
                agent_id: r.get("agent_id"),
                mode: str_to_workspace_mode(&r.get::<String, _>("mode")),
                local_path: r.get("local_path"),
                storage_key: r.get("storage_key"),
                created_at: r.get("created_at"),
                archived: r.get("archived"),
            })
            .collect())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn status_to_str(s: &AgentStatus) -> &'static str {
    match s {
        AgentStatus::Pending => "pending",
        AgentStatus::Preflight => "preflight",
        AgentStatus::Clarifying => "clarifying",
        AgentStatus::Running => "running",
        AgentStatus::Waiting => "waiting",
        AgentStatus::Delegating => "delegating",
        AgentStatus::Completed => "completed",
        AgentStatus::Failed => "failed",
        AgentStatus::Paused => "paused",
    }
}

fn str_to_status(s: &str) -> AgentStatus {
    match s {
        "preflight" => AgentStatus::Preflight,
        "clarifying" => AgentStatus::Clarifying,
        "running" => AgentStatus::Running,
        "waiting" => AgentStatus::Waiting,
        "delegating" => AgentStatus::Delegating,
        "completed" => AgentStatus::Completed,
        "failed" => AgentStatus::Failed,
        "paused" => AgentStatus::Paused,
        _ => AgentStatus::Pending,
    }
}

fn goal_status_to_str(s: &GoalStatus) -> &'static str {
    match s {
        GoalStatus::Open => "open",
        GoalStatus::InProgress => "in_progress",
        GoalStatus::Completed => "completed",
        GoalStatus::Failed => "failed",
    }
}

fn row_to_agent_state(row: &PgRow) -> AgentState {
    let pending_children: Vec<String> = row
        .try_get::<serde_json::Value, _>("pending_children")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let plan = row
        .try_get::<Option<serde_json::Value>, _>("plan")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok());

    AgentState {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        goal: row.get("goal"),
        status: str_to_status(&row.get::<String, _>("status")),
        current_task: row.get("current_task"),
        current_step: row.get::<i32, _>("current_step") as u32,
        workspace_path: row.get("workspace_path"),
        memory_ref: row.get("memory_ref"),
        next_run: row.get::<DateTime<Utc>, _>("next_run"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
        started_at: row.try_get::<Option<DateTime<Utc>>, _>("started_at").ok().flatten(),
        plan,
        metadata: row.get("metadata"),
        parent_agent_id: row.get("parent_agent_id"),
        pending_children,
    }
}

fn mode_to_str(m: &WorkspaceMode) -> &'static str {
    match m {
        WorkspaceMode::Local => "local",
        WorkspaceMode::Remote => "remote",
        WorkspaceMode::Hybrid => "hybrid",
    }
}

fn str_to_workspace_mode(s: &str) -> WorkspaceMode {
    match s {
        "remote" => WorkspaceMode::Remote,
        "local" => WorkspaceMode::Local,
        _ => WorkspaceMode::Hybrid,
    }
}
