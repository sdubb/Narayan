use anyhow::Result;
use std::sync::Arc;

use crate::boundry::{
    audit::{hash_payload, BoundaryAuditStore},
    data_barrier::DataBarrierPolicy,
    governance::GovernanceStore,
    BoundaryAuditEvent, BoundaryEnvelope, BoundaryOutcome, BoundaryResponse, BoundarySide,
    BoundaryScope, BoundaryStep,
};
use crate::storage::PostgresStore;

/// The boundary runtime executes `acp_boundary` steps.
///
/// CrossEnterprise path:
///   payload → data_barrier check → Ed25519 sign → ACP HTTP send → park or await response
///
/// CrossTeam path (same Narayan instance, different teams):
///   payload → data_barrier check → write to peer team pending queue → park or await
pub struct BoundaryRuntime {
    pub store: Arc<PostgresStore>,
    pub audit: Arc<BoundaryAuditStore>,
    pub governance: Arc<GovernanceStore>,
    pub http_client: reqwest::Client,
    /// This instance's Ed25519 private key (hex-encoded). Used to sign outbound envelopes.
    pub signing_key_hex: String,
    /// This instance's ACP base URL — used as the callback endpoint in envelopes.
    pub acp_base_url: String,
    pub tenant_id: String,
}

impl BoundaryRuntime {
    /// Execute a requester-side boundary step.
    /// Returns the typed response payload on success.
    pub async fn execute_requester_step(
        &self,
        step: &BoundaryStep,
        resolved_inputs: serde_json::Value,
        workflow_id: &str,
        scope: &BoundaryScope,
        data_barrier: &DataBarrierPolicy,
    ) -> Result<BoundaryResponse> {
        let payload_bytes = serde_json::to_string(&resolved_inputs)?.len();

        // 1. Data barrier check
        let barrier_violations = data_barrier.check_outbound(&resolved_inputs, payload_bytes, None);
        if !barrier_violations.is_empty() {
            anyhow::bail!(
                "data barrier violations: {}",
                barrier_violations.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("; ")
            );
        }

        // 2. Build envelope
        let envelope_id = uuid::Uuid::new_v4().to_string();
        let correlation_token = uuid::Uuid::new_v4().to_string();
        let idempotency_key = format!("{}-{}", step.id, workflow_id);
        let now = chrono::Utc::now();
        let expires_at = now + chrono::Duration::seconds(step.retry_policy.max_attempts as i64 * 60 + 300);

        let signature = sign_envelope(&self.signing_key_hex, &envelope_id, &resolved_inputs);

        let callback_endpoint = format!("{}/boundary/callback", self.acp_base_url);

        let envelope = BoundaryEnvelope {
            envelope_id: envelope_id.clone(),
            handshake_id: step.handshake_id.clone(),
            handshake_version: step.handshake_version,
            requester_tenant_id: self.tenant_id.clone(),
            responder_tenant_id: String::new(), // filled from handshake
            payload: apply_redaction(&resolved_inputs, &data_barrier.redact_outbound_fields),
            sent_at: now,
            expires_at,
            idempotency_key: idempotency_key.clone(),
            requester_signature: signature,
            callback_endpoint: callback_endpoint.clone(),
            correlation_token: correlation_token.clone(),
        };

        // 3. Persist audit record (EnvelopeSent)
        let payload_hash = hash_payload(&resolved_inputs);
        self.audit
            .append(
                &envelope_id,
                &step.handshake_id,
                &self.tenant_id,
                BoundarySide::Requester,
                BoundaryAuditEvent::EnvelopeSent,
                serde_json::json!({}), // visible fields applied by caller
                &payload_hash,
            )
            .await?;

        // 4. Park the envelope so we survive a restart
        self.audit
            .park_envelope(
                &envelope_id,
                &step.handshake_id,
                &self.tenant_id,
                workflow_id,
                &step.id,
                &correlation_token,
                &idempotency_key,
                now,
                expires_at,
            )
            .await?;

        // 5. Route by scope
        let response = match scope {
            BoundaryScope::CrossEnterprise => {
                self.send_cross_enterprise(&envelope, step).await?
            }
            BoundaryScope::CrossTeam { responder_team_id, .. } => {
                self.send_cross_team(&envelope, responder_team_id).await?
            }
        };

        // 6. Record response received
        let resp_payload_hash = hash_payload(
            response.payload.as_ref().unwrap_or(&serde_json::Value::Null),
        );
        self.audit
            .append(
                &envelope_id,
                &step.handshake_id,
                &self.tenant_id,
                BoundarySide::Requester,
                BoundaryAuditEvent::ResponseReceived,
                serde_json::json!({}),
                &resp_payload_hash,
            )
            .await?;

        // 7. Remove from pending
        self.audit.resolve_envelope(&envelope_id).await?;

        Ok(response)
    }

    async fn send_cross_enterprise(
        &self,
        envelope: &BoundaryEnvelope,
        step: &BoundaryStep,
    ) -> Result<BoundaryResponse> {
        let url = format!("{}/boundary/receive", step.peer_endpoint);
        let resp = self
            .http_client
            .post(&url)
            .json(envelope)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("ACP boundary send failed: HTTP {}", resp.status());
        }

        let boundary_response: BoundaryResponse = resp.json().await?;
        Ok(boundary_response)
    }

    async fn send_cross_team(
        &self,
        envelope: &BoundaryEnvelope,
        responder_team_id: &str,
    ) -> Result<BoundaryResponse> {
        // CrossTeam: write to the responder team's pending queue in the same DB.
        // The responder team's boundary agent polls this queue.
        let pool = self.store.pool();
        sqlx::query(
            "INSERT INTO boundary_pending_envelopes
             (envelope_id, handshake_id, tenant_id, workflow_id, step_id,
              correlation_token, idempotency_key, sent_at, expires_at)
             VALUES ($1, $2, $3, 'cross_team', 'cross_team', $4, $5, $6, $7)
             ON CONFLICT (envelope_id) DO NOTHING",
        )
        .bind(&envelope.envelope_id)
        .bind(&envelope.handshake_id)
        .bind(responder_team_id) // scoped to responder team
        .bind(&envelope.correlation_token)
        .bind(&envelope.idempotency_key)
        .bind(envelope.sent_at)
        .bind(envelope.expires_at)
        .execute(&pool)
        .await?;

        // For now return a Pending response — the response will come via callback
        Ok(BoundaryResponse {
            envelope_id: envelope.envelope_id.clone(),
            handshake_id: envelope.handshake_id.clone(),
            correlation_token: envelope.correlation_token.clone(),
            outcome: BoundaryOutcome::Pending { estimated_completion_secs: Some(300) },
            payload: None,
            failure: None,
            responded_at: chrono::Utc::now(),
            responder_signature: String::new(),
        })
    }
}

/// Apply field redaction: replace redacted fields with a SHA-256 hash.
fn apply_redaction(payload: &serde_json::Value, redact_fields: &[String]) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    let obj = match payload.as_object() {
        Some(o) => o,
        None => return payload.clone(),
    };
    let mut out = serde_json::Map::new();
    for (k, v) in obj {
        if redact_fields.contains(k) {
            let mut hasher = Sha256::new();
            hasher.update(v.to_string().as_bytes());
            out.insert(k.clone(), serde_json::Value::String(format!("[redacted:{}]", hex::encode(hasher.finalize()))));
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// Ed25519 signing stub. In production: parse signing_key_hex as ed25519 private key bytes.
fn sign_envelope(signing_key_hex: &str, envelope_id: &str, payload: &serde_json::Value) -> String {
    // v1 stub — HMAC-SHA256 for structural wiring without the ed25519-dalek dep yet
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(signing_key_hex.as_bytes());
    hasher.update(b"||");
    hasher.update(envelope_id.as_bytes());
    hasher.update(b"||");
    hasher.update(payload.to_string().as_bytes());
    format!("sig_hmac_{}", hex::encode(hasher.finalize()))
}
