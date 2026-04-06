use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

use crate::boundry::{BoundaryAuditEvent, BoundaryAuditRecord, BoundarySide};
use crate::util::new_id;

/// Append-only audit ledger for boundary exchanges.
/// Every envelope sent or received writes an immutable record.
/// Records are chain-hashed so neither party can alter their copy
/// without breaking the chain verifiable by the other.
pub struct BoundaryAuditStore {
    pool: PgPool,
}

impl BoundaryAuditStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_audit_ledger (
                record_id           TEXT PRIMARY KEY,
                envelope_id         TEXT NOT NULL,
                handshake_id        TEXT NOT NULL,
                tenant_id           TEXT NOT NULL,
                side                TEXT NOT NULL CHECK (side IN ('requester', 'responder')),
                event               JSONB NOT NULL,
                visible_payload     JSONB NOT NULL DEFAULT '{}',
                payload_hash        TEXT NOT NULL,
                chain_hash          TEXT NOT NULL,
                previous_chain_hash TEXT,
                recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS boundary_audit_ledger_envelope
             ON boundary_audit_ledger(envelope_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS boundary_audit_ledger_handshake
             ON boundary_audit_ledger(handshake_id, recorded_at)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_pending_envelopes (
                envelope_id         TEXT PRIMARY KEY,
                handshake_id        TEXT NOT NULL,
                tenant_id           TEXT NOT NULL,
                workflow_id         TEXT NOT NULL,
                step_id             TEXT NOT NULL,
                correlation_token   TEXT NOT NULL,
                idempotency_key     TEXT NOT NULL UNIQUE,
                sent_at             TIMESTAMPTZ NOT NULL,
                expires_at          TIMESTAMPTZ NOT NULL,
                attempt_count       INTEGER NOT NULL DEFAULT 1,
                approval_review_id  TEXT    -- set when parked for approval
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ── Audit record operations ───────────────────────────────────────────────

    /// Append a new audit record. Computes and sets the chain_hash automatically.
    pub async fn append(
        &self,
        envelope_id: &str,
        handshake_id: &str,
        tenant_id: &str,
        side: BoundarySide,
        event: BoundaryAuditEvent,
        visible_payload: serde_json::Value,
        payload_hash: &str,
    ) -> Result<BoundaryAuditRecord> {
        let record_id = new_id();
        let now = chrono::Utc::now();

        // Fetch the previous chain hash for this handshake+tenant to build the chain
        let previous_chain_hash: Option<String> = sqlx::query(
            "SELECT chain_hash FROM boundary_audit_ledger
             WHERE handshake_id = $1 AND tenant_id = $2
             ORDER BY recorded_at DESC LIMIT 1",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?
        .map(|r| r.get("chain_hash"));

        let chain_hash = compute_chain_hash(
            previous_chain_hash.as_deref(),
            &record_id,
            envelope_id,
            payload_hash,
        );

        let side_str = match &side { BoundarySide::Requester => "requester", BoundarySide::Responder => "responder" };
        let event_json = serde_json::to_value(&event)?;

        sqlx::query(
            "INSERT INTO boundary_audit_ledger
             (record_id, envelope_id, handshake_id, tenant_id, side, event,
              visible_payload, payload_hash, chain_hash, previous_chain_hash, recorded_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&record_id)
        .bind(envelope_id)
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(side_str)
        .bind(&event_json)
        .bind(&visible_payload)
        .bind(payload_hash)
        .bind(&chain_hash)
        .bind(&previous_chain_hash)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(BoundaryAuditRecord {
            record_id,
            envelope_id: envelope_id.to_string(),
            handshake_id: handshake_id.to_string(),
            recorded_at: now,
            side,
            event,
            visible_payload_fields: visible_payload,
            payload_hash: payload_hash.to_string(),
            chain_hash,
            previous_chain_hash,
        })
    }

    /// Verify the chain integrity for all records of a handshake+tenant.
    /// Returns Ok(record_count) if valid, or Err with the broken record_id.
    pub async fn verify_chain(&self, handshake_id: &str, tenant_id: &str) -> Result<usize> {
        let rows = sqlx::query(
            "SELECT record_id, envelope_id, payload_hash, chain_hash, previous_chain_hash
             FROM boundary_audit_ledger
             WHERE handshake_id = $1 AND tenant_id = $2
             ORDER BY recorded_at ASC",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;

        let count = rows.len();
        let mut expected_prev: Option<String> = None;

        for row in &rows {
            let record_id: String = row.get("record_id");
            let envelope_id: String = row.get("envelope_id");
            let payload_hash: String = row.get("payload_hash");
            let chain_hash: String = row.get("chain_hash");
            let previous_chain_hash: Option<String> = row.get("previous_chain_hash");

            // Verify previous_chain_hash matches what we tracked
            if previous_chain_hash != expected_prev {
                anyhow::bail!(
                    "chain broken at record {}: expected prev {:?}, got {:?}",
                    record_id, expected_prev, previous_chain_hash
                );
            }

            // Recompute and verify the chain_hash
            let computed = compute_chain_hash(
                previous_chain_hash.as_deref(),
                &record_id,
                &envelope_id,
                &payload_hash,
            );
            if computed != chain_hash {
                anyhow::bail!(
                    "chain hash mismatch at record {}: expected {}, got {}",
                    record_id, computed, chain_hash
                );
            }

            expected_prev = Some(chain_hash);
        }

        Ok(count)
    }

    /// Query audit records for a handshake, paginated.
    pub async fn query(
        &self,
        handshake_id: &str,
        tenant_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let rows = sqlx::query(
            "SELECT record_id, envelope_id, side, event, visible_payload, payload_hash,
                    chain_hash, previous_chain_hash, recorded_at
             FROM boundary_audit_ledger
             WHERE handshake_id = $1 AND tenant_id = $2
             ORDER BY recorded_at DESC
             LIMIT $3 OFFSET $4",
        )
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "record_id": r.get::<String, _>("record_id"),
                    "envelope_id": r.get::<String, _>("envelope_id"),
                    "side": r.get::<String, _>("side"),
                    "event": r.get::<serde_json::Value, _>("event"),
                    "visible_payload": r.get::<serde_json::Value, _>("visible_payload"),
                    "payload_hash": r.get::<String, _>("payload_hash"),
                    "chain_hash": r.get::<String, _>("chain_hash"),
                    "previous_chain_hash": r.get::<Option<String>, _>("previous_chain_hash"),
                    "recorded_at": r.get::<chrono::DateTime<chrono::Utc>, _>("recorded_at"),
                })
            })
            .collect())
    }

    // ── Pending envelope operations ───────────────────────────────────────────

    pub async fn park_envelope(
        &self,
        envelope_id: &str,
        handshake_id: &str,
        tenant_id: &str,
        workflow_id: &str,
        step_id: &str,
        correlation_token: &str,
        idempotency_key: &str,
        sent_at: chrono::DateTime<chrono::Utc>,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO boundary_pending_envelopes
             (envelope_id, handshake_id, tenant_id, workflow_id, step_id,
              correlation_token, idempotency_key, sent_at, expires_at, attempt_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1)
             ON CONFLICT (envelope_id) DO NOTHING",
        )
        .bind(envelope_id)
        .bind(handshake_id)
        .bind(tenant_id)
        .bind(workflow_id)
        .bind(step_id)
        .bind(correlation_token)
        .bind(idempotency_key)
        .bind(sent_at)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn resolve_envelope(&self, envelope_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM boundary_pending_envelopes WHERE envelope_id = $1")
            .bind(envelope_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// ── Chain hash computation ────────────────────────────────────────────────────

/// Computes SHA-256(previous_chain_hash || record_id || envelope_id || payload_hash).
/// The `||` is simple string concatenation with a separator to avoid collisions.
pub fn compute_chain_hash(
    previous_chain_hash: Option<&str>,
    record_id: &str,
    envelope_id: &str,
    payload_hash: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(previous_chain_hash.unwrap_or("GENESIS").as_bytes());
    hasher.update(b"||");
    hasher.update(record_id.as_bytes());
    hasher.update(b"||");
    hasher.update(envelope_id.as_bytes());
    hasher.update(b"||");
    hasher.update(payload_hash.as_bytes());
    hex::encode(hasher.finalize())
}

/// SHA-256 hash of a JSON payload (deterministic, sorted keys).
pub fn hash_payload(payload: &serde_json::Value) -> String {
    let canonical = serde_json::to_string(payload).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_hash_is_deterministic() {
        let h1 = compute_chain_hash(None, "rec1", "env1", "hash1");
        let h2 = compute_chain_hash(None, "rec1", "env1", "hash1");
        assert_eq!(h1, h2);
    }

    #[test]
    fn chain_hash_changes_on_mutation() {
        let h1 = compute_chain_hash(None, "rec1", "env1", "hash1");
        let h2 = compute_chain_hash(None, "rec1", "env1", "MUTATED");
        assert_ne!(h1, h2);
    }

    #[test]
    fn genesis_differs_from_chained() {
        let genesis = compute_chain_hash(None, "rec1", "env1", "hash1");
        let chained = compute_chain_hash(Some(&genesis), "rec2", "env1", "hash2");
        assert_ne!(genesis, chained);
    }

    #[test]
    fn payload_hash_deterministic() {
        let p = serde_json::json!({ "a": 1, "b": true });
        assert_eq!(hash_payload(&p), hash_payload(&p));
    }
}
