//! StepOrchestrator — unified per-step lifecycle hooks for both
//! the linear AgentLoop and the DAG engine.
//!
//! Extracts the pre-step, execution, and post-step logic that was
//! previously inline in `loop.rs` into a reusable component. Both
//! execution engines call the orchestrator instead of the executor
//! directly, giving DAG steps access to:
//!   - Knowledge graph injection (pre-step)
//!   - Delegation context injection (pre-step)
//!   - Template variable resolution (pre-step)
//!   - Cognitive limits enforcement (pre-step)
//!   - Delegation detection (post-step)
//!   - Clarification detection (post-step)
//!   - FailureRule deterministic abort (post-step)
//!   - Knowledge graph extraction (post-step)
//!   - Citation tracking (post-step)
//!   - Debug recording (post-step)
//!
//! The evaluator (Continue/Retry/Abort/GoalComplete) is NOT called
//! per-step by the orchestrator. In DAG mode, evaluation happens
//! per-*cycle* (batch) in the DagEngine. In linear mode, the AgentLoop
//! still calls the evaluator after receiving the StepVerdict.

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    agent::{
        executor::{Executor, StepResult},
        planner::{Plan, PlannedStep},
        prompts::StepHistory,
        template_vars::{self, TemplateContext},
    },
    events::{AgentEvent, EventBus},
    knowledge::graph::KnowledgeGraph,
    segments::AgentServices,
    state::AgentState,
};

// ═══════════════════════════════════════════════════════════════════════════
// STEP VERDICT — what happened after executing a step
// ═══════════════════════════════════════════════════════════════════════════

/// The orchestrator's verdict after running a step — tells the caller
/// what to do next. Richer than raw StepResult because it accounts
/// for delegation, clarification, and failure rule matches.
#[derive(Debug)]
pub enum StepVerdict {
    /// Step executed successfully. `result` has tool outputs.
    Executed {
        result: StepResult,
        /// Entities extracted from the output for the knowledge graph.
        extracted_entities: Vec<(String, String)>,
    },
    /// Step was skipped by condition gate.
    Skipped { reason: String },
    /// Step spawned child agents — caller should mark as delegating.
    Delegating { result: StepResult, child_ids: Vec<String> },
    /// Step needs human input — caller should pause this step.
    NeedsClarification { result: StepResult, questions: Vec<crate::agent::clarifier::ClarificationQuestion> },
    /// Step matched a deterministic FailureRule — abort without evaluator.
    DeterministicAbort { result: StepResult, reason: String },
    /// Execution error (not a tool failure — an infrastructure error).
    Error { error: anyhow::Error },
}

// ═══════════════════════════════════════════════════════════════════════════
// STEP ORCHESTRATOR
// ═══════════════════════════════════════════════════════════════════════════

/// Shared per-step lifecycle orchestration.
///
/// Wraps the `Executor` with pre-step and post-step hooks extracted
/// from the original `AgentLoop`. Both the linear loop and the DAG
/// engine use this to get consistent behavior.
pub struct StepOrchestrator {
    executor: Arc<dyn Executor>,
    event_bus: Arc<EventBus>,
    knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
    services: Arc<AgentServices>,
    store: Option<Arc<crate::storage::PostgresStore>>,
    vector_store: Arc<dyn crate::memory::VectorStore>,
    embedder: Arc<dyn crate::memory::EmbeddingModel>,
}

impl StepOrchestrator {
    pub fn new(
        executor: Arc<dyn Executor>,
        event_bus: Arc<EventBus>,
        knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
        services: Arc<AgentServices>,
        vector_store: Arc<dyn crate::memory::VectorStore>,
        embedder: Arc<dyn crate::memory::EmbeddingModel>,
    ) -> Self {
        Self { executor, event_bus, knowledge_graph, services, store: None, vector_store, embedder }
    }

    pub fn with_store(mut self, store: Arc<crate::storage::PostgresStore>) -> Self {
        self.store = Some(store);
        self
    }

    // ── MAIN ENTRY POINT ─────────────────────────────────────────────

    /// Execute a single step with full lifecycle orchestration.
    ///
    /// This is the unified entry point that both `loop.rs` and
    /// `dag_engine.rs` call instead of raw `executor.execute_step()`.
    ///
    /// Flow:
    /// 1. Pre-step: cognitive limits, knowledge injection, template resolution
    /// 2. Execute: call executor.execute_step()
    /// 3. Post-step: delegation check, clarification check, failure rules,
    ///    knowledge extraction, citation tracking, event emission
    ///
    /// The evaluator is NOT called here — that's the caller's responsibility
    /// (per-step in linear mode, per-cycle in DAG mode).
    pub async fn run_step(
        &self,
        state: &mut AgentState,
        step: &PlannedStep,
        plan: &Plan,
        history: &mut StepHistory,
    ) -> StepVerdict {
        // ── 1. Pre-step: Knowledge graph injection ───────────────────
        self.inject_knowledge(state, history);

        // ── 2a. Pre-step: Inject delegation ctx ──────────────────────────────
        let step = self.inject_delegation_ctx(state, &step);

        // ── 2b. Pre-step: Template variable resolution ────────────────
        let step = self.resolve_templates(state, &step);

        // ── 3. Emit StepStarted event ────────────────────────────────
        self.event_bus.publish(AgentEvent::StepStarted {
            agent_id: state.id.clone(),
            step_index: step.index,
            description: step.description.clone(),
            tool: step.tool.clone(),
            success_criteria: (!step.success_criteria.trim().is_empty()).then(|| step.success_criteria.clone()),
            condition: format_step_condition_opt(&step),
        });

        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            step_description = %step.description,
            tool = ?step.tool,
            "orchestrator: executing step"
        );

        // ── 4. Execute ──────────────────────────────────────────────
        let result = match self.executor.execute_step(state, &step, plan, history).await {
            Ok(result) => result,
            Err(error) => {
                return StepVerdict::Error { error };
            }
        };

        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            success = result.success,
            tools_called = ?result.tools_called,
            output_len = result.output.len(),
            "orchestrator: step execution complete"
        );

        // ── 5. Post-step: Debug recording ────────────────────────────
        self.record_debug(state, &step, &result);

        // ── 6. Post-step: Persist step output ────────────────────────
        persist_step_output(state, &step, &result);

        // ── 7. Post-step: Check for delegation ───────────────────────
        if let Some(child_ids) = self.detect_delegation(&result) {
            self.emit_tool_result_events(state, &step, &result);
            for cid in &child_ids {
                self.event_bus.publish(AgentEvent::ChildSpawned {
                    agent_id: state.id.clone(),
                    child_agent_id: cid.clone(),
                    sub_goal: step.description.clone(),
                });
            }
            return StepVerdict::Delegating { result, child_ids };
        }

        // ── 8. Post-step: Emit tool result events ────────────────────
        self.emit_tool_result_events(state, &step, &result);

        // ── 9. Post-step: Check for clarification ────────────────────
        if let Some(questions) = self.detect_clarification(&result) {
            return StepVerdict::NeedsClarification { result, questions };
        }

        // ── 10. Post-step: FailureRule deterministic abort ───────────
        if !result.success {
            if let Some(reason) = self.check_failure_rules(state, &result).await {
                return StepVerdict::DeterministicAbort { result, reason };
            }
        }

        // ── 11. Post-step: Knowledge extraction ──────────────────────
        let extracted_entities = self.extract_knowledge(&result);

        // ── 12. Post-step: Citation tracking ─────────────────────────
        self.record_citations(state, &step, &result).await;

        // ── 13. Post-step: Vector store persistence ──────────────────
        self.persist_to_vector_store(state, &step, &result).await;

        StepVerdict::Executed { result, extracted_entities }
    }

    // ── PRE-STEP HOOKS ───────────────────────────────────────────────

    fn inject_knowledge(&self, state: &AgentState, history: &mut StepHistory) {
        if let Ok(graph) = self.knowledge_graph.try_lock() {
            let facts = graph.get_related(&state.goal);
            if !facts.is_empty() {
                let recent_count = facts.len().min(5);
                let facts_text = facts
                    .iter()
                    .take(recent_count)
                    .map(|n| format!("{}: {}", n.id, n.value))
                    .collect::<Vec<_>>()
                    .join("\n");
                history.inject_facts(&facts_text);
                tracing::debug!(
                    agent_id = %state.id,
                    total = facts.len(),
                    injected = recent_count,
                    "orchestrator: knowledge facts injected"
                );
            }
        }
    }

    fn resolve_templates(&self, state: &AgentState, step: &PlannedStep) -> PlannedStep {
        if template_vars::has_templates(&step.tool_args) {
            let ctx = TemplateContext::from_agent_state(state);
            let resolved = template_vars::resolve_step_templates(step, &ctx);
            tracing::debug!(
                agent_id = %state.id,
                step_index = step.index,
                "orchestrator: template variables resolved"
            );
            resolved
        } else {
            step.clone()
        }
    }

    fn inject_delegation_ctx(&self, state: &AgentState, step: &PlannedStep) -> PlannedStep {
        // Tools that need tenant_id / agent_id injected automatically
        let needs_ctx = matches!(
            step.tool.as_deref(),
            Some("delegate")
                | Some("send_message")
                | Some("message_inbox")
                | Some("task_create")
                | Some("task_get")
                | Some("task_list")
                | Some("task_update")
                | Some("task_stop")
                | Some("task_output")
                | Some("enter_worktree")
                | Some("exit_worktree")
                | Some("vector_store")
                | Some("vector_search")
                | Some("vector_delete")
                | Some("mcp_session")
                | Some("search_mcp_registry")
        );
        if needs_ctx {
            let mut s = step.clone();
            let mut args = step.tool_args.clone().unwrap_or_default();
            args["tenant_id"] = serde_json::json!(state.tenant_id);
            args["agent_id"] = serde_json::json!(state.id);
            if matches!(
                step.tool.as_deref(),
                Some("delegate")
                    | Some("task_create")
                    | Some("task_get")
                    | Some("task_list")
                    | Some("task_update")
                    | Some("task_stop")
                    | Some("task_output")
            ) && args
                .get("task_id")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
            {
                if let Some(current_task) = state.current_task.as_ref() {
                    args["task_id"] = serde_json::json!(current_task);
                }
            }
            if step.tool.as_deref() == Some("delegate") {
                args["parent_agent_id"] = serde_json::json!(state.id);
            }
            if matches!(step.tool.as_deref(), Some("send_message") | Some("message_inbox")) {
                if let Some(parent_agent_id) = state.parent_agent_id.as_ref() {
                    args["parent_agent_id"] = serde_json::json!(parent_agent_id);
                }
                if let Some(current_task) = state.current_task.as_ref() {
                    args["task_id"] = serde_json::json!(current_task);
                }
            }
            if matches!(step.tool.as_deref(), Some("enter_worktree") | Some("exit_worktree")) {
                args["workspace_path"] = serde_json::json!(state.workspace_path);
            }
            s.tool_args = Some(args);
            s
        } else {
            step.clone()
        }
    }

    // ── POST-STEP HOOKS ──────────────────────────────────────────────

    fn record_debug(&self, state: &mut AgentState, step: &PlannedStep, result: &StepResult) {
        let mut recorder: AgentRecorder = state
            .metadata
            .get("debug_recording")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_else(AgentRecorder::new);
        recorder.record(
            step.index,
            step.description.clone(),
            serde_json::to_string(&result.tool_results).unwrap_or_default(),
        );
        state.metadata["debug_recording"] = serde_json::to_value(&recorder.steps).unwrap_or_default();
    }

    fn detect_delegation(&self, result: &StepResult) -> Option<Vec<String>> {
        for tool_result in &result.tool_results {
            if let Some(arr) = tool_result.output.get("child_agent_ids").and_then(|v| v.as_array()) {
                let child_ids: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                if !child_ids.is_empty() {
                    return Some(child_ids);
                }
            }
        }
        None
    }

    fn detect_clarification(&self, result: &StepResult) -> Option<Vec<crate::agent::clarifier::ClarificationQuestion>> {
        for tool_result in &result.tool_results {
            if tool_result.output.get("status").and_then(|v| v.as_str()) == Some("awaiting_user_input") {
                let parsed = crate::agent::clarifier::parse_clarification_questions(
                    tool_result.output.get("questions").unwrap_or(&serde_json::Value::Null),
                );
                if !parsed.is_empty() {
                    return Some(parsed);
                }
            }
        }
        None
    }

    async fn check_failure_rules(&self, state: &AgentState, result: &StepResult) -> Option<String> {
        let role_id = state.metadata.get("role_id").and_then(|v| v.as_str()).map(String::from);

        if let (Some(ref store), Some(ref rid)) = (&self.store, role_id.as_ref()) {
            if let Ok(Some(role)) = store.get_agent_role(&state.tenant_id, rid).await {
                // Check each failure rule for a deterministic match
                for rule in &role.execution_guidelines.failure_handling {
                    let scope_matches = rule
                        .tool_scope
                        .as_ref()
                        .map_or(true, |scope: &String| result.tools_called.iter().any(|t| t.contains(scope.as_str())));
                    if !scope_matches {
                        continue;
                    }

                    // Check if the error output contains the rule text
                    let error_text = result
                        .tool_results
                        .iter()
                        .filter(|r| !r.success)
                        .filter_map(|r| r.error.as_deref())
                        .collect::<Vec<_>>()
                        .join(" ");

                    let output_text = format!("{} {}", result.output, error_text);
                    let lower = output_text.to_lowercase();

                    if lower.contains(&rule.text.to_lowercase()) {
                        match &rule.action {
                            crate::agent::definition::FailureAction::Abort => {
                                return Some(format!("FailureRule matched: {} (action: abort)", rule.text));
                            }
                            crate::agent::definition::FailureAction::EscalateToHuman { notify_channel } => {
                                if let Some(channel) = notify_channel {
                                    tracing::warn!(
                                        agent_id = %state.id,
                                        channel = %channel,
                                        "FailureRule: escalating to human"
                                    );
                                }
                                return Some(format!("FailureRule matched: {} (action: escalate)", rule.text));
                            }
                            _ => {
                                // SkipAndLog, RetryOnce — not abort-level
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn extract_knowledge(&self, result: &StepResult) -> Vec<(String, String)> {
        let mut entities = Vec::new();
        if result.success {
            for extracted in extract_entities(&result.output) {
                if let Ok(mut graph) = self.knowledge_graph.try_lock() {
                    let _ = graph.add_node(extracted.0.clone(), extracted.1.clone());
                }
                entities.push(extracted);
            }
        }
        entities
    }

    fn emit_tool_result_events(&self, state: &AgentState, step: &PlannedStep, result: &StepResult) {
        for (index, tool_result) in result.tool_results.iter().enumerate() {
            self.event_bus.publish(AgentEvent::ToolResult {
                agent_id: state.id.clone(),
                step_index: step.index,
                tool_name: result.tools_called.get(index).cloned().unwrap_or_else(|| "tool".into()),
                success: tool_result.success,
                output_preview: crate::util::truncate(
                    &serde_json::to_string(&tool_result.output).unwrap_or_default(),
                    600,
                )
                .to_string(),
                error: tool_result.error.clone(),
            });
        }
    }

    async fn record_citations(&self, state: &AgentState, step: &PlannedStep, result: &StepResult) {
        if !result.success || result.output.is_empty() {
            return;
        }
        if let Some(ref ct) = self.services.citations {
            for tool_name in &result.tools_called {
                let confidence = if result.success { 1.0_f64 } else { 0.5_f64 };
                let _: Result<_, _> = ct
                    .record(
                        &state.id,
                        &state.tenant_id,
                        step.index,
                        &crate::util::truncate(&result.output, 200),
                        "tool_output",
                        tool_name,
                        &crate::util::truncate(&result.output, 200),
                        confidence,
                    )
                    .await;
                self.event_bus.publish(AgentEvent::CitationRecorded {
                    agent_id: state.id.clone(),
                    step_index: step.index,
                    claim: crate::util::truncate(&result.output, 120).to_string(),
                    source_ref: tool_name.clone(),
                    source_type: "tool_output".into(),
                    confidence,
                });
            }
        }
    }

    async fn persist_to_vector_store(&self, state: &AgentState, step: &PlannedStep, result: &StepResult) {
        if !result.success || result.output.is_empty() {
            return;
        }
        let content = format!(
            "Step {} — {} | Output: {}",
            step.index,
            step.description,
            crate::util::truncate(&result.output, 500)
        );
        match self.embedder.embed(&content).await {
            Ok(embedding) => {
                let doc =
                    crate::memory::VectorDocument::new(state.tenant_id.clone(), state.id.clone(), content, embedding)
                        .with_metadata(serde_json::json!({
                            "step": step.index,
                            "goal": state.goal,
                            "auto": true,
                        }));
                if let Err(e) = self.vector_store.upsert(doc).await {
                    tracing::debug!(error = %e, "vector upsert failed — continuing");
                }
            }
            Err(e) => tracing::debug!(error = %e, "embedding failed — skipping vector store"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HELPER FUNCTIONS (extracted from loop.rs)
// ═══════════════════════════════════════════════════════════════════════════

/// Persist step output into state.metadata for downstream steps and UI.
fn persist_step_output(state: &mut AgentState, step: &PlannedStep, result: &StepResult) {
    let key = format!("step_{}_output", step.index);
    let value = serde_json::json!({
        "success": result.success,
        "output": crate::util::truncate(&result.output, 2000),
        "tools_called": result.tools_called,
        "items_processed": result.items_processed,
    });
    state.metadata[&key] = value;

    if let Some(candidate) = result.final_answer_candidate.clone() {
        state.set_final_answer(candidate);
    }
}

/// Extract named entities from text for the knowledge graph.
/// Simple pattern-based extraction (not LLM).
fn extract_entities(text: &str) -> Vec<(String, String)> {
    let mut entities = Vec::new();

    // Extract URLs
    for word in text.split_whitespace() {
        if word.starts_with("http://") || word.starts_with("https://") {
            let cleaned = word.trim_matches(|c: char| {
                !c.is_alphanumeric() && c != ':' && c != '/' && c != '.' && c != '-' && c != '_'
            });
            entities.push(("url".to_string(), cleaned.to_string()));
        }
    }

    // Extract email-like patterns
    for word in text.split_whitespace() {
        if word.contains('@') && word.contains('.') {
            entities.push(("email".to_string(), word.to_string()));
        }
    }

    entities
}

/// Format step condition for event display.
fn format_step_condition_opt(step: &PlannedStep) -> Option<String> {
    step.condition.as_ref().map(|cond| format!("{:?}", cond))
}

/// Debug recorder for step-by-step tracing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AgentRecorder {
    steps: Vec<RecordedStep>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RecordedStep {
    index: usize,
    description: String,
    tool_results: String,
}

impl AgentRecorder {
    fn new() -> Self {
        Self { steps: Vec::new() }
    }

    fn record(&mut self, index: usize, description: String, tool_results: String) {
        self.steps.push(RecordedStep { index, description, tool_results });
    }
}
