---
description: How the Agent Runtime Goal execution flow works (AgentLoop + DAG Engine)
---

# Goal Chat / Agent Runtime Workflow

The Goal Chat workflow describes how a triggered agent role executes its goal at runtime. This is the core execution engine — the `AgentLoop` state machine driven by the `WorkerPool`, with optional DAG engine routing for parallel workflows.

## Prerequisites

- Agent role is saved and active (via Plan Mode)
- Trigger fires (schedule, webhook, user message, manual, or workforce event)
- WorkerPool has available capacity
- LLM provider credentials configured

## Steps

### Phase 1: Trigger & Dispatch

1. **Trigger fires** — cron scheduler, webhook handler, manual API call, or workforce event
2. **GoalInstance created** — status `Pending`, stored in PostgreSQL
3. **Task enqueued** — WorkerPool picks up the task asynchronously
4. **AgentState created** — ephemeral runtime state with goal, role config, workspace path

### Phase 2: AgentLoop.run_step()

5. **Cognitive control check**
   - `CognitiveControlLoop.should_continue()` — prevents infinite loops
   - Safety limits: max steps (50) and timeout (300s)

6. **Preflight (first run only)**
   - Credential checks, SLA setup, role-policy validation
   - If missing credentials → `NeedsClarification` with approval card

7. **Planning**
   - Priority order:
     1. Direct-response fast path (simple conversational requests)
     2. Pre-built skill match from `SkillRegistry`
     3. `Plan::from_workflow_outline(role)` — deterministic, no LLM call
     4. LLM planner fallback — only when no workflow outline exists
   - Plan approval gate if configured → `PlanApprovalNeeded`

8. **DAG routing check**
   - If plan has `depends_on` edges + `workflow_store` available:
     - Create `Workflow` and persist via `WorkflowStore.create_workflow()`
     - Delegate to `DagEngine.run_workflow()` (see Phase 3)
   - Otherwise: linear step-by-step execution continues

### Phase 2b: Linear Execution Path

9. **Step condition evaluation**
   - Check `StepCondition` (exists, equals, contains, gt, lt) against state metadata
   - Skip step if condition not met

10. **Clarification gate**
    - If needed: pause and return `NeedsClarification` with specific questions

11. **Inject facts**
    - Top 5 recent facts from current run (optimized — not all historical)
    - Knowledge graph + vector store retrieval

12. **Execute step**
    - `LlmExecutor.execute_step()` — tool calls, LLM interaction
    - Write `step_outputs` to state metadata (items_processed + connector_writes)
    - Persist full output to disk artifact file

13. **FailureAction check (pre-evaluator)**
    - `check_failure_rules_for_deterministic_abort()` BEFORE evaluator LLM call
    - `Abort` → immediate return, no LLM call (saves 10-15% unnecessary calls)

14. **Evaluate & Reflect**
    - `LlmEvaluator.evaluate_and_reflect()` — verdict on step quality
    - Early completion check: `check_early_completion()` mid-run

15. **Verdict dispatch**
    - `Continue` → re-enqueue for next step
    - `Retry` → exponential backoff and retry
    - `GoalComplete` → completion criteria check
    - `Abort` / `PermanentError` / `PolicyViolation` → terminal failure
    - `TransientError` → wait and retry
    - `RateLimited` → wait specified duration

16. **Atomic save**
    - `StepStateTransaction.commit()` — all metadata mutations at once
    - Crash-safe: either all changes commit or none do

17. **Completion path**
    - `check_completion_criteria()` → `Complete` or `PartiallyComplete`
    - Write `criteria_checks` to `goal_instance.result`
    - Fire-and-forget savings estimation

### Phase 3: DAG Engine Path (Parallel Workflows)

18. **Workflow loading**
    - Load fresh workflow state from DB (single source of truth)
    - Expand any `ForEach` templates that are ready

19. **Deadlock detection**
    - If no steps can ever progress → `Deadlocked` with blocked step IDs

20. **Ready step resolution**
    - Find steps whose predecessors all `Succeeded` and retry time has elapsed
    - Fan-out: independent steps execute concurrently via `tokio::spawn`
    - Fan-in: step waits until ALL predecessors succeed

21. **Parallel execution**
    - Each step is isolated: reads from DB, writes output to DB
    - No shared mutable state — `AgentState` is config/identity only
    - Step artifacts written to `_dag/step_{index}/output.json`

22. **Cycle loop**
    - Continue scheduling cycles until all steps are terminal
    - Sleep between cycles respecting `next_retry_at` timestamps

### Phase 4: Post-Execution

23. **Memory consolidation**
    - `MemoryConsolidator.consolidate_agent()` — topic memory + pgvector
    - Fire-and-forget, best-effort

24. **Parent notification** (if child agent)
    - `notify_parent_of_terminal_result()` — structured result contract
    - AgentMessage persisted to Postgres with status, artifacts, findings, confidence

25. **Savings estimation**
    - `WorkSavingsEstimator.estimate()` — human hours/cost saved
    - Written to `goal_instance.human_hours_saved` and `human_cost_saved_usd`

## Key Files

- `src/agent/loop.rs` — AgentLoop state machine, StepOutcome, StepStateTransaction
- `src/agent/dag_engine.rs` — DagEngine, parallel execution, CycleOutcome
- `src/agent/dag.rs` — Workflow, StepNode, StepStatus, ForEach expansion
- `src/agent/executor.rs` — LlmExecutor, tool calls, step execution
- `src/agent/evaluator.rs` — LlmEvaluator, CompletionCriteria checks
- `src/agent/planner.rs` — Plan, PlannedStep, Plan::from_workflow_outline()
- `src/agent/preflight.rs` — Credential checks, SLA setup
- `src/agent/step_artifacts.rs` — Per-step output files
- `src/worker/` — WorkerPool, task queue consumer

## Notes

- The worker is agnostic to whether a workflow is linear or DAG-based
- `AgentState` is config/metadata/identity only — NOT a data pipeline
- Step history is capped at `STEP_HISTORY_CAP = 30` to prevent unbounded growth
- Conversation history is bounded to prevent LLM context window overflow
- Custom tool policy is strict: `create_workspace_tool` blocked at runtime

---

## Flow Diagram

```mermaid
flowchart TD
    Trigger([Trigger Fires<br/>Schedule / Webhook / Manual /<br/>WorkforceEvent / UserMessage])

    Trigger --> CreateGoal["Create GoalInstance<br/>status: Pending"]
    CreateGoal --> Enqueue["WorkerPool<br/>picks up task"]
    Enqueue --> CreateState["Create AgentState<br/>ephemeral runtime state"]

    CreateState --> CogCheck{"Cognitive<br/>control<br/>safe?"}
    CogCheck -->|No| Abort1["Abort: exceeded<br/>safety limits"]
    CogCheck -->|Yes| StatusCheck{"AgentStatus?"}

    StatusCheck -->|Pending| PreFlight["Preflight<br/>credentials, SLA, policy"]
    PreFlight --> CredsOk{"Credentials<br/>available?"}
    CredsOk -->|No| NeedsCreds["NeedsClarification<br/>approval card"]
    CredsOk -->|Yes| Planning

    StatusCheck -->|Clarifying| WaitClarify["Wait for user<br/>clarification answers"]
    StatusCheck -->|PlanApprovalNeeded| WaitApproval["Wait for user<br/>plan approval"]
    StatusCheck -->|Running| Planning

    Planning["Planning Phase"]
    Planning --> PlanSource{"Plan source?"}

    PlanSource -->|"Direct response"| DirectPlan["Single-step plan:<br/>answer in chat"]
    PlanSource -->|"Skill match"| SkillPlan["Plan::from_skill()"]
    PlanSource -->|"Workflow outline"| WFPlan["Plan::from_workflow_outline()<br/>deterministic — no LLM"]
    PlanSource -->|"No outline"| LLMPlan["LLM Planner<br/>fallback path"]

    DirectPlan --> DagCheck
    SkillPlan --> DagCheck
    WFPlan --> DagCheck
    LLMPlan --> DagCheck

    DagCheck{"Plan has<br/>depends_on<br/>edges?"}

    %% ── DAG PATH ──
    DagCheck -->|"Yes + WorkflowStore"| DagCreate["WorkflowStore<br/>create_workflow()"]
    DagCreate --> DagLoop["DagEngine.run_workflow()"]

    DagLoop --> LoadWF["Load workflow<br/>from DB"]
    LoadWF --> ExpandFE["Expand ForEach<br/>templates"]
    ExpandFE --> DagComplete{"All steps<br/>terminal?"}
    DagComplete -->|Yes| DagDone["WorkflowOutcome::<br/>Completed"]
    DagComplete -->|No| Deadlock{"Deadlocked?"}
    Deadlock -->|Yes| DagFail["WorkflowOutcome::<br/>Failed"]
    Deadlock -->|No| FindReady["resolve_ready_steps()<br/>deps all Succeeded"]
    FindReady --> HasReady{"Ready steps?"}
    HasReady -->|No| WaitRetry["Sleep until<br/>next_retry_at"]
    WaitRetry --> LoadWF
    HasReady -->|Yes| ParExec["tokio::spawn<br/>per ready step"]

    ParExec --> StepIso["Each step isolated:<br/>read from DB → execute → write to DB"]
    StepIso --> StepResult{"Step<br/>succeeded?"}
    StepResult -->|Yes| MarkSuccess["mark_succeeded()<br/>checkpoint to DB"]
    StepResult -->|No| RetryCheck{"Retry<br/>available?"}
    RetryCheck -->|Yes| ScheduleRetry["Schedule retry<br/>with backoff"]
    RetryCheck -->|No| MarkFail["mark_failed()<br/>skip dependents"]

    MarkSuccess --> LoadWF
    ScheduleRetry --> LoadWF
    MarkFail --> LoadWF

    %% ── LINEAR PATH ──
    DagCheck -->|No| CondCheck{"Step condition<br/>met?"}
    CondCheck -->|No| SkipStep["Skip step<br/>persist skip output"]
    CondCheck -->|Yes| InjectFacts["Inject facts<br/>top 5 from current run"]

    InjectFacts --> ExecStep["LlmExecutor<br/>.execute_step()"]
    ExecStep --> WriteOutput["Write step_outputs<br/>to metadata + disk"]
    WriteOutput --> FailCheck{"Step<br/>failed?"}

    FailCheck -->|Yes| FailAction{"FailureAction<br/>rule match?"}
    FailAction -->|"Abort"| AbortNow["Immediate abort<br/>no LLM evaluator call"]
    FailAction -->|Other| Evaluate
    FailCheck -->|No| Evaluate

    Evaluate["LlmEvaluator<br/>.evaluate_and_reflect()"]
    Evaluate --> EarlyCheck["check_early_completion()<br/>mid-run criteria"]
    EarlyCheck --> Verdict{"Verdict?"}

    Verdict -->|Continue| AtomicSave["StepStateTransaction<br/>.commit()"]
    Verdict -->|Retry| BackoffRetry["Exponential<br/>backoff retry"]
    Verdict -->|GoalComplete| CompCheck
    Verdict -->|"Abort/Permanent/<br/>PolicyViolation"| Terminal["Terminal failure<br/>mark_failed()"]
    Verdict -->|"Transient/<br/>RateLimited"| WaitAndRetry["Wait and<br/>retry"]

    AtomicSave --> NextStep["Re-enqueue<br/>for next step"]
    NextStep --> CogCheck

    BackoffRetry --> CogCheck
    WaitAndRetry --> CogCheck
    SkipStep --> AtomicSave

    CompCheck["check_completion_criteria()"]
    CompCheck --> CompResult{"All criteria<br/>satisfied?"}
    CompResult -->|Yes| GoalComplete["GoalComplete ✓"]
    CompResult -->|No| PartialComplete["PartiallyComplete<br/>criteria_checks logged"]

    %% ── POST-EXECUTION ──
    GoalComplete --> PostExec
    PartialComplete --> PostExec
    DagDone --> PostExec
    Terminal --> PostExecFail
    DagFail --> PostExecFail
    AbortNow --> PostExecFail

    PostExec["Post-Execution"]
    PostExec --> MemConsol["Memory consolidation<br/>topic + pgvector"]
    MemConsol --> NotifyParent{"Has parent<br/>agent?"}
    NotifyParent -->|Yes| SendResult["notify_parent_of_terminal_result()<br/>AgentMessage with result contract"]
    NotifyParent -->|No| Savings
    SendResult --> Savings

    Savings["WorkSavingsEstimator<br/>.estimate()"]
    Savings --> Done([Goal Execution Complete ✓])

    PostExecFail["Post-Execution: Failure"]
    PostExecFail --> NotifyParentFail{"Has parent<br/>agent?"}
    NotifyParentFail -->|Yes| SendFail["Notify parent:<br/>Failed result"]
    NotifyParentFail -->|No| FailDone
    SendFail --> FailDone([Goal Execution Failed ✗])

    Abort1 --> FailDone

    style Trigger fill:#1a1a2e,stroke:#e94560,color:#fff
    style Done fill:#0f3460,stroke:#16c79a,color:#fff
    style FailDone fill:#1a1a2e,stroke:#e94560,color:#fff
    style DagLoop fill:#162447,stroke:#e2b93b,color:#fff
    style ParExec fill:#162447,stroke:#16c79a,color:#fff
    style ExecStep fill:#1a1a2e,stroke:#e94560,color:#fff
    style Evaluate fill:#1a1a2e,stroke:#e2b93b,color:#fff
    style CompCheck fill:#0f3460,stroke:#16c79a,color:#fff
    style PostExec fill:#0f3460,stroke:#16c79a,color:#fff
    style PostExecFail fill:#1a1a2e,stroke:#e94560,color:#fff
```
