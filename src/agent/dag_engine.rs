//! DAG Engine — the core scheduler loop for deterministic workflows.
//!
//! Replaces the linear `AgentLoop` for workflows that have a DAG-based
//! execution plan. Steps are executed in parallel when independent,
//! with crash-safe durable checkpointing via the `WorkflowStore`.
//!
//! # Key Design Decisions
//!
//! 1. **DB is the single source of truth** — `AgentState` is treated as
//!    read-only config/identity inside the engine. All step state lives in
//!    the `WorkflowStore`.
//!
//! 2. **Steps are isolated pure functions** — each step reads its inputs
//!    from predecessor outputs in the DB and writes its outputs to the DB.
//!    No shared mutable `AgentState`.
//!
//! 3. **Evaluator is optional** — deterministic workflows use engine-managed
//!    success/failure based on tool results. The evaluator is kept as an
//!    optional post-step quality gate (validator, not decision maker).
//!
//! 4. **Scheduler loop with retry timing** — the engine sleeps between
//!    cycles, respecting `next_retry_at` timestamps to avoid busy-looping.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use chrono::{Duration, Utc};
use tokio_util::sync::CancellationToken;
use tracing;

use crate::{
    agent::{
        dag::{StepNode, StepStatus, WorkflowStatus},
        executor::Executor,
        planner::{ConditionOp, Plan, StepCondition, StructuredCondition},
        prompts::StepHistory,
        step_artifacts::write_step_artifact,
        workflow_compiler::TypedExpression,
    },
    events::{AgentEvent, EventBus},
    state::AgentState,
    storage::WorkflowStore,
    tools::validate_output_against_schema,
};

// ═══════════════════════════════════════════════════════════════════════════
// OUTCOMES
// ═══════════════════════════════════════════════════════════════════════════

/// Outcome of a single scheduling cycle.
pub enum CycleOutcome {
    /// More steps are ready — execute next cycle immediately.
    Continue,
    /// No steps ready now; sleep until this time (retry or poll interval).
    WaitUntil(chrono::DateTime<Utc>),
    /// All steps are in terminal states — workflow complete.
    Complete,
    /// Unrecoverable failure.
    Failed(String),
    /// No actionable steps remain — deadlocked.
    Deadlocked(Vec<String>),
}

/// Final outcome of the entire workflow execution.
pub enum WorkflowOutcome {
    Completed,
    Failed(String),
    Cancelled,
}

// ═══════════════════════════════════════════════════════════════════════════
// STEP INPUT — Isolated input built from predecessor outputs in DB
// ═══════════════════════════════════════════════════════════════════════════

/// Isolated input for a step — built from predecessor outputs stored in DB.
/// Ensures step isolation: no shared mutable state.
pub struct StepInput {
    pub data: serde_json::Value,
    pub predecessor_outputs: HashMap<String, serde_json::Value>,
}

impl StepInput {
    /// Load predecessor step outputs from the WorkflowStore (DB).
    pub async fn from_predecessors(store: &dyn WorkflowStore, step: &StepNode) -> Result<Self> {
        let mut predecessor_outputs = HashMap::new();
        for dep_id in &step.depends_on {
            if let Some(output) = store.get_step_output(dep_id).await? {
                predecessor_outputs.insert(dep_id.clone(), output);
            }
        }
        let data = serde_json::json!(predecessor_outputs);
        Ok(Self { data, predecessor_outputs })
    }
}

fn build_dag_step_metadata(step: &StepNode, input: &StepInput) -> serde_json::Value {
    serde_json::json!({
        "dag_step_id": step.id,
        "dag_step_index": step.index,
        "dag_step_input": input.data,
        "dag_predecessor_outputs": input.predecessor_outputs,
    })
}

fn mark_step_running_in_memory(workflow: &mut crate::agent::dag::Workflow, step_id: &str) {
    if let Some(node) = workflow.nodes.iter_mut().find(|n| n.id == step_id) {
        if matches!(node.status, StepStatus::Pending | StepStatus::Ready | StepStatus::Retrying { .. }) {
            node.status = StepStatus::Running;
            if node.started_at.is_none() {
                node.started_at = Some(Utc::now());
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CONDITION EVALUATION — Deterministic skip gate
// ═══════════════════════════════════════════════════════════════════════════

/// Resolve a dot-notation path against predecessor outputs.
///
/// Supports three path formats:
///   - `$.deps.step-0.output.count`
///   - `step-0.output.count`
///   - `result_of_step_0.output.count`
///
/// Returns `None` if the path doesn't resolve (used by Exists/NotExists).
fn resolve_path(path: &str, input: &StepInput) -> Option<serde_json::Value> {
    let trimmed = path.trim();

    // Strip leading `$.deps.` or `deps.` prefix
    let normalized = trimmed.strip_prefix("$.deps.").or_else(|| trimmed.strip_prefix("deps.")).unwrap_or(trimmed);

    // Split into segments: first segment is the step reference, rest is the path
    let mut segments: Vec<&str> = normalized.split('.').collect();
    if segments.is_empty() {
        return None;
    }

    // Resolve the step key from the first segment
    let step_key = if let Some(rest) = segments[0].strip_prefix("result_of_step_") {
        // "result_of_step_0" → "step-0"
        let digit_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digit_len == 0 {
            return None;
        }
        let step_id = format!("step-{}", &rest[..digit_len]);
        // If there's trailing text after digits in this segment (shouldn't be), ignore it
        segments[0] = ""; // consumed
        step_id
    } else {
        // "step-0" or any direct key into predecessor_outputs
        let key = segments[0].to_string();
        segments[0] = ""; // consumed
        key
    };

    // Look up the step in predecessor outputs
    let mut current = input.predecessor_outputs.get(&step_key)?.clone();

    // Walk remaining path segments
    for segment in &segments[1..] {
        if segment.is_empty() {
            continue;
        }
        // Try as object key first
        if let Some(val) = current.get(*segment) {
            current = val.clone();
        }
        // Try as array index
        else if let Ok(idx) = segment.parse::<usize>() {
            current = current.get(idx)?.clone();
        } else {
            return None;
        }
    }

    Some(current)
}

/// Evaluate a step condition against predecessor data.
///
/// Returns `true` if the step should execute, `false` if it should be skipped.
/// On resolution failure (bad path), defaults to `true` (fail-open).
fn evaluate_condition(condition: &StepCondition, input: &StepInput) -> bool {
    match condition {
        StepCondition::Deterministic(cond) => evaluate_structured_condition(cond, input),
        StepCondition::Expression(expr) => evaluate_typed_condition(expr, input),
    }
}

fn evaluate_typed_condition(expr: &TypedExpression, input: &StepInput) -> bool {
    let result = evaluate_typed_expression(expr, input);
    match result {
        Some(serde_json::Value::Bool(value)) => value,
        Some(other) => {
            tracing::warn!(expr = ?expr, result = ?other, "typed expression did not evaluate to a boolean");
            false
        }
        None => {
            tracing::warn!(expr = ?expr, "typed expression failed to evaluate; defaulting to false");
            false
        }
    }
}

fn evaluate_typed_expression(expr: &TypedExpression, input: &StepInput) -> Option<serde_json::Value> {
    if let Some(value) = &expr.value {
        return Some(value.clone());
    }

    if let Some(path) = &expr.path {
        return resolve_path(path, input);
    }

    if let Some(function) = &expr.function {
        let args: Option<Vec<serde_json::Value>> =
            expr.args.iter().map(|arg| evaluate_typed_expression(arg, input)).collect();
        let args = args?;
        let value = match function.as_str() {
            "len" | "count" => {
                let first = args.first()?;
                let count = match first {
                    serde_json::Value::Array(values) => values.len(),
                    serde_json::Value::Object(map) => map.len(),
                    serde_json::Value::String(text) => text.chars().count(),
                    _ => return None,
                };
                serde_json::json!(count)
            }
            _ => return None,
        };
        return Some(value);
    }

    if let Some(op) = expr.op.as_deref() {
        let left = expr.left.as_deref().and_then(|value| evaluate_typed_expression(value, input))?;
        let right = expr.right.as_deref().and_then(|value| evaluate_typed_expression(value, input));
        let result = match op {
            "gt" => numeric_value(&left, right.as_ref(), |l, r| l > r),
            "gte" => numeric_value(&left, right.as_ref(), |l, r| l >= r),
            "lt" => numeric_value(&left, right.as_ref(), |l, r| l < r),
            "lte" => numeric_value(&left, right.as_ref(), |l, r| l <= r),
            "eq" => compare_values(&left, right.as_ref(), |l, r| l == r),
            "neq" => compare_values(&left, right.as_ref(), |l, r| l != r),
            "and" => bool_value(&left).zip(right.as_ref().and_then(bool_value)).map(|(l, r)| l && r),
            "or" => bool_value(&left).zip(right.as_ref().and_then(bool_value)).map(|(l, r)| l || r),
            "not" => bool_value(&left).map(|value| !value),
            _ => None,
        }?;
        return Some(serde_json::Value::Bool(result));
    }

    None
}

fn bool_value(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Number(n) => n.as_f64().map(|n| n != 0.0),
        serde_json::Value::String(s) => Some(!s.is_empty()),
        serde_json::Value::Array(values) => Some(!values.is_empty()),
        serde_json::Value::Object(values) => Some(!values.is_empty()),
        serde_json::Value::Null => Some(false),
    }
}

fn numeric_value(
    left: &serde_json::Value,
    right: Option<&serde_json::Value>,
    cmp: impl Fn(f64, f64) -> bool,
) -> Option<bool> {
    let l = left.as_f64()?;
    let r = right?.as_f64()?;
    Some(cmp(l, r))
}

fn compare_values(
    left: &serde_json::Value,
    right: Option<&serde_json::Value>,
    cmp: impl Fn(&serde_json::Value, &serde_json::Value) -> bool,
) -> Option<bool> {
    Some(cmp(left, right?))
}

fn evaluate_structured_condition(cond: &StructuredCondition, input: &StepInput) -> bool {
    let resolved = resolve_path(&cond.left, input);

    // Warn on resolution failure for operators that expect a value.
    // Exists/NotExists intentionally test for absence, so no warning for those.
    if resolved.is_none()
        && !matches!(
            cond.operator,
            ConditionOp::Exists | ConditionOp::NotExists | ConditionOp::Empty | ConditionOp::IsFalsy
        )
    {
        tracing::warn!(
            path = %cond.left,
            operator = ?cond.operator,
            "Condition path failed to resolve — defaulting to true (fail-open, step will run)"
        );
    }

    let result = match &cond.operator {
        ConditionOp::Exists => {
            matches!(&resolved, Some(v) if !v.is_null())
        }
        ConditionOp::NotExists => {
            matches!(&resolved, None | Some(serde_json::Value::Null))
        }
        ConditionOp::Equals => match (&resolved, &cond.right) {
            (Some(left), Some(right)) => left == right,
            (None | Some(serde_json::Value::Null), None) => true,
            _ => false,
        },
        ConditionOp::NotEquals => match (&resolved, &cond.right) {
            (Some(left), Some(right)) => left != right,
            (None | Some(serde_json::Value::Null), None) => false,
            _ => true,
        },
        ConditionOp::GreaterThan => numeric_cmp(&resolved, &cond.right, |l, r| l > r),
        ConditionOp::LessThan => numeric_cmp(&resolved, &cond.right, |l, r| l < r),
        ConditionOp::GreaterThanEquals => numeric_cmp(&resolved, &cond.right, |l, r| l >= r),
        ConditionOp::LessThanEquals => numeric_cmp(&resolved, &cond.right, |l, r| l <= r),
        ConditionOp::NotEmpty => {
            match &resolved {
                None | Some(serde_json::Value::Null) => false,
                Some(serde_json::Value::String(s)) => !s.is_empty(),
                Some(serde_json::Value::Array(a)) => !a.is_empty(),
                Some(serde_json::Value::Object(m)) => !m.is_empty(),
                _ => true, // numbers, bools are "not empty"
            }
        }
        ConditionOp::Empty => match &resolved {
            None | Some(serde_json::Value::Null) => true,
            Some(serde_json::Value::String(s)) => s.is_empty(),
            Some(serde_json::Value::Array(a)) => a.is_empty(),
            Some(serde_json::Value::Object(m)) => m.is_empty(),
            _ => false,
        },
        ConditionOp::IsTruthy => match &resolved {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Some(serde_json::Value::String(s)) => !s.is_empty(),
            Some(serde_json::Value::Array(a)) => !a.is_empty(),
            Some(serde_json::Value::Object(_)) => true,
        },
        ConditionOp::IsFalsy => match &resolved {
            None | Some(serde_json::Value::Null) => true,
            Some(serde_json::Value::Bool(b)) => !*b,
            Some(serde_json::Value::Number(n)) => n.as_f64().map(|f| f == 0.0).unwrap_or(true),
            Some(serde_json::Value::String(s)) => s.is_empty(),
            Some(serde_json::Value::Array(a)) => a.is_empty(),
            Some(serde_json::Value::Object(_)) => false,
        },
        ConditionOp::Contains => match (&resolved, &cond.right) {
            (Some(serde_json::Value::String(haystack)), Some(serde_json::Value::String(needle))) => {
                haystack.contains(needle.as_str())
            }
            (Some(serde_json::Value::Array(arr)), Some(needle)) => arr.contains(needle),
            _ => false,
        },
    };

    tracing::debug!(
        path = %cond.left,
        operator = ?cond.operator,
        resolved_value = ?resolved,
        result,
        "Condition evaluated"
    );

    result
}

/// Helper for numeric comparisons. Returns `false` (fail-open: run the step)
/// if either side can't be parsed as a number.
fn numeric_cmp(
    resolved: &Option<serde_json::Value>,
    right: &Option<serde_json::Value>,
    cmp: impl Fn(f64, f64) -> bool,
) -> bool {
    let left_num = resolved.as_ref().and_then(|v| v.as_f64());
    let right_num = right.as_ref().and_then(|v| v.as_f64());
    match (left_num, right_num) {
        (Some(l), Some(r)) => cmp(l, r),
        _ => true, // fail-open: can't compare → run the step
    }
}

fn validate_step_schema(
    step: &StepNode,
    label: &str,
    value: &serde_json::Value,
    schema: Option<&serde_json::Value>,
) -> Result<(), anyhow::Error> {
    let Some(schema) = schema else {
        return Ok(());
    };

    let result = validate_output_against_schema(&step.id, value, schema);
    match result {
        Ok(()) => Ok(()),
        Err(err) => match step.schema_mode {
            crate::agent::definition::SchemaMode::Off => Ok(()),
            crate::agent::definition::SchemaMode::Warn => {
                tracing::warn!(step_id = %step.id, label, error = %err, "schema validation warning");
                Ok(())
            }
            crate::agent::definition::SchemaMode::Strict => {
                Err(anyhow::anyhow!("{} schema validation failed for {}: {}", label, step.id, err))
            }
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DAG ENGINE
// ═══════════════════════════════════════════════════════════════════════════

pub struct DagEngine {
    executor: Arc<dyn Executor>,
    workflow_store: Arc<dyn WorkflowStore>,
    event_bus: Arc<EventBus>,
    /// When set, provides full per-step lifecycle hooks (knowledge injection,
    /// delegation detection, clarification, failure rules, citations).
    /// When None, falls back to direct executor.execute_step() calls.
    orchestrator: Option<Arc<crate::agent::orchestrator::StepOrchestrator>>,
}

impl DagEngine {
    pub fn new(executor: Arc<dyn Executor>, workflow_store: Arc<dyn WorkflowStore>, event_bus: Arc<EventBus>) -> Self {
        Self { executor, workflow_store, event_bus, orchestrator: None }
    }

    /// Set the orchestrator for full per-step lifecycle hooks.
    pub fn with_orchestrator(mut self, orchestrator: Arc<crate::agent::orchestrator::StepOrchestrator>) -> Self {
        self.orchestrator = Some(orchestrator);
        self
    }

    // ── SCHEDULER LOOP ─────────────────────────────────────────────────

    /// Run the workflow to completion.
    ///
    /// Repeatedly calls `run_cycle` until the workflow finishes, is cancelled,
    /// or deadlocks. Sleeps between cycles to respect retry timing.
    pub async fn run_workflow(&self, config: &AgentState, cancel: CancellationToken) -> Result<WorkflowOutcome> {
        tracing::info!(agent_id = config.id, "DAG engine: starting workflow execution");

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!(agent_id = config.id, "DAG engine: cancelled");
                    // Mark workflow as cancelled in DB
                    if let Ok(Some(wf)) = self.workflow_store.resume_workflow(&config.id).await {
                        let _ = self.workflow_store.update_workflow_status(
                            &wf.id, WorkflowStatus::Cancelled
                        ).await;
                    }
                    return Ok(WorkflowOutcome::Cancelled);
                }
                outcome = self.run_cycle(config) => {
                    match outcome? {
                        CycleOutcome::Continue => continue,
                        CycleOutcome::WaitUntil(instant) => {
                            let delay = (instant - Utc::now())
                                .to_std()
                                .unwrap_or(std::time::Duration::from_secs(1));
                            tracing::debug!(
                                agent_id = config.id,
                                delay_secs = delay.as_secs(),
                                "DAG engine: sleeping until next retry"
                            );
                            tokio::select! {
                                _ = cancel.cancelled() => {
                                    return Ok(WorkflowOutcome::Cancelled);
                                }
                                _ = tokio::time::sleep(delay) => continue,
                            }
                        }
                        CycleOutcome::Complete => {
                            tracing::info!(agent_id = config.id, "DAG engine: workflow complete");
                            if let Ok(Some(wf)) = self.workflow_store.resume_workflow(&config.id).await {
                                let _ = self.workflow_store.update_workflow_status(
                                    &wf.id, WorkflowStatus::Completed
                                ).await;
                            }
                            return Ok(WorkflowOutcome::Completed);
                        }
                        CycleOutcome::Failed(reason) => {
                            tracing::error!(
                                agent_id = config.id,
                                reason = reason,
                                "DAG engine: workflow failed"
                            );
                            if let Ok(Some(wf)) = self.workflow_store.resume_workflow(&config.id).await {
                                let _ = self.workflow_store.update_workflow_status(
                                    &wf.id, WorkflowStatus::Failed
                                ).await;
                            }
                            return Ok(WorkflowOutcome::Failed(reason));
                        }
                        CycleOutcome::Deadlocked(blocked_ids) => {
                            let reason = format!(
                                "DAG deadlocked. Blocked by failed steps: {:?}",
                                blocked_ids
                            );
                            tracing::error!(agent_id = config.id, reason = reason, "deadlock");
                            if let Ok(Some(wf)) = self.workflow_store.resume_workflow(&config.id).await {
                                let _ = self.workflow_store.update_workflow_status(
                                    &wf.id, WorkflowStatus::Failed
                                ).await;
                            }
                            return Ok(WorkflowOutcome::Failed(reason));
                        }
                    }
                }
            }
        }
    }

    // ── SINGLE CYCLE ───────────────────────────────────────────────────

    /// Execute one scheduling cycle:
    /// 1. Load workflow from DB (single source of truth)
    /// 2. Detect deadlock
    /// 3. Find ready steps (deps satisfied + retry time elapsed)
    /// 4. Execute ready steps in parallel (each is isolated)
    /// 5. Persist results to DB
    /// 6. Return cycle outcome
    async fn run_cycle(&self, config: &AgentState) -> Result<CycleOutcome> {
        // 1. Load fresh workflow state from DB
        let mut workflow = match self.workflow_store.resume_workflow(&config.id).await? {
            Some(wf) => wf,
            None => return Ok(CycleOutcome::Failed(format!("no active workflow for agent {}", config.id))),
        };

        // 1.5 Expand any ForEach templates that are ready
        if let Some((new_steps, updated_deps)) = workflow.expand_foreach_nodes() {
            tracing::info!(
                agent_id = config.id,
                new_nodes_count = new_steps.len(),
                "DAG engine: expanded ForEach template"
            );
            self.workflow_store.save_expanded_nodes(&workflow.id, &new_steps, &updated_deps).await?;
        }

        // 2. Check completion
        if workflow.is_complete() {
            return Ok(CycleOutcome::Complete);
        }

        // 3. Deadlock detection
        if let Some(blocked) = workflow.detect_deadlock() {
            return Ok(CycleOutcome::Deadlocked(blocked));
        }

        // 4. Find ready steps (respects retry timing)
        let ready: Vec<StepNode> = workflow.ready_steps().into_iter().cloned().collect();

        if ready.is_empty() {
            // Not deadlocked, but no steps ready yet → waiting for retries
            if let Some(next_retry) = workflow.next_retry_time() {
                return Ok(CycleOutcome::WaitUntil(next_retry));
            }
            // Steps still running (possibly from another cycle)
            return Ok(CycleOutcome::WaitUntil(Utc::now() + Duration::seconds(2)));
        }

        tracing::info!(
            agent_id = config.id,
            ready_count = ready.len(),
            step_ids = ?ready.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            "DAG engine: executing ready steps"
        );

        // 5. Execute ready steps in parallel — EACH STEP IS ISOLATED
        let results = self.execute_parallel(config, &ready).await;

        // 6. Persist step results + transition state machine
        for (step_id, outcome) in results {
            match outcome {
                DagStepOutcome::Skipped => {
                    // Condition gate skip — DB already persisted in execute_parallel
                    let _ = workflow.mark_skipped(&step_id);
                }
                DagStepOutcome::Executed(step_result) => {
                    mark_step_running_in_memory(&mut workflow, &step_id);
                    if step_result.success {
                        let output_value = if step_result.tool_results.is_empty() {
                            serde_json::json!({
                                "output": step_result.output,
                                "final_answer_candidate": step_result.final_answer_candidate,
                            })
                        } else {
                            serde_json::json!({
                                "output": step_result.output,
                                "final_answer_candidate": step_result.final_answer_candidate,
                                "tool_results": step_result.tool_results,
                            })
                        };

                        let step = workflow.get_step(&step_id).unwrap().clone();
                        if let Err(error) =
                            validate_step_schema(&step, "output", &output_value, step.output_schema.as_ref())
                        {
                            let error_msg = error.to_string();
                            tracing::warn!(step_id = %step_id, error = %error_msg, "output schema validation failed");
                            mark_step_running_in_memory(&mut workflow, &step_id);
                            match workflow.handle_failure(&step_id, &error_msg) {
                                Ok(new_status) => {
                                    let step = workflow.get_step(&step_id).unwrap();
                                    let _ = self
                                        .workflow_store
                                        .update_step_status(&step_id, &new_status, step.attempt, None, Some(&error_msg))
                                        .await;
                                }
                                Err(e) => {
                                    tracing::warn!(step_id = %step_id, error = %e, "state transition error");
                                }
                            }
                        } else {
                            let _ = write_step_artifact(
                                Path::new(&config.workspace_path),
                                &config.id,
                                step.index,
                                &output_value,
                            )
                            .await;

                            mark_step_running_in_memory(&mut workflow, &step_id);
                            if let Err(e) = workflow.mark_succeeded(&step_id, output_value.clone()) {
                                tracing::warn!(step_id, error = %e, "state transition error");
                            }
                            let _ = self
                                .workflow_store
                                .update_step_status(
                                    &step_id,
                                    &StepStatus::Succeeded,
                                    step.attempt,
                                    Some(&output_value),
                                    None,
                                )
                                .await;
                        }
                    } else {
                        let error_msg = step_result.output.clone();
                        mark_step_running_in_memory(&mut workflow, &step_id);
                        match workflow.handle_failure(&step_id, &error_msg) {
                            Ok(new_status) => {
                                let step = workflow.get_step(&step_id).unwrap();
                                let _ = self
                                    .workflow_store
                                    .update_step_status(&step_id, &new_status, step.attempt, None, Some(&error_msg))
                                    .await;
                            }
                            Err(e) => {
                                tracing::warn!(step_id, error = %e, "state transition error");
                            }
                        }
                    }
                }
                DagStepOutcome::AwaitingChildren { child_ids, .. } => {
                    // Step delegated — DB already updated in execute_parallel.
                    // Sync the in-memory workflow state.
                    if let Some(node) = workflow.nodes.iter_mut().find(|n| n.id == step_id) {
                        node.status = StepStatus::AwaitingChildren { child_ids };
                    }
                    tracing::info!(step_id = %step_id, "DAG cycle: step is awaiting child agents");
                }
                DagStepOutcome::AwaitingInput { questions, .. } => {
                    // Step needs human input — DB already updated in execute_parallel.
                    // Sync the in-memory workflow state.
                    if let Some(node) = workflow.nodes.iter_mut().find(|n| n.id == step_id) {
                        node.status = StepStatus::AwaitingInput { questions: questions.clone() };
                    }
                    self.event_bus.publish(AgentEvent::ClarificationNeeded {
                        agent_id: config.id.clone(),
                        questions: questions
                            .iter()
                            .map(|q| crate::agent::clarifier::ClarificationQuestion::new(q.clone()))
                            .collect(),
                    });
                    tracing::info!(step_id = %step_id, "DAG cycle: step is awaiting user input");
                }
                DagStepOutcome::DeterministicAbort { reason, .. } => {
                    // FailureRule matched — treat as terminal failure
                    mark_step_running_in_memory(&mut workflow, &step_id);
                    match workflow.handle_failure(&step_id, &reason) {
                        Ok(new_status) => {
                            let step = workflow.get_step(&step_id).unwrap();
                            let _ = self
                                .workflow_store
                                .update_step_status(&step_id, &new_status, step.attempt, None, Some(&reason))
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(step_id = %step_id, error = %e, "state transition error");
                        }
                    }
                }
                DagStepOutcome::Error(exec_error) => {
                    let error_msg = format!("{:#}", exec_error);
                    tracing::error!(step_id, error = error_msg, "step execution error");
                    mark_step_running_in_memory(&mut workflow, &step_id);
                    match workflow.handle_failure(&step_id, &error_msg) {
                        Ok(new_status) => {
                            let step = workflow.get_step(&step_id).unwrap();
                            let _ = self
                                .workflow_store
                                .update_step_status(&step_id, &new_status, step.attempt, None, Some(&error_msg))
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(step_id, error = %e, "state transition error");
                        }
                    }
                }
            }
        }

        // Emit progress event
        self.event_bus.publish(AgentEvent::StepCompleted {
            agent_id: config.id.clone(),
            step_index: 0, // DAG doesn't have a single "current step"
            summary: "DAG cycle completed".to_string(),
            description: None,
            success: true,
        });

        Ok(CycleOutcome::Continue)
    }

    // ── PARALLEL EXECUTOR ──────────────────────────────────────────────

    /// Execute steps in parallel via `tokio::spawn`.
    /// Each step is isolated: reads from DB, produces output, never touches AgentState.
    ///
    /// When the orchestrator is set, each step gets full lifecycle hooks
    /// (knowledge injection, delegation detection, clarification, failure rules).
    async fn execute_parallel(&self, config: &AgentState, steps: &[StepNode]) -> Vec<(String, DagStepOutcome)> {
        let mut handles = Vec::new();

        for step in steps {
            let executor = Arc::clone(&self.executor);
            let store = Arc::clone(&self.workflow_store);
            let orchestrator = self.orchestrator.clone();
            let step_owned = step.clone();
            let config_snapshot = config.clone();

            let handle = tokio::spawn(async move {
                // Build step input from predecessor outputs in DB
                let step_input = match StepInput::from_predecessors(store.as_ref(), &step_owned).await {
                    Ok(input) => input,
                    Err(error) => return (step_owned.id.clone(), DagStepOutcome::Error(error)),
                };

                // ── Condition gate ──────────────────────────────────────
                if let Some(ref condition) = step_owned.condition {
                    if !evaluate_condition(condition, &step_input) {
                        tracing::info!(
                            step_id = %step_owned.id,
                            condition = ?condition,
                            "DAG engine: condition evaluated false, skipping step"
                        );
                        let _ = store
                            .update_step_status(&step_owned.id, &StepStatus::Skipped, step_owned.attempt, None, None)
                            .await;
                        return (step_owned.id.clone(), DagStepOutcome::Skipped);
                    }
                }

                // Mark step as Running in DB BEFORE execution
                let _ = store
                    .update_step_status(&step_owned.id, &StepStatus::Running, step_owned.attempt, None, None)
                    .await;

                if let Err(error) =
                    validate_step_schema(&step_owned, "input", &step_input.data, step_owned.input_schema.as_ref())
                {
                    return (step_owned.id.clone(), DagStepOutcome::Error(error));
                }

                let mut config_snapshot = config_snapshot;
                config_snapshot.metadata["dag_step_context"] = build_dag_step_metadata(&step_owned, &step_input);

                // Inject predecessor outputs for template resolution
                for (dep_key, dep_value) in &step_input.predecessor_outputs {
                    let meta_key = format!("{}_output", dep_key.replace('-', "_"));
                    config_snapshot.metadata[&meta_key] = dep_value.clone();
                }

                let planned_step = step_owned.to_planned_step();
                let synthetic_plan = Plan {
                    goal: config_snapshot.goal.clone(),
                    job_type: None,
                    steps: vec![planned_step.clone()],
                    rationale: String::new(),
                };
                let mut history = StepHistory::new();

                // ── Execute via orchestrator (full hooks) or executor (direct) ──
                if let Some(ref orch) = orchestrator {
                    let verdict =
                        orch.run_step(&mut config_snapshot, &planned_step, &synthetic_plan, &mut history).await;

                    match verdict {
                        crate::agent::orchestrator::StepVerdict::Executed { result, .. } => {
                            (step_owned.id.clone(), DagStepOutcome::Executed(result))
                        }
                        crate::agent::orchestrator::StepVerdict::Skipped { .. } => {
                            (step_owned.id.clone(), DagStepOutcome::Skipped)
                        }
                        crate::agent::orchestrator::StepVerdict::Delegating { result, child_ids } => {
                            tracing::info!(
                                step_id = %step_owned.id,
                                child_count = child_ids.len(),
                                "DAG engine: step delegated to child agents"
                            );
                            let _ = store
                                .update_step_status(
                                    &step_owned.id,
                                    &StepStatus::AwaitingChildren { child_ids: child_ids.clone() },
                                    step_owned.attempt,
                                    None,
                                    None,
                                )
                                .await;
                            (step_owned.id.clone(), DagStepOutcome::AwaitingChildren { result, child_ids })
                        }
                        crate::agent::orchestrator::StepVerdict::NeedsClarification { result, questions } => {
                            let question_strings: Vec<String> = questions.into_iter().map(|q| q.prompt).collect();
                            tracing::info!(
                                step_id = %step_owned.id,
                                question_count = question_strings.len(),
                                "DAG engine: step awaiting user input"
                            );
                            let _ = store
                                .update_step_status(
                                    &step_owned.id,
                                    &StepStatus::AwaitingInput { questions: question_strings.clone() },
                                    step_owned.attempt,
                                    None,
                                    None,
                                )
                                .await;
                            (
                                step_owned.id.clone(),
                                DagStepOutcome::AwaitingInput { result, questions: question_strings },
                            )
                        }
                        crate::agent::orchestrator::StepVerdict::DeterministicAbort { result, reason } => {
                            tracing::warn!(
                                step_id = %step_owned.id,
                                reason = %reason,
                                "DAG engine: step aborted by FailureRule"
                            );
                            (step_owned.id.clone(), DagStepOutcome::DeterministicAbort { result, reason })
                        }
                        crate::agent::orchestrator::StepVerdict::Error { error } => {
                            (step_owned.id.clone(), DagStepOutcome::Error(error))
                        }
                    }
                } else {
                    // Fallback: direct executor call (no orchestrator hooks)
                    let result =
                        executor.execute_step(&config_snapshot, &planned_step, &synthetic_plan, &history).await;

                    match result {
                        Ok(r) => (step_owned.id.clone(), DagStepOutcome::Executed(r)),
                        Err(e) => (step_owned.id.clone(), DagStepOutcome::Error(e)),
                    }
                }
            });

            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((id, outcome)) => results.push((id, outcome)),
                Err(join_error) => {
                    tracing::error!(error = %join_error, "step task panicked");
                    results.push((
                        "unknown".to_string(),
                        DagStepOutcome::Error(anyhow::anyhow!("step task panicked: {}", join_error)),
                    ));
                }
            }
        }

        results
    }
}

/// Internal outcome type for DAG step execution.
/// Richer than raw StepResult — captures delegation, clarification, and abort.
pub enum DagStepOutcome {
    /// Step executed (success or failure — check result.success).
    Executed(crate::agent::executor::StepResult),
    /// Step was skipped by condition gate.
    Skipped,
    /// Step delegated to child agents — awaiting their completion.
    AwaitingChildren { result: crate::agent::executor::StepResult, child_ids: Vec<String> },
    /// Step needs user input — awaiting response.
    AwaitingInput { result: crate::agent::executor::StepResult, questions: Vec<String> },
    /// Step aborted by deterministic FailureRule match.
    DeterministicAbort { result: crate::agent::executor::StepResult, reason: String },
    /// Infrastructure error during execution.
    Error(anyhow::Error),
}
