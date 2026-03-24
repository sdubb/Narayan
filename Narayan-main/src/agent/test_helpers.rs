//! Mock implementations of agent traits for testing.
//!
//! Each mock uses a queue-based approach: responses are pushed into a
//! `tokio::sync::Mutex<Vec<T>>` and popped (FIFO) on every trait method call.
//! When the queue is empty a sensible default is returned instead.

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    agent::{
        clarifier::{ClarificationAnswers, ClarificationResult, Clarifier},
        evaluator::{EvalReflection, EvalVerdict, Evaluator},
        executor::{Executor, StepResult},
        planner::{Plan, PlannedStep, Planner},
        preflight::{Preflight, PreflightResult},
        prompts::StepHistory,
        reflector::{Reflection, Reflector},
    },
    state::AgentState,
    tools::ToolResult,
};

// ---------------------------------------------------------------------------
// MockPlanner
// ---------------------------------------------------------------------------

pub struct MockPlanner {
    create_responses: Mutex<Vec<Plan>>,
    revise_responses: Mutex<Vec<Plan>>,
}

impl MockPlanner {
    pub fn new() -> Self {
        Self { create_responses: Mutex::new(Vec::new()), revise_responses: Mutex::new(Vec::new()) }
    }

    pub fn from_create_responses(responses: Vec<Plan>) -> Self {
        Self { create_responses: Mutex::new(responses), revise_responses: Mutex::new(Vec::new()) }
    }

    pub fn from_revise_responses(responses: Vec<Plan>) -> Self {
        Self { create_responses: Mutex::new(Vec::new()), revise_responses: Mutex::new(responses) }
    }

    pub async fn push_create(&self, plan: Plan) {
        self.create_responses.lock().await.push(plan);
    }

    pub async fn push_revise(&self, plan: Plan) {
        self.revise_responses.lock().await.push(plan);
    }
}

#[async_trait]
impl Planner for MockPlanner {
    async fn create_plan(&self, _state: &AgentState, _context: &str, _available_tools: &[&str]) -> Result<Plan> {
        let mut queue = self.create_responses.lock().await;
        if queue.is_empty() {
            Ok(Plan { goal: "default mock goal".into(), job_type: None, steps: Vec::new(), rationale: String::new() })
        } else {
            Ok(queue.remove(0))
        }
    }

    async fn revise_plan(&self, plan: &Plan, _state: &AgentState, _feedback: &str) -> Result<Plan> {
        let mut queue = self.revise_responses.lock().await;
        if queue.is_empty() {
            Ok(plan.clone())
        } else {
            Ok(queue.remove(0))
        }
    }
}

// ---------------------------------------------------------------------------
// MockExecutor
// ---------------------------------------------------------------------------

pub struct MockExecutor {
    responses: Mutex<Vec<StepResult>>,
}

impl MockExecutor {
    pub fn new() -> Self {
        Self { responses: Mutex::new(Vec::new()) }
    }

    pub fn from_responses(responses: Vec<StepResult>) -> Self {
        Self { responses: Mutex::new(responses) }
    }

    pub async fn push(&self, result: StepResult) {
        self.responses.lock().await.push(result);
    }
}

#[async_trait]
impl Executor for MockExecutor {
    async fn execute_step(
        &self,
        _state: &AgentState,
        step: &PlannedStep,
        _plan: &Plan,
        _history: &StepHistory,
    ) -> Result<StepResult> {
        let mut queue = self.responses.lock().await;
        if queue.is_empty() {
            Ok(StepResult {
                step_index: step.index,
                success: true,
                output: "mock output".into(),
                final_answer_candidate: Some("mock output".into()),
                tool_results: Vec::new(),
                tools_called: Vec::new(),
                        items_processed: 0,
            connector_writes: vec![],
        })
        } else {
            Ok(queue.remove(0))
        }
    }
}

// ---------------------------------------------------------------------------
// MockEvaluator
// ---------------------------------------------------------------------------

pub struct MockEvaluator {
    responses: Mutex<Vec<EvalVerdict>>,
}

impl MockEvaluator {
    pub fn new() -> Self {
        Self { responses: Mutex::new(Vec::new()) }
    }

    pub fn from_responses(responses: Vec<EvalVerdict>) -> Self {
        Self { responses: Mutex::new(responses) }
    }

    pub async fn push(&self, verdict: EvalVerdict) {
        self.responses.lock().await.push(verdict);
    }
}

#[async_trait]
impl Evaluator for MockEvaluator {
    async fn evaluate(
        &self,
        _state: &AgentState,
        _plan: &Plan,
        _step: &PlannedStep,
        _result: &StepResult,
        _retry_count: u32,
    ) -> Result<EvalVerdict> {
        let mut queue = self.responses.lock().await;
        if queue.is_empty() {
            Ok(EvalVerdict::Continue)
        } else {
            Ok(queue.remove(0))
        }
    }

    async fn evaluate_and_reflect(
        &self,
        state: &AgentState,
        plan: &Plan,
        step: &PlannedStep,
        result: &StepResult,
        retry_count: u32,
    ) -> Result<EvalReflection> {
        let verdict = self.evaluate(state, plan, step, result, retry_count).await?;
        Ok(EvalReflection {
            verdict,
            summary: result.output.clone(),
            key_findings: vec![],
            should_revise: false,
            revision_feedback: String::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// MockReflector
// ---------------------------------------------------------------------------

pub struct MockReflector {
    responses: Mutex<Vec<Reflection>>,
}

impl MockReflector {
    pub fn new() -> Self {
        Self { responses: Mutex::new(Vec::new()) }
    }

    pub fn from_responses(responses: Vec<Reflection>) -> Self {
        Self { responses: Mutex::new(responses) }
    }

    pub async fn push(&self, reflection: Reflection) {
        self.responses.lock().await.push(reflection);
    }
}

#[async_trait]
impl Reflector for MockReflector {
    async fn reflect(&self, _state: &AgentState, _plan: &Plan, _result: &StepResult) -> Result<Reflection> {
        let mut queue = self.responses.lock().await;
        if queue.is_empty() {
            Ok(Reflection { summary: "mock reflection".into(), key_findings: Vec::new(), revised_plan: None })
        } else {
            Ok(queue.remove(0))
        }
    }

    async fn revise_plan(&self, plan: &Plan, _state: &AgentState, _feedback: &str) -> Result<Plan> {
        Ok(plan.clone())
    }
}

// ---------------------------------------------------------------------------
// MockPreflight
// ---------------------------------------------------------------------------

pub struct MockPreflight {
    responses: Mutex<Vec<PreflightResult>>,
}

impl MockPreflight {
    pub fn new() -> Self {
        Self { responses: Mutex::new(Vec::new()) }
    }

    pub fn from_responses(responses: Vec<PreflightResult>) -> Self {
        Self { responses: Mutex::new(responses) }
    }

    pub async fn push(&self, result: PreflightResult) {
        self.responses.lock().await.push(result);
    }
}

#[async_trait]
impl Preflight for MockPreflight {
    async fn check(&self, _state: &AgentState, _available_tools: &[&str]) -> Result<PreflightResult> {
        let mut queue = self.responses.lock().await;
        if queue.is_empty() {
            Ok(PreflightResult::Feasible)
        } else {
            Ok(queue.remove(0))
        }
    }
}

// ---------------------------------------------------------------------------
// MockClarifier
// ---------------------------------------------------------------------------

pub struct MockClarifier {
    check_responses: Mutex<Vec<ClarificationResult>>,
    incorporate_responses: Mutex<Vec<String>>,
}

impl MockClarifier {
    pub fn new() -> Self {
        Self { check_responses: Mutex::new(Vec::new()), incorporate_responses: Mutex::new(Vec::new()) }
    }

    pub fn from_check_responses(responses: Vec<ClarificationResult>) -> Self {
        Self { check_responses: Mutex::new(responses), incorporate_responses: Mutex::new(Vec::new()) }
    }

    pub fn from_incorporate_responses(responses: Vec<String>) -> Self {
        Self { check_responses: Mutex::new(Vec::new()), incorporate_responses: Mutex::new(responses) }
    }

    pub async fn push_check(&self, result: ClarificationResult) {
        self.check_responses.lock().await.push(result);
    }

    pub async fn push_incorporate(&self, result: String) {
        self.incorporate_responses.lock().await.push(result);
    }
}

#[async_trait]
impl Clarifier for MockClarifier {
    async fn check(&self, _state: &AgentState) -> Result<ClarificationResult> {
        let mut queue = self.check_responses.lock().await;
        if queue.is_empty() {
            Ok(ClarificationResult::Clear)
        } else {
            Ok(queue.remove(0))
        }
    }

    async fn incorporate(&self, _state: &AgentState, _answers: &ClarificationAnswers) -> Result<String> {
        let mut queue = self.incorporate_responses.lock().await;
        if queue.is_empty() {
            Ok("mock incorporated context".into())
        } else {
            Ok(queue.remove(0))
        }
    }
}
