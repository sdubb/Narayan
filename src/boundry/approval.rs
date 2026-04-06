/// Structured approval outcome — richer than binary approve/reject.
/// This is the typed value returned by an approver through the Reviews UI
/// and stored as the boundary step's output when an approval is required.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundaryApprovalOutcome {
    pub review_id: String,
    pub decision: ApprovalDecision,
    /// Human-readable notes from the approver. Stored in the audit ledger.
    pub notes: Option<String>,
    /// Conditions the requesting side must satisfy when decision is
    /// ApprovedWithConditions or PartiallyApproved.
    #[serde(default)]
    pub conditions: Vec<String>,
    /// Limits on scope, volume, or data when partially approved.
    #[serde(default)]
    pub scope_limits: Vec<ScopeLimit>,
    /// If decision is EscalatedTo, the next approver identifier.
    pub escalated_to: Option<ApproverIdentity>,
    /// If DeferredUntil, the exact time to retry.
    pub deferred_until: Option<chrono::DateTime<chrono::Utc>>,
    pub decided_at: chrono::DateTime<chrono::Utc>,
    pub decided_by: String, // tenant_id of the approver
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Full approval. Envelope proceeds as-is.
    Approved,
    /// Approved but with conditions the requester must satisfy.
    ApprovedWithConditions,
    /// Partially approved — some fields or sub-tasks approved, others not.
    PartiallyApproved,
    /// Rejected. Envelope fails. Requester receives BoundaryFailureKind::PolicyRejected.
    Rejected,
    /// Escalated to a different approver. Envelope stays parked.
    EscalatedTo,
    /// Deferred — approver wants to decide later. Envelope stays parked until the time.
    DeferredUntil,
}

/// A limit on what the approved envelope is allowed to do.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScopeLimit {
    pub field: String,
    pub operator: String, // "lte", "gte", "eq", "in"
    pub value: serde_json::Value,
}

// ── Approval Policy (on the handshake) ────────────────────────────────────────

/// Policy attached to a handshake that determines when human approval is required.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BoundaryApprovalPolicy {
    /// If set, every envelope on this handshake requires approval.
    #[serde(default)]
    pub always_require: bool,

    /// Expression-based rules — if any evaluates to true, approval is required.
    /// These are the same DSL expressions used in the AutoApprovals engine.
    /// Example: "payload.amount > 100000"
    #[serde(default)]
    pub require_when: Vec<String>,

    /// Who can approve. Must have this many approvals before proceeding.
    pub quorum: Option<QuorumSpec>,

    /// What happens if no approver acts within timeout_secs.
    pub timeout_action: TimeoutAction,

    /// How long (seconds) to wait before applying timeout_action.
    #[serde(default = "default_approval_timeout")]
    pub timeout_secs: u64,

    /// Whether a single approver can delegate to another approver.
    #[serde(default)]
    pub delegation_allowed: bool,
}

fn default_approval_timeout() -> u64 {
    172_800 // 48 hours
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuorumSpec {
    /// All approvers must approve (AND), or any one is sufficient (OR).
    pub mode: QuorumMode,
    pub approvers: Vec<ApproverSpec>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuorumMode {
    /// All listed approvers must approve.
    All,
    /// Any listed approver can approve.
    Any,
}

/// Identifies a potential approver.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApproverSpec {
    pub identity: ApproverIdentity,
    /// Optional — if set, this approver may only approve under these conditions.
    pub scope: Option<String>,
}

/// How an approver is identified.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ApproverIdentity {
    /// A specific Narayan tenant account.
    Tenant { tenant_id: String },
    /// Any admin member of a specific team.
    TeamAdmin { team_id: String },
    /// Any member of a specific team.
    TeamMember { team_id: String, min_role: String },
}

/// What happens when approval times out.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutAction {
    /// Fail the envelope with PolicyRejected (safe default).
    #[default]
    Reject,
    /// Escalate to the next approver chain if defined; otherwise reject.
    Escalate,
    /// Auto-approve (only safe for low-stakes handshakes, requires explicit opt-in).
    AutoApprove,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_decision_serde() {
        let d = ApprovalDecision::ApprovedWithConditions;
        let s = serde_json::to_string(&d).unwrap();
        let back: ApprovalDecision = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn policy_default_has_48h_timeout() {
        let p = BoundaryApprovalPolicy::default();
        assert_eq!(p.timeout_secs, 172_800);
    }
}
