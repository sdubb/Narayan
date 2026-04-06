//! Review module — replaces lightweight workflow_hints with full WorkflowContract.
//!
//! The contract is the single artefact that the orchestrator, compiler, and
//! runtime all agree on before an agent draft is saved.  It captures:
//!   - every workflow step (extracted from the intent DSL),
//!   - the compiler's current validation state,
//!   - boundary requirements (tools & connectors the role references),
//!   - subsystem requirements (which AGENT_SUBSYSTEMS are in play),
//!   - an explicit approval status,
//!   - governance checks (PII redaction, data barrier, approval policy).
//!
//! `plan_mode_scaffold_specs` now builds on `build_workflow_contract` instead of
//! the old `workflow_hints_for_compilation`, giving downstream stages a typed,
//! auditable object rather than a bag of hint strings.

use serde::{Deserialize, Serialize};

use crate::{
    agent::{
        definition::{
            AgentDefinition, AgentRole, ExecutionStrategy, PlanModePhase, PlanModeSession, ToolPool,
        },
        planner::AdaptiveResearchMemo,
    },
    state::{SessionTask, SessionTaskOutput, SessionTaskResultStatus, SessionTaskStatus},
};

use super::intent::AGENT_SUBSYSTEMS;
use super::subsystems::SubsystemPolicy;

// ── Contract types ───────────────────────────────────────────────────────────

/// Full typed contract that replaces the old `Vec<String>` workflow hints.
/// Consumed by the compiler, the review UI, and the approval gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowContract {
    /// Ordered steps extracted from the intent `workflow_dsl`.
    pub steps: Vec<WorkflowContractStep>,

    /// Current state of the compiler validation pipeline.
    pub compiler_validation: CompilerValidationState,

    /// Boundary requirements derived from role tools/connectors
    /// (e.g. "salesforce connector required", "web_search tool required").
    pub boundary_requirements: Vec<String>,

    /// Subsystem requirements — which of the canonical AGENT_SUBSYSTEMS are
    /// referenced or needed by the draft.
    pub subsystem_requirements: Vec<String>,

    /// Whether the contract has been approved, is pending, or was rejected.
    pub approval_status: ApprovalStatus,

    /// Governance checks that must pass before the contract is approved.
    pub governance_checks: Vec<GovernanceCheck>,
}

/// One step in the workflow contract — mirrors a `workflow_dsl` entry but with
/// all optional fields surfaced so the compiler can validate them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowContractStep {
    /// Stable step id (from the DSL `id` field or a generated fallback).
    pub id: String,

    /// Human-readable description of what this step does.
    pub description: String,

    /// Exact tool name, if one was resolved during intent extraction.
    pub tool: Option<String>,

    /// Hint for tool selection when `tool` is not yet pinned.
    pub tool_hint: Option<String>,

    /// Resource type the step operates on (e.g. "salesforce_lead", "csv_row").
    pub resource_type: Option<String>,
}

/// Snapshot of where the compiler pipeline stands for this draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerValidationState {
    /// Current compiler stage label (mirrors `PlanModeCompilerStage` as a string).
    pub stage: String,

    /// How many repair passes have been attempted so far.
    pub repair_passes: u32,

    /// Outstanding issues or blockers the compiler has flagged.
    pub issues: Vec<String>,

    /// `true` when the compiler considers the draft valid for execution.
    pub is_valid: bool,
}

/// Approval gate state.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    #[default]
    Pending,
    Approved,
    Rejected,
}

/// A single governance check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceCheck {
    /// Short machine-friendly name (e.g. "pii_redaction", "data_barrier").
    pub name: String,

    /// Whether this check passed.
    pub passed: bool,

    /// Optional human-readable detail on why the check passed or failed.
    pub detail: Option<String>,
}

/// One row in the review checklist returned by `review_checklist`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewChecklistItem {
    /// Short label for the checklist row.
    pub label: String,

    /// Whether this row passes.
    pub passed: bool,

    /// Optional detail on the check outcome.
    pub detail: Option<String>,
}

// ── Existing helpers (kept) ──────────────────────────────────────────────────

/// Static section titles used by the review UI.
pub fn review_section_titles() -> &'static [&'static str] {
    &[
        "purpose",
        "tools",
        "connectors",
        "boundary",
        "memory",
        "storage",
        "workspace",
        "scheduler",
        "skills",
    ]
}

/// Short hint string shown during the review phase.
pub fn review_hint(session: &PlanModeSession) -> String {
    format!(
        "Review the compiled agent draft for {} with {} pending clarification step(s).",
        session.draft_agent.name,
        session.pending_steps.len()
    )
}

/// Apply role-category-aware defaults to the agent definition and role.
pub fn apply_role_policy_defaults(agent: &mut AgentDefinition, role: &mut AgentRole) {
    if agent.persona.trim().is_empty() {
        agent.persona = role.role_category.default_persona().to_string();
    }

    role.memory_scope = role.role_category.default_memory_scope();

    if role.execution_limits == crate::agent::definition::ExecutionLimits::default() {
        role.execution_limits = role.role_category.default_execution_limits();
    }

    role.execution_guidelines.permission_mode = role.role_category.default_permission_mode();
    if matches!(
        role.role_category,
        crate::agent::definition::RoleCategory::SoftwareEngineer
    ) {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::AdaptivePlanning;
        role.execution_guidelines.tool_pool = ToolPool::Worker;
    } else if matches!(
        role.role_category,
        crate::agent::definition::RoleCategory::ResearchAnalyst
    ) {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::CoordinatorShell;
        role.execution_guidelines.tool_pool = ToolPool::Coordinator;
    } else if role.execution_guidelines.execution_strategy == ExecutionStrategy::AdaptivePlanning {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::DeterministicWorkflow;
    }

    if matches!(
        role.execution_guidelines.execution_strategy,
        ExecutionStrategy::CoordinatorShell
    ) && matches!(role.execution_guidelines.tool_pool, ToolPool::Worker)
    {
        role.execution_guidelines.tool_pool = ToolPool::Coordinator;
    }
}

/// Convert AdaptivePlanning → DeterministicWorkflow on final save.
pub fn finalize_saved_role_execution_strategy(role: &mut AgentRole) {
    if matches!(
        role.execution_guidelines.execution_strategy,
        ExecutionStrategy::AdaptivePlanning
    ) {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::DeterministicWorkflow;
    }
}

// ── Phase rank (kept) ────────────────────────────────────────────────────────

pub(super) fn phase_rank(phase: &PlanModePhase) -> u8 {
    match phase {
        PlanModePhase::CapturingIntent => 0,
        PlanModePhase::ResolvingConnectors => 1,
        PlanModePhase::CapturingClarifications => 2,
        PlanModePhase::CapturingConstraints => 3,
        PlanModePhase::Reviewing => 4,
        PlanModePhase::Complete => 5,
    }
}

// ── build_workflow_contract ──────────────────────────────────────────────────

/// Build a full `WorkflowContract` from the session's intent, draft role, and
/// current compiler state.  This is the replacement for the old
/// `workflow_hints_for_compilation` — instead of a flat `Vec<String>` the
/// caller gets a typed, auditable contract.
pub fn build_workflow_contract(
    intent: &serde_json::Value,
    session: &PlanModeSession,
    role: Option<&AgentRole>,
) -> WorkflowContract {
    // ── 1. Extract steps from workflow_dsl ──────────────────────────────
    let steps = extract_contract_steps(intent);

    // ── 2. Populate compiler validation from session ────────────────────
    let compiler_validation = CompilerValidationState {
        stage: format!("{:?}", session.compiler_stage),
        repair_passes: session.compiler_repair_passes as u32,
        issues: session.compiler_validation_issues.clone(),
        is_valid: session.compiler_validation_issues.is_empty()
            && matches!(
                session.compiler_stage,
                crate::agent::definition::PlanModeCompilerStage::Review
                    | crate::agent::definition::PlanModeCompilerStage::Bind
            ),
    };

    // ── 3. Boundary requirements from role tools/connectors ─────────────
    let boundary_requirements = build_boundary_requirements(role);

    // ── 4. Subsystem requirements from AGENT_SUBSYSTEMS ─────────────────
    let subsystem_requirements = build_subsystem_requirements(intent, role);

    // ── 5. Approval status based on phase ───────────────────────────────
    let approval_status = match session.phase {
        PlanModePhase::Complete => ApprovalStatus::Approved,
        PlanModePhase::Reviewing => ApprovalStatus::Pending,
        _ => ApprovalStatus::Pending,
    };

    // ── 6. Governance checks ────────────────────────────────────────────
    let governance_checks = run_governance_checks(intent, session, role);

    WorkflowContract {
        steps,
        compiler_validation,
        boundary_requirements,
        subsystem_requirements,
        approval_status,
        governance_checks,
    }
}

/// Extract `WorkflowContractStep`s from the intent's `workflow_dsl` array.
fn extract_contract_steps(intent: &serde_json::Value) -> Vec<WorkflowContractStep> {
    let mut steps = Vec::new();

    if let Some(dsl_steps) = intent["workflow_dsl"].as_array() {
        for (idx, step) in dsl_steps.iter().enumerate() {
            let id = step["id"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("step_{}", idx));

            let description = step["description"]
                .as_str()
                .unwrap_or("(no description)")
                .to_string();

            let tool = step["tool"].as_str().map(String::from);
            let tool_hint = step["tool_hint"].as_str().map(String::from);
            let resource_type = step["resource_type"].as_str().map(String::from);

            steps.push(WorkflowContractStep {
                id,
                description,
                tool,
                tool_hint,
                resource_type,
            });
        }
    }

    // Also fold in hints from the adaptive research memo if present.
    if let Some(memo) = intent
        .get("_adaptive_research_memo")
        .and_then(|v| serde_json::from_value::<AdaptiveResearchMemo>(v.clone()).ok())
    {
        for (idx, hint) in memo.workflow_hints.iter().enumerate() {
            // Only add memo hints that are not already covered by a step description.
            let already_covered = steps.iter().any(|s| s.description == *hint);
            if !already_covered {
                steps.push(WorkflowContractStep {
                    id: format!("memo_{}", idx),
                    description: hint.clone(),
                    tool: None,
                    tool_hint: None,
                    resource_type: None,
                });
            }
        }
    }

    steps
}

/// Derive boundary requirements from the role's declared tools and connectors.
fn build_boundary_requirements(role: Option<&AgentRole>) -> Vec<String> {
    let mut reqs = Vec::new();

    if let Some(role) = role {
        for connector in &role.connectors {
            reqs.push(format!("{} connector required", connector));
        }
        for tool in &role.tools {
            reqs.push(format!("{} tool required", tool));
        }
    }

    reqs.sort();
    reqs.dedup();
    reqs
}

/// Determine which AGENT_SUBSYSTEMS the draft references.
///
/// We scan the intent text, role tools, and role connectors for mentions of
/// each subsystem name.  Any subsystem that appears is listed as a requirement.
fn build_subsystem_requirements(
    intent: &serde_json::Value,
    role: Option<&AgentRole>,
) -> Vec<String> {
    let intent_text = intent.to_string().to_lowercase();

    let role_text = role
        .map(|r| {
            let mut buf = r.purpose.clone();
            for c in &r.connectors {
                buf.push(' ');
                buf.push_str(c);
            }
            for t in &r.tools {
                buf.push(' ');
                buf.push_str(t);
            }
            buf.to_lowercase()
        })
        .unwrap_or_default();

    let mut reqs = Vec::new();
    for &subsystem in AGENT_SUBSYSTEMS {
        if intent_text.contains(subsystem) || role_text.contains(subsystem) {
            reqs.push(subsystem.to_string());
        }
    }
    reqs
}

/// Run the canonical governance checks against the current draft.
///
/// Three checks are always run:
///   1. **PII redaction** — flags if the intent references PII-like data
///      without a redaction or anonymisation step.
///   2. **Data barrier** — flags if the workflow reads AND writes to different
///      external systems without an explicit data-barrier step.
///   3. **Approval policy** — ensures the role's permission mode is not
///      `TrustedAuto` unless the phase has reached Reviewing.
fn run_governance_checks(
    intent: &serde_json::Value,
    session: &PlanModeSession,
    role: Option<&AgentRole>,
) -> Vec<GovernanceCheck> {
    let mut checks = Vec::new();

    // ── PII redaction check ─────────────────────────────────────────────
    let intent_lower = intent.to_string().to_lowercase();
    let pii_terms = [
        "email",
        "phone",
        "address",
        "social security",
        "ssn",
        "date of birth",
        "dob",
        "credit card",
        "passport",
    ];
    let mentions_pii = pii_terms.iter().any(|term| intent_lower.contains(term));
    let has_redaction_step = intent_lower.contains("redact")
        || intent_lower.contains("anonymi")
        || intent_lower.contains("mask")
        || intent_lower.contains("pii_filter");
    checks.push(GovernanceCheck {
        name: "pii_redaction".to_string(),
        passed: !mentions_pii || has_redaction_step,
        detail: if mentions_pii && !has_redaction_step {
            Some(
                "Workflow references PII-like data but no redaction or anonymisation step was found."
                    .to_string(),
            )
        } else {
            None
        },
    });

    // ── Data barrier check ──────────────────────────────────────────────
    let data_sources = intent["data_sources"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let write_targets = intent["write_targets"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let has_data_barrier_step = intent_lower.contains("data_barrier")
        || intent_lower.contains("data barrier")
        || intent_lower.contains("transform_gate");
    let cross_system = data_sources > 0 && write_targets > 0;
    checks.push(GovernanceCheck {
        name: "data_barrier".to_string(),
        passed: !cross_system || has_data_barrier_step,
        detail: if cross_system && !has_data_barrier_step {
            Some(
                "Workflow reads from external sources and writes to external targets without an explicit data barrier step."
                    .to_string(),
            )
        } else {
            None
        },
    });

    // ── Approval policy check ───────────────────────────────────────────
    let permission_mode = role
        .map(|r| r.execution_guidelines.permission_mode)
        .unwrap_or_default();
    let is_trusted_auto = matches!(
        permission_mode,
        crate::agent::definition::PermissionMode::TrustedAuto
    );
    let review_reached = phase_rank(&session.phase) >= phase_rank(&PlanModePhase::Reviewing);
    checks.push(GovernanceCheck {
        name: "approval_policy".to_string(),
        passed: !is_trusted_auto || review_reached,
        detail: if is_trusted_auto && !review_reached {
            Some(
                "Permission mode is TrustedAuto but the review phase has not been reached yet."
                    .to_string(),
            )
        } else {
            None
        },
    });

    checks
}

// ── review_checklist ─────────────────────────────────────────────────────────

/// Build a review checklist covering the key gates before a draft can be
/// approved and saved.
pub fn review_checklist(contract: &WorkflowContract) -> Vec<ReviewChecklistItem> {
    let mut items = Vec::new();

    // 1. Compiler draft compiled?
    items.push(ReviewChecklistItem {
        label: "Compiler draft compiled".to_string(),
        passed: contract.compiler_validation.is_valid,
        detail: if contract.compiler_validation.is_valid {
            None
        } else {
            Some(format!(
                "Stage: {}, issues: {}",
                contract.compiler_validation.stage,
                if contract.compiler_validation.issues.is_empty() {
                    "(none)".to_string()
                } else {
                    contract.compiler_validation.issues.join("; ")
                }
            ))
        },
    });

    // 2. All boundary handshakes accepted?
    let boundary_ok = !contract.boundary_requirements.is_empty();
    items.push(ReviewChecklistItem {
        label: "All boundary handshakes accepted".to_string(),
        passed: boundary_ok,
        detail: if !boundary_ok {
            Some("No boundary requirements found — verify tools and connectors are declared.".to_string())
        } else {
            Some(format!(
                "{} requirement(s) declared",
                contract.boundary_requirements.len()
            ))
        },
    });

    // 3. All subsystems explicitly configured?
    let subsystems_ok = !contract.subsystem_requirements.is_empty();
    items.push(ReviewChecklistItem {
        label: "All subsystems explicitly configured".to_string(),
        passed: subsystems_ok,
        detail: if subsystems_ok {
            Some(format!(
                "Configured: {}",
                contract.subsystem_requirements.join(", ")
            ))
        } else {
            Some("No subsystem requirements detected — verify if the workflow needs memory, storage, etc.".to_string())
        },
    });

    // 4. Approval policy set?
    let approval_set = contract.approval_status != ApprovalStatus::Pending;
    items.push(ReviewChecklistItem {
        label: "Approval policy set".to_string(),
        passed: approval_set,
        detail: match &contract.approval_status {
            ApprovalStatus::Pending => Some("Awaiting approval decision.".to_string()),
            ApprovalStatus::Approved => Some("Contract approved.".to_string()),
            ApprovalStatus::Rejected => Some("Contract was rejected — revision needed.".to_string()),
        },
    });

    // 5. Governance checks passed?
    let all_governance_passed = contract.governance_checks.iter().all(|c| c.passed);
    let failed_checks: Vec<&str> = contract
        .governance_checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.name.as_str())
        .collect();
    items.push(ReviewChecklistItem {
        label: "Governance checks passed".to_string(),
        passed: all_governance_passed,
        detail: if all_governance_passed {
            Some(format!(
                "All {} check(s) passed.",
                contract.governance_checks.len()
            ))
        } else {
            Some(format!("Failed: {}", failed_checks.join(", ")))
        },
    });

    items
}

// ── plan_mode_scaffold_specs (updated) ───────────────────────────────────────

/// Build the scaffold spec tuples that the orchestrator uses to create
/// `SessionTask` entries for the plan-mode conversation.
///
/// This version uses `build_workflow_contract` instead of the old
/// `workflow_hints_for_compilation`.
pub fn plan_mode_scaffold_specs(
    session: &PlanModeSession,
) -> Vec<(
    String,
    String,
    String,
    SessionTaskStatus,
    serde_json::Value,
    Option<SessionTaskOutput>,
)> {
    let phase = phase_rank(&session.phase);
    let mut specs = Vec::new();

    // ── Intent task ─────────────────────────────────────────────────────
    let intent_output = session.intent_cache.as_ref().map(|intent| {
        let contract = build_workflow_contract(intent, session, session.draft_role.as_ref());
        let findings: Vec<String> = contract
            .steps
            .iter()
            .take(4)
            .map(|s| s.description.clone())
            .collect();
        SessionTaskOutput {
            status: SessionTaskResultStatus::Complete,
            artifacts: Vec::new(),
            findings,
            confidence: 1.0,
            note: Some("intent, workflow shape, and operating category captured".into()),
        }
    });
    specs.push((
        format!("planmode:{}:intent", session.id),
        "Capture intent and workflow shape".into(),
        "Lock down the business goal, compiler draft, trigger guess, and output direction before execution design."
            .into(),
        if session.intent_cache.is_some() {
            SessionTaskStatus::Completed
        } else {
            SessionTaskStatus::InProgress
        },
        serde_json::json!({
            "phase": "capturing_intent",
            "recommended_tools": ["ask_user:clarification", "task_create", "task_update"],
        }),
        intent_output,
    ));

    // ── Resources task ──────────────────────────────────────────────────
    let resources_complete = phase > phase_rank(&PlanModePhase::ResolvingConnectors);
    specs.push((
        format!("planmode:{}:resources", session.id),
        "Resolve systems, resources, and access".into(),
        "Confirm connectors, databases, MCP servers, and any deferred capabilities before the workflow is finalized.".into(),
        if resources_complete {
            SessionTaskStatus::Completed
        } else if phase >= phase_rank(&PlanModePhase::ResolvingConnectors) {
            SessionTaskStatus::InProgress
        } else {
            SessionTaskStatus::Pending
        },
        serde_json::json!({
            "phase": "resolving_connectors",
            "recommended_tools": ["tool_search", "mcp_session:list_resources", "mcp_session:read_resource", "request_more_tools"],
        }),
        resources_complete.then(|| SessionTaskOutput {
            status: SessionTaskResultStatus::Complete,
            artifacts: Vec::new(),
            findings: session
                .draft_role
                .as_ref()
                .map(|role| role.connectors.clone())
                .unwrap_or_default(),
            confidence: 1.0,
            note: Some("connector and capability requirements resolved".into()),
        }),
    ));

    // ── Research / compile-contract task ─────────────────────────────────
    let research_memo = session
        .intent_cache
        .as_ref()
        .and_then(|intent| intent.get("_adaptive_research_memo"))
        .and_then(|value| serde_json::from_value::<AdaptiveResearchMemo>(value.clone()).ok());
    let research_complete = research_memo.is_some();
    specs.push((
        format!("planmode:{}:research", session.id),
        "Research and compile execution contract".into(),
        "Synthesize findings, assumptions, and risks into compile-ready workflow contract before deterministic execution is saved.".into(),
        if research_complete {
            SessionTaskStatus::Completed
        } else if session.pending_steps.is_empty() && session.intent_cache.is_some() {
            SessionTaskStatus::InProgress
        } else {
            SessionTaskStatus::Pending
        },
        serde_json::json!({
            "phase": "research_compile",
            "recommended_tools": ["task_update", "tool_search", "ask_user:decision"],
        }),
        research_memo.map(|memo| SessionTaskOutput {
            status: SessionTaskResultStatus::Complete,
            artifacts: Vec::new(),
            findings: memo.workflow_hints.into_iter().take(5).collect(),
            confidence: 1.0,
            note: Some(memo.summary),
        }),
    ));

    // ── Review task ─────────────────────────────────────────────────────
    let review_status = if phase >= phase_rank(&PlanModePhase::Reviewing) {
        SessionTaskStatus::InProgress
    } else if session.pending_steps.is_empty() && session.intent_cache.is_some() {
        SessionTaskStatus::InProgress
    } else {
        SessionTaskStatus::Pending
    };

    // Build a contract-aware review description so the checklist is visible.
    let review_description = if let Some(intent) = session.intent_cache.as_ref() {
        let contract = build_workflow_contract(intent, session, session.draft_role.as_ref());
        let checklist = review_checklist(&contract);
        let checklist_lines: Vec<String> = checklist
            .iter()
            .map(|item| {
                let mark = if item.passed { "x" } else { " " };
                format!("[{}] {}", mark, item.label)
            })
            .collect();
        format!(
            "Use the checklist to validate workflow steps, required arguments, sandbox behavior, and agent subsystems ({}) before approval.\n\nChecklist:\n{}",
            AGENT_SUBSYSTEMS.join(", "),
            checklist_lines.join("\n"),
        )
    } else {
        format!(
            "Use the checklist to validate workflow steps, required arguments, sandbox behavior, and agent subsystems ({}) before approval.",
            crate::agent::plan_mode::subsystems::AGENT_SUBSYSTEMS.join(", ")
        )
    };

    specs.push((
        format!("planmode:{}:review", session.id),
        "Review, preflight, and sandbox the draft".into(),
        review_description,
        review_status,
        serde_json::json!({
            "phase": "reviewing",
            "recommended_tools": ["task_list", "task_output", "ask_user:decision", "tool_search"],
        }),
        None,
    ));

    // ── Save task ───────────────────────────────────────────────────────
    let save_status = if session.phase == PlanModePhase::Complete {
        SessionTaskStatus::Completed
    } else {
        SessionTaskStatus::Pending
    };
    specs.push((
        format!("planmode:{}:save", session.id),
        "Save or revise the final draft".into(),
        "Approval stays separate from clarifications: revise if needed, otherwise save the draft as the execution contract.".into(),
        save_status,
        serde_json::json!({
            "phase": "save",
            "recommended_tools": ["ask_user:approval", "task_output"],
        }),
        (session.phase == PlanModePhase::Complete).then(|| SessionTaskOutput {
            status: SessionTaskResultStatus::Complete,
            artifacts: Vec::new(),
            findings: vec!["plan approved and ready to save".into()],
            confidence: 1.0,
            note: Some("plan mode reached completion".into()),
        }),
    ));

    specs
}

// ── workflow_hints_for_compilation ─────────────────────────────────────────────

/// Extract ordered workflow hints from the intent DSL + research memo.
/// Used by the compiler, `apply_execution_hints`, and fallback research memo.
pub fn workflow_hints_for_compilation(intent: &serde_json::Value) -> Vec<String> {
    let mut hints: Vec<String> = intent
        .get("workflow_dsl")
        .and_then(|value| value.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|value| {
                    if let Some(text) = value.as_str() {
                        Some(text.trim().to_string())
                    } else {
                        value.as_object().and_then(|object| {
                            object
                                .get("description")
                                .or_else(|| object.get("type"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.trim().to_string())
                        })
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if hints.is_empty() {
        if let Some(actions) = intent["actions"].as_array() {
            hints.extend(actions.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_string())));
        }
    }

    if let Some(memo) = intent
        .get("_adaptive_research_memo")
        .and_then(|value| serde_json::from_value::<AdaptiveResearchMemo>(value.clone()).ok())
    {
        hints.extend(memo.workflow_hints);
    }

    let mut merged = Vec::new();
    for hint in hints {
        let normalized = hint.trim();
        if normalized.is_empty() {
            continue;
        }
        if !merged.iter().any(|existing: &String| existing.eq_ignore_ascii_case(normalized)) {
            merged.push(normalized.to_string());
        }
    }
    merged
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dummy_session() -> PlanModeSession {
        let agent = AgentDefinition::new(
            "agent-1".into(),
            "tenant-1".into(),
            "Test Agent".into(),
        );
        PlanModeSession {
            id: "sess-1".into(),
            tenant_id: "tenant-1".into(),
            draft_agent: agent,
            draft_role: None,
            conversation: Vec::new(),
            attachments: Vec::new(),
            attachment_context: String::new(),
            session_workspace: None,
            goal_fingerprint: None,
            repair_version: 1,
            reused_from_session_id: None,
            repair_root_session_id: None,
            phase: PlanModePhase::Reviewing,
            compiler_stage: crate::agent::definition::PlanModeCompilerStage::Review,
            compiler_repair_passes: 0,
            compiler_validation_issues: Vec::new(),
            intent_cache: None,
            pending_steps: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_extract_contract_steps_from_dsl() {
        let intent = json!({
            "workflow_dsl": [
                { "id": "s1", "description": "Fetch leads from Salesforce", "tool": "salesforce", "resource_type": "lead" },
                { "id": "s2", "description": "Enrich lead data", "tool_hint": "web_search" }
            ]
        });

        let steps = extract_contract_steps(&intent);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].id, "s1");
        assert_eq!(steps[0].tool.as_deref(), Some("salesforce"));
        assert_eq!(steps[0].resource_type.as_deref(), Some("lead"));
        assert_eq!(steps[1].id, "s2");
        assert_eq!(steps[1].tool_hint.as_deref(), Some("web_search"));
        assert!(steps[1].tool.is_none());
    }

    #[test]
    fn test_build_workflow_contract_empty_intent() {
        let session = dummy_session();
        let intent = json!({});
        let contract = build_workflow_contract(&intent, &session, None);

        assert!(contract.steps.is_empty());
        assert!(contract.compiler_validation.is_valid);
        assert_eq!(contract.approval_status, ApprovalStatus::Pending);
    }

    #[test]
    fn test_governance_pii_check_fails() {
        let session = dummy_session();
        let intent = json!({
            "workflow_dsl": [
                { "description": "Fetch user email addresses from DB" }
            ]
        });

        let contract = build_workflow_contract(&intent, &session, None);
        let pii_check = contract.governance_checks.iter().find(|c| c.name == "pii_redaction").unwrap();
        assert!(!pii_check.passed, "PII check should fail when email is mentioned without redaction");
    }

    #[test]
    fn test_governance_pii_check_passes_with_redaction() {
        let session = dummy_session();
        let intent = json!({
            "workflow_dsl": [
                { "description": "Fetch user email addresses from DB" },
                { "description": "Redact PII fields before output" }
            ]
        });

        let contract = build_workflow_contract(&intent, &session, None);
        let pii_check = contract.governance_checks.iter().find(|c| c.name == "pii_redaction").unwrap();
        assert!(pii_check.passed, "PII check should pass when redaction step exists");
    }

    #[test]
    fn test_review_checklist_structure() {
        let contract = WorkflowContract {
            steps: vec![WorkflowContractStep {
                id: "s1".into(),
                description: "Test step".into(),
                tool: None,
                tool_hint: None,
                resource_type: None,
            }],
            compiler_validation: CompilerValidationState {
                stage: "Review".into(),
                repair_passes: 0,
                issues: Vec::new(),
                is_valid: true,
            },
            boundary_requirements: vec!["salesforce connector required".into()],
            subsystem_requirements: vec!["memory".into()],
            approval_status: ApprovalStatus::Approved,
            governance_checks: vec![
                GovernanceCheck { name: "pii_redaction".into(), passed: true, detail: None },
                GovernanceCheck { name: "data_barrier".into(), passed: true, detail: None },
                GovernanceCheck { name: "approval_policy".into(), passed: true, detail: None },
            ],
        };

        let checklist = review_checklist(&contract);
        assert_eq!(checklist.len(), 5);
        assert!(checklist[0].passed); // compiler
        assert!(checklist[1].passed); // boundary
        assert!(checklist[2].passed); // subsystems
        assert!(checklist[3].passed); // approval
        assert!(checklist[4].passed); // governance
    }

    #[test]
    fn test_approval_status_default_is_pending() {
        assert_eq!(ApprovalStatus::default(), ApprovalStatus::Pending);
    }

    #[test]
    fn test_review_section_titles_unchanged() {
        let titles = review_section_titles();
        assert_eq!(titles.len(), 9);
        assert_eq!(titles[0], "purpose");
    }

    #[test]
    fn test_phase_rank_ordering() {
        assert!(phase_rank(&PlanModePhase::CapturingIntent) < phase_rank(&PlanModePhase::Reviewing));
        assert!(phase_rank(&PlanModePhase::Reviewing) < phase_rank(&PlanModePhase::Complete));
    }
}
