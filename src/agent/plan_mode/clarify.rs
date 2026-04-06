//! Clarification engine — drives the multi-step clarification conversation
//! that resolves connectors, triggers, outputs, constraints, boundary
//! handshakes, and subsystem bindings before the workflow is compiled.
//!
//! All public functions are standalone (not methods on a struct) so the
//! orchestrator can call them directly with whatever subset of dependencies
//! each function actually needs.

use std::sync::Arc;

use anyhow::Result;

use crate::{
    agent::definition::{
        AgentRole, ExecutionGuidelines, PlanModeMessage, PlanModePhase,
        PlanModeSession, TenantConnector, TriggerType,
    },
    agent::plan_mode::steps::{
        generate_steps, parse_and_apply, ClarificationStep, StepField,
    },
    boundry::{AskUserBoundaryHandshake, TypedSchema},
    connectors::ConnectorInstallStore,
    gateway::LlmGateway,
    storage::PostgresStore,
    tools::ToolRegistry,
};

use super::intent::AGENT_SUBSYSTEMS;

// Re-export from orchestrator so existing call-sites keep working.
use crate::tools::connector_tool::ALL_CONNECTORS as BUILTIN_CONNECTORS;

// ── ClarificationEngine ────────────────────────────────────────────────────

/// Holds the shared references needed by the clarification functions that
/// require async I/O (loading connectors, calling the LLM, etc.).
///
/// The orchestrator creates one of these early and passes `&self` into the
/// standalone functions below.
pub struct ClarificationEngine {
    pub gateway: Arc<dyn LlmGateway>,
    pub store: Arc<PostgresStore>,
    pub installs: Arc<ConnectorInstallStore>,
    pub tools: Arc<ToolRegistry>,
}

impl ClarificationEngine {
    pub fn new(
        gateway: Arc<dyn LlmGateway>,
        store: Arc<PostgresStore>,
        installs: Arc<ConnectorInstallStore>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        Self { gateway, store, installs, tools }
    }
}

// ── build_step_queue_and_ask ────────────────────────────────────────────────
// Extracted from PlanModeManager lines 865-896.

/// Generate the clarification step queue for the given intent, store it in the
/// session, and return the first question text.  Shared by handle_intent and
/// handle_connector_clarification.
pub(super) async fn build_step_queue_and_ask(
    engine: &ClarificationEngine,
    session: &mut PlanModeSession,
    intent: &serde_json::Value,
) -> String {
    let installed: Vec<String> = engine
        .installs
        .list_for_tenant(&session.tenant_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.connector_type)
        .collect();

    // Load existing roles on this agent so the step pipeline can ask about
    // workforce event filters and depends_on ordering.
    let existing_role_names: Vec<String> = engine
        .store
        .list_roles_for_agent(&session.tenant_id, &session.draft_agent.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.name)
        .collect();

    let steps = generate_steps(
        intent,
        intent["category"].as_str().unwrap_or("general"),
        &installed,
        &existing_role_names,
    );

    session.pending_steps = steps
        .iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect();

    steps
        .first()
        .map(|s| s.question.clone())
        .unwrap_or_else(|| "Any constraints or rules for this agent?".into())
}

// ── build_clarification_refinement_context ──────────────────────────────────
// Extracted from PlanModeManager lines 898-944.

/// Builds a context string from the conversation history, draft role snapshot,
/// and pending clarification steps.  Used by the intent refiner after
/// clarifications are complete.
pub(super) fn build_clarification_refinement_context(session: &PlanModeSession) -> String {
    let mut parts = Vec::new();

    // Most-recent conversation history (up to 8 turns, displayed oldest-first)
    let history: Vec<&PlanModeMessage> = session.conversation.iter().rev().take(8).collect();
    if !history.is_empty() {
        parts.push("PLAN MODE CONVERSATION (most recent last):".into());
        for message in history.into_iter().rev() {
            parts.push(format!("{}: {}", message.role, message.content));
        }
    }

    // Current draft role snapshot
    if let Some(role) = session.draft_role.as_ref() {
        parts.push("CURRENT DRAFT SNAPSHOT:".into());
        parts.push(format!("category: {}", role.role_category.as_str()));
        parts.push(format!(
            "connectors: {}",
            if role.connectors.is_empty() {
                "none".into()
            } else {
                role.connectors.join(", ")
            }
        ));
        parts.push(format!(
            "tools: {}",
            if role.tools.is_empty() {
                "none".into()
            } else {
                role.tools.join(", ")
            }
        ));
        parts.push(format!(
            "trigger: {}",
            crate::agent::agent_chat::trigger_summary(&role.trigger)
        ));
        parts.push(format!(
            "constraints: {}",
            if session.draft_agent.constraints.is_empty() {
                "none".into()
            } else {
                session.draft_agent.constraints.join("; ")
            }
        ));
    }

    // Pending clarification step summaries
    if !session.pending_steps.is_empty() {
        let step_summaries: Vec<String> = session
            .pending_steps
            .iter()
            .filter_map(|value| {
                serde_json::from_value::<ClarificationStep>(value.clone()).ok()
            })
            .map(|step| format!("{} -> {}", step.id, step.question))
            .collect();
        if !step_summaries.is_empty() {
            parts.push("UNANSWERED CLARIFICATIONS:".into());
            parts.extend(step_summaries.into_iter().take(6));
        }
    }

    parts.join("\n")
}

// ── handle_clarifications ───────────────────────────────────────────────────
// Extracted from PlanModeManager lines 2097-2182.

/// Process the user's answer to the current clarification step.
///
/// Pops the front pending step, calls `parse_and_apply` to fold the answer
/// into the draft role, handles pending-role splits, then either advances to
/// the next step or transitions to the reviewing phase.
///
/// Returns the assistant reply text for this turn.
pub(super) async fn handle_clarifications(
    _engine: &ClarificationEngine,
    session: &mut PlanModeSession,
    answer: &str,
) -> Result<String> {
    // Pop the front step — that is the one we are answering now.
    let current_step: Option<ClarificationStep> = if !session.pending_steps.is_empty() {
        let raw = session.pending_steps.remove(0);
        serde_json::from_value(raw).ok()
    } else {
        None
    };

    if let Some(step) = current_step {
        // Parse and apply the answer for this step.
        let mut agent_constraints = session.draft_agent.constraints.clone();
        let mut pending_roles: Option<Vec<serde_json::Value>> = None;

        let summary = if let Some(role) = session.draft_role.as_mut() {
            parse_and_apply(
                &step,
                answer,
                role,
                &mut agent_constraints,
                session
                    .intent_cache
                    .as_ref()
                    .unwrap_or(&serde_json::json!({})),
                &mut pending_roles,
            )
        } else {
            "Step processed.".into()
        };

        session.draft_agent.constraints = agent_constraints;

        // If user chose to split roles, stash pending responsibilities.
        if let Some(remaining) = pending_roles {
            if !session.draft_agent.memory_ref.contains("|pending_roles:") {
                let meta = session.draft_agent.memory_ref.clone();
                session.draft_agent.memory_ref = format!(
                    "{}|pending_roles:{}",
                    meta,
                    serde_json::to_string(&remaining).unwrap_or_default()
                );
            }
        }

        // Advance to next step or move to review.
        if let Some(next_raw) = session.pending_steps.first() {
            if let Ok(next_step) =
                serde_json::from_value::<ClarificationStep>(next_raw.clone())
            {
                // Show confirmation + next question
                return Ok(format!("\u{2713} {}\n\n{}", summary, next_step.question));
            }
        }

        // No more steps — transition back to the orchestrator for review.
        session.phase = PlanModePhase::Reviewing;
        return Ok(format!("✓ {}\n\nContinue with the review.", summary));
    }

    // pending_steps was already empty — go straight to review.
    session.phase = PlanModePhase::Reviewing;
    Ok("Continue with the review.".into())
}

// ── handle_connector_clarification ──────────────────────────────────────────
// Extracted from PlanModeManager lines 1909-2095.

/// Full connector-matching logic against the user's free-text answer.
///
/// Handles DB, API, MCP, and ACP connector matching against built-in
/// connectors and tenant-installed connectors.  When the connector is
/// resolved it falls through to regenerate the clarification step queue.
pub(super) async fn handle_connector_clarification(
    engine: &ClarificationEngine,
    session: &mut PlanModeSession,
    answer: &str,
) -> Result<String> {
    use super::intent::{
        intent_needs_acp_connection, intent_needs_database_connection,
    };

    let answer_lower = answer.to_lowercase();
    let mut pending_connector_resolution = false;
    let mut pending_custom_tool_categories: Vec<String> = Vec::new();
    let tenant_connectors = engine
        .store
        .list_tenant_connectors(&session.tenant_id)
        .await
        .unwrap_or_default();

    if let Some(intent) = session.intent_cache.as_ref() {
        pending_connector_resolution =
            intent["_pending_connector_resolution"].as_bool().unwrap_or(false);
        pending_custom_tool_categories = intent["_pending_custom_tool_categories"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|value| value.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
    }

    let local_document_workflow = session
        .intent_cache
        .as_ref()
        .map(intent_prefers_local_document_workflow)
        .unwrap_or(false);
    let needs_db_connection = session
        .intent_cache
        .as_ref()
        .map(intent_needs_database_connection)
        .unwrap_or(false);
    let needs_acp_connection = session
        .intent_cache
        .as_ref()
        .map(intent_needs_acp_connection)
        .unwrap_or(false);

    if let Some(role) = session.draft_role.as_mut() {
        // ── Handle pending custom tool categories ──────────────────────
        if !pending_custom_tool_categories.is_empty() {
            if let Some(intent) = session
                .intent_cache
                .as_mut()
                .and_then(|value| value.as_object_mut())
            {
                intent.remove("_pending_custom_tool_categories");
            }
            session.phase = PlanModePhase::ResolvingConnectors;
            pending_custom_tool_categories.clear();
            return Ok(
                "Deterministic custom logic should use data_engine in plan mode. \
                 If you need arbitrary code later, mark it as a missing capability \
                 for the future sandbox runtime."
                    .into(),
            );
        }

        // ── Handle pending connector resolution ────────────────────────
        if pending_connector_resolution {
            let matched: Vec<&crate::tools::connector_tool::ConnectorDef> = BUILTIN_CONNECTORS
                .iter()
                .filter(|entry| contains_connector_name(&answer_lower, entry.name))
                .collect();
            let matched_db_name =
                answer_mentions_tenant_database(&answer_lower, &tenant_connectors);
            let matched_api_name =
                answer_mentions_tenant_api(&answer_lower, &tenant_connectors);
            let matched_mcp_name =
                answer_mentions_tenant_mcp(&answer_lower, &tenant_connectors);
            let matched_acp_name =
                answer_mentions_tenant_acp(&answer_lower, &tenant_connectors);

            // Decline external connector when workflow is local-only
            if !needs_db_connection
                && !needs_acp_connection
                && (answer_declines_external_connector(&answer_lower)
                    || (local_document_workflow && matched.is_empty()))
            {
                if let Some(intent) = session
                    .intent_cache
                    .as_mut()
                    .and_then(|value| value.as_object_mut())
                {
                    intent.remove("_pending_connector_resolution");
                }
                pending_connector_resolution = false;
            }

            if pending_connector_resolution {
                // Multiple built-in matches — ask user to pick one
                if matched.len() > 1 {
                    let choices = matched
                        .iter()
                        .map(|entry| entry.name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    session.phase = PlanModePhase::ResolvingConnectors;
                    return Ok(format!(
                        "I found multiple connector names in your answer: {}. \
                         Please reply with one exact connector name.",
                        choices
                    ));
                }

                // ── Single built-in match ──────────────────────────────
                if let Some(entry) = matched.first().copied() {
                    role.connectors.retain(|connector_name| {
                        BUILTIN_CONNECTORS
                            .iter()
                            .find(|candidate| candidate.name == connector_name.as_str())
                            .map(|candidate| candidate.category != entry.category)
                            .unwrap_or(true)
                    });
                    role.connectors.push(entry.name.to_string());
                    role.connectors.sort();
                    role.connectors.dedup();
                    session.draft_agent.connectors = role.connectors.clone();
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    session.phase = PlanModePhase::ResolvingConnectors;
                    return Ok("Connector matched. Continue with the next clarification step.".into());

                // ── Tenant database match ──────────────────────────────
                } else if let Some(db_name) = matched_db_name {
                    if !role
                        .connectors
                        .iter()
                        .any(|connector_name| connector_name == &db_name)
                    {
                        role.connectors.push(db_name.clone());
                        role.connectors.sort();
                        role.connectors.dedup();
                        session.draft_agent.connectors = role.connectors.clone();
                    }
                    if !role
                        .tools
                        .iter()
                        .any(|tool| tool == &format!("external_db:{}", db_name))
                    {
                        role.tools.push(format!("external_db:{}", db_name));
                        role.tools.sort();
                        role.tools.dedup();
                    }
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    if let Some(intent) = session.intent_cache.as_mut() {
                        persist_selected_external_db(intent, &db_name);
                    }
                    pending_connector_resolution = false;

                // ── Tenant API match ───────────────────────────────────
                } else if let Some(api_name) = matched_api_name {
                    if !role
                        .connectors
                        .iter()
                        .any(|connector_name| connector_name == &api_name)
                    {
                        role.connectors.push(api_name.clone());
                        role.connectors.sort();
                        role.connectors.dedup();
                        session.draft_agent.connectors = role.connectors.clone();
                    }
                    if !role
                        .tools
                        .iter()
                        .any(|tool| tool == &format!("external_api:{}", api_name))
                    {
                        role.tools.push(format!("external_api:{}", api_name));
                        role.tools.sort();
                        role.tools.dedup();
                    }
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    pending_connector_resolution = false;

                // ── Tenant MCP match ───────────────────────────────────
                } else if let Some(mcp_name) = matched_mcp_name {
                    if !role
                        .connectors
                        .iter()
                        .any(|connector_name| connector_name == &mcp_name)
                    {
                        role.connectors.push(mcp_name.clone());
                        role.connectors.sort();
                        role.connectors.dedup();
                        session.draft_agent.connectors = role.connectors.clone();
                    }
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    pending_connector_resolution = false;

                // ── Tenant ACP match ───────────────────────────────────
                } else if let Some(acp_name) = matched_acp_name {
                    if !role
                        .connectors
                        .iter()
                        .any(|connector_name| connector_name == &acp_name)
                    {
                        role.connectors.push(acp_name.clone());
                        role.connectors.sort();
                        role.connectors.dedup();
                        session.draft_agent.connectors = role.connectors.clone();
                    }
                    if !role
                        .tools
                        .iter()
                        .any(|tool| tool == &format!("acp_session:{}", acp_name))
                    {
                        role.tools.push(format!("acp_session:{}", acp_name));
                        role.tools.sort();
                        role.tools.dedup();
                    }
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    if let Some(intent) = session.intent_cache.as_mut() {
                        persist_selected_acp_peer(intent, &acp_name);
                    }
                    pending_connector_resolution = false;

                // ── Unresolved: ask for inline card or exact name ──────
                } else if needs_db_connection {
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    session.phase = PlanModePhase::ResolvingConnectors;
                    return Ok("Connector matched. Continue with the next clarification step.".into());
                } else if needs_acp_connection {
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    session.phase = PlanModePhase::ResolvingConnectors;
                    return Ok("Connector matched. Continue with the next clarification step.".into());
                } else if local_document_workflow {
                    if let Some(intent) = session
                        .intent_cache
                        .as_mut()
                        .and_then(|value| value.as_object_mut())
                    {
                        intent.remove("_pending_connector_resolution");
                    }
                    pending_connector_resolution = false;
                } else {
                    session.phase = PlanModePhase::ResolvingConnectors;
                    return Ok("Please name the exact connector to use, or continue with the relevant setup card.".into());
                }
            }
        }
    }

    // Still unresolved after the match block — stay in ResolvingConnectors.
    if !pending_custom_tool_categories.is_empty() || pending_connector_resolution {
        session.phase = PlanModePhase::ResolvingConnectors;
        if needs_db_connection {
            return Ok("The draft still needs database setup. Continue with the database card.".into());
        }
        if session
            .intent_cache
            .as_ref()
            .map(|intent| crate::agent::plan_mode::intent::intent_needs_api_connection(intent))
            .unwrap_or(false)
        {
            return Ok("The draft still needs API setup. Continue with the API card.".into());
        }
        if session
            .intent_cache
            .as_ref()
            .map(|intent| crate::agent::plan_mode::intent::intent_needs_mcp_connection(intent))
            .unwrap_or(false)
        {
            return Ok("The draft still needs MCP setup. Continue with the MCP card.".into());
        }
        if needs_acp_connection {
            return Ok("The draft still needs ACP setup. Continue with the ACP card.".into());
        }
        return Ok("Please confirm the pending connector or custom-tool setup first.".into());
    }

    // Regenerate the step queue now that the connector is confirmed.
    let intent = session
        .intent_cache
        .clone()
        .unwrap_or_else(|| serde_json::json!({ "trigger_hint": "manual" }));
    session.phase = PlanModePhase::CapturingClarifications;
    Ok(build_step_queue_and_ask(engine, session, &intent).await)
}

// ── handle_constraints ──────────────────────────────────────────────────────
// Extracted from PlanModeManager lines 2183-2215.

/// Parses user constraints into structured guidelines and plain constraint
/// strings, then transitions to the reviewing phase.
pub(super) async fn handle_constraints(
    session: &mut PlanModeSession,
    answer: &str,
) -> Result<String> {
    let lower = answer.to_lowercase();
    let is_empty = lower.contains("no constraint")
        || lower.contains("none")
        || lower.contains("n/a")
        || lower.contains("defaults")
        || answer.trim().len() < 4;

    if !is_empty {
        // Parse domain skill answers + user constraints into structured guidelines.
        let from_user = ExecutionGuidelines::from_user_constraints(answer);
        if let Some(role) = session.draft_role.as_mut() {
            role.execution_guidelines.extend_dedup(from_user);
        }

        // Also parse plain constraint strings into agent.constraints
        // (for hard rules that should be visible in the review card).
        let constraint_items: Vec<String> = answer
            .split(&[',', ';', '\n'][..])
            .map(|s| s.trim().trim_end_matches('.').to_string())
            .filter(|s| s.len() > 8)
            .filter(|s| {
                let l = s.to_lowercase();
                !l.starts_with("mandatory")
                    && !l.starts_with("before confirm")
                    && !l.starts_with("execution brief")
            })
            .collect();
        session.draft_agent.constraints.extend(constraint_items);
    }

    session.phase = PlanModePhase::Reviewing;
    Ok("Continue with the review.".into())
}

// ── boundary_handshake_question ─────────────────────────────────────────────

/// Build the structured ask_user payload for boundary handshakes.
pub fn boundary_handshake_question(
    id: impl Into<String>,
    prompt: impl Into<String>,
    suggested_peer_endpoint: Option<String>,
    suggested_peer_name: Option<String>,
    suggested_request_schema: Option<TypedSchema>,
    suggested_response_schema: Option<TypedSchema>,
    required: bool,
    resume_token: impl Into<String>,
) -> AskUserBoundaryHandshake {
    AskUserBoundaryHandshake {
        id: id.into(),
        question_type: "boundary_handshake".into(),
        prompt: prompt.into(),
        suggested_peer_endpoint,
        suggested_peer_name,
        suggested_request_schema,
        suggested_response_schema,
        required,
        resume_token: resume_token.into(),
    }
}

// ── generate_boundary_handshake_steps ───────────────────────────────────────

/// Generate `ClarificationStep` entries for boundary setup when the intent
/// detects cross-enterprise or cross-team handoff needs.
///
/// Each workflow_dsl step that references an external enterprise, a
/// cross-team peer, or an ACP connection gets a dedicated boundary
/// handshake clarification step so the user can configure the handshake
/// before the workflow is compiled.
pub(super) fn generate_boundary_handshake_steps(
    intent: &serde_json::Value,
    _session: &PlanModeSession,
) -> Vec<ClarificationStep> {
    let mut steps = Vec::new();

    let workflow_dsl = match intent["workflow_dsl"].as_array() {
        Some(arr) => arr,
        None => return steps,
    };

    // Detect cross-boundary indicators in intent
    let needs_cross_enterprise = intent_has_cross_enterprise_signals(intent);
    let needs_cross_team = intent_has_cross_team_signals(intent);

    if !needs_cross_enterprise && !needs_cross_team {
        return steps;
    }

    // Walk the workflow DSL and emit a boundary step for each external handoff.
    for (idx, dsl_step) in workflow_dsl.iter().enumerate() {
        let step_type = dsl_step["type"].as_str().unwrap_or("");
        let description = dsl_step["description"].as_str().unwrap_or("");
        let tool = dsl_step["tool"].as_str().unwrap_or("");
        let step_id = dsl_step["id"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("step_{}", idx));

        let is_boundary_step = step_type == "boundary"
            || step_type == "cross_enterprise"
            || step_type == "cross_team"
            || tool.contains("acp_session")
            || tool.contains("boundary")
            || description_implies_cross_boundary(description);

        if !is_boundary_step {
            continue;
        }

        // Infer the peer name from DSL metadata if available.
        let suggested_peer = dsl_step["peer"]
            .as_str()
            .or_else(|| dsl_step["target_agent"].as_str())
            .or_else(|| dsl_step["counterparty"].as_str())
            .map(String::from);

        let prompt = if needs_cross_enterprise {
            format!(
                "Step \"{}\" requires a cross-enterprise handoff. \
                 Who is the external counterparty, and what data do you exchange?{}",
                description,
                suggested_peer
                    .as_deref()
                    .map(|p| format!(" (suggested: {})", p))
                    .unwrap_or_default()
            )
        } else {
            format!(
                "Step \"{}\" communicates with another team's agent. \
                 Please confirm the peer agent and the request/response contract.{}",
                description,
                suggested_peer
                    .as_deref()
                    .map(|p| format!(" (suggested: {})", p))
                    .unwrap_or_default()
            )
        };

        let clarification = ClarificationStep::new(
            format!("boundary_{}", step_id),
            prompt,
            StepField::AgentConstraint, // boundary details stored as structured constraints
        )
        .with_question_type("boundary_handshake");

        steps.push(clarification);
    }

    // If the intent signals cross-boundary need but no DSL step was flagged,
    // add a catch-all boundary step.
    if steps.is_empty() {
        let prompt = if needs_cross_enterprise {
            "This workflow involves cross-enterprise communication. \
             Which external organization will this agent exchange data with, \
             and what is the expected request/response contract?"
        } else {
            "This workflow involves cross-team agent communication. \
             Which internal team's agent will this agent communicate with, \
             and what data will be exchanged?"
        };

        steps.push(
            ClarificationStep::new("boundary_general", prompt, StepField::AgentConstraint)
                .with_question_type("boundary_handshake"),
        );
    }

    steps
}

/// Check if the intent signals cross-enterprise communication needs.
fn intent_has_cross_enterprise_signals(intent: &serde_json::Value) -> bool {
    let text = collect_intent_text(intent);
    let lower = text.to_lowercase();
    [
        "cross-enterprise",
        "cross enterprise",
        "external organization",
        "external company",
        "partner api",
        "partner agent",
        "vendor",
        "supplier",
        "counterparty",
        "b2b",
        "inter-company",
        "inter company",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

/// Check if the intent signals cross-team communication needs.
fn intent_has_cross_team_signals(intent: &serde_json::Value) -> bool {
    let text = collect_intent_text(intent);
    let lower = text.to_lowercase();
    [
        "cross-team",
        "cross team",
        "other team",
        "another team",
        "internal agent",
        "peer agent",
        "teammate",
        "sibling agent",
        "handoff",
        "hand off",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

/// Check if a step description implies a cross-boundary handoff.
fn description_implies_cross_boundary(description: &str) -> bool {
    let lower = description.to_lowercase();
    [
        "send to",
        "receive from",
        "handoff",
        "hand off",
        "cross-enterprise",
        "cross-team",
        "external agent",
        "peer agent",
        "partner",
        "counterparty",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

/// Collect all textual content from intent for keyword scanning.
fn collect_intent_text(intent: &serde_json::Value) -> String {
    let mut text = String::new();
    for key in &[
        "data_sources",
        "write_targets",
        "actions",
        "constraints",
    ] {
        if let Some(arr) = intent[*key].as_array() {
            for v in arr {
                if let Some(s) = v.as_str() {
                    text.push_str(s);
                    text.push(' ');
                } else if let Some(obj) = v.as_object() {
                    if let Some(d) = obj.get("description").and_then(|v| v.as_str()) {
                        text.push_str(d);
                        text.push(' ');
                    }
                }
            }
        }
    }
    if let Some(steps) = intent["workflow_dsl"].as_array() {
        for step in steps {
            if let Some(d) = step["description"].as_str() {
                text.push_str(d);
                text.push(' ');
            }
            if let Some(t) = step["type"].as_str() {
                text.push_str(t);
                text.push(' ');
            }
        }
    }
    if let Some(output) = intent["output_hint"].as_str() {
        text.push_str(output);
    }
    text
}

// ── generate_subsystem_steps ────────────────────────────────────────────────

/// Generate `ClarificationStep` entries for each agent subsystem that needs
/// explicit configuration.
///
/// Scans the draft role's current tools, connectors, and intent to determine
/// which subsystems (memory, knowledge, swarm, scheduler, skills, storage,
/// workspace) require user input.  Only subsystems that are relevant to the
/// intent but not yet configured produce a step.
pub(super) fn generate_subsystem_steps(
    intent: &serde_json::Value,
    session: &PlanModeSession,
) -> Vec<ClarificationStep> {
    let mut steps = Vec::new();

    let role = match session.draft_role.as_ref() {
        Some(r) => r,
        None => return steps,
    };

    let intent_text = collect_intent_text(intent).to_lowercase();
    let category = intent["category"].as_str().unwrap_or("general");

    for &subsystem in AGENT_SUBSYSTEMS {
        let relevant = subsystem_is_relevant(subsystem, &intent_text, category, role);
        if !relevant {
            continue;
        }

        let already_configured = subsystem_is_configured(subsystem, role, session);
        if already_configured {
            continue;
        }

        let (question, hint) = subsystem_clarification_prompt(subsystem, category);
        let mut step = ClarificationStep::new(
            format!("subsystem_{}", subsystem),
            question,
            StepField::AgentConstraint,
        )
        .with_question_type("subsystem_config");

        if let Some(h) = hint {
            step = step.with_hint(h);
        }

        steps.push(step);
    }

    steps
}

/// Determine whether a subsystem is relevant to the current intent.
fn subsystem_is_relevant(
    subsystem: &str,
    intent_text: &str,
    category: &str,
    role: &AgentRole,
) -> bool {
    match subsystem {
        "memory" => {
            // Memory is relevant for multi-turn, stateful, or learning agents.
            intent_text.contains("remember")
                || intent_text.contains("history")
                || intent_text.contains("context")
                || intent_text.contains("stateful")
                || intent_text.contains("learn")
                || intent_text.contains("multi-turn")
                || intent_text.contains("conversation")
                || category == "customer_support"
                || category == "research_analyst"
        }
        "knowledge" => {
            // Knowledge is relevant when the agent needs a knowledge base.
            intent_text.contains("knowledge")
                || intent_text.contains("rag")
                || intent_text.contains("retrieval")
                || intent_text.contains("search documents")
                || intent_text.contains("corpus")
                || intent_text.contains("faq")
                || intent_text.contains("wiki")
                || category == "research_analyst"
                || category == "customer_support"
        }
        "swarm" => {
            // Swarm is relevant for multi-agent coordination.
            intent_text.contains("swarm")
                || intent_text.contains("multi-agent")
                || intent_text.contains("coordinate")
                || intent_text.contains("delegate")
                || intent_text.contains("fan-out")
                || intent_text.contains("parallel agents")
                || !role.connectors.iter().all(|c| !c.contains("acp"))
        }
        "scheduler" => {
            // Scheduler is relevant for timed or recurring workflows.
            intent_text.contains("schedule")
                || intent_text.contains("cron")
                || intent_text.contains("recurring")
                || intent_text.contains("every day")
                || intent_text.contains("every hour")
                || intent_text.contains("periodic")
                || matches!(role.trigger.trigger_type, TriggerType::Schedule)
        }
        "skills" => {
            // Skills are relevant for agents that need domain-specific capabilities.
            intent_text.contains("skill")
                || intent_text.contains("capability")
                || intent_text.contains("plugin")
                || intent_text.contains("extension")
        }
        "storage" => {
            // Storage is relevant when the agent produces persistent artifacts.
            intent_text.contains("store")
                || intent_text.contains("persist")
                || intent_text.contains("save")
                || intent_text.contains("database")
                || intent_text.contains("file output")
                || intent_text.contains("artifact")
                || !role.tools.iter().all(|t| !t.starts_with("external_db:"))
        }
        "workspace" => {
            // Workspace is relevant for file-based or document workflows.
            intent_text.contains("workspace")
                || intent_text.contains("file")
                || intent_text.contains("document")
                || intent_text.contains("upload")
                || intent_text.contains("download")
                || intent_text.contains("attachment")
        }
        _ => false,
    }
}

/// Check whether a subsystem is already configured in the current draft.
fn subsystem_is_configured(
    subsystem: &str,
    role: &AgentRole,
    session: &PlanModeSession,
) -> bool {
    match subsystem {
        "memory" => {
            // Configured if memory_scope is non-default or memory_ref is set.
            role.memory_scope != role.role_category.default_memory_scope()
                || !session.draft_agent.memory_ref.is_empty()
        }
        "knowledge" => {
            // Configured if any knowledge-related tool is in the tool list.
            role.tools.iter().any(|t| {
                t.contains("knowledge") || t.contains("rag") || t.contains("search")
            })
        }
        "swarm" => {
            // Configured if ACP connectors are already bound.
            role.connectors.iter().any(|c| c.contains("acp"))
                || role.tools.iter().any(|t| t.starts_with("acp_session:"))
        }
        "scheduler" => {
            // Configured if trigger is already schedule-type with a cron.
            matches!(role.trigger.trigger_type, TriggerType::Schedule)
                && role.trigger.cron.is_some()
        }
        "skills" => {
            // Configured if execution_guidelines already has skill references.
            !role.execution_guidelines.completion_criteria.is_empty()
        }
        "storage" => {
            // Configured if external_db tool is bound.
            role.tools.iter().any(|t| t.starts_with("external_db:"))
        }
        "workspace" => {
            // Configured if workspace-related constraints exist.
            session.draft_agent.constraints.iter().any(|c| {
                let l = c.to_lowercase();
                l.contains("workspace") || l.contains("file") || l.contains("directory")
            })
        }
        _ => false,
    }
}

/// Return the clarification question and optional hint for a subsystem.
fn subsystem_clarification_prompt(
    subsystem: &str,
    category: &str,
) -> (String, Option<String>) {
    match subsystem {
        "memory" => (
            "How should this agent handle memory? Options: \
             per-session (forget after each run), per-user (remember across runs for each user), \
             or shared (single memory across all users)."
                .into(),
            Some("Most agents use per-session memory. Choose per-user if the agent needs to recall past interactions.".into()),
        ),
        "knowledge" => (
            "Does this agent need a knowledge base? If so, describe what documents or data \
             it should be able to search (e.g., product FAQ, internal wiki, support tickets)."
                .into(),
            Some("Leave empty if the agent only works with live data from connectors.".into()),
        ),
        "swarm" => (
            "This agent may need to coordinate with other agents. Should it delegate tasks \
             to child agents, or communicate peer-to-peer with sibling agents? \
             Describe the coordination pattern."
                .into(),
            None,
        ),
        "scheduler" => (
            "How often should this agent run? Provide a schedule \
             (e.g., 'every hour', 'daily at 9am', 'every Monday at 8am') \
             or say 'on demand' for manual triggers only."
                .into(),
            Some("Cron expressions are also accepted (e.g., '0 9 * * 1-5' for weekdays at 9am).".into()),
        ),
        "skills" => (
            format!(
                "Are there any domain-specific skills or plugins this {} agent should have? \
                 Describe them, or say 'none' to use the defaults.",
                category
            ),
            None,
        ),
        "storage" => (
            "Where should this agent store its output? Options: \
             workspace (local files), database (external DB), or both. \
             If database, which database connection should it use?"
                .into(),
            Some("If you already configured a database connector, it will be used automatically.".into()),
        ),
        "workspace" => (
            "Does this agent need a dedicated workspace directory? If so, describe the \
             file types and folder structure it should use."
                .into(),
            Some("A default workspace is created automatically if not specified.".into()),
        ),
        _ => (
            format!("Configure the {} subsystem for this agent.", subsystem),
            None,
        ),
    }
}

// ── Connector-matching helpers ───────────────────────────────────────────────
// These are small pure functions used by handle_connector_clarification to
// match user answers against installed connectors.  Duplicated here from the
// orchestrator to keep the clarify module self-contained.

fn contains_connector_name(answer_lower: &str, connector_name: &str) -> bool {
    let name = connector_name.to_ascii_lowercase();
    answer_lower
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .any(|token| token == name)
}

fn answer_declines_external_connector(answer_lower: &str) -> bool {
    [
        "none",
        "no connector",
        "no external connector",
        "no external connectors",
        "built-in",
        "builtin",
        "local",
        "local only",
        "read-only",
        "read only",
        "workspace",
        "document",
        "documents",
        "file",
        "files",
        "uploaded file",
        "uploaded documents",
    ]
    .iter()
    .any(|phrase| answer_lower.contains(phrase))
}

fn answer_mentions_tenant_database(
    answer_lower: &str,
    tenant_connectors: &[TenantConnector],
) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category == "connector/database")
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

fn answer_mentions_tenant_api(
    answer_lower: &str,
    tenant_connectors: &[TenantConnector],
) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category != "connector/database" && !tc.category.contains("mcp"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

fn answer_mentions_tenant_mcp(
    answer_lower: &str,
    tenant_connectors: &[TenantConnector],
) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category.contains("mcp"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

fn answer_mentions_tenant_acp(
    answer_lower: &str,
    tenant_connectors: &[TenantConnector],
) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category.contains("acp") || tc.category.contains("agent"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

fn intent_prefers_local_document_workflow(intent: &serde_json::Value) -> bool {
    let text = collect_intent_text(intent);
    let lower = text.to_lowercase();
    let has_document_terms = [
        "document", "documents", "file", "files", "pdf", "csv",
        "spreadsheet", "attachment", "uploaded", "upload",
    ]
    .iter()
    .any(|term| lower.contains(term));
    let has_read_terms = [
        "read", "review", "analyze", "analyse", "summarize", "summarise",
        "extract", "inspect", "highlight", "report",
    ]
    .iter()
    .any(|term| lower.contains(term));

    let write_targets_empty = intent["write_targets"]
        .as_array()
        .map(|arr| arr.is_empty())
        .unwrap_or(true);
    let output_hint = intent["output_hint"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    let local_output_hint = matches!(output_hint.as_str(), "" | "workspace" | "report")
        || output_hint.contains("chat");

    has_document_terms && has_read_terms && write_targets_empty && local_output_hint
}

fn persist_selected_external_db(intent: &mut serde_json::Value, db_name: &str) {
    if let Some(intent_object) = intent.as_object_mut() {
        intent_object.insert("uses_external_db".into(), serde_json::json!(db_name));
    }
}

fn persist_selected_acp_peer(intent: &mut serde_json::Value, peer_name: &str) {
    if let Some(intent_object) = intent.as_object_mut() {
        intent_object.insert("uses_acp_peer".into(), serde_json::json!(peer_name));
    }
}
