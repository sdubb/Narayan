//! Durable persistence layer for DAG workflow executions.
//!
//! Provides the `WorkflowStore` trait and its Postgres implementation.
//! Every step state transition is persisted to the database before execution
//! begins, ensuring crash-safe resume at the step boundary.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::agent::dag::{StepNode, StepStatus, Workflow, WorkflowStatus};

// ═══════════════════════════════════════════════════════════════════════════
// TRAIT
// ═══════════════════════════════════════════════════════════════════════════

/// Persistence interface for DAG workflows.
///
/// The DB is the **single source of truth** for step state.
/// Steps read inputs from and write outputs to the store.
/// No shared mutable in-memory state.
#[async_trait]
pub trait WorkflowStore: Send + Sync {
    /// Create a new workflow execution with all its steps.
    async fn create_workflow(&self, workflow: &Workflow) -> Result<()>;

    /// Load a complete workflow by ID.
    async fn get_workflow(&self, workflow_id: &str) -> Result<Option<Workflow>>;

    /// Load the active (non-terminal) workflow for an agent.
    /// Used on resume after crash.
    async fn resume_workflow(&self, agent_id: &str) -> Result<Option<Workflow>>;

    /// Update a single step's status, attempt count, output, and error.
    async fn update_step_status(
        &self,
        step_id: &str,
        status: &StepStatus,
        attempt: u32,
        output: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> Result<()>;

    /// Get the output_data for a specific step. Used by `StepInput::from_predecessors`.
    async fn get_step_output(&self, step_id: &str) -> Result<Option<serde_json::Value>>;

    /// Mark the overall workflow status (completed, failed, cancelled).
    async fn update_workflow_status(&self, workflow_id: &str, status: WorkflowStatus) -> Result<()>;

    /// Atomic persist for DAG expansion: insert new nodes and update dependencies of existing nodes.
    async fn save_expanded_nodes(
        &self,
        workflow_id: &str,
        new_steps: &[StepNode],
        updated_dependencies: &[(String, Vec<String>)],
    ) -> Result<()>;
}

// ═══════════════════════════════════════════════════════════════════════════
// POSTGRES IMPLEMENTATION
// ═══════════════════════════════════════════════════════════════════════════

pub struct PgWorkflowStore {
    pool: PgPool,
}

impl PgWorkflowStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run the schema migration for workflow tables.
    /// Called from `PostgresStore::migrate()`.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS workflow_executions (
                id          TEXT PRIMARY KEY,
                agent_id    TEXT NOT NULL,
                tenant_id   TEXT NOT NULL,
                goal        TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'running',
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_wf_exec_agent
             ON workflow_executions (agent_id) WHERE status = 'running'",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS workflow_steps (
                id               TEXT PRIMARY KEY,
                workflow_id      TEXT NOT NULL REFERENCES workflow_executions(id),
                step_index       INT NOT NULL,
                description      TEXT NOT NULL,
                tool             TEXT,
                tool_args        JSONB,
                success_criteria TEXT NOT NULL DEFAULT '',
                condition        JSONB,
                depends_on       TEXT[] NOT NULL DEFAULT '{}',
                status           TEXT NOT NULL DEFAULT 'pending',
                attempt          INT NOT NULL DEFAULT 0,
                max_retries      INT NOT NULL DEFAULT 1,
                retry_backoff    INT NOT NULL DEFAULT 2,
                schema_mode      TEXT NOT NULL DEFAULT 'strict',
                input_schema     JSONB,
                output_schema    JSONB,
                input_data       JSONB,
                output_data      JSONB,
                error            TEXT,
                started_at       TIMESTAMPTZ,
                completed_at     TIMESTAMPTZ,
                retry_at         TIMESTAMPTZ,
                created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_wf_steps_workflow
             ON workflow_steps (workflow_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_wf_steps_status
             ON workflow_steps (workflow_id, status)",
        )
        .execute(&self.pool)
        .await?;

        // Add iteration expansion columns
        sqlx::query("ALTER TABLE workflow_steps ADD COLUMN IF NOT EXISTS step_kind TEXT NOT NULL DEFAULT '{\"type\":\"normal\"}'")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE workflow_steps ADD COLUMN IF NOT EXISTS foreach TEXT").execute(&self.pool).await?;
        sqlx::query("ALTER TABLE workflow_steps ADD COLUMN IF NOT EXISTS parent_step_id TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE workflow_steps ADD COLUMN IF NOT EXISTS item_index INT").execute(&self.pool).await?;

        // Add workflow_id column to agents table for routing
        sqlx::query("ALTER TABLE agents ADD COLUMN IF NOT EXISTS workflow_id TEXT").execute(&self.pool).await?;

        Ok(())
    }
}

#[async_trait]
impl WorkflowStore for PgWorkflowStore {
    async fn create_workflow(&self, workflow: &Workflow) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Insert workflow execution
        sqlx::query(
            "INSERT INTO workflow_executions (id, agent_id, tenant_id, goal, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&workflow.id)
        .bind(&workflow.agent_id)
        .bind(&workflow.tenant_id)
        .bind(&workflow.goal)
        .bind(workflow_status_str(&workflow.status))
        .bind(workflow.created_at)
        .bind(workflow.updated_at)
        .execute(&mut *tx)
        .await?;

        // Insert all steps
        for node in &workflow.nodes {
            let depends_on: Vec<&str> = node.depends_on.iter().map(String::as_str).collect();
            let status_str = step_status_to_str(&node.status);

            let kind_json = serde_json::to_string(&node.kind).unwrap_or_else(|_| "{\"type\":\"normal\"}".to_string());
            let (parent_step_id, item_index) = match &node.kind {
                crate::agent::dag::StepKind::ForEachItem { parent, index } => {
                    (Some(parent.as_str()), Some(*index as i32))
                }
                _ => (None, None),
            };

            sqlx::query(
                "INSERT INTO workflow_steps (
                    id, workflow_id, step_index, description, tool, tool_args,
                    success_criteria, condition, foreach, step_kind, parent_step_id, item_index, 
                    depends_on, status, attempt,
                    max_retries, retry_backoff, schema_mode,
                    input_schema, output_schema, created_at, updated_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)",
            )
            .bind(&node.id)
            .bind(&workflow.id)
            .bind(node.index as i32)
            .bind(&node.description)
            .bind(&node.tool)
            .bind(&node.tool_args)
            .bind(&node.success_criteria)
            .bind(node.condition.as_ref().and_then(|c| serde_json::to_value(c).ok()))
            .bind(&node.foreach)
            .bind(kind_json)
            .bind(parent_step_id)
            .bind(item_index)
            .bind(&depends_on)
            .bind(status_str)
            .bind(node.attempt as i32)
            .bind(node.retry_policy.max_attempts as i32)
            .bind(node.retry_policy.backoff_secs as i32)
            .bind(schema_mode_str(&node.schema_mode))
            .bind(&node.input_schema)
            .bind(&node.output_schema)
            .bind(workflow.created_at)
            .bind(workflow.updated_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn get_workflow(&self, workflow_id: &str) -> Result<Option<Workflow>> {
        let wf_row = sqlx::query("SELECT * FROM workflow_executions WHERE id = $1")
            .bind(workflow_id)
            .fetch_optional(&self.pool)
            .await?;

        let Some(wf_row) = wf_row else { return Ok(None) };

        let step_rows = sqlx::query("SELECT * FROM workflow_steps WHERE workflow_id = $1 ORDER BY step_index")
            .bind(workflow_id)
            .fetch_all(&self.pool)
            .await?;

        let nodes = step_rows.iter().map(row_to_step_node).collect();

        Ok(Some(Workflow {
            id: wf_row.get("id"),
            agent_id: wf_row.get("agent_id"),
            tenant_id: wf_row.get("tenant_id"),
            goal: wf_row.get("goal"),
            nodes,
            status: parse_workflow_status(wf_row.get::<String, _>("status").as_str()),
            created_at: wf_row.get("created_at"),
            updated_at: wf_row.get("updated_at"),
        }))
    }

    async fn resume_workflow(&self, agent_id: &str) -> Result<Option<Workflow>> {
        let wf_row = sqlx::query(
            "SELECT * FROM workflow_executions
             WHERE agent_id = $1 AND status = 'running'
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(wf_row) = wf_row else { return Ok(None) };
        let workflow_id: String = wf_row.get("id");
        self.get_workflow(&workflow_id).await
    }

    async fn update_step_status(
        &self,
        step_id: &str,
        status: &StepStatus,
        attempt: u32,
        output: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> Result<()> {
        let status_str = step_status_to_str(status);
        let retry_at = match status {
            StepStatus::Retrying { next_retry_at, .. } => Some(*next_retry_at),
            _ => None,
        };
        let completed_at: Option<DateTime<Utc>> = if status.is_terminal() { Some(Utc::now()) } else { None };
        let started_at: Option<DateTime<Utc>> =
            if matches!(status, StepStatus::Running) { Some(Utc::now()) } else { None };

        sqlx::query(
            "UPDATE workflow_steps SET
                status = $1,
                attempt = $2,
                output_data = COALESCE($3, output_data),
                error = COALESCE($4, error),
                retry_at = $5,
                completed_at = COALESCE($6, completed_at),
                started_at = COALESCE($7, started_at),
                updated_at = NOW()
             WHERE id = $8",
        )
        .bind(status_str)
        .bind(attempt as i32)
        .bind(output)
        .bind(error)
        .bind(retry_at)
        .bind(completed_at)
        .bind(started_at)
        .bind(step_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn get_step_output(&self, step_id: &str) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT output_data FROM workflow_steps WHERE id = $1")
            .bind(step_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.and_then(|r| r.get::<Option<serde_json::Value>, _>("output_data")))
    }

    async fn update_workflow_status(&self, workflow_id: &str, status: WorkflowStatus) -> Result<()> {
        sqlx::query("UPDATE workflow_executions SET status = $1, updated_at = NOW() WHERE id = $2")
            .bind(workflow_status_str(&status))
            .bind(workflow_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn save_expanded_nodes(
        &self,
        workflow_id: &str,
        new_steps: &[StepNode],
        updated_dependencies: &[(String, Vec<String>)],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for node in new_steps {
            let depends_on: Vec<&str> = node.depends_on.iter().map(String::as_str).collect();
            let status_str = step_status_to_str(&node.status);
            let kind_json = serde_json::to_string(&node.kind).unwrap_or_else(|_| "{\"type\":\"normal\"}".to_string());
            let (parent_step_id, item_index) = match &node.kind {
                crate::agent::dag::StepKind::ForEachItem { parent, index } => {
                    (Some(parent.as_str()), Some(*index as i32))
                }
                _ => (None, None),
            };

            sqlx::query(
                "INSERT INTO workflow_steps (
                    id, workflow_id, step_index, description, tool, tool_args,
                    success_criteria, condition, foreach, step_kind, parent_step_id, item_index, 
                    depends_on, status, attempt,
                    max_retries, retry_backoff, schema_mode,
                    input_schema, output_schema, created_at, updated_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,NOW(),NOW())",
            )
            .bind(&node.id)
            .bind(workflow_id)
            .bind(node.index as i32)
            .bind(&node.description)
            .bind(&node.tool)
            .bind(&node.tool_args)
            .bind(&node.success_criteria)
            .bind(node.condition.as_ref().and_then(|c| serde_json::to_value(c).ok()))
            .bind(&node.foreach)
            .bind(kind_json)
            .bind(parent_step_id)
            .bind(item_index)
            .bind(&depends_on)
            .bind(status_str)
            .bind(node.attempt as i32)
            .bind(node.retry_policy.max_attempts as i32)
            .bind(node.retry_policy.backoff_secs as i32)
            .bind(schema_mode_str(&node.schema_mode))
            .bind(&node.input_schema)
            .bind(&node.output_schema)
            .execute(&mut *tx)
            .await?;
        }

        for (node_id, deps) in updated_dependencies {
            let depends_on: Vec<&str> = deps.iter().map(String::as_str).collect();
            sqlx::query(
                "UPDATE workflow_steps SET depends_on = $1, updated_at = NOW() WHERE id = $2 AND workflow_id = $3",
            )
            .bind(&depends_on)
            .bind(node_id)
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════════════════

fn step_status_to_str(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::Ready => "ready",
        StepStatus::Running => "running",
        StepStatus::Succeeded => "succeeded",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
        StepStatus::Retrying { .. } => "retrying",
        StepStatus::AwaitingInput { .. } => "awaiting_input",
        StepStatus::AwaitingChildren { .. } => "awaiting_children",
    }
}

fn parse_step_status(s: &str, retry_at: Option<DateTime<Utc>>, attempt: i32) -> StepStatus {
    match s {
        "pending" => StepStatus::Pending,
        "ready" => StepStatus::Ready,
        "running" => StepStatus::Running,
        "succeeded" => StepStatus::Succeeded,
        "failed" => StepStatus::Failed,
        "skipped" => StepStatus::Skipped,
        "retrying" => {
            StepStatus::Retrying { attempt: attempt as u32, next_retry_at: retry_at.unwrap_or_else(Utc::now) }
        }
        "awaiting_input" => StepStatus::AwaitingInput { questions: vec![] },
        "awaiting_children" => StepStatus::AwaitingChildren { child_ids: vec![] },
        _ => StepStatus::Pending,
    }
}

fn workflow_status_str(status: &WorkflowStatus) -> &'static str {
    match status {
        WorkflowStatus::Running => "running",
        WorkflowStatus::Completed => "completed",
        WorkflowStatus::Failed => "failed",
        WorkflowStatus::Cancelled => "cancelled",
    }
}

fn parse_workflow_status(s: &str) -> WorkflowStatus {
    match s {
        "completed" => WorkflowStatus::Completed,
        "failed" => WorkflowStatus::Failed,
        "cancelled" => WorkflowStatus::Cancelled,
        _ => WorkflowStatus::Running,
    }
}

fn schema_mode_str(mode: &crate::agent::definition::SchemaMode) -> &'static str {
    match mode {
        crate::agent::definition::SchemaMode::Strict => "strict",
        crate::agent::definition::SchemaMode::Warn => "warn",
        crate::agent::definition::SchemaMode::Off => "off",
    }
}

fn parse_schema_mode(s: &str) -> crate::agent::definition::SchemaMode {
    match s {
        "warn" => crate::agent::definition::SchemaMode::Warn,
        "off" => crate::agent::definition::SchemaMode::Off,
        _ => crate::agent::definition::SchemaMode::Strict,
    }
}

fn row_to_step_node(row: &sqlx::postgres::PgRow) -> StepNode {
    let status_str: String = row.get("status");
    let retry_at: Option<DateTime<Utc>> = row.get("retry_at");
    let attempt: i32 = row.get("attempt");
    let max_retries: i32 = row.get("max_retries");
    let retry_backoff: i32 = row.get("retry_backoff");
    let depends_on: Vec<String> = row.get("depends_on");
    let schema_mode_str_val: String = row.get("schema_mode");
    let condition_json: Option<serde_json::Value> = row.try_get("condition").unwrap_or(None);
    let foreach: Option<String> = row.try_get("foreach").unwrap_or(None);
    let step_kind_str: String = row.try_get("step_kind").unwrap_or_else(|_| "{\"type\":\"normal\"}".to_string());
    let kind: crate::agent::dag::StepKind = serde_json::from_str(&step_kind_str).unwrap_or_default();

    StepNode {
        id: row.get("id"),
        index: row.get::<i32, _>("step_index") as usize,
        description: row.get("description"),
        tool: row.get("tool"),
        tool_args: row.get("tool_args"),
        success_criteria: row.get("success_criteria"),
        condition: condition_json.and_then(|v| serde_json::from_value(v).ok()),
        foreach,
        kind,
        depends_on,
        status: parse_step_status(&status_str, retry_at, attempt),
        attempt: attempt as u32,
        retry_policy: crate::agent::definition::RetryPolicy {
            max_attempts: max_retries as u32,
            backoff_secs: retry_backoff as u64,
            retry_on: vec![],
        },
        schema_mode: parse_schema_mode(&schema_mode_str_val),
        input_schema: row.get("input_schema"),
        output_schema: row.get("output_schema"),
        output_data: row.get("output_data"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        error: row.get("error"),
    }
}
