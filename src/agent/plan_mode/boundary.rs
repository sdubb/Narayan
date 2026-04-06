//! Boundary handshake flow — detects, collects, and injects cross-enterprise
//! and cross-team boundary handshakes during plan mode.
//!
//! Public entry points:
//!   - `build_boundary_setup_card`  — wraps clarify::boundary_handshake_question
//!   - `planning_hint`              — static hint injected into the planner prompt
//!   - `detect_boundary_needs`      — scans intent + role to discover required handshakes
//!   - `collect_boundary_answers`   — parses user answers into a setup result
//!   - `inject_boundary_into_role`  — patches the AgentRole with the accepted boundary

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[allow(unused_imports)]
use crate::agent::definition::{AgentRole, TenantConnector};
#[allow(unused_imports)]
use crate::boundry::{AskUserBoundaryHandshake, BoundaryHandshake, TypedSchema};

use super::clarify::boundary_handshake_question;

// ─────────────────────────────────────────────────────────────────────────────
// BoundaryScope (plan-mode local enum — mirrors boundry::BoundaryScope tags
// without pulling in the team-id fields that are only known after collection)
// ─────────────────────────────────────────────────────────────────────────────

/// The scope of a detected boundary need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryScope {
    /// Crosses a company boundary via ACP.
    CrossEnterprise,
    /// Stays within the same Narayan instance, different teams.
    CrossTeam,
}

// ─────────────────────────────────────────────────────────────────────────────
// BoundaryNeed — one detected need for a handshake
// ─────────────────────────────────────────────────────────────────────────────

/// A single detected boundary handshake requirement.
///
/// Produced by `detect_boundary_needs` and consumed by the orchestrator to
/// decide which `AskUserBoundaryHandshake` cards to present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryNeed {
    /// Optional hint about the peer extracted from intent text or tool names.
    /// e.g. `"acme-supplier"` if the intent mentions `acp_session:acme-supplier`.
    pub peer_hint: Option<String>,

    /// Whether this is a cross-enterprise or cross-team boundary.
    pub scope: BoundaryScope,

    /// `true` if the boundary is required for the workflow to execute.
    /// `false` if it is advisory (the user can skip it).
    pub required: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// BoundarySetupResult — what we get after the user answers the handshake card
// ─────────────────────────────────────────────────────────────────────────────

/// The resolved outcome of a single boundary handshake collection.
///
/// Produced by `collect_boundary_answers` and consumed by
/// `inject_boundary_into_role` to patch the role before compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundarySetupResult {
    /// Unique handshake identifier (UUID v4).
    pub handshake_id: String,

    /// Handshake version (starts at 1).
    pub version: u32,

    /// The ACP endpoint of the counterparty.
    pub peer_endpoint: String,

    /// Human-readable name of the counterparty.
    pub peer_name: String,

    /// The local party's role in this handshake (`requester` or `responder`).
    pub role: String,

    /// Scope of the boundary.
    pub scope: BoundaryScope,

    /// Whether the user accepted the handshake.
    pub accepted: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Existing public helpers (carried forward from the old stub)
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a suggested cross-boundary handoff into a structured setup card.
///
/// Delegates entirely to `clarify::boundary_handshake_question` so that the
/// card structure is defined in exactly one place.
pub fn build_boundary_setup_card(
    id: impl Into<String>,
    prompt: impl Into<String>,
    suggested_peer_endpoint: Option<String>,
    suggested_peer_name: Option<String>,
    suggested_request_schema: Option<TypedSchema>,
    suggested_response_schema: Option<TypedSchema>,
    required: bool,
    resume_token: impl Into<String>,
) -> AskUserBoundaryHandshake {
    boundary_handshake_question(
        id,
        prompt,
        suggested_peer_endpoint,
        suggested_peer_name,
        suggested_request_schema,
        suggested_response_schema,
        required,
        resume_token,
    )
}

/// Static planning hint injected into the planner system prompt so the LLM
/// knows boundary handshakes are a first-class concept.
pub fn planning_hint() -> &'static str {
    "Boundary handshakes are first-class: every cross-company or cross-team \
     handoff must have an accepted handshake before execution."
}

// ─────────────────────────────────────────────────────────────────────────────
// detect_boundary_needs
// ─────────────────────────────────────────────────────────────────────────────

/// Keyword patterns that signal a cross-enterprise boundary in natural language.
const CROSS_ENTERPRISE_TERMS: &[&str] = &[
    "cross-enterprise",
    "external organization",
    "partner",
    "vendor",
    "third-party agent",
    "third party agent",
    "supplier",
    "external company",
    "cross-company",
    "inter-company",
    "cross company",
];

/// Keyword patterns that signal a cross-team boundary (same org, different team).
const CROSS_TEAM_TERMS: &[&str] = &[
    "cross-team",
    "cross team",
    "other team",
    "another team",
    "different team",
    "sister team",
    "internal handoff",
];

/// Examine the intent text, role tools, and role connectors to determine
/// whether this workflow needs one or more boundary handshakes.
///
/// # Detection heuristics (evaluated in order)
///
/// 1. **Explicit ACP session tools** — any tool starting with `acp_session:`
///    implies a known cross-enterprise peer. The suffix after the colon is
///    used as `peer_hint`.
///
/// 2. **ACP / agent connectors** — if a connector's name contains `"acp"` or
///    `"agent"` it likely represents a boundary integration. Marked required
///    because the connector was explicitly configured.
///
/// 3. **Natural-language intent scanning** — the raw intent string is
///    checked for boundary-related terms (see `CROSS_ENTERPRISE_TERMS` and
///    `CROSS_TEAM_TERMS`). These needs are marked advisory (`required = false`)
///    because the user has not yet confirmed.
pub(super) fn detect_boundary_needs(
    intent_text: &str,
    role: &AgentRole,
) -> Vec<BoundaryNeed> {
    let mut needs: Vec<BoundaryNeed> = Vec::new();

    // ── 1. Scan tools for explicit acp_session:{peer} entries ────────────
    for tool in &role.tools {
        if let Some(peer) = tool.strip_prefix("acp_session:") {
            let peer = peer.trim();
            if !peer.is_empty() {
                // Avoid duplicates if the same peer appears more than once.
                let already = needs.iter().any(|n| {
                    n.peer_hint.as_deref() == Some(peer)
                        && n.scope == BoundaryScope::CrossEnterprise
                });
                if !already {
                    needs.push(BoundaryNeed {
                        peer_hint: Some(peer.to_owned()),
                        scope: BoundaryScope::CrossEnterprise,
                        required: true,
                    });
                }
            }
        }
    }

    // ── 2. Scan connectors for ACP / agent integration names ─────────────
    for connector_name in &role.connectors {
        let lower = connector_name.to_lowercase();
        if lower.contains("acp") || lower.contains("agent") {
            let already = needs.iter().any(|n| {
                n.peer_hint.as_deref() == Some(connector_name.as_str())
            });
            if !already {
                needs.push(BoundaryNeed {
                    peer_hint: Some(connector_name.clone()),
                    scope: BoundaryScope::CrossEnterprise,
                    required: true,
                });
            }
        }
    }

    // ── 3. Scan intent text for boundary-related natural-language terms ──
    let lower_intent = intent_text.to_lowercase();

    // Cross-enterprise terms
    for term in CROSS_ENTERPRISE_TERMS {
        if lower_intent.contains(term) {
            // Try to extract a peer hint from the text near the term.
            let peer_hint = extract_peer_hint_near_term(&lower_intent, term);
            let already = needs.iter().any(|n| {
                n.scope == BoundaryScope::CrossEnterprise
                    && n.peer_hint == peer_hint
            });
            if !already {
                needs.push(BoundaryNeed {
                    peer_hint,
                    scope: BoundaryScope::CrossEnterprise,
                    required: false,
                });
            }
            // One match per scope category is enough from NL scanning.
            break;
        }
    }

    // Cross-team terms
    for term in CROSS_TEAM_TERMS {
        if lower_intent.contains(term) {
            let peer_hint = extract_peer_hint_near_term(&lower_intent, term);
            let already = needs.iter().any(|n| {
                n.scope == BoundaryScope::CrossTeam
                    && n.peer_hint == peer_hint
            });
            if !already {
                needs.push(BoundaryNeed {
                    peer_hint,
                    scope: BoundaryScope::CrossTeam,
                    required: false,
                });
            }
            break;
        }
    }

    needs
}

/// Best-effort extraction of a quoted or parenthesised peer name near a
/// boundary keyword.  Returns `None` if no obvious candidate is found.
///
/// Examples:
///   `cross-enterprise partner "Acme Corp"` → Some("acme corp")
///   `send to vendor (SupplierX)`           → Some("supplierx")
fn extract_peer_hint_near_term(text: &str, _term: &str) -> Option<String> {
    // Strategy: look for the first quoted string or parenthesised token in
    // the whole intent.  A more sophisticated approach would look near the
    // term, but for plan-mode this is sufficient — the user will confirm.

    // Try double-quoted
    if let Some(start) = text.find('"') {
        if let Some(end) = text[start + 1..].find('"') {
            let candidate = &text[start + 1..start + 1 + end];
            if !candidate.is_empty() {
                return Some(candidate.to_owned());
            }
        }
    }

    // Try parenthesised
    if let Some(start) = text.find('(') {
        if let Some(end) = text[start + 1..].find(')') {
            let candidate = &text[start + 1..start + 1 + end];
            let trimmed = candidate.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }

    None
}

// ─────────────────────────────────────────────────────────────────────────────
// collect_boundary_answers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the user's answer to a boundary handshake card into a
/// `BoundarySetupResult`.
///
/// # Expected answer format
///
/// The UI submits a JSON-like answer with the following fields:
///
/// ```text
/// {
///   "peer_endpoint": "https://acp.acme.com/v1",
///   "peer_name": "Acme Corp",
///   "role": "requester",        // or "responder"
///   "accepted": true
/// }
/// ```
///
/// If the answer is a bare string, we attempt a best-effort parse:
/// - `"accept"` / `"yes"` → accepted with the suggested values from the need
/// - `"reject"` / `"skip"` / `"no"` → not accepted
///
/// # Errors
///
/// Returns `None` if the answer cannot be parsed at all.  Callers should
/// re-prompt the user in that case.
pub(super) fn collect_boundary_answers(
    answer: &str,
    boundary_need: &BoundaryNeed,
) -> Option<BoundarySetupResult> {
    let trimmed = answer.trim();

    // ── Try JSON parse first ─────────────────────────────────────────────
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let peer_endpoint = parsed
            .get("peer_endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let peer_name = parsed
            .get("peer_name")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                boundary_need
                    .peer_hint
                    .as_deref()
                    .unwrap_or("unknown-peer")
            })
            .to_owned();
        let role = parsed
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("requester")
            .to_owned();
        let accepted = parsed
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        return Some(BoundarySetupResult {
            handshake_id: Uuid::new_v4().to_string(),
            version: 1,
            peer_endpoint,
            peer_name,
            role,
            scope: boundary_need.scope.clone(),
            accepted,
        });
    }

    // ── Bare-string fallback ─────────────────────────────────────────────
    let lower = trimmed.to_lowercase();
    if lower == "accept" || lower == "yes" || lower == "y" {
        let peer_name = boundary_need
            .peer_hint
            .clone()
            .unwrap_or_else(|| "unknown-peer".to_owned());
        return Some(BoundarySetupResult {
            handshake_id: Uuid::new_v4().to_string(),
            version: 1,
            peer_endpoint: String::new(),
            peer_name,
            role: "requester".to_owned(),
            scope: boundary_need.scope.clone(),
            accepted: true,
        });
    }

    if lower == "reject" || lower == "skip" || lower == "no" || lower == "n" {
        let peer_name = boundary_need
            .peer_hint
            .clone()
            .unwrap_or_else(|| "unknown-peer".to_owned());
        return Some(BoundarySetupResult {
            handshake_id: Uuid::new_v4().to_string(),
            version: 1,
            peer_endpoint: String::new(),
            peer_name,
            role: "requester".to_owned(),
            scope: boundary_need.scope.clone(),
            accepted: false,
        });
    }

    // Cannot interpret the answer.
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// inject_boundary_into_role
// ─────────────────────────────────────────────────────────────────────────────

/// Patch an `AgentRole` with the accepted boundary handshake result.
///
/// Mutations performed:
///
/// 1. **Tool injection** — adds `acp_session:{peer_name}` to `role.tools`
///    unless it is already present.  This ensures the compiled workflow can
///    reference the tool that invokes the boundary peer.
///
/// 2. **Connector injection** — adds a connector name derived from the peer
///    (`"acp_{peer_name}"`, normalised) to `role.connectors` unless it is
///    already present.  The runtime connector resolver uses this entry to
///    locate the physical ACP transport config.
///
/// If the setup result has `accepted == false`, no mutations are performed
/// and the function returns early.
pub(super) fn inject_boundary_into_role(
    result: &BoundarySetupResult,
    role: &mut AgentRole,
) {
    if !result.accepted {
        return;
    }

    // ── 1. Tool injection ────────────────────────────────────────────────
    let tool_name = format!("acp_session:{}", result.peer_name);
    if !role.tools.iter().any(|t| t == &tool_name) {
        role.tools.push(tool_name);
    }

    // ── 2. Connector injection ───────────────────────────────────────────
    let connector_name = normalise_connector_name(&result.peer_name);
    if !role.connectors.iter().any(|c| c == &connector_name) {
        role.connectors.push(connector_name);
    }
}

/// Normalise a peer name into a valid connector identifier.
///
/// Rules:
/// - lowercase
/// - non-alphanumeric characters replaced with `_`
/// - leading/trailing underscores stripped
/// - prefixed with `acp_`
///
/// Example: `"Acme Corp"` → `"acp_acme_corp"`
fn normalise_connector_name(peer_name: &str) -> String {
    let slug: String = peer_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let slug = slug.trim_matches('_');
    format!("acp_{slug}")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::AgentRole;

    /// Helper to create a minimal AgentRole for testing.
    fn test_role(tools: Vec<&str>, connectors: Vec<&str>) -> AgentRole {
        AgentRole {
            id: "role-1".into(),
            agent_id: "agent-1".into(),
            tenant_id: "tenant-1".into(),
            version: 1,
            status: crate::agent::definition::RoleStatus::Draft,
            name: "test-role".into(),
            trigger: Default::default(),
            purpose: "test".into(),
            role_category: Default::default(),
            execution_guidelines: Default::default(),
            connectors: connectors.into_iter().map(String::from).collect(),
            tools: tools.into_iter().map(String::from).collect(),
            output_spec: Default::default(),
            memory_scope: Default::default(),
            execution_limits: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // ── detect_boundary_needs ────────────────────────────────────────────

    #[test]
    fn detect_acp_session_tool() {
        let role = test_role(vec!["acp_session:acme-supplier"], vec![]);
        let needs = detect_boundary_needs("some intent", &role);
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].peer_hint.as_deref(), Some("acme-supplier"));
        assert_eq!(needs[0].scope, BoundaryScope::CrossEnterprise);
        assert!(needs[0].required);
    }

    #[test]
    fn detect_acp_connector() {
        let role = test_role(vec![], vec!["acp_partner_gateway"]);
        let needs = detect_boundary_needs("some intent", &role);
        assert_eq!(needs.len(), 1);
        assert_eq!(
            needs[0].peer_hint.as_deref(),
            Some("acp_partner_gateway")
        );
        assert!(needs[0].required);
    }

    #[test]
    fn detect_agent_connector() {
        let role = test_role(vec![], vec!["external_agent_bridge"]);
        let needs = detect_boundary_needs("some intent", &role);
        assert_eq!(needs.len(), 1);
        assert!(needs[0].required);
    }

    #[test]
    fn detect_cross_enterprise_from_intent() {
        let role = test_role(vec![], vec![]);
        let needs = detect_boundary_needs(
            "Send PO to our cross-enterprise partner for approval",
            &role,
        );
        assert!(needs.iter().any(|n| n.scope == BoundaryScope::CrossEnterprise));
        assert!(needs.iter().all(|n| !n.required));
    }

    #[test]
    fn detect_cross_team_from_intent() {
        let role = test_role(vec![], vec![]);
        let needs = detect_boundary_needs(
            "Hand off the enriched lead to the cross-team sales pipeline",
            &role,
        );
        assert!(needs.iter().any(|n| n.scope == BoundaryScope::CrossTeam));
    }

    #[test]
    fn detect_vendor_from_intent() {
        let role = test_role(vec![], vec![]);
        let needs = detect_boundary_needs(
            "Ask the vendor to validate the invoice",
            &role,
        );
        assert!(needs.iter().any(|n| n.scope == BoundaryScope::CrossEnterprise));
    }

    #[test]
    fn detect_no_boundary() {
        let role = test_role(vec!["web_search"], vec!["salesforce"]);
        let needs = detect_boundary_needs(
            "Enrich inbound leads and update CRM",
            &role,
        );
        assert!(needs.is_empty());
    }

    #[test]
    fn detect_deduplicates_same_peer() {
        let role = test_role(
            vec!["acp_session:acme", "acp_session:acme"],
            vec![],
        );
        let needs = detect_boundary_needs("intent", &role);
        assert_eq!(needs.len(), 1);
    }

    // ── collect_boundary_answers ─────────────────────────────────────────

    #[test]
    fn collect_json_answer() {
        let need = BoundaryNeed {
            peer_hint: Some("acme".into()),
            scope: BoundaryScope::CrossEnterprise,
            required: true,
        };
        let answer = r#"{
            "peer_endpoint": "https://acp.acme.com/v1",
            "peer_name": "Acme Corp",
            "role": "requester",
            "accepted": true
        }"#;
        let result = collect_boundary_answers(answer, &need).unwrap();
        assert!(result.accepted);
        assert_eq!(result.peer_name, "Acme Corp");
        assert_eq!(result.peer_endpoint, "https://acp.acme.com/v1");
        assert_eq!(result.role, "requester");
        assert_eq!(result.scope, BoundaryScope::CrossEnterprise);
        assert_eq!(result.version, 1);
        assert!(!result.handshake_id.is_empty());
    }

    #[test]
    fn collect_bare_accept() {
        let need = BoundaryNeed {
            peer_hint: Some("acme".into()),
            scope: BoundaryScope::CrossEnterprise,
            required: true,
        };
        let result = collect_boundary_answers("accept", &need).unwrap();
        assert!(result.accepted);
        assert_eq!(result.peer_name, "acme");
    }

    #[test]
    fn collect_bare_reject() {
        let need = BoundaryNeed {
            peer_hint: None,
            scope: BoundaryScope::CrossTeam,
            required: false,
        };
        let result = collect_boundary_answers("skip", &need).unwrap();
        assert!(!result.accepted);
    }

    #[test]
    fn collect_unparseable_returns_none() {
        let need = BoundaryNeed {
            peer_hint: None,
            scope: BoundaryScope::CrossEnterprise,
            required: false,
        };
        assert!(collect_boundary_answers("gibberish foo bar", &need).is_none());
    }

    // ── inject_boundary_into_role ────────────────────────────────────────

    #[test]
    fn inject_adds_tool_and_connector() {
        let mut role = test_role(vec![], vec![]);
        let result = BoundarySetupResult {
            handshake_id: "hs-1".into(),
            version: 1,
            peer_endpoint: "https://acp.acme.com".into(),
            peer_name: "Acme Corp".into(),
            role: "requester".into(),
            scope: BoundaryScope::CrossEnterprise,
            accepted: true,
        };
        inject_boundary_into_role(&result, &mut role);
        assert!(role.tools.contains(&"acp_session:Acme Corp".to_owned()));
        assert!(role.connectors.contains(&"acp_acme_corp".to_owned()));
    }

    #[test]
    fn inject_skips_when_not_accepted() {
        let mut role = test_role(vec![], vec![]);
        let result = BoundarySetupResult {
            handshake_id: "hs-1".into(),
            version: 1,
            peer_endpoint: String::new(),
            peer_name: "Acme Corp".into(),
            role: "requester".into(),
            scope: BoundaryScope::CrossEnterprise,
            accepted: false,
        };
        inject_boundary_into_role(&result, &mut role);
        assert!(role.tools.is_empty());
        assert!(role.connectors.is_empty());
    }

    #[test]
    fn inject_does_not_duplicate() {
        let mut role = test_role(
            vec!["acp_session:Acme Corp"],
            vec!["acp_acme_corp"],
        );
        let result = BoundarySetupResult {
            handshake_id: "hs-1".into(),
            version: 1,
            peer_endpoint: "https://acp.acme.com".into(),
            peer_name: "Acme Corp".into(),
            role: "requester".into(),
            scope: BoundaryScope::CrossEnterprise,
            accepted: true,
        };
        inject_boundary_into_role(&result, &mut role);
        assert_eq!(role.tools.len(), 1);
        assert_eq!(role.connectors.len(), 1);
    }

    // ── normalise_connector_name ─────────────────────────────────────────

    #[test]
    fn normalise_basic() {
        assert_eq!(normalise_connector_name("Acme Corp"), "acp_acme_corp");
    }

    #[test]
    fn normalise_already_clean() {
        assert_eq!(normalise_connector_name("supplier"), "acp_supplier");
    }

    #[test]
    fn normalise_strips_edges() {
        assert_eq!(normalise_connector_name("--vendor--"), "acp_vendor");
    }

    // ── planning_hint ────────────────────────────────────────────────────

    #[test]
    fn hint_is_nonempty() {
        assert!(!planning_hint().is_empty());
    }
}
