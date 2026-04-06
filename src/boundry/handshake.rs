use anyhow::Result;
use sqlx::{PgPool, Row};

use crate::boundry::{BoundaryParty, BoundaryScope};
use crate::util::new_id;

/// Handshake lifecycle: create draft, accept, load, persist.
pub struct HandshakeStore {
    pool: PgPool,
}

impl HandshakeStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_handshakes (
                handshake_id          TEXT NOT NULL,
                handshake_version     INTEGER NOT NULL DEFAULT 1,
                tenant_id             TEXT NOT NULL,
                requester_tenant_id   TEXT NOT NULL,
                responder_tenant_id   TEXT NOT NULL,
                requester_name        TEXT NOT NULL,
                responder_name        TEXT NOT NULL,
                requester_endpoint    TEXT NOT NULL,
                responder_endpoint    TEXT NOT NULL,
                requester_team_id     TEXT,
                responder_team_id     TEXT,
                scope                 TEXT NOT NULL DEFAULT 'cross_enterprise',
                request_schema        JSONB NOT NULL DEFAULT '{}',
                response_schema       JSONB NOT NULL DEFAULT '{}',
                request_visible_fields  TEXT[] NOT NULL DEFAULT '{}',
                response_visible_fields TEXT[] NOT NULL DEFAULT '{}',
                response_timeout_secs   INTEGER NOT NULL DEFAULT 300,
                idempotent              BOOLEAN NOT NULL DEFAULT TRUE,
                requester_accepted      BOOLEAN NOT NULL DEFAULT FALSE,
                responder_accepted      BOOLEAN NOT NULL DEFAULT FALSE,
                requester_signature     TEXT,
                responder_signature     TEXT,
                revocation_state        JSONB NOT NULL DEFAULT '\"active\"',
                revocation_state_text   TEXT NOT NULL DEFAULT 'active',
                valid_from              TIMESTAMPTZ,
                valid_until             TIMESTAMPTZ,
                data_barrier            JSONB NOT NULL DEFAULT '{}',
                consent_version         INTEGER NOT NULL DEFAULT 1,
                rate_limit              JSONB,
                approval_policy         JSONB,
                created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                accepted_at             TIMESTAMPTZ,
                PRIMARY KEY (handshake_id, handshake_version, tenant_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS boundary_handshakes_tenant
             ON boundary_handshakes(tenant_id, revocation_state_text)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ── Draft creation ─────────────────────────────────────────────────────────

    /// Create a new draft handshake on the requester's side.
    /// Returns the handshake_id. The responder must separately call accept().
    pub async fn create_draft(
        &self,
        tenant_id: &str,
        requester: &BoundaryParty,
        responder: &BoundaryParty,
        scope: &BoundaryScope,
        request_schema: serde_json::Value,
        response_schema: serde_json::Value,
        request_visible_fields: Vec<String>,
        response_visible_fields: Vec<String>,
        response_timeout_secs: u64,
        idempotent: bool,
    ) -> Result<String> {
        let handshake_id = new_id();
        let scope_str = match scope {
            BoundaryScope::CrossEnterprise => "cross_enterprise",
            BoundaryScope::CrossTeam { .. } => "cross_team",
        };
        let (req_team_id, resp_team_id) = match scope {
            BoundaryScope::CrossTeam { requester_team_id, responder_team_id } =>
                (Some(requester_team_id.as_str()), Some(responder_team_id.as_str())),
            _ => (None, None),
        };

        sqlx::query(
            r#"INSERT INTO boundary_handshakes
               (handshake_id, handshake_version, tenant_id,
                requester_tenant_id, responder_tenant_id,
                requester_name, responder_name,
                requester_endpoint, responder_endpoint,
                requester_team_id, responder_team_id, scope,
                request_schema, response_schema,
                request_visible_fields, response_visible_fields,
                response_timeout_secs, idempotent,
                requester_accepted, responder_accepted)
               VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                       $12, $13, $14, $15, $16, $17, TRUE, FALSE)"#,
        )
        .bind(&handshake_id)
        .bind(tenant_id)
        .bind(&requester.tenant_id)
        .bind(&responder.tenant_id)
        .bind(&requester.display_name)
        .bind(&responder.display_name)
        .bind(&requester.acp_endpoint)
        .bind(&responder.acp_endpoint)
        .bind(req_team_id)
        .bind(resp_team_id)
        .bind(scope_str)
        .bind(&request_schema)
        .bind(&response_schema)
        .bind(&request_visible_fields)
        .bind(&response_visible_fields)
        .bind(response_timeout_secs as i64)
        .bind(idempotent)
        .execute(&self.pool)
        .await?;

        Ok(handshake_id)
    }

    /// Accept a handshake on the responder's side.
    /// Both sides must have requester_accepted AND responder_accepted = TRUE
    /// before the handshake is considered live.
    pub async fn accept(&self, handshake_id: &str, tenant_id: &str, as_responder: bool) -> Result<()> {
        let column = if as_responder { "responder_accepted" } else { "requester_accepted" };
        let sql = format!(
            "UPDATE boundary_handshakes SET {} = TRUE, accepted_at = NOW()
             WHERE handshake_id = $1 AND tenant_id = $2",
            column
        );
        sqlx::query(&sql)
            .bind(handshake_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Load the most recent version of a handshake for a given tenant.
    pub async fn load(
        &self,
        handshake_id: &str,
        tenant_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query(
            "SELECT * FROM boundary_handshakes
             WHERE handshake_id = $1 AND tenant_id = $2
             ORDER BY handshake_version DESC
             LIMIT 1",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            serde_json::json!({
                "handshake_id": r.get::<String, _>("handshake_id"),
                "handshake_version": r.get::<i32, _>("handshake_version"),
                "tenant_id": r.get::<String, _>("tenant_id"),
                "requester_tenant_id": r.get::<String, _>("requester_tenant_id"),
                "responder_tenant_id": r.get::<String, _>("responder_tenant_id"),
                "requester_name": r.get::<String, _>("requester_name"),
                "responder_name": r.get::<String, _>("responder_name"),
                "requester_endpoint": r.get::<String, _>("requester_endpoint"),
                "responder_endpoint": r.get::<String, _>("responder_endpoint"),
                "scope": r.get::<String, _>("scope"),
                "request_schema": r.get::<serde_json::Value, _>("request_schema"),
                "response_schema": r.get::<serde_json::Value, _>("response_schema"),
                "requester_accepted": r.get::<bool, _>("requester_accepted"),
                "responder_accepted": r.get::<bool, _>("responder_accepted"),
                "revocation_state": r.get::<serde_json::Value, _>("revocation_state"),
                "valid_from": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("valid_from"),
                "valid_until": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("valid_until"),
                "approval_policy": r.get::<Option<serde_json::Value>, _>("approval_policy"),
                "rate_limit": r.get::<Option<serde_json::Value>, _>("rate_limit"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "accepted_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("accepted_at"),
            })
        }))
    }

    /// List all handshakes for a tenant.
    pub async fn list_for_tenant(&self, tenant_id: &str) -> Result<Vec<serde_json::Value>> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (handshake_id) handshake_id, handshake_version,
             requester_name, responder_name, scope,
             requester_accepted, responder_accepted, revocation_state_text, created_at, accepted_at
             FROM boundary_handshakes WHERE tenant_id = $1
             ORDER BY handshake_id, handshake_version DESC",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "handshake_id": r.get::<String, _>("handshake_id"),
                    "handshake_version": r.get::<i32, _>("handshake_version"),
                    "requester_name": r.get::<String, _>("requester_name"),
                    "responder_name": r.get::<String, _>("responder_name"),
                    "scope": r.get::<String, _>("scope"),
                    "requester_accepted": r.get::<bool, _>("requester_accepted"),
                    "responder_accepted": r.get::<bool, _>("responder_accepted"),
                    "revocation_state": r.get::<String, _>("revocation_state_text"),
                    "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                    "accepted_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("accepted_at"),
                })
            })
            .collect())
    }

    /// Check if a handshake is active and both parties have accepted.
    pub async fn is_live(&self, handshake_id: &str, tenant_id: &str) -> Result<bool> {
        let row = sqlx::query(
            "SELECT requester_accepted, responder_accepted, revocation_state_text
             FROM boundary_handshakes
             WHERE handshake_id = $1 AND tenant_id = $2
             ORDER BY handshake_version DESC LIMIT 1",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .map(|r| {
                r.get::<bool, _>("requester_accepted")
                    && r.get::<bool, _>("responder_accepted")
                    && r.get::<String, _>("revocation_state_text") == "active"
            })
            .unwrap_or(false))
    }
}
