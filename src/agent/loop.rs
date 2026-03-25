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
    },
    cognition::{
        control_loop::CognitiveControlLoop,
        judgement::{JudgementContext, JudgementEngine, JudgementRecommendation, JudgementSignal},
    },
    compliance::sla::{EscalationAction, SlaStatus},
    debug::recorder::AgentRecorder,
    events::{AgentEvent, EventBus},
    knowledge::graph::KnowledgeGraph,
    segments::AgentServices,
    skill_evolution::evolution::evolve_skill,
    skills::registry::SkillRegistry,
    state::{AgentState, AgentStatus},
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

fn persist_step_output(
    state: &mut AgentState,
    step: &crate::agent::planner::PlannedStep,
    result: &crate::agent::executor::StepResult,
) {
    let record = serde_json::json!({
        "step_index": step.index,
        "description": step.description,
        "success": result.success,
        "output": result.output,
        "final_answer_candidate": result.final_answer_candidate,
        "tools_called": result.tools_called,
        "tool_results": result.tool_results,
    });

    if let Some(outputs) = state.metadata.get_mut("step_outputs").and_then(|value| value.as_array_mut()) {
        if outputs.len() <= step.index {
            outputs.resize(step.index + 1, serde_json::Value::Null);
        }
        outputs[step.index] = record;
    } else {
        let mut outputs = Vec::new();
        outputs.resize(step.index + 1, serde_json::Value::Null);
        outputs[step.index] = record;
        state.metadata["step_outputs"] = serde_json::Value::Array(outputs);
    }
}

fn persist_skipped_step_output(state: &mut AgentState, step: &crate::agent::planner::PlannedStep, summary: &str) {
    let record = serde_json::json!({
        "step_index": step.index,
        "description": step.description,
        "success": true,
        "skipped": true,
        "output": summary,
        "final_answer_candidate": serde_json::Value::Null,
        "tools_called": [],
        "tool_results": [],
    });

    if let Some(outputs) = state.metadata.get_mut("step_outputs").and_then(|value| value.as_array_mut()) {
        if outputs.len() <= step.index {
            outputs.resize(step.index + 1, serde_json::Value::Null);
        }
        outputs[step.index] = record;
    } else {
        let mut outputs = Vec::new();
        outputs.resize(step.index + 1, serde_json::Value::Null);
        outputs[step.index] = record;
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
    step.condition.as_ref().map(|condition| {
        let value = condition
            .value
            .as_ref()
            .map(|value| match value {
                serde_json::Value::String(text) => format!(" \"{text}\""),
                other => format!(" {}", other),
            })
            .unwrap_or_default();
        format!("{} {}{}", condition.reference, condition.operator, value)
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
    Failed(String),
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
        prompt: "Add an LLM provider API key to continue.".into(),
        placeholder: Some("After adding credentials in Settings, click Submit to retry.".into()),
        helper_text: Some(
            "This workspace does not have any tenant provider credentials configured, and no platform fallback key is available."
                .into(),
        ),
        options: Vec::new(),
        required: false,
        secret: false,
        store_as_credential: None,
        connector_type: Some("provider_credentials".into()),
        action_label: Some("Open Settings -> Credentials".into()),
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
    vector_store: Arc<crate::memory::PgVectorStore>,
    embedder: Arc<dyn crate::memory::EmbeddingModel>,
    services: Arc<AgentServices>,
    store: Option<Arc<crate::storage::PostgresStore>>,
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
        vector_store: Arc<crate::memory::PgVectorStore>,
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
            services,
            store: None,
            max_steps: 50,
            timeout_secs: 300,
        }
    }

    pub fn with_store(mut self, store: Arc<crate::storage::PostgresStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_limits(mut self, max_steps: usize, timeout_secs: u64) -> Self {
        self.max_steps = max_steps;
        self.timeout_secs = timeout_secs;
        self
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
                        index: 0,
                        description: "Answer the user's message directly in chat.".into(),
                        tool: None,
                        tool_args: None,
                        success_criteria: "User receives a direct answer.".into(),
                        condition: None,
                    }],
                }
            } else if let Some(skill) = maybe_skill {
                tracing::info!(
                    agent_id = %state.id,
                    skill    = %skill.name,
                    "using pre-built skill — skipping LLM planning"
                );
                Plan::from_skill(&skill)
            } else if let Some(workflow_plan) = self.try_plan_from_workflow_outline(state).await {
                tracing::info!(
                    agent_id = %state.id,
                    steps    = workflow_plan.steps.len(),
                    "using workflow outline — skipping LLM planning"
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
                let reason = format!(
                    "runtime does not invent plans anymore; rerun plan mode to produce a workflow outline for '{}'",
                    state.goal
                );
                tracing::error!(
                    agent_id = %state.id,
                    goal = %state.goal,
                    reason = %reason,
                    "no deterministic runtime plan available"
                );
                state.goal = orig;
                state.mark_failed();
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
            *plan = Some(new_plan);
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
            self.event_bus.publish(AgentEvent::StepCompleted {
                agent_id: state.id.clone(),
                step_index: step.index,
                success: true,
                summary,
                description: Some(step.description.clone()),
            });
            return Ok(StepOutcome::Continue { delay_secs: 0 });
        }

        self.event_bus.publish(AgentEvent::StepStarted {
            agent_id: state.id.clone(),
            step_index: step.index,
            description: step.description.clone(),
            tool: step.tool.clone(),
            success_criteria: (!step.success_criteria.trim().is_empty()).then(|| step.success_criteria.clone()),
            condition: format_step_condition(&step),
        });
        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            step_description = %step.description,
            planner_hint = ?step.tool,
            "agent loop executing step"
        );
        state.mark_running();

        // ── 5a. Inject knowledge graph facts into history ───────────────────
        {
            let graph = self.knowledge_graph.lock().await;
            let facts = graph.get_related(&state.goal);
            if !facts.is_empty() {
                let facts_text = facts.iter().map(|n| format!("{}: {}", n.id, n.value)).collect::<Vec<_>>().join("\n");
                history.inject_facts(&facts_text);
                tracing::debug!(
                    agent_id = %state.id,
                    fact_count = facts.len(),
                    "injected knowledge graph facts into executor context"
                );
            }
        }

        // ── 5b. Inject delegation context for delegate tool ────────────────
        let step_exec = self.inject_delegation_ctx(&step, state);

        // ── 6. Execute step ────────────────────────────────────────────────
        // plane_guard validated inside executor before each tool call
        let result = match self.executor.execute_step(state, &step_exec, current_plan, history).await {
            Ok(result) => result,
            Err(error) if is_missing_provider_credentials_error(&error) => {
                return self.prompt_for_provider_credentials(state);
            }
            Err(error) => return Err(error),
        };
        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            success = result.success,
            tools_called = ?result.tools_called,
            output = %truncate_for_log(&result.output, 400),
            "agent loop executor result"
        );

        // ── 7. Debug recording ─────────────────────────────────────────────
        {
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

        if let Some(candidate) = result.final_answer_candidate.clone() {
            state.set_final_answer(candidate);
        }
        persist_step_output(state, &step, &result);

        // ── 8. Delegation check ─────────────────────────────────────────────
        for tool_result in &result.tool_results {
            if let Some(arr) = tool_result.output.get("child_agent_ids").and_then(|v| v.as_array()) {
                let child_ids: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                if !child_ids.is_empty() {
                    for cid in &child_ids {
                        self.event_bus.publish(AgentEvent::ChildSpawned {
                            agent_id: state.id.clone(),
                            child_agent_id: cid.clone(),
                            sub_goal: step.description.clone(),
                        });
                    }
                    state.advance_step();
                    state.mark_delegating(child_ids.clone());
                    return Ok(StepOutcome::Delegating { child_ids });
                }
            }
        }

        // Emit tool result events
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

        if let Some(question_output) = result.tool_results.iter().find(|tool_result| {
            tool_result.output.get("status").and_then(|v| v.as_str()) == Some("awaiting_user_input")
        }) {
            let questions = crate::agent::clarifier::parse_clarification_questions(
                question_output.output.get("questions").unwrap_or(&serde_json::Value::Null),
            );
            if !questions.is_empty() {
                state.metadata["clarification_questions"] = serde_json::to_value(&questions)?;
                state.mark_clarifying();
                self.event_bus.publish(AgentEvent::ClarificationNeeded {
                    agent_id: state.id.clone(),
                    questions: questions.clone(),
                });
                return Ok(StepOutcome::NeedsClarification { questions });
            }
        }

        if is_direct_response_goal(&state.goal)
            && current_plan.steps.len() == 1
            && step.tool.is_none()
            && result.success
            && result.tool_results.is_empty()
        {
            let answer = state.final_answer().map(str::to_string).unwrap_or_else(|| result.output.clone());
            state.set_final_answer(answer.clone());
            state.metadata["last_reflection"] = serde_json::Value::String(answer.clone());
            state.metadata["key_findings"] = serde_json::json!([]);
            history.push(step.index, step.description.clone(), true, &answer);
            state.mark_completed();
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

        // ── 9. Evaluate + Reflect (one combined LLM call) ──────────────────
        let retry_count = state.metadata.get("retry_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let eval = self.evaluator.evaluate_and_reflect(state, current_plan, &step, &result, retry_count).await?;
        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            verdict = ?eval.verdict,
            summary = %truncate_for_log(&eval.summary, 300),
            should_revise = eval.should_revise,
            "agent loop evaluation complete"
        );

        state.metadata["last_reflection"] = serde_json::Value::String(eval.summary.clone());
        state.metadata["key_findings"] = serde_json::json!(eval.key_findings);

        // ── Write step_outputs for CompletionCriteria and savings estimator ──
        if result.items_processed > 0 || !result.connector_writes.is_empty() {
            let entry = serde_json::json!({
                "step":      step.index,
                "success":   result.success,
                "processed": result.items_processed,
                "connectors": result.connector_writes,
            });
            let mut outputs =
                state.metadata.get("step_outputs").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            outputs.push(entry);
            state.metadata["step_outputs"] = serde_json::Value::Array(outputs);
        }

        // ── Apply FailureAction overrides from role guidelines ────────────
        // If a FailureRule matches the current step failure, override the
        // evaluator verdict before the match arm below.
        let eval_verdict = if !result.success {
            let role_id = state.metadata.get("role_id").and_then(|v| v.as_str()).map(String::from);
            if let (Some(ref store), Some(ref rid)) = (&self.store, role_id.as_ref()) {
                if let Ok(Some(role)) = store.get_agent_role(&state.tenant_id, rid).await {
                    apply_failure_action_override(eval.verdict.clone(), &result, &role, state, &self.services)
                } else {
                    eval.verdict.clone()
                }
            } else {
                eval.verdict.clone()
            }
        } else {
            eval.verdict.clone()
        };

        // Update step history for next executor call
        let history_summary = step_history_summary(&result, &eval.summary);
        history.push(step.index, step.description.clone(), result.success, &history_summary);
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

        // ── 11. Knowledge graph — extract and store entities ────────────────
        {
            let mut graph = self.knowledge_graph.lock().await;
            for finding in extract_entities(&eval.summary) {
                graph.add_node(finding.0, finding.1);
            }
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

        // ── 12a. Optional plan revision from combined eval ───────────────────
        if eval.should_revise && !eval.revision_feedback.is_empty() {
            tracing::info!(
                agent_id = %state.id,
                step_index = step.index,
                "runtime revision requested by evaluator; leaving repair to plan mode"
            );
        }

        // ── 12b. Persist key findings to pgvector ────────────────────────────
        if result.success && !eval.summary.is_empty() {
            use crate::memory::VectorStore;
            let content = format!("Step {} — {} | Finding: {}", step.index, step.description, eval.summary);
            match self.embedder.embed(&content).await {
                Ok(embedding) => {
                    let doc = crate::memory::VectorDocument::new(
                        state.tenant_id.clone(),
                        state.id.clone(),
                        content,
                        embedding,
                    )
                    .with_metadata(serde_json::json!({
                        "step":  step.index,
                        "goal":  state.goal,
                        "auto":  true,
                    }));
                    if let Err(e) = self.vector_store.upsert(doc).await {
                        tracing::debug!(error = %e, "vector upsert failed — continuing");
                    }
                }
                Err(e) => tracing::debug!(error = %e, "embedding failed — skipping vector store"),
            }
        }

        // ── 12c. Citation tracking — record source attribution per step ──────
        if result.success && !eval.summary.is_empty() {
            if let Some(ref ct) = self.services.citations {
                for tool_name in &result.tools_called {
                    let confidence = if result.success { 1.0_f64 } else { 0.5_f64 };
                    let _ = ct
                        .record(
                            &state.id,
                            &state.tenant_id,
                            step.index,
                            &eval.summary,
                            "tool_output",
                            tool_name,
                            &crate::util::truncate(&result.output, 200),
                            confidence,
                        )
                        .await;
                    // Emit SSE so the frontend can render citations live
                    self.event_bus.publish(AgentEvent::CitationRecorded {
                        agent_id: state.id.clone(),
                        step_index: step.index,
                        claim: crate::util::truncate(&eval.summary, 120).to_string(),
                        source_ref: tool_name.clone(),
                        source_type: "tool_output".into(),
                        confidence,
                    });
                }
                tracing::debug!(
                    agent_id   = %state.id,
                    step       = step.index,
                    tool_count = result.tools_called.len(),
                    "citations recorded"
                );
            }
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
                state.advance_step();
                state.mark_waiting(next_run_after(0));
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
                self.event_bus.publish(AgentEvent::GoalFailed { agent_id: state.id.clone(), reason: reason.clone() });
                self.event_bus.close(&state.id);
                Ok(StepOutcome::Failed(reason))
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
                self.event_bus.publish(AgentEvent::PreflightFailed { agent_id: state.id.clone(), reason: msg.clone() });
                self.event_bus.close(&state.id);
                Ok(StepOutcome::Infeasible { reason: msg })
            }
        }
    }

    /// Try to build a deterministic Plan from the role's enriched workflow outline.
    /// Returns None if no role is found or the workflow outline is empty, causing
    /// the caller to fail fast and let plan mode repair the role.
    async fn try_plan_from_workflow_outline(&self, state: &AgentState) -> Option<Plan> {
        let store = self.store.as_ref()?;
        let role_id = state.metadata.get("role_id").and_then(|v| v.as_str())?;
        let role = store.get_agent_role(&state.tenant_id, role_id).await.ok()??;

        if !role.execution_guidelines.has_workflow_outline() {
            return None;
        }

        let input_data = state.metadata.get("input_data").cloned().unwrap_or_else(|| serde_json::json!({}));

        Some(Plan::from_workflow_outline(&role, &input_data))
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
            if step.tool.as_deref() == Some("delegate") {
                args["parent_agent_id"] = serde_json::json!(state.id);
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

        let resolved = crate::agent::executor::resolve_reference_from_state(&condition.reference, state)
            .map_err(anyhow::Error::msg);
        let operator = condition.operator.as_str();

        let should_run = match operator {
            "exists" => resolved.is_ok(),
            "not_exists" => resolved.is_err(),
            "truthy" => resolved.map(|value| condition_truthy(&value)).unwrap_or(false),
            "falsy" => resolved.map(|value| !condition_truthy(&value)).unwrap_or(true),
            "nonempty" => resolved.map(|value| condition_truthy(&value)).unwrap_or(false),
            "empty" => resolved.map(|value| !condition_truthy(&value)).unwrap_or(true),
            "equals" => {
                let actual = resolved?;
                let expected = condition
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("condition.operator 'equals' requires condition.value"))?;
                actual == *expected
            }
            "not_equals" => {
                let actual = resolved?;
                let expected = condition
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("condition.operator 'not_equals' requires condition.value"))?;
                actual != *expected
            }
            "contains" => {
                let actual = resolved?;
                let expected = condition
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("condition.operator 'contains' requires condition.value"))?;
                condition_contains(&actual, expected)
            }
            "gt" => {
                let actual = resolved?;
                let expected = condition
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("condition.operator 'gt' requires condition.value"))?;
                condition_compare_numbers(&actual, expected, |left, right| left > right)?
            }
            "gte" => {
                let actual = resolved?;
                let expected = condition
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("condition.operator 'gte' requires condition.value"))?;
                condition_compare_numbers(&actual, expected, |left, right| left >= right)?
            }
            "lt" => {
                let actual = resolved?;
                let expected = condition
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("condition.operator 'lt' requires condition.value"))?;
                condition_compare_numbers(&actual, expected, |left, right| left < right)?
            }
            "lte" => {
                let actual = resolved?;
                let expected = condition
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("condition.operator 'lte' requires condition.value"))?;
                condition_compare_numbers(&actual, expected, |left, right| left <= right)?
            }
            other => return Err(anyhow::anyhow!("unsupported step condition operator '{other}'")),
        };

        if should_run {
            Ok(None)
        } else {
            Ok(Some(format!(
                "Skipped step {} because condition {} {} was not satisfied.",
                step.index, condition.reference, condition.operator
            )))
        }
    }
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

// ── FailureAction override ────────────────────────────────────────────────

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
            planner::{Plan, PlannedStep, Planner},
            reflector::Reflection,
            test_helpers::{MockClarifier, MockEvaluator, MockExecutor, MockPlanner, MockPreflight, MockReflector},
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
                index: 0,
                description: "Inspect failing workflow".into(),
                tool: Some("file_read".into()),
                tool_args: Some(serde_json::json!({"path": ".github/workflows/ci.yml"})),
                success_criteria: "workflow reviewed".into(),
                condition: None,
            }],
            rationale: "inspect before changing".into(),
        }
    }

    fn make_loop(
        planner: Arc<dyn Planner>,
        executor: Arc<dyn Executor>,
        evaluator: Arc<dyn Evaluator>,
        reflector: Arc<dyn Reflector>,
        preflight: Arc<dyn Preflight>,
        clarifier: Arc<dyn Clarifier>,
    ) -> AgentLoop {
        make_loop_with_registry(planner, executor, evaluator, reflector, preflight, clarifier, SkillRegistry::new())
    }

    fn make_loop_with_registry(
        _planner: Arc<dyn Planner>,
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
            Arc::new(MockPlanner::new()),
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
            Arc::new(MockPlanner::new()),
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
            Arc::new(MockPlanner::new()),
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
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
            Arc::new(MockPlanner::new()),
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
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
    async fn test_inject_delegation_ctx_adds_agent_and_tenant_identifiers() {
        let loop_runtime = make_loop(
            Arc::new(MockPlanner::new()),
            Arc::new(MockExecutor::new()),
            Arc::new(MockEvaluator::new()),
            Arc::new(MockReflector::new()),
            Arc::new(MockPreflight::new()),
            Arc::new(MockClarifier::new()),
        );
        let state = make_state();
        let step = PlannedStep {
            index: 0,
            description: "Delegate the sub-task".into(),
            tool: Some("delegate".into()),
            tool_args: Some(serde_json::json!({"goal": "check logs"})),
            success_criteria: "child created".into(),
            condition: None,
        };

        let injected = loop_runtime.inject_delegation_ctx(&step, &state);

        assert_eq!(injected.tool_args.as_ref().and_then(|v| v.get("tenant_id")), Some(&serde_json::json!("tenant-1")));
        assert_eq!(injected.tool_args.as_ref().and_then(|v| v.get("agent_id")), Some(&serde_json::json!("agent-1")));
        assert_eq!(
            injected.tool_args.as_ref().and_then(|v| v.get("parent_agent_id")),
            Some(&serde_json::json!("agent-1"))
        );
    }

    #[tokio::test]
    async fn test_run_step_fails_when_cognitive_control_limit_is_hit() {
        let loop_runtime = make_loop(
            Arc::new(MockPlanner::new()),
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
            Arc::new(MockPlanner::new()),
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
            Arc::new(MockPlanner::new()),
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
            Arc::new(MockPlanner::new()),
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
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
            Arc::new(MockPlanner::new()),
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
            Arc::new(MockPlanner::new()),
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: false,
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
            Arc::new(MockPlanner::new()),
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: false,
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
            StepOutcome::Failed(reason) => assert!(reason.contains("permission denied")),
            other => panic!("expected failed outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Failed);
    }

    #[tokio::test]
    async fn test_run_step_goal_complete_verdict_marks_completed() {
        let loop_runtime = make_loop(
            Arc::new(MockPlanner::new()),
            Arc::new(MockExecutor::from_responses(vec![StepResult {
                step_index: 0,
                success: true,
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
}
