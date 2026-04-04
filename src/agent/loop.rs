use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};

use crate::{
    agent::{
        clarifier::{ClarificationResult, Clarifier},
        evaluator::{check_completion_criteria, EvalVerdict, Evaluator},
        executor::Executor,
        planner::Plan,
        preflight::{Preflight, PreflightResult},
        prompts::{is_direct_response_goal, StepHistory},
        reflector::Reflector,
        workflow_compiler::{data_signature_from_value, TypedExpression},
    },
    cognition::{
        control_loop::CognitiveControlLoop,
        judgement::{JudgementContext, JudgementEngine, JudgementRecommendation, JudgementSignal},
    },
    compliance::sla::{EscalationAction, SlaStatus},
    events::{AgentEvent, EventBus},
    knowledge::graph::KnowledgeGraph,
    segments::AgentServices,
    skill_evolution::evolution::evolve_skill,
    skills::registry::SkillRegistry,
    state::{
        AgentMessage, AgentMessageKind, AgentState, AgentStatus, SessionTask, SessionTaskOutput,
        SessionTaskResultStatus, SessionTaskStatus,
    },
    tools::ToolRegistry,
    util::next_run_after,
};

fn truncate_for_log(value: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(value.len().min(max_chars));
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...(truncated)");
    }
    out
}

fn completion_summary(state: &AgentState) -> String {
    state
        .final_answer()
        .filter(|answer| !looks_like_placeholder(answer))
        .or_else(|| state.metadata.get("last_reflection").and_then(|value| value.as_str()))
        .filter(|answer| !looks_like_placeholder(answer))
        .unwrap_or("goal achieved")
        .to_string()
}

fn looks_like_placeholder(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized.is_empty()
        || normalized == "goal complete"
        || normalized == "goal achieved"
        || normalized == "no output"
        || normalized.starts_with("step complete")
}

fn step_history_summary(result: &crate::agent::executor::StepResult, fallback: &str) -> String {
    if let Some(answer) = result.final_answer_candidate.as_deref().filter(|value| !looks_like_placeholder(value)) {
        return answer.to_string();
    }

    if !result.tool_results.is_empty() {
        let details = result
            .tool_results
            .iter()
            .enumerate()
            .map(|(index, tool_result)| {
                let tool_name = result.tools_called.get(index).map(String::as_str).unwrap_or("tool");
                let payload = if tool_result.output.is_null() {
                    tool_result.error.clone().unwrap_or_else(|| "no output".into())
                } else {
                    serde_json::to_string(&tool_result.output).unwrap_or_default()
                };
                format!("{tool_name}: {payload}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !details.trim().is_empty() {
            return details;
        }
    }

    if !looks_like_placeholder(&result.output) {
        return result.output.clone();
    }

    fallback.to_string()
}

/// Maximum step_outputs entries kept in the metadata JSONB column.
/// Older entries are evicted. Full outputs remain on disk as artifact files.
const STEP_OUTPUTS_METADATA_CAP: usize = 30;

/// Atomic state transaction — batches all step-related mutations for consistency.
/// Ensures crash-safety: either all changes commit or none do.
struct StepStateTransaction {
    retry_count: Option<u32>,
    last_error: Option<String>,
    last_reflection: Option<String>,
    key_findings: Option<Vec<String>>,
    step_outputs_entry: Option<serde_json::Value>,
    // Progress tracking deltas (Phase 0B)
    progress_tool_calls_delta: u32,
    progress_tokens_delta: u64,
}

impl StepStateTransaction {
    fn new() -> Self {
        Self {
            retry_count: None,
            last_error: None,
            last_reflection: None,
            key_findings: None,
            step_outputs_entry: None,
            progress_tool_calls_delta: 0,
            progress_tokens_delta: 0,
        }
    }

    fn with_retry_count(mut self, count: u32) -> Self {
        self.retry_count = Some(count);
        self
    }

    fn with_error(mut self, error: String) -> Self {
        self.last_error = Some(error);
        self
    }

    fn with_reflection(mut self, reflection: String) -> Self {
        self.last_reflection = Some(reflection);
        self
    }

    fn with_key_findings(mut self, findings: Vec<String>) -> Self {
        self.key_findings = Some(findings);
        self
    }

    fn with_step_output(mut self, entry: serde_json::Value) -> Self {
        self.step_outputs_entry = Some(entry);
        self
    }

    fn with_progress(mut self, tool_calls: u32, tokens: u64) -> Self {
        self.progress_tool_calls_delta = tool_calls;
        self.progress_tokens_delta = tokens;
        self
    }

    /// Atomically commit all mutations to state.
    /// Call this once at the end of step processing to ensure consistency.
    fn commit(self, state: &mut AgentState) {
        if let Some(count) = self.retry_count {
            state.metadata["retry_count"] = serde_json::json!(count);
        }

        if let Some(ref error) = self.last_error {
            state.metadata["last_step_error"] = serde_json::Value::String(error.clone());
        } else {
            // Clear error on success
            state.metadata.as_object_mut().map(|m| m.remove("last_step_error"));
        }

        if let Some(ref reflection) = self.last_reflection {
            state.metadata["last_reflection"] = serde_json::Value::String(reflection.clone());
        }

        if let Some(ref findings) = self.key_findings {
            state.metadata["key_findings"] = serde_json::json!(findings);
        }

        if let Some(ref entry) = self.step_outputs_entry {
            let mut outputs =
                state.metadata.get("step_outputs").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            outputs.push(entry.clone());
            // Phase 0C: Cap the metadata array to prevent unbounded growth.
            // Full outputs are preserved on disk as artifact files.
            if outputs.len() > STEP_OUTPUTS_METADATA_CAP {
                let excess = outputs.len() - STEP_OUTPUTS_METADATA_CAP;
                outputs.drain(..excess);
            }
            state.metadata["step_outputs"] = serde_json::Value::Array(outputs);
        }

        // Phase 0B: Progress tracking — accumulate deltas
        if self.progress_tool_calls_delta > 0 || self.progress_tokens_delta > 0 {
            let prev_tool_count = state.metadata.get("progress_tool_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let prev_token_count = state.metadata.get("progress_token_count").and_then(|v| v.as_u64()).unwrap_or(0);

            state.metadata["progress_tool_count"] =
                serde_json::json!(prev_tool_count + self.progress_tool_calls_delta as u64);
            state.metadata["progress_token_count"] = serde_json::json!(prev_token_count + self.progress_tokens_delta);
            state.metadata["progress_last_step"] = serde_json::json!(state.current_step);
            state.metadata["progress_updated_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
        }

        tracing::debug!(
            "step state transaction committed: {} fields updated",
            [
                self.retry_count.is_some(),
                self.last_error.is_some(),
                self.last_reflection.is_some(),
                self.key_findings.is_some(),
                self.step_outputs_entry.is_some(),
            ]
            .iter()
            .filter(|&&b| b)
            .count()
        );
    }
}

fn persist_step_output(
    state: &mut AgentState,
    step: &crate::agent::planner::PlannedStep,
    result: &crate::agent::executor::StepResult,
) {
    // Phase 0A: Full output goes to disk artifact.
    // The async write is fire-and-forget (best-effort); the compact pointer
    // in metadata is the durable record.
    let workspace = state.workspace_path.clone();
    let agent_id = state.id.clone();
    let step_index = step.index;
    let full_record = serde_json::json!({
        "step_index": step.index,
        "description": step.description,
        "success": result.success,
        "output": result.output,
        "final_answer_candidate": result.final_answer_candidate,
        "tools_called": result.tools_called,
        "tool_results": result.tool_results,
    });
    tokio::spawn(async move {
        if let Err(e) = crate::agent::step_artifacts::write_step_artifact(
            Path::new(&workspace),
            &agent_id,
            step_index,
            &full_record,
        )
        .await
        {
            tracing::warn!(step_index, error = %e, "failed to write step artifact");
        }
    });

    // Compact pointer → metadata JSONB (Phase 0A)
    let artifact_path =
        crate::agent::step_artifacts::step_artifact_path(Path::new(&state.workspace_path), &state.id, step.index);
    let pointer = crate::agent::step_artifacts::compact_step_pointer(
        step.index,
        &step.description,
        result.success,
        &result.output,
        &artifact_path,
        &result.tools_called,
    );

    if let Some(outputs) = state.metadata.get_mut("step_outputs").and_then(|value| value.as_array_mut()) {
        if outputs.len() <= step.index {
            outputs.resize(step.index + 1, serde_json::Value::Null);
        }
        outputs[step.index] = pointer;
    } else {
        let mut outputs = Vec::new();
        outputs.resize(step.index + 1, serde_json::Value::Null);
        outputs[step.index] = pointer;
        state.metadata["step_outputs"] = serde_json::Value::Array(outputs);
    }
}

fn persist_skipped_step_output(state: &mut AgentState, step: &crate::agent::planner::PlannedStep, summary: &str) {
    // Skipped steps don't need a full artifact file — just the compact pointer.
    let artifact_path =
        crate::agent::step_artifacts::step_artifact_path(Path::new(&state.workspace_path), &state.id, step.index);
    let pointer = crate::agent::step_artifacts::compact_step_pointer(
        step.index,
        &step.description,
        true,
        summary,
        &artifact_path,
        &[],
    );

    if let Some(outputs) = state.metadata.get_mut("step_outputs").and_then(|value| value.as_array_mut()) {
        if outputs.len() <= step.index {
            outputs.resize(step.index + 1, serde_json::Value::Null);
        }
        outputs[step.index] = pointer;
    } else {
        let mut outputs = Vec::new();
        outputs.resize(step.index + 1, serde_json::Value::Null);
        outputs[step.index] = pointer;
        state.metadata["step_outputs"] = serde_json::Value::Array(outputs);
    }
}

fn persist_judgement_signal(state: &mut AgentState, signal: &JudgementSignal) {
    let record = serde_json::json!({
        "step_index": signal.step_index,
        "step_description": signal.step_description,
        "job_type": signal.job_type,
        "profile": signal.profile,
        "score": signal.score,
        "confidence": signal.confidence,
        "recommendation": signal.recommendation,
        "summary": signal.summary,
        "reasons": signal.reasons,
        "timestamp": signal.timestamp,
    });

    if let Some(signals) = state.metadata.get_mut("judgement_signals").and_then(|value| value.as_array_mut()) {
        signals.push(record);
    } else {
        state.metadata["judgement_signals"] = serde_json::json!([record]);
    }
}

fn condition_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(boolean) => *boolean,
        serde_json::Value::Number(number) => number.as_f64().map(|number| number != 0.0).unwrap_or(false),
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(map) => !map.is_empty(),
    }
}

fn condition_contains(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    match (actual, expected) {
        (serde_json::Value::String(actual), serde_json::Value::String(expected)) => {
            actual.to_ascii_lowercase().contains(&expected.to_ascii_lowercase())
        }
        (serde_json::Value::Array(items), expected) => items.iter().any(|item| item == expected),
        (serde_json::Value::Object(map), serde_json::Value::String(expected)) => map.contains_key(expected),
        _ => false,
    }
}

fn condition_compare_numbers(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    cmp: impl Fn(f64, f64) -> bool,
) -> Result<bool> {
    let actual =
        actual.as_f64().ok_or_else(|| anyhow::anyhow!("condition comparison requires numeric actual value"))?;
    let expected =
        expected.as_f64().ok_or_else(|| anyhow::anyhow!("condition comparison requires numeric expected value"))?;
    Ok(cmp(actual, expected))
}

fn format_step_condition(step: &crate::agent::planner::PlannedStep) -> Option<String> {
    step.condition.as_ref().map(|condition| match condition {
        crate::agent::planner::StepCondition::Deterministic(cond) => {
            let value = cond
                .right
                .as_ref()
                .map(|value| match value {
                    serde_json::Value::String(text) => format!(" \"{text}\""),
                    other => format!(" {}", other),
                })
                .unwrap_or_default();
            format!("{} {:?}{}", cond.left, cond.operator, value)
        }
        crate::agent::planner::StepCondition::Expression(expr) => {
            serde_json::to_string(expr).unwrap_or_else(|_| "{}".into())
        }
    })
}

fn plan_step_event(step: &crate::agent::planner::PlannedStep) -> crate::events::PlanStepEvent {
    crate::events::PlanStepEvent {
        step_index: step.index,
        description: step.description.clone(),
        tool: step.tool.clone(),
        success_criteria: step.success_criteria.clone(),
        condition: format_step_condition(step),
    }
}

// ── Outcome ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum StepOutcome {
    Continue {
        delay_secs: i64,
    },
    NeedsClarification {
        questions: Vec<crate::agent::clarifier::ClarificationQuestion>,
    },
    /// Plan created and stored; waiting for user to approve before execution.
    PlanApprovalNeeded,
    Infeasible {
        reason: String,
    },
    Complete,
    /// Run ended but not all CompletionCriteria were satisfied.
    PartiallyComplete {
        note: String,
    },
    /// Generic failure — consider using more specific variants when possible
    Failed(String),
    /// Transient error that should retry without evaluator LLM call
    /// (connection timeout, temporary service unavailable, etc.)
    TransientError {
        reason: String,
        retry_after_secs: u64,
    },
    /// Permanent error that LLM cannot resolve
    /// (missing credentials, invalid schema, tool not found, etc.)
    PermanentError {
        reason: String,
    },
    /// Policy or plane guard violation — needs role/permission review
    PolicyViolation {
        reason: String,
    },
    /// Rate limited by external service
    RateLimited {
        retry_after_secs: u64,
        reason: String,
    },
    Delegating {
        child_ids: Vec<String>,
    },
}

fn is_missing_provider_credentials_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("has no provider credentials configured")
        || message.contains("no platform fallback is available")
        || message.contains("call put /credentials")
}

fn provider_credentials_questions() -> Vec<crate::agent::clarifier::ClarificationQuestion> {
    vec![crate::agent::clarifier::ClarificationQuestion {
        id: "llm_provider_credentials".into(),
        question_type: Some("approval".into()),
        prompt: "Add an LLM provider API key to continue.".into(),
        placeholder: Some("After adding credentials in Settings, click Submit to retry.".into()),
        helper_text: Some(
            "This workspace does not have any tenant provider credentials configured, and no platform fallback key is available."
                .into(),
        ),
        options: Vec::new(),
        multi_select: false,
        recommended: Vec::new(),
        preview: None,
        required: false,
        secret: false,
        store_as_credential: None,
        connector_type: Some("provider_credentials".into()),
        action_label: Some("Open Settings -> Credentials".into()),
        card_type: Some("provider_credentials".into()),
        required_fields: vec!["api_key".into()],
        binding_target: Some("provider_credentials".into()),
        resume_token: Some("provider_credentials".into()),
    }]
}

// ── AgentLoop ──────────────────────────────────────────────────────────────

pub struct AgentLoop {
    executor: Arc<dyn Executor>,
    evaluator: Arc<dyn Evaluator>,
    reflector: Arc<dyn Reflector>,
    preflight: Arc<dyn Preflight>,
    clarifier: Arc<dyn Clarifier>,
    tools: Arc<ToolRegistry>,
    event_bus: Arc<EventBus>,
    skill_registry: Arc<RwLock<SkillRegistry>>,
    knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
    vector_store: Arc<dyn crate::memory::VectorStore>,
    embedder: Arc<dyn crate::memory::EmbeddingModel>,
    memory_consolidator: Option<Arc<crate::memory::MemoryConsolidator>>,
    services: Arc<AgentServices>,
    store: Option<Arc<crate::storage::PostgresStore>>,
    /// DAG workflow persistence — when set, enables DAG engine routing.
    workflow_store: Option<Arc<dyn crate::storage::WorkflowStore>>,
    max_steps: usize,
    timeout_secs: u64,
}

impl AgentLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        executor: Arc<dyn Executor>,
        evaluator: Arc<dyn Evaluator>,
        reflector: Arc<dyn Reflector>,
        preflight: Arc<dyn Preflight>,
        clarifier: Arc<dyn Clarifier>,
        tools: Arc<ToolRegistry>,
        event_bus: Arc<EventBus>,
        skill_registry: Arc<RwLock<SkillRegistry>>,
        knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
        vector_store: Arc<dyn crate::memory::VectorStore>,
        embedder: Arc<dyn crate::memory::EmbeddingModel>,
        services: Arc<AgentServices>,
    ) -> Self {
        Self {
            executor,
            evaluator,
            reflector,
            preflight,
            clarifier,
            tools,
            event_bus,
            skill_registry,
            knowledge_graph,
            vector_store,
            embedder,
            memory_consolidator: None,
            services,
            store: None,
            workflow_store: None,
            max_steps: 50,
            timeout_secs: 300,
        }
    }

    pub fn with_store(mut self, store: Arc<crate::storage::PostgresStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_memory_consolidator(mut self, consolidator: Arc<crate::memory::MemoryConsolidator>) -> Self {
        self.memory_consolidator = Some(consolidator);
        self
    }

    pub fn with_limits(mut self, max_steps: usize, timeout_secs: u64) -> Self {
        self.max_steps = max_steps;
        self.timeout_secs = timeout_secs;
        self
    }

    pub fn with_workflow_store(mut self, store: Arc<dyn crate::storage::WorkflowStore>) -> Self {
        self.workflow_store = Some(store);
        self
    }

    async fn send_agent_message(&self, message: AgentMessage) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        if let Err(error) = store.create_agent_message(&message).await {
            tracing::warn!(
                sender = %message.sender_agent_id,
                recipient = %message.recipient_agent_id,
                error = %error,
                "failed to persist agent message"
            );
            return;
        }

        self.event_bus.publish(AgentEvent::AgentMessageSent {
            agent_id: message.sender_agent_id.clone(),
            recipient_agent_id: message.recipient_agent_id.clone(),
            message_kind: format!("{:?}", message.message_kind).to_ascii_lowercase(),
            task_id: message.task_id.clone(),
            has_result_contract: message.has_result_contract(),
        });
        self.event_bus.publish(AgentEvent::AgentMessageReceived {
            agent_id: message.recipient_agent_id.clone(),
            sender_agent_id: message.sender_agent_id.clone(),
            message_kind: format!("{:?}", message.message_kind).to_ascii_lowercase(),
            task_id: message.task_id.clone(),
            has_result_contract: message.has_result_contract(),
        });
    }

    async fn notify_parent_of_terminal_result(
        &self,
        state: &AgentState,
        status: SessionTaskResultStatus,
        note: impl Into<String>,
        findings: Vec<String>,
        confidence: f64,
    ) {
        let Some(parent_agent_id) = state.parent_agent_id.clone() else {
            return;
        };

        let note = note.into();
        let task_id = state
            .metadata
            .get("delegation_context")
            .and_then(|value| value.get("task_id"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| state.current_task.clone());
        let artifacts =
            if state.workspace_path.trim().is_empty() { Vec::new() } else { vec![state.workspace_path.clone()] };

        let mut message = AgentMessage::new(
            uuid::Uuid::new_v4().to_string(),
            state.tenant_id.clone(),
            state.id.clone(),
            parent_agent_id,
            AgentMessageKind::Result,
            "worker_result",
            note.clone(),
        );
        message.task_id = task_id;
        message.step_index = Some(state.current_step);
        message.metadata = serde_json::json!({
            "auto_generated": true,
            "worker_type": state
                .metadata
                .get("delegation_context")
                .and_then(|value| value.get("worker_type"))
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "write_scope": state
                .metadata
                .get("delegation_context")
                .and_then(|value| value.get("write_scope"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        });
        message.result_contract = Some(SessionTaskOutput {
            status,
            artifacts,
            findings,
            confidence: confidence.clamp(0.0, 1.0),
            note: Some(note),
        });
        self.send_agent_message(message).await;
    }

    async fn maybe_consolidate_memory(&self, state: &mut AgentState) {
        let Some(consolidator) = self.memory_consolidator.as_ref() else {
            return;
        };
        match consolidator.consolidate_agent(state, false).await {
            Ok(result) => {
                crate::memory::apply_consolidation_metadata(state, &result);
                tracing::debug!(
                    agent_id = %state.id,
                    changed = result.changed,
                    skipped = result.skipped,
                    topics_saved = result.topics_saved.len(),
                    pruned_topics = result.pruned_topics.len(),
                    "memory consolidation completed"
                );
            }
            Err(error) => {
                tracing::warn!(agent_id = %state.id, error = %error, "memory consolidation failed");
            }
        }
    }

    /// Execute exactly one step of the agent state machine.
    /// Called by the worker — NEVER runs a continuous loop itself.
    pub async fn run_step(
        &self,
        state: &mut AgentState,
        plan: &mut Option<Plan>,
        history: &mut StepHistory,
    ) -> Result<StepOutcome> {
        tracing::info!(
            agent_id = %state.id,
            status = ?state.status,
            current_step = state.current_step,
            has_plan = plan.is_some(),
            goal = %truncate_for_log(&state.goal, 200),
            "agent loop step starting"
        );

        // ── 1. Cognitive control — prevent infinite loops ──────────────────
        let control = CognitiveControlLoop::new(self.max_steps, self.timeout_secs);
        if !control.should_continue(state) {
            let reason = format!(
                "agent exceeded safety limits ({} steps / {}s timeout) — aborting to prevent infinite loop",
                self.max_steps, self.timeout_secs
            );
            tracing::error!(agent_id = %state.id, reason = %reason, "cognitive control limit hit");
            state.mark_failed();
            self.notify_parent_of_terminal_result(
                state,
                SessionTaskResultStatus::Failed,
                reason.clone(),
                vec![reason.clone()],
                1.0,
            )
            .await;
            self.event_bus.publish(AgentEvent::GoalFailed { agent_id: state.id.clone(), reason: reason.clone() });
            self.event_bus.close(&state.id);
            return Ok(StepOutcome::Failed(reason));
        }

        // ── 2. Pre-flight (first run only) ─────────────────────────────────
        if state.status == AgentStatus::Pending {
            return self.run_preflight(state).await;
        }

        // ── 3. Clarification — wait for user answer ─────────────────────────
        if state.status == AgentStatus::Clarifying {
            return Ok(StepOutcome::NeedsClarification {
                questions: state
                    .metadata
                    .get("clarification_questions")
                    .map(crate::agent::clarifier::parse_clarification_questions)
                    .unwrap_or_default(),
            });
        }

        // ── 3b. Plan approval — wait for user sign-off ──────────────────────
        if state.status == AgentStatus::PlanApprovalNeeded {
            return Ok(StepOutcome::PlanApprovalNeeded);
        }

        // ── 4. Planning ─────────────────────────────────────────────────────
        if plan.is_none() {
            self.event_bus.publish(AgentEvent::PlanningStarted { agent_id: state.id.clone() });

            if state.metadata.get("refined_goal").is_none() {
                if let Some(answer_value) = state.metadata.get("clarification_answers").cloned() {
                    if let Ok(answers) =
                        serde_json::from_value::<crate::agent::clarifier::ClarificationAnswers>(answer_value)
                    {
                        match self.clarifier.incorporate(state, &answers).await {
                            Ok(refined_goal) => state.metadata["refined_goal"] = serde_json::json!(refined_goal),
                            Err(error) => tracing::warn!(
                                agent_id = %state.id,
                                error = %error,
                                "failed to incorporate clarification answers"
                            ),
                        }
                    }
                }
            }

            let _tool_names: Vec<&str> = self.tools.list();

            // 4a. Check skill registry first — skip LLM planning if a skill matches
            let maybe_skill = {
                let reg = self.skill_registry.read().await;
                reg.get(&state.goal).cloned().or_else(|| {
                    // Fuzzy match: check if any skill name appears in the goal
                    reg.find_matching(&state.goal).cloned()
                })
            };

            let new_plan = if is_direct_response_goal(&state.goal) {
                tracing::info!(
                    agent_id = %state.id,
                    goal = %state.goal,
                    "agent loop selected direct-response fast path"
                );
                Plan {
                    goal: state.goal.clone(),
                    job_type: Some("general".into()),
                    rationale: "Simple conversational request; answer directly without tools.".into(),
                    steps: vec![crate::agent::planner::PlannedStep {
                        foreach: None,
                        index: 0,
                        description: "Answer the user's message directly in chat.".into(),
                        tool: Some(crate::agent::workflow_compiler::LLM_WORKER_TOOL_NAME.into()),
                        tool_args: Some(serde_json::json!({
                            "instruction": "Answer the user's message directly in chat.",
                            "response_format": "text",
                        })),
                        success_criteria: "User receives a direct answer.".into(),
                        condition: None,
                        depends_on: vec![],
                    }],
                }
            } else if let Some(skill) = maybe_skill {
                tracing::info!(
                    agent_id = %state.id,
                    skill    = %skill.name,
                    "using pre-built skill — skipping LLM planning"
                );
                Plan::from_skill(&skill)
            } else if let Some(workflow_plan) = self.try_plan_from_compiled_workflow(state).await {
                tracing::info!(
                    agent_id = %state.id,
                    steps    = workflow_plan.steps.len(),
                    "using compiled workflow artifact — skipping LLM planning"
                );
                workflow_plan
            } else {
                // Use refined goal if user answered clarification questions
                let refined = state
                    .metadata
                    .get("refined_goal")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| state.goal.clone());
                let orig = state.goal.clone();
                if refined != orig {
                    state.goal = refined;
                }
                let reason = state
                    .metadata
                    .get("recompile_reason")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| {
                        format!(
                            "runtime does not invent plans anymore; rerun plan mode to produce a compiled workflow artifact for '{}'",
                            state.goal
                        )
                    });
                tracing::error!(
                    agent_id = %state.id,
                    goal = %state.goal,
                    reason = %reason,
                    "no deterministic runtime plan available"
                );
                state.goal = orig;
                state.mark_failed();
                self.notify_parent_of_terminal_result(
                    state,
                    SessionTaskResultStatus::Failed,
                    reason.clone(),
                    vec![reason.clone()],
                    1.0,
                )
                .await;
                self.event_bus.publish(AgentEvent::GoalFailed { agent_id: state.id.clone(), reason: reason.clone() });
                self.event_bus.close(&state.id);
                return Ok(StepOutcome::Failed(reason));
            };

            self.event_bus.publish(AgentEvent::PlanCreated {
                agent_id: state.id.clone(),
                step_count: new_plan.steps.len(),
                rationale: new_plan.rationale.clone(),
                job_type: new_plan.job_type.clone(),
                steps: new_plan.steps.iter().map(plan_step_event).collect(),
            });

            tracing::info!(
                agent_id = %state.id,
                rationale = %new_plan.rationale,
                steps = new_plan.steps.len(),
                "auto-approving deterministic plan — skipping approval gate"
            );
            self.sync_session_tasks_for_plan(state, &new_plan).await;
            *plan = Some(new_plan);
        }

        // ── 4c. DAG routing — fan-out/fan-in workflows ──────────────────────
        // If we have a workflow store, route deterministic plans through the
        // DagEngine by default. Explicit DAG edges are preserved, while
        // ordinary plans fall back to sequential execution inside the engine.
        if let (Some(wf_store), Some(current_plan)) = (&self.workflow_store, plan.as_ref()) {
            let not_already_running = state.workflow_id.is_none();

            if not_already_running {
                tracing::info!(
                    agent_id = %state.id,
                    steps = current_plan.steps.len(),
                    "workflow persistence available — routing plan to DagEngine"
                );

                let workflow = crate::agent::dag::Workflow::from_plan(current_plan, &state.id, &state.tenant_id);
                state.workflow_id = Some(workflow.id.clone());

                // Persist workflow to DB before execution
                if let Err(e) = wf_store.create_workflow(&workflow).await {
                    tracing::error!(error = %e, "failed to persist workflow — falling back to linear");
                    state.workflow_id = None;
                } else {
                    // Run the DAG engine to completion — with full orchestrator hooks
                    let mut orch = crate::agent::orchestrator::StepOrchestrator::new(
                        Arc::clone(&self.executor),
                        Arc::clone(&self.event_bus),
                        Arc::clone(&self.knowledge_graph),
                        Arc::clone(&self.services),
                        Arc::clone(&self.vector_store),
                        Arc::clone(&self.embedder),
                    );
                    if let Some(ref store) = self.store {
                        orch = orch.with_store(Arc::clone(store));
                    }
                    let dag_engine = crate::agent::dag_engine::DagEngine::new(
                        Arc::clone(&self.executor),
                        Arc::clone(wf_store),
                        Arc::clone(&self.event_bus),
                    )
                    .with_orchestrator(Arc::new(orch));
                    let cancel = tokio_util::sync::CancellationToken::new();
                    match dag_engine.run_workflow(state, cancel).await {
                        Ok(crate::agent::dag_engine::WorkflowOutcome::Completed) => {
                            state.workflow_id = None;
                            let summary = completion_summary(state);
                            state.mark_completed();
                            self.event_bus.publish(AgentEvent::GoalComplete {
                                agent_id: state.id.clone(),
                                summary: summary.clone(),
                            });
                            self.event_bus.close(&state.id);
                            return Ok(StepOutcome::Complete);
                        }
                        Ok(crate::agent::dag_engine::WorkflowOutcome::Cancelled) => {
                            state.workflow_id = None;
                            state.mark_failed();
                            return Ok(StepOutcome::Failed("DAG workflow cancelled".into()));
                        }
                        Ok(crate::agent::dag_engine::WorkflowOutcome::Failed(reason)) => {
                            state.workflow_id = None;
                            state.mark_failed();
                            self.event_bus
                                .publish(AgentEvent::GoalFailed { agent_id: state.id.clone(), reason: reason.clone() });
                            self.event_bus.close(&state.id);
                            return Ok(StepOutcome::Failed(reason));
                        }
                        Err(e) => {
                            let reason = format!("DAG engine error: {:#}", e);
                            state.workflow_id = None;
                            state.mark_failed();
                            self.event_bus
                                .publish(AgentEvent::GoalFailed { agent_id: state.id.clone(), reason: reason.clone() });
                            self.event_bus.close(&state.id);
                            return Ok(StepOutcome::Failed(reason));
                        }
                    }
                }
            }
        }

        let current_plan = plan.as_ref().unwrap();

        // ── 5. Completion check ─────────────────────────────────────────────
        if current_plan.is_complete(state.current_step as usize) {
            let summary = completion_summary(state);
            tracing::info!(
                agent_id = %state.id,
                summary = %truncate_for_log(&summary, 300),
                "agent loop reached plan completion"
            );
            state.mark_completed();
            self.maybe_consolidate_memory(state).await;
            self.notify_parent_of_terminal_result(
                state,
                SessionTaskResultStatus::Complete,
                summary.clone(),
                state
                    .metadata
                    .get("key_findings")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items.iter().filter_map(|value| value.as_str().map(str::to_string)).collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| vec![summary.clone()]),
                1.0,
            )
            .await;

            // Emit RoleCompleted if this agent is executing a specific role
            if let (Some(agent_def_id), Some(role_id), Some(role_name)) = (
                state.metadata.get("agent_definition_id").and_then(|v| v.as_str()),
                state.metadata.get("role_id").and_then(|v| v.as_str()),
                state.metadata.get("role_name").and_then(|v| v.as_str()),
            ) {
                let output_data = state.metadata.get("final_output").cloned().unwrap_or(serde_json::json!({}));
                self.event_bus.publish(AgentEvent::RoleCompleted {
                    agent_definition_id: agent_def_id.to_string(),
                    role_id: role_id.to_string(),
                    role_name: role_name.to_string(),
                    output_data,
                });
                tracing::info!(
                    agent_definition_id = %agent_def_id,
                    role_id = %role_id,
                    role_name = %role_name,
                    "RoleCompleted event emitted for workforce event subscriptions"
                );
            }

            self.event_bus.publish(AgentEvent::GoalComplete { agent_id: state.id.clone(), summary: summary.clone() });
            self.event_bus.close(&state.id);
            return Ok(StepOutcome::Complete);
        }

        let step = current_plan.next_step(state.current_step as usize).unwrap().clone();

        if let Some(summary) = self.evaluate_step_condition(state, &step)? {
            persist_skipped_step_output(state, &step, &summary);
            state.metadata["last_reflection"] = serde_json::Value::String(summary.clone());
            state.metadata["key_findings"] = serde_json::json!([]);
            state.metadata["retry_count"] = serde_json::json!(0);
            state.metadata.as_object_mut().map(|object| object.remove("last_step_error"));
            history.push(step.index, step.description.clone(), true, &summary);
            state.advance_step();
            state.mark_waiting(next_run_after(0));
            self.mark_step_task_finished(
                state,
                &step,
                SessionTaskStatus::Completed,
                Some(SessionTaskOutput {
                    status: SessionTaskResultStatus::Complete,
                    artifacts: Vec::new(),
                    findings: vec![summary.clone()],
                    confidence: 1.0,
                    note: Some("step skipped by deterministic condition".into()),
                }),
            )
            .await;
            self.event_bus.publish(AgentEvent::StepCompleted {
                agent_id: state.id.clone(),
                step_index: step.index,
                success: true,
                summary,
                description: Some(step.description.clone()),
            });
            return Ok(StepOutcome::Continue { delay_secs: 0 });
        }

        let orchestrator = crate::agent::orchestrator::StepOrchestrator::new(
            self.executor.clone(),
            self.event_bus.clone(),
            self.knowledge_graph.clone(),
            self.services.clone(),
            self.vector_store.clone(),
            self.embedder.clone(),
        );
        let orchestrator = match &self.store {
            Some(s) => orchestrator.with_store(s.clone()),
            None => orchestrator,
        };

        state.mark_running();
        self.mark_step_task_in_progress(state, &step).await;

        let verdict = orchestrator.run_step(state, &step, current_plan, history).await;

        let result = match verdict {
            crate::agent::orchestrator::StepVerdict::Executed { result, .. } => result,
            crate::agent::orchestrator::StepVerdict::Skipped { .. } => {
                return Ok(StepOutcome::Continue { delay_secs: 0 })
            }
            crate::agent::orchestrator::StepVerdict::Delegating { child_ids, .. } => {
                state.advance_step();
                state.mark_delegating(child_ids.clone());
                return Ok(StepOutcome::Delegating { child_ids });
            }
            crate::agent::orchestrator::StepVerdict::NeedsClarification { questions, .. } => {
                state.metadata["clarification_questions"] = serde_json::to_value(&questions)?;
                state.mark_clarifying();
                return Ok(StepOutcome::NeedsClarification { questions });
            }
            crate::agent::orchestrator::StepVerdict::DeterministicAbort { reason, .. } => {
                state.metadata["last_reflection"] =
                    serde_json::Value::String("Aborted by deterministic FailureRule".into());
                state.metadata["key_findings"] = serde_json::json!([]);
                state.mark_failed();
                let display_reason = format!("Step {} blocked by policy/permission rule: {}", step.index, reason);
                self.notify_parent_of_terminal_result(
                    state,
                    SessionTaskResultStatus::Failed,
                    display_reason.clone(),
                    vec![display_reason.clone()],
                    1.0,
                )
                .await;
                self.event_bus
                    .publish(AgentEvent::GoalFailed { agent_id: state.id.clone(), reason: display_reason.clone() });
                self.event_bus.close(&state.id);
                return Ok(StepOutcome::PermanentError { reason: display_reason });
            }
            crate::agent::orchestrator::StepVerdict::Error { error } => {
                if is_missing_provider_credentials_error(&error) {
                    return self.prompt_for_provider_credentials(state);
                }
                return Err(error);
            }
        };

        // ── Direct response fast path ──────────────────────────────────────────
        if is_direct_response_goal(&state.goal)
            && current_plan.steps.len() == 1
            && step.tool.as_deref() == Some(crate::agent::workflow_compiler::LLM_WORKER_TOOL_NAME)
            && result.success
            && result.tool_results.is_empty()
        {
            let answer = state.final_answer().map(str::to_string).unwrap_or_else(|| result.output.clone());
            state.set_final_answer(answer.clone());
            state.metadata["last_reflection"] = serde_json::Value::String(answer.clone());
            state.metadata["key_findings"] = serde_json::json!([]);
            history.push(step.index, step.description.clone(), true, &answer);
            state.mark_completed();
            self.maybe_consolidate_memory(state).await;
            self.notify_parent_of_terminal_result(
                state,
                SessionTaskResultStatus::Complete,
                answer.clone(),
                vec![answer.clone()],
                1.0,
            )
            .await;
            self.event_bus.publish(AgentEvent::StepCompleted {
                agent_id: state.id.clone(),
                step_index: step.index,
                success: true,
                summary: "Direct response delivered".into(),
                description: Some(step.description.clone()),
            });
            self.event_bus.publish(AgentEvent::GoalComplete { agent_id: state.id.clone(), summary: answer });
            self.event_bus.close(&state.id);
            return Ok(StepOutcome::Complete);
        }

        // ── 9b. Evaluate + Reflect (LLM) ──────────────────────────────────
        let retry_count = state.metadata.get("retry_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let eval = self.evaluator.evaluate_and_reflect(state, current_plan, &step, &result, retry_count, 3).await?;

        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            verdict = ?eval.verdict,
            summary = %truncate_for_log(&eval.summary, 300),
            should_revise = eval.should_revise,
            "agent loop evaluation complete"
        );

        self.event_bus.publish(AgentEvent::EvaluationComplete {
            agent_id: state.id.clone(),
            step_index: step.index,
            verdict: format!("{:?}", eval.verdict),
            summary: eval.summary.clone(),
            key_findings: eval.key_findings.clone(),
        });

        state.metadata["last_reflection"] = serde_json::Value::String(eval.summary.clone());
        state.metadata["key_findings"] = serde_json::json!(eval.key_findings);

        // Update step history for next executor call
        let history_summary = step_history_summary(&result, &eval.summary);
        history.push(step.index, step.description.clone(), result.success, &history_summary);

        let eval_verdict = eval.verdict.clone();
        let judgement = JudgementEngine::default().evaluate(JudgementContext {
            state,
            plan: current_plan,
            step: &step,
            result: &result,
            eval: &eval,
            eval_verdict: eval_verdict.clone(),
            retry_count,
        });
        state.metadata["last_judgement"] = serde_json::to_value(&judgement).unwrap_or_default();
        if !matches!(judgement.recommendation, JudgementRecommendation::Continue) {
            persist_judgement_signal(state, &judgement);
            let recommendation = match judgement.recommendation {
                JudgementRecommendation::Continue => "continue",
                JudgementRecommendation::Watch => "watch",
                JudgementRecommendation::Revise => "revise",
                JudgementRecommendation::Escalate => "escalate",
            }
            .to_string();
            self.event_bus.publish(AgentEvent::JudgementSignal {
                agent_id: state.id.clone(),
                step_index: judgement.step_index,
                step_description: judgement.step_description.clone(),
                job_type: judgement.job_type.clone(),
                profile: judgement.profile.clone(),
                score: judgement.score,
                confidence: judgement.confidence,
                recommendation,
                summary: judgement.summary.clone(),
                reasons: judgement.reasons.clone(),
                timestamp: judgement.timestamp.clone(),
            });
        }

        // ── 12. Skill evolution ──────────────────────────────────────────────
        if result.success {
            let skill_name = state.metadata.get("active_skill").and_then(|v| v.as_str()).map(String::from);

            if let Some(ref name) = skill_name {
                let maybe_old = {
                    let reg = self.skill_registry.read().await;
                    reg.get(name).cloned()
                };
                if let Some(old_skill) = maybe_old {
                    let improvements: Vec<String> = result
                        .tool_results
                        .iter()
                        .filter(|r| r.success)
                        .filter_map(|r| r.output.get("text").and_then(|v| v.as_str()))
                        .map(|s| crate::util::truncate(s, 80).to_string())
                        .take(2)
                        .collect();

                    if !improvements.is_empty() {
                        let evolved = evolve_skill(&old_skill, improvements);
                        tracing::info!(agent_id = %state.id, skill = %evolved.name, "skill evolved");
                        let mut reg = self.skill_registry.write().await;
                        reg.register(evolved);
                    }
                }
            }
        }

        if eval.should_revise && !eval.revision_feedback.is_empty() {
            tracing::info!(
                agent_id = %state.id,
                step_index = step.index,
                "runtime revision requested by evaluator; leaving repair to plan mode"
            );
        }

        // ── 12d. SLA check — fire escalation actions if threshold crossed ────
        if let Some(ref sla) = self.services.sla {
            if let Some(sla_val) = state.metadata.get("sla_status").cloned() {
                if let Ok(sla_status) = serde_json::from_value::<SlaStatus>(sla_val) {
                    let actions = sla.check(&sla_status);
                    // Compute pct_elapsed from the actual SlaStatus fields
                    let now = chrono::Utc::now();
                    let total = (sla_status.resolution_deadline - sla_status.started_at).num_seconds().max(1) as f64;
                    let elapsed = (now - sla_status.started_at).num_seconds().max(0) as f64;
                    let pct_elapsed = (elapsed / total * 100.0).min(100.0);
                    let deadline_str = sla_status.resolution_deadline.to_rfc3339();
                    for action in &actions {
                        match action {
                            EscalationAction::EscalateToHuman { reason } => {
                                tracing::warn!(
                                    agent_id = %state.id,
                                    reason   = %reason,
                                    "SLA breach — escalating to human review"
                                );
                                self.event_bus.publish(AgentEvent::SlaCheck {
                                    agent_id: state.id.clone(),
                                    pct_elapsed,
                                    message: reason.clone(),
                                    action: Some("escalate".into()),
                                    deadline: Some(deadline_str.clone()),
                                });
                                if let Some(ref rq) = self.services.reviews {
                                    match rq
                                        .submit(&state.tenant_id, &state.id, step.index, reason, "sla_escalation")
                                        .await
                                    {
                                        Ok(review_id) => {
                                            self.event_bus.publish(AgentEvent::ReviewRequired {
                                                agent_id: state.id.clone(),
                                                review_id,
                                                summary: reason.clone(),
                                                reason: "SLA breach escalation".into(),
                                                rule_id: Some("sla_escalation".into()),
                                            });
                                        }
                                        Err(e) => {
                                            tracing::error!(error = %e, "failed to submit SLA review");
                                        }
                                    }
                                }
                            }
                            EscalationAction::Notify { message } => {
                                tracing::warn!(agent_id = %state.id, message = %message, "SLA notification");
                                self.event_bus.publish(AgentEvent::SlaCheck {
                                    agent_id: state.id.clone(),
                                    pct_elapsed,
                                    message: message.clone(),
                                    action: Some("notify".into()),
                                    deadline: Some(deadline_str.clone()),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // ── 13. Advance state ───────────────────────────────────────────────
        match eval_verdict {
            EvalVerdict::Continue => {
                // Reset retry counter and clear error for the new step
                state.metadata["retry_count"] = serde_json::json!(0);
                state.metadata.as_object_mut().map(|m| m.remove("last_step_error"));

                // ── OPTIMIZATION: Check CompletionCriteria mid-run ──────────────────
                // If deterministic criteria are met before all steps, early-exit with success
                let should_early_complete = if let Some(ref store) = self.store {
                    if let Some(role_id) = state.metadata.get("role_id").and_then(|v| v.as_str()) {
                        match store.get_agent_role(&state.tenant_id, role_id).await {
                            Ok(Some(role)) => check_early_completion(state, &role),
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if should_early_complete {
                    tracing::info!(
                        agent_id = %state.id,
                        step_index = step.index,
                        "early completion triggered by mid-run CompletionCriteria check"
                    );
                    if state.final_answer().is_none() && !looks_like_placeholder(&eval.summary) {
                        state.set_final_answer(eval.summary.clone());
                    }
                    state.mark_completed();
                    self.maybe_consolidate_memory(state).await;
                    self.notify_parent_of_terminal_result(
                        state,
                        SessionTaskResultStatus::Complete,
                        eval.summary.clone(),
                        vec![eval.summary.clone()],
                        1.0,
                    )
                    .await;
                    self.mark_step_task_finished(
                        state,
                        &step,
                        SessionTaskStatus::Completed,
                        Some(SessionTaskOutput {
                            status: SessionTaskResultStatus::Complete,
                            artifacts: Vec::new(),
                            findings: vec![eval.summary.clone()],
                            confidence: 1.0,
                            note: Some("completion criteria satisfied early".into()),
                        }),
                    )
                    .await;
                    self.event_bus.publish(AgentEvent::GoalComplete {
                        agent_id: state.id.clone(),
                        summary: "Goal achieved (early completion by criteria)".into(),
                    });
                    self.event_bus.close(&state.id);
                    return Ok(StepOutcome::Complete);
                }

                state.advance_step();
                state.mark_waiting(next_run_after(0));
                self.mark_step_task_finished(
                    state,
                    &step,
                    SessionTaskStatus::Completed,
                    Some(SessionTaskOutput {
                        status: SessionTaskResultStatus::Complete,
                        artifacts: Vec::new(),
                        findings: vec![eval.summary.clone()],
                        confidence: 1.0,
                        note: Some("step completed successfully".into()),
                    }),
                )
                .await;
                self.event_bus.publish(AgentEvent::StepCompleted {
                    agent_id: state.id.clone(),
                    step_index: step.index,
                    success: true,
                    summary: eval.summary,
                    description: Some(step.description.clone()),
                });
                Ok(StepOutcome::Continue { delay_secs: 0 })
            }
            EvalVerdict::GoalComplete => {
                if state.final_answer().is_none() && !looks_like_placeholder(&eval.summary) {
                    state.set_final_answer(eval.summary.clone());
                }
                let summary = completion_summary(state);
                // ── CompletionCriteria check ─────────────────────────────────
                let (all_satisfied, criterion_results) = if let Some(ref store) = self.store {
                    if let Some(role_id) = state.metadata.get("role_id").and_then(|v| v.as_str()) {
                        match store.get_agent_role(&state.tenant_id, role_id).await {
                            Ok(Some(role)) => check_completion_criteria(&role, state),
                            _ => (true, vec![]),
                        }
                    } else {
                        (true, vec![])
                    }
                } else {
                    (true, vec![])
                };

                // Write per-criterion results into goal_instance.result for the UI
                let criteria_json = serde_json::to_value(&criterion_results).unwrap_or_default();
                let base_result = state.metadata.get("step_outputs").cloned().unwrap_or(serde_json::json!({}));
                let judgement_json = state.metadata.get("judgement_signals").cloned().unwrap_or(serde_json::json!([]));
                let enriched_result = serde_json::json!({
                    "step_outputs":   base_result,
                    "judgement_signals": judgement_json,
                    "criteria_checks": criteria_json,
                    "all_criteria_satisfied": all_satisfied,
                });

                if all_satisfied {
                    state.mark_completed();
                    self.maybe_consolidate_memory(state).await;
                    self.notify_parent_of_terminal_result(
                        state,
                        SessionTaskResultStatus::Complete,
                        summary.clone(),
                        vec![summary.clone()],
                        1.0,
                    )
                    .await;
                    self.mark_step_task_finished(
                        state,
                        &step,
                        SessionTaskStatus::Completed,
                        Some(SessionTaskOutput {
                            status: SessionTaskResultStatus::Complete,
                            artifacts: Vec::new(),
                            findings: vec![summary.clone()],
                            confidence: 1.0,
                            note: Some("goal completed and criteria satisfied".into()),
                        }),
                    )
                    .await;
                    // Write criteria results before persisting
                    if let Some(ref store) = self.store {
                        if let Some(gi_id) = state.metadata.get("goal_instance_id").and_then(|v| v.as_str()) {
                            let _ = store.update_goal_instance_result(&state.tenant_id, gi_id, enriched_result).await;
                        }
                    }
                    self.event_bus.publish(AgentEvent::GoalComplete { agent_id: state.id.clone(), summary });
                    self.event_bus.close(&state.id);
                    Ok(StepOutcome::Complete)
                } else {
                    let failed: Vec<&str> =
                        criterion_results.iter().filter(|r| !r.satisfied).map(|r| r.description.as_str()).collect();
                    let note = format!("{} criteria not met: {}", failed.len(), failed.join("; "));
                    tracing::warn!(agent_id = %state.id, note = %note, "goal partially complete");
                    state.mark_partially_complete(note.clone(), enriched_result.clone());
                    self.notify_parent_of_terminal_result(
                        state,
                        SessionTaskResultStatus::Partial,
                        note.clone(),
                        failed.iter().map(|value| value.to_string()).collect(),
                        1.0,
                    )
                    .await;
                    self.mark_step_task_finished(
                        state,
                        &step,
                        SessionTaskStatus::Blocked,
                        Some(SessionTaskOutput {
                            status: SessionTaskResultStatus::Partial,
                            artifacts: Vec::new(),
                            findings: failed.iter().map(|value| value.to_string()).collect(),
                            confidence: 1.0,
                            note: Some(note.clone()),
                        }),
                    )
                    .await;
                    if let Some(ref store) = self.store {
                        if let Some(gi_id) = state.metadata.get("goal_instance_id").and_then(|v| v.as_str()) {
                            let _ = store.update_goal_instance_result(&state.tenant_id, gi_id, enriched_result).await;
                        }
                    }
                    self.event_bus.publish(AgentEvent::GoalComplete {
                        agent_id: state.id.clone(),
                        summary: format!("{} [PARTIAL: {}]", summary, note),
                    });
                    self.event_bus.close(&state.id);
                    Ok(StepOutcome::PartiallyComplete { note })
                }
            }
            EvalVerdict::Retry => {
                // Increment retry counter so evaluator's 3-retry limit works
                let new_retry = retry_count + 1;
                state.metadata["retry_count"] = serde_json::json!(new_retry);

                // Exponential backoff: 10s, 20s, 40s
                let delay = 10i64 * 2i64.pow(retry_count.min(4));
                state.mark_waiting(next_run_after(delay));

                // Store last failure details so the executor can include them on retry
                let last_error = result
                    .tool_results
                    .iter()
                    .filter(|tool_result| !tool_result.success)
                    .filter_map(|tool_result| tool_result.error.clone())
                    .collect::<Vec<_>>()
                    .join(" | ");
                state.metadata["last_step_error"] =
                    serde_json::Value::String(if last_error.is_empty() { eval.summary.clone() } else { last_error });
                self.mark_step_task_finished(
                    state,
                    &step,
                    SessionTaskStatus::Blocked,
                    Some(SessionTaskOutput {
                        status: SessionTaskResultStatus::Partial,
                        artifacts: Vec::new(),
                        findings: vec![eval.summary.clone()],
                        confidence: 0.5,
                        note: Some(format!("retry scheduled after {} seconds", delay)),
                    }),
                )
                .await;

                self.event_bus.publish(AgentEvent::StepRetrying {
                    agent_id: state.id.clone(),
                    step_index: step.index,
                    delay_secs: delay,
                    reason: eval.summary.clone(),
                    retry_count: new_retry,
                });
                tracing::warn!(
                    agent_id = %state.id,
                    step = step.index,
                    retry = new_retry,
                    delay_secs = delay,
                    reason = %truncate_for_log(&eval.summary, 200),
                    "step retrying with backoff"
                );
                Ok(StepOutcome::Continue { delay_secs: delay })
            }
            EvalVerdict::Abort => {
                state.mark_failed();
                let reason = format!("step {} aborted: {}", step.index, eval.summary);
                self.notify_parent_of_terminal_result(
                    state,
                    SessionTaskResultStatus::Failed,
                    reason.clone(),
                    vec![reason.clone()],
                    1.0,
                )
                .await;
                self.mark_step_task_finished(
                    state,
                    &step,
                    SessionTaskStatus::Failed,
                    Some(SessionTaskOutput {
                        status: SessionTaskResultStatus::Failed,
                        artifacts: Vec::new(),
                        findings: vec![reason.clone()],
                        confidence: 1.0,
                        note: Some(eval.summary.clone()),
                    }),
                )
                .await;
                self.event_bus.publish(AgentEvent::GoalFailed { agent_id: state.id.clone(), reason: reason.clone() });
                self.event_bus.close(&state.id);
                // Classify the error for smarter retry/escalation logic
                Ok(classify_error(&reason))
            }
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────────

    async fn run_preflight(&self, state: &mut AgentState) -> Result<StepOutcome> {
        state.mark_preflight();
        self.event_bus.publish(AgentEvent::PreflightStarted { agent_id: state.id.clone() });

        let tool_names: Vec<&str> = self.tools.list();
        let preflight_result = match self.preflight.check(state, &tool_names).await {
            Ok(result) => result,
            Err(error) if is_missing_provider_credentials_error(&error) => {
                return self.prompt_for_provider_credentials(state);
            }
            Err(error) => return Err(error),
        };

        match preflight_result {
            PreflightResult::Feasible => {
                self.event_bus.publish(AgentEvent::PreflightPassed { agent_id: state.id.clone() });
                let clarification = match self.clarifier.check(state).await {
                    Ok(result) => result,
                    Err(error) if is_missing_provider_credentials_error(&error) => {
                        return self.prompt_for_provider_credentials(state);
                    }
                    Err(error) => return Err(error),
                };

                match clarification {
                    ClarificationResult::Clear => {
                        state.mark_waiting(next_run_after(0));
                        Ok(StepOutcome::Continue { delay_secs: 0 })
                    }
                    ClarificationResult::NeedsInput { questions } => {
                        state.metadata["clarification_questions"] = serde_json::to_value(&questions)?;
                        state.mark_clarifying();
                        self.event_bus.publish(AgentEvent::ClarificationNeeded {
                            agent_id: state.id.clone(),
                            questions: questions.clone(),
                        });
                        Ok(StepOutcome::NeedsClarification { questions })
                    }
                }
            }
            PreflightResult::Infeasible { reason, missing_tools } => {
                state.mark_failed();
                let msg = format!(
                    "Goal not achievable. {}. Missing tools: {}",
                    reason,
                    if missing_tools.is_empty() { "none".into() } else { missing_tools.join(", ") }
                );
                self.notify_parent_of_terminal_result(
                    state,
                    SessionTaskResultStatus::Failed,
                    msg.clone(),
                    vec![msg.clone()],
                    1.0,
                )
                .await;
                self.event_bus.publish(AgentEvent::PreflightFailed { agent_id: state.id.clone(), reason: msg.clone() });
                self.event_bus.close(&state.id);
                Ok(StepOutcome::Infeasible { reason: msg })
            }
        }
    }

    /// Try to build a deterministic Plan from the saved compiler artifact.
    /// Returns None if the role has no compiled workflow, allowing the caller
    /// to fail fast instead of invoking the old runtime repair path.
    async fn try_plan_from_compiled_workflow(&self, state: &mut AgentState) -> Option<Plan> {
        let store = self.store.as_ref()?;
        let role_id = state.metadata.get("role_id").and_then(|v| v.as_str())?.to_string();
        let mut role = store.get_agent_role(&state.tenant_id, &role_id).await.ok()??;

        if matches!(
            role.execution_guidelines.execution_strategy,
            crate::agent::definition::ExecutionStrategy::CoordinatorShell
        ) {
            state.metadata["coordinator_shell"] = serde_json::json!({
                "mode": "coordinator_shell",
                "tool_pool": "coordinator",
                "goal": state.goal.clone(),
                "pending_children": state.pending_children.clone(),
                "worker_messages": state.metadata.get("worker_messages").cloned().unwrap_or_else(|| serde_json::json!([])),
                "assembled_at": chrono::Utc::now().to_rfc3339(),
            });
            return Some(self.build_coordinator_shell_plan(state, &role));
        }

        let compiled = role.execution_guidelines.compiled_workflow.as_ref()?;
        let input_data = state.metadata.get("input_data").cloned().unwrap_or_else(|| serde_json::json!({}));
        let data_signature = data_signature_from_value(&input_data);
        state.metadata["workflow_data_signature"] =
            serde_json::to_value(&data_signature).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(policy) = &compiled.variant_policy {
            if let Some(selection) = policy.select(
                &data_signature,
                &compiled.execution,
                &compiled.execution_constraints,
                &compiled.data_strategy,
                &compiled.scheduler,
            ) {
                state.metadata["workflow_variant"] =
                    serde_json::to_value(&selection).unwrap_or_else(|_| serde_json::json!({}));
                state.metadata["workflow_variant_id"] = serde_json::json!(selection.variant_id.clone());
                state.metadata["workflow_execution_profile"] = serde_json::json!({
                    "execution": selection.execution,
                    "execution_constraints": selection.execution_constraints,
                    "data_strategy": selection.data_strategy,
                    "scheduler": selection.scheduler,
                });
            } else if matches!(policy.fallback, crate::agent::workflow_compiler::VariantFallbackMode::Recompile) {
                state.metadata["needs_recompile"] = serde_json::json!(true);
                state.metadata["recompile_reason"] =
                    serde_json::json!("no workflow variant matched the current data signature");
                return None;
            }
        }

        Some(Plan::from_compiled_workflow(compiled, &role))
    }

    fn build_coordinator_shell_plan(&self, state: &mut AgentState, role: &crate::agent::definition::AgentRole) -> Plan {
        let worker_messages =
            state.metadata.get("worker_messages").and_then(|value| value.as_array()).cloned().unwrap_or_default();
        let pending_children = state.pending_children.clone();
        let research_hints = role.execution_guidelines.workflow_hints();

        let research_sub_goals = if research_hints.is_empty() {
            vec![
                format!("Research the systems, dependencies, and open questions needed to accomplish: {}", state.goal),
                format!("Identify risks, unknowns, and validation ideas for: {}", state.goal),
            ]
        } else {
            research_hints.iter().take(2).map(|hint| format!("Research and validate: {}", hint)).collect::<Vec<_>>()
        };

        let implementation_sub_goals = vec![
            format!("Implement the synthesized spec for: {}", state.goal),
            format!("Keep the implementation scoped to the verified workspace boundaries for: {}", state.goal),
        ];

        let verification_sub_goals = vec![
            format!("Verify the implementation independently for: {}", state.goal),
            "Report only concrete failures, regressions, or missing evidence.".into(),
        ];

        let research_step = if let Some(child_id) = pending_children.first().cloned() {
            crate::agent::planner::PlannedStep {
                foreach: None,
                index: 1,
                description: "Continue the most relevant existing research worker with fresh context.".into(),
                tool: Some("delegate".into()),
                tool_args: Some(serde_json::json!({
                    "continue_child_id": child_id,
                    "worker_type": "research",
                    "sub_goals": [],
                })),
                success_criteria: "research worker resumed or refreshed".into(),
                condition: None,
                depends_on: vec![0],
            }
        } else {
            crate::agent::planner::PlannedStep {
                foreach: None,
                index: 1,
                description: "Spawn parallel research workers for independent discovery.".into(),
                tool: Some("delegate".into()),
                tool_args: Some(serde_json::json!({
                    "sub_goals": research_sub_goals,
                    "worker_type": "research",
                    "write_scope": ["research", "analysis"],
                })),
                success_criteria: "research workers spawned and scheduled".into(),
                condition: None,
                depends_on: vec![0],
            }
        };

        state.metadata["coordinator_shell"] = serde_json::json!({
            "mode": "coordinator_shell",
            "tool_pool": "coordinator",
            "goal": state.goal.clone(),
            "research_hints": research_hints,
            "worker_messages_seen": worker_messages.len(),
            "pending_children": pending_children,
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });

        Plan {
            goal: state.goal.clone(),
            job_type: Some("coordinator".into()),
            rationale: "coordinator-shell plan: task-first research, synthesis, implementation, and verification".into(),
            steps: vec![
                crate::agent::planner::PlannedStep {
                    foreach: None,
                    index: 0,
                    description: "Assemble the coordination brief from durable tasks, cached prompt context, and the current goal.".into(),
                    tool: Some("task_list".into()),
                    tool_args: Some(serde_json::json!({
                        "status": "pending",
                    })),
                    success_criteria: "coordination brief assembled".into(),
                    condition: None,
                    depends_on: vec![],
                },
                research_step,
                crate::agent::planner::PlannedStep {
                    foreach: None,
                    index: 2,
                    description: "Receive structured worker notifications and inspect the inbox before synthesizing.".into(),
                    tool: Some("message_inbox".into()),
                    tool_args: Some(serde_json::json!({
                        "action": "list",
                        "direction": "inbox",
                        "undelivered_only": true,
                        "limit": 25,
                    })),
                    success_criteria: "worker notifications reviewed".into(),
                    condition: None,
                    depends_on: vec![1],
                },
                crate::agent::planner::PlannedStep {
                    foreach: None,
                    index: 3,
                    description: "Synthesize the findings into a self-contained implementation and verification spec; continue or respawn workers when the inbox shows stale context overlap.".into(),
                    tool: Some(crate::agent::workflow_compiler::LLM_WORKER_TOOL_NAME.into()),
                    tool_args: Some(serde_json::json!({
                        "instruction": "Synthesize the findings into a self-contained implementation and verification spec; continue or respawn workers when the inbox shows stale context overlap.",
                        "response_format": "text",
                    })),
                    success_criteria: "self-contained spec produced".into(),
                    condition: None,
                    depends_on: vec![2],
                },
                crate::agent::planner::PlannedStep {
                    foreach: None,
                    index: 4,
                    description: "Run implementation through a scoped worker using the synthesized spec.".into(),
                    tool: Some("delegate".into()),
                    tool_args: Some(serde_json::json!({
                        "sub_goals": implementation_sub_goals,
                        "worker_type": "implementation",
                        "write_scope": ["workspace"],
                    })),
                    success_criteria: "implementation worker spawned and scheduled".into(),
                    condition: None,
                    depends_on: vec![3],
                },
                crate::agent::planner::PlannedStep {
                    foreach: None,
                    index: 5,
                    description: "Run an independent verification worker from fresh context.".into(),
                    tool: Some("delegate".into()),
                    tool_args: Some(serde_json::json!({
                        "sub_goals": verification_sub_goals,
                        "worker_type": "verification",
                        "write_scope": ["verification"],
                    })),
                    success_criteria: "verification worker spawned and scheduled".into(),
                    condition: None,
                    depends_on: vec![4],
                },
                crate::agent::planner::PlannedStep {
                    foreach: None,
                    index: 6,
                    description: "Finalize from verified state only and record the durable result contract.".into(),
                    tool: Some("task_output".into()),
                    tool_args: Some(serde_json::json!({
                        "status": "complete",
                        "note": "coordinator shell completed after verification",
                    })),
                    success_criteria: "verified final result recorded".into(),
                    condition: None,
                    depends_on: vec![5],
                },
            ],
        }
    }

    async fn sync_session_tasks_for_plan(&self, state: &mut AgentState, plan: &Plan) {
        let Some(store) = self.store.as_ref() else {
            return;
        };

        for step in &plan.steps {
            let task_id = self.step_task_id(state, step.index);
            let mut task =
                store.get_session_task(&state.tenant_id, &task_id).await.ok().flatten().unwrap_or_else(|| {
                    SessionTask::new(
                        task_id.clone(),
                        state.tenant_id.clone(),
                        state.id.clone(),
                        format!("Step {}: {}", step.index + 1, step.description),
                        step.success_criteria.clone(),
                    )
                });

            task.subject = format!("Step {}: {}", step.index + 1, step.description);
            task.description = step.success_criteria.clone();
            task.metadata["step_index"] = serde_json::json!(step.index);
            task.metadata["planner_hint"] = serde_json::json!(step.tool);
            task.metadata["execution_contract"] = serde_json::json!("workflow_step");
            if step.index < state.current_step as usize {
                task.set_status(SessionTaskStatus::Completed);
            } else if step.index == state.current_step as usize {
                task.set_status(SessionTaskStatus::Pending);
            } else if !matches!(
                task.status,
                SessionTaskStatus::Completed | SessionTaskStatus::Failed | SessionTaskStatus::Stopped
            ) {
                task.set_status(SessionTaskStatus::Pending);
            }
            let _ = store.upsert_session_task(&task).await;
        }
    }

    fn step_task_id(&self, state: &AgentState, step_index: usize) -> String {
        format!("{}:workflow_step:{}", state.id, step_index)
    }

    async fn mark_step_task_in_progress(&self, state: &mut AgentState, step: &crate::agent::planner::PlannedStep) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let task_id = self.step_task_id(state, step.index);
        let mut task = store.get_session_task(&state.tenant_id, &task_id).await.ok().flatten().unwrap_or_else(|| {
            SessionTask::new(
                task_id.clone(),
                state.tenant_id.clone(),
                state.id.clone(),
                format!("Step {}: {}", step.index + 1, step.description),
                step.success_criteria.clone(),
            )
        });
        task.set_status(SessionTaskStatus::InProgress);
        task.metadata["step_index"] = serde_json::json!(step.index);
        let _ = store.upsert_session_task(&task).await;
        state.current_task = Some(task_id);
    }

    async fn mark_step_task_finished(
        &self,
        state: &mut AgentState,
        step: &crate::agent::planner::PlannedStep,
        status: SessionTaskStatus,
        output: Option<SessionTaskOutput>,
    ) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let task_id = self.step_task_id(state, step.index);
        let Some(mut task) = store.get_session_task(&state.tenant_id, &task_id).await.ok().flatten() else {
            return;
        };
        task.set_status(status);
        if let Some(output) = output {
            task.set_output(output);
        }
        let _ = store.upsert_session_task(&task).await;
        if state.current_task.as_deref() == Some(task_id.as_str()) {
            state.current_task = None;
        }
    }

    fn prompt_for_provider_credentials(&self, state: &mut AgentState) -> Result<StepOutcome> {
        let questions = provider_credentials_questions();
        state.metadata["clarification_questions"] = serde_json::to_value(&questions)?;
        state.mark_clarifying();
        self.event_bus
            .publish(AgentEvent::ClarificationNeeded { agent_id: state.id.clone(), questions: questions.clone() });
        Ok(StepOutcome::NeedsClarification { questions })
    }

    fn inject_delegation_ctx(
        &self,
        step: &crate::agent::planner::PlannedStep,
        state: &AgentState,
    ) -> crate::agent::planner::PlannedStep {
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

    fn evaluate_step_condition(
        &self,
        state: &AgentState,
        step: &crate::agent::planner::PlannedStep,
    ) -> Result<Option<String>> {
        let Some(condition) = step.condition.as_ref() else {
            return Ok(None);
        };

        let (should_run, condition_desc) =
            match condition {
                crate::agent::planner::StepCondition::Deterministic(cond) => {
                    let resolved = crate::agent::executor::resolve_reference_from_state(&cond.left, state)
                        .map_err(anyhow::Error::msg);

                    let operator = match &cond.operator {
                        crate::agent::planner::ConditionOp::Exists => "exists",
                        crate::agent::planner::ConditionOp::NotExists => "not_exists",
                        crate::agent::planner::ConditionOp::IsTruthy => "truthy",
                        crate::agent::planner::ConditionOp::IsFalsy => "falsy",
                        crate::agent::planner::ConditionOp::NotEmpty => "nonempty",
                        crate::agent::planner::ConditionOp::Empty => "empty",
                        crate::agent::planner::ConditionOp::Equals => "equals",
                        crate::agent::planner::ConditionOp::NotEquals => "not_equals",
                        crate::agent::planner::ConditionOp::Contains => "contains",
                        crate::agent::planner::ConditionOp::GreaterThan => "gt",
                        crate::agent::planner::ConditionOp::GreaterThanEquals => "gte",
                        crate::agent::planner::ConditionOp::LessThan => "lt",
                        crate::agent::planner::ConditionOp::LessThanEquals => "lte",
                    };

                    let should_run =
                        match operator {
                            "exists" => resolved.is_ok(),
                            "not_exists" => resolved.is_err(),
                            "truthy" => resolved.map(|value| condition_truthy(&value)).unwrap_or(false),
                            "falsy" => resolved.map(|value| !condition_truthy(&value)).unwrap_or(true),
                            "nonempty" => resolved.map(|value| condition_truthy(&value)).unwrap_or(false),
                            "empty" => resolved.map(|value| !condition_truthy(&value)).unwrap_or(true),
                            "equals" => {
                                let actual = resolved?;
                                let expected = cond.right.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("condition.operator 'equals' requires condition.right")
                                })?;
                                actual == *expected
                            }
                            "not_equals" => {
                                let actual = resolved?;
                                let expected = cond.right.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("condition.operator 'not_equals' requires condition.right")
                                })?;
                                actual != *expected
                            }
                            "contains" => {
                                let actual = resolved?;
                                let expected = cond.right.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("condition.operator 'contains' requires condition.right")
                                })?;
                                condition_contains(&actual, expected)
                            }
                            "gt" => {
                                let actual = resolved?;
                                let expected = cond.right.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("condition.operator 'gt' requires condition.right")
                                })?;
                                condition_compare_numbers(&actual, expected, |left, right| left > right)?
                            }
                            "gte" => {
                                let actual = resolved?;
                                let expected = cond.right.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("condition.operator 'gte' requires condition.right")
                                })?;
                                condition_compare_numbers(&actual, expected, |left, right| left >= right)?
                            }
                            "lt" => {
                                let actual = resolved?;
                                let expected = cond.right.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("condition.operator 'lt' requires condition.right")
                                })?;
                                condition_compare_numbers(&actual, expected, |left, right| left < right)?
                            }
                            "lte" => {
                                let actual = resolved?;
                                let expected = cond.right.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("condition.operator 'lte' requires condition.right")
                                })?;
                                condition_compare_numbers(&actual, expected, |left, right| left <= right)?
                            }
                            other => return Err(anyhow::anyhow!("unsupported step condition operator '{other}'")),
                        };
                    (should_run, format!("{} {:?}", cond.left, cond.operator))
                }
                crate::agent::planner::StepCondition::Expression(expr) => (
                    Self::evaluate_typed_step_condition(expr, state)?,
                    serde_json::to_string(expr).unwrap_or_else(|_| "{}".into()),
                ),
            };

        if should_run {
            Ok(None)
        } else {
            Ok(Some(format!("Skipped step {} because condition {} was not satisfied.", step.index, condition_desc)))
        }
    }

    fn evaluate_typed_step_condition(expr: &TypedExpression, state: &AgentState) -> Result<bool> {
        let result = evaluate_typed_expression(expr, state)?;
        match result {
            serde_json::Value::Bool(value) => Ok(value),
            other => Err(anyhow::anyhow!("typed expression for step condition must evaluate to boolean, got {other}")),
        }
    }
}

fn evaluate_typed_expression(expr: &TypedExpression, state: &AgentState) -> Result<serde_json::Value> {
    if let Some(value) = &expr.value {
        return Ok(value.clone());
    }

    if let Some(path) = &expr.path {
        return crate::agent::executor::resolve_reference_from_state(path, state).map_err(anyhow::Error::msg);
    }

    if let Some(function) = &expr.function {
        let args = expr.args.iter().map(|arg| evaluate_typed_expression(arg, state)).collect::<Result<Vec<_>>>()?;

        return match function.as_str() {
            "len" | "count" => {
                let first =
                    args.first().ok_or_else(|| anyhow::anyhow!("function '{function}' requires one argument"))?;
                let count = match first {
                    serde_json::Value::Array(values) => values.len(),
                    serde_json::Value::Object(map) => map.len(),
                    serde_json::Value::String(text) => text.chars().count(),
                    other => {
                        return Err(anyhow::anyhow!(
                            "function '{function}' expects array, object, or string input, got {other}"
                        ));
                    }
                };
                Ok(serde_json::json!(count))
            }
            other => Err(anyhow::anyhow!("unsupported typed expression function '{other}'")),
        };
    }

    if let Some(op) = expr.op.as_deref() {
        let left = expr
            .left
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("typed expression operator '{op}' requires left operand"))?;
        let left = evaluate_typed_expression(left, state)?;
        let right = match expr.right.as_deref() {
            Some(right) => Some(evaluate_typed_expression(right, state)?),
            None => None,
        };

        let result = match op {
            "gt" => numeric_value(&left, right.as_ref(), |l, r| l > r)?,
            "gte" => numeric_value(&left, right.as_ref(), |l, r| l >= r)?,
            "lt" => numeric_value(&left, right.as_ref(), |l, r| l < r)?,
            "lte" => numeric_value(&left, right.as_ref(), |l, r| l <= r)?,
            "eq" => compare_values(&left, right.as_ref(), |l, r| l == r)?,
            "neq" => compare_values(&left, right.as_ref(), |l, r| l != r)?,
            "and" => bool_value(&left)
                .zip(right.as_ref().and_then(bool_value))
                .map(|(l, r)| l && r)
                .ok_or_else(|| anyhow::anyhow!("typed expression operator 'and' requires boolean operands"))?,
            "or" => bool_value(&left)
                .zip(right.as_ref().and_then(bool_value))
                .map(|(l, r)| l || r)
                .ok_or_else(|| anyhow::anyhow!("typed expression operator 'or' requires boolean operands"))?,
            "not" => !bool_value(&left)
                .ok_or_else(|| anyhow::anyhow!("typed expression operator 'not' requires boolean operand"))?,
            other => return Err(anyhow::anyhow!("unsupported typed expression operator '{other}'")),
        };

        return Ok(serde_json::Value::Bool(result));
    }

    Err(anyhow::anyhow!("typed expression is missing value, path, function, or operator"))
}

fn bool_value(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(boolean) => Some(*boolean),
        serde_json::Value::Number(number) => number.as_f64().map(|number| number != 0.0),
        serde_json::Value::String(text) => Some(!text.trim().is_empty()),
        serde_json::Value::Array(items) => Some(!items.is_empty()),
        serde_json::Value::Object(map) => Some(!map.is_empty()),
        serde_json::Value::Null => Some(false),
    }
}

fn numeric_value(
    left: &serde_json::Value,
    right: Option<&serde_json::Value>,
    cmp: impl Fn(f64, f64) -> bool,
) -> Result<bool> {
    let left = left.as_f64().ok_or_else(|| anyhow::anyhow!("typed comparison requires numeric left operand"))?;
    let right = right
        .ok_or_else(|| anyhow::anyhow!("typed comparison requires numeric right operand"))?
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("typed comparison requires numeric right operand"))?;
    Ok(cmp(left, right))
}

fn compare_values(
    left: &serde_json::Value,
    right: Option<&serde_json::Value>,
    cmp: impl Fn(&serde_json::Value, &serde_json::Value) -> bool,
) -> Result<bool> {
    let right = right.ok_or_else(|| anyhow::anyhow!("typed comparison requires right operand"))?;
    Ok(cmp(left, right))
}

// ── Knowledge graph entity extraction ────────────────────────────────────────
// Lightweight heuristic: look for "Key: Value" or "Entity: fact" patterns.
fn extract_entities(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() == 2 {
                let k = parts[0].trim();
                let v = parts[1].trim();
                if k.len() > 1 && k.len() < 60 && v.len() > 1 {
                    return Some((k.to_string(), crate::util::truncate(v, 120).to_string()));
                }
            }
            None
        })
        .take(5)
        .collect()
}

// ── Optimization helpers ────────────────────────────────────────────────────

/// Get facts from recent successful steps only (not all related facts).
/// This reduces noise and ensures agent sees facts from its own run, not historical.
/// Returns facts from current step and N previous successful steps (default N=2).
async fn get_recent_step_facts(
    knowledge_graph: &tokio::sync::Mutex<KnowledgeGraph>,
    agent_id: &str,
    current_step: usize,
    lookback_steps: usize,
) -> Vec<(String, String)> {
    if let Ok(graph) = knowledge_graph.try_lock() {
        // For now, use the same pattern but add logging about optimization
        let facts = graph.get_related(&agent_id.to_string());
        tracing::debug!(
            agent_id = %agent_id,
            current_step = current_step,
            lookback_steps = lookback_steps,
            fact_count = facts.len(),
            "optimized knowledge graph injection: recent facts only"
        );
        facts.iter().map(|n| (n.id.clone(), n.value.clone())).collect()
    } else {
        tracing::debug!(agent_id = %agent_id, "knowledge graph lock contended—skipping fact injection");
        vec![]
    }
}

/// Check if we should trigger early completion based on CompletionCriteria mid-run.
/// Returns true if goal is effectively complete mid-plan (before all steps executed).
fn check_early_completion(state: &AgentState, role: &crate::agent::definition::AgentRole) -> bool {
    // Check deterministic CompletionCriteria that could fire mid-run
    for criterion in &role.execution_guidelines.completion_criteria {
        use crate::agent::definition::CompletionCheck;
        match &criterion.check {
            CompletionCheck::RecordUpdated { .. } => {
                tracing::debug!("early completion: record updated criterion met");
                return true;
            }
            CompletionCheck::AllItemsProcessed { .. } => {
                // Check if all items were already processed (in metadata)
                if let Some(step_outputs) = state.metadata.get("step_outputs").and_then(|v| v.as_array()) {
                    let total_processed: u64 =
                        step_outputs.iter().filter_map(|o| o.get("processed").and_then(|v| v.as_u64())).sum();

                    if total_processed > 0 {
                        tracing::debug!(total_processed = total_processed, "early completion: all items criterion");
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

// ── FailureAction override ────────────────────────────────────────────────

/// Check if any FailureRule should abort deterministically (without evaluator LLM call).
/// Returns Some(EvalVerdict::Abort) if a matching rule forbids retry, None otherwise.
/// This fast-paths failures that evaluator LLM can't resolve (e.g., permission denials).
fn check_failure_rules_for_deterministic_abort(
    result: &crate::agent::executor::StepResult,
    role: &crate::agent::definition::AgentRole,
) -> Option<EvalVerdict> {
    use crate::agent::definition::FailureAction;

    let error_text: String = result
        .tool_results
        .iter()
        .filter(|r| !r.success)
        .filter_map(|r| r.error.as_deref())
        .collect::<Vec<_>>()
        .join(" | ")
        .to_lowercase();

    for rule in &role.execution_guidelines.failure_handling {
        // Match by tool_scope if specified
        let scope_matches = match &rule.tool_scope {
            Some(scope) => result.tools_called.iter().any(|t| t.contains(scope.as_str())),
            None => true,
        };
        if !scope_matches {
            continue;
        }

        // Check if rule text matches the error
        let rule_lower = rule.text.to_lowercase();
        let text_matches = error_text.is_empty()
            || error_text.contains(&rule_lower)
            || rule_lower.contains("any")
            || rule_lower.contains("all");

        if !text_matches {
            continue;
        }

        // Abort action is deterministic — no LLM needed
        match &rule.action {
            FailureAction::Abort => {
                tracing::warn!(action_type = "Abort", rule = %rule.text, "deterministic abort rule matched");
                return Some(EvalVerdict::Abort);
            }
            _ => {} // RetryOnce and EscalateToHuman need evaluator context
        }
    }

    None
}

/// Classify an error message into a specific StepOutcome variant for smarter retry logic.
fn classify_error(reason: &str) -> StepOutcome {
    let lower = reason.to_lowercase();

    // Policy/permission violations
    if lower.contains("policy")
        || lower.contains("permission denied")
        || lower.contains("access denied")
        || lower.contains("plane guard")
        || lower.contains("forbidden")
    {
        return StepOutcome::PolicyViolation { reason: reason.to_string() };
    }

    // Rate limiting
    if lower.contains("rate limit") || lower.contains("too many requests") || lower.contains("429") {
        // Try to extract retry-after header hint or default to 60s
        let retry_secs = 60u64;
        return StepOutcome::RateLimited { retry_after_secs: retry_secs, reason: reason.to_string() };
    }

    // Permanent configuration errors
    if lower.contains("credential")
        || lower.contains("not found")
        || lower.contains("invalid schema")
        || lower.contains("authentication failed")
        || lower.contains("oauth")
        || lower.contains("api key")
    {
        return StepOutcome::PermanentError { reason: reason.to_string() };
    }

    // Transient errors (timeouts, connection issues, temporary unavailability)
    if lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
        || lower.contains("connection reset")
        || lower.contains("503")
        || lower.contains("504")
    {
        return StepOutcome::TransientError { reason: reason.to_string(), retry_after_secs: 30 };
    }

    // Default to generic Failed for anything else
    StepOutcome::Failed(reason.to_string())
}

/// Check the role's failure_handling rules against the current step failure.
/// If a matching rule overrides the evaluator verdict, return the override.
/// This implements FailureAction::RetryOnce and EscalateToHuman.
fn apply_failure_action_override(
    original: EvalVerdict,
    result: &crate::agent::executor::StepResult,
    role: &crate::agent::definition::AgentRole,
    state: &mut crate::state::AgentState,
    services: &crate::segments::AgentServices,
) -> EvalVerdict {
    use crate::agent::definition::FailureAction;

    // Collect error text from failed tool results
    let error_text: String = result
        .tool_results
        .iter()
        .filter(|r| !r.success)
        .filter_map(|r| r.error.as_deref())
        .collect::<Vec<_>>()
        .join(" | ")
        .to_lowercase();

    for rule in &role.execution_guidelines.failure_handling {
        // Match by tool_scope if specified, otherwise applies to any failure
        let scope_matches = match &rule.tool_scope {
            Some(scope) => result.tools_called.iter().any(|t| t.contains(scope.as_str())),
            None => true,
        };
        if !scope_matches {
            continue;
        }

        // Check if the rule's text matches the current failure context
        let rule_lower = rule.text.to_lowercase();
        let text_matches = error_text.is_empty()           // any failure
            || error_text.contains(&rule_lower)
            || rule_lower.contains("any")
            || rule_lower.contains("all");

        if !text_matches {
            continue;
        }

        match &rule.action {
            FailureAction::RetryOnce => {
                // Only override to Retry if we haven't already retried this step
                let retry_count = state.metadata.get("retry_count").and_then(|v| v.as_u64()).unwrap_or(0);
                if retry_count == 0 {
                    tracing::info!(
                        agent_id = %state.id,
                        rule     = %rule.text,
                        "FailureAction::RetryOnce — forcing Retry"
                    );
                    return EvalVerdict::Retry;
                }
                // Already retried once — fall through to original verdict
            }

            FailureAction::EscalateToHuman { notify_channel } => {
                // Submit a human review request and abort the run
                if let Some(ref rq) = services.reviews {
                    let reason = format!("FailureAction escalation: {} | error: {}", rule.text, error_text);
                    let channel = notify_channel.as_deref().unwrap_or("unspecified");
                    tracing::warn!(
                        agent_id = %state.id,
                        channel  = %channel,
                        reason   = %reason,
                        "FailureAction::EscalateToHuman — submitting review"
                    );
                    // Fire-and-forget — escalation failure is non-fatal
                    let rq_clone = rq.clone();
                    let tenant = state.tenant_id.clone();
                    let aid = state.id.clone();
                    let step_idx = result.step_index;
                    let r = reason.clone();
                    tokio::spawn(async move {
                        let _ = rq_clone.submit(&tenant, &aid, step_idx, &r, "failure_rule").await;
                    });
                }
                return EvalVerdict::Abort;
            }

            FailureAction::Abort => {
                return EvalVerdict::Abort;
            }

            FailureAction::SkipSilently => {
                // Advance silently — no log written
                return EvalVerdict::Continue;
            }

            FailureAction::SkipAndLog { log_path } => {
                // Write the skip record to the log file so:
                //   1. CompletionCriteria::ErrorsLogged check passes
                //   2. Users can inspect what was skipped after the run
                let ws = state.workspace_path.trim_end_matches('/');
                let abs_path =
                    if log_path.starts_with('/') { log_path.clone() } else { format!("{}/{}", ws, log_path) };

                let error_text = result
                    .tool_results
                    .iter()
                    .filter(|r| !r.success)
                    .filter_map(|r| r.error.as_deref())
                    .collect::<Vec<_>>()
                    .join(" | ");
                let skip_reason = if !error_text.is_empty() { error_text } else { rule.text.clone() };

                let entry = format!(
                    "[{}] step={} tool={} reason={}\n",
                    chrono::Utc::now().to_rfc3339(),
                    result.step_index,
                    result.tools_called.first().map(String::as_str).unwrap_or("unknown"),
                    skip_reason,
                );

                // Ensure directory exists and append atomically
                if let Some(parent) = std::path::Path::new(&abs_path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&abs_path) {
                    let _ = f.write_all(entry.as_bytes());
                }

                // Set metadata flag so CompletionCriteria::ErrorsLogged check passes
                // even if no other step explicitly sets it
                state.metadata["errors_logged"] = serde_json::json!(true);

                tracing::info!(
                    agent_id   = %state.id,
                    step       = result.step_index,
                    log_path   = %abs_path,
                    reason     = %rule.text,
                    "SkipAndLog — skip recorded"
                );

                return EvalVerdict::Continue;
            }
        }
    }

    original
}

// ── Tests ────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::postgres::PgPoolOptions;

    use super::*;
    use crate::{
        agent::{
            evaluator::EvalVerdict,
            executor::StepResult,
            planner::{Plan, PlannedStep},
            reflector::Reflection,
            test_helpers::{MockClarifier, MockEvaluator, MockExecutor, MockPreflight, MockReflector},
        },
        memory::{DistanceMetric, PgVectorStore},
        state::AgentState,
        tools::ToolResult,
    };

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "fix CI pipeline".into(), "/tmp/ws".into())
    }

    fn make_plan() -> Plan {
        Plan {
            goal: "fix CI pipeline".into(),
            job_type: Some("software_engineer".into()),
            steps: vec![PlannedStep {
                foreach: None,
                index: 0,
                description: "Inspect failing workflow".into(),
                tool: Some("file_read".into()),
                tool_args: Some(serde_json::json!({"path": ".github/workflows/ci.yml"})),
                success_criteria: "workflow reviewed".into(),
                condition: None,
                depends_on: vec![],
            }],
            rationale: "inspect before changing".into(),
        }
    }

    fn make_loop(
        executor: Arc<dyn Executor>,
        evaluator: Arc<dyn Evaluator>,
        reflector: Arc<dyn Reflector>,
        preflight: Arc<dyn Preflight>,
        clarifier: Arc<dyn Clarifier>,
    ) -> AgentLoop {
        make_loop_with_registry(executor, evaluator, reflector, preflight, clarifier, SkillRegistry::new())
    }

    fn make_loop_with_registry(
        executor: Arc<dyn Executor>,
        evaluator: Arc<dyn Evaluator>,
        reflector: Arc<dyn Reflector>,
        preflight: Arc<dyn Preflight>,
        clarifier: Arc<dyn Clarifier>,
        skill_registry: SkillRegistry,
    ) -> AgentLoop {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://narayan:narayan@localhost/narayan")
            .expect("lazy pool should be created");
        let vector_store = PgVectorStore::new(pool, 4, DistanceMetric::Cosine);
        let embedder: Arc<dyn crate::memory::EmbeddingModel> =
            Arc::new(crate::memory::embeddings::StubEmbeddingModel::new(4));

        AgentLoop::new(
            executor,
            evaluator,
            reflector,
            preflight,
            clarifier,
            Arc::new(crate::tools::ToolRegistry::new()),
            Arc::new(EventBus::new()),
            Arc::new(RwLock::new(skill_registry)),
            Arc::new(Mutex::new(KnowledgeGraph::new())),
            vector_store,
            embedder,
            Arc::new(crate::segments::AgentServices::none()),
        )
    }

    // ── extract_entities ─────────────────────────────────────────────────

    #[test]
    fn test_extract_entities_basic() {
        let input = "Name: Alice\nRole: Engineer";
        let entities = extract_entities(input);
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0], ("Name".to_string(), "Alice".to_string()));
        assert_eq!(entities[1], ("Role".to_string(), "Engineer".to_string()));
    }

    #[test]
    fn test_extract_entities_ignores_short_keys() {
        // Keys with len <= 1 should be filtered out
        let input = "A: value\nB: other";
        let entities = extract_entities(input);
        assert_eq!(entities.len(), 0, "single-char keys should be ignored");
    }

    #[test]
    fn test_extract_entities_max_five() {
        let input = (0..10).map(|i| format!("key_{i}: value_{i}")).collect::<Vec<_>>().join("\n");
        let entities = extract_entities(&input);
        assert_eq!(entities.len(), 5, "should return at most 5 entities");
    }

    #[test]
    fn test_extract_entities_empty() {
        let entities = extract_entities("");
        assert!(entities.is_empty(), "empty input should yield no entities");
    }

    #[test]
    fn test_extract_entities_no_colon() {
        let input = "no colon here\njust plain text";
        let entities = extract_entities(input);
        assert!(entities.is_empty(), "lines without colons should yield no entities");
    }

    // ── StepOutcome ──────────────────────────────────────────────────────

    #[test]
    fn test_step_outcome_debug() {
        let outcome = StepOutcome::Complete;
        let debug_str = format!("{:?}", outcome);
        assert_eq!(debug_str, "Complete");

        let outcome = StepOutcome::Failed("timeout".to_string());
        let debug_str = format!("{:?}", outcome);
        assert!(debug_str.contains("timeout"), "Debug should contain the failure reason");

        let outcome = StepOutcome::Continue { delay_secs: 5 };
        let debug_str = format!("{:?}", outcome);
        assert!(debug_str.contains("5"), "Debug should contain the delay value");

        let outcome = StepOutcome::NeedsClarification {
            questions: vec![crate::agent::clarifier::ClarificationQuestion::new("What scope?")],
        };
        let debug_str = format!("{:?}", outcome);
        assert!(debug_str.contains("What scope?"), "Debug should contain the question");

        let outcome = StepOutcome::Delegating { child_ids: vec!["child-1".to_string()] };
        let debug_str = format!("{:?}", outcome);
        assert!(debug_str.contains("child-1"), "Debug should contain the child id");
    }

    #[tokio::test]
    async fn test_run_step_pending_feasible_and_clear_transitions_to_waiting() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::from_responses(vec![PreflightResult::Feasible])),
            Arc::new(MockClarifier::from_check_responses(vec![ClarificationResult::Clear])),
        );
        let mut state = make_state();
        let mut plan = None;
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("preflight path should succeed");

        match outcome {
            StepOutcome::Continue { delay_secs } => assert_eq!(delay_secs, 0),
            other => panic!("expected continue outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Waiting);
    }

    #[tokio::test]
    async fn test_run_step_pending_with_clarification_needed_persists_questions() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::from_responses(vec![PreflightResult::Feasible])),
            Arc::new(MockClarifier::from_check_responses(vec![ClarificationResult::NeedsInput {
                questions: vec![crate::agent::clarifier::ClarificationQuestion::new(
                    "Which repository should be fixed?",
                )],
            }])),
        );
        let mut state = make_state();
        let mut plan = None;
        let mut history = StepHistory::new();

        let outcome = loop_runtime
            .run_step(&mut state, &mut plan, &mut history)
            .await
            .expect("clarification path should succeed");

        match outcome {
            StepOutcome::NeedsClarification { questions } => {
                assert_eq!(
                    questions,
                    vec![crate::agent::clarifier::ClarificationQuestion::new("Which repository should be fixed?")]
                )
            }
            other => panic!("expected clarification outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Clarifying);
        assert_eq!(
            state.metadata["clarification_questions"][0]["prompt"],
            serde_json::json!("Which repository should be fixed?")
        );
    }

    #[tokio::test]
    async fn test_run_step_detects_delegation_from_tool_output() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
                skipped: false,
                output: "delegated".into(),
                final_answer_candidate: Some("delegated".into()),
                tool_results: vec![ToolResult::ok(serde_json::json!({
                    "child_agent_ids": ["child-1", "child-2"]
                }))],
                tools_called: vec!["delegate".into()],
                items_processed: 0,
                connector_writes: vec![],
            }])),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        state.mark_waiting(chrono::Utc::now());
        let mut plan = Some(make_plan());
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("delegation path should succeed");

        match outcome {
            StepOutcome::Delegating { child_ids } => {
                assert_eq!(child_ids, vec!["child-1".to_string(), "child-2".to_string()])
            }
            other => panic!("expected delegating outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Delegating);
        assert_eq!(state.pending_children, vec!["child-1".to_string(), "child-2".to_string()]);
        assert_eq!(state.current_step, 1);
    }

    #[tokio::test]
    async fn test_run_step_continue_advances_history_and_waiting_state() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
                skipped: false,
                output: "STEP COMPLETE".into(),
                final_answer_candidate: Some("STEP COMPLETE".into()),
                tool_results: vec![],
                tools_called: vec![],
                items_processed: 0,
                connector_writes: vec![],
            }])),
            Arc::new(MockEvaluator::from_responses(vec![EvalVerdict::Continue])),
            Arc::new(MockReflector::from_responses(vec![Reflection {
                summary: String::new(),
                key_findings: vec![],
                revised_plan: None,
            }])),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        state.mark_waiting(chrono::Utc::now());
        let mut plan = Some(make_plan());
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("continue path should succeed");

        match outcome {
            StepOutcome::Continue { delay_secs } => assert_eq!(delay_secs, 0),
            other => panic!("expected continue outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Waiting);
        assert_eq!(state.current_step, 1);
        assert_eq!(history.len(), 1);
    }

    #[tokio::test]
    async fn test_run_step_fails_when_cognitive_control_limit_is_hit() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        )
        .with_limits(1, 300);
        let mut state = make_state();
        state.current_step = 1;
        state.mark_waiting(chrono::Utc::now());
        let mut plan = Some(make_plan());
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("limit check should succeed");

        match outcome {
            StepOutcome::Failed(reason) => assert!(reason.contains("exceeded safety limits")),
            other => panic!("expected failed outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Failed);
    }

    #[tokio::test]
    async fn test_run_step_pending_infeasible_marks_agent_failed() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::from_responses(vec![PreflightResult::Infeasible {
                reason: "missing browser".into(),
                missing_tools: vec!["browser".into()],
            }])),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        let mut plan = None;
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("preflight should succeed");

        match outcome {
            StepOutcome::Infeasible { reason } => {
                assert!(reason.contains("missing browser"));
                assert!(reason.contains("browser"));
            }
            other => panic!("expected infeasible outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Failed);
    }

    #[tokio::test]
    async fn test_run_step_returns_existing_clarification_questions_without_progressing() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        state.mark_clarifying();
        state.metadata["clarification_questions"] = serde_json::json!(["Which repo?"]);
        let mut plan = None;
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("clarifying state should succeed");

        match outcome {
            StepOutcome::NeedsClarification { questions } => {
                assert_eq!(questions, vec![crate::agent::clarifier::ClarificationQuestion::new("Which repo?")])
            }
            other => panic!("expected clarification outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Clarifying);
        assert!(plan.is_none());
    }

    #[tokio::test]
    async fn test_run_step_uses_matching_skill_registry_plan_when_no_plan_exists() {
        let mut reg = SkillRegistry::new();
        reg.register(crate::skills::registry::Skill::new(
            "ci",
            "fix CI pipeline",
            vec!["Inspect failing workflow".into(), "Patch workflow".into()],
        ));
        let loop_runtime = make_loop_with_registry(
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
                skipped: false,
                output: "STEP COMPLETE".into(),
                final_answer_candidate: Some("STEP COMPLETE".into()),
                tool_results: vec![],
                tools_called: vec![],
                items_processed: 0,
                connector_writes: vec![],
            }])),
            Arc::new(MockEvaluator::from_responses(vec![EvalVerdict::Continue])),
            Arc::new(MockReflector::from_responses(vec![Reflection {
                summary: String::new(),
                key_findings: vec![],
                revised_plan: None,
            }])),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
            reg,
        );
        let mut state = make_state();
        state.goal = "fix ci today".into();
        state.mark_waiting(chrono::Utc::now());
        let mut plan = None;
        let mut history = StepHistory::new();

        // First call creates the plan and immediately executes it.
        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("skill plan path should succeed");

        match outcome {
            StepOutcome::Continue { delay_secs } => assert_eq!(delay_secs, 0),
            other => panic!("expected immediate execution outcome, got {other:?}"),
        }
        let p = plan.as_ref().expect("skill plan should be created");
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.rationale, "using pre-built skill: ci");
        assert_eq!(state.status, AgentStatus::Waiting);
        assert_eq!(state.current_step, 1);
    }

    #[tokio::test]
    async fn test_run_step_completes_immediately_when_plan_is_already_complete() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        state.mark_waiting(chrono::Utc::now());
        state.current_step = 1;
        state.metadata["last_reflection"] = serde_json::json!("all done");
        let mut plan = Some(make_plan());
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("completion check should succeed");

        assert!(matches!(outcome, StepOutcome::Complete));
        assert_eq!(state.status, AgentStatus::Completed);
    }

    #[tokio::test]
    async fn test_run_step_retry_keeps_same_step_and_reschedules() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: false,
                skipped: false,
                output: "temporary failure".into(),
                final_answer_candidate: Some("temporary failure".into()),
                tool_results: vec![ToolResult::err("timeout")],
                tools_called: vec!["shell".into()],
                items_processed: 0,
                connector_writes: vec![],
            }])),
            Arc::new(MockEvaluator::from_responses(vec![EvalVerdict::Retry])),
            Arc::new(MockReflector::from_responses(vec![Reflection {
                summary: "retry after timeout".into(),
                key_findings: vec![],
                revised_plan: None,
            }])),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        state.mark_waiting(chrono::Utc::now());
        let mut plan = Some(make_plan());
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("retry path should succeed");

        match outcome {
            StepOutcome::Continue { delay_secs } => assert_eq!(delay_secs, 10),
            other => panic!("expected continue/retry outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Waiting);
        assert_eq!(state.current_step, 0);
        assert_eq!(history.len(), 1);
    }

    #[tokio::test]
    async fn test_run_step_abort_marks_failed() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: false,
                skipped: false,
                output: "STEP FAILED: permission denied".into(),
                final_answer_candidate: None,
                tool_results: vec![ToolResult::err("permission denied")],
                tools_called: vec!["file_write".into()],
                items_processed: 0,
                connector_writes: vec![],
            }])),
            Arc::new(MockEvaluator::from_responses(vec![EvalVerdict::Abort])),
            Arc::new(MockReflector::from_responses(vec![Reflection {
                summary: "permission denied".into(),
                key_findings: vec![],
                revised_plan: None,
            }])),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        state.mark_waiting(chrono::Utc::now());
        let mut plan = Some(make_plan());
        let mut history = StepHistory::new();

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("abort path should succeed");

        match outcome {
            StepOutcome::PolicyViolation { reason } => assert!(reason.contains("permission denied")),
            other => panic!("expected failed outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Failed);
    }

    #[tokio::test]
    async fn test_run_step_goal_complete_verdict_marks_completed() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
                skipped: false,
                output: "done".into(),
                final_answer_candidate: Some("done".into()),
                tool_results: vec![],
                tools_called: vec![],
                items_processed: 0,
                connector_writes: vec![],
            }])),
            Arc::new(MockEvaluator::from_responses(vec![EvalVerdict::GoalComplete])),
            Arc::new(MockReflector::from_responses(vec![Reflection {
                summary: "goal finished".into(),
                key_findings: vec![],
                revised_plan: None,
            }])),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let mut state = make_state();
        state.mark_waiting(chrono::Utc::now());
        let mut plan = Some(make_plan());
        let mut history = StepHistory::new();

        let outcome = loop_runtime
            .run_step(&mut state, &mut plan, &mut history)
            .await
            .expect("goal complete path should succeed");

        assert!(matches!(outcome, StepOutcome::Complete));
        assert_eq!(state.status, AgentStatus::Completed);
    }

    #[test]
    fn test_build_coordinator_shell_plan_has_orchestration_steps() {
        let loop_runtime = make_loop(
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );

        let mut state = make_state();
        state.pending_children = vec!["child-1".into()];
        state.metadata["worker_messages"] = serde_json::json!([
            { "id": "msg-1", "body": "research complete" }
        ]);

        let mut role = crate::agent::definition::AgentRole::new(
            "role-1".into(),
            state.id.clone(),
            state.tenant_id.clone(),
            "Coordinator".into(),
        );
        role.role_category = crate::agent::definition::RoleCategory::ResearchAnalyst;
        role.execution_guidelines.execution_strategy = crate::agent::definition::ExecutionStrategy::CoordinatorShell;
        role.execution_guidelines.tool_pool = crate::agent::definition::ToolPool::Coordinator;
        role.execution_guidelines.add_priority("step: synthesize worker findings");

        let plan = loop_runtime.build_coordinator_shell_plan(&mut state, &role);

        assert_eq!(plan.steps.len(), 7);
        assert_eq!(plan.steps[0].tool.as_deref(), Some("task_list"));
        assert_eq!(plan.steps[1].tool.as_deref(), Some("delegate"));
        assert_eq!(plan.steps[2].tool.as_deref(), Some("message_inbox"));
        assert_eq!(plan.steps[3].tool.as_deref(), Some(crate::agent::workflow_compiler::LLM_WORKER_TOOL_NAME));
        assert_eq!(plan.steps[4].tool.as_deref(), Some("delegate"));
        assert_eq!(plan.steps[5].tool.as_deref(), Some("delegate"));
        assert_eq!(plan.steps[6].tool.as_deref(), Some("task_output"));
        assert_eq!(
            plan.steps[1]
                .tool_args
                .as_ref()
                .and_then(|args| args.get("continue_child_id"))
                .and_then(|value| value.as_str()),
            Some("child-1")
        );
        assert_eq!(state.metadata["coordinator_shell"]["mode"].as_str(), Some("coordinator_shell"));
    }
}
