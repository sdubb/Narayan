use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Row};

use serde::Serialize;

use crate::{
    agent::definition::{
        AgentDefinition, AgentDefinitionStatus, AgentRole, ConnectorAuthType, ConnectorSource, EndpointDef,
        ExecutionLimits, MemoryScope, OutputSpec, RoleStatus, TenantConnector, TenantWasmTool, TriggerDef,
        WasmToolPermissions, WasmToolResourceLimits, WasmToolRunAudit, WorkforceEventSubscription,
    },
    state::{AgentState, AgentStatus, GoalInstance, GoalInstanceStatus, GoalState, GoalStatus, TriggerSource},
    workspace::{manager::WorkspaceInfo, resolver::WorkspaceMode},
};

#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub id: String,
    pub tenant_id: String,
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

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
        let _ = sqlx::query("CREATE EXTENSION IF NOT EXISTS vector").execute(&self.pool).await;

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
                final_answer     TEXT,
                metadata         JSONB NOT NULL DEFAULT '{}',
                parent_agent_id  TEXT,
                pending_children JSONB NOT NULL DEFAULT '[]'
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE agents ADD COLUMN IF NOT EXISTS final_answer TEXT").execute(&self.pool).await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS agents_next_run ON agents (next_run) WHERE status IN ('pending', 'waiting')",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agents_tenant ON agents (tenant_id, status)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS agents_parent ON agents (parent_agent_id) WHERE parent_agent_id IS NOT NULL",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agents_delegating ON agents (status) WHERE status = 'delegating'")
            .execute(&self.pool)
            .await?;

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
        sqlx::query("CREATE INDEX IF NOT EXISTS goals_tenant ON goals (tenant_id)").execute(&self.pool).await?;

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
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS workspaces_created ON workspaces (created_at) WHERE archived = FALSE")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS costs (
                id                TEXT PRIMARY KEY,
                tenant_id         TEXT NOT NULL,
                agent_id          TEXT,
                model             TEXT,
                input_tokens      BIGINT NOT NULL DEFAULT 0,
                output_tokens     BIGINT NOT NULL DEFAULT 0,
                total_cost_usd    DOUBLE PRECISION NOT NULL DEFAULT 0,
                period_start      TIMESTAMPTZ NOT NULL DEFAULT date_trunc('month', NOW()),
                created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS costs_tenant_period ON costs (tenant_id, period_start)")
            .execute(&self.pool)
            .await?;

        // ── Conversations ────────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS conversations (
                id         TEXT PRIMARY KEY,
                tenant_id  TEXT NOT NULL,
                title      TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS conversations_tenant ON conversations (tenant_id, updated_at DESC)")
            .execute(&self.pool)
            .await?;

        // Add conversation_id column to agents table
        sqlx::query("ALTER TABLE agents ADD COLUMN IF NOT EXISTS conversation_id TEXT").execute(&self.pool).await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS agents_conversation ON agents (conversation_id) WHERE conversation_id IS NOT NULL",
        )
        .execute(&self.pool)
        .await?;

        // Add plan_rejection_count — survives server restarts so the 3-rejection cap always works.
        sqlx::query("ALTER TABLE agents ADD COLUMN IF NOT EXISTS plan_rejection_count INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await?;

        // ── AgentDefinitions ─────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS agent_definitions (
                id          TEXT PRIMARY KEY,
                tenant_id   TEXT NOT NULL,
                name        TEXT NOT NULL,
                persona     TEXT NOT NULL DEFAULT '',
                connectors  JSONB NOT NULL DEFAULT '[]',
                constraints JSONB NOT NULL DEFAULT '[]',
                memory_ref  TEXT NOT NULL DEFAULT '',
                status      TEXT NOT NULL DEFAULT 'draft',
                created_at  TIMESTAMPTZ NOT NULL,
                updated_at  TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agent_defs_tenant ON agent_definitions (tenant_id, status)")
            .execute(&self.pool)
            .await?;

        // ── AgentRoles ───────────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS agent_roles (
                id                   TEXT PRIMARY KEY,
                agent_id             TEXT NOT NULL REFERENCES agent_definitions(id) ON DELETE CASCADE,
                tenant_id            TEXT NOT NULL,
                version              INTEGER NOT NULL DEFAULT 1,
                status               TEXT NOT NULL DEFAULT 'draft',
                name                 TEXT NOT NULL,
                trigger              JSONB NOT NULL DEFAULT '{}',
                purpose              TEXT NOT NULL DEFAULT '',
                role_category        TEXT NOT NULL DEFAULT 'general',
                execution_guidelines TEXT,
                connectors           JSONB NOT NULL DEFAULT '[]',
                tools                JSONB NOT NULL DEFAULT '[]',
                output_spec          JSONB NOT NULL DEFAULT '{}',
                memory_scope         TEXT NOT NULL DEFAULT 'agent',
                execution_limits     JSONB NOT NULL DEFAULT '{}',
                created_at           TIMESTAMPTZ NOT NULL,
                updated_at           TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agent_roles_agent ON agent_roles (agent_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS agent_roles_tenant ON agent_roles (tenant_id, status)")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE agent_roles ADD COLUMN IF NOT EXISTS role_category TEXT NOT NULL DEFAULT 'general'")
            .execute(&self.pool)
            .await?;

        // ── GoalInstances ────────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS goal_instances (
                id                              TEXT PRIMARY KEY,
                tenant_id                       TEXT NOT NULL,
                agent_id                        TEXT NOT NULL,
                role_id                         TEXT NOT NULL,
                role_version                    INTEGER NOT NULL,
                input_data                      JSONB NOT NULL DEFAULT '{}',
                status                          TEXT NOT NULL DEFAULT 'pending',
                result                          JSONB,
                failure_reason                  TEXT,
                trigger_source                  JSONB NOT NULL DEFAULT '{}',
                is_test                         BOOLEAN NOT NULL DEFAULT FALSE,
                cost_usd                        DOUBLE PRECISION NOT NULL DEFAULT 0,
                human_hours_saved               DOUBLE PRECISION NOT NULL DEFAULT 0,
                human_cost_saved_usd            DOUBLE PRECISION NOT NULL DEFAULT 0,
                agent_state_id                  TEXT,
                triggered_by_goal_instance_id   TEXT,
                created_at                      TIMESTAMPTZ NOT NULL,
                updated_at                      TIMESTAMPTZ NOT NULL,
                completed_at                    TIMESTAMPTZ
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS goal_inst_tenant ON goal_instances (tenant_id, status)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS goal_inst_agent ON goal_instances (agent_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS goal_inst_role ON goal_instances (role_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS goal_inst_pending ON goal_instances (created_at)
             WHERE status = 'pending'",
        )
        .execute(&self.pool)
        .await?;

        // ── TenantConnectors ─────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_connectors (
                id                   TEXT PRIMARY KEY,
                tenant_id            TEXT NOT NULL,
                name                 TEXT NOT NULL,
                category             TEXT NOT NULL,
                base_url             TEXT NOT NULL,
                auth_type            JSONB NOT NULL DEFAULT '{}',
                auth_credential_key  TEXT,
                source               JSONB NOT NULL DEFAULT '{}',
                source_docs          TEXT,
                endpoints            JSONB NOT NULL DEFAULT '[]',
                summary              TEXT NOT NULL DEFAULT '',
                created_at           TIMESTAMPTZ NOT NULL,
                updated_at           TIMESTAMPTZ NOT NULL,
                UNIQUE (tenant_id, name)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS tenant_conn_tenant ON tenant_connectors (tenant_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS tenant_conn_category ON tenant_connectors (tenant_id, category)")
            .execute(&self.pool)
            .await?;

        // ── TenantWasmTools ──────────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_wasm_tools (
                id                 TEXT PRIMARY KEY,
                tenant_id          TEXT NOT NULL,
                name               TEXT NOT NULL,
                description        TEXT NOT NULL DEFAULT '',
                module_bytes       BYTEA NOT NULL,
                module_sha256      TEXT NOT NULL,
                module_size_bytes  BIGINT NOT NULL DEFAULT 0,
                exports            JSONB NOT NULL DEFAULT '[]',
                permissions        JSONB NOT NULL DEFAULT '{}',
                limits             JSONB NOT NULL DEFAULT '{}',
                enabled            BOOLEAN NOT NULL DEFAULT TRUE,
                version            INTEGER NOT NULL DEFAULT 1,
                last_used_at       TIMESTAMPTZ,
                created_at         TIMESTAMPTZ NOT NULL,
                updated_at         TIMESTAMPTZ NOT NULL,
                UNIQUE (tenant_id, name)
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS tenant_wasm_tools_tenant ON tenant_wasm_tools (tenant_id, name)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS tenant_wasm_tools_enabled ON tenant_wasm_tools (tenant_id, enabled)")
            .execute(&self.pool)
            .await?;

        // ── Wasm tool run audit ──────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS wasm_tool_runs (
                id                 TEXT PRIMARY KEY,
                tenant_id          TEXT NOT NULL,
                tool_name          TEXT NOT NULL,
                tool_version       INTEGER NOT NULL DEFAULT 1,
                agent_id           TEXT,
                role_id            TEXT,
                goal_instance_id   TEXT,
                success            BOOLEAN NOT NULL,
                elapsed_ms         BIGINT NOT NULL DEFAULT 0,
                fuel_used          BIGINT,
                memory_limit_bytes BIGINT NOT NULL DEFAULT 0,
                error              TEXT,
                created_at         TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS wasm_tool_runs_tenant ON wasm_tool_runs (tenant_id, created_at DESC)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS wasm_tool_runs_tool ON wasm_tool_runs (tenant_id, tool_name, created_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        // ── WorkforceEventSubscriptions ──────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS workforce_event_subscriptions (
                id                   TEXT PRIMARY KEY,
                tenant_id            TEXT NOT NULL,
                subscriber_role_id   TEXT NOT NULL,
                subscriber_agent_id  TEXT NOT NULL,
                event_filter         TEXT NOT NULL,
                input_mapping        JSONB NOT NULL DEFAULT '{}',
                active               BOOLEAN NOT NULL DEFAULT TRUE,
                created_at           TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS wf_subs_tenant ON workforce_event_subscriptions (tenant_id) WHERE active = TRUE",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS wf_subs_role ON workforce_event_subscriptions (subscriber_role_id)")
            .execute(&self.pool)
            .await?;

        // ── PlanModeSessions ─────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS plan_mode_sessions (
                id             TEXT PRIMARY KEY,
                tenant_id      TEXT NOT NULL,
                agent_id       TEXT NOT NULL,
                goal_fingerprint TEXT NOT NULL DEFAULT '',
                repair_version INTEGER NOT NULL DEFAULT 1,
                reused_from_session_id TEXT,
                repair_root_session_id TEXT,
                phase          TEXT NOT NULL,
                conversation   JSONB NOT NULL DEFAULT '[]',
                attachments    JSONB NOT NULL DEFAULT '[]',
                attachment_context TEXT NOT NULL DEFAULT '',
                session_workspace  TEXT,
                draft_role     JSONB,
                intent_cache   JSONB,
                pending_steps  JSONB NOT NULL DEFAULT '[]',
                created_at     TIMESTAMPTZ NOT NULL,
                updated_at     TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE plan_mode_sessions ADD COLUMN IF NOT EXISTS attachments JSONB NOT NULL DEFAULT '[]'")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "ALTER TABLE plan_mode_sessions ADD COLUMN IF NOT EXISTS goal_fingerprint TEXT NOT NULL DEFAULT ''",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "ALTER TABLE plan_mode_sessions ADD COLUMN IF NOT EXISTS repair_version INTEGER NOT NULL DEFAULT 1",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE plan_mode_sessions ADD COLUMN IF NOT EXISTS reused_from_session_id TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE plan_mode_sessions ADD COLUMN IF NOT EXISTS repair_root_session_id TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "ALTER TABLE plan_mode_sessions ADD COLUMN IF NOT EXISTS attachment_context TEXT NOT NULL DEFAULT ''",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE plan_mode_sessions ADD COLUMN IF NOT EXISTS session_workspace TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS plan_mode_sessions_tenant ON plan_mode_sessions (tenant_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS plan_mode_sessions_goal_fingerprint
             ON plan_mode_sessions (tenant_id, goal_fingerprint, repair_version DESC, updated_at DESC)",
        )
        .execute(&self.pool)
        .await?;

        // ── RoleChatSessions ─────────────────────────────────────────────
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS role_chat_sessions (
                id              TEXT PRIMARY KEY,
                tenant_id       TEXT NOT NULL,
                role_id         TEXT NOT NULL,
                agent_id        TEXT NOT NULL,
                conversation    JSONB NOT NULL DEFAULT '[]',
                pending_change  JSONB,
                created_at      TIMESTAMPTZ NOT NULL,
                updated_at      TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS role_chat_sessions_role ON role_chat_sessions (tenant_id, role_id)")
            .execute(&self.pool)
            .await?;

        // ── ScheduleTicker: next_run_at for cron-based roles ────────────
        sqlx::query("ALTER TABLE agent_roles ADD COLUMN IF NOT EXISTS next_run_at TIMESTAMPTZ")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_roles_next_run
             ON agent_roles (next_run_at)
             WHERE status = 'active' AND trigger->>'trigger_type' = 'schedule'",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ── Agent CRUD ──────────────────────────────────────────────────────────

    pub async fn upsert_agent(&self, state: &AgentState) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agents (
                id, tenant_id, goal, status, current_task, current_step,
                workspace_path, memory_ref, next_run, created_at, updated_at,
                started_at, plan, final_answer, metadata, parent_agent_id, pending_children,
                conversation_id, plan_rejection_count
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
            ON CONFLICT (id) DO UPDATE SET
                goal                 = EXCLUDED.goal,
                status               = EXCLUDED.status,
                current_task         = EXCLUDED.current_task,
                current_step         = EXCLUDED.current_step,
                workspace_path       = EXCLUDED.workspace_path,
                memory_ref           = EXCLUDED.memory_ref,
                next_run             = EXCLUDED.next_run,
                updated_at           = EXCLUDED.updated_at,
                started_at           = COALESCE(agents.started_at, EXCLUDED.started_at),
                plan                 = EXCLUDED.plan,
                final_answer         = EXCLUDED.final_answer,
                metadata             = EXCLUDED.metadata,
                pending_children     = EXCLUDED.pending_children,
                conversation_id      = EXCLUDED.conversation_id,
                plan_rejection_count = EXCLUDED.plan_rejection_count
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
        .bind(state.final_answer.as_deref())
        .bind(&state.metadata)
        .bind(&state.parent_agent_id)
        .bind(serde_json::json!(state.pending_children))
        .bind(&state.conversation_id)
        .bind(state.plan_rejection_count as i32)
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
                  -- NOTE: plan_approval_needed, clarifying, paused, and delegating
                  -- are intentionally excluded from scheduling.  These statuses
                  -- require human input or external completion before the agent
                  -- can proceed.  Do NOT add them to this IN clause.
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

    /// Get all child agents for a given parent agent.
    pub async fn get_agent_children(&self, tenant_id: &str, parent_id: &str) -> Result<Vec<AgentState>> {
        let rows =
            sqlx::query("SELECT * FROM agents WHERE parent_agent_id = $1 AND tenant_id = $2 ORDER BY created_at ASC")
                .bind(parent_id)
                .bind(tenant_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.iter().map(|r| row_to_agent_state(r)).collect())
    }

    /// Update agent plan in the database.
    pub async fn update_agent_plan(&self, tenant_id: &str, agent_id: &str, plan: &serde_json::Value) -> Result<()> {
        sqlx::query("UPDATE agents SET plan = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(plan)
            .bind(agent_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update agent metadata in the database.
    pub async fn update_agent_metadata(
        &self,
        tenant_id: &str,
        agent_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query("UPDATE agents SET metadata = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(metadata)
            .bind(agent_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update agent status.
    pub async fn update_agent_status(&self, tenant_id: &str, agent_id: &str, status: &str) -> Result<()> {
        sqlx::query("UPDATE agents SET status = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3")
            .bind(status)
            .bind(agent_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
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

    // ── Conversation CRUD ──────────────────────────────────────────────────

    pub async fn create_conversation(&self, id: &str, tenant_id: &str, title: Option<&str>) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO conversations (id, tenant_id, title, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(tenant_id)
        .bind(title)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_conversation(&self, tenant_id: &str, id: &str) -> Result<Option<Conversation>> {
        let row = sqlx::query("SELECT * FROM conversations WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| Conversation {
            id: r.get("id"),
            tenant_id: r.get("tenant_id"),
            title: r.get("title"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    pub async fn list_conversations(&self, tenant_id: &str) -> Result<Vec<Conversation>> {
        let rows = sqlx::query("SELECT * FROM conversations WHERE tenant_id = $1 ORDER BY updated_at DESC LIMIT 100")
            .bind(tenant_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| Conversation {
                id: r.get("id"),
                tenant_id: r.get("tenant_id"),
                title: r.get("title"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    pub async fn touch_conversation(&self, id: &str) -> Result<()> {
        sqlx::query("UPDATE conversations SET updated_at = NOW() WHERE id = $1").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_agents_in_conversation(&self, tenant_id: &str, conversation_id: &str) -> Result<Vec<AgentState>> {
        let rows = sqlx::query(
            "SELECT * FROM agents WHERE tenant_id = $1 AND conversation_id = $2
             ORDER BY created_at ASC",
        )
        .bind(tenant_id)
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| row_to_agent_state(r)).collect())
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

    // ── RoleChatSession CRUD ────────────────────────────────────────────────

    pub async fn upsert_role_chat_session(&self, session: &crate::agent::role_chat::RoleChatSession) -> Result<()> {
        let conversation = serde_json::to_value(&session.conversation).unwrap_or(serde_json::json!([]));
        let pending_change = session.pending_change.as_ref().and_then(|c| serde_json::to_value(c).ok());

        sqlx::query(
            r#"INSERT INTO role_chat_sessions
                (id, tenant_id, role_id, agent_id, conversation, pending_change, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
               ON CONFLICT (id) DO UPDATE SET
                conversation   = EXCLUDED.conversation,
                pending_change = EXCLUDED.pending_change,
                updated_at     = EXCLUDED.updated_at"#,
        )
        .bind(&session.id)
        .bind(&session.tenant_id)
        .bind(&session.role_id)
        .bind(&session.agent_id)
        .bind(&conversation)
        .bind(&pending_change)
        .bind(session.created_at)
        .bind(session.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_role_chat_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<Option<crate::agent::role_chat::RoleChatSession>> {
        let row = sqlx::query("SELECT * FROM role_chat_sessions WHERE id=$1 AND tenant_id=$2")
            .bind(session_id)
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            None => Ok(None),
            Some(r) => {
                let conversation: Vec<crate::agent::role_chat::RoleChatMessage> = {
                    let v: serde_json::Value = r.get("conversation");
                    serde_json::from_value(v).unwrap_or_default()
                };
                let pending_change = r
                    .try_get::<Option<serde_json::Value>, _>("pending_change")
                    .ok()
                    .flatten()
                    .and_then(|v| serde_json::from_value(v).ok());
                Ok(Some(crate::agent::role_chat::RoleChatSession {
                    id: r.get("id"),
                    tenant_id: r.get("tenant_id"),
                    role_id: r.get("role_id"),
                    agent_id: r.get("agent_id"),
                    conversation,
                    pending_change,
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                }))
            }
        }
    }

    pub async fn delete_role_chat_session(&self, tenant_id: &str, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM role_chat_sessions WHERE id=$1 AND tenant_id=$2")
            .bind(session_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_plan_mode_session(&self, session: &crate::agent::definition::PlanModeSession) -> Result<()> {
        let phase = serde_json::to_value(&session.phase).unwrap_or(serde_json::json!("capturing_intent"));
        let conversation = serde_json::to_value(&session.conversation).unwrap_or(serde_json::json!([]));
        let attachments = serde_json::to_value(&session.attachments).unwrap_or(serde_json::json!([]));
        let draft_role = session.draft_role.as_ref().and_then(|r| serde_json::to_value(r).ok());
        let intent_cache = session.intent_cache.as_ref().and_then(|v| serde_json::to_value(v).ok());

        sqlx::query(
            r#"
            INSERT INTO plan_mode_sessions
                (id, tenant_id, agent_id, goal_fingerprint, repair_version, reused_from_session_id, repair_root_session_id, phase, conversation, attachments, attachment_context, session_workspace, draft_role, intent_cache, pending_steps, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
            ON CONFLICT (id) DO UPDATE SET
                goal_fingerprint = EXCLUDED.goal_fingerprint,
                repair_version   = EXCLUDED.repair_version,
                reused_from_session_id = EXCLUDED.reused_from_session_id,
                repair_root_session_id = EXCLUDED.repair_root_session_id,
                phase         = EXCLUDED.phase,
                conversation  = EXCLUDED.conversation,
                attachments   = EXCLUDED.attachments,
                attachment_context = EXCLUDED.attachment_context,
                session_workspace  = EXCLUDED.session_workspace,
                draft_role    = EXCLUDED.draft_role,
                intent_cache  = EXCLUDED.intent_cache,
                pending_steps = EXCLUDED.pending_steps,
                updated_at    = EXCLUDED.updated_at
            "#,
        )
        .bind(&session.id)
        .bind(&session.tenant_id)
        .bind(&session.draft_agent.id)
        .bind(session.goal_fingerprint.as_deref().unwrap_or(""))
        .bind(session.repair_version as i32)
        .bind(&session.reused_from_session_id)
        .bind(&session.repair_root_session_id)
        .bind(phase.as_str().unwrap_or("capturing_intent"))
        .bind(&conversation)
        .bind(&attachments)
        .bind(&session.attachment_context)
        .bind(&session.session_workspace)
        .bind(&draft_role)
        .bind(&intent_cache)
        .bind(serde_json::to_value(&session.pending_steps).unwrap_or(serde_json::json!([])))
        .bind(session.created_at)
        .bind(session.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_plan_mode_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<Option<crate::agent::definition::PlanModeSession>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, agent_id, goal_fingerprint, repair_version, reused_from_session_id, repair_root_session_id, phase, conversation, attachments, attachment_context, session_workspace, draft_role, intent_cache, pending_steps, created_at, updated_at
             FROM plan_mode_sessions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(session_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            None => Ok(None),
            Some(r) => {
                let agent_id: String = r.get("agent_id");
                let draft_agent = self.get_agent_definition(tenant_id, &agent_id).await?.unwrap_or_else(|| {
                    crate::agent::definition::AgentDefinition::new(
                        agent_id.clone(),
                        tenant_id.to_string(),
                        "Agent".into(),
                    )
                });

                let phase: crate::agent::definition::PlanModePhase = {
                    let s: String = r.get("phase");
                    serde_json::from_value(serde_json::json!(s))
                        .unwrap_or(crate::agent::definition::PlanModePhase::CapturingIntent)
                };
                let conversation: Vec<crate::agent::definition::PlanModeMessage> = {
                    let v: serde_json::Value = r.get("conversation");
                    serde_json::from_value(v).unwrap_or_default()
                };
                let goal_fingerprint: Option<String> = {
                    let value: String = r.get("goal_fingerprint");
                    if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    }
                };
                let repair_version: u32 = r.try_get::<i32, _>("repair_version").unwrap_or(1).max(1) as u32;
                let reused_from_session_id: Option<String> =
                    r.try_get::<Option<String>, _>("reused_from_session_id").ok().flatten();
                let repair_root_session_id: Option<String> =
                    r.try_get::<Option<String>, _>("repair_root_session_id").ok().flatten();
                let attachments: Vec<crate::agent::definition::PlanModeAttachment> = {
                    let v: serde_json::Value = r.get("attachments");
                    serde_json::from_value(v).unwrap_or_default()
                };
                let attachment_context: String = r.get("attachment_context");
                let session_workspace: Option<String> = r.get("session_workspace");
                let draft_role: Option<crate::agent::definition::AgentRole> = r
                    .try_get::<Option<serde_json::Value>, _>("draft_role")
                    .ok()
                    .flatten()
                    .and_then(|v| serde_json::from_value(v).ok());
                let intent_cache: Option<serde_json::Value> =
                    r.try_get::<Option<serde_json::Value>, _>("intent_cache").ok().flatten();
                let pending_steps: Vec<serde_json::Value> = r
                    .try_get::<serde_json::Value, _>("pending_steps")
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();

                Ok(Some(crate::agent::definition::PlanModeSession {
                    id: r.get("id"),
                    tenant_id: r.get("tenant_id"),
                    draft_agent,
                    draft_role,
                    conversation,
                    attachments,
                    attachment_context,
                    session_workspace,
                    goal_fingerprint,
                    repair_version,
                    reused_from_session_id,
                    repair_root_session_id,
                    phase,
                    intent_cache,
                    pending_steps,
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                }))
            }
        }
    }

    pub async fn get_latest_plan_mode_session_by_goal_fingerprint(
        &self,
        tenant_id: &str,
        goal_fingerprint: &str,
    ) -> Result<Option<crate::agent::definition::PlanModeSession>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, agent_id, goal_fingerprint, repair_version, reused_from_session_id, repair_root_session_id, phase, conversation, attachments, attachment_context, session_workspace, draft_role, intent_cache, pending_steps, created_at, updated_at
             FROM plan_mode_sessions
             WHERE tenant_id = $1 AND goal_fingerprint = $2 AND goal_fingerprint <> '' AND draft_role IS NOT NULL
             ORDER BY repair_version DESC, updated_at DESC
             LIMIT 1",
        )
        .bind(tenant_id)
        .bind(goal_fingerprint)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            None => Ok(None),
            Some(r) => {
                let agent_id: String = r.get("agent_id");
                let draft_agent = self.get_agent_definition(tenant_id, &agent_id).await?.unwrap_or_else(|| {
                    crate::agent::definition::AgentDefinition::new(
                        agent_id.clone(),
                        tenant_id.to_string(),
                        "Agent".into(),
                    )
                });

                let phase: crate::agent::definition::PlanModePhase = {
                    let s: String = r.get("phase");
                    serde_json::from_value(serde_json::json!(s))
                        .unwrap_or(crate::agent::definition::PlanModePhase::CapturingIntent)
                };
                let conversation: Vec<crate::agent::definition::PlanModeMessage> = {
                    let v: serde_json::Value = r.get("conversation");
                    serde_json::from_value(v).unwrap_or_default()
                };
                let attachments: Vec<crate::agent::definition::PlanModeAttachment> = {
                    let v: serde_json::Value = r.get("attachments");
                    serde_json::from_value(v).unwrap_or_default()
                };
                let attachment_context: String = r.get("attachment_context");
                let session_workspace: Option<String> = r.get("session_workspace");
                let draft_role: Option<crate::agent::definition::AgentRole> = r
                    .try_get::<Option<serde_json::Value>, _>("draft_role")
                    .ok()
                    .flatten()
                    .and_then(|v| serde_json::from_value(v).ok());
                let intent_cache: Option<serde_json::Value> =
                    r.try_get::<Option<serde_json::Value>, _>("intent_cache").ok().flatten();
                let pending_steps: Vec<serde_json::Value> = r
                    .try_get::<serde_json::Value, _>("pending_steps")
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let goal_fingerprint: Option<String> = {
                    let value: String = r.get("goal_fingerprint");
                    if value.trim().is_empty() {
                        None
                    } else {
                        Some(value)
                    }
                };
                let repair_version: u32 = r.try_get::<i32, _>("repair_version").unwrap_or(1).max(1) as u32;
                let reused_from_session_id: Option<String> =
                    r.try_get::<Option<String>, _>("reused_from_session_id").ok().flatten();
                let repair_root_session_id: Option<String> =
                    r.try_get::<Option<String>, _>("repair_root_session_id").ok().flatten();

                Ok(Some(crate::agent::definition::PlanModeSession {
                    id: r.get("id"),
                    tenant_id: r.get("tenant_id"),
                    draft_agent,
                    draft_role,
                    conversation,
                    attachments,
                    attachment_context,
                    session_workspace,
                    goal_fingerprint,
                    repair_version,
                    reused_from_session_id,
                    repair_root_session_id,
                    phase,
                    intent_cache,
                    pending_steps,
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                }))
            }
        }
    }

    pub async fn delete_plan_mode_session(&self, tenant_id: &str, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM plan_mode_sessions WHERE id = $1 AND tenant_id = $2")
            .bind(session_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── AgentDefinition CRUD ────────────────────────────────────────────────

    pub async fn upsert_agent_definition(&self, def: &AgentDefinition) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_definitions
                (id, tenant_id, name, persona, connectors, constraints, memory_ref, status, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            ON CONFLICT (id) DO UPDATE SET
                name        = EXCLUDED.name,
                persona     = EXCLUDED.persona,
                connectors  = EXCLUDED.connectors,
                constraints = EXCLUDED.constraints,
                memory_ref  = EXCLUDED.memory_ref,
                status      = EXCLUDED.status,
                updated_at  = EXCLUDED.updated_at
            "#,
        )
        .bind(&def.id)
        .bind(&def.tenant_id)
        .bind(&def.name)
        .bind(&def.persona)
        .bind(serde_json::json!(def.connectors))
        .bind(serde_json::json!(def.constraints))
        .bind(&def.memory_ref)
        .bind(agent_def_status_to_str(&def.status))
        .bind(def.created_at)
        .bind(def.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_agent_definition(&self, tenant_id: &str, id: &str) -> Result<Option<AgentDefinition>> {
        let row = sqlx::query("SELECT * FROM agent_definitions WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_to_agent_definition))
    }

    pub async fn list_agent_definitions(&self, tenant_id: &str) -> Result<Vec<AgentDefinition>> {
        let rows = sqlx::query("SELECT * FROM agent_definitions WHERE tenant_id = $1 ORDER BY created_at DESC")
            .bind(tenant_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_agent_definition).collect())
    }

    pub async fn delete_agent_definition(&self, tenant_id: &str, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM agent_definitions WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── AgentRole CRUD ──────────────────────────────────────────────────────

    pub async fn upsert_agent_role(&self, role: &AgentRole) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_roles
                (id, agent_id, tenant_id, version, status, name, trigger, purpose,
                 role_category, execution_guidelines, connectors, tools, output_spec, memory_scope,
                 execution_limits, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
            ON CONFLICT (id) DO UPDATE SET
                version              = EXCLUDED.version,
                status               = EXCLUDED.status,
                name                 = EXCLUDED.name,
                trigger              = EXCLUDED.trigger,
                purpose              = EXCLUDED.purpose,
                role_category        = EXCLUDED.role_category,
                execution_guidelines = EXCLUDED.execution_guidelines,
                connectors           = EXCLUDED.connectors,
                tools                = EXCLUDED.tools,
                output_spec          = EXCLUDED.output_spec,
                memory_scope         = EXCLUDED.memory_scope,
                execution_limits     = EXCLUDED.execution_limits,
                updated_at           = EXCLUDED.updated_at
            "#,
        )
        .bind(&role.id)
        .bind(&role.agent_id)
        .bind(&role.tenant_id)
        .bind(role.version as i32)
        .bind(role_status_to_str(&role.status))
        .bind(&role.name)
        .bind(serde_json::to_value(&role.trigger).unwrap_or_default())
        .bind(&role.purpose)
        .bind(role.role_category.as_str())
        .bind(serde_json::to_value(&role.execution_guidelines).unwrap_or_default())
        .bind(serde_json::json!(role.connectors))
        .bind(serde_json::json!(role.tools))
        .bind(serde_json::to_value(&role.output_spec).unwrap_or_default())
        .bind(memory_scope_to_str(&role.memory_scope))
        .bind(serde_json::to_value(&role.execution_limits).unwrap_or_default())
        .bind(role.created_at)
        .bind(role.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_agent_role(&self, tenant_id: &str, id: &str) -> Result<Option<AgentRole>> {
        let row = sqlx::query("SELECT * FROM agent_roles WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_to_agent_role))
    }

    pub async fn list_roles_for_agent(&self, tenant_id: &str, agent_id: &str) -> Result<Vec<AgentRole>> {
        let rows =
            sqlx::query("SELECT * FROM agent_roles WHERE agent_id = $1 AND tenant_id = $2 ORDER BY created_at ASC")
                .bind(agent_id)
                .bind(tenant_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.iter().map(row_to_agent_role).collect())
    }

    /// Returns all active roles with webhook or workforce_event triggers for a tenant.
    /// Used by the scheduler and connector poller to match incoming events.
    pub async fn list_active_trigger_roles(&self, tenant_id: &str) -> Result<Vec<AgentRole>> {
        let rows = sqlx::query(
            "SELECT * FROM agent_roles
             WHERE tenant_id = $1
               AND status = 'active'
               AND (trigger->>'trigger_type' IN ('webhook', 'workforce_event', 'schedule'))",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_agent_role).collect())
    }

    pub async fn delete_agent_role(&self, tenant_id: &str, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM agent_roles WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Schedule Ticker queries ─────────────────────────────────────────────

    /// Atomically claim scheduled roles whose cron is due.
    /// Sets `next_run_at` to a far-future sentinel so they aren't re-claimed.
    /// Caller must compute the real next_run_at and call `update_role_next_run_at`.
    pub async fn claim_due_scheduled_roles(&self, limit: i64) -> Result<Vec<AgentRole>> {
        let rows = sqlx::query(
            r#"
            UPDATE agent_roles
            SET next_run_at = '9999-01-01T00:00:00Z'::timestamptz, updated_at = NOW()
            WHERE id IN (
                SELECT id FROM agent_roles
                WHERE status = 'active'
                  AND trigger->>'trigger_type' = 'schedule'
                  AND trigger->>'cron' IS NOT NULL
                  AND (next_run_at IS NULL OR next_run_at <= NOW())
                ORDER BY next_run_at ASC NULLS FIRST
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            RETURNING *
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_agent_role).collect())
    }

    /// Set the next fire time for a scheduled role after processing.
    pub async fn update_role_next_run_at(&self, role_id: &str, next: chrono::DateTime<chrono::Utc>) -> Result<()> {
        sqlx::query("UPDATE agent_roles SET next_run_at = $1, updated_at = NOW() WHERE id = $2")
            .bind(next)
            .bind(role_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── GoalInstance CRUD ───────────────────────────────────────────────────

    pub async fn upsert_goal_instance(&self, gi: &GoalInstance) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO goal_instances
                (id, tenant_id, agent_id, role_id, role_version, input_data, status,
                 result, failure_reason, trigger_source, is_test, cost_usd,
                 agent_state_id, triggered_by_goal_instance_id,
                 created_at, updated_at, completed_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
            ON CONFLICT (id) DO UPDATE SET
                status                          = EXCLUDED.status,
                result                          = EXCLUDED.result,
                failure_reason                  = EXCLUDED.failure_reason,
                cost_usd                        = EXCLUDED.cost_usd,
                agent_state_id                  = EXCLUDED.agent_state_id,
                updated_at                      = EXCLUDED.updated_at,
                completed_at                    = EXCLUDED.completed_at
            "#,
        )
        .bind(&gi.id)
        .bind(&gi.tenant_id)
        .bind(&gi.agent_id)
        .bind(&gi.role_id)
        .bind(gi.role_version as i32)
        .bind(&gi.input_data)
        .bind(goal_instance_status_to_str(&gi.status))
        .bind(&gi.result)
        .bind(&gi.failure_reason)
        .bind(serde_json::to_value(&gi.trigger_source).unwrap_or_default())
        .bind(gi.is_test)
        .bind(gi.cost_usd)
        .bind(&gi.agent_state_id)
        .bind(&gi.triggered_by_goal_instance_id)
        .bind(gi.created_at)
        .bind(gi.updated_at)
        .bind(gi.completed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_goal_instance(&self, tenant_id: &str, id: &str) -> Result<Option<GoalInstance>> {
        let row = sqlx::query("SELECT * FROM goal_instances WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_to_goal_instance))
    }

    pub async fn list_goal_instances_for_role(
        &self,
        tenant_id: &str,
        role_id: &str,
        limit: i64,
    ) -> Result<Vec<GoalInstance>> {
        let rows = sqlx::query(
            "SELECT * FROM goal_instances
             WHERE role_id = $1 AND tenant_id = $2
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(role_id)
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_goal_instance).collect())
    }

    pub async fn list_goal_instances_for_agent(
        &self,
        tenant_id: &str,
        agent_id: &str,
        limit: i64,
    ) -> Result<Vec<GoalInstance>> {
        let rows = sqlx::query(
            "SELECT * FROM goal_instances
             WHERE agent_id = $1 AND tenant_id = $2
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(agent_id)
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_goal_instance).collect())
    }

    /// Update the result JSON on a goal instance (writes criteria_checks, step_outputs).
    pub async fn update_goal_instance_result(
        &self,
        tenant_id: &str,
        goal_instance_id: &str,
        result: serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE goal_instances SET result = $1, updated_at = NOW()
             WHERE id = $2 AND tenant_id = $3",
        )
        .bind(&result)
        .bind(goal_instance_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Write savings estimates back to a completed GoalInstance.
    pub async fn update_goal_instance_savings(
        &self,
        tenant_id: &str,
        goal_instance_id: &str,
        human_hours_saved: f64,
        human_cost_saved_usd: f64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE goal_instances SET
                human_hours_saved    = $1,
                human_cost_saved_usd = $2,
                updated_at           = NOW()
             WHERE id = $3 AND tenant_id = $4",
        )
        .bind(human_hours_saved)
        .bind(human_cost_saved_usd)
        .bind(goal_instance_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Aggregate savings summary across all completed runs for a tenant.
    pub async fn get_tenant_savings_summary(
        &self,
        tenant_id: &str,
    ) -> Result<crate::agent::savings::TenantSavingsSummary> {
        // Total row
        let total_row = sqlx::query(
            "SELECT
                COUNT(*)                     AS runs,
                COALESCE(SUM(human_hours_saved),    0) AS total_hours,
                COALESCE(SUM(human_cost_saved_usd), 0) AS total_human_cost,
                COALESCE(SUM(cost_usd),             0) AS total_ai_cost
             FROM goal_instances
             WHERE tenant_id = $1 AND status = 'completed'",
        )
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await?;

        let total_runs: u64 = total_row.get::<i64, _>("runs") as u64;
        let total_human_hours: f64 = total_row.get("total_hours");
        let total_human_cost: f64 = total_row.get("total_human_cost");
        let total_ai_cost: f64 = total_row.get("total_ai_cost");

        // Per-role breakdown
        let role_rows = sqlx::query(
            "SELECT
                gi.role_id,
                ar.name                                AS role_name,
                COUNT(*)                               AS runs,
                COALESCE(SUM(gi.human_hours_saved),    0) AS hours,
                COALESCE(SUM(gi.human_cost_saved_usd), 0) AS human_cost,
                COALESCE(SUM(gi.cost_usd),             0) AS ai_cost
             FROM goal_instances gi
             LEFT JOIN agent_roles ar ON ar.id = gi.role_id AND ar.tenant_id = gi.tenant_id
             WHERE gi.tenant_id = $1 AND gi.status = 'completed'
             GROUP BY gi.role_id, ar.name
             ORDER BY hours DESC
             LIMIT 20",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;

        let by_role: Vec<crate::agent::savings::RoleSavings> = role_rows
            .iter()
            .map(|r| crate::agent::savings::RoleSavings {
                role_id: r.get("role_id"),
                role_name: r.try_get("role_name").unwrap_or_else(|_| "Unknown Role".into()),
                runs: r.get::<i64, _>("runs") as u64,
                human_hours_saved: r.get("hours"),
                human_cost_saved_usd: r.get("human_cost"),
                ai_cost_usd: r.get("ai_cost"),
            })
            .collect();

        let roi = crate::agent::savings::TenantSavingsSummary::roi_multiple(total_human_cost, total_ai_cost);

        Ok(crate::agent::savings::TenantSavingsSummary {
            total_runs,
            total_human_hours: (total_human_hours * 100.0).round() / 100.0,
            total_human_cost_usd: (total_human_cost * 100.0).round() / 100.0,
            total_ai_cost_usd: (total_ai_cost * 10000.0).round() / 10000.0,
            roi_multiple: roi,
            by_role,
        })
    }

    // ── TenantConnector CRUD ────────────────────────────────────────────────

    pub async fn upsert_tenant_connector(&self, tc: &TenantConnector) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO tenant_connectors
                (id, tenant_id, name, category, base_url, auth_type, auth_credential_key,
                 source, source_docs, endpoints, summary, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            ON CONFLICT (tenant_id, name) DO UPDATE SET
                category            = EXCLUDED.category,
                base_url            = EXCLUDED.base_url,
                auth_type           = EXCLUDED.auth_type,
                auth_credential_key = EXCLUDED.auth_credential_key,
                source              = EXCLUDED.source,
                source_docs         = EXCLUDED.source_docs,
                endpoints           = EXCLUDED.endpoints,
                summary             = EXCLUDED.summary,
                updated_at          = EXCLUDED.updated_at
            "#,
        )
        .bind(&tc.id)
        .bind(&tc.tenant_id)
        .bind(&tc.name)
        .bind(&tc.category)
        .bind(&tc.base_url)
        .bind(serde_json::to_value(&tc.auth_type).unwrap_or_default())
        .bind(&tc.auth_credential_key)
        .bind(serde_json::to_value(&tc.source).unwrap_or_default())
        .bind(&tc.source_docs)
        .bind(serde_json::to_value(&tc.endpoints).unwrap_or_default())
        .bind(&tc.summary)
        .bind(tc.created_at)
        .bind(tc.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_tenant_connector(&self, tenant_id: &str, name: &str) -> Result<Option<TenantConnector>> {
        let row = sqlx::query("SELECT * FROM tenant_connectors WHERE tenant_id = $1 AND name = $2")
            .bind(tenant_id)
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_to_tenant_connector))
    }

    pub async fn list_tenant_connectors(&self, tenant_id: &str) -> Result<Vec<TenantConnector>> {
        let rows = sqlx::query("SELECT * FROM tenant_connectors WHERE tenant_id = $1 ORDER BY category, name")
            .bind(tenant_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_tenant_connector).collect())
    }

    pub async fn list_tenant_connectors_by_category(
        &self,
        tenant_id: &str,
        category: &str,
    ) -> Result<Vec<TenantConnector>> {
        let rows = sqlx::query("SELECT * FROM tenant_connectors WHERE tenant_id = $1 AND category = $2 ORDER BY name")
            .bind(tenant_id)
            .bind(category)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_tenant_connector).collect())
    }

    pub async fn delete_tenant_connector(&self, tenant_id: &str, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM tenant_connectors WHERE tenant_id = $1 AND name = $2")
            .bind(tenant_id)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── TenantWasmTool CRUD ────────────────────────────────────────────────

    pub async fn upsert_tenant_wasm_tool(&self, tool: &TenantWasmTool, module_bytes: &[u8]) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO tenant_wasm_tools
                (id, tenant_id, name, description, module_bytes, module_sha256, module_size_bytes,
                 exports, permissions, limits, enabled, version, last_used_at, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
            ON CONFLICT (tenant_id, name) DO UPDATE SET
                description        = EXCLUDED.description,
                module_bytes       = EXCLUDED.module_bytes,
                module_sha256      = EXCLUDED.module_sha256,
                module_size_bytes  = EXCLUDED.module_size_bytes,
                exports            = EXCLUDED.exports,
                permissions        = EXCLUDED.permissions,
                limits             = EXCLUDED.limits,
                enabled            = EXCLUDED.enabled,
                version            = tenant_wasm_tools.version + 1,
                updated_at         = EXCLUDED.updated_at
            "#,
        )
        .bind(&tool.id)
        .bind(&tool.tenant_id)
        .bind(&tool.name)
        .bind(&tool.description)
        .bind(module_bytes)
        .bind(&tool.module_sha256)
        .bind((tool.module_size_bytes.min(i64::MAX as u64)) as i64)
        .bind(serde_json::to_value(&tool.exports).unwrap_or_default())
        .bind(serde_json::to_value(&tool.permissions).unwrap_or_default())
        .bind(serde_json::to_value(&tool.limits.clamped()).unwrap_or_default())
        .bind(tool.enabled)
        .bind(tool.version as i32)
        .bind(tool.last_used_at)
        .bind(tool.created_at)
        .bind(tool.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_tenant_wasm_tool(&self, tenant_id: &str, name: &str) -> Result<Option<TenantWasmTool>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, name, description, module_sha256, module_size_bytes, exports,
                    permissions, limits, enabled, version, last_used_at, created_at, updated_at
             FROM tenant_wasm_tools
             WHERE tenant_id = $1 AND name = $2",
        )
        .bind(tenant_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_ref().map(row_to_tenant_wasm_tool))
    }

    pub async fn get_tenant_wasm_tool_with_module(
        &self,
        tenant_id: &str,
        name: &str,
    ) -> Result<Option<(TenantWasmTool, Vec<u8>)>> {
        let row = sqlx::query("SELECT * FROM tenant_wasm_tools WHERE tenant_id = $1 AND name = $2")
            .bind(tenant_id)
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            let module_bytes = r.try_get::<Vec<u8>, _>("module_bytes").unwrap_or_default();
            (row_to_tenant_wasm_tool(&r), module_bytes)
        }))
    }

    pub async fn list_tenant_wasm_tools(&self, tenant_id: &str) -> Result<Vec<TenantWasmTool>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, name, description, module_sha256, module_size_bytes, exports,
                    permissions, limits, enabled, version, last_used_at, created_at, updated_at
             FROM tenant_wasm_tools
             WHERE tenant_id = $1
             ORDER BY name",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_tenant_wasm_tool).collect())
    }

    pub async fn set_tenant_wasm_tool_enabled(&self, tenant_id: &str, name: &str, enabled: bool) -> Result<()> {
        sqlx::query(
            "UPDATE tenant_wasm_tools
             SET enabled = $1, updated_at = NOW()
             WHERE tenant_id = $2 AND name = $3",
        )
        .bind(enabled)
        .bind(tenant_id)
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn touch_tenant_wasm_tool_last_used(&self, tenant_id: &str, name: &str) -> Result<()> {
        sqlx::query(
            "UPDATE tenant_wasm_tools
             SET last_used_at = NOW(), updated_at = NOW()
             WHERE tenant_id = $1 AND name = $2",
        )
        .bind(tenant_id)
        .bind(name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_tenant_wasm_tool(&self, tenant_id: &str, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM tenant_wasm_tools WHERE tenant_id = $1 AND name = $2")
            .bind(tenant_id)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Wasm tool run audit ────────────────────────────────────────────────

    pub async fn insert_wasm_tool_run_audit(&self, run: &WasmToolRunAudit) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO wasm_tool_runs
                (id, tenant_id, tool_name, tool_version, agent_id, role_id, goal_instance_id,
                 success, elapsed_ms, fuel_used, memory_limit_bytes, error, created_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            "#,
        )
        .bind(&run.id)
        .bind(&run.tenant_id)
        .bind(&run.tool_name)
        .bind(run.tool_version as i32)
        .bind(&run.agent_id)
        .bind(&run.role_id)
        .bind(&run.goal_instance_id)
        .bind(run.success)
        .bind((run.elapsed_ms.min(i64::MAX as u64)) as i64)
        .bind(run.fuel_used.map(|v| (v.min(i64::MAX as u64)) as i64))
        .bind((run.memory_limit_bytes.min(i64::MAX as u64)) as i64)
        .bind(&run.error)
        .bind(run.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_wasm_tool_run_audit(
        &self,
        tenant_id: &str,
        tool_name: Option<&str>,
        limit: i64,
    ) -> Result<Vec<WasmToolRunAudit>> {
        let rows = if let Some(tool_name) = tool_name {
            sqlx::query(
                "SELECT * FROM wasm_tool_runs
                 WHERE tenant_id = $1 AND tool_name = $2
                 ORDER BY created_at DESC
                 LIMIT $3",
            )
            .bind(tenant_id)
            .bind(tool_name)
            .bind(limit.max(1).min(200))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT * FROM wasm_tool_runs
                 WHERE tenant_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2",
            )
            .bind(tenant_id)
            .bind(limit.max(1).min(200))
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows.iter().map(row_to_wasm_tool_run_audit).collect())
    }

    // ── WorkforceEventSubscription CRUD ─────────────────────────────────────

    pub async fn upsert_workforce_subscription(&self, sub: &WorkforceEventSubscription) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO workforce_event_subscriptions
                (id, tenant_id, subscriber_role_id, subscriber_agent_id,
                 event_filter, input_mapping, active, created_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            ON CONFLICT (id) DO UPDATE SET
                event_filter  = EXCLUDED.event_filter,
                input_mapping = EXCLUDED.input_mapping,
                active        = EXCLUDED.active
            "#,
        )
        .bind(&sub.id)
        .bind(&sub.tenant_id)
        .bind(&sub.subscriber_role_id)
        .bind(&sub.subscriber_agent_id)
        .bind(&sub.event_filter)
        .bind(&sub.input_mapping)
        .bind(sub.active)
        .bind(sub.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns all active subscriptions for a tenant.
    /// Called by the workforce event dispatcher after every GoalInstance completion.
    pub async fn list_active_workforce_subscriptions(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<WorkforceEventSubscription>> {
        let rows = sqlx::query(
            "SELECT * FROM workforce_event_subscriptions
             WHERE tenant_id = $1 AND active = TRUE",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_workforce_subscription).collect())
    }

    pub async fn deactivate_workforce_subscription(&self, tenant_id: &str, id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE workforce_event_subscriptions
             SET active = FALSE WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
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
        AgentStatus::PlanApprovalNeeded => "plan_approval_needed",
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
        "plan_approval_needed" => AgentStatus::PlanApprovalNeeded,
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

    let plan =
        row.try_get::<Option<serde_json::Value>, _>("plan").ok().flatten().and_then(|v| serde_json::from_value(v).ok());
    let metadata: serde_json::Value = row.get("metadata");
    let final_answer = row
        .try_get::<Option<String>, _>("final_answer")
        .ok()
        .flatten()
        .or_else(|| metadata.get("final_answer").and_then(|value| value.as_str()).map(str::to_string));

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
        final_answer,
        metadata,
        parent_agent_id: row.get("parent_agent_id"),
        pending_children,
        conversation_id: row.try_get("conversation_id").ok().flatten(),
        plan_rejection_count: row.try_get::<i32, _>("plan_rejection_count").ok().map(|v| v as u32).unwrap_or(0),
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

// ── AgentDefinition helpers ────────────────────────────────────────────────

fn agent_def_status_to_str(s: &AgentDefinitionStatus) -> &'static str {
    match s {
        AgentDefinitionStatus::Draft => "draft",
        AgentDefinitionStatus::Active => "active",
        AgentDefinitionStatus::Paused => "paused",
        AgentDefinitionStatus::Archived => "archived",
    }
}

fn str_to_agent_def_status(s: &str) -> AgentDefinitionStatus {
    match s {
        "active" => AgentDefinitionStatus::Active,
        "paused" => AgentDefinitionStatus::Paused,
        "archived" => AgentDefinitionStatus::Archived,
        _ => AgentDefinitionStatus::Draft,
    }
}

fn row_to_agent_definition(row: &PgRow) -> AgentDefinition {
    let connectors: Vec<String> = row
        .try_get::<serde_json::Value, _>("connectors")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let constraints: Vec<String> = row
        .try_get::<serde_json::Value, _>("constraints")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    AgentDefinition {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        name: row.get("name"),
        persona: row.get("persona"),
        connectors,
        constraints,
        memory_ref: row.try_get("memory_ref").unwrap_or_default(),
        status: str_to_agent_def_status(&row.get::<String, _>("status")),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ── AgentRole helpers ──────────────────────────────────────────────────────

fn role_status_to_str(s: &RoleStatus) -> &'static str {
    match s {
        RoleStatus::Draft => "draft",
        RoleStatus::Testing => "testing",
        RoleStatus::Active => "active",
        RoleStatus::Paused => "paused",
        RoleStatus::Archived => "archived",
    }
}

fn str_to_role_status(s: &str) -> RoleStatus {
    match s {
        "testing" => RoleStatus::Testing,
        "active" => RoleStatus::Active,
        "paused" => RoleStatus::Paused,
        "archived" => RoleStatus::Archived,
        _ => RoleStatus::Draft,
    }
}

fn memory_scope_to_str(s: &MemoryScope) -> &'static str {
    match s {
        MemoryScope::Global => "global",
        MemoryScope::Agent => "agent",
        MemoryScope::Role => "role",
    }
}

fn str_to_memory_scope(s: &str) -> MemoryScope {
    match s {
        "global" => MemoryScope::Global,
        "role" => MemoryScope::Role,
        _ => MemoryScope::Agent,
    }
}

fn str_to_role_category(s: &str) -> crate::agent::definition::RoleCategory {
    crate::agent::definition::RoleCategory::from_slug(s)
}

fn row_to_agent_role(row: &PgRow) -> AgentRole {
    let connectors: Vec<String> = row
        .try_get::<serde_json::Value, _>("connectors")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let tools: Vec<String> = row
        .try_get::<serde_json::Value, _>("tools")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let trigger: TriggerDef = row
        .try_get::<serde_json::Value, _>("trigger")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let output_spec: OutputSpec = row
        .try_get::<serde_json::Value, _>("output_spec")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let execution_limits: ExecutionLimits = row
        .try_get::<serde_json::Value, _>("execution_limits")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    AgentRole {
        id: row.get("id"),
        agent_id: row.get("agent_id"),
        tenant_id: row.get("tenant_id"),
        version: row.get::<i32, _>("version") as u32,
        status: str_to_role_status(&row.get::<String, _>("status")),
        name: row.get("name"),
        trigger,
        purpose: row.get("purpose"),
        role_category: str_to_role_category(
            &row.try_get::<String, _>("role_category").unwrap_or_else(|_| "general".into()),
        ),
        execution_guidelines: row
            .try_get::<Option<serde_json::Value>, _>("execution_guidelines")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        connectors,
        tools,
        output_spec,
        memory_scope: str_to_memory_scope(&row.try_get::<String, _>("memory_scope").unwrap_or_default()),
        execution_limits,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ── GoalInstance helpers ───────────────────────────────────────────────────

fn goal_instance_status_to_str(s: &GoalInstanceStatus) -> &'static str {
    match s {
        GoalInstanceStatus::Pending => "pending",
        GoalInstanceStatus::Running => "running",
        GoalInstanceStatus::Completed => "completed",
        GoalInstanceStatus::PartiallyComplete => "partially_complete",
        GoalInstanceStatus::Failed => "failed",
        GoalInstanceStatus::Cancelled => "cancelled",
    }
}

fn str_to_goal_instance_status(s: &str) -> GoalInstanceStatus {
    match s {
        "running" => GoalInstanceStatus::Running,
        "completed" => GoalInstanceStatus::Completed,
        "partially_complete" => GoalInstanceStatus::PartiallyComplete,
        "failed" => GoalInstanceStatus::Failed,
        "cancelled" => GoalInstanceStatus::Cancelled,
        _ => GoalInstanceStatus::Pending,
    }
}

fn row_to_goal_instance(row: &PgRow) -> GoalInstance {
    let trigger_source: TriggerSource = row
        .try_get::<serde_json::Value, _>("trigger_source")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(TriggerSource::Manual { created_by: "unknown".into() });
    GoalInstance {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        agent_id: row.get("agent_id"),
        role_id: row.get("role_id"),
        role_version: row.get::<i32, _>("role_version") as u32,
        input_data: row.try_get("input_data").unwrap_or(serde_json::Value::Null),
        status: str_to_goal_instance_status(&row.get::<String, _>("status")),
        result: row.try_get("result").ok().flatten(),
        failure_reason: row.try_get("failure_reason").ok().flatten(),
        trigger_source,
        is_test: row.try_get("is_test").unwrap_or(false),
        cost_usd: row.try_get("cost_usd").unwrap_or(0.0),
        human_hours_saved: row.try_get("human_hours_saved").unwrap_or(0.0),
        human_cost_saved_usd: row.try_get("human_cost_saved_usd").unwrap_or(0.0),
        agent_state_id: row.try_get("agent_state_id").ok().flatten(),
        triggered_by_goal_instance_id: row.try_get("triggered_by_goal_instance_id").ok().flatten(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.try_get("completed_at").ok().flatten(),
    }
}

// ── TenantConnector helpers ────────────────────────────────────────────────

fn row_to_tenant_connector(row: &PgRow) -> TenantConnector {
    let auth_type: ConnectorAuthType = row
        .try_get::<serde_json::Value, _>("auth_type")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(ConnectorAuthType::Bearer);
    let source: ConnectorSource = row
        .try_get::<serde_json::Value, _>("source")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(ConnectorSource::Manual);
    let endpoints: Vec<EndpointDef> = row
        .try_get::<serde_json::Value, _>("endpoints")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    TenantConnector {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        name: row.get("name"),
        category: row.get("category"),
        base_url: row.get("base_url"),
        auth_type,
        auth_credential_key: row.try_get("auth_credential_key").ok().flatten(),
        source,
        source_docs: row.try_get("source_docs").ok().flatten(),
        endpoints,
        summary: row.try_get("summary").unwrap_or_default(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ── WorkforceEventSubscription helpers ────────────────────────────────────

fn row_to_tenant_wasm_tool(row: &PgRow) -> TenantWasmTool {
    let exports: Vec<String> = row
        .try_get::<serde_json::Value, _>("exports")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let permissions: WasmToolPermissions = row
        .try_get::<serde_json::Value, _>("permissions")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let limits: WasmToolResourceLimits = row
        .try_get::<serde_json::Value, _>("limits")
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    TenantWasmTool {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        name: row.get("name"),
        description: row.try_get("description").unwrap_or_default(),
        module_sha256: row.get("module_sha256"),
        module_size_bytes: row.try_get::<i64, _>("module_size_bytes").unwrap_or_default().max(0) as u64,
        exports,
        permissions,
        limits,
        enabled: row.try_get("enabled").unwrap_or(true),
        version: row.try_get::<i32, _>("version").unwrap_or(1).max(1) as u32,
        last_used_at: row.try_get("last_used_at").ok().flatten(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_wasm_tool_run_audit(row: &PgRow) -> WasmToolRunAudit {
    WasmToolRunAudit {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        tool_name: row.get("tool_name"),
        tool_version: row.try_get::<i32, _>("tool_version").unwrap_or(1).max(1) as u32,
        agent_id: row.try_get("agent_id").ok().flatten(),
        role_id: row.try_get("role_id").ok().flatten(),
        goal_instance_id: row.try_get("goal_instance_id").ok().flatten(),
        success: row.try_get("success").unwrap_or(false),
        elapsed_ms: row.try_get::<i64, _>("elapsed_ms").unwrap_or_default().max(0) as u64,
        fuel_used: row.try_get::<Option<i64>, _>("fuel_used").ok().flatten().map(|v| v.max(0) as u64),
        memory_limit_bytes: row.try_get::<i64, _>("memory_limit_bytes").unwrap_or_default().max(0) as u64,
        error: row.try_get("error").ok().flatten(),
        created_at: row.get("created_at"),
    }
}

fn row_to_workforce_subscription(row: &PgRow) -> WorkforceEventSubscription {
    WorkforceEventSubscription {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        subscriber_role_id: row.get("subscriber_role_id"),
        subscriber_agent_id: row.get("subscriber_agent_id"),
        event_filter: row.get("event_filter"),
        input_mapping: row.try_get("input_mapping").unwrap_or(serde_json::Value::Object(Default::default())),
        active: row.try_get("active").unwrap_or(true),
        created_at: row.get("created_at"),
    }
}
