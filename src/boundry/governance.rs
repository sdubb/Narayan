/// The revocation and freeze state of a boundary handshake.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationState {
    /// Handshake is active and executable.
    #[default]
    Active,
    /// Emergency administrative freeze. No new envelopes can be sent or received.
    /// Can be unfrozen — requires both parties to re-sign the handshake.
    Frozen { frozen_by: String, frozen_at: chrono::DateTime<chrono::Utc>, reason: String },
    /// Permanent revocation. Cannot be reversed. A new handshake must be created.
    Revoked { revoked_by: String, revoked_at: chrono::DateTime<chrono::Utc>, reason: String },
}

impl RevocationState {
    pub fn is_active(&self) -> bool {
        matches!(self, RevocationState::Active)
    }

    pub fn is_frozen(&self) -> bool {
        matches!(self, RevocationState::Frozen { .. })
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self, RevocationState::Revoked { .. })
    }

    pub fn is_executable(&self) -> bool {
        self.is_active()
    }
}

/// Operations that modify revocation state. All operations are stored in
/// the boundary_freeze_log for audit purposes.
pub struct GovernanceStore {
    pool: sqlx::PgPool,
}

impl GovernanceStore {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_freeze_log (
                id           TEXT PRIMARY KEY,
                handshake_id TEXT NOT NULL,
                tenant_id    TEXT NOT NULL,
                action       TEXT NOT NULL, -- 'freeze' | 'unfreeze' | 'revoke'
                actor        TEXT NOT NULL, -- tenant_id of actor
                reason       TEXT NOT NULL,
                occurred_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS boundary_freeze_log_handshake
             ON boundary_freeze_log(handshake_id, occurred_at)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_breach_reports (
                id              TEXT PRIMARY KEY,
                handshake_id    TEXT NOT NULL,
                reporter_tenant TEXT NOT NULL,
                severity        TEXT NOT NULL DEFAULT 'low', -- 'low' | 'medium' | 'high' | 'critical'
                description     TEXT NOT NULL,
                affected_envelopes TEXT[] NOT NULL DEFAULT '{}',
                reported_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                resolved_at     TIMESTAMPTZ
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_consent_history (
                id              TEXT PRIMARY KEY,
                handshake_id    TEXT NOT NULL,
                version         INTEGER NOT NULL,
                consented_by    TEXT NOT NULL,
                consent_text    TEXT NOT NULL,
                consented_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS boundary_consent_history_handshake
             ON boundary_consent_history(handshake_id, version)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_rate_counters (
                handshake_id TEXT NOT NULL,
                tenant_id    TEXT NOT NULL,
                window_start TIMESTAMPTZ NOT NULL,
                request_count INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (handshake_id, tenant_id, window_start)
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn freeze(
        &self,
        handshake_id: &str,
        tenant_id: &str,
        actor: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO boundary_freeze_log (id, handshake_id, tenant_id, action, actor, reason)
             VALUES ($1, $2, $3, 'freeze', $4, $5)",
        )
        .bind(crate::util::new_id())
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(actor)
        .bind(reason)
        .execute(&self.pool)
        .await?;

        // Update the handshake revocation_state to Frozen in boundary_handshakes
        let revocation_json = serde_json::json!({
            "frozen": { "frozen_by": actor, "frozen_at": chrono::Utc::now(), "reason": reason }
        });
        sqlx::query(
            "UPDATE boundary_handshakes SET revocation_state = $1 WHERE handshake_id = $2 AND tenant_id = $3",
        )
        .bind(revocation_json)
        .bind(handshake_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn unfreeze(
        &self,
        handshake_id: &str,
        tenant_id: &str,
        actor: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO boundary_freeze_log (id, handshake_id, tenant_id, action, actor, reason)
             VALUES ($1, $2, $3, 'unfreeze', $4, 'unfreeze by actor')",
        )
        .bind(crate::util::new_id())
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(actor)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "UPDATE boundary_handshakes SET revocation_state = '\"active\"' WHERE handshake_id = $1 AND tenant_id = $2",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn revoke(
        &self,
        handshake_id: &str,
        tenant_id: &str,
        actor: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO boundary_freeze_log (id, handshake_id, tenant_id, action, actor, reason)
             VALUES ($1, $2, $3, 'revoke', $4, $5)",
        )
        .bind(crate::util::new_id())
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(actor)
        .bind(reason)
        .execute(&self.pool)
        .await?;

        let revocation_json = serde_json::json!({
            "revoked": { "revoked_by": actor, "revoked_at": chrono::Utc::now(), "reason": reason }
        });
        sqlx::query(
            "UPDATE boundary_handshakes SET revocation_state = $1 WHERE handshake_id = $2 AND tenant_id = $3",
        )
        .bind(revocation_json)
        .bind(handshake_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn report_breach(
        &self,
        handshake_id: &str,
        reporter_tenant: &str,
        severity: &str,
        description: &str,
        affected_envelopes: Vec<String>,
    ) -> anyhow::Result<String> {
        let id = crate::util::new_id();
        sqlx::query(
            "INSERT INTO boundary_breach_reports
             (id, handshake_id, reporter_tenant, severity, description, affected_envelopes)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(handshake_id)
        .bind(reporter_tenant)
        .bind(severity)
        .bind(description)
        .bind(&affected_envelopes)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Check rate limit returns true if the current request would exceed the limit.
    pub async fn is_rate_limited(
        &self,
        handshake_id: &str,
        tenant_id: &str,
        max_requests: u32,
        window_secs: u64,
    ) -> anyhow::Result<bool> {
        let window_start = chrono::Utc::now()
            - chrono::Duration::seconds(window_secs as i64);

        let row = sqlx::query(
            "SELECT COALESCE(SUM(request_count), 0) as total
             FROM boundary_rate_counters
             WHERE handshake_id = $1 AND tenant_id = $2 AND window_start >= $3",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await?;

        use sqlx::Row;
        let total: i64 = row.get("total");
        Ok(total >= max_requests as i64)
    }

    /// Increment the rate counter for this handshake.
    pub async fn increment_rate_counter(
        &self,
        handshake_id: &str,
        tenant_id: &str,
        window_secs: u64,
    ) -> anyhow::Result<()> {
        // Round window start to nearest window boundary
        let now = chrono::Utc::now();
        let epoch_secs = now.timestamp();
        let window_start_secs = epoch_secs - (epoch_secs % window_secs as i64);
        let window_start = chrono::DateTime::from_timestamp(window_start_secs, 0)
            .unwrap_or(now);

        sqlx::query(
            "INSERT INTO boundary_rate_counters (handshake_id, tenant_id, window_start, request_count)
             VALUES ($1, $2, $3, 1)
             ON CONFLICT (handshake_id, tenant_id, window_start) DO UPDATE
             SET request_count = boundary_rate_counters.request_count + 1",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(window_start)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Returns true if the handshake is currently frozen.
    /// Uses the freeze_log — checks that the most recent action for this handshake is 'freeze'.
    pub async fn is_frozen(&self, handshake_id: &str, tenant_id: &str) -> anyhow::Result<bool> {
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT action FROM boundary_freeze_log
             WHERE handshake_id = $1 AND tenant_id = $2
             ORDER BY occurred_at DESC LIMIT 1",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.get::<String, _>("action") == "freeze").unwrap_or(false))
    }

    /// Atomic check-and-increment. Returns true if the request is ALLOWED (not rate-limited).
    /// Call this instead of is_rate_limited + increment_rate_counter separately.
    pub async fn check_and_increment_rate(
        &self,
        handshake_id: &str,
        tenant_id: &str,
        max_requests: u32,
        window_secs: u64,
    ) -> anyhow::Result<bool> {
        let limited = self.is_rate_limited(handshake_id, tenant_id, max_requests, window_secs).await?;
        if limited {
            return Ok(false);
        }
        self.increment_rate_counter(handshake_id, tenant_id, window_secs).await?;
        Ok(true)
    }
}
