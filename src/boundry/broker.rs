// src/boundry/broker.rs
//
// Narayan as a Universal Boundary Broker.
//
// Problem this solves
// ───────────────────
// The original boundary model assumes both Company A and Company B run Narayan.
// That is a strong assumption. In practice:
//   - Company A runs Narayan.
//   - Company B runs LangGraph, CrewAI, a custom ACP agent, or a legacy REST service.
//   - Neither company wants to give the other a direct network path.
//   - Both companies want governance, audit, and approval on the exchange.
//
// Narayan Broker is the answer. It is a neutral, trusted intermediary that:
//   1. Accepts ACP messages from any external agent (Company B's agent).
//   2. Applies the same governance layer (handshake, data barrier, revocation,
//      rate limit, approval policy, bilateral audit) as the native boundary.
//   3. Routes the governed payload to Company A's Narayan workflow step.
//   4. Returns Company A's response back through the same governance path.
//
// Neither party sees the other's internal structure.
// Both parties get the full audit trail.
// Either party can freeze or revoke at any time.
//
// External agents connect via ACP. They do not need to run Narayan.
// They only need to:
//   1. Register an ExternalAgentRegistration with the Narayan broker tenant.
//   2. Accept the handshake (HTTP POST to our /boundary/broker/handshake/:id/accept).
//   3. Send envelopes to /boundary/broker/receive.
//   4. Poll or webhook-receive responses from /boundary/broker/response.
//
// Architecture
// ────────────
//
//  External Agent (any platform)
//         │
//         │  ACP envelope (HTTP POST)
//         ▼
//  ┌──────────────────────────────────────────┐
//  │         NARAYAN BROKER LAYER             │
//  │                                           │
//  │  ① Verify signature (Ed25519 or HMAC)    │
//  │  ② Validate handshake (scope: Brokered)  │
//  │  ③ Check revocation + freeze state       │
//  │  ④ Run data barrier (PII scan + redact)  │
//  │  ⑤ Check rate limit                      │
//  │  ⑥ Evaluate approval policy              │
//  │  ⑦ Write audit record (external side)    │
//  │  ⑧ Route to internal Narayan workflow    │
//  │     OR park for approval                 │
//  │  ⑨ Write audit record (internal side)   │
//  │  ⑩ Return governed response             │
//  └──────────────────────────────────────────┘
//         │
//         │  Governed, typed response
//         ▼
//  External Agent receives structured response

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;

use crate::boundry::{
    audit::{hash_payload, BoundaryAuditStore},
    data_barrier::DataBarrierPolicy,
    governance::GovernanceStore,
    BoundaryAuditEvent, BoundaryEnvelope, BoundarySide,
};
use crate::util::new_id;

// ─── External agent registration ─────────────────────────────────────────────

/// An external agent registered with the Narayan broker.
/// This is NOT a Narayan tenant — it is a foreign platform's agent endpoint
/// that can participate in governed boundary exchanges through Narayan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAgentRegistration {
    /// Unique identifier for this external agent, generated at registration.
    pub external_agent_id: String,

    /// The Narayan tenant that acts as broker for this external agent.
    /// All governance (handshakes, audit, approvals) is owned by this tenant.
    pub broker_tenant_id: String,

    /// Human-readable name for the external agent (e.g., "BankCorp Credit Agent").
    pub display_name: String,

    /// The ACP/HTTP endpoint Narayan will call to deliver governed responses.
    /// None if the external agent polls instead of receiving webhooks.
    pub callback_endpoint: Option<String>,

    /// Platform the external agent runs on (informational, for UI).
    /// e.g., "langchain", "crewai", "custom_acp", "n8n", "zapier", "rest"
    pub platform_hint: String,

    /// Verification method for incoming envelopes from this agent.
    pub verification: ExternalAgentVerification,

    /// Whether this external agent is currently allowed to send envelopes.
    pub status: ExternalAgentStatus,

    /// Allowed handshake IDs for this agent. If empty: any brokered handshake
    /// belonging to broker_tenant_id is allowed.
    pub allowed_handshake_ids: Vec<String>,

    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ExternalAgentVerification {
    /// HMAC-SHA256 using a shared secret. Simple but sufficient for mutual-trust pairs.
    HmacSha256 { secret_hash: String },
    /// Ed25519 public key. The external agent signs each envelope.
    Ed25519 { public_key_hex: String },
    /// API key in Authorization header. Least secure but easiest for legacy systems.
    ApiKey { key_hash: String },
    /// No verification. Use only for internal testing.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentStatus {
    Active,
    Suspended,
    Revoked,
}

// ─── Brokered boundary scope ──────────────────────────────────────────────────

/// Extended BoundaryScope for brokered (external agent) connections.
/// This is the third scope beyond CrossEnterprise and CrossTeam.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerScope {
    /// The Narayan tenant acting as broker.
    pub broker_tenant_id: String,
    /// The registered external agent on the sending side.
    pub external_agent_id: String,
    /// Which side of the handshake the external agent occupies.
    pub external_agent_role: BrokerAgentRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerAgentRole {
    /// External agent sends requests; Narayan workflow responds.
    ExternalRequester,
    /// Narayan workflow sends requests; external agent responds.
    ExternalResponder,
    /// Both sides are external agents; Narayan purely brokers.
    BothExternal,
}

// ─── Brokered envelope ────────────────────────────────────────────────────────

/// An envelope received from an external (non-Narayan) agent.
/// The broker verifies, governs, and forwards this to the internal workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokeredEnvelope {
    /// Narayan envelope, built by or on behalf of the external agent.
    pub envelope: BoundaryEnvelope,
    /// The external agent ID that sent this envelope.
    pub external_agent_id: String,
    /// Verification material (signature, api key hash, etc.) for the incoming envelope.
    pub verification_material: Option<String>,
    /// Platform-specific metadata the external agent attaches (optional, redacted if PII).
    #[serde(default)]
    pub platform_metadata: serde_json::Value,
}

// ─── Broker store ─────────────────────────────────────────────────────────────

pub struct BrokerStore {
    pool: PgPool,
}

impl BrokerStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        // External agent registry
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_external_agents (
                external_agent_id       TEXT PRIMARY KEY,
                broker_tenant_id        TEXT NOT NULL,
                display_name            TEXT NOT NULL,
                callback_endpoint       TEXT,
                platform_hint           TEXT NOT NULL DEFAULT 'unknown',
                verification            JSONB NOT NULL,
                status                  TEXT NOT NULL DEFAULT 'active'
                                            CHECK (status IN ('active', 'suspended', 'revoked')),
                allowed_handshake_ids   TEXT[] NOT NULL DEFAULT '{}',
                created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                last_seen_at            TIMESTAMPTZ
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS boundary_external_agents_tenant
             ON boundary_external_agents(broker_tenant_id, status)",
        )
        .execute(&self.pool)
        .await?;

        // Brokered envelope queue: envelopes from external agents waiting to be
        // routed into the internal workflow or approval queue.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_broker_queue (
                queue_id                TEXT PRIMARY KEY,
                external_agent_id       TEXT NOT NULL,
                broker_tenant_id        TEXT NOT NULL,
                handshake_id            TEXT NOT NULL,
                envelope_id             TEXT NOT NULL UNIQUE,
                payload                 JSONB NOT NULL,
                payload_hash            TEXT NOT NULL,
                platform_metadata       JSONB NOT NULL DEFAULT '{}',
                status                  TEXT NOT NULL DEFAULT 'pending'
                                            CHECK (status IN ('pending', 'routed', 'parked_approval', 'rejected', 'responded')),
                received_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                routed_at               TIMESTAMPTZ,
                responded_at            TIMESTAMPTZ,
                expires_at              TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        // Response delivery queue: governed responses waiting to be sent back.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS boundary_broker_responses (
                response_id             TEXT PRIMARY KEY,
                queue_id                TEXT NOT NULL REFERENCES boundary_broker_queue(queue_id),
                external_agent_id       TEXT NOT NULL,
                envelope_id             TEXT NOT NULL,
                payload                 JSONB NOT NULL,
                payload_hash            TEXT NOT NULL,
                delivered               BOOLEAN NOT NULL DEFAULT FALSE,
                delivery_attempts       INTEGER NOT NULL DEFAULT 0,
                last_attempt_at         TIMESTAMPTZ,
                created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // ── External agent registration ───────────────────────────────────────────

    pub async fn register_external_agent(
        &self,
        broker_tenant_id: &str,
        display_name: &str,
        platform_hint: &str,
        callback_endpoint: Option<&str>,
        verification: &ExternalAgentVerification,
        allowed_handshake_ids: Vec<String>,
    ) -> Result<String> {
        let external_agent_id = new_id();
        let verification_json = serde_json::to_value(verification)?;

        sqlx::query(
            "INSERT INTO boundary_external_agents
             (external_agent_id, broker_tenant_id, display_name, callback_endpoint,
              platform_hint, verification, allowed_handshake_ids)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&external_agent_id)
        .bind(broker_tenant_id)
        .bind(display_name)
        .bind(callback_endpoint)
        .bind(platform_hint)
        .bind(&verification_json)
        .bind(&allowed_handshake_ids)
        .execute(&self.pool)
        .await?;

        Ok(external_agent_id)
    }

    pub async fn load_external_agent(
        &self,
        external_agent_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query_as::<_, (String, String, String, Option<String>, String, serde_json::Value, String, serde_json::Value, Option<chrono::DateTime<chrono::Utc>>)>(
            "SELECT external_agent_id, broker_tenant_id, display_name, callback_endpoint,
                    platform_hint, verification, status, '[]'::jsonb, last_seen_at
             FROM boundary_external_agents WHERE external_agent_id = $1",
        )
        .bind(external_agent_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id, tenant, name, cb, platform, verification, status, _, last_seen)| {
            serde_json::json!({
                "external_agent_id": id,
                "broker_tenant_id": tenant,
                "display_name": name,
                "callback_endpoint": cb,
                "platform_hint": platform,
                "verification": verification,
                "status": status,
                "last_seen_at": last_seen,
            })
        }))
    }

    pub async fn list_external_agents(&self, broker_tenant_id: &str) -> Result<Vec<serde_json::Value>> {
        let rows = sqlx::query(
            "SELECT external_agent_id, display_name, platform_hint, status,
                    allowed_handshake_ids, created_at, last_seen_at
             FROM boundary_external_agents WHERE broker_tenant_id = $1
             ORDER BY created_at DESC",
        )
        .bind(broker_tenant_id)
        .fetch_all(&self.pool)
        .await?;

        use sqlx::Row;
        Ok(rows.into_iter().map(|r| {
            serde_json::json!({
                "external_agent_id": r.get::<String, _>("external_agent_id"),
                "display_name": r.get::<String, _>("display_name"),
                "platform_hint": r.get::<String, _>("platform_hint"),
                "status": r.get::<String, _>("status"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                "last_seen_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_seen_at"),
            })
        }).collect())
    }

    pub async fn suspend_external_agent(&self, external_agent_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE boundary_external_agents SET status = 'suspended' WHERE external_agent_id = $1",
        )
        .bind(external_agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn revoke_external_agent(&self, external_agent_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE boundary_external_agents SET status = 'revoked' WHERE external_agent_id = $1",
        )
        .bind(external_agent_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Broker queue operations ───────────────────────────────────────────────

    /// Enqueue an inbound brokered envelope for governance processing.
    pub async fn enqueue_inbound(
        &self,
        external_agent_id: &str,
        broker_tenant_id: &str,
        handshake_id: &str,
        envelope_id: &str,
        payload: &serde_json::Value,
        payload_hash: &str,
        platform_metadata: &serde_json::Value,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<String> {
        let queue_id = new_id();
        sqlx::query(
            "INSERT INTO boundary_broker_queue
             (queue_id, external_agent_id, broker_tenant_id, handshake_id,
              envelope_id, payload, payload_hash, platform_metadata, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&queue_id)
        .bind(external_agent_id)
        .bind(broker_tenant_id)
        .bind(handshake_id)
        .bind(envelope_id)
        .bind(payload)
        .bind(payload_hash)
        .bind(platform_metadata)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(queue_id)
    }

    pub async fn mark_routed(&self, queue_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE boundary_broker_queue SET status = 'routed', routed_at = NOW()
             WHERE queue_id = $1",
        )
        .bind(queue_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_parked_approval(&self, queue_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE boundary_broker_queue SET status = 'parked_approval' WHERE queue_id = $1",
        )
        .bind(queue_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store a governed response ready to be delivered to the external agent.
    pub async fn store_response(
        &self,
        queue_id: &str,
        external_agent_id: &str,
        envelope_id: &str,
        payload: &serde_json::Value,
        payload_hash: &str,
    ) -> Result<String> {
        let response_id = new_id();
        sqlx::query(
            "INSERT INTO boundary_broker_responses
             (response_id, queue_id, external_agent_id, envelope_id, payload, payload_hash)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&response_id)
        .bind(queue_id)
        .bind(external_agent_id)
        .bind(envelope_id)
        .bind(payload)
        .bind(payload_hash)
        .execute(&self.pool)
        .await?;

        // Mark original envelope as responded
        sqlx::query(
            "UPDATE boundary_broker_queue SET status = 'responded', responded_at = NOW()
             WHERE queue_id = $1",
        )
        .bind(queue_id)
        .execute(&self.pool)
        .await?;

        Ok(response_id)
    }

    /// Poll for undelivered responses for an external agent.
    /// Called by external agents that use polling instead of webhooks.
    pub async fn poll_responses(
        &self,
        external_agent_id: &str,
        limit: i64,
    ) -> Result<Vec<serde_json::Value>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT response_id, envelope_id, payload, created_at
             FROM boundary_broker_responses
             WHERE external_agent_id = $1 AND delivered = FALSE
             ORDER BY created_at ASC LIMIT $2",
        )
        .bind(external_agent_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| {
            serde_json::json!({
                "response_id": r.get::<String, _>("response_id"),
                "envelope_id": r.get::<String, _>("envelope_id"),
                "payload": r.get::<serde_json::Value, _>("payload"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        }).collect())
    }

    pub async fn mark_response_delivered(&self, response_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE boundary_broker_responses
             SET delivered = TRUE, last_attempt_at = NOW(),
                 delivery_attempts = delivery_attempts + 1
             WHERE response_id = $1",
        )
        .bind(response_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

// ─── Broker runtime ───────────────────────────────────────────────────────────

/// Governs inbound messages from external (non-Narayan) agents.
/// Steps:
///  1. Verify signature
///  2. Validate handshake (must be BrokerScope)
///  3. Governance checks (revocation, rate limit)
///  4. Data barrier (PII scan, field redaction, residency)
///  5. Approval policy (park if required)
///  6. Write bilateral audit record
///  7. Route to internal workflow
pub struct BrokerRuntime {
    pub store: Arc<BrokerStore>,
    pub audit: Arc<BoundaryAuditStore>,
    pub governance: Arc<GovernanceStore>,
    pub http_client: reqwest::Client,
    pub broker_tenant_id: String,
}

impl BrokerRuntime {
    /// Process an inbound envelope from an external agent.
    /// Returns the queue_id of the enqueued item.
    pub async fn receive_from_external(
        &self,
        external_agent_id: &str,
        handshake_id: &str,
        envelope: &BoundaryEnvelope,
        platform_metadata: serde_json::Value,
        data_barrier: &DataBarrierPolicy,
    ) -> Result<BrokerReceiveOutcome> {
        let payload_bytes = serde_json::to_string(&envelope.payload)?.len();
        let payload_hash = hash_payload(&envelope.payload);

        // 1. Data barrier
        let violations = data_barrier.check_outbound(&envelope.payload, payload_bytes, None);
        if !violations.is_empty() {
            // Write a rejection audit record
            self.audit.append(
                &envelope.envelope_id,
                handshake_id,
                &self.broker_tenant_id,
                BoundarySide::Responder,
                BoundaryAuditEvent::DataBarrierViolation,
                serde_json::json!({ "violations": violations.iter().map(|v| v.to_string()).collect::<Vec<_>>() }),
                &payload_hash,
            ).await?;
            anyhow::bail!("data barrier rejected envelope: {} violation(s)", violations.len());
        }

        // 2. Check governance (revocation + freeze)
        let is_frozen = self.governance.is_frozen(handshake_id, &self.broker_tenant_id).await?;
        if is_frozen {
            anyhow::bail!("handshake {} is frozen — contact your broker administrator", handshake_id);
        }

        // 3. Rate limit
        let allowed = self.governance.check_and_increment_rate(handshake_id, &self.broker_tenant_id, 100, 3600).await?;
        if !allowed {
            anyhow::bail!("rate limit exceeded for handshake {}", handshake_id);
        }

        // 4. Write audit record
        self.audit.append(
            &envelope.envelope_id,
            handshake_id,
            &self.broker_tenant_id,
            BoundarySide::Responder,
            BoundaryAuditEvent::EnvelopeReceived,
            serde_json::json!({}),
            &payload_hash,
        ).await?;

        // 5. Enqueue for routing
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(3600);
        let queue_id = self.store.enqueue_inbound(
            external_agent_id,
            &self.broker_tenant_id,
            handshake_id,
            &envelope.envelope_id,
            &envelope.payload,
            &payload_hash,
            &platform_metadata,
            expires_at,
        ).await?;

        Ok(BrokerReceiveOutcome {
            queue_id,
            envelope_id: envelope.envelope_id.clone(),
            status: BrokerStatus::Queued,
        })
    }

    /// Deliver a governed response back to the external agent via webhook.
    /// Falls back to storing in the poll queue if webhook delivery fails.
    pub async fn deliver_response(
        &self,
        queue_id: &str,
        external_agent_id: &str,
        envelope_id: &str,
        payload: serde_json::Value,
        callback_endpoint: Option<&str>,
        data_barrier: &DataBarrierPolicy,
        handshake_id: &str,
    ) -> Result<()> {
        let payload_bytes = serde_json::to_string(&payload)?.len();
        let violations = data_barrier.check_outbound(&payload, payload_bytes, None);
        if !violations.is_empty() {
            anyhow::bail!("response data barrier violation(s): {}", violations.len());
        }

        let payload_hash = hash_payload(&payload);

        // Store response in DB (guaranteed delivery via polling even if webhook fails)
        let response_id = self.store.store_response(
            queue_id,
            external_agent_id,
            envelope_id,
            &payload,
            &payload_hash,
        ).await?;

        // Attempt webhook delivery if callback_endpoint is configured
        if let Some(endpoint) = callback_endpoint {
            let webhook_body = serde_json::json!({
                "response_id": response_id,
                "envelope_id": envelope_id,
                "payload": payload,
            });
            let result = self.http_client.post(endpoint)
                .json(&webhook_body)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;

            if let Ok(resp) = result {
                if resp.status().is_success() {
                    self.store.mark_response_delivered(&response_id).await?;
                    tracing::info!(
                        response_id = %response_id,
                        external_agent_id = %external_agent_id,
                        "broker response delivered via webhook"
                    );
                } else {
                    tracing::warn!(
                        response_id = %response_id,
                        status = %resp.status(),
                        "broker webhook delivery failed — response stays in poll queue"
                    );
                }
            }
        }

        // Write audit record for response sent
        self.audit.append(
            envelope_id,
            handshake_id,
            &self.broker_tenant_id,
            BoundarySide::Responder,
            BoundaryAuditEvent::ResponseSent,
            serde_json::json!({}),
            &payload_hash,
        ).await?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerReceiveOutcome {
    pub queue_id: String,
    pub envelope_id: String,
    pub status: BrokerStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerStatus {
    /// Envelope accepted and queued for routing into the internal workflow.
    Queued,
    /// Envelope parked — approval required before routing.
    ParkedForApproval,
    /// Envelope immediately rejected (governance violation).
    Rejected { reason: String },
}
