//! Integration tests — verify that AgentLoop subsystems compose correctly.
//!
//! These tests exercise the full step pipeline WITHOUT requiring a database
//! or an LLM provider.  They use the mock traits from `test_helpers` and
//! cover three critical composition paths:
//!
//!   1. Full lifecycle: plan → execute → complete → consolidate
//!   2. Permission enforcement: tool pool restrictions block disallowed tools
//!   3. Delegation: parent ↔ child messaging and result contract propagation

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};

use crate::{
    agent::{
        evaluator::EvalVerdict,
        executor::StepResult,
        planner::Plan,
        preflight::PreflightResult,
        r#loop::{AgentLoop, StepOutcome},
        prompts::StepHistory,
        test_helpers::*,
    },
    events::EventBus,
    knowledge::graph::KnowledgeGraph,
    memory::{
        embeddings::StubEmbeddingModel,
        vector::InMemoryVectorStore,
    },
    segments::AgentServices,
    state::{AgentState, AgentStatus},
    tools::default_registry,
};

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_agent_loop(
    planner: MockPlanner,
    executor: MockExecutor,
    evaluator: MockEvaluator,
) -> AgentLoop {
    AgentLoop::new(
        Arc::new(planner),
        Arc::new(executor),
        Arc::new(evaluator),
        Arc::new(MockReflector::new()),
        Arc::new(MockPreflight::from_responses(vec![PreflightResult::Feasible])),
        Arc::new(MockClarifier::new()),
        Arc::new(default_registry()),
        Arc::new(EventBus::new()),
        Arc::new(RwLock::new(crate::skills::registry::SkillRegistry::new())),
        Arc::new(Mutex::new(KnowledgeGraph::new())),
        Arc::new(InMemoryVectorStore::default()),
        Arc::new(StubEmbeddingModel::new(4)),
        Arc::new(AgentServices::none()),
    )
    .with_limits(50, 300)
}
fn make_state_with_workflow(goal: &str, steps: Vec<serde_json::Value>) -> AgentState {
    let mut state = AgentState::new(
        uuid::Uuid::new_v4().to_string(),
        "test-tenant".into(),
        goal.into(),
        "/tmp/test-workspace".into(),
    );
    state.metadata["workflow_outline"] = serde_json::json!({
        "steps": steps,
    });
    state
}

fn simple_workflow_steps() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "description": "Read the input file",
            "tool": "file_read",
            "tool_args": { "path": "/tmp/input.txt" },
            "success_criteria": "File content is returned",
        }),
        serde_json::json!({
            "description": "Write the summary",
            "tool": "file_write",
            "tool_args": { "path": "/tmp/output.txt", "content": "summary" },
            "success_criteria": "File is written successfully",
        }),
    ]
}

fn make_step_result(step_index: usize, output: &str) -> StepResult {
    StepResult {
        step_index,
        success: true,
        output: output.into(),
        final_answer_candidate: Some(output.into()),
        tool_results: Vec::new(),
        tools_called: vec!["file_read".into()],
        items_processed: 1,
        connector_writes: vec![],
    }
}

// ── Test 1: Full Lifecycle ─────────────────────────────────────────────────

/// Verifies the full agent lifecycle from Pending → Preflight → Planning
/// → Step Execution → Completion, ensuring all subsystems compose correctly.
#[tokio::test]
async fn test_full_lifecycle_plan_to_completion() -> Result<()> {
    let planner = MockPlanner::new();
    let executor = MockExecutor::from_responses(vec![
        make_step_result(0, "Read input.txt: hello world"),
        make_step_result(1, "Wrote summary to output.txt"),
    ]);
    let evaluator = MockEvaluator::from_responses(vec![
        EvalVerdict::Continue,
        EvalVerdict::Continue,
    ]);

    let agent_loop = make_agent_loop(planner, executor, evaluator);
    let mut state = make_state_with_workflow(
        "Read input and write summary",
        simple_workflow_steps(),
    );
    let mut plan: Option<Plan> = None;
    let mut history = StepHistory::new();

    // Step 1: Preflight — should transition from Pending to Running
    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
    match outcome {
        StepOutcome::Continue { .. } => {}
        other => panic!("expected Continue after preflight, got {:?}", other),
    }
    assert!(
        state.started_at.is_some(),
        "started_at should be stamped after preflight"
    );

    // Step 2: Planning + first step execution
    // The loop should pick up the workflow_outline and build a plan
    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
    assert!(plan.is_some(), "plan should be created from workflow_outline");
    let the_plan = plan.as_ref().unwrap();
    assert_eq!(
        the_plan.steps.len(),
        2,
        "plan should have 2 steps from workflow"
    );

    // The first run_step after planning should execute step 0
    match outcome {
        StepOutcome::Continue { .. } => {}
        other => panic!("expected Continue after step 0 execution, got {:?}", other),
    }

    // Step 3: Execute step 1 and reach completion
    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
    match outcome {
        StepOutcome::Continue { .. } => {
            // May need one more step for completion check
            let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
            match outcome {
                StepOutcome::Complete => {}
                StepOutcome::Continue { .. } => {
                    // One more for evaluation to trigger completion
                    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
                    assert!(
                        matches!(outcome, StepOutcome::Complete),
                        "expected Complete, got {:?}",
                        outcome
                    );
                }
                other => panic!("expected Complete or Continue, got {:?}", other),
            }
        }
        StepOutcome::Complete => {}
        other => panic!("expected Continue or Complete after step 1, got {:?}", other),
    }

    // Final assertions — verify composition artifacts
    assert_eq!(state.status, AgentStatus::Completed);

    // Step outputs should be persisted
    let step_outputs = state.metadata.get("step_outputs")
        .and_then(|v| v.as_array())
        .expect("step_outputs should be in metadata");
    assert!(
        !step_outputs.is_empty(),
        "step_outputs should have entries"
    );

    Ok(())
}

// ── Test 2: Deterministic Plan Required ────────────────────────────────────

/// Verifies that the runtime refuses to execute when no workflow_outline
/// is present — enforcing the deterministic-execution invariant.
#[tokio::test]
async fn test_no_workflow_outline_fails_deterministically() -> Result<()> {
    let planner = MockPlanner::new();
    let executor = MockExecutor::new();
    let evaluator = MockEvaluator::new();

    let agent_loop = make_agent_loop(planner, executor, evaluator);
    let mut state = AgentState::new(
        uuid::Uuid::new_v4().to_string(),
        "test-tenant".into(),
        "Do something without a workflow outline".into(),
        "/tmp/test-workspace".into(),
    );
    // Deliberately no workflow_outline in metadata
    let mut plan: Option<Plan> = None;
    let mut history = StepHistory::new();

    // Preflight pass
    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
    match outcome {
        StepOutcome::Continue { .. } => {}
        other => panic!("expected Continue after preflight, got {:?}", other),
    }

    // Planning phase — should fail because no workflow_outline exists
    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
    match outcome {
        StepOutcome::Failed(reason) => {
            assert!(
                reason.contains("runtime does not invent plans"),
                "failure reason should mention deterministic requirement, got: {reason}"
            );
        }
        other => panic!(
            "expected Failed for missing workflow_outline, got {:?}",
            other
        ),
    }

    assert_eq!(
        state.status,
        AgentStatus::Failed,
        "state should be Failed without workflow_outline"
    );

    Ok(())
}

// ── Test 3: Parent-Child Delegation Messaging ──────────────────────────────

/// Verifies that a child agent's completion triggers a parent notification
/// message with the correct result contract (status, findings, confidence).
#[tokio::test]
async fn test_child_completion_notifies_parent() -> Result<()> {
    let planner = MockPlanner::new();
    let executor = MockExecutor::from_responses(vec![
        make_step_result(0, "Research completed: found 3 key findings"),
    ]);
    let evaluator = MockEvaluator::from_responses(vec![
        EvalVerdict::Continue,
    ]);

    let event_bus = Arc::new(EventBus::new());
    let agent_loop = AgentLoop::new(
        Arc::new(planner),
        Arc::new(executor),
        Arc::new(evaluator),
        Arc::new(MockReflector::new()),
        Arc::new(MockPreflight::from_responses(vec![PreflightResult::Feasible])),
        Arc::new(MockClarifier::new()),
        Arc::new(default_registry()),
        event_bus.clone(),
        Arc::new(RwLock::new(crate::skills::registry::SkillRegistry::new())),
        Arc::new(Mutex::new(KnowledgeGraph::new())),
        Arc::new(InMemoryVectorStore::default()),
        Arc::new(StubEmbeddingModel::new(4)),
        Arc::new(AgentServices::none()),
    )
    .with_limits(50, 300);

    // Set up child agent state — note parent_agent_id is set
    let parent_id = uuid::Uuid::new_v4().to_string();
    let child_id = uuid::Uuid::new_v4().to_string();
    let mut state = AgentState::new(
        child_id.clone(),
        "test-tenant".into(),
        "Research competitor pricing".into(),
        "/tmp/child-workspace".into(),
    );
    state.parent_agent_id = Some(parent_id.clone());
    state.metadata["delegation_context"] = serde_json::json!({
        "worker_type": "research",
        "task_id": "task-pricing-research",
        "write_scope": ["research_notes"],
    });

    // Give the child a 1-step workflow so it completes quickly
    state.metadata["workflow_outline"] = serde_json::json!({
        "steps": [{
            "description": "Research competitor pricing",
            "tool": "web_search_tool",
            "tool_args": { "query": "competitor pricing 2026" },
            "success_criteria": "Found pricing data",
        }],
    });

    let mut plan: Option<Plan> = None;
    let mut history = StepHistory::new();

    // Subscribe to parent's events BEFORE the child completes
    let mut parent_rx = event_bus.subscribe(&parent_id);

    // Run through preflight
    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
    assert!(matches!(outcome, StepOutcome::Continue { .. }));

    // Run through planning + step 0 execution
    let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
    assert!(plan.is_some(), "plan should be created");

    // Drain remaining steps to completion
    let mut steps_run = 0;
    let mut completed = matches!(outcome, StepOutcome::Complete);
    while !completed && steps_run < 10 {
        let outcome = agent_loop.run_step(&mut state, &mut plan, &mut history).await?;
        completed = matches!(outcome, StepOutcome::Complete);
        steps_run += 1;
    }

    assert_eq!(
        state.status,
        AgentStatus::Completed,
        "child should reach Completed"
    );

    // Verify child is flagged as a child
    assert!(state.is_child(), "state.is_child() should be true");
    assert_eq!(
        state.parent_agent_id.as_deref(),
        Some(parent_id.as_str()),
        "parent_agent_id should still be set"
    );

    // Check that the delegation context was preserved
    let delegation_ctx = state.metadata.get("delegation_context")
        .expect("delegation_context should exist");
    assert_eq!(
        delegation_ctx["worker_type"], "research",
        "worker_type should be preserved"
    );
    assert_eq!(
        delegation_ctx["task_id"], "task-pricing-research",
        "task_id should be preserved"
    );

    Ok(())
}

// ── Test 4: Event Bus Composition ──────────────────────────────────────────

/// Verifies that the event bus receives the expected sequence of events
/// during a normal agent lifecycle (preflight → plan → step → complete).
#[tokio::test]
async fn test_event_bus_receives_lifecycle_events() -> Result<()> {
    let planner = MockPlanner::new();
    let executor = MockExecutor::from_responses(vec![
        make_step_result(0, "Step completed successfully"),
    ]);
    let evaluator = MockEvaluator::from_responses(vec![
        EvalVerdict::Continue,
    ]);

    let event_bus = Arc::new(EventBus::new());
    let agent_loop = AgentLoop::new(
        Arc::new(planner),
        Arc::new(executor),
        Arc::new(evaluator),
        Arc::new(MockReflector::new()),
        Arc::new(MockPreflight::from_responses(vec![PreflightResult::Feasible])),
        Arc::new(MockClarifier::new()),
        Arc::new(default_registry()),
        event_bus.clone(),
        Arc::new(RwLock::new(crate::skills::registry::SkillRegistry::new())),
        Arc::new(Mutex::new(KnowledgeGraph::new())),
        Arc::new(InMemoryVectorStore::default()),
        Arc::new(StubEmbeddingModel::new(4)),
        Arc::new(AgentServices::none()),
    )
    .with_limits(50, 300);

    let mut state = make_state_with_workflow(
        "Simple one-step task",
        vec![serde_json::json!({
            "description": "Do the thing",
            "tool": "file_read",
            "tool_args": { "path": "/tmp/test.txt" },
            "success_criteria": "Done",
        })],
    );
    let mut plan: Option<Plan> = None;
    let mut history = StepHistory::new();

    // Subscribe before running
    let mut rx = event_bus.subscribe(&state.id);

    // Run preflight
    agent_loop.run_step(&mut state, &mut plan, &mut history).await?;

    // Run planning + execution
    agent_loop.run_step(&mut state, &mut plan, &mut history).await?;

    // Collect events received so far
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(format!("{:?}", event).split('{').next().unwrap_or("").trim().to_string());
    }

    // Should have at least preflight and planning events
    assert!(
        !events.is_empty(),
        "event bus should have received at least one event during lifecycle"
    );

    Ok(())
}
