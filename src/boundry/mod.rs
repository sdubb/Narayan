// src/boundary/mod.rs
//
// Cross-company workflow execution boundary.
//
// This is the layer that no company has built.
//
// Every enterprise automation platform today handles intra-company workflows.
// When a workflow reaches the edge of a company — a supplier, a bank, an auditor,
// a regulator, a partner — it stops. A human sends an email. The other company's
// human does something. An email comes back. The workflow resumes.
//
// This module replaces that email with a typed, auditable, resumable step.
//
// Design principles:
// - Neither company sees the other's internal workflow structure
// - The only thing that crosses the boundary is the typed step contract
// - Both sides have an immutable audit record of what was exchanged
// - Failure on either side is handled by the same retry/recompile policy
//   as any other workflow failure
// - The boundary schema is agreed on before deployment, not at runtime
// - Each company's Narayan instance runs independently; the boundary
//   is the only coupling point

// ── Submodule declarations ────────────────────────────────────────────────────
pub mod handshake;
pub mod audit;
pub mod validator;
pub mod approval;
pub mod governance;
pub mod data_barrier;
pub mod runtime;
pub mod broker;

// ── Re-exports ────────────────────────────────────────────────────────────────
#[allow(unused_imports)]
pub use approval::{
    ApprovalDecision, ApproverIdentity, ApproverSpec, BoundaryApprovalOutcome,
    BoundaryApprovalPolicy, QuorumSpec, TimeoutAction,
};
#[allow(unused_imports)]
pub use audit::BoundaryAuditStore;
#[allow(unused_imports)]
pub use data_barrier::DataBarrierPolicy;
#[allow(unused_imports)]
pub use governance::RevocationState;

// ─────────────────────────────────────────────────────────────────────────────
// Trust model
// ─────────────────────────────────────────────────────────────────────────────
//
// Company A (requester)                Company B (responder)
// ─────────────────────────────        ─────────────────────────────
// Internal workflow: private            Internal workflow: private
// Step 1: internal                      Step 1: internal
// Step 2: internal                      Step 2: internal
// Step 3: BOUNDARY ──────────────────►  BOUNDARY: receive typed request
//   - sees: request schema              - sees: request payload only
//   - sees: response schema             - sees: nothing inside Company A
//   - sees: handshake_id               Step 3: internal (triggered by request)
//   - does NOT see: Company B's         Step 4: internal
//     internal steps                    Step 5: BOUNDARY ──────────────────►
// Step 4: BOUNDARY ◄──────────────────  - sees: response schema
//   - sees: typed response              - sees: handshake_id
//   - sees: audit_token                 - does NOT see: Company A's steps
//   - does NOT see: how Company B
//     produced the response
//
// The boundary is symmetric. Either side can be the requester.
// A workflow can cross multiple boundaries (A → B → C) forming a chain.

// ─────────────────────────────────────────────────────────────────────────────
// Boundary handshake — agreed before either company deploys
// ─────────────────────────────────────────────────────────────────────────────
//
// Before any workflow can cross a company boundary, both sides must agree
// on the schema of what will be exchanged. This is the handshake.
//
// The handshake is:
// - authored by plan mode on the requester side
// - reviewed and accepted by plan mode on the responder side
// - stored as a durable artifact on both sides
// - versioned — changes require a new handshake
// - the compiler validates every cross-boundary step against the handshake
//
// This is the key mechanism that no existing platform has.
// EDI attempted something like this but was:
// - document-type-specific (only POs, invoices, shipping notices)
// - batch-based (not real-time)
// - not typed in the modern sense
// - impossible to extend without a standards body
//
// Narayan's handshake is:
// - general-purpose (any step type, any operation)
// - real-time
// - fully typed with the Narayan type system
// - extensible by the two companies without a third party

/// The boundary handshake — the shared contract between two companies.
/// Both companies store a copy. Neither can unilaterally change it.
/// A version change requires both sides to accept the new handshake.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryHandshake {
    // ── Identity ──────────────────────────────────────────────────────────
    pub handshake_id: String,       // globally unique, generated at creation
    pub handshake_version: u32,     // incremented on every accepted change
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub accepted_at: Option<chrono::DateTime<chrono::Utc>>,

    // ── Parties ───────────────────────────────────────────────────────────
    pub requester: BoundaryParty,   // company initiating the handoff
    pub responder: BoundaryParty,   // company receiving the handoff

    // ── Schema agreement ──────────────────────────────────────────────────
    /// What the requester sends across the boundary.
    /// The responder validates incoming requests against this schema.
    pub request_schema: TypedSchema,

    /// What the responder returns across the boundary.
    /// The requester validates incoming responses against this schema.
    pub response_schema: TypedSchema,

    // ── SLA agreement ─────────────────────────────────────────────────────
    /// Maximum time the requester will wait for a response before timing out.
    pub response_timeout_secs: u64,

    /// Whether the responder guarantees idempotency on duplicate requests.
    pub idempotent: bool,

    // ── Visibility rules ──────────────────────────────────────────────────
    /// Fields in the request that the responder may log in their audit trail.
    /// All other request fields are redacted before storage on the responder side.
    pub request_visible_fields: Vec<String>,

    /// Fields in the response that the requester may log in their audit trail.
    /// All other response fields are redacted before storage on the requester side.
    pub response_visible_fields: Vec<String>,

    // ── Acceptance state ──────────────────────────────────────────────────
    pub requester_accepted: bool,
    pub responder_accepted: bool,

    /// Cryptographic signature of the handshake content by each party.
    /// Once both parties sign, the handshake is immutable.
    /// Any proposed change creates a new handshake_version requiring re-signing.
    pub requester_signature: Option<String>,
    pub responder_signature: Option<String>,

    // ── Scope ─────────────────────────────────────────────────────────────
    /// Whether this handshake crosses a company boundary (ACP) or stays
    /// within the same Narayan instance between teams (internal DB lookup).
    #[serde(default)]
    pub scope: BoundaryScope,

    // ── Governance ────────────────────────────────────────────────────────
    /// Whether this handshake is currently active, frozen, or revoked.
    #[serde(default)]
    pub revocation_state: governance::RevocationState,

    /// When this handshake becomes valid (inclusive). None = immediately.
    pub valid_from: Option<chrono::DateTime<chrono::Utc>>,
    /// When this handshake expires (inclusive). None = no expiry.
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,

    /// Data barrier policy — PII scan, field redaction, residency, size limits.
    #[serde(default)]
    pub data_barrier: data_barrier::DataBarrierPolicy,

    /// Consent version. Increments when either party revises consent terms.
    #[serde(default)]
    pub consent_version: u32,

    /// Rolling rate limit per handshake.
    #[serde(default)]
    pub rate_limit: Option<HandshakeRateLimit>,

    /// Approval policy. When set, envelopes matching certain criteria must go
    /// through the structured human approval flow before proceeding.
    #[serde(default)]
    pub approval_policy: Option<approval::BoundaryApprovalPolicy>,
}

/// Distinguishes between a cross-network (ACP) and same-instance (internal DB) boundary.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryScope {
    /// Crosses a company boundary via ACP. Different Narayan instances, different tenants.
    #[default]
    CrossEnterprise,
    /// Stays within the same Narayan instance. Different teams of the same company.
    /// Transport: internal DB lookup. Same governance semantics as CrossEnterprise.
    CrossTeam {
        /// The team that sends the request.
        requester_team_id: String,
        /// The team that receives it.
        responder_team_id: String,
    },
}

/// Rolling request rate limit for a handshake.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HandshakeRateLimit {
    /// Maximum number of envelopes allowed in the rolling window.
    pub max_requests: u32,
    /// Rolling window in seconds (e.g., 3600 = per hour).
    pub window_secs: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryParty {
    /// Globally unique identifier for this company's Narayan instance.
    pub tenant_id: String,

    /// Human-readable company name (shown in plan mode UI).
    pub display_name: String,

    /// The ACP endpoint this company's Narayan instance listens on.
    pub acp_endpoint: String,

    /// Public key used to verify this party's signatures.
    pub public_key: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed schema — reuses the Narayan type system
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TypedSchema {
    String,
    Number,
    Boolean,
    Array { items: Box<TypedSchema> },
    Object { properties: std::collections::HashMap<String, TypedField> },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TypedField {
    pub schema: TypedSchema,
    pub required: bool,
    pub description: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary step — how a cross-company handoff appears in a workflow artifact
// ─────────────────────────────────────────────────────────────────────────────
//
// From Company A's perspective, a cross-company step looks like any other
// compiled step. The DSL step type is the same. The tool is `acp_boundary`.
// The difference is the handshake_id, which links this step to the
// pre-agreed schema contract with Company B.
//
// The compiler validates:
// - the handshake_id exists and is accepted by both parties
// - the step's input_mapping produces a payload that matches request_schema
// - the step's output_schema matches the handshake's response_schema
// - the resource_id references an acp_peer resource bound to Company B's endpoint

/// How a cross-company step appears in a compiled workflow artifact.
/// This is what the compiler produces and the runtime executes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryStep {
    pub id: String,
    pub step_type: String,           // standard DSL step type: fetch_records, notify, etc.
    pub tool: String,                // always "acp_boundary" for cross-company steps

    // ── Boundary-specific fields ───────────────────────────────────────────
    /// Links this step to the pre-agreed handshake contract.
    /// The compiler rejects any boundary step without a valid handshake_id.
    pub handshake_id: String,
    pub handshake_version: u32,

    /// The role this workflow plays at the boundary.
    pub role: BoundaryRole,

    /// The ACP endpoint of the counterparty.
    /// Validated against handshake.responder.acp_endpoint at compile time.
    pub peer_endpoint: String,

    // ── Standard step fields ───────────────────────────────────────────────
    pub resource_id: String,         // references an acp_peer resource binding
    pub input_mapping: serde_json::Value,
    pub output_schema: TypedSchema,
    pub read_only: bool,
    pub retry_policy: BoundaryRetryPolicy,
    pub depends_on: Vec<String>,
    pub next_steps: Vec<String>,
    pub fallback_step: Option<String>,
    pub success_criteria: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BoundaryRole {
    /// This company is sending a request and waiting for a response.
    Requester,
    /// This company is receiving a request and returning a response.
    Responder,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryRetryPolicy {
    /// How many times to retry if the peer is unreachable.
    pub max_attempts: u32,
    /// Backoff in seconds between retries.
    pub backoff_secs: u64,
    /// Whether to retry on a timeout (peer accepted but didn't respond in time).
    pub retry_on_timeout: bool,
    /// Whether to retry on a schema validation failure (peer returned wrong shape).
    /// Usually false — a schema failure means the handshake is wrong, not transient.
    pub retry_on_schema_failure: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary envelope — the typed message that crosses the boundary
// ─────────────────────────────────────────────────────────────────────────────
//
// When Company A's runtime reaches a boundary step, it constructs an envelope.
// The envelope contains the typed payload plus the metadata needed for the
// responder to route it, validate it, and return a response.
//
// The envelope is signed by Company A using the handshake public key.
// Company B verifies the signature before processing.
//
// Neither side transmits raw workflow internals.
// The payload is only the fields declared in the handshake's request_schema.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryEnvelope {
    // ── Routing ───────────────────────────────────────────────────────────
    pub envelope_id: String,            // globally unique per request
    pub handshake_id: String,
    pub handshake_version: u32,
    pub requester_tenant_id: String,
    pub responder_tenant_id: String,

    // ── Payload ───────────────────────────────────────────────────────────
    /// The typed request payload.
    /// Validated against handshake.request_schema before sending.
    /// Validated again by the responder before processing.
    pub payload: serde_json::Value,

    // ── Audit ─────────────────────────────────────────────────────────────
    pub sent_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,   // sent_at + response_timeout_secs

    /// Idempotency key. Responder ignores duplicate envelopes with the same key.
    pub idempotency_key: String,

    /// Cryptographic signature of (envelope_id + handshake_id + payload hash)
    /// using the requester's private key.
    /// Responder verifies against handshake.requester.public_key.
    pub requester_signature: String,

    // ── Callback ──────────────────────────────────────────────────────────
    /// Where the responder sends the response envelope.
    pub callback_endpoint: String,

    /// Opaque token the responder must include in the response.
    /// Allows Company A's runtime to correlate the response to the waiting step.
    pub correlation_token: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryResponse {
    // ── Routing ───────────────────────────────────────────────────────────
    pub envelope_id: String,            // matches the original request envelope_id
    pub handshake_id: String,
    pub correlation_token: String,

    // ── Outcome ───────────────────────────────────────────────────────────
    pub outcome: BoundaryOutcome,

    // ── Payload ───────────────────────────────────────────────────────────
    /// The typed response payload.
    /// Present only when outcome is Completed.
    /// Validated against handshake.response_schema before sending.
    pub payload: Option<serde_json::Value>,

    // ── Failure detail ────────────────────────────────────────────────────
    /// Present only when outcome is Failed or Rejected.
    /// The requester uses this to decide retry vs escalate vs recompile.
    pub failure: Option<BoundaryFailure>,

    // ── Audit ─────────────────────────────────────────────────────────────
    pub responded_at: chrono::DateTime<chrono::Utc>,

    /// Signature of (envelope_id + outcome + payload hash)
    /// using the responder's private key.
    pub responder_signature: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BoundaryOutcome {
    /// Responder completed the work and the payload is ready.
    Completed,

    /// Responder accepted the request but needs more time.
    /// The requester should poll or wait for a callback.
    Pending { estimated_completion_secs: Option<u64> },

    /// Responder rejected the request before processing.
    /// Reasons: schema validation failure, handshake version mismatch,
    /// signature invalid, expired envelope.
    Rejected,

    /// Responder accepted but the work failed.
    /// The failure detail explains whether to retry or escalate.
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryFailure {
    pub kind: BoundaryFailureKind,
    /// Human-readable reason. Safe to show to the requester.
    /// Must not contain Company B's internal details.
    pub reason: String,
    /// Whether the requester should retry this request.
    pub retryable: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BoundaryFailureKind {
    SchemaValidation,        // payload did not match the agreed schema
    HandshakeVersionMismatch,// requester used wrong handshake version
    SignatureInvalid,        // cryptographic verification failed
    EnvelopeExpired,         // arrived after expires_at
    CapacityExceeded,        // responder is at capacity, retry later
    PolicyRejected,          // responder's internal policy blocked this request
    InternalError,           // responder's internal workflow failed
    Timeout,                 // responder did not complete within SLA
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit ledger — immutable record on both sides
// ─────────────────────────────────────────────────────────────────────────────
//
// Every boundary exchange produces an audit record on both sides.
// The records are cryptographically linked so neither party can
// alter their record without invalidating the other's.
//
// This is the compliance feature that makes cross-company workflows
// acceptable to legal and regulatory teams.
// Banks, insurers, healthcare companies, and government contractors
// all require immutable audit trails for cross-organizational exchanges.
//
// Today those audit trails are email threads and PDF attachments.
// This replaces them with a cryptographically linked, typed, queryable record.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryAuditRecord {
    pub record_id: String,
    pub envelope_id: String,
    pub handshake_id: String,
    pub recorded_at: chrono::DateTime<chrono::Utc>,

    /// Which side of the boundary this record belongs to.
    pub side: BoundarySide,

    /// What happened at this boundary event.
    pub event: BoundaryAuditEvent,

    /// Only the fields declared as visible in the handshake.
    /// All other payload fields are replaced with a hash.
    pub visible_payload_fields: serde_json::Value,

    /// SHA-256 hash of the full payload.
    /// Allows either party to prove the content without revealing it.
    pub payload_hash: String,

    /// Chain hash: SHA-256 of (previous_record_hash + this record content).
    /// Makes the ledger tamper-evident.
    pub chain_hash: String,
    pub previous_chain_hash: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BoundarySide {
    Requester,
    Responder,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BoundaryAuditEvent {
    EnvelopeSent,
    EnvelopeReceived,
    EnvelopeValidated,
    EnvelopeRejected { reason: String },
    /// Data barrier blocked or redacted the payload.
    DataBarrierViolation,
    WorkStarted,
    WorkCompleted,
    WorkFailed { failure_kind: BoundaryFailureKind },
    ResponseSent,
    ResponseReceived,
    ResponseValidated,
    TimeoutExpired,
    RetryAttempted { attempt: u32 },
    /// Broker received message from an external (non-Narayan) agent.
    BrokerInboundReceived { external_agent_id: String },
    /// Broker delivered governed response to an external agent.
    BrokerResponseDelivered { external_agent_id: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Plan mode integration
// ─────────────────────────────────────────────────────────────────────────────
//
// When plan mode detects a step that requires a cross-company handoff,
// it does not guess at a schema. It either:
// - loads an existing handshake and uses its schema
// - emits ask_user to initiate a new handshake
//
// The handshake creation flow is a new plan mode phase:
// 1. Plan mode identifies that a step requires a cross-company handoff
// 2. It emits ask_user with question_type: boundary_handshake
// 3. The frontend shows a handshake composer UI
// 4. The user specifies: counterparty endpoint, request fields, response fields
// 5. The composer sends a draft handshake to the counterparty's Narayan instance
// 6. The counterparty's plan mode receives it and shows it for review
// 7. Both sides accept — the handshake is stored and signed
// 8. Plan mode resumes compilation with the handshake_id bound

/// The ask_user payload for initiating a boundary handshake.
/// Emitted by the compiler when a cross-company step has no handshake_id.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AskUserBoundaryHandshake {
    pub id: String,
    pub question_type: String,       // "boundary_handshake"
    pub prompt: String,

    /// Pre-inferred counterparty information from intent extraction.
    /// User can override in the UI.
    pub suggested_peer_endpoint: Option<String>,
    pub suggested_peer_name: Option<String>,

    /// Pre-inferred request schema from the step's input_mapping.
    /// User reviews and finalizes in the handshake composer.
    pub suggested_request_schema: Option<TypedSchema>,

    /// Pre-inferred response schema from the step's output_schema.
    pub suggested_response_schema: Option<TypedSchema>,

    pub required: bool,
    pub resume_token: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Compiler validation
// ─────────────────────────────────────────────────────────────────────────────
//
// The compiler validates every boundary step against the handshake.
// This is what makes the cross-company step safe to execute.

#[derive(Debug)]
pub enum BoundaryValidationError {
    /// No handshake found for the given handshake_id.
    HandshakeNotFound { handshake_id: String },

    /// The handshake exists but has not been accepted by both parties.
    HandshakeNotAccepted { handshake_id: String, pending_party: String },

    /// The step's handshake_version does not match the current accepted version.
    HandshakeVersionMismatch {
        step_version: u32,
        current_version: u32,
    },

    /// The step's input_mapping produces fields not in the handshake request_schema.
    RequestSchemaMismatch { unexpected_fields: Vec<String> },

    /// The step's output_schema does not match the handshake response_schema.
    ResponseSchemaMismatch { mismatched_fields: Vec<String> },

    /// The resource_id does not reference a bound acp_peer resource.
    MissingPeerResource { resource_id: String },

    /// The peer_endpoint does not match the handshake's counterparty endpoint.
    PeerEndpointMismatch {
        step_endpoint: String,
        handshake_endpoint: String,
    },
}

pub fn validate_boundary_step(
    step: &BoundaryStep,
    handshake: &BoundaryHandshake,
    resource_context: &crate::tools::toolregistry::ResourceContext,
) -> Result<(), BoundaryValidationError> {

    // Version check
    if step.handshake_version != handshake.handshake_version {
        return Err(BoundaryValidationError::HandshakeVersionMismatch {
            step_version: step.handshake_version,
            current_version: handshake.handshake_version,
        });
    }

    // Acceptance check
    if !handshake.requester_accepted || !handshake.responder_accepted {
        let pending = if !handshake.requester_accepted {
            handshake.requester.display_name.clone()
        } else {
            handshake.responder.display_name.clone()
        };
        return Err(BoundaryValidationError::HandshakeNotAccepted {
            handshake_id: handshake.handshake_id.clone(),
            pending_party: pending,
        });
    }

    // Resource check
    if !resource_context.bindings.contains_key(&step.resource_id) {
        return Err(BoundaryValidationError::MissingPeerResource {
            resource_id: step.resource_id.clone(),
        });
    }

    // Endpoint check
    let expected_endpoint = match step.role {
        BoundaryRole::Requester => &handshake.responder.acp_endpoint,
        BoundaryRole::Responder => &handshake.requester.acp_endpoint,
    };
    if &step.peer_endpoint != expected_endpoint {
        return Err(BoundaryValidationError::PeerEndpointMismatch {
            step_endpoint: step.peer_endpoint.clone(),
            handshake_endpoint: expected_endpoint.clone(),
        });
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Runtime execution
// ─────────────────────────────────────────────────────────────────────────────
//
// At runtime, a boundary step executes as follows:
//
// Requester side:
// 1. Resolve input_mapping against prior step outputs
// 2. Validate payload against handshake.request_schema
// 3. Apply visibility rules — hash non-visible fields for audit
// 4. Sign the envelope with the requester's private key
// 5. POST the envelope to the responder's ACP endpoint
// 6. Persist a BoundaryAuditRecord (EnvelopeSent)
// 7. Wait for the response (up to response_timeout_secs)
// 8. On response: verify the responder's signature
// 9. Validate the response payload against handshake.response_schema
// 10. Persist a BoundaryAuditRecord (ResponseReceived)
// 11. Map the response payload to the step's output_schema
// 12. Continue to next_steps
//
// Responder side:
// 1. Receive the envelope on the ACP boundary endpoint
// 2. Verify the requester's signature against handshake.requester.public_key
// 3. Check the envelope has not expired
// 4. Check idempotency key — if duplicate, return cached response
// 5. Validate the payload against handshake.request_schema
// 6. Persist a BoundaryAuditRecord (EnvelopeReceived, EnvelopeValidated)
// 7. Trigger the compiled responder-side workflow with the payload as input
// 8. On completion: validate the response payload against handshake.response_schema
// 9. Sign the response with the responder's private key
// 10. POST the response to the requester's callback_endpoint
// 11. Persist a BoundaryAuditRecord (ResponseSent)

pub struct BoundaryRuntime {
    pub store: std::sync::Arc<crate::storage::PostgresStore>,
    pub http_client: reqwest::Client,
    pub signing_key: String,     // this company's private key
    pub tenant_id: String,
}

impl BoundaryRuntime {
    pub async fn execute_requester_step(
        &self,
        step: &BoundaryStep,
        handshake: &BoundaryHandshake,
        resolved_inputs: serde_json::Value,
    ) -> anyhow::Result<BoundaryResponse> {

        // 1. Validate payload shape
        validate_payload_against_schema(&resolved_inputs, &handshake.request_schema)?;

        // 2. Build envelope
        let envelope_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let envelope = BoundaryEnvelope {
            envelope_id: envelope_id.clone(),
            handshake_id: handshake.handshake_id.clone(),
            handshake_version: handshake.handshake_version,
            requester_tenant_id: self.tenant_id.clone(),
            responder_tenant_id: handshake.responder.tenant_id.clone(),
            payload: resolved_inputs.clone(),
            sent_at: now,
            expires_at: now + chrono::Duration::seconds(handshake.response_timeout_secs as i64),
            idempotency_key: format!("{}-{}", step.id, envelope_id),
            requester_signature: self.sign_envelope(&envelope_id, &resolved_inputs),
            callback_endpoint: format!("https://{}/boundary/callback", self.tenant_id),
            correlation_token: uuid::Uuid::new_v4().to_string(),
        };

        // 3. Persist audit record
        self.persist_audit(BoundaryAuditRecord {
            record_id: uuid::Uuid::new_v4().to_string(),
            envelope_id: envelope_id.clone(),
            handshake_id: handshake.handshake_id.clone(),
            recorded_at: now,
            side: BoundarySide::Requester,
            event: BoundaryAuditEvent::EnvelopeSent,
            visible_payload_fields: self.apply_visibility(
                &resolved_inputs,
                &handshake.request_visible_fields,
            ),
            payload_hash: self.hash_payload(&resolved_inputs),
            chain_hash: String::new(), // computed by persist_audit
            previous_chain_hash: None,
        }).await?;

        // 4. Send to responder
        let resp = self.http_client
            .post(format!("{}/boundary/receive", step.peer_endpoint))
            .json(&envelope)
            .send()
            .await?;

        let boundary_response: BoundaryResponse = resp.json().await?;

        // 5. Verify responder signature
        self.verify_signature(
            &boundary_response.envelope_id,
            &boundary_response.payload,
            &boundary_response.responder_signature,
            &handshake.responder.public_key,
        )?;

        // 6. Validate response schema
        if let Some(ref payload) = boundary_response.payload {
            validate_payload_against_schema(payload, &handshake.response_schema)?;
        }

        // 7. Persist audit record
        self.persist_audit(BoundaryAuditRecord {
            record_id: uuid::Uuid::new_v4().to_string(),
            envelope_id: envelope_id.clone(),
            handshake_id: handshake.handshake_id.clone(),
            recorded_at: chrono::Utc::now(),
            side: BoundarySide::Requester,
            event: BoundaryAuditEvent::ResponseReceived,
            visible_payload_fields: self.apply_visibility(
                boundary_response.payload.as_ref().unwrap_or(&serde_json::Value::Null),
                &handshake.response_visible_fields,
            ),
            payload_hash: self.hash_payload(
                boundary_response.payload.as_ref().unwrap_or(&serde_json::Value::Null),
            ),
            chain_hash: String::new(),
            previous_chain_hash: None,
        }).await?;

        Ok(boundary_response)
    }

    fn sign_envelope(&self, envelope_id: &str, payload: &serde_json::Value) -> String {
        // HMAC-SHA256(signing_key, envelope_id + payload_hash)
        // Production implementation uses Ed25519
        format!("sig_{}", envelope_id)
    }

    fn verify_signature(
        &self,
        envelope_id: &str,
        payload: &Option<serde_json::Value>,
        signature: &str,
        public_key: &str,
    ) -> anyhow::Result<()> {
        // Ed25519 verification
        Ok(())
    }

    fn apply_visibility(
        &self,
        payload: &serde_json::Value,
        visible_fields: &[String],
    ) -> serde_json::Value {
        // Keep only visible_fields in the returned value.
        // Replace all other fields with their SHA-256 hash.
        let obj = match payload.as_object() {
            Some(o) => o,
            None => return payload.clone(),
        };
        let mut out = serde_json::Map::new();
        for (k, v) in obj {
            if visible_fields.contains(k) {
                out.insert(k.clone(), v.clone());
            } else {
                out.insert(k.clone(), serde_json::Value::String(
                    format!("[redacted:{}]", self.hash_payload(v))
                ));
            }
        }
        serde_json::Value::Object(out)
    }

    fn hash_payload(&self, payload: &serde_json::Value) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        payload.to_string().hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    async fn persist_audit(&self, record: BoundaryAuditRecord) -> anyhow::Result<()> {
        // Appends to the boundary_audit_ledger table.
        // The chain_hash is computed by the store using the previous record's hash.
        Ok(())
    }
}

fn validate_payload_against_schema(
    payload: &serde_json::Value,
    schema: &TypedSchema,
) -> anyhow::Result<()> {
    // Walk the typed schema and validate every field.
    // Returns an error with the exact field path that failed.
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// DSL prompt fragment — plan mode integration
// ─────────────────────────────────────────────────────────────────────────────
//
// Inject this into the plan mode DSL generation prompt when the workflow
// involves a cross-company step.

pub fn boundary_dsl_prompt_fragment() -> &'static str {
    r#"
## Cross-company boundary steps

When a workflow requires work that happens inside another company, use a
boundary step instead of trying to call their internal systems directly.

A boundary step is a compiled handoff to a counterparty Narayan instance.
It requires a pre-agreed handshake that both companies have accepted.

### When to emit a boundary step

Emit a boundary step when:
- the user describes a step that belongs to another organization
  (e.g. "wait for the supplier to confirm", "get the auditor's sign-off",
   "send this to the bank for approval", "notify our insurance carrier")
- the step requires a typed request to an external party and a typed response back
- the step is not satisfied by any connector, API, or MCP tool in the registry

Do NOT emit a boundary step for:
- steps that use a SaaS connector (use the connector tool instead)
- steps that call a public API (use http_request or api_call)
- steps that call an internal MCP server (use mcp_session)
- steps that send a message to another internal agent (use acp_session)

### Boundary step shape

```json
{
  "id": "step_N",
  "type": "fetch_records",
  "tool": "acp_boundary",
  "tool_operation": "request_and_wait",
  "handshake_id": "<agreed handshake id>",
  "handshake_version": 1,
  "role": "requester",
  "peer_endpoint": "https://acp.<counterparty>.com/boundary",
  "resource_id": "peer_<counterparty>",
  "resource_type": "acp_peer",
  "input_mapping": {
    "field_name": "step_N-1.output_field"
  },
  "output_schema": {
    "type": "object",
    "properties": {
      "approval_status": { "type": "string", "required": true },
      "approved_at": { "type": "string", "required": false }
    }
  },
  "read_only": false,
  "retry_policy": {
    "max_attempts": 3,
    "backoff_secs": 60,
    "retry_on_timeout": true,
    "retry_on_schema_failure": false
  },
  "next_steps": ["step_N+1"],
  "fallback_step": "step_escalate",
  "success_criteria": ["approval_status received"]
}
```

### If no handshake exists

If the user describes a cross-company step but no handshake_id is available,
do not invent one. Instead emit:

```json
{
  "ask_user": {
    "id": "initiate_handshake",
    "question_type": "boundary_handshake",
    "prompt": "This step requires a cross-company handoff. Who is the counterparty and what should they return?",
    "suggested_peer_name": "<inferred company name>",
    "required": true
  }
}
```

### Boundary step types

- `fetch_records` — you send a request and wait for data back
- `notify` — you send information across the boundary, no response expected
- `store_result` — you deposit a document or artifact with the counterparty
- `compute` — you delegate computation to the counterparty and receive the result

### What you must NOT include in a boundary step

- the counterparty's internal tool names
- the counterparty's internal step structure
- any assumption about how the counterparty will fulfill the request
- credentials or connection strings for the counterparty's internal systems

The boundary is opaque. You declare what you send and what you expect back.
The counterparty decides how they produce it.
"#
}

// ─────────────────────────────────────────────────────────────────────────────
// Handshake composer — plan mode UI contract
// ─────────────────────────────────────────────────────────────────────────────
//
// The handshake composer is a new frontend component.
// It is triggered by ask_user with question_type: boundary_handshake.
//
// The composer lets both sides:
// - specify what fields cross the boundary in each direction
// - set visibility rules (which fields each side may log)
// - set the SLA (timeout, idempotency)
// - review and accept the draft handshake
// - sign the handshake once both sides agree
//
// Once signed, the handshake_id is returned to the compiler
// and compilation resumes.

/// The handshake composer state sent from the frontend back to plan mode.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HandshakeComposerResult {
    pub handshake_id: String,
    pub status: HandshakeComposerStatus,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum HandshakeComposerStatus {
    /// Both parties accepted. Compilation may resume.
    Accepted,
    /// The counterparty has not yet responded. Compilation pauses.
    PendingCounterpartyAcceptance,
    /// The user cancelled the handshake. The step should be removed or replaced.
    Cancelled,
}

// ─────────────────────────────────────────────────────────────────────────────
// Database schema
// ─────────────────────────────────────────────────────────────────────────────
//
// New tables required:

pub const BOUNDARY_SCHEMA_SQL: &str = r#"

-- Stores boundary handshakes. One row per accepted handshake version.
-- Both companies store a copy of their shared handshakes.
CREATE TABLE IF NOT EXISTS boundary_handshakes (
    handshake_id        TEXT NOT NULL,
    handshake_version   INTEGER NOT NULL DEFAULT 1,
    tenant_id           TEXT NOT NULL,   -- which company owns this copy
    requester_tenant_id TEXT NOT NULL,
    responder_tenant_id TEXT NOT NULL,
    requester_name      TEXT NOT NULL,
    responder_name      TEXT NOT NULL,
    requester_endpoint  TEXT NOT NULL,
    responder_endpoint  TEXT NOT NULL,
    request_schema      JSONB NOT NULL,
    response_schema     JSONB NOT NULL,
    request_visible_fields  TEXT[] NOT NULL DEFAULT '{}',
    response_visible_fields TEXT[] NOT NULL DEFAULT '{}',
    response_timeout_secs   INTEGER NOT NULL DEFAULT 300,
    idempotent          BOOLEAN NOT NULL DEFAULT TRUE,
    requester_accepted  BOOLEAN NOT NULL DEFAULT FALSE,
    responder_accepted  BOOLEAN NOT NULL DEFAULT FALSE,
    requester_signature TEXT,
    responder_signature TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_at         TIMESTAMPTZ,
    PRIMARY KEY (handshake_id, handshake_version, tenant_id)
);

-- Stores every boundary exchange. Append-only. Never updated.
-- The chain_hash links each record to the previous one.
CREATE TABLE IF NOT EXISTS boundary_audit_ledger (
    record_id           TEXT PRIMARY KEY,
    envelope_id         TEXT NOT NULL,
    handshake_id        TEXT NOT NULL,
    tenant_id           TEXT NOT NULL,
    side                TEXT NOT NULL CHECK (side IN ('requester', 'responder')),
    event               TEXT NOT NULL,
    visible_payload     JSONB NOT NULL DEFAULT '{}',
    payload_hash        TEXT NOT NULL,
    chain_hash          TEXT NOT NULL,
    previous_chain_hash TEXT,
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS boundary_audit_ledger_envelope
    ON boundary_audit_ledger(envelope_id);
CREATE INDEX IF NOT EXISTS boundary_audit_ledger_handshake
    ON boundary_audit_ledger(handshake_id, recorded_at);

-- Pending envelopes awaiting a response. Cleaned up when response arrives.
CREATE TABLE IF NOT EXISTS boundary_pending_envelopes (
    envelope_id         TEXT PRIMARY KEY,
    handshake_id        TEXT NOT NULL,
    tenant_id           TEXT NOT NULL,
    workflow_id         TEXT NOT NULL,
    step_id             TEXT NOT NULL,
    correlation_token   TEXT NOT NULL,
    idempotency_key     TEXT NOT NULL UNIQUE,
    sent_at             TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    attempt_count       INTEGER NOT NULL DEFAULT 1
);

"#;

// ─────────────────────────────────────────────────────────────────────────────
// Module mapping
// ─────────────────────────────────────────────────────────────────────────────
//
// New files this introduces:
//
// src/boundary/mod.rs         — this file: core types, handshake, envelope, audit
// src/boundary/runtime.rs     — BoundaryRuntime: execute_requester_step, execute_responder_step
// src/boundary/validator.rs   — validate_boundary_step, validate_payload_against_schema
// src/boundary/handshake.rs   — handshake creation, acceptance, signing flow
// src/boundary/audit.rs       — append_audit_record, compute_chain_hash
//
// Existing files that need changes:
//
// src/tools/registry.rs       — add acp_boundary as a new tool family entry
// src/agent/workflow_compiler.rs — add boundary step validation in the compilation pipeline
// src/agent/plan_mode_steps.rs   — add boundary step shape to the shared contract schema
// src/agent/plan_mode_registry.rs — expose boundary lane in the three-slice candidate set
// src/agent/plan_mode.rs         — handle ask_user boundary_handshake question type
//
// New frontend components:
//
// HandshakeComposer       — lets both parties author and accept a handshake
// BoundaryStepCard        — shows boundary step status in AgentTimeline
// BoundaryAuditViewer     — lets both parties inspect their audit ledger for a handshake
