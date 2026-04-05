//! DAG-based workflow execution primitives.
//!
//! This module defines the core types and algorithms for the durable DAG
//! execution engine. It replaces the linear `Vec<PlannedStep>` with a true
//! directed acyclic graph that supports:
//!
//! - Fan-out (parallel execution of independent steps)
//! - Fan-in (joining parallel branches)
//! - Step-level state machine (Pending → Ready → Running → Succeeded/Failed/Retried)
//! - Engine-managed retries with exponential backoff
//! - Deadlock detection
//! - Conditional step skipping
//!
//! The `Workflow` struct is the top-level container. It owns a list of `StepNode`s
//! and provides methods to query ready steps, detect deadlocks, and transition
//! step states.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::{
    definition::{RetryPolicy, SchemaMode},
    planner::{Plan, PlannedStep, StepCondition},
};

// ═══════════════════════════════════════════════════════════════════════════
// STEP STATUS — State Machine
// ═══════════════════════════════════════════════════════════════════════════

/// Step-level status — the state machine for a single execution node.
///
/// Legal transitions:
/// ```text
///   Pending → Ready → Running → Succeeded
///                              → Failed
///                              → Retrying → Running (retry loop)
///   Pending → Skipped (condition evaluated to false)
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StepStatus {
    /// Waiting for dependency steps to complete.
    Pending,
    /// All dependencies satisfied — queued for execution.
    Ready,
    /// Currently executing (owned by a worker).
    Running,
    /// Completed successfully — output is available.
    Succeeded,
    /// Terminal failure — retries exhausted or non-retryable error.
    Failed,
    /// Skipped — step condition evaluated to false.
    Skipped,
    /// Failed but will be retried. The scheduler loop sleeps until
    /// `next_retry_at` before marking this step as ready again.
    Retrying {
        attempt: u32,
        #[serde(with = "chrono::serde::ts_seconds")]
        next_retry_at: DateTime<Utc>,
    },
    /// Waiting for human input (e.g. clarification question).
    /// Sibling steps continue executing; this step resumes when
    /// the user responds. Contains the questions being asked.
    AwaitingInput { questions: Vec<String> },
    /// Waiting for child agent(s) to complete a delegated sub-goal.
    /// Sibling steps continue executing; this step resumes when
    /// all children finish.
    AwaitingChildren { child_ids: Vec<String> },
}

impl StepStatus {
    /// Returns true for terminal states (Succeeded, Failed, Skipped).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Skipped)
    }

    /// Returns true for states that indicate the step can eventually proceed.
    pub fn is_actionable(&self) -> bool {
        matches!(
            self,
            Self::Pending
                | Self::Ready
                | Self::Running
                | Self::Retrying { .. }
                | Self::AwaitingInput { .. }
                | Self::AwaitingChildren { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepKind {
    Normal,
    ForEachTemplate,
    ForEachItem { parent: String, index: usize },
    ForEachJoin { parent: String },
}

impl Default for StepKind {
    fn default() -> Self {
        Self::Normal
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STEP NODE — A single node in the execution DAG
// ═══════════════════════════════════════════════════════════════════════════

/// A node in the execution DAG.
///
/// Each `StepNode` represents a single unit of work with:
/// - Dependency edges (`depends_on`) that define the DAG topology
/// - A state machine (`status`) tracking its lifecycle
/// - Engine-managed retry policy
/// - Optional input/output JSON schemas for data validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepNode {
    /// Unique step identifier (e.g., "step-0", "step-enrich-lead").
    pub id: String,
    /// Position index for ordering and display.
    pub index: usize,
    /// Human-readable description of what this step does.
    pub description: String,
    /// Tool to invoke (None = pure LLM reasoning step).
    pub tool: Option<String>,
    /// Arguments for the tool, potentially with template references.
    pub tool_args: Option<serde_json::Value>,
    /// Human-readable success criteria.
    pub success_criteria: String,
    /// Optional condition — step is skipped if condition evaluates to false.
    pub condition: Option<StepCondition>,
    /// Iteration template expression (e.g. `$.deps.step-1.outputs`).
    pub foreach: Option<String>,
    /// The structural kind of this step.
    #[serde(default)]
    pub kind: StepKind,

    // ── DAG edges ──────────────────────────────────────────────────────
    /// Step IDs this node depends on. All must be Succeeded or Skipped
    /// before this node becomes Ready.
    pub depends_on: Vec<String>,

    // ── State machine ──────────────────────────────────────────────────
    /// Current lifecycle status.
    pub status: StepStatus,
    /// Number of execution attempts so far (0 = never started).
    pub attempt: u32,
    /// Engine-managed retry policy.
    pub retry_policy: RetryPolicy,

    // ── Schema enforcement ─────────────────────────────────────────────
    /// Validation mode for this step (default: Strict).
    pub schema_mode: SchemaMode,
    /// JSON Schema for expected input from predecessor steps.
    pub input_schema: Option<serde_json::Value>,
    /// JSON Schema for the output this step must produce.
    pub output_schema: Option<serde_json::Value>,

    // ── Output ─────────────────────────────────────────────────────────
    /// Actual output data after successful execution.
    pub output_data: Option<serde_json::Value>,

    // ── Timing ─────────────────────────────────────────────────────────
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message if the step failed.
    pub error: Option<String>,
}

impl StepNode {
    /// Convert this DAG node back to a `PlannedStep` for the existing
    /// executor interface. Bridges the old and new architectures.
    pub fn to_planned_step(&self) -> PlannedStep {
        PlannedStep {
            foreach: self.foreach.clone(),
            index: self.index as usize,
            description: self.description.clone(),
            tool: self.tool.clone(),
            tool_args: self.tool_args.clone(),
            success_criteria: self.success_criteria.clone(),
            condition: self.condition.clone(),
            depends_on: self
                .depends_on
                .iter()
                .filter_map(|id| {
                    // Convert "step-N" back to N
                    id.strip_prefix("step-").and_then(|n| n.parse().ok())
                })
                .collect(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// WORKFLOW — The complete execution DAG
// ═══════════════════════════════════════════════════════════════════════════

/// The complete execution DAG for an agent run.
///
/// Owns a list of `StepNode`s and provides methods for:
/// - Querying ready steps (dependencies satisfied + retry timing)
/// - Transitioning step states
/// - Deadlock detection
/// - Completion checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique workflow execution ID.
    pub id: String,
    /// Agent that owns this workflow.
    pub agent_id: String,
    /// Tenant for multi-tenancy.
    pub tenant_id: String,
    /// The goal being achieved.
    pub goal: String,
    /// DAG nodes — the execution plan.
    pub nodes: Vec<StepNode>,
    /// Overall workflow status.
    pub status: WorkflowStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl Workflow {
    // ── Ready steps query ──────────────────────────────────────────────

    /// Resolves a simple expression like `deps.step-1.outputs` against previous step outputs.
    fn resolve_collection(&self, expression: &str) -> Vec<serde_json::Value> {
        let parts: Vec<&str> = expression.split('.').collect();
        if parts.len() >= 3 && parts[0] == "deps" {
            let dep_id = parts[1];
            let key = parts[2];
            if let Some(dep_step) = self.get_step(dep_id) {
                if let Some(ref output) = dep_step.output_data {
                    if let Some(arr) = output.get(key).and_then(|v| v.as_array()) {
                        return arr.clone();
                    }
                }
            }
        }
        if parts.len() == 2 && parts[0] == "deps" {
            let dep_id = parts[1];
            if let Some(dep_step) = self.get_step(dep_id) {
                if let Some(ref output) = dep_step.output_data {
                    if let Some(arr) = output.as_array() {
                        return arr.clone();
                    }
                }
            }
        }
        vec![]
    }

    /// Recursively render `{item}` placeholders in a JSON value.
    fn render_item_template(template: &serde_json::Value, item_data: &serde_json::Value) -> serde_json::Value {
        match template {
            serde_json::Value::String(s) => {
                let mut result = s.clone();
                if result.contains("{item}") {
                    let replacement = match item_data {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    result = result.replace("{item}", &replacement);
                }
                while let Some(start) = result.find("{item.") {
                    let rest = &result[start + 6..];
                    if let Some(end) = rest.find('}') {
                        let key = &rest[..end];
                        let replacement = item_data
                            .get(key)
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .unwrap_or_else(|| format!("{{item.{}}}", key));
                        result = format!("{}{}{}", &result[..start], replacement, &rest[end + 1..]);
                    } else {
                        break;
                    }
                }
                serde_json::Value::String(result)
            }
            serde_json::Value::Object(map) => {
                let rendered: serde_json::Map<String, serde_json::Value> =
                    map.iter().map(|(k, v)| (k.clone(), Self::render_item_template(v, item_data))).collect();
                serde_json::Value::Object(rendered)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| Self::render_item_template(v, item_data)).collect())
            }
            other => other.clone(),
        }
    }

    /// Evaluates `Pending` `ForEachTemplate` steps. If dependencies are satisfied,
    /// it expands them into `ForEachItem` steps and a `ForEachJoin` step.
    /// Returns `Some((new_steps, updated_dependencies))` if any nodes were expanded.
    pub fn expand_foreach_nodes(&mut self) -> Option<(Vec<StepNode>, Vec<(String, Vec<String>)>)> {
        let mut to_expand = Vec::new();

        for node in &self.nodes {
            if matches!(node.kind, StepKind::ForEachTemplate) && matches!(node.status, StepStatus::Pending) {
                if self.deps_satisfied(node) {
                    to_expand.push((node.id.clone(), node.foreach.clone()));
                }
            }
        }

        if to_expand.is_empty() {
            return None;
        }

        let mut newly_added_nodes = Vec::new();
        let mut updated_dependencies = Vec::new();

        for (template_id, foreach_expr) in to_expand {
            let join_id = format!("{}_join", template_id);
            let items = if let Some(expr) = foreach_expr { self.resolve_collection(&expr) } else { vec![] };

            let template_node = self.nodes.iter().find(|n| n.id == template_id).unwrap().clone();
            let mut child_ids = Vec::new();

            for (idx, item) in items.into_iter().enumerate() {
                let child_id = format!("{}[{}]", template_id, idx);
                child_ids.push(child_id.clone());

                let mut child = template_node.clone();
                child.id = child_id;
                child.kind = StepKind::ForEachItem { parent: template_id.clone(), index: idx };
                child.status = StepStatus::Pending;

                if let Some(args) = child.tool_args.as_mut() {
                    *args = Self::render_item_template(args, &item);
                }
                newly_added_nodes.push(child);
            }

            // Join node
            let mut join_node = template_node.clone();
            join_node.id = join_id.clone();
            join_node.kind = StepKind::ForEachJoin { parent: template_id.clone() };
            join_node.depends_on = child_ids; // It depends on all items
            join_node.status = StepStatus::Pending;
            join_node.description = format!("Join results for {}", template_id);
            join_node.tool = None; // A pure synchronization node
            newly_added_nodes.push(join_node);

            // Change template node status to Succeeded so it doesn't block anymore,
            // but we don't care about its output data since downstream depends on join node now.
            if let Some(template_mut) = self.nodes.iter_mut().find(|n| n.id == template_id) {
                template_mut.status = StepStatus::Succeeded; // Expanded successfully
                template_mut.completed_at = Some(Utc::now());
            }

            // Rewrite downstream dependencies
            for node in &mut self.nodes {
                let mut changed = false;
                for dep in &mut node.depends_on {
                    if *dep == template_id {
                        *dep = join_id.clone();
                        changed = true;
                    }
                }
                if changed {
                    updated_dependencies.push((node.id.clone(), node.depends_on.clone()));
                }
            }
        }

        let returned_nodes = newly_added_nodes.clone();
        self.nodes.extend(newly_added_nodes);
        Some((returned_nodes, updated_dependencies))
    }

    /// Returns step nodes whose dependencies are all satisfied and whose
    /// status allows execution.
    ///
    /// A step is "ready" when:
    /// 1. Status is `Pending` AND all dependencies are `Succeeded` or `Skipped`
    /// 2. Status is `Retrying` AND `now >= next_retry_at`
    ///
    /// This respects retry timing — a `Retrying` step is NOT ready until
    /// its backoff period has elapsed.
    pub fn ready_steps(&self) -> Vec<&StepNode> {
        let now = Utc::now();
        self.nodes
            .iter()
            .filter(|node| match &node.status {
                StepStatus::Pending => self.deps_satisfied(node),
                StepStatus::Retrying { next_retry_at, .. } => now >= *next_retry_at,
                _ => false,
            })
            .collect()
    }

    /// Check if all of a node's dependencies are in terminal-success states.
    fn deps_satisfied(&self, node: &StepNode) -> bool {
        node.depends_on.iter().all(|dep_id| {
            self.nodes
                .iter()
                .any(|n| &n.id == dep_id && matches!(n.status, StepStatus::Succeeded | StepStatus::Skipped))
        })
    }

    // ── Completion checks ──────────────────────────────────────────────

    /// Returns true when ALL steps are in terminal states.
    pub fn is_complete(&self) -> bool {
        self.nodes.iter().all(|n| n.status.is_terminal())
    }

    /// Returns true when any step has terminally failed.
    pub fn has_failures(&self) -> bool {
        self.nodes.iter().any(|n| matches!(n.status, StepStatus::Failed))
    }

    // ── Deadlock detection ─────────────────────────────────────────────

    /// Detect deadlock: no steps are actionable but workflow is not complete.
    ///
    /// This catches:
    /// - Cycles in the DAG (shouldn't happen but safety net)
    /// - All remaining steps blocked by failed dependencies
    ///
    /// Returns the IDs of blocking (failed) steps for diagnostics.
    pub fn detect_deadlock(&self) -> Option<Vec<String>> {
        if self.is_complete() {
            return None;
        }

        let has_active = self.nodes.iter().any(|n| {
            matches!(
                n.status,
                StepStatus::Running
                    | StepStatus::Retrying { .. }
                    | StepStatus::AwaitingInput { .. }
                    | StepStatus::AwaitingChildren { .. }
            )
        });

        if has_active {
            return None;
        }

        let has_ready = !self.ready_steps().is_empty();
        if has_ready {
            return None;
        }

        // Deadlock: workflow not complete, but no steps can make progress.
        // Return the failed steps that are blocking.
        Some(self.nodes.iter().filter(|n| matches!(n.status, StepStatus::Failed)).map(|n| n.id.clone()).collect())
    }

    // ── Retry timing ───────────────────────────────────────────────────

    /// Returns the earliest `next_retry_at` across all `Retrying` steps.
    /// The scheduler loop uses this to sleep until the next retry is due.
    pub fn next_retry_time(&self) -> Option<DateTime<Utc>> {
        self.nodes
            .iter()
            .filter_map(|n| {
                if let StepStatus::Retrying { next_retry_at, .. } = &n.status {
                    Some(*next_retry_at)
                } else {
                    None
                }
            })
            .min()
    }

    // ── State transitions ──────────────────────────────────────────────

    /// Transition a step to `Running`. Validates the current state.
    pub fn mark_running(&mut self, step_id: &str) -> Result<(), WorkflowError> {
        let step = self.get_step_mut(step_id)?;
        match &step.status {
            StepStatus::Pending | StepStatus::Ready | StepStatus::Retrying { .. } => {
                step.status = StepStatus::Running;
                step.started_at = Some(Utc::now());
                self.updated_at = Utc::now();
                Ok(())
            }
            other => Err(WorkflowError::InvalidTransition {
                step_id: step_id.to_string(),
                from: format!("{:?}", other),
                to: "Running".to_string(),
            }),
        }
    }

    /// Transition a step to `Succeeded` with output data.
    pub fn mark_succeeded(&mut self, step_id: &str, output: serde_json::Value) -> Result<(), WorkflowError> {
        let step = self.get_step_mut(step_id)?;
        match &step.status {
            StepStatus::Running => {
                step.status = StepStatus::Succeeded;
                step.output_data = Some(output);
                step.completed_at = Some(Utc::now());
                self.updated_at = Utc::now();
                Ok(())
            }
            other => Err(WorkflowError::InvalidTransition {
                step_id: step_id.to_string(),
                from: format!("{:?}", other),
                to: "Succeeded".to_string(),
            }),
        }
    }

    /// Handle a step failure: either retry (if policy allows) or fail terminally.
    pub fn handle_failure(&mut self, step_id: &str, error: &str) -> Result<StepStatus, WorkflowError> {
        let new_status = {
            let step = self.get_step_mut(step_id)?;
            match &step.status {
                StepStatus::Running => {
                    step.attempt += 1;
                    let retry_allowed = step.attempt < step.retry_policy.max_attempts
                        && step.retry_policy.matches_retry_condition(error);
                    if retry_allowed {
                        // Exponential backoff
                        let backoff_secs = step.retry_policy.backoff_secs * 2u64.pow(step.attempt.saturating_sub(1));
                        let next_retry_at = Utc::now() + chrono::Duration::seconds(backoff_secs as i64);
                        step.status = StepStatus::Retrying { attempt: step.attempt, next_retry_at };
                        step.error = Some(error.to_string());
                    } else {
                        step.status = StepStatus::Failed;
                        step.error = Some(error.to_string());
                        step.completed_at = Some(Utc::now());
                    }
                    step.status.clone()
                }
                other => {
                    return Err(WorkflowError::InvalidTransition {
                        step_id: step_id.to_string(),
                        from: format!("{:?}", other),
                        to: "Failed/Retrying".to_string(),
                    })
                }
            }
        };
        self.updated_at = Utc::now();
        Ok(new_status)
    }

    /// Mark a step as skipped (condition evaluated to false).
    pub fn mark_skipped(&mut self, step_id: &str) -> Result<(), WorkflowError> {
        let step = self.get_step_mut(step_id)?;
        match &step.status {
            StepStatus::Pending | StepStatus::Ready => {
                step.status = StepStatus::Skipped;
                step.completed_at = Some(Utc::now());
                self.updated_at = Utc::now();
                Ok(())
            }
            other => Err(WorkflowError::InvalidTransition {
                step_id: step_id.to_string(),
                from: format!("{:?}", other),
                to: "Skipped".to_string(),
            }),
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    fn get_step_mut(&mut self, step_id: &str) -> Result<&mut StepNode, WorkflowError> {
        self.nodes.iter_mut().find(|n| n.id == step_id).ok_or_else(|| WorkflowError::StepNotFound(step_id.to_string()))
    }

    pub fn get_step(&self, step_id: &str) -> Option<&StepNode> {
        self.nodes.iter().find(|n| n.id == step_id)
    }

    // ── Constructors ───────────────────────────────────────────────────

    /// Build a Workflow from a Plan (linear → trivial DAG with sequential deps).
    ///
    /// If `PlannedStep.depends_on` is empty, each step i depends on step i-1
    /// (sequential execution). If `depends_on` is populated, those are used
    /// directly to build the DAG edges.
    pub fn from_plan(plan: &Plan, agent_id: &str, tenant_id: &str) -> Self {
        let nodes = plan
            .steps
            .iter()
            .map(|step| {
                let depends_on = if step.depends_on.is_empty() {
                    // Linear fallback: each step depends on its predecessor
                    if step.index > 0 {
                        vec![format!("step-{}", step.index - 1)]
                    } else {
                        vec![]
                    }
                } else {
                    step.depends_on.iter().map(|i| format!("step-{}", i)).collect()
                };

                StepNode {
                    id: format!("step-{}", step.index),
                    index: step.index,
                    description: step.description.clone(),
                    tool: step.tool.clone(),
                    tool_args: step.tool_args.clone(),
                    success_criteria: step.success_criteria.clone(),
                    condition: step.condition.clone(),
                    foreach: step.foreach.clone(),
                    kind: if step.foreach.is_some() { StepKind::ForEachTemplate } else { StepKind::Normal },
                    depends_on,
                    status: StepStatus::Pending,
                    attempt: 0,
                    retry_policy: RetryPolicy::default(),
                    schema_mode: SchemaMode::default(),
                    input_schema: None,
                    output_schema: None,
                    output_data: None,
                    started_at: None,
                    completed_at: None,
                    error: None,
                }
            })
            .collect();

        let now = Utc::now();
        Self {
            id: format!("wf-{}", uuid::Uuid::new_v4()),
            agent_id: agent_id.to_string(),
            tenant_id: tenant_id.to_string(),
            goal: plan.goal.clone(),
            nodes,
            status: WorkflowStatus::Running,
            created_at: now,
            updated_at: now,
        }
    }

}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("step '{0}' not found in workflow")]
    StepNotFound(String),

    #[error("invalid transition for step '{step_id}': {from} → {to}")]
    InvalidTransition { step_id: String, from: String, to: String },
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal workflow with the given topology.
    fn make_workflow(steps: Vec<(&str, Vec<&str>)>) -> Workflow {
        let nodes = steps
            .into_iter()
            .enumerate()
            .map(|(i, (id, deps))| StepNode {
                id: id.to_string(),
                index: i,
                description: format!("Step {}", id),
                tool: None,
                tool_args: None,
                success_criteria: String::new(),
                condition: None,
                foreach: None,
                kind: StepKind::Normal,
                depends_on: deps.into_iter().map(String::from).collect(),
                status: StepStatus::Pending,
                attempt: 0,
                retry_policy: RetryPolicy { max_attempts: 3, backoff_secs: 1, retry_on: vec![] },
                schema_mode: SchemaMode::Strict,
                input_schema: None,
                output_schema: None,
                output_data: None,
                started_at: None,
                completed_at: None,
                error: None,
            })
            .collect();

        Workflow {
            id: "test-wf".into(),
            agent_id: "test-agent".into(),
            tenant_id: "test-tenant".into(),
            goal: "test goal".into(),
            nodes,
            status: WorkflowStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── ready_steps tests ──────────────────────────────────────────────

    #[test]
    fn test_ready_steps_linear() {
        // A → B → C (linear chain)
        let wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"]), ("C", vec!["B"])]);

        let ready = wf.ready_steps();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "A");
    }

    #[test]
    fn test_ready_steps_fan_out() {
        // A → B, C, D (A is root, B/C/D have no deps on each other)
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"]), ("C", vec!["A"]), ("D", vec!["A"])]);

        // Execute A
        wf.mark_running("A").unwrap();
        wf.mark_succeeded("A", serde_json::json!({})).unwrap();

        let ready = wf.ready_steps();
        assert_eq!(ready.len(), 3);
        let ready_ids: Vec<&str> = ready.iter().map(|n| n.id.as_str()).collect();
        assert!(ready_ids.contains(&"B"));
        assert!(ready_ids.contains(&"C"));
        assert!(ready_ids.contains(&"D"));
    }

    #[test]
    fn test_ready_steps_fan_in() {
        // A, B → C (diamond: both A and B must complete before C)
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec![]), ("C", vec!["A", "B"])]);

        // Only A done — C not ready
        wf.mark_running("A").unwrap();
        wf.mark_succeeded("A", serde_json::json!({})).unwrap();
        let ready = wf.ready_steps();
        let ready_ids: Vec<&str> = ready.iter().map(|n| n.id.as_str()).collect();
        assert!(ready_ids.contains(&"B"));
        assert!(!ready_ids.contains(&"C"));

        // Both A and B done — C ready
        wf.mark_running("B").unwrap();
        wf.mark_succeeded("B", serde_json::json!({})).unwrap();
        let ready = wf.ready_steps();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "C");
    }

    #[test]
    fn test_ready_steps_diamond() {
        // Full diamond: A → B, C → D
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"]), ("C", vec!["A"]), ("D", vec!["B", "C"])]);

        // Start: only A ready
        assert_eq!(wf.ready_steps().len(), 1);

        // A done: B and C ready
        wf.mark_running("A").unwrap();
        wf.mark_succeeded("A", serde_json::json!({})).unwrap();
        assert_eq!(wf.ready_steps().len(), 2);

        // B done, C still pending: D not ready
        wf.mark_running("B").unwrap();
        wf.mark_succeeded("B", serde_json::json!({})).unwrap();
        let ready = wf.ready_steps();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "C");

        // C done: D ready
        wf.mark_running("C").unwrap();
        wf.mark_succeeded("C", serde_json::json!({})).unwrap();
        let ready = wf.ready_steps();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "D");
    }

    #[test]
    fn test_ready_steps_skipped_dependency() {
        // A → B, where A is skipped — B should still be ready
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"])]);
        wf.mark_skipped("A").unwrap();

        let ready = wf.ready_steps();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "B");
    }

    // ── Retry timing tests ─────────────────────────────────────────────

    #[test]
    fn test_ready_steps_retry_timing_not_ready() {
        let mut wf = make_workflow(vec![("A", vec![])]);

        // Put A in Retrying state with future time
        wf.mark_running("A").unwrap();
        wf.handle_failure("A", "transient error").unwrap();

        // A is retrying but time hasn't elapsed
        let ready = wf.ready_steps();
        assert!(ready.is_empty());
    }

    #[test]
    fn test_ready_steps_retry_timing_ready() {
        let mut wf = make_workflow(vec![("A", vec![])]);

        // Put A in Retrying state with past time
        wf.mark_running("A").unwrap();
        let step = wf.nodes.iter_mut().find(|n| n.id == "A").unwrap();
        step.attempt = 1;
        step.status = StepStatus::Retrying { attempt: 1, next_retry_at: Utc::now() - chrono::Duration::seconds(10) };

        let ready = wf.ready_steps();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "A");
    }

    // ── Completion tests ───────────────────────────────────────────────

    #[test]
    fn test_is_complete_all_succeeded() {
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"])]);
        wf.mark_running("A").unwrap();
        wf.mark_succeeded("A", serde_json::json!({})).unwrap();
        wf.mark_running("B").unwrap();
        wf.mark_succeeded("B", serde_json::json!({})).unwrap();
        assert!(wf.is_complete());
    }

    #[test]
    fn test_is_complete_mixed_terminal() {
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec![])]);
        wf.mark_running("A").unwrap();
        wf.mark_succeeded("A", serde_json::json!({})).unwrap();
        wf.mark_skipped("B").unwrap();
        assert!(wf.is_complete());
    }

    #[test]
    fn test_is_not_complete_while_running() {
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"])]);
        wf.mark_running("A").unwrap();
        assert!(!wf.is_complete());
    }

    // ── Deadlock detection tests ───────────────────────────────────────

    #[test]
    fn test_deadlock_detected_when_dependency_failed() {
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"])]);
        // A fails terminally
        wf.mark_running("A").unwrap();
        // Exhaust retries
        let step = wf.nodes.iter_mut().find(|n| n.id == "A").unwrap();
        step.attempt = 3; // max_attempts is 3
        step.status = StepStatus::Failed;
        step.completed_at = Some(Utc::now());

        let deadlock = wf.detect_deadlock();
        assert!(deadlock.is_some());
        assert_eq!(deadlock.unwrap(), vec!["A".to_string()]);
    }

    #[test]
    fn test_no_deadlock_when_step_running() {
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec!["A"])]);
        wf.mark_running("A").unwrap();
        assert!(wf.detect_deadlock().is_none());
    }

    #[test]
    fn test_no_deadlock_when_complete() {
        let mut wf = make_workflow(vec![("A", vec![])]);
        wf.mark_running("A").unwrap();
        wf.mark_succeeded("A", serde_json::json!({})).unwrap();
        assert!(wf.detect_deadlock().is_none());
    }

    // ── State transition tests ─────────────────────────────────────────

    #[test]
    fn test_invalid_transition_succeeded_to_running() {
        let mut wf = make_workflow(vec![("A", vec![])]);
        wf.mark_running("A").unwrap();
        wf.mark_succeeded("A", serde_json::json!({})).unwrap();
        assert!(wf.mark_running("A").is_err());
    }

    #[test]
    fn test_handle_failure_retries() {
        let mut wf = make_workflow(vec![("A", vec![])]);
        wf.mark_running("A").unwrap();

        // First failure — should retry
        let status = wf.handle_failure("A", "timeout").unwrap();
        assert!(matches!(status, StepStatus::Retrying { attempt: 1, .. }));
    }

    #[test]
    fn test_handle_failure_exhausted() {
        let mut wf = make_workflow(vec![("A", vec![])]);

        // Attempt 1
        wf.mark_running("A").unwrap();
        wf.handle_failure("A", "err1").unwrap();

        // Attempt 2
        let step = wf.nodes.iter_mut().find(|n| n.id == "A").unwrap();
        step.status = StepStatus::Running;
        let step_id = "A".to_string();
        wf.handle_failure(&step_id, "err2").unwrap();

        // Attempt 3 — max_attempts is 3, so this should be terminal
        let step = wf.nodes.iter_mut().find(|n| n.id == "A").unwrap();
        step.status = StepStatus::Running;
        let status = wf.handle_failure("A", "err3").unwrap();
        assert!(matches!(status, StepStatus::Failed));
    }

    #[test]
    fn test_step_not_found() {
        let mut wf = make_workflow(vec![("A", vec![])]);
        assert!(wf.mark_running("nonexistent").is_err());
    }

    // ── next_retry_time tests ──────────────────────────────────────────

    #[test]
    fn test_next_retry_time() {
        let mut wf = make_workflow(vec![("A", vec![]), ("B", vec![])]);
        wf.mark_running("A").unwrap();
        wf.handle_failure("A", "err").unwrap();

        let nrt = wf.next_retry_time();
        assert!(nrt.is_some());
    }

    #[test]
    fn test_next_retry_time_none_when_no_retries() {
        let wf = make_workflow(vec![("A", vec![])]);
        assert!(wf.next_retry_time().is_none());
    }

    // ── from_plan tests ────────────────────────────────────────────────

    #[test]
    fn test_from_plan_linear() {
        let plan = Plan {
            goal: "test".into(),
            job_type: None,
            steps: vec![
                PlannedStep {
                    foreach: None,
                    index: 0,
                    description: "first".into(),
                    tool: Some("web_search".into()),
                    tool_args: None,
                    success_criteria: String::new(),
                    condition: None,
                    depends_on: vec![],
                },
                PlannedStep {
                    foreach: None,
                    index: 1,
                    description: "second".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: String::new(),
                    condition: None,
                    depends_on: vec![],
                },
            ],
            rationale: String::new(),
        };

        let wf = Workflow::from_plan(&plan, "agent-1", "tenant-1");
        assert_eq!(wf.nodes.len(), 2);
        assert_eq!(wf.nodes[0].depends_on.len(), 0); // Root
        assert_eq!(wf.nodes[1].depends_on, vec!["step-0"]); // Linear dep
    }

    #[test]
    fn test_from_plan_with_explicit_deps() {
        let plan = Plan {
            goal: "test".into(),
            job_type: None,
            steps: vec![
                PlannedStep {
                    foreach: None,
                    index: 0,
                    description: "A".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: String::new(),
                    condition: None,
                    depends_on: vec![],
                },
                PlannedStep {
                    foreach: None,
                    index: 1,
                    description: "B".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: String::new(),
                    condition: None,
                    depends_on: vec![0],
                },
                PlannedStep {
                    foreach: None,
                    index: 2,
                    description: "C".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: String::new(),
                    condition: None,
                    depends_on: vec![0],
                },
                PlannedStep {
                    foreach: None,
                    index: 3,
                    description: "D".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: String::new(),
                    condition: None,
                    depends_on: vec![1, 2],
                },
            ],
            rationale: String::new(),
        };

        let wf = Workflow::from_plan(&plan, "agent-1", "tenant-1");
        assert_eq!(wf.nodes[3].depends_on, vec!["step-1", "step-2"]);
    }

    #[test]
    fn test_handle_failure_respects_retry_on_filters() {
        let mut wf = make_workflow(vec![("A", vec![])]);
        wf.nodes.iter_mut().find(|n| n.id == "A").unwrap().retry_policy =
            RetryPolicy { max_attempts: 3, backoff_secs: 1, retry_on: vec![r"timeout".into()] };

        wf.mark_running("A").unwrap();
        let status = wf.handle_failure("A", "request timeout while calling provider").unwrap();
        assert!(matches!(status, StepStatus::Retrying { .. }));

        let step = wf.nodes.iter_mut().find(|n| n.id == "A").unwrap();
        step.status = StepStatus::Running;
        let status = wf.handle_failure("A", "fatal schema mismatch").unwrap();
        assert!(matches!(status, StepStatus::Failed));
    }
}
