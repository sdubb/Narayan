use anyhow::Result;
use chrono::Utc;
use sqlx::{FromRow, PgPool, Row};

use crate::{
    tenant::{
        config::TenantConfig,
        model::{Tenant, TenantPlan, TenantStatus},
    },
    util::new_id,
};

pub struct TenantStore {
    pool: PgPool,
}

impl TenantStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenants (
                id         TEXT PRIMARY KEY,
                username   TEXT,
                name       TEXT NOT NULL,
                email      TEXT NOT NULL UNIQUE,
                password_hash TEXT,
                key_hash   TEXT NOT NULL,
                key_prefix TEXT NOT NULL,
                status     TEXT NOT NULL DEFAULT 'active',
                plan       TEXT NOT NULL DEFAULT 'free',
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE tenants ADD COLUMN IF NOT EXISTS username TEXT").execute(&self.pool).await?;
        sqlx::query("ALTER TABLE tenants ADD COLUMN IF NOT EXISTS password_hash TEXT").execute(&self.pool).await?;
        sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS tenants_username ON tenants (LOWER(username)) WHERE username IS NOT NULL")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS tenants_key_prefix ON tenants (key_prefix)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_configs (
                tenant_id   TEXT PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
                credentials JSONB NOT NULL DEFAULT '{}',
                routing     JSONB NOT NULL DEFAULT '{}',
                metadata    JSONB NOT NULL DEFAULT '{}'
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tenant_policy_rules (
                id        TEXT PRIMARY KEY,
                tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
                rules     JSONB NOT NULL DEFAULT '[]',
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS tenant_policy_rules_tenant ON tenant_policy_rules (tenant_id)")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Load tenant-specific policy rules from DB.
    /// Returns an empty ruleset (platform defaults only) if no custom rules are stored.
    pub async fn get_policy_rules(&self, tenant_id: &str) -> Result<crate::policy::rules::PolicyRuleSet> {
        let row = sqlx::query("SELECT rules FROM tenant_policy_rules WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;

        let rules: Vec<crate::policy::rules::PolicyRule> = row
            .and_then(|r| r.try_get::<serde_json::Value, _>("rules").ok())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        Ok(crate::policy::rules::PolicyRuleSet { tenant_id: tenant_id.into(), rules })
    }

    /// Upsert tenant-specific policy rules.
    pub async fn upsert_policy_rules(
        &self,
        tenant_id: &str,
        rules: &crate::policy::rules::PolicyRuleSet,
    ) -> Result<()> {
        let id = crate::util::new_id();
        sqlx::query(
            r#"INSERT INTO tenant_policy_rules (id, tenant_id, rules)
               VALUES ($1, $2, $3)
               ON CONFLICT (tenant_id) DO UPDATE SET
                   rules      = EXCLUDED.rules,
                   updated_at = NOW()"#,
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(serde_json::to_value(&rules.rules)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Tenant CRUD ─────────────────────────────────────────────────────────

    pub async fn create_tenant(
        &self,
        username: String,
        name: String,
        email: String,
        password_hash: String,
        key_hash: String,
        key_prefix: String,
    ) -> Result<Tenant> {
        let now = Utc::now();
        let id = new_id();

        sqlx::query(
            r#"INSERT INTO tenants (id, username, name, email, password_hash, key_hash, key_prefix, status, plan, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,'active','free',$8,$9)"#,
        )
        .bind(&id)
        .bind(&username)
        .bind(&name)
        .bind(&email)
        .bind(&password_hash)
        .bind(&key_hash)
        .bind(&key_prefix)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        // Create default empty config
        sqlx::query(
            "INSERT INTO tenant_configs (tenant_id, credentials, routing, metadata) VALUES ($1,'{}','{}','{}')",
        )
        .bind(&id)
        .execute(&self.pool)
        .await?;

        Ok(Tenant {
            id,
            username,
            name,
            email,
            key_hash,
            key_prefix,
            status: TenantStatus::Active,
            plan: TenantPlan::Free,
            created_at: now,
            updated_at: now,
        })
    }

    /// Look up tenant by the prefix of their API key.
    /// Used to quickly narrow candidates before full hash comparison.
    pub async fn get_by_key_prefix(&self, prefix: &str) -> Result<Option<Tenant>> {
        let row = sqlx::query_as::<_, TenantRow>(
            "SELECT id, username, name, email, key_hash, key_prefix, status, plan,
                    created_at, updated_at
             FROM tenants WHERE key_prefix = $1 AND status = 'active'",
        )
        .bind(prefix)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(row_to_tenant))
    }

    pub async fn get_by_id(&self, id: &str) -> Result<Option<Tenant>> {
        let row = sqlx::query_as::<_, TenantRow>(
            "SELECT id, username, name, email, key_hash, key_prefix, status, plan,
                    created_at, updated_at
             FROM tenants WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(row_to_tenant))
    }

    pub async fn update_plan(&self, tenant_id: &str, plan: &str) -> Result<()> {
        sqlx::query("UPDATE tenants SET plan = $1, updated_at = NOW() WHERE id = $2")
            .bind(plan)
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn suspend(&self, tenant_id: &str) -> Result<()> {
        sqlx::query("UPDATE tenants SET status = 'suspended', updated_at = NOW() WHERE id = $1")
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn activate(&self, tenant_id: &str) -> Result<()> {
        sqlx::query("UPDATE tenants SET status = 'active', updated_at = NOW() WHERE id = $1")
            .bind(tenant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<Tenant>> {
        let rows = sqlx::query_as::<_, TenantRow>(
            "SELECT id, username, name, email, key_hash, key_prefix, status, plan,
                    created_at, updated_at
             FROM tenants ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_tenant).collect())
    }

    pub async fn get_auth_by_identifier(&self, identifier: &str) -> Result<Option<TenantAuthRow>> {
        let row = sqlx::query_as::<_, TenantAuthRow>(
            "SELECT id, username, email, password_hash, plan
               FROM tenants
              WHERE status = 'active'
                AND (
                    LOWER(email) = LOWER($1)
                    OR LOWER(COALESCE(username, '')) = LOWER($1)
                )
              LIMIT 1",
        )
        .bind(identifier.trim())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    // ── Tenant config ───────────────────────────────────────────────────────

    pub async fn get_config(&self, tenant_id: &str) -> Result<TenantConfig> {
        let row = sqlx::query("SELECT credentials, routing, metadata FROM tenant_configs WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(r) => {
                let credentials_val: serde_json::Value = r.get("credentials");
                let routing_val: serde_json::Value = r.get("routing");
                let metadata: serde_json::Value = r.get("metadata");
                let credentials = serde_json::from_value(credentials_val).unwrap_or_default();
                let routing = serde_json::from_value(routing_val).unwrap_or_default();
                Ok(TenantConfig { tenant_id: tenant_id.to_string(), credentials, routing, metadata })
            }
            None => Ok(TenantConfig::new(tenant_id.to_string())),
        }
    }

    pub async fn upsert_config(&self, config: &TenantConfig) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO tenant_configs (tenant_id, credentials, routing, metadata)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (tenant_id) DO UPDATE SET
                credentials = EXCLUDED.credentials,
                routing     = EXCLUDED.routing,
                metadata    = EXCLUDED.metadata
        "#,
        )
        .bind(&config.tenant_id)
        .bind(serde_json::to_value(&config.credentials)?)
        .bind(serde_json::to_value(&config.routing)?)
        .bind(&config.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

// ── Internal row type for sqlx query_as ───────────────────────────────────

#[derive(FromRow)]
struct TenantRow {
    id: String,
    username: Option<String>,
    name: String,
    email: String,
    key_hash: String,
    key_prefix: String,
    status: String,
    plan: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(FromRow)]
pub struct TenantAuthRow {
    pub id: String,
    pub username: Option<String>,
    pub email: String,
    pub password_hash: Option<String>,
    pub plan: String,
}

fn row_to_tenant(r: TenantRow) -> Tenant {
    let status = match r.status.as_str() {
        "suspended" => TenantStatus::Suspended,
        "deleted" => TenantStatus::Deleted,
        _ => TenantStatus::Active,
    };
    let plan = match r.plan.as_str() {
        "go" => TenantPlan::Go,
        "pro" => TenantPlan::Pro,
        "enterprise" => TenantPlan::Enterprise,
        _ => TenantPlan::Free,
    };
    Tenant {
        id: r.id,
        username: r.username.unwrap_or_else(|| r.email.split('@').next().unwrap_or_default().to_string()),
        name: r.name,
        email: r.email,
        key_hash: r.key_hash,
        key_prefix: r.key_prefix,
        status,
        plan,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}
