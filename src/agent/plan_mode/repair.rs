//! Compiler repair logic for plan-mode.
//!
//! Contains the compile → validate → repair loop that the orchestrator uses
//! after the initial intent extraction. The loop compiles the workflow DSL,
//! and if the compiler returns an error it rebuilds the repair context and
//! asks the LLM to refine the intent — up to a configurable number of passes.
//!
//! All functions are standalone (not methods on a struct) so the orchestrator
//! can call them without coupling to a particular manager type.
//!
//! ## Note on `revise_from_test_result`
//!
//! That function is a thin wrapper that sets `session.phase = Reviewing` and
//! calls `PlanModeManager::turn()`, so it stays in orchestrator.rs where it
//! has access to the full turn machinery.

use anyhow::Result;
use serde_json::Value;

use crate::agent::definition::{
    AgentRole, PlanModeCompilerStage, PlanModeSession, PlanModeTestResult, TenantConnector,
};
use crate::agent::workflow_compiler::{CompilerResult, WorkflowCompiler};
use crate::tools::ToolRegistry;

use super::intent::compact_intent_snapshot;

// Re-export IntentExtractor from the orchestrator (it still lives there).
// The orchestrator owns IntentExtractor; we only need a reference to call `.refine()`.
use super::orchestrator::IntentExtractor;

// ── 1. compact_repair_context ───────────────────────────────────────────────

/// Build a compact, diff-style context string for compiler repair prompts.
///
/// Combines the current validation issues with a compact snapshot of the
/// intent (trimmed workflow_dsl, key fields) so the LLM can focus on what
/// needs to change without being overwhelmed by the full intent payload.
pub fn compact_repair_context(issues: &[String], intent: &Value) -> String {
    let snapshot = compact_intent_snapshot(intent);
    format!(
        "VALIDATION ISSUES:\n{}\n\nCURRENT DRAFT SNAPSHOT:\n{}",
        issues.join("\n"),
        serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| snapshot.to_string())
    )
}

// ── 2. validate_and_repair_compiler_draft ───────────────────────────────────

/// Compile the workflow DSL from the intent, auto-repairing up to `max_repair_passes` times.
///
/// Returns `(final_intent, Option<question>)`:
///  - `None` question means the compiler succeeded and `session.draft_role` has been updated.
///  - `Some(question)` means the compiler needs user input (NeedsCard or exhausted retries).
///
/// Side-effects on `session`:
///  - `compiler_stage` is updated at each stage transition.
///  - `compiler_repair_passes` tracks how many repair rounds have been attempted.
///  - `compiler_validation_issues` holds the last set of issues (cleared on success).
///  - On `CompilerResult::Ready`, `draft_role.execution_guidelines.compiled_workflow` is set.
pub async fn validate_and_repair_compiler_draft(
    session: &mut PlanModeSession,
    role: &AgentRole,
    intent: Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
    extractor: &IntentExtractor,
    tools: &ToolRegistry,
) -> Result<(Value, Option<String>)> {
    let mut current_intent = intent;
    let mut repair_passes = session.compiler_repair_passes;
    let mut last_issues: Vec<String> = Vec::new();

    loop {
        // ── mark: validating ─────────────────────────────────────────
        session.compiler_stage = PlanModeCompilerStage::Validate;
        session.compiler_validation_issues = last_issues.clone();

        match WorkflowCompiler::compile(role, &current_intent, tools) {
            // ── success ──────────────────────────────────────────────
            Ok(CompilerResult::Ready(compiled)) => {
                // Stamp the compiled workflow onto the draft role.
                if let Some(draft_role) = session.draft_role.as_mut() {
                    draft_role.execution_guidelines.compiled_workflow = Some(compiled.clone());
                }
                session.compiler_stage = PlanModeCompilerStage::Bind;
                session.compiler_repair_passes = repair_passes;
                session.compiler_validation_issues.clear();
                return Ok((current_intent, None));
            }

            // ── needs inline card (DB / API / MCP setup) ────────────
            Ok(CompilerResult::NeedsCard(card)) => {
                session.compiler_stage = PlanModeCompilerStage::Review;
                session.compiler_repair_passes = repair_passes;

                let question = match card.card_type.as_str() {
                    "database" => format!(
                        "The compiler needs a database connection before it can finish this workflow.\n\
                         Please open the database card for `{}` and then reply with the saved database name.",
                        card.binding_target
                    ),
                    "api_auth" => format!(
                        "The compiler needs API auth before it can finish this workflow.\n\
                         Please open the API card for `{}` and then reply once the connection is saved.",
                        card.binding_target
                    ),
                    "mcp" => format!(
                        "The compiler needs an MCP connection before it can finish this workflow.\n\
                         Please open the MCP card for `{}` and then reply once the server is saved.",
                        card.binding_target
                    ),
                    _ => format!(
                        "The compiler needs additional setup before it can finish this workflow: {}",
                        card.card_type
                    ),
                };

                session.compiler_validation_issues = vec![question.clone()];
                return Ok((current_intent, Some(question)));
            }

            // ── compiler error → attempt repair ─────────────────────
            Err(error) => {
                let issue = error.to_string();
                last_issues = vec![issue.clone()];
                session.compiler_validation_issues = last_issues.clone();

                // Exhausted repair budget → surface as a followup question.
                if repair_passes >= 2 {
                    session.compiler_stage = PlanModeCompilerStage::Review;
                    session.compiler_repair_passes = repair_passes;
                    return Ok((current_intent, Some(compiler_followup_question(&last_issues))));
                }

                // Still have retries — build repair context and refine.
                repair_passes = repair_passes.saturating_add(1);
                session.compiler_stage = PlanModeCompilerStage::Repair;
                session.compiler_repair_passes = repair_passes;

                let detail_context = format!(
                    "{}\n\n{}",
                    compact_repair_context(&last_issues, &current_intent),
                    super::registry::build_registry_candidate_context(
                        tools,
                        &current_intent,
                        installed,
                        tenant_connectors,
                    )
                );

                current_intent = extractor
                    .refine(
                        &session.id,
                        &session.tenant_id,
                        &role.purpose,
                        &current_intent,
                        &detail_context,
                    )
                    .await?;
            }
        }
    }
}

// ── 3. compiler_followup_question ───────────────────────────────────────────

/// Deduplicate issues and format them as a single follow-up question string.
///
/// Called when the compiler repair loop has exhausted its budget so the
/// orchestrator can surface the remaining issues to the user as a natural
/// language question.
pub fn compiler_followup_question(issues: &[String]) -> String {
    let mut unique: Vec<String> = Vec::new();
    for issue in issues {
        let trimmed = issue.trim();
        if !trimmed.is_empty()
            && !unique
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
        {
            unique.push(trimmed.to_string());
        }
    }

    if unique.is_empty() {
        return "I still need one more compiler detail before I can finish the workflow draft."
            .into();
    }

    format!(
        "I still need a bit more detail before I can finish the workflow draft:\n\
         - {}\n\n\
         Please clarify the missing step or setup detail, then I'll recompile.",
        unique.join("\n- ")
    )
}

// ── 4. build_revision_prompt_from_test_result ───────────────────────────────

/// Render a `PlanModeTestResult` as JSON and wrap it in a repair prompt.
///
/// The returned string is fed back into the LLM so it can repair the current
/// draft after a deterministic plan test fails or partially passes.
pub fn build_revision_prompt_from_test_result(test_result: &PlanModeTestResult) -> String {
    let rendered = serde_json::to_string_pretty(test_result)
        .unwrap_or_else(|_| test_result.summary.clone());
    format!(
        "The deterministic plan test failed or only partially passed.\n\
         Please repair the current draft using the structured test result below.\n\
         Keep the workflow deterministic and only change what is needed so the plan will pass the next test run.\n\n\
         TEST RESULT:\n{}\n",
        rendered
    )
}

// ── 5. revise_from_test_result ──────────────────────────────────────────────
//
// This is intentionally NOT included here. It is a thin wrapper that:
//
//   1. Sets `session.phase = PlanModePhase::Reviewing`
//   2. Calls `self.turn(session, &prompt).await`
//
// Because it needs `PlanModeManager::turn()` (the full conversation turn
// machinery), it stays in orchestrator.rs as a method on `PlanModeManager`.
// It uses `build_revision_prompt_from_test_result` from this module to
// construct the prompt.
