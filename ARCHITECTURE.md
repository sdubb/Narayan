# Narayan Architecture

_Last updated: March 2026. Reflects the plan-mode-first architecture, deterministic workflow outlines, test/revise loop, goal-fingerprint repair reuse, multi-role agents, connector system, execution guidelines, completion criteria, savings estimation, role chat, and runtime gap fixes._

---

## What Narayan is

Narayan is a B2B AI agent platform. Tenants configure automation agents through a conversational plan mode interface — no code, no JSON — and plan mode now also validates and repairs drafts before save. Those agents can run on a schedule, in response to external events, on demand, or after another role completes. Agents read from and write to SaaS connectors (Salesforce, Zendesk, GitHub, Slack, and 22 built-ins total), external databases, REST APIs, and MCP servers.

The platform is a Rust backend (Axum, SQLx, Tokio) with a React + Vite frontend. All agent state, role config, run history, and credential data live in PostgreSQL. Vector memory uses pgvector. Workspaces are ephemeral directories on the host filesystem.

---

## Top-level modules

```
src/
├── agent/          Core agent runtime — plan mode, execution, evaluation
├── api/            Axum routes and SSE streaming
├── auth/           JWT + API key authentication
├── billing/        Stripe + PayPal subscription management
├── browser/        Headless Chrome pool for web automation
├── cognition/      Cognitive control loop for multi-step reasoning
├── compliance/     PII redaction, SLA tracking, citations, evidence packaging
├── config.rs       Environment-based configuration
├── connectors/     22 built-in SaaS connector definitions + OAuth + webhooks
├── debug/          Step recorder and replay
├── events/         In-process SSE event bus + workforce event dispatch
├── gateway/        LLM gateway — routing, cost tracking, rate limiting
├── knowledge/      In-memory knowledge graph (entity → relationship)
├── main.rs         Wiring — constructs and connects all components
├── memory/         pgvector embeddings store
├── metrics/        Prometheus counters
├── providers/      LLM provider adapters (OpenAI, Anthropic, etc.)
├── scheduler/      Cron scheduler + task queue
├── segments/       Domain segment bundles (customer_support, sales_revops, etc.)
├── skill_evolution/Skill self-improvement loop
├── skill_marketplace/ Skill publish/install flow
├── skills/         SkillRegistry — curated + domain plan-mode skills
├── state/          AgentState, GoalInstance, GoalState, WorkforceEvent
├── storage/        PostgresStore — single DB access layer
├── tenant/         Tenant model, credential store, provider config
├── tools/          ~70 tool implementations + ToolRegistry
├── webhooks/       Inbound webhook routing
├── worker/         Worker pool — consumes task queue, drives AgentLoop
└── workspace/      Per-agent workspace directories
```

---

## Core data model

### AgentDefinition
The top-level entity for a tenant's automation. Holds name, persona, constraints (hard rules that apply to all roles), connector allowlist, and status.

### AgentRole
One automation responsibility within an agent. A single `AgentDefinition` can have multiple roles — each with its own trigger, output spec, connectors, and execution guidelines. Roles are the unit of scheduling and debugging.

```
AgentRole {
    trigger:               TriggerDef,
    role_category:         RoleCategory,
    execution_guidelines:  ExecutionGuidelines,
    output_spec:           OutputSpec,
    connectors:            Vec<String>,
    tools:                 Vec<String>,   // tool overrides/scopes, e.g. "external_db:prod", "run_registered_wasm", "wasm_tool:lead_score_v1"
    memory_scope:          MemoryScope,   // global | agent | role
    execution_limits:      ExecutionLimits,
}
```

`role_category` is persisted and treated as first-class runtime policy (runtime derives job type from it before falling back to heuristic detection).

`memory_scope` and `execution_limits` are also persisted on each role and injected into runtime role-policy context on every run.

### TriggerDef
```
TriggerDef {
    trigger_type:  Schedule | Webhook | UserMessage | Manual | WorkforceEvent,
    cron:          Option<String>,        // "0 9 * * 1"
    source_connector: Option<String>,     // "salesforce"
    event_filter:  Option<String>,        // "lead_created"
    confidence:    TriggerConfidence,     // High | Medium | Low
    input_mapping: Option<serde_json::Value>,
    ...
}
```

`TriggerConfidence` records how certain the plan mode parser was. `Medium`/`Low` triggers were confirmed by the user before saving.

### ExecutionGuidelines
The typed contract between plan mode and the runtime. Four buckets:

```
ExecutionGuidelines {
    rules:               Vec<GuidelineRule>,       // BEFORE/AFTER/ALWAYS guards
    failure_handling:    Vec<FailureRule>,          // tool-scoped failure responses
    priorities:          Vec<String>,              // relative weights
    completion_criteria: Vec<CompletionCriterion>, // done-when assertions
}
```

`workflow_outline: Vec<WorkflowStep>` is the execution contract. It stores ordered, typed steps - description, tool, args template, success criteria, and condition - and is the source of truth for runtime execution and plan-mode test mode. When present, runtime builds a deterministic `Plan` from it instead of asking the LLM planner to invent one. The `planner` module still exists as the `Plan` translator and fallback path, but workflow-outline roles do not rely on it to invent new steps.

**GuidelineRule** - `{ text, tool_scope: Option<String>, phase: Before|After|Always }`. Rendered as numbered list in role-policy prompts with scope prefixes like `[BEFORE salesforce.update_record]`.

**FailureRule** — `{ text, tool_scope, action: FailureAction }`. `FailureAction` is a tagged enum: `SkipAndLog { log_path }`, `SkipSilently`, `RetryOnce`, `EscalateToHuman { notify_channel }`, `Abort`. The agent loop evaluates matching rules before the LLM evaluator on every step failure.

**CompletionCriterion** — `{ description, check: CompletionCheck }`. `CompletionCheck` variants: `AllItemsProcessed { collection_hint }`, `OutputExists { path_hint }`, `RecordUpdated { connector }`, `CountMatches { source, target }`, `ErrorsLogged { log_hint }`, `Custom { assertion }`. Checked mechanically against `state.metadata["step_outputs"]` and workspace at run completion.

### GoalInstance
One run of one AgentRole. Created by the scheduler or via webhook. Fields include status (`Pending → Running → Completed | PartiallyComplete | Failed | Cancelled`), `result` (JSONB — carries `criteria_checks`, `step_outputs`, processed item counts), `cost_usd`, `human_hours_saved`, `human_cost_saved_usd`.

`PartiallyComplete` is a first-class status: set when all plan steps ran but one or more `CompletionCriterion` checks failed. The `result.criteria_checks` array carries per-criterion `{ description, satisfied, check_type, detail }` for the run browser UI.

---

## Plan mode

Plan mode is the one-time conversational setup for an agent role. Users either describe what they want in plain language (free-form path) or select one of 20 pre-built templates (template fast-path). Both paths produce identical `AgentRole` output — the template path just skips the questions it already answers, reducing setup from ~7 turns to 0–3.

### Phase flow - free-form path

1. CapturingIntent
   - LLM pass 1: IntentExtractor with a compact capability directory
   - code builds targeted detail for the inferred categories/candidates
   - LLM pass 2: IntentExtractor refinement with focused tool/connector detail
   - generate_steps() fills the pending_steps queue in the session
2. ResolvingConnectors
   - one or more clarifying questions if connector or custom-tool scope is unresolved
3. CapturingClarifications
   - one step per turn from the queue
   - domain skill execution brief injected
   - default completion criteria generated if none set
4. Reviewing
   - the user reviews the full draft
   - deterministic test can run before save
5. Complete
   - save() persists AgentDefinition + AgentRole
   - the completed plan-mode session snapshot is preserved for repair reuse

### Deterministic test and repair loop

Plan mode now has a dedicated validation path:
- `POST /plan-mode/sessions/:id/test` runs deterministic preflight + sandbox validation.
- Preflight checks tool existence, connector setup, args, and schema only.
- Sandbox runs `Plan::from_workflow_outline(role)` only. It never calls the LLM planner.
- `src/agent/planner.rs` still owns the `Plan` data model and the deterministic `Plan::from_workflow_outline(role)` conversion used by plan mode and runtime validation. The old LLM-generated plan path remains only as a fallback when no workflow outline exists.
- The result is structured JSON: `status`, `steps`, `criteria_checks`, `summary`, `confidence`.
- Uploaded documents are saved in the session workspace, extracted into concise attachment context, and reused in the draft prompt instead of being dumped wholesale into the LLM context.
- If the result is `fail` or `partial`, `POST /plan-mode/sessions/:id/revise` feeds the structured result back into plan mode to repair the draft.
- Save is a soft gate: users see a warning on non-pass results but can still override.
- Matching goals reuse the latest repaired snapshot via `goal_fingerprint`, with `repair_version`, `reused_from_session_id`, and `repair_root_session_id` tracking the chain.
- Fingerprint reuse is heuristic, not absolute. The fingerprint is derived from the normalized goal text plus role category, trigger, connectors, tools, and workflow outline. If the draft changes materially, it gets a new fingerprint/version so unrelated goals do not inherit old repairs.

### IntentExtractor
Two-pass LLM extraction with a typed JSON schema.

Pass 1 receives a compact capability directory (tool categories, connector categories, installed vs available status, tenant custom connections) and returns inferred intent categories/candidates.

Pass 2 receives targeted detailed context only for inferred categories/candidates (selected tool-category specs and connector operation summaries) and refines the result.

Returns intent plus runtime policy hints, including:
- `trigger_confidence: "high"|"medium"|"low"` + `trigger_confirmation` question
- `output_questions: []` — LLM-generated specific questions about output destination
- `multi_role_suggested: bool` + `responsibilities: []` — multi-role split detection
- `uses_external_db`, `uses_external_api` — named custom connection references
- `preferred_tool_categories`, `preferred_tools`
- `candidate_wasm_tools` (exact tenant WASM tool names when deterministic custom logic is needed)
- `needed_connector_categories`, `candidate_connectors`
- `missing_capabilities` (`custom_db`, `custom_api`, `connector/<category>`, `tool/<category>`)
- `workflow_outline` (ordered `WorkflowStep` entries persisted as the execution contract)

#### Two-pass inference example

User input:

> "When a new Zendesk ticket arrives, summarize it, create a Notion page, and notify #support-alerts in Slack."

Pass 1 prompt context includes:
- compact tool category map (names only, no full schema dump)
- connector categories and names with status (`installed` / `available`)
- tenant custom connections by name/summary

Pass 1 output (abridged):
```json
{
  "category": "customer_support",
  "preferred_tool_categories": ["data", "communication"],
  "preferred_tools": ["data_extractor"],
  "candidate_wasm_tools": [],
  "needed_connector_categories": ["support", "project_management", "communication"],
  "candidate_connectors": ["zendesk", "notion", "slack"],
  "missing_capabilities": [],
  "workflow_outline": [
    "fetch new support ticket",
    "summarize issue and context",
    "create destination knowledge record",
    "send notification to channel"
  ],
  "trigger_hint": "webhook",
  "trigger_source": "zendesk",
  "trigger_event": "ticket_created"
}
```

Code then builds targeted detail for pass 2:
- tool specs for `data` and `communication` categories
- connector ops/status for `zendesk`, `notion`, `slack`

Pass 2 output refines exact preferences and keeps the same intent shape. Plan mode persists these as role policy (`role.tools`, connector scope, `ExecutionGuidelines` hints, and `workflow_outline`) before runtime execution starts.

#### Plan mode vs runtime tool discovery

Plan mode does not execute tools and does not call `request_more_tools`. Its job is to infer durable policy and scope.

- Plan mode grounding:
  - pass 1 gets compact capability directory (category maps + connector directory + tenant custom connections + enabled tenant WASM names)
  - pass 2 gets targeted detail for inferred categories/candidates only
- Runtime grounding:
  - executor prompts include category quick maps
  - when a step needs more depth, runtime can call `request_more_tools` by category
  - selector still enforces hard tool budget and role scope
  - runtime executes the saved `workflow_outline` when present instead of inventing a new plan

This split keeps plan mode deterministic and lightweight while still letting runtime fetch detailed tool context exactly when needed.

#### How hint persistence works

`apply_execution_hints()` writes intent hints into typed role policy with explicit hygiene:
- `preferred_tool_categories` → `GuidelineRule`: `Prefer these tool categories when relevant: ...`
- `candidate_wasm_tools` → role tool scope entries: `run_registered_wasm` + `wasm_tool:<name>`
- `needed_connector_categories` → `GuidelineRule`: `Prefer connectors from these categories when relevant: ...`
- `workflow_outline` → `ExecutionGuidelines.workflow_outline` as ordered `WorkflowStep` entries

Before writing fresh hint-derived values, plan mode removes prior entries with those prefixes so re-runs/reconfiguration don't keep stale duplicates.

At runtime:
- `workflow_outline` drives deterministic execution order.
- `workflow_hints()` returns only `step:` tagged priorities (so policy rules like "Never auto-send..." are not misinterpreted as workflow order)
- `preferred_*_categories()` reads all matching prefixed rules (not just first match), then de-duplicates.

#### Role policy defaults at setup time

After intent extraction, plan mode sets durable role policy defaults from inferred `role_category`:
- `role.role_category = RoleCategory::from_slug(intent.category)`
- if agent persona is empty, use category default persona
- set role `memory_scope` from category default
- set role `execution_limits` from category default (when still default/empty)

Template fast-path applies the same policy defaults so template-created roles and free-form roles follow the same runtime contract.

### ClarificationStep pipeline (`plan_mode_steps.rs`)
`generate_steps(intent, category, installed, existing_roles)` builds an ordered queue. `existing_roles` is the list of role names already on the agent — loaded from the DB before queue generation so the pipeline can ask about cross-role relationships.

Step order:
1. `RoleSplit` — if `multi_role_suggested`, ask A/B
2. **If `trigger_hint == "workforce_event"`:**
   - `WorkforceEventFilter` — "Which role triggers this?" → sets `workforce_event_filter = "role_name == 'X' AND status == 'completed'"`
   - `WorkforceEventInputMapping` — "What data do you need?" → `infer_input_mapping()` converts natural language to JSONPath: `{ "lead_ids": "$.output_data.lead_ids" }`
   - `DependsOnRole` (optional) — "Enforce strict ordering too?" → stores `"name:Role Name"` hint, resolved to actual UUID at `save()` time
3. `Trigger` — confirm cron/event (skip if `trigger_confidence == "high"` or WorkforceEvent)
4. `OutputDestination` — ask where output goes if `output_destination_hint` is empty
5. Domain steps — 4–5 typed questions per category (see below)
6. `CompletionCriteria` — "what does done look like?" or "auto"

`ResolvingConnectors` clarification now uses exact connector-name token matching (not free substring matching against summaries). If multiple connector names are present in one reply, plan mode asks the user to choose one exact name; if none are detected, it re-prompts with explicit examples.

The same resolving phase is also used for custom deterministic logic gaps. If intent inference returns `missing_capabilities` like `tool/<category>` and no suitable `candidate_wasm_tools`, plan mode blocks progression and asks the user to select (or set up) an enabled tenant WASM tool before moving to runtime.

Each `ClarificationStep { id, question, field: StepField, required, hint }` maps to one field on the draft role. `parse_and_apply()` is a typed switch — no free-text blob parsing. The queue is serialised as `pending_steps: Vec<serde_json::Value>` in the session and persisted between turns.

**`infer_input_mapping(answer)`** — 14 keyword patterns map natural language to JSONPath expressions. "lead IDs" → `$.output_data.lead_ids`, "file path" → `$.output_data.output_path`, "count" → `$.output_data.processed`, "ticket IDs" → `$.output_data.ticket_ids`, etc. Falls back to `$.output_data` for unrecognised descriptions.

### Domain steps (`domain_steps_for(category)`)
Seven categories, each with 3–5 typed steps:

| Category | Key questions | Fields |
|---|---|---|
| `customer_support` | Response mode, SLA, escalation, knowledge source | GuidelineRule, AgentConstraint, FailureHandling |
| `sales_revops` | Write-back, enrichment sources, outreach mode, skip criteria | GuidelineRule, FailureHandling |
| `finance_accounting` | Write access, approval threshold, mismatch handling | AgentConstraint, FailureHandling |
| `devops` / `it_ops_itsm` | Environment, blast radius, alert channel, rollback | AgentConstraint, FailureHandling |
| `hr_people_ops` | Visibility, write-back, communication mode | AgentConstraint, GuidelineRule |
| `legal_contract` | Action scope, escalation clauses, output format | AgentConstraint, FailureHandling, OutputFormat |
| `research_analyst` | Depth, freshness, on-no-results | GuidelineRule, AgentConstraint, FailureHandling |

### Domain skill registry (`skills/registry.rs`)
`curated_skills()` includes both operational skills (Gmail connector onboarding, database monitoring) and plan-mode domain skills named `planmode:<category>`. The plan-mode skills carry the EXECUTION BRIEF text block, which `ExecutionGuidelines::from_skill_text()` parses into typed rules + failure handlers + completion criteria. Injected into the role at the end of `CapturingClarifications`.

### Template fast-path (`agent/templates.rs`)

When `template_id` is passed to `start_plan_mode_session`, plan mode skips `CapturingIntent` entirely — no `IntentExtractor` LLM call. Instead:

1. `find_template(id)` locates the matching `RoleTemplate`
2. `tmpl.build_role(agent_id, tenant_id)` constructs a fully pre-configured `AgentRole` with typed guidelines, failure rules, and completion criteria
3. `tmpl.intent()` is injected as `intent_cache` — the category, trigger, and output fields are already correct
4. `phase` is set to `CapturingClarifications` with only `tmpl.ask_steps` in the queue — 0 to 3 questions per template, only genuinely unknown values like connector channel names or database names
5. Required connectors are checked against installed ones — if any are missing, the response prompts the user to connect them in Settings first

If `ask_steps` is empty, phase jumps directly to `Reviewing` and shows the review card immediately.

**`RoleTemplate` struct:**
```
RoleTemplate {
    id, name, description, persona, category, emoji,
    required_connectors: &[&str],   // checked at session start
    intent:    fn() -> serde_json::Value,  // pre-answered intent_cache
    build_role: fn(agent_id, tenant_id) -> AgentRole,  // typed pre-built role
    ask_steps: &[&str],             // only what can't be pre-answered
}
```

All 20 templates are static data — no DB table, no migrations, no API to manage them.

### Multi-role sessions
If the user chooses split, remaining `RoleResponsibility` objects are stashed in `draft_agent.memory_ref` as `|pending_roles:[...]`. After `save()` returns, the frontend detects this and immediately opens plan mode again for role 2 on the same agent, pre-populated with the responsibility name. This repeats until all pending roles are configured.

**Adding a role to an existing agent** — `PlanModeChat` passes `existingAgentId` to the session. `build_step_queue_and_ask` loads the agent's existing role names from the DB and passes them to `generate_steps()`. If the new role should trigger from an existing one, the `WorkforceEventFilter` and `DependsOnRole` steps surface automatically with the existing role names listed. `save()` resolves `"name:Role Name"` hints to real role UUIDs at write time.

---

## Agent runtime

### Worker → AgentLoop
The `WorkerPool` runs a configurable number of async workers. Each worker pops tasks from the queue and calls `AgentLoop::run_step()` once per task. The loop is not a continuous loop — it runs exactly one step, returns a `StepOutcome`, and re-enqueues if more steps remain.

```
StepOutcome {
    Continue { delay_secs },
    NeedsClarification { questions },
    PlanApprovalNeeded,
    Infeasible { reason },
    Complete,
    PartiallyComplete { note },   // new: criteria not all met
    Failed(String),
    Delegating { child_ids },
}
```

### Run step sequence
```
1. Preflight           → credential checks, SLA setup, role-policy checks
2. Deterministic plan  → Plan::from_workflow_outline(role) when workflow_outline exists
3. Clarification gate  → ask user if needed
4. Execute step        → LlmExecutor.execute_step()
5. Write step_outputs  → items_processed + connector_writes → state.metadata
6. FailureAction check → apply_failure_action_override() before evaluator
7. Evaluate + Reflect  → LlmEvaluator.evaluate_and_reflect()
8. Verdict dispatch    → Continue | Retry (backoff) | GoalComplete | Abort
9. GoalComplete path   → check_completion_criteria() → Complete | PartiallyComplete
10. Persistence        → write criteria_checks to goal_instance.result
```

The normal runtime path is workflow-outline-first. It does not ask the LLM planner to invent a plan when a role already has `workflow_outline`; the LLM planner is only used as a fallback when the outline is missing or invalid.

Custom tool policy in runtime is strict:
- `create_workspace_tool` is blocked during run execution.
- `run_registered_wasm` is allowed only for plan-mode-approved role scopes (`wasm_tool:<name>` markers persisted on `role.tools`).
- If a step requests an out-of-scope WASM tool, executor returns an explicit scope error instead of attempting dynamic tool creation.

### FailureAction override (`loop.rs: apply_failure_action_override`)
Before the evaluator verdict is dispatched, `apply_failure_action_override` checks the role's `failure_handling` rules against the current step failure. It matches by `tool_scope` (which tools were called) and error text. If a match is found:
- `RetryOnce` → forces `EvalVerdict::Retry` (only on first failure; falls through after)
- `EscalateToHuman` → submits a review request via `services.reviews`, returns `Abort`
- `SkipSilently` / `SkipAndLog` → returns `Continue` (advances to next step)
- `Abort` → returns `Abort`

### CompletionCriteria check (`evaluator.rs: check_completion_criteria`)
Called on every `GoalComplete` verdict. Returns `Vec<CriterionResult>`, each with `satisfied: bool`, `check_type`, and a human-readable `detail` string. Results written to `goal_instance.result["criteria_checks"]`. If any criterion fails, the run is marked `PartiallyComplete` with the criteria list as the note.

### step_outputs metadata
Every step where `items_processed > 0` or `connector_writes` is non-empty writes an entry to `state.metadata["step_outputs"]`:
```json
{ "step": 3, "success": true, "processed": 47, "connectors": ["salesforce"] }
```
This is the canonical source for completion criteria checking and savings estimation.

---

## Connector system

### Built-in connectors (22)
Defined in `tools/connector_tool.rs` as `ALL_CONNECTORS: &[ConnectorDef]`. Each has `name`, `category`, `keywords`, `summary`, `auth_type` (Bearer/OAuth2/ApiKey), and `settings` fields for subdomain/domain config. Grouped into domain segments:
- `customer_support`: zendesk, intercom, freshdesk
- `sales_revops`: salesforce, hubspot
- `devops`: github, pagerduty, servicenow
- `finance_accounting`: quickbooks, stripe, docusign
- `hr_people_ops`: greenhouse
- `productivity`: slack, notion, gmail, outlook
- `engineering`: dbt_cloud

OAuth flows in `connectors/oauth.rs`. Webhook ingestion in `connectors/poller.rs`. Credential storage in `connectors/installs.rs` (ConnectorInstallStore).

### ConnectorTool (`tools/connector_tool.rs`)
Each connector registers as a named tool in `ToolRegistry`. At execution time, `ConnectorTool::execute()` priority order:
1. Explicit `auth_token` arg → MCP session
2. Stored token from `ConnectorInstallStore` → `rest_execute()` (real HTTP API calls)
3. Fallback → MCP session

`rest_execute()` implements ~100 operations across all 20 connectors. Tenant ID is injected into tool args by the executor before dispatch so credential lookup requires no user input.

### External connections (custom)
Three types of custom connections registered by tenants:
- **Databases** → `external_db` tool. Operations: `schema`, `query`, `execute`, `table_preview`, `explain`. 60s timeout, 1000-row cap. SELECT enforced.
- **REST APIs** → `external_api` tool. All HTTP verbs. Token loaded from `connector_installs`.
- **MCP servers** → registered as named connectors. Tools discovered via `tools/list`.

Plan mode detects custom connection mentions via `IntentExtractor` (`uses_external_db`, `uses_external_api` fields) and routes them to the right tool in `execution_guidelines.rules`.

---

## Role chat

`RoleChatManager` provides a conversational interface for existing roles. Three methods:

**`start(tenant_id, role_id)`** — loads role config + last 5 run records. Returns greeting with role summary and plain-language run history.

**`turn(session, message)`** — builds system prompt injecting role config + last 10 runs (timestamp, status, cost, failure reason). LLM reply is parsed for a `\`\`\`change` block. If found, returns a `RoleChange` for user confirmation.

**`apply_change(tenant_id, role_id, change)`** — handles 12 change types:
`Schedule`, `AddConstraint`, `RemoveConstraint`, `UpdateGuidelines`, `UpdateOutput`, `UpdateConnectors`, `RenameRole`, `PauseRole`, `ResumeRole`, `AddFailureRule`, `RemoveFailureRule`, `SetFailureRules`

The LLM never writes directly. Every change goes through a frontend confirmation card before `apply` is called. `FailureRuleEditor` in the UI can also call `AddFailureRule`/`RemoveFailureRule` directly without the LLM.

---

## Savings estimation (`agent/savings.rs`)

Runs fire-and-forget on every `Complete` or `PartiallyComplete` outcome in `worker.rs`.

**`WorkSavingsEstimator.estimate(gi, role)`**:
1. Category from role purpose → market hourly rate (legal $180/hr → general $35/hr)
2. `extract_item_count()` → reads `gi.result["processed"]` or `completion_criteria.AllItemsProcessed`
3. `minutes_per_item()` → scans `execution_guidelines.rules` text for work type keywords
4. `human_hours = items × minutes / 60`
5. `quality_factor()` → 0.0 if no output, 0.5 if result exists but no counts, 1.0 with real counts
6. For `PartiallyComplete`: `partial_completion_fraction()` pro-rates by `processed/expected`

Results written to `goal_instance.human_hours_saved` and `human_cost_saved_usd`.

`GET /savings` aggregates per-tenant: total runs, total human hours, total human cost, total AI cost, ROI multiple, per-role breakdown.

---

## Database schema (key tables)

```
agent_definitions       — AgentDefinition (JSONB: connectors, constraints)
agent_roles             — AgentRole (JSONB: trigger, execution_guidelines, output_spec, tools)
goal_instances          — One run per role trigger (JSONB: result/criteria_checks, DOUBLE: cost_usd, human_hours_saved)
plan_mode_sessions      — Plan-mode conversation snapshots (JSONB: conversation, attachments, pending_steps, intent_cache, draft_role; columns: attachment_context, session_workspace, goal_fingerprint, repair_version, reused_from_session_id, repair_root_session_id)
role_chat_sessions      — In-progress role chat conversations (JSONB: conversation, pending_change)
role_chat_sessions      — same (JSONB: pending_change for typed RoleChange)
connector_installs      — OAuth tokens + API keys per tenant per connector
tenant_connectors       — Custom connections (databases, REST APIs, MCP servers)
agents                  — Runtime AgentState (ephemeral, re-created per run)
vector_documents        — pgvector embeddings for step findings
```

All queries bind `tenant_id` from the JWT-validated `AuthenticatedTenant` extractor. Cross-tenant reads are structurally impossible — `tenant_id` is never read from the request body.

---

## API surface

### Agent management
```
GET    /agent-definitions              — list with roles embedded
POST   /agent-definitions             — create
GET    /agent-definitions/:id          — get
PUT    /agent-definitions/:id          — update
DELETE /agent-definitions/:id          — delete
GET    /agent-definitions/:id/roles   — list roles
POST   /agent-definitions/:id/roles   — create role
PUT    /agent-definitions/:id/roles/:role_id
DELETE /agent-definitions/:id/roles/:role_id
GET    /agent-definitions/:id/goal-instances
GET    /agent-definitions/:id/roles/:role_id/goal-instances
POST   /agent-definitions/:id/roles/:role_id/trigger
GET    /goal-instances/:id             — full detail with criteria_checks
```

### Plan mode
```
GET    /plan-mode/templates            — list all 20 pre-built templates (id, name, description, persona, emoji, required_connectors)
POST   /plan-mode/sessions             — start (body: agent_name, agent_id?, template_id?)
POST   /plan-mode/sessions/:id/turn   — send message, get reply
POST   /plan-mode/sessions/:id/test   — deterministic preflight + sandbox validation
POST   /plan-mode/sessions/:id/revise  — feed a failed/partial test result back into plan mode
POST   /plan-mode/sessions/:id/save   — save AgentDefinition + AgentRole
```

### Role chat
```
POST   /roles/:role_id/chat                    — start session
POST   /roles/:role_id/chat/:sid/turn          — send message
POST   /roles/:role_id/chat/:sid/apply         — apply confirmed RoleChange
```

### Connections
```
POST   /connections/mcp/test, /connections/mcp
POST   /connections/api/test, /connections/api
POST   /connections/db/test,  /connections/db
GET    /connections
DELETE /connections/:name
```

### ROI
```
GET    /savings                        — tenant aggregate + per-role breakdown
```

---

## Frontend structure

```
src/
├── pages/
│   ├── ChatPage.jsx       — shell: agent list sidebar + main content + SavingsCard
│   ├── AgentPage.jsx      — agent detail: roles, run history, savings
│   ├── AuthPage.jsx
│   └── SettingsPage.jsx
├── components/
│   ├── agent/
│   │   ├── PlanModeChat.jsx      — locked conversational overlay for new agents
│   │   ├── RoleChatDrawer.jsx    — slide-in chat + FailureRuleEditor
│   │   ├── RunDetailDrawer.jsx   — criteria checklist + step outputs per run
│   │   ├── FailureRuleEditor.jsx — inline failure rule add/remove/edit
│   │   ├── AgentTimeline.jsx     — SSE-driven live step timeline
│   │   └── ...
│   ├── cards/
│   │   ├── SavingsCard.jsx       — ROI banner: hours saved, cost, multiplier
│   │   ├── PlanApprovalCard.jsx  — credential gap + plan confirm flow
│   │   └── ...
│   ├── layout/
│   │   └── Sidebar.jsx           — agent list with role counts and live status
│   └── settings/
│       └── ConnectorsTab.jsx     — built-in OAuth + custom MCP/API/DB connections
└── api/index.js           — typed API client
```

### Key frontend state flows

**New agent**: `ChatPage` → `PlanModeChat` (no cancel) → POST `/plan-mode/sessions` → sequential turns → POST `/plan-mode/sessions/:id/save` → sidebar refreshes.

**Add role**: `AgentPage` → `PlanModeChat` (with cancel, `existingAgentId` set) → same plan mode flow → role added to existing agent.

**Run detail**: `AgentPage` run row click → `RunDetailDrawer` → GET `/goal-instances/:id` → criteria checklist + step outputs + savings stats.

**Role chat**: `AgentPage` Chat button → `RoleChatDrawer` → session start loads role + failure rules → conversation + `FailureRuleEditor` → confirmed changes via POST `…/apply`.

---

## Key design decisions

**Plan mode is sequential, not a free-form chat.** The `ClarificationStep` pipeline means each turn has exactly one question, one answer, one field written. There is no blob parsing or regex. Ambiguous answers stay in the queue for re-asking. The draft also carries a typed `workflow_outline`, a deterministic test pass, and a repair loop before save.

**Templates are static data, not database records.** All 20 `RoleTemplate` structs live in `agent/templates.rs` as a `static` array. No migration, no admin API, no versioning complexity. Each template carries `build_role` and `intent` as function pointers — the pre-configured role is constructed at request time, not stored. Templates can only be changed by deploying new code, which is the right constraint: templates represent product decisions, not user data.

**`generate_steps()` is context-aware.** It accepts `existing_roles` (loaded from the DB) so it can ask meaningful cross-role questions — "which role triggers this?" with actual role names listed. WorkforceEvent triggers get three dedicated steps that fully configure `workforce_event_filter`, `input_mapping`, and `depends_on_role_id` before save.

**`save()` resolves name hints to real IDs.** `DependsOnRole` stores `"name:Lead Enrichment & Drafts"` during the conversation, resolved to the actual UUID at write time. Keeps the conversational step simple while ensuring the DB always has a valid reference.

**ExecutionGuidelines is a typed contract.** The planner receives a numbered, phase-prefixed prompt (`RULES: 1. [BEFORE salesforce.update] Read first…`). The evaluator receives `DONE WHEN ALL OF: [ ] …`. Both are derived from the same typed struct — no prompt engineering divergence. `workflow_outline` is the execution contract, not a soft hint.

**Repair is session-local and versioned.** `goal_fingerprint`, `repair_version`, `reused_from_session_id`, and `repair_root_session_id` track the repair chain for one normalized goal. The same goal can reuse its latest repaired snapshot, while completed sessions remain immutable snapshots on disk and in PostgreSQL.

**FailureAction is checked before the evaluator.** This means role-level failure rules fire deterministically, not depending on LLM judgment. The LLM's `Retry` verdict is additive on top of the `RetryOnce` override — they don't conflict.

**CompletionCriteria are checked mechanically.** No LLM call at run completion. File existence, item counts, and connector write records are checked against `state.metadata` and the workspace. Results are persisted to `goal_instance.result["criteria_checks"]` for offline browsing.

**The review card shows what will be active.** `active_services_for_category(category)` returns the compliance services that will automatically activate (SLA tracking, PII redaction, citations, evidence packaging, human review queue). Users see these before confirming — services are never silently activated.

**Savings estimation is quality-gated.** A run that produced no output gets 0 credit. Partial runs are pro-rated. The estimator uses structured `step_outputs` metadata, not output text.

**Tool expansion is staged and bounded.** Both plan mode and executor prompts include compact category quick maps (filesystem/web/code/data/memory/infra/integration/communication/security/automation) so the model can call `request_more_tools` by category when needed without receiving all tool schemas up front.

**Runtime custom tool creation is disabled.** Custom deterministic logic must be onboarded and tested in plan mode (or tenant settings) first, then explicitly approved per role. Runtime only executes those approved tools through `run_registered_wasm`.

**Role-category tool injection is capped.** The selector limits role-category expansion to a small per-category slice (currently 4 tools/category) before applying keyword scoring, preventing broad categories from consuming the full 20-tool budget.

**All tenant_id bindings come from JWT.** Every DB query in PostgresStore takes `tenant_id: &str` as the first parameter. The HTTP layer always passes `tenant.tenant_id` from `AuthenticatedTenant` — never from request body or path params.

---

## Segment system

Domain-specific capability bundles in `src/segments/`. Each segment registers connectors, tools, and services appropriate to a job category. Runtime execution and plan-mode grounding have access only to the tools registered for the tenant's segment. Current segments: `compliance_ops`, `customer_success_renewals`, `customer_support`, `data_analytics`, `engineering`, `finance_accounting`, `hr_people_ops`, `it_ops_itsm`, `legal_contract`, `marketing_growth`, `procurement_vendor_ops`, `research_intelligence`, `sales_revops`, `security_ops_grc`.

---

## Skill system

`SkillRegistry` holds `Skill { name, description, steps, aliases, version }`. `Plan::from_skill()` builds a deterministic plan from a skill without an LLM call. Skills evolve via `skill_evolution/evolution.rs` — successful step outputs are extracted and used to improve existing skill steps.

The marketplace (`skill_marketplace/`) allows skills to be uploaded, discovered, and installed by name. Skills in `curated_skills()` ship with the platform and include the plan-mode domain skills (`planmode:customer_support` etc.) plus internal workflow guidance packs such as the Superpowers-style planning and review skills.

---

## Compliance layer

- **PII redaction** (`compliance/pii.rs`) — scrubs tool args before they leave the process
- **SLA tracking** (`compliance/sla.rs`) — monitors elapsed time, fires `EscalateToHuman` or `Notify` escalation actions
- **Evidence packaging** (`compliance/evidence.rs`) — fire-and-forget on completion and failure; bundles step history + tool outputs into an evidence record
- **Citations** (`compliance/citations.rs`) — records source attribution per step for auditability
- **Human reviews** (`compliance/reviewer.rs`) — review queue for plan approval, credential gaps, SLA breaches, and `FailureAction::EscalateToHuman` triggers

---

## Example walkthroughs

---

### Example 1 — Lead enrichment agent (sales_revops)

**Scenario:** A sales ops manager wants an agent that runs every Monday morning, enriches the week's new Salesforce leads with company info and recent news, drafts a personalised outreach email per lead, and posts a summary to Slack when done.

---

**Step 1 — You click "New Agent"**

`PlanModeChat` opens. No cancel button. First message:

> _What should this agent do?_

You type: _"Every Monday enrich our Salesforce leads — pull company info and recent news, skip leads with no email, draft a personalised outreach email per lead and save it. Also notify #sales-ops when done."_

---

**Step 2 — IntentExtractor runs in two passes**

Pass 1 (compact capability directory) infers categories/candidates.  
Pass 2 (targeted detail for inferred categories/candidates) refines exact tool/connector preferences.

Final output includes:
```json
{
  "category": "sales_revops",
  "preferred_tool_categories": ["web", "communication", "data"],
  "preferred_tools": ["web_search_tool", "file_write"],
  "needed_connector_categories": ["crm", "communication"],
  "candidate_connectors": ["salesforce", "slack"],
  "missing_capabilities": [],
  "workflow_outline": [
    "fetch new leads from CRM",
    "enrich each lead with recent company data",
    "draft outreach per lead",
    "notify channel with run summary"
  ],
  "trigger_hint": "schedule",
  "trigger_cron": "0 9 * * 1",
  "trigger_confidence": "medium",
  "trigger_confirmation": "I guessed: every Monday at 9am UTC — is that right?",
  "output_hint": "email_draft",
  "output_destination_hint": "workspace/drafts/",
  "output_questions": [],
  "multi_role_suggested": true,
  "multi_role_reason": "lead enrichment and Slack notification have different triggers and outputs",
  "responsibilities": [
    { "name": "Lead Enrichment & Drafts", "trigger_hint": "schedule" },
    { "name": "Slack Notification", "trigger_hint": "workforce_event" }
  ]
}
```

`generate_steps()` builds queue: RoleSplit → Trigger → domain steps (write_back, enrichment_sources, outreach_mode, skip_criteria) → CompletionCriteria.

---

**Step 3 — CapturingClarifications (5 turns)**

| Turn | Question | Your answer | Field written |
|---|---|---|---|
| 1 | Two responsibilities detected — one role or split? | B — separate | `RoleSplit` → pending_roles stashed |
| 2 | Every Monday 9am UTC — right? | Yes but 8am London | `TriggerDef { cron: "0 8 * * 1", timezone: "Europe/London" }` |
| 3 | Write back to Salesforce automatically or tasks only? | Update lead Description | `GuidelineRule::always("Update Description field after enrichment")` |
| 4 | Enrichment: web search, LinkedIn, or CRM only? | Web search + LinkedIn | `GuidelineRule::always("Use web_search and LinkedIn")` |
| 5 | Skip criteria? | Missing email, already in active Outreach sequence | Two `FailureRule`s: SkipAndLog + SkipSilently |

Then CompletionCriteria turn: you say _"auto"_ → `default_completion_criteria()` generates: all leads processed, drafts in workspace/drafts/, errors.txt written.

Domain skill execution brief injected: "Read before write", "Never overwrite CRM notes", "On Salesforce query fail → retry once".

---

**Step 4 — Reviewing**

```
Agent: Lead Enrichment Bot
Role:  Lead Enrichment & Drafts
Trigger: 0 8 * * 1 (Europe/London)
Connectors: salesforce, slack
Output: workspace/drafts/

RULES:
1. Update lead Description field after enrichment
2. Use web_search and LinkedIn for enrichment
3. Save drafts to workspace/drafts/ — never send directly
4. [BEFORE salesforce.update_record] Read current record first

FAILURE HANDLING:
1. Skip leads with no email → Skip, log to workspace/errors.txt
2. Skip leads in active Outreach sequence → Skip silently
3. [salesforce.query fails] → Retry once

DONE WHEN ALL OF:
1. [ ] All leads from salesforce query processed
2. [ ] Output files written to workspace/drafts/
3. [ ] workspace/errors.txt written
```

Before saving, you can click Run test. The draft runs deterministic preflight + sandbox validation from the saved workflow_outline. If it fails, the Revise plan action feeds the structured result back into plan mode and reopens the draft; if it passes, you save.

You say _"yes"_ → saved. Plan mode reopens for Role 2 (Slack Notification). Now with the updated pipeline, 3 turns instead of 2:

| Turn | Question | Your answer | Field written |
|---|---|---|---|
| 1 | Which role triggers this? (existing: Lead Enrichment & Drafts) | Lead Enrichment & Drafts | `workforce_event_filter = "role_name == 'Lead Enrichment & Drafts' AND status == 'completed'"` |
| 2 | What data do you need from that run? | The count of leads processed | `input_mapping = { "lead_count": "$.output_data.processed" }` |
| 3 | Where should the output go? | #sales-ops | `OutputDestination::Channel { connector: "slack", channel: "#sales-ops" }` |

Review card shows: _"Trigger: runs after 'Lead Enrichment & Drafts' completes"_. Done.

---

**Step 5 — Monday 8am London**

Scheduler fires. GoalInstance created. Executor runs:

```
1. salesforce.query_records — fetch leads created this week
2. [for each lead] web_search "{company} recent news"
3. [for each lead] file_write workspace/drafts/{lead_id}.md
4. salesforce.update_record — write enrichment to Description
5. file_write workspace/errors.txt — log skipped leads
```

`step_outputs` accumulates: `{ step: 1, processed: 47, connectors: [] }`, `{ step: 4, processed: 44, connectors: ["salesforce"] }`.

`check_completion_criteria` runs:
- `AllItemsProcessed`: ✓ 47 items processed
- `OutputExists workspace/drafts/`: ✓ found
- `ErrorsLogged workspace/errors.txt`: ✓ found

`GoalInstanceStatus::Completed`. Savings estimated: 47 leads × 8 min/lead × $48/hr = **$300.80** saved. AI cost: **$0.62**. ROI: **485×**.

Role 2 fires via WorkforceEvent → Slack posts: _"Lead enrichment complete: 47 leads processed, 3 skipped, 47 drafts in workspace/drafts/"_.

---

### Example 2 — Support ticket response agent (customer_support)

**Scenario:** A customer success manager wants an agent that drafts a reply whenever a new Zendesk ticket is created, searches the help docs first, escalates billing disputes to a human, and always drafts for approval rather than auto-sending.

---

**Step 1 — Intent**

You type: _"When a new Zendesk ticket comes in, search our help docs at docs.acme.com and draft a reply. Billing disputes should always go to a human. Drafts only — never send automatically."_

IntentExtractor (pass 1 + pass 2 refinement) returns:
```json
{
  "category": "customer_support",
  "trigger_hint": "webhook",
  "trigger_source": "zendesk",
  "trigger_event": "ticket_created",
  "trigger_confidence": "high",
  "multi_role_suggested": false
}
```

Confidence is high — trigger step skipped. Output destination hint = `"email_draft via zendesk"`. Queue: output destination → domain steps → CompletionCriteria.

---

**Step 2 — CapturingClarifications (5 turns)**

| Turn | Question | Your answer | Field written |
|---|---|---|---|
| 1 | What is the URL of your help documentation? | docs.acme.com | `GuidelineRule::always("Search docs.acme.com before composing reply")` |
| 2 | Which Slack channel or email should escalations go to? | #cs-escalations | `FailureRule { EscalateToHuman { notify_channel: "#cs-escalations" } }` |
| 3 | First-response SLA? | 1 hour | `AgentConstraint: "First response within 1 hour"` |
| 4 | Draft mode? | Always draft, never auto-send | `GuidelineRule::always("Always save as draft in Zendesk — never publish without human review")` |

CompletionCriteria auto: ticket draft written, reply attached to ticket.

---

**Step 3 — Trigger fires**

New ticket created in Zendesk → `connector_inbound` handler matches the role's `event_filter: "ticket_created"` → GoalInstance created with ticket payload as `input_data`.

Executor runs:
```
1. web_fetch docs.acme.com/search?q={ticket_subject}
2. Compose draft reply using knowledge base content
3. zendesk.create_ticket_reply — attach draft (draft: true, not published)
```

`check_completion_criteria`:
- `RecordUpdated { connector: "zendesk" }`: ✓ write found in step_outputs

Ticket now has a draft reply waiting for human approval in Zendesk.

If the ticket subject contains "billing" or "invoice" → `apply_failure_action_override` matches the EscalateToHuman rule → submits a review request → agent run aborts cleanly → human notified on #cs-escalations.

---

### Example 3 — Chatting with your agent after it runs

**Scenario:** It's Tuesday. The Monday enrichment run completed. You open the Lead Enrichment Bot in the sidebar, click on the role, and see the run row: _"Completed — +5.0h saved — 2h ago"_. You have questions.

---

**You click the run row**

`RunDetailDrawer` opens. You see:

```
✓ Completed
AI cost: $0.62    Human hours: 5.0h    ROI: 485×

COMPLETION CRITERIA
✓ All 47 leads processed from salesforce query            PASS
  ✓ 47 items processed
✓ Output files written to workspace/drafts/               PASS
  ✓ Found output at /workspace/agent-xyz/drafts/
✓ workspace/errors.txt written                            PASS
  ✓ Error log written at /workspace/agent-xyz/errors.txt

STEP OUTPUTS
Step 1  ✓  47 items processed
Step 4  ✓  44 items processed  via salesforce
```

You expand the errors criterion and see: _"3 leads skipped — no email on file"_.

---

**You click "Chat" on the role card**

`RoleChatDrawer` opens. Greeting:

> _I'm looking at **Lead Enrichment & Drafts**._
>
> _What it does: Enrich Salesforce leads and draft outreach emails_
> _Trigger: Schedule `0 8 * * 1` (Europe/London)_
> _Connectors: salesforce, slack_
>
> _Recent runs: 3 completed, 0 failed — last run: Mon 17 Mar at 08:01 — completed_
>
> _What would you like to know or change?_

---

**Turn 1 — You ask a question**

> _"Why were 3 leads skipped?"_

> _The 3 skipped leads had no email address in Salesforce. The skip rule you set is working correctly — they were logged to workspace/errors.txt with reason "no_email". If you'd like, I can add a step to search LinkedIn for their email before skipping — want me to propose that change?_

---

**Turn 2 — You ask about the output**

> _"Are the drafts any good? What format are they in?"_

> _The drafts are saved as markdown files in workspace/drafts/, one per lead. They follow the format you described: personalised opening based on company news, value prop, and a call to action. The last run saved 47 files averaging 180 words each._
>
> _The output spec says markdown — if you'd prefer plain text or HTML for your email client, I can change that. Want me to update the output format?_

---

**Turn 3 — You request a change**

> _"Change the schedule to run Tuesday and Thursday at 7am, not just Mondays"_

The LLM detects a schedule change intent and replies:

> _I'll update the schedule to run Tuesday and Thursday at 7am London time — cron: `0 7 * * 2,4` (Europe/London). That means 2 runs per week instead of 1._

A **confirmation card** appears:

```
┌─────────────────────────────────────────┐
│  ↗  Change schedule                      │
│  Tue + Thu at 7am London (0 7 * * 2,4)  │
│                                          │
│  [Apply change]        [Dismiss]         │
└─────────────────────────────────────────┘
```

You click **Apply change** → `roleChat.apply()` → `RoleChangeType::Schedule` → `upsert_agent_role` with new cron. The sidebar reflects the updated trigger immediately.

---

**Turn 4 — You edit failure rules directly**

Below the chat, the `FailureRuleEditor` panel shows your current rules:

```
[Skip & Log]               Skip leads with no email
[Skip silently]            Skip leads in active Outreach sequence
[Retry ×1]  [salesforce]  salesforce.query fails
```

You click **Add rule** → fill in: _"If web_search returns no results for a company"_ → action: **Skip & Log** → tool scope: **web_search** → click **Add rule**.

The rule is added immediately via `AddFailureRule` change type — no LLM involved, no confirmation card needed for direct edits. The role is saved. You see the new rule appear in the list.

---

**Turn 5 — You ask about performance over time**

> _"How has the run time been changing? Are we getting slower?"_

> _Looking at the last 4 runs: Mon Mar 17 (47 leads, 4m 12s), Mon Mar 10 (38 leads, 3m 44s), Mon Mar 3 (51 leads, 4m 31s), Mon Feb 24 (29 leads, 2m 58s). Run time scales with lead count — roughly 5.4 seconds per lead. Nothing suggests degradation. The slowest part is usually the web_search step, which averages 2.1s per lead._

---

**This is the "AI employee" model**

The role chat is the interface for managing your AI employee the same way you'd manage a human one: ask why something happened, ask what they did, request a schedule change, add a new rule for an edge case you just discovered. The difference is that changes take effect in seconds, the employee never forgets the rule, and every change is confirmed before it applies.


---

## Custom connections — global database, REST APIs, MCP servers

Beyond the 22 built-in connectors, tenants register their own connections that are available to any agent they build. These are tenant-global — registered once in Settings, usable in any role.

### Registering connections

**Settings → Connectors → Custom connections** has three add flows:

**Database** — paste a Postgres connection string, set a name, choose read-only or read-write. Narayan opens a real connection, counts tables, stores the connection string encrypted in `connector_installs`. Any role can then say "use the prod database" and plan mode will name it in `execution_guidelines.rules` as `"Use tool external_db with db='prod_db'. Start with operation='schema'."`.

**REST API** — provide base URL, auth type (Bearer/API key/None), token, and a test path. Narayan does a GET to verify the endpoint responds. Any role can say "hit our backend API" and the `external_api` tool handles all HTTP verbs, loading the stored token for auth.

**MCP server** — provide the server URL and an optional bearer token. Narayan calls `tools/list` on the MCP server and shows the discovered tool names. These appear as named connectors in plan mode: `"name='acme-data-tools' — 8 tools available"`.

All three show up in the `ConnectorsTab` under "Your connections" with type labels, connection status, and summary. Deleting a connection removes it from `tenant_connectors` and clears the stored token.

### Tenant custom deterministic tools (WASM)

Tenants can also register WASM modules via Settings (`POST /tenant-wasm-tools`). These are validated, resource-capped, and audit-logged at registration time. Plan mode can then infer/select exact WASM tool names (`candidate_wasm_tools`) and persist them as role scope (`wasm_tool:<name>`), so runtime can execute only pre-approved custom logic through `run_registered_wasm`.

### How plan mode uses custom connections

`IntentExtractor` pass 1 receives a broader `CAPABILITY DIRECTORY` block. It includes:
- compact tool category quick maps (names only, no full schema dump)
- built-in connector categories with connector names and status (`installed` vs `available`)
- tenant custom connections by name/type/summary

Tenant custom connection section looks like:

```
Databases (use external_db tool, reference by name):
  - name='prod_db' — Production PostgreSQL with leads, accounts, and orders tables

REST APIs (use external_api tool, reference by name):
  - name='acme_backend' — Internal REST API for order management

MCP servers (available as connector tools):
  - name='acme-data-tools' — 8 tools: query_orders, list_customers, ...
```

Then pass 2 receives targeted detail only for inferred categories/candidates (for example, selected tool categories plus connector operation summaries).

When a user says _"query our database for orders over $10k"_, the LLM extracts `uses_external_db: "prod_db"` and `ConnectorResolver` writes `tool_overrides: ["external_db:prod_db"]` into the role. At execution time, the executor injects `tenant_id` into tool args so `external_db` can look up the stored credentials without the LLM ever seeing the connection string.

### Tool behaviour

**`external_db`** — operations: `schema` (tables + columns + row counts), `query` (SELECT enforced, 1000-row cap, 60s timeout), `execute` (writes only if `allow_writes=true` was set at registration), `table_preview`, `explain`. Row data is typed (not stringified). The planner is instructed to call `schema` first to discover the structure before writing queries.

**`external_api`** — all HTTP verbs. GET args become query params; POST/PUT/PATCH args become JSON body. Base URL and auth token loaded from `connector_installs` by `tenant_id`. 30s timeout.

**MCP tools** — routed via `tools/mcp_session.rs`. The `McpSessionTool` maintains a persistent connection per server URL. Tool calls are forwarded as MCP `tools/call` requests. The stored bearer token is attached automatically.

---

## Workforce events — cross-agent chaining

Roles can trigger other roles when they complete. This is how multi-role agents coordinate without polling or external orchestrators.

### How it works

When a role is saved with `TriggerType::WorkforceEvent`, `sync_subscriptions_for_role()` creates a `WorkforceEventSubscription` record:

```
WorkforceEventSubscription {
    id, tenant_id, subscriber_role_id,
    filter: "role_name == 'Lead Enrichment & Drafts' AND status == 'completed'",
    input_mapping: { "lead_ids": "$.output_data.lead_ids" }
}
```

When any GoalInstance completes or fails, `dispatch_workforce_event()` in `events/workforce.rs` fires. It loads all active subscriptions for the tenant, evaluates each filter expression against the event payload, and creates a new GoalInstance for each matching subscriber role. The `input_mapping` extracts fields from the triggering run's output and passes them as `input_data` to the new run.

### How plan mode configures workforce triggers

Plan mode fully configures workforce triggers through the `ClarificationStep` pipeline. When `trigger_hint == "workforce_event"`:

1. **`WorkforceEventFilter` step** — asks "Which role triggers this?" and shows existing role names. Answer becomes `workforce_event_filter = "role_name == 'Lead Enrichment & Drafts' AND status == 'completed'"`. If the user says "any role", the filter is `"status == 'completed'"`.

2. **`WorkforceEventInputMapping` step** — asks "What data do you need from that run?" `infer_input_mapping()` converts the answer to a JSONPath mapping: `{ "lead_ids": "$.output_data.lead_ids" }`. Stored as `trigger.input_mapping`.

3. **`DependsOnRole` step** (optional) — asks about strict within-agent ordering. Stores a name hint resolved to a real role UUID at `save()` time.

The review summary shows the resolved trigger: _"runs after 'Lead Enrichment & Drafts' completes"_ — not the generic "runs after another role".

### Cross-agent chaining example

```
Agent: Revenue Pipeline
├── Role A: Lead Enrichment          trigger: Schedule (Mon 8am)
│                                    output: workspace/drafts/ + lead_ids
│                                    ↓ WorkforceEvent on Complete
├── Role B: Slack Notification       trigger: WorkforceEvent (A completes)
│                                    input_mapping: { "lead_count": "$.output_data.processed" }
│                                    output: #sales-ops message
│                                    ↓ WorkforceEvent on Complete
└── Role C: Weekly Summary Report    trigger: WorkforceEvent (B completes)
                                     output: workspace/weekly-summary.md
```

Each role runs in isolation with its own GoalInstance, completion criteria, and savings estimation. Failures in one role don't cascade — each subscription fires independently.

### Within-agent dependencies

`TriggerDef.depends_on_role_id` can reference another role in the same agent. This creates a strict ordering within an agent without needing WorkforceEvent subscriptions. Set via the `DependsOnRole` clarification step in plan mode — name hint resolved to real ID at save time.

### Delegation (within a run)

During a single run, a step can call the `delegate` tool to spawn a child agent for a sub-task. The parent run suspends and waits for the child. This is used for parallel work — e.g. enriching 50 leads by spawning 5 child agents of 10 leads each. `StepOutcome::Delegating { child_ids }` signals this to the worker, which tracks child completion before resuming the parent.

---

## Plan approval mode — credential gap handling

Before a plan executes, the system checks whether all required credentials are available. If any are missing, the run pauses and the user is asked to connect them via a UI card.

### Flow

```
Plan created → LlmPreflight.check()
    ↓
credential_requirements.scan(plan, installed_connectors)
    → finds: ["salesforce OAuth token missing"]
    ↓
state.mark_plan_approval_needed()
AgentEvent::PlanApprovalNeeded { agent_id, plan, credential_gaps }
    ↓
Frontend: PlanApprovalCard renders
    - Shows the planned steps
    - Shows each credential gap with a "Connect in Settings" action button
    - User connects → clicks Submit → run resumes
    ↓
Executor re-runs with credentials now available
```

### PlanApprovalCard

Shows the full plan before execution begins, allowing the user to review and approve. Two distinct modes:
- **Credential gap**: blocked — user must connect missing credentials before proceeding
- **Replanning**: plan was revised mid-run — user reviews and approves the new plan

The card sends an SSE event when the user approves, which unblocks the waiting worker.

---

## Multi-role session flow

When plan mode detects multiple responsibilities, the session produces multiple roles on one agent rather than making the user start over.

**During plan mode:** if the user chooses "B — separate roles", remaining `RoleResponsibility` objects are serialised into `draft_agent.memory_ref` as `|pending_roles:[...]`. After `save()`, the frontend reads this field and immediately reopens `PlanModeChat` for role 2 on the same agent, pre-populated with the responsibility name. This repeats until all pending roles are configured.

**Result:** one `AgentDefinition` with N `AgentRole` records, each with its own trigger, guidelines, and criteria. All roles appear in `AgentPage` under the same agent card. The sidebar shows the agent with role count and status.

---

## Cognitive control loop

`CognitiveControlLoop` in `cognition/control_loop.rs` tracks step count and wall-clock time within a single run. It enforces:

- **`max_steps`** (default 50) — if the plan grows beyond this (e.g. through replanning), the run is aborted with `Infeasible`
- **`timeout_secs`** (default 300) — if a run exceeds 5 minutes total, it is aborted

These limits are configurable via `AgentLoop::with_limits()` and can be overridden per tenant via `execution_limits` on `AgentRole`.

---

## WASM tools

WASM-related tools in `tools/`:

- **`wasm_compile`** — compiles Rust or AssemblyScript source to `.wasm` using a sandboxed build environment
- **`wasm_inspect`** — reads a `.wasm` file and lists its exported functions, memory, and imports
- **`wasm_call`** — calls a named export in a loaded `.wasm` module with typed args
- **`wasm_exec`** — executes a `.wasm` file with WASI support for file/stdio access within the workspace
- **`run_registered_wasm`** — executes tenant-registered WASM modules with strict per-tool permissions and resource limits

For production role execution, the preferred path is `run_registered_wasm` with plan-mode approval:
- register and test module first in tenant settings/plan mode
- persist role scope as `wasm_tool:<name>`
- runtime executes only approved names

Runtime dynamic custom-tool creation is intentionally blocked; this keeps execution deterministic, auditable, and policy-bound.

---

## Knowledge graph + vector memory

### In-run knowledge graph (`knowledge/graph.rs`)

An in-memory directed graph built during a run. Each successful step's findings are parsed by `extract_entities()` and added as `(entity_name, entity_type)` nodes. The evaluator's `key_findings` are also added. The graph persists for the duration of the run and is available to the executor for entity-aware tool calls (e.g. referencing a company name found in step 2 in step 5 without the LLM having to re-read the full context).

### pgvector semantic memory (`memory/`)

Step summaries and findings are embedded and stored in pgvector. On each step, `vector_search` is available as a tool for the executor to retrieve relevant prior context from other runs — enabling agents to accumulate knowledge across weeks of operation. `memory_store`, `memory_recall`, and `memory_forget` tools provide explicit read/write/delete access.

---

## Skill evolution

`skill_evolution/evolution.rs` implements self-improving skills. After a successful step that used a skill:

1. Successful tool outputs from that step are extracted (up to 2 snippets, 80 chars each)
2. `evolve_skill()` generates a new version of the skill with the improvement snippets added to the last step's description
3. The updated skill is registered back into `SkillRegistry`

This means a skill that initially says "query the database" evolves over runs to say "query the database — last successful query: SELECT lead_id, company FROM leads WHERE created_at > NOW() - INTERVAL '7 days'". The skill becomes more specific over time based on what actually worked.

---

## Debug and replay

`debug/recorder.rs` — `AgentRecorder` captures a full execution trace per run: every step with its plan step, tool calls, tool results, evaluator verdict, and timing. Stored as a structured log.

`debug/replay.rs` — `AgentReplay` can re-execute a recorded trace against a different model, different tool registry, or with modified parameters without hitting real external APIs. Used for post-mortem analysis and regression testing when a run produces unexpected results.


---

## Pre-built templates (`agent/templates.rs`)

20 `RoleTemplate` structs covering three personas — teams, founders, and personal use. Each template completely pre-answers the `IntentExtractor`'s questions, pre-builds the `AgentRole` with typed guidelines, failure rules, and completion criteria, and lists only the 0–3 questions the user must answer themselves.

### For teams

| # | ID | Name | Trigger | Connectors | Ask steps |
|---|---|---|---|---|---|
| 1 | `invoice_processor` | Invoice Processor | Gmail webhook | gmail, quickbooks | approval_threshold, output_dest |
| 2 | `support_ticket_responder` | Support Ticket Responder | Zendesk webhook | zendesk | docs_url, escalation_channel |
| 3 | `contract_risk_reviewer` | Contract Risk Reviewer | User message | — | output_dest |
| 4 | `employee_onboarding` | New Employee Onboarding | Greenhouse webhook | greenhouse, gmail | output_dest |
| 5 | `compliance_deadline_monitor` | Compliance Deadline Monitor | Schedule Mon–Fri 8am | gmail, slack | db_name, escalation_channel |
| 6 | `sales_pipeline_health` | Sales Pipeline Health | Schedule Mon 8am | salesforce, gmail | inactivity_days, output_dest |
| 7 | `competitor_intelligence` | Competitor Intelligence Brief | Schedule Fri 9am | slack | competitor_names, slack_channel |

### For founders

| # | ID | Name | Trigger | Connectors | Ask steps |
|---|---|---|---|---|---|
| 8 | `investor_update_writer` | Investor Update Writer | Schedule Fri 5pm | gmail | db_name, metrics_table, investor_email |
| 9 | `churn_early_warning` | Customer Churn Early Warning | Schedule Mon–Fri 9am | gmail | db_name, inactivity_days |
| 10 | `applicant_screener` | Job Applicant Screener | Greenhouse webhook | greenhouse, gmail | job_requirements, output_dest |
| 11 | `pre_demo_brief` | Pre-Demo Sales Brief | HubSpot meeting booked | hubspot | delivery_channel |
| 12 | `expense_analyser` | Monthly Expense Analyser | Schedule 1st of month 9am | quickbooks, gmail | output_dest |
| 13 | `code_review_assistant` | Code Review Assistant | GitHub PR opened | github, slack | slack_channel |

### For personal use

| # | ID | Name | Trigger | Connectors | Ask steps |
|---|---|---|---|---|---|
| 14 | `tax_document_collector` | Tax Document Collector | User message | gmail | tax_year |
| 15 | `job_application_tracker` | Job Application Tracker | User message | gmail | — |
| 16 | `weekly_research_brief` | Weekly Research Brief | Schedule Mon 8am | gmail | research_topic, output_email |
| 17 | `document_explainer` | Document Plain-English Explainer | User message | — | — |
| 18 | `options_researcher` | Options Researcher | User message | gmail | — |
| 19 | `news_monitor` | News Monitor and Alerter | Schedule Mon–Fri 8am | gmail | monitor_subject, output_email |
| 20 | `meeting_prep` | Meeting and Interview Prep | User message | — | — |

### What each template pre-configures

Every template carries the complete execution contract for its workflow. Example — `invoice_processor`:

**Guidelines (typed `GuidelineRule`):**
- `[BEFORE pdf_read]` Only process emails with PDF attachments
- `ALWAYS` Extract: vendor, invoice number, amount, due date, line items
- `ALWAYS` Match invoice against open POs in QuickBooks before posting
- `ALWAYS` Never post to QuickBooks without a matching PO or explicit approval
- `[AFTER quickbooks]` Write confirmation to workspace/processed.txt
- `ALWAYS` Flag invoices over $5,000 for human approval

**Failure rules (typed `FailureRule`):**
- Invoice has no matching PO → `SkipAndLog` to workspace/errors.txt `[quickbooks]`
- Duplicate invoice number → `SkipAndLog`
- Invoice over $50,000 → `EscalateToHuman` → #finance-alerts
- QuickBooks timeout → `RetryOnce` `[quickbooks]`

**Completion criteria (typed `CompletionCriterion`):**
- `RecordUpdated { connector: "quickbooks" }` — invoice posted
- `ErrorsLogged { log_hint: "workspace/errors.txt" }` — mismatches recorded

**Segment services activated automatically (finance_accounting):**
PII redaction, citation recording, evidence packaging, human review queue.

This is the same depth for all 20 templates — not placeholder text, not generic rules. Each one was designed for the exact failure modes and output requirements of that specific workflow.

### How `build_template_clarification_steps` works

Maps the `ask_steps` string array to typed `ClarificationStep` objects. 16 known step names, each producing a specific targeted question with the right `StepField` so `parse_and_apply` writes to the correct field on the draft role. Unknown step names are silently skipped — safe to add new step names without breaking existing templates.

### Adding a new template

1. Add a new `RoleTemplate` entry to the `TEMPLATES` static array in `agent/templates.rs`
2. Implement `build_role` with typed guidelines/failure rules/criteria for that workflow
3. Implement `intent()` returning the pre-answered intent JSON
4. List any new `ask_steps` names in `build_template_clarification_steps` with their question and `StepField`
5. Deploy — no migration, no DB change


---

---

# Builder's handbook — context for the next session

This section exists so that a future Claude instance, a new engineer, or the original author returning after time away can understand not just *what* was built but *why*, *how the pieces connect*, and *where the sharp edges are*. Read this before touching anything.

---

## How this codebase was built — the full arc

Narayan started as a basic agent loop with a plan/execute/evaluate cycle. Over many sessions it grew into a full B2B agent platform. The additions were not random — each one solved a specific problem that the previous version exposed. Here is the sequence:

**Session 1-2:** Basic `AgentLoop` (plan → execute → evaluate), `WorkerPool`, `PostgresStore`, JWT auth, basic connectors.

**Session 3-4:** Plan mode — the conversational setup flow. The key insight was that users shouldn't configure YAML or JSON — they should describe what they want in one sentence and the system should derive the full role config. This led to `IntentExtractor` + `ClarificationStep` pipeline (typed, sequential, no free-text blob parsing). The current implementation runs `IntentExtractor` in two passes: compact capability directory first, then targeted detail refinement.

**Session 5-6:** `ExecutionGuidelines` typed contract. Before this, guidelines were `Vec<String>`. The switch to typed `GuidelineRule` / `FailureRule` / `CompletionCriterion` was the most important architectural decision in the project — it made the planner prompt, the evaluator prompt, and the completion check all derive from the same source of truth.

**Session 7-8:** Connector system — 22 built-in connectors, `external_db`, `external_api`, MCP. Custom connections injected into plan mode context so the LLM knows what the tenant has available before the first question.

**Session 9-10:** Gap fixes — `PartiallyComplete` status, `CriterionResult` typed completion check, `SkipAndLog` actually writing the log file, `items_processed` in `StepResult`, `FailureAction` override before the LLM evaluator, savings quality gate.

**Session 11-12:** Novel features — `RunDetailDrawer` (criteria checklist per run), `FailureRuleEditor` (inline in role chat), `check_completion_criteria` returning typed results written to `goal_instance.result["criteria_checks"]`.

**Session 13:** Plan mode connected to everything — `WorkforceEventFilter` + `WorkforceEventInputMapping` + `DependsOnRole` steps so workforce chaining is configured through plan mode, not manually. `active_services_for_category()` discloses segment services in review card.

**Session 14:** 20 pre-built templates in `agent/templates.rs` — static `RoleTemplate` structs with `build_role` fn pointers. Template fast-path in `start_plan_mode_session` skips `IntentExtractor` entirely, enters `CapturingClarifications` with 0-3 questions.

**Session 15:** Role-policy grounding pass — persisted `role_category`, defaulted persona/memory scope/execution_limits by category, two-pass intent capability grounding, execution-hint hygiene (`step:` workflow priorities + stale-hint cleanup), safer connector clarification matching, and bounded per-category tool expansion in selector/runtime prompts.

**Session 16:** Plan mode core + deterministic test mode + repair reuse — `workflow_outline` became the execution contract, plan test now runs preflight + sandbox without the LLM planner, and goal fingerprinting plus session-local repair snapshots keep the latest good draft reusable for the same normalized goal.

---

## The three things that make this different from other agent platforms

**1. ExecutionGuidelines is a contract, not a prompt.**
Every other platform puts guidelines in a free-text system prompt field. Here, guidelines are typed structs — `GuidelineRule { text, tool_scope, phase }`, `FailureRule { text, tool_scope, action: FailureAction }`, `CompletionCriterion { description, check: CompletionCheck }`, and `workflow_outline: Vec<WorkflowStep>`. This means:
- The planner prompt is generated deterministically from the struct, not written by hand
- The evaluator sees `DONE WHEN ALL OF:` with checkboxes, not a paragraph
- Completion is checked mechanically (file exists? connector wrote?) not by LLM judgment
- The `FailureRuleEditor` UI can add/remove typed rules without the LLM
- Templates pre-fill the exact right rules for each workflow

**2. FailureActions fire before the LLM evaluator.**
`apply_failure_action_override()` in `loop.rs` checks the role's `failure_handling` rules against every step failure *before* asking the LLM whether to retry or abort. `RetryOnce` fires deterministically on the first failure regardless of what the LLM thinks. `SkipAndLog` writes to `workspace/errors.txt` and sets `state.metadata["errors_logged"] = true` so the `ErrorsLogged` completion criterion passes. This is why the two are connected — if `SkipAndLog` didn't set that flag, `check_completion_criteria` would incorrectly mark the run as `PartiallyComplete` even when it succeeded.

**3. Plan mode is a typed pipeline, not a conversation.**
`generate_steps()` returns a queue of `ClarificationStep` objects. Each step has a `StepField` enum variant that maps directly to one field on the draft role. `parse_and_apply()` is a match statement — no regex, no LLM parsing. The queue is serialised as JSON in `session.pending_steps` and persisted across HTTP requests. The result is that plan mode is deterministic and testable — every question has exactly one answer that writes exactly one field. It also has a deterministic test/revise loop and goal-fingerprint reuse for repeated goals.

---

## The most important file relationships

```
agent/definition.rs          ← THE source of truth
    AgentRole
    ExecutionGuidelines       ← rules + failure_handling + priorities + completion_criteria
    TriggerDef                ← trigger_type + cron + workforce_event_filter + input_mapping + depends_on_role_id
    GoalInstanceStatus        ← Pending → Running → Completed | PartiallyComplete | Failed | Cancelled
    PlanModeSession           ← phase + conversation + attachments + attachment_context + session_workspace + intent_cache + pending_steps + draft_role + goal_fingerprint + repair_version

agent/plan_mode.rs            ← plan mode conversation manager
    PlanModeManager::turn()   ← dispatches to handle_intent / handle_clarifications / handle_review
    test()                    ← deterministic preflight + sandbox validation
    revise_from_test_result() ← session-local repair loop using structured test output
    handle_intent()           ← calls IntentExtractor pass 1 + pass 2 refinement, ConnectorResolver, build_step_queue_and_ask
    build_capability_directory() / build_detailed_capability_context() ← staged grounding input for plan mode inference
    apply_execution_hints()   ← stores preferred categories + workflow_outline into ExecutionGuidelines (with stale-hint cleanup)
    compute_plan_mode_goal_fingerprint() ← goal-normalized repair key
    apply_role_policy_defaults() ← category-derived persona/memory_scope/execution_limits defaults
    handle_connector_clarification() ← exact connector-name token matching with explicit disambiguation
    build_step_queue_and_ask()← loads existing_roles from DB, calls generate_steps()
    save()                    ← resolves "name:Role Name" hints to UUIDs, preserves completed snapshot, calls sync_subscriptions_for_role
    build_review_summary()    ← shows trigger description, connectors, services, active_services_for_category()

agent/plan_mode_steps.rs      ← the step pipeline
    generate_steps()          ← intent + category + installed + existing_roles → Vec<ClarificationStep>
    parse_and_apply()         ← StepField match → writes typed field on draft role
    infer_input_mapping()     ← natural language → JSONPath { "lead_ids": "$.output_data.lead_ids" }
    domain_steps_for()        ← 7 categories × 3-5 typed steps each

agent/templates.rs            ← 20 pre-built templates
    RoleTemplate              ← static struct with build_role fn pointer + intent fn pointer
    find_template(id)         ← used by start_plan_mode_session template fast-path

agent/planner.rs              ← deterministic plan construction helpers + planner prompt utilities
    load_role_context()       ← injects role policy context (category, limits, memory scope, tool/category hints)
    Plan::from_workflow_outline() ← builds runtime plan from the saved workflow_outline

agent/executor.rs             ← LLM executor
    load_role_execution_policy() ← injects same role policy into step execution prompting
    execute_step()            ← selector gets role.tools + preferred_tool_categories before heuristic fallback
    run_registered_wasm guard ← enforces role-approved `wasm_tool:<name>` scope, blocks out-of-scope tool_name
    create_workspace_tool     ← hard-blocked at runtime (plan-mode-only onboarding policy)

tools/selector.rs             ← per-step tool budgeter
    select_tools_for_step()   ← honors role.tools + role categories, capped to MAX_TOOLS=20
    MAX_ROLE_CATEGORY_TOOLS   ← per-category cap to prevent broad-category tool flooding
    RUNTIME_BLOCKED_TOOLS     ← excludes runtime-only forbidden tools (e.g. `create_workspace_tool`)

agent/prompts.rs              ← prompt renderers
    ExecutorPrompt::system()  ← includes request_more_tools category quick maps + connector category hints + "no runtime custom tool creation" rule

agent/evaluator.rs            ← step evaluation + completion criteria check
    check_completion_criteria()← returns Vec<CriterionResult> — NOT (bool, String)
    CriterionResult           ← { description, satisfied, check_type, detail }
    LlmEvaluator              ← fast-path for unambiguous success, LLM call for ambiguous

agent/loop.rs                 ← the step state machine — most complex file in the codebase
    run_step()                ← workflow-outline-first sequence (preflight → execute → evaluate → criteria check)
    apply_failure_action_override()← FailureAction dispatch BEFORE evaluator
    EvalVerdict               ← Continue | Retry | Abort | GoalComplete → dispatched in match

agent/savings.rs              ← ROI estimation — fire-and-forget after Complete/PartiallyComplete
    quality_factor()          ← 0.0 (no output) / 0.5 (result exists, no counts) / 1.0 (real counts)
    partial_completion_fraction()← processed/expected from result, default 0.5 if unmeasurable

agent/role_chat.rs            ← conversational role editing
    RoleChangeType            ← 12 variants including AddFailureRule / RemoveFailureRule / SetFailureRules
    apply_change()            ← typed match → modifies role → upsert_agent_role

storage/postgres.rs           ← every DB operation
    update_goal_instance_result()← writes criteria_checks to goal_instance.result JSONB
    update_goal_instance_savings()← writes hours/cost after savings estimation
    plan_mode_sessions rows    ← preserve completed snapshots and repair chain metadata

api/routes.rs                 ← all HTTP handlers
    start_plan_mode_session() ← template fast-path + free-form path
    test_plan_mode_session()   ← deterministic preflight + sandbox validation
    revise_plan_mode_session() ← feed structured test output back into plan mode
    get_goal_instance_detail()← GET /goal-instances/:id — full criteria_checks for RunDetailDrawer
    list_plan_mode_templates()← GET /plan-mode/templates — 20 template metadata

events/workforce.rs           ← cross-role chaining
    dispatch_workforce_event()← fires on GoalInstance complete/fail, evaluates filter, creates new GoalInstance
    sync_subscriptions_for_role()← called in plan_mode.save() — creates WorkforceEventSubscription from trigger
```

---

## The data flow for a template-started run — end to end

```
User clicks "Invoice Processor" template in UI
    ↓
POST /plan-mode/sessions { template_id: "invoice_processor" }
    ↓
find_template("invoice_processor") → RoleTemplate
build_role(agent_id, tenant_id) → AgentRole with:
    - rules: ["Never post without PO", "Flag >$5k", ...]
    - failure_handling: [SkipAndLog, RetryOnce, EscalateToHuman]
    - completion_criteria: [RecordUpdated("quickbooks"), ErrorsLogged("workspace/errors.txt")]
session.intent_cache = tmpl.intent()  ← bypasses IntentExtractor
session.phase = CapturingClarifications
session.pending_steps = [approval_threshold_step, output_dest_step]
    ↓
User answers 2 questions → parse_and_apply() writes to draft role
    ↓
build_review_summary() → shows trigger, connectors, "Active services: PII redaction, Evidence packaging..."
User says "yes" → save()
    ↓
upsert_agent_role() → role stored as JSONB including full ExecutionGuidelines
sync_subscriptions_for_role() → no WorkforceEventSubscription (schedule trigger)
    ↓
── Monday 8am ──
Scheduler fires GoalInstance
Worker pops task → AgentLoop.run_step()
    1. Preflight: check Gmail + QuickBooks credentials installed
    2. Build deterministic plan from workflow_outline → steps: [fetch_email, pdf_read, match_po, post_quickbooks, write_log]
    3. Execute step 1: gmail.get_message() → ToolResult { success: true, output: {...}, processed: 1 }
    4. loop.rs writes step_outputs: { step: 1, processed: 1, connectors: [] } to state.metadata
    5. FailureAction check: result.success = true → no override
    6. EvalVerdict::Continue → advance step
    [steps 2-4 execute similarly]
    5. QuickBooks returns 429 timeout → result.success = false
    6. apply_failure_action_override: matches "QuickBooks timeout" rule → RetryOnce → EvalVerdict::Retry
    7. Retry fires: step re-executes, succeeds
    8. EvalVerdict::GoalComplete triggered on final step
    9. check_completion_criteria(role, state):
        - RecordUpdated("quickbooks"): ✓ step_outputs has connector "quickbooks" + success=true
        - ErrorsLogged("workspace/errors.txt"): ✓ state.metadata["errors_logged"] = true (set by SkipAndLog)
        all_satisfied = true
    10. update_goal_instance_result() writes criteria_checks to DB
    11. state.mark_completed()
    12. spawn_savings_estimation() fire-and-forget:
        quality_factor = 1.0 (processed > 0)
        human_hours = 3 invoices × 12 min × $58/hr = $34.80 saved
        AI cost: $0.04 → ROI: 870×
```

---

## Sharp edges — things that will bite you if you forget them

**`SkipAndLog` MUST set `state.metadata["errors_logged"] = true`.**
The `ErrorsLogged` completion criterion checks this flag. If `SkipAndLog` only writes the file but doesn't set the flag, runs where the workspace doesn't persist (e.g. container restarts) will incorrectly fail the criterion. Both must happen — see `loop.rs: apply_failure_action_override`.

**`items_processed` is in `StepResult`, written to `state.metadata` by `loop.rs`, NOT by the executor.**
The executor returns `items_processed: u64` in `StepResult` because it holds `&AgentState` (immutable). `loop.rs` holds `&mut AgentState` and writes it to `step_outputs`. If you add a new tool that returns item counts, make sure the output has a `count`, `processed`, `total`, or `rows` field — the executor scans for these.

**Templates use fn pointers — they can't serialise/deserialise.**
`build_role: fn(agent_id: &str, tenant_id: &str) -> AgentRole` and `intent: fn() -> serde_json::Value` have `#[serde(skip)]`. The template metadata (id, name, description, etc.) serialises for the API response, but the functions don't. Never try to store a `RoleTemplate` in the database — reconstruct the role by calling `build_role()` at request time.

**`depends_on_role_id` is stored as `"name:Role Name"` during plan mode, resolved to UUID in `save()`.**
If you see `depends_on_role_id = "name:Lead Enrichment & Drafts"` in a draft role, that's correct — it's a hint that gets resolved. If it's still a name string in a saved role (not during a session), something went wrong in the `save()` resolution block.

**`workforce_event_filter` must be a valid filter expression.**
`dispatch_workforce_event()` evaluates `"role_name == 'X' AND status == 'completed'"`. The filter parser is simple — it handles `==`, `AND`, single-quoted string values. It does not handle `OR`, `!=`, or nested expressions. Keep filters simple.

**`check_completion_criteria` returns `Vec<CriterionResult>`, NOT `(bool, Option<String>)`.**
This was changed from the older return type. Any call site that destructures `(bool, String)` is outdated. The new return is `(bool, Vec<CriterionResult>)` — the bool is `all_satisfied`, the vec has per-criterion detail. Both the `Complete` and `PartiallyComplete` paths in `loop.rs` use the vec to write `criteria_checks` to the goal instance result.

**`savings_estimation` fires for BOTH `Complete` and `PartiallyComplete`.**
Worker.rs handles both in separate arms but both call `spawn_savings_estimation`. Partial runs are pro-rated by `partial_completion_fraction()`. If you add a new `StepOutcome` that represents successful-but-degraded execution, add savings estimation there too.

**`active_services_for_category()` is hardcoded in `plan_mode.rs`.**
It returns what *should* be active based on the segment architecture in `src/segments/`. If you add a new segment service (e.g. audio redaction for `hr_people_ops`), update both the segment plugin AND `active_services_for_category()` — they're not automatically in sync.

---

## The 8 failing tests — what they are and why they're safe to fix

All 8 are test infrastructure issues from the `StepResult` field additions and `AgentLoop.with_store()` wiring. None represent broken production logic.

**6 executor tests** — all panic at the same mock response queue `vec.remove(0)` on empty vec. The mock pops responses one per LLM call. Our changes cause one extra call path (items_processed extraction reads tool outputs). Fix: add a fallback default response when the queue is empty rather than panicking.

**1 evaluator test** — `"STEP COMPLETE"` != `"goal complete"`. The test expects `sanitize_final_answer_candidate` to strip the `"STEP COMPLETE"` suffix and fall through to the `"goal complete"` default. The fast-path now uses `final_answer_candidate` directly without sanitising. Fix: run it through `sanitize_final_answer_candidate` in the fast-path.

**1 loop test** — `expected continue, got PlanApprovalNeeded`. The `AgentLoop::with_store()` builder was added. This test constructs `AgentLoop` without a store (`self.store = None`). Some code path now behaves differently when `store = None`. Fix: check if the test needs `.with_store(mock_store)` or if the state needs `AgentStatus::Running` set explicitly to bypass preflight.

---

## Where to look when something goes wrong

| Symptom | Where to look |
|---|---|
| Run marks PartiallyComplete unexpectedly | `check_completion_criteria` in `evaluator.rs` — check which criterion failed and why. Look at `state.metadata["step_outputs"]` and workspace path. |
| SkipAndLog fires but ErrorsLogged criterion fails | `apply_failure_action_override` in `loop.rs` — confirm `state.metadata["errors_logged"] = true` is being set AND the log file is being written to the right path. |
| Savings estimation gives 0 credit | `quality_factor()` in `savings.rs` — `gi.result` is probably null or empty. Check that the executor is writing `count`/`processed` to tool outputs. |
| Plan mode asks redundant questions | `generate_steps()` in `plan_mode_steps.rs` — check `trigger_confidence` and `output_destination_hint` from `IntentExtractor`. High confidence + non-empty hint = step skipped. |
| WorkforceEvent trigger fires on wrong role | `workforce_event_filter` on the subscription in `WorkforceEventSubscription`. Check what was set during plan mode — it should be `"role_name == 'X' AND status == 'completed'"`. |
| Template fast-path skips to review immediately | `ask_steps` array on the template is empty — intended. Templates with zero unknowns jump straight to review. |
| Role chat FailureRuleEditor changes not persisting | `sessionId` is null when `apply` is called — the session may not have started yet. The guard in `RoleChatDrawer` only calls `roleChat.apply()` if `sessionId` is set. If session failed to start, rules won't save. |
| `depends_on_role_id` is still a name string after save | The `save()` resolution block couldn't find the named role. Check that `list_roles_for_agent` returns the role and the name comparison is case-insensitive. |

---

## State of the codebase as of this session

- **Templates:** All 20 templates are defined and wired through template fast-path.
- **Plan mode:** Includes two-pass intent extraction, connector resolution, clarification pipeline, deterministic test/revise flow, and workforce-event setup steps.
- **Role policy:** `role_category`, `memory_scope`, and `execution_limits` are persisted and used by runtime prompts. `workflow_outline` is the execution contract for both runtime and test mode.
- **Repair reuse:** `goal_fingerprint`, `repair_version`, `reused_from_session_id`, and `repair_root_session_id` keep same-goal drafts reusable without mutating older snapshots.
- **Completion criteria:** Mechanical checks produce typed `criteria_checks` persisted to DB and shown in run detail UI.
- **Failure handling:** `SkipAndLog` writes the log file and sets metadata for `ErrorsLogged` checks.
- **Savings:** Estimation remains quality-gated and pro-rated for partial runs.
- **Tenant WASM tools:** Tenant-specific WASM modules can be registered/tested up front, approved in plan mode per role, and executed with strict per-tool CPU/memory/time caps and audit logging.
- **Docs:** This file is continuously updated; avoid relying on static line-count/build-count claims.

---

## Tenant WASM tool architecture (new)

Narayan supports tenant-specific custom WASM tools as a policy-first path:

1. Register
- `POST /tenant-wasm-tools` accepts a base64 `.wasm` module + metadata.
- Module is validated before storing.
- The system persists:
  - `permissions` (workspace read/write, env allowlist)
  - `limits` (memory, fuel, timeout), clamped to hard platform maxima
  - export names, hash, version, and timestamps

2. Approve in plan mode
- Intent inference can return `candidate_wasm_tools` by exact name.
- Plan mode persists role scope as:
  - `run_registered_wasm`
  - `wasm_tool:<name>` (one or more approved tool names)
- If custom deterministic logic is needed but no suitable enabled tool is available, plan mode blocks and asks for setup/selection before save.

3. Execute (runtime-enforced)
- Executor can call `run_registered_wasm` only when the requested `tool_name` is in the role's approved scope.
- Executor injects `tenant_id`, workspace path, and run context (`agent_id`, `role_id`, `goal_instance_id`) automatically.
- The tool loads the module from `tenant_wasm_tools`, enforces strict caps at runtime, and runs in a WASI sandbox.
- Runtime `create_workspace_tool` is blocked to prevent unapproved dynamic tool creation.

4. Observe
- Every invocation is recorded in `wasm_tool_runs` with:
  - success/failure
  - elapsed time
  - fuel used
  - memory limit used
  - associated agent/role/goal instance IDs
- `GET /tenant-wasm-tools/runs` returns recent audits.

Design intent: use LLMs for reasoning and orchestration, but execute tenant business logic in deterministic, bounded WASM with hard resource ceilings.
