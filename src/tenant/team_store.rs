use anyhow::Result;
use chrono::Utc;
use sqlx::{FromRow, PgPool, Row};

use crate::{
    tenant::team_model::{TeamMember, TeamMemberRole, TeamStatus, TeamSummary, TenantTeam},
    util::new_id,
};

pub struct TeamStore {
    pool: PgPool,
}

impl TeamStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── Migrations ──────────────────────────────────────────────────────────

    pub async fn migrate(&self) -> Result<()> {
        // Teams table — one row per department/sub-group within a tenant.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_teams (
                id          TEXT PRIMARY KEY,
                tenant_id   TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                name        TEXT NOT NULL,
                slug        TEXT NOT NULL,
                description TEXT,
                status      TEXT NOT NULL DEFAULT 'active',
                metadata    JSONB NOT NULL DEFAULT '{}',
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE (tenant_id, slug)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS tenant_teams_tenant ON tenant_teams(tenant_id)",
        )
        .execute(&self.pool)
        .await?;

        // Team membership — maps tenant accounts to roles within a team.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_team_members (
                team_id    TEXT NOT NULL REFERENCES tenant_teams(id) ON DELETE CASCADE,
                tenant_id  TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                role       TEXT NOT NULL DEFAULT 'member',
                added_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (team_id, tenant_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS tenant_team_members_tenant ON tenant_team_members(tenant_id)",
        )
        .execute(&self.pool)
        .await?;

        // Agent-to-team scoping (additive — NULL means tenant-wide, unchanged behaviour).
        sqlx::query(
            "ALTER TABLE agent_definitions ADD COLUMN IF NOT EXISTS team_id TEXT REFERENCES tenant_teams(id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS agent_definitions_team ON agent_definitions(team_id) WHERE team_id IS NOT NULL",
        )
        .execute(&self.pool)
        .await
        .ok(); // ignore if agent_definitions doesn't exist yet in test environments

        Ok(())
    }

    // ── Team CRUD ────────────────────────────────────────────────────────────

    pub async fn create_team(
        &self,
        tenant_id: &str,
        name: String,
        slug: String,
        description: Option<String>,
    ) -> Result<TenantTeam> {
        let id = new_id();
        let now = Utc::now();

        sqlx::query(
            r#"INSERT INTO tenant_teams (id, tenant_id, name, slug, description, status, metadata, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, 'active', '{}', $6, $7)"#,
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(&name)
        .bind(&slug)
        .bind(&description)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(TenantTeam {
            id,
            tenant_id: tenant_id.to_string(),
            name,
            slug,
            description,
            status: TeamStatus::Active,
            metadata: serde_json::Value::Object(Default::default()),
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get_team(&self, team_id: &str) -> Result<Option<TenantTeam>> {
        let row = sqlx::query_as::<_, TeamRow>(
            "SELECT id, tenant_id, name, slug, description, status, metadata, created_at, updated_at
             FROM tenant_teams WHERE id = $1",
        )
        .bind(team_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(team_from_row))
    }

    /// Returns all teams for a tenant, with member counts.
    pub async fn list_teams_for_tenant(&self, tenant_id: &str) -> Result<Vec<TeamSummary>> {
        let rows = sqlx::query(
            r#"SELECT t.id, t.name, t.slug, t.status,
                      COUNT(m.tenant_id) AS member_count
               FROM tenant_teams t
               LEFT JOIN tenant_team_members m ON m.team_id = t.id
               WHERE t.tenant_id = $1
               GROUP BY t.id, t.name, t.slug, t.status
               ORDER BY t.name"#,
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| TeamSummary {
                id: r.get("id"),
                name: r.get("name"),
                slug: r.get("slug"),
                status: if r.get::<String, _>("status") == "suspended" {
                    TeamStatus::Suspended
                } else {
                    TeamStatus::Active
                },
                member_count: r.get("member_count"),
            })
            .collect())
    }

    pub async fn suspend_team(&self, team_id: &str) -> Result<()> {
        sqlx::query("UPDATE tenant_teams SET status = 'suspended', updated_at = NOW() WHERE id = $1")
            .bind(team_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn activate_team(&self, team_id: &str) -> Result<()> {
        sqlx::query("UPDATE tenant_teams SET status = 'active', updated_at = NOW() WHERE id = $1")
            .bind(team_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Membership ────────────────────────────────────────────────────────────

    pub async fn add_member(&self, team_id: &str, tenant_id: &str, role: TeamMemberRole) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO tenant_team_members (team_id, tenant_id, role, added_at)
               VALUES ($1, $2, $3, NOW())
               ON CONFLICT (team_id, tenant_id) DO UPDATE SET role = EXCLUDED.role"#,
        )
        .bind(team_id)
        .bind(tenant_id)
        .bind(role.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_member(&self, team_id: &str, tenant_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM tenant_team_members WHERE team_id = $1 AND tenant_id = $2")
            .bind(team_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_member_role(&self, team_id: &str, tenant_id: &str) -> Result<Option<TeamMemberRole>> {
        let row = sqlx::query("SELECT role FROM tenant_team_members WHERE team_id = $1 AND tenant_id = $2")
            .bind(team_id)
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|r| TeamMemberRole::from_str(r.get::<String, _>("role").as_str())))
    }

    pub async fn list_members(&self, team_id: &str) -> Result<Vec<TeamMember>> {
        let rows = sqlx::query(
            "SELECT team_id, tenant_id, role, added_at FROM tenant_team_members WHERE team_id = $1 ORDER BY added_at",
        )
        .bind(team_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| TeamMember {
                team_id: r.get("team_id"),
                tenant_id: r.get("tenant_id"),
                role: TeamMemberRole::from_str(r.get::<String, _>("role").as_str()),
                added_at: r.get("added_at"),
            })
            .collect())
    }

    /// Check if a tenant is a member of this team with at least the given role.
    pub async fn assert_member_role(
        &self,
        team_id: &str,
        tenant_id: &str,
        required: &TeamMemberRole,
    ) -> Result<TeamMemberRole> {
        let role = self
            .get_member_role(team_id, tenant_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("not a member of team {}", team_id))?;

        if !role.satisfies(required) {
            anyhow::bail!(
                "insufficient team role: have {}, need {}",
                role,
                required
            );
        }
        Ok(role)
    }

    /// Verify a team belongs to the given tenant (ownership check before any mutation).
    pub async fn assert_team_owner(&self, team_id: &str, tenant_id: &str) -> Result<TenantTeam> {
        let team = self
            .get_team(team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("team not found: {}", team_id))?;

        if team.tenant_id != tenant_id {
            anyhow::bail!("team {} does not belong to tenant {}", team_id, tenant_id);
        }
        Ok(team)
    }
}

// ── Internal row mapping ────────────────────────────────────────────────────

#[derive(FromRow)]
struct TeamRow {
    id: String,
    tenant_id: String,
    name: String,
    slug: String,
    description: Option<String>,
    status: String,
    metadata: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn team_from_row(r: TeamRow) -> TenantTeam {
    TenantTeam {
        id: r.id,
        tenant_id: r.tenant_id,
        name: r.name,
        slug: r.slug,
        description: r.description,
        status: if r.status == "suspended" { TeamStatus::Suspended } else { TeamStatus::Active },
        metadata: r.metadata,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}
