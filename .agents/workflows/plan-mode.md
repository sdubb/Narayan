---
description: How to run the Plan Mode conversational agent setup flow
---

# Plan Mode Workflow

Plan mode is the one-time conversational setup phase where a user describes what an agent role should do in plain business language. The LLM infers the workflow, asks clarifying questions, and produces a fully configured `AgentRole`.

## Prerequisites

- Backend is running (`cargo run`)
- PostgreSQL database is available
- At least one LLM provider credential is configured

## Steps

1. **Create a Plan Mode Session**
   ```
   POST /plan-mode/sessions
   Body: { "tenant_id": "<tenant>", "agent_id": "<agent_id>", "description": "..." }
   ```
   - Optionally pass `template_id` to use the template fast-path (skips CapturingIntent)

2. **CapturingIntent Phase**
   - LLM Pass 1: `IntentExtractor.extract_initial()` with compact capability directory
   - Code builds targeted detail for inferred categories/candidates
   - LLM Pass 2: `IntentExtractor.refine()` with focused tool/connector detail
   - `generate_steps()` fills the `pending_steps` queue in the session

3. **ResolvingConnectors Phase**
   - `ConnectorResolver.resolve()` maps intent to specific connector names + tool overrides
   - If ambiguous: asks clarifying question (multiple connectors in same category, missing DB/API/MCP, etc.)
   - Handles: external_db, external_api, MCP server, built-in connector disambiguation

4. **CapturingClarifications Phase**
   - One step per turn from the `pending_steps` queue
   - Step order: RoleSplit → WorkforceEventFilter → Trigger → OutputDestination → Domain steps → CompletionCriteria
   - Domain skill execution brief injected via `ExecutionGuidelines::from_skill_text()`
   - Default completion criteria generated if none set

5. **Reviewing Phase**
   ```
   POST /plan-mode/sessions/:id/turn
   Body: { "message": "looks good" }
   ```
   - Shows full draft config as a review card
   - User confirms, requests changes, or runs deterministic test

6. **Test & Repair (Optional)**
   ```
   POST /plan-mode/sessions/:id/test
   ```
   - Preflight checks: tool existence, connector setup, args, schema
   - Sandbox: `Plan::from_workflow_outline(role)` — never calls LLM planner
   - If `fail` or `partial`: `POST /plan-mode/sessions/:id/revise` to repair the draft

7. **Save & Complete**
   ```
   POST /plan-mode/sessions/:id/save
   ```
   - Persists `AgentDefinition` + `AgentRole` to PostgreSQL
   - Sets role policy defaults from `RoleCategory`
   - Syncs workforce subscriptions if trigger is WorkforceEvent
   - Session snapshot preserved for repair reuse via `goal_fingerprint`

## Key Files

- `src/agent/plan_mode.rs` — main PlanModeManager, IntentExtractor, ConnectorResolver
- `src/agent/plan_mode_steps.rs` — ClarificationStep pipeline and `generate_steps()`
- `src/agent/templates.rs` — 23 pre-built RoleTemplates (template fast-path)
- `src/agent/definition.rs` — AgentRole, ExecutionGuidelines, WorkflowStep, TriggerDef
- `src/agent/planner.rs` — Plan data model, `Plan::from_workflow_outline()`

## Notes

- Plan mode does NOT execute tools — its job is to infer durable policy and scope
- Template fast-path skips CapturingIntent entirely (0 LLM calls for setup)
- Multi-role sessions stash pending roles in `draft_agent.memory_ref` as `|pending_roles:[...]`
- Save is a soft gate: non-pass test results show a warning but user can override

---

## Flow Diagram

```mermaid
flowchart TD
    Start([User Starts Plan Mode]) --> HasTemplate{template_id<br/>provided?}

    HasTemplate -->|Yes| TemplateLoad["find_template(id)<br/>build_role() + intent()"]
    HasTemplate -->|No| FreeForm["CapturingIntent Phase"]

    TemplateLoad --> TemplateSteps{"ask_steps<br/>empty?"}
    TemplateSteps -->|Yes| ReviewPhase
    TemplateSteps -->|No| Clarifications

    FreeForm --> Pass1["LLM Pass 1: IntentExtractor<br/>compact capability directory"]
    Pass1 --> BuildDetail["Code builds targeted<br/>detail for inferred categories"]
    BuildDetail --> Pass2["LLM Pass 2: IntentExtractor<br/>refine with focused context"]
    Pass2 --> GenSteps["generate_steps()<br/>build pending_steps queue"]

    GenSteps --> ResolveConn["ResolvingConnectors Phase<br/>ConnectorResolver.resolve()"]
    ResolveConn --> ConnAmbiguous{"Ambiguous or<br/>missing connector?"}
    ConnAmbiguous -->|Yes| AskConnector["Ask clarifying question<br/>which connector to use?"]
    AskConnector --> UserConnAnswer["User answers<br/>connector choice"]
    UserConnAnswer --> ResolveConn
    ConnAmbiguous -->|No| Clarifications

    Clarifications["CapturingClarifications Phase"]
    Clarifications --> StepQueue{More pending<br/>steps?}
    StepQueue -->|Yes| AskStep["Ask next clarification step<br/>RoleSplit → Trigger → Output → Domain → Criteria"]
    AskStep --> UserStepAnswer["User answers<br/>clarification"]
    UserStepAnswer --> ApplyStep["parse_and_apply()<br/>typed field update"]
    ApplyStep --> StepQueue
    StepQueue -->|No| InjectSkill["Inject domain skill<br/>execution brief"]
    InjectSkill --> ReviewPhase

    ReviewPhase["Reviewing Phase<br/>Show full draft config"]
    ReviewPhase --> UserReview{"User decision"}
    UserReview -->|Approve| SavePhase
    UserReview -->|Request Changes| Clarifications
    UserReview -->|Run Test| TestPhase

    TestPhase["Deterministic Test<br/>POST /plan-mode/sessions/:id/test"]
    TestPhase --> Preflight["Preflight checks<br/>tools, connectors, args, schema"]
    Preflight --> Sandbox["Sandbox validation<br/>Plan::from_workflow_outline()"]
    Sandbox --> TestResult{"Test result"}
    TestResult -->|Pass| ReviewPhase
    TestResult -->|Fail/Partial| Revise["POST /revise<br/>Feed result back to repair draft"]
    Revise --> ReviewPhase

    SavePhase["Save & Complete<br/>POST /plan-mode/sessions/:id/save"]
    SavePhase --> PersistDef["Persist AgentDefinition<br/>+ AgentRole to PostgreSQL"]
    PersistDef --> SetDefaults["Set role policy defaults<br/>from RoleCategory"]
    SetDefaults --> SyncWF["Sync workforce<br/>subscriptions"]
    SyncWF --> CheckMulti{"More pending<br/>roles?"}
    CheckMulti -->|Yes| NextRole(["Open plan mode<br/>for next role"])
    CheckMulti -->|No| Done([Plan Mode Complete ✓])

    style Start fill:#1a1a2e,stroke:#e94560,color:#fff
    style Done fill:#0f3460,stroke:#16c79a,color:#fff
    style NextRole fill:#0f3460,stroke:#e2b93b,color:#fff
    style FreeForm fill:#162447,stroke:#e94560,color:#fff
    style TemplateLoad fill:#162447,stroke:#16c79a,color:#fff
    style ReviewPhase fill:#1a1a2e,stroke:#e2b93b,color:#fff
    style SavePhase fill:#1a1a2e,stroke:#16c79a,color:#fff
    style TestPhase fill:#1a1a2e,stroke:#e94560,color:#fff
```
