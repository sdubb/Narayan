use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};

use crate::{
    agent::{
        clarifier::{ClarificationResult, Clarifier},
        evaluator::{EvalVerdict, Evaluator},
        executor::Executor,
        planner::{Plan, Planner},
        preflight::{Preflight, PreflightResult},
        prompts::StepHistory,
        reflector::Reflector,
    },
    cognition::control_loop::CognitiveControlLoop,
    compliance::sla::{EscalationAction, SlaStatus},
    debug::recorder::AgentRecorder,
    events::{AgentEvent, EventBus},
    knowledge::graph::KnowledgeGraph,
    segments::registry::AgentServices,
    skill_evolution::evolution::evolve_skill,
    skills::registry::SkillRegistry,
    state::{AgentState, AgentStatus},
    tools::ToolRegistry,
    util::next_run_after,
};

// ── Outcome ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum StepOutcome {
    Continue { delay_secs: i64 },
    NeedsClarification { questions: Vec<String> },
    Infeasible { reason: String },
    Complete,
    Failed(String),
    Delegating { child_ids: Vec<String> },
}

// ── AgentLoop ──────────────────────────────────────────────────────────────

pub struct AgentLoop {
    planner:        Arc<dyn Planner>,
    executor:       Arc<dyn Executor>,
    evaluator:      Arc<dyn Evaluator>,
    reflector:      Arc<dyn Reflector>,
    preflight:      Arc<dyn Preflight>,
    clarifier:      Arc<dyn Clarifier>,
    tools:          Arc<ToolRegistry>,
    event_bus:      Arc<EventBus>,
    skill_registry: Arc<RwLock<SkillRegistry>>,
    knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
    vector_store:   Arc<crate::memory::PgVectorStore>,
    embedder:       Arc<dyn crate::memory::EmbeddingModel>,
    services:       Arc<AgentServices>,
    max_steps:      usize,
    timeout_secs:   u64,
}

impl AgentLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        planner:        Arc<dyn Planner>,
        executor:       Arc<dyn Executor>,
        evaluator:      Arc<dyn Evaluator>,
        reflector:      Arc<dyn Reflector>,
        preflight:      Arc<dyn Preflight>,
        clarifier:      Arc<dyn Clarifier>,
        tools:          Arc<ToolRegistry>,
        event_bus:      Arc<EventBus>,
        skill_registry: Arc<RwLock<SkillRegistry>>,
        knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
        vector_store:   Arc<crate::memory::PgVectorStore>,
        embedder:       Arc<dyn crate::memory::EmbeddingModel>,
        services:       Arc<AgentServices>,
    ) -> Self {
        Self {
            planner, executor, evaluator, reflector, preflight, clarifier,
            tools, event_bus, skill_registry, knowledge_graph,
            vector_store, embedder, services,
            max_steps: 50, timeout_secs: 300,
        }
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
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            });
        }

        // ── 4. Planning ─────────────────────────────────────────────────────
        if plan.is_none() {
            self.event_bus.publish(AgentEvent::PlanningStarted { agent_id: state.id.clone() });

            let tool_names: Vec<&str> = self.tools.list();
            let ctx = state.metadata.get("last_reflection").and_then(|v| v.as_str()).unwrap_or("").to_string();

            // 4a. Check skill registry first — skip LLM planning if a skill matches
            let maybe_skill = {
                let reg = self.skill_registry.read().await;
                reg.get(&state.goal).cloned().or_else(|| {
                    // Fuzzy match: check if any skill name appears in the goal
                    reg.find_matching(&state.goal).cloned()
                })
            };

            let new_plan = if let Some(skill) = maybe_skill {
                tracing::info!(
                    agent_id = %state.id,
                    skill    = %skill.name,
                    "using pre-built skill — skipping LLM planning"
                );
                Plan::from_skill(&skill)
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
                let p = self.planner.create_plan(state, &ctx, &tool_names).await?;
                state.goal = orig;
                p
            };

            self.event_bus.publish(AgentEvent::PlanCreated {
                agent_id: state.id.clone(),
                step_count: new_plan.steps.len(),
                rationale: new_plan.rationale.clone(),
            });
            *plan = Some(new_plan);
        }

        let current_plan = plan.as_ref().unwrap();

        // ── 5. Completion check ─────────────────────────────────────────────
        if current_plan.is_complete(state.current_step as usize) {
            let summary =
                state.metadata.get("last_reflection").and_then(|v| v.as_str()).unwrap_or("goal achieved").to_string();
            state.mark_completed();
            self.event_bus.publish(AgentEvent::GoalComplete { agent_id: state.id.clone(), summary: summary.clone() });
            self.event_bus.close(&state.id);
            return Ok(StepOutcome::Complete);
        }

        let step = current_plan.next_step(state.current_step as usize).unwrap().clone();

        self.event_bus.publish(AgentEvent::StepStarted {
            agent_id: state.id.clone(),
            step_index: step.index,
            description: step.description.clone(),
        });
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
        let result = self.executor.execute_step(state, &step_exec, current_plan, history).await?;

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
        for tool_result in &result.tool_results {
            self.event_bus.publish(AgentEvent::ToolResult {
                agent_id: state.id.clone(),
                step_index: step.index,
                tool_name: "tool".into(),
                success: tool_result.success,
                output_preview: crate::util::truncate(
                    &serde_json::to_string(&tool_result.output).unwrap_or_default(),
                    100,
                )
                .to_string(),
            });
        }

        // ── 9. Evaluate + Reflect (one combined LLM call) ──────────────────
        let retry_count = state.metadata.get("retry_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let eval = self.evaluator.evaluate_and_reflect(state, current_plan, &step, &result, retry_count).await?;

        state.metadata["last_reflection"] = serde_json::Value::String(eval.summary.clone());
        state.metadata["key_findings"] = serde_json::json!(eval.key_findings);

        // Update step history for next executor call
        history.push(step.index, step.description.clone(), result.success, &eval.summary);

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
        let maybe_revised = if eval.should_revise && !eval.revision_feedback.is_empty() {
            match self.reflector.revise_plan(current_plan, state, &eval.revision_feedback).await {
                Ok(p) => {
                    tracing::info!(agent_id = %state.id, "plan revised from eval feedback");
                    Some(p)
                }
                Err(e) => {
                    tracing::warn!(agent_id = %state.id, error = %e, "plan revision failed");
                    None
                }
            }
        } else {
            None
        };

        if let Some(revised) = maybe_revised {
            *plan = Some(revised);
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
                    let _ = ct.record(
                        &state.id,
                        &state.tenant_id,
                        step.index,
                        &eval.summary,
                        "tool_output",
                        tool_name,
                        &crate::util::truncate(&result.output, 200),
                        confidence,
                    ).await;
                    // Emit SSE so the frontend can render citations live
                    self.event_bus.publish(AgentEvent::CitationRecorded {
                        agent_id:    state.id.clone(),
                        step_index:  step.index,
                        claim:       crate::util::truncate(&eval.summary, 120).to_string(),
                        source_ref:  tool_name.clone(),
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
                                    agent_id:    state.id.clone(),
                                    pct_elapsed,
                                    message:     reason.clone(),
                                    action:      Some("escalate".into()),
                                    deadline:    Some(deadline_str.clone()),
                                });
                                if let Some(ref rq) = self.services.reviews {
                                    match rq.submit(
                                        &state.tenant_id,
                                        &state.id,
                                        step.index,
                                        reason,
                                        "sla_escalation",
                                    ).await {
                                        Ok(review_id) => {
                                            self.event_bus.publish(AgentEvent::ReviewRequired {
                                                agent_id:  state.id.clone(),
                                                review_id,
                                                summary:   reason.clone(),
                                                reason:    "SLA breach escalation".into(),
                                                rule_id:   Some("sla_escalation".into()),
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
                                    agent_id:    state.id.clone(),
                                    pct_elapsed,
                                    message:     message.clone(),
                                    action:      Some("notify".into()),
                                    deadline:    Some(deadline_str.clone()),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // ── 13. Advance state ───────────────────────────────────────────────
        match eval.verdict {
            EvalVerdict::Continue => {
                state.advance_step();
                state.mark_waiting(next_run_after(0));
                self.event_bus.publish(AgentEvent::StepCompleted {
                    agent_id: state.id.clone(),
                    step_index: step.index,
                    success: true,
                    summary: eval.summary,
                });
                Ok(StepOutcome::Continue { delay_secs: 0 })
            }
            EvalVerdict::GoalComplete => {
                state.mark_completed();
                self.event_bus
                    .publish(AgentEvent::GoalComplete { agent_id: state.id.clone(), summary: eval.summary });
                self.event_bus.close(&state.id);
                Ok(StepOutcome::Complete)
            }
            EvalVerdict::Retry => {
                state.mark_waiting(next_run_after(10));
                self.event_bus.publish(AgentEvent::StepRetrying {
                    agent_id: state.id.clone(),
                    step_index: step.index,
                    delay_secs: 10,
                    reason: eval.summary.clone(),
                });
                Ok(StepOutcome::Continue { delay_secs: 10 })
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
        match self.preflight.check(state, &tool_names).await? {
            PreflightResult::Feasible => {
                self.event_bus.publish(AgentEvent::PreflightPassed { agent_id: state.id.clone() });
                match self.clarifier.check(state).await? {
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

    fn inject_delegation_ctx(
        &self,
        step: &crate::agent::planner::PlannedStep,
        state: &AgentState,
    ) -> crate::agent::planner::PlannedStep {
        // Tools that need tenant_id / agent_id injected automatically
        let needs_ctx = matches!(
            step.tool.as_deref(),
            Some("delegate") | Some("vector_store") | Some("vector_search") | Some("vector_delete")
            | Some("mcp_session") | Some("search_mcp_registry")
        );
        if needs_ctx {
            let mut s = step.clone();
            let mut args = step.tool_args.clone().unwrap_or_default();
            args["tenant_id"] = serde_json::json!(state.tenant_id);
            args["agent_id"]  = serde_json::json!(state.id);
            if step.tool.as_deref() == Some("delegate") {
                args["parent_agent_id"] = serde_json::json!(state.id);
            }
            s.tool_args = Some(args);
            s
        } else {
            step.clone()
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
        planner: Arc<dyn Planner>,
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
            planner,
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
            Arc::new(crate::segments::registry::AgentServices::none()),
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

        let outcome = StepOutcome::NeedsClarification { questions: vec!["What scope?".to_string()] };
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
                questions: vec!["Which repository should be fixed?".into()],
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
                assert_eq!(questions, vec!["Which repository should be fixed?".to_string()])
            }
            other => panic!("expected clarification outcome, got {other:?}"),
        }
        assert_eq!(state.status, AgentStatus::Clarifying);
        assert_eq!(
            state.metadata["clarification_questions"][0],
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
                tool_results: vec![ToolResult::ok(serde_json::json!({
                    "child_agent_ids": ["child-1", "child-2"]
                }))],
                tools_called: vec!["delegate".into()],
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
                tool_results: vec![],
                tools_called: vec![],
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
            StepOutcome::NeedsClarification { questions } => assert_eq!(questions, vec!["Which repo?".to_string()]),
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
                tool_results: vec![],
                tools_called: vec![],
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

        let outcome =
            loop_runtime.run_step(&mut state, &mut plan, &mut history).await.expect("skill plan path should succeed");

        match outcome {
            StepOutcome::Continue { delay_secs } => assert_eq!(delay_secs, 0),
            other => panic!("expected continue outcome, got {other:?}"),
        }
        let plan = plan.expect("skill plan should be created");
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.rationale, "using pre-built skill: ci");
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
                tool_results: vec![ToolResult::err("timeout")],
                tools_called: vec!["shell".into()],
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
                tool_results: vec![ToolResult::err("permission denied")],
                tools_called: vec!["file_write".into()],
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
                tool_results: vec![],
                tools_called: vec![],
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
