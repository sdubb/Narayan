# Narayan Architecture 2

_Compiler-first workflow runtime, replacing the old planner-centered loop._

---

## Overview

Narayan now follows a strict compiler/runtime split:

- **Plan mode** is a compiler.
- **Runtime** is a deterministic executor.
- **LLM use during execution** is allowed only as an explicit compiled worker step.
- **Setup gaps** are resolved through frontend cards triggered by `ask_user`.

The goal is not a conversational agent loop. The goal is a workflow system that compiles human intent into a fully typed, validated, executable DAG.

---

## Core Principle

The system works like this:

`user intent -> compiler -> compiled workflow artifact -> runtime scheduler -> tool/LLM workers`

The runtime does not invent missing steps, repair broken structure, or call the planner to guess what should happen next.

If a workflow cannot be executed safely, the failure is handled by:

- retries for transient issues
- resume policies for recoverable interruptions
- recompile/fork for structural or policy failures

---

## What Changed From the Old Architecture

- The old runtime planner is removed.
- The old outline fallback path is removed.
- The compiled workflow artifact is now the source of truth.
- `Plan` and `PlannedStep` remain as shared execution data structures, not as an LLM decision layer.
- The old planner service no longer owns runtime behavior.

---

## Compilation Pipeline

### Phase 1: Intent Extraction

Plan mode extracts:

- entities
- operations
- constraints
- outputs
- setup requirements

The output of this phase is structured intent, not an executable workflow yet.

### Phase 2: DSL Generation

The compiler converts intent into a strict, tool-agnostic DSL.

Allowed DSL step types:

- `fetch_records`
- `filter`
- `compute`
- `aggregate`
- `detect_anomaly`
- `branch`
- `notify`
- `store_result`

DSL rules:

- no tool references
- no implicit data flow
- no free-form expressions
- no custom step types
- every step must define output shape and next step or branch paths

### Phase 3: Deterministic Tool Binding

The DSL is then bound to exact tools and operations using:

- tool registry
- capability validation
- deterministic binding rules
- explicit resource bindings

This stage produces the compiled workflow artifact that runtime executes.

---

## Type System

Narayan uses a typed workflow contract.

Supported types:

- primitive: `number`, `string`, `boolean`
- composite: `array`, `object`

Rules:

- every output must declare a type
- every input must match an expected type
- nested types must be explicit
- type compatibility is checked during compilation

Example:

```json
{
  "type": "array",
  "items": {
    "type": "object",
    "fields": {
      "id": "number",
      "email": "string"
    }
  }
}
```

---

## Expression System

Conditions and branches use a typed expression DSL.

Rules:

- no string expressions
- no language-specific syntax
- all operators must validate operand types
- all functions must exist in the expression registry

Example:

```json
{
  "type": "boolean",
  "op": "gt",
  "left": {
    "type": "number",
    "fn": "len",
    "args": ["step_2.anomalies"]
  },
  "right": {
    "type": "number",
    "value": 0
  }
}
```

---

## Tool Registry

The tool registry is the source of truth for:

- supported operations
- input schema
- output schema
- required resources
- capability constraints

The compiler reads the registry to choose valid bindings.
The runtime reads the registry to validate execution.

Binding must be deterministic. The compiler may not use fuzzy matching to invent tool usage.

---

## Workflow Artifact

The compiled workflow artifact is the saved contract that runtime follows.

Top-level fields typically include:

- `workflow_id`
- `workflow_version`
- `parent_workflow_version`
- `recompile_reason`
- `dsl_version`
- `binding_version`
- `runtime_version`
- `compiler_version`
- `tool_registry_version`
- `entry_step`
- `resources`
- `permissions`
- `state_schema`
- `execution`
- `execution_constraints`
- `data_strategy`
- `determinism`
- `variant_policy`
- `recompile_policy`
- `execution_snapshot`
- `steps`

Each step contains:

- `id`
- `tool`
- `operation`
- `args`
- `input_mapping`
- `output_mapping`
- `output_schema`
- `retry_policy`
- `execution_policy`
- `locks`
- `depends_on`
- `next_steps`
- `success_criteria`

---

## Resource Model

Resources are explicit and named.

Example:

```json
{
  "resources": {
    "db_main": {
      "type": "database",
      "connector": "postgres",
      "permissions": ["read_only"]
    }
  }
}
```

Rules:

- steps reference resources by id
- permissions are enforced at runtime
- no hidden connection state

---

## Execution Model

Runtime is a deterministic scheduler/executor.

Runtime responsibilities:

- resolve inputs
- validate schema
- execute the compiled tool or LLM worker step
- validate output
- persist state
- enforce execution policy
- evaluate expressions
- schedule next steps
- respect locks and concurrency limits

Runtime must not:

- call the planner
- guess missing arguments
- modify the workflow artifact
- open setup cards
- switch tools on its own

---

## LLM as Worker

LLM reasoning is still allowed, but only as an explicit compiled worker step.

The reserved worker node is `llm_worker`.

Each `llm_worker` step should also carry an explicit `llm_role` so the compiler and runtime know what kind of reasoning it performs.

Each `llm_worker` step should also carry an explicit generation budget:

- `execution_intent` for strict vs balanced vs creative behavior
- `max_tokens`
- `temperature`
- optional cost or cadence hints for frequent low-budget runs

That lets the compiler keep recurring minute/hourly jobs lean while still giving high-value drafting or recovery steps a larger budget when needed.

Recommended roles:

- `extractor`
- `router`
- `drafter`
- `critic`
- `validator`
- `recovery`
- `failure_classifier`

Important rule:

- `tool: None` is reserved for structural nodes such as branches and other non-executing control-flow helpers.
- An LLM reasoning step must be compiled as `tool: "llm_worker"`.

That worker step must still have:

- explicit input mapping
- explicit instruction/prompt
- explicit output schema
- explicit success criteria
- explicit retry and resume policy
- explicit `llm_role`
- explicit generation budget

Use cases:

- summarize a Zendesk ticket
- extract intent from text
- draft an email
- classify a message
- generate a plan summary

Multiple `llm_worker` steps may appear in the same workflow. They can run sequentially or as separate branches, just like any other DAG node. This preserves deterministic orchestration while still allowing LLM-powered processing inside the DAG.

Failure handling should stay deterministic first:

- classify failures without the LLM when the runtime can do so safely
- use `failure_classifier` only when the failure is ambiguous or structurally unclear
- only recompile when the failure is structural or policy-related

Avoid using `executor` as an `llm_role` name. The runtime is already the executor, so roles should describe reasoning intent instead: `drafter`, `router`, `validator`, and so on.

---

## ask_user and Frontend Cards

`ask_user` is not a chat tool.

It triggers frontend setup cards for:

- database
- connector
- MCP
- API auth

Rules:

- compilation pauses when a required setup is missing
- the frontend card collects the missing configuration
- the compiler resumes with the updated resource context
- runtime never opens cards

`ask_user` can also emit structured questions for the frontend when the missing information is a real user choice instead of a setup dependency.

Supported question modes:

- `mcq` - one choice from a bounded list
- `multi_select` - choose more than one option
- `text` - free-form text answer
- `card_open` - open a setup card instead of asking a text question
- `hybrid` - show choices plus a text fallback

Example structured question:

```json
{
  "id": "trigger_mode",
  "question_type": "mcq",
  "prompt": "How should this workflow start?",
  "options": ["Manual", "Schedule", "Webhook", "After another role"],
  "recommended": ["Schedule"],
  "required": true
}
```

Example setup question:

```json
{
  "id": "connect_database",
  "question_type": "card_open",
  "prompt": "Connect the database before continuing.",
  "card_type": "database",
  "binding_target": "db_main",
  "required_fields": ["host", "port", "db_name"],
  "resume_token": "bind_db_main"
}
```

Frontend behavior:

- `PlanModeChat` renders the current question inline during plan mode.
- `ClarificationCard` renders question chips, option buttons, text input, or setup-card actions depending on `question_type`.
- `PlanApprovalCard` shows the compiled plan with LLM roles and budgets.
- `AgentTimeline` shows clarification cards during runtime/repair flows.

### End-to-End Example

1. The user logs in and opens the agent composer.
2. The user types: `Watch my users table and alert me when failed_logins > 5.`
3. Plan mode starts compiling the goal.
4. The compiler extracts structured intent:
   - database monitoring
   - users table
   - anomaly detection
   - notification output
5. The compiler detects that the database is not connected yet.
6. Instead of guessing, plan mode emits `ask_user` with `question_type: card_open`.
7. The frontend opens the database setup card.
8. The user enters the database host, port, database name, and credentials.
9. The saved connection updates the resource context.
10. Plan mode resumes compilation with the new resource binding.
11. The compiler produces a typed workflow:
   - fetch records from `db_main`
   - run anomaly detection
   - branch on whether anomalies exist
   - notify or store the result
12. If a reasoning step is required, the compiler emits an explicit `llm_worker` node with:
   - `llm_role`
   - `execution_intent`
   - budget settings
   - input mapping
   - output schema
13. The user reviews the compiled workflow in the plan UI.
14. The user runs test and save.
15. Runtime loads the compiled artifact, resolves inputs, enforces locks and policies, and executes the DAG.
16. If runtime hits a structural or policy failure, it marks the workflow for recompile and forks a new version using the preserved lineage.
17. The frontend shows the run timeline, step cards, cost, failures, and any clarification/setup prompts.

### Frontend Flow Summary

- `ChatPage` opens the agent surface.
- `PlanModeChat` is the compiler UI.
- `ClarificationCard` handles user questions and setup routing.
- `DatabaseConnectionCard`, `CustomConnectionCard`, and `ConnectorSetupModal` collect missing setup state.
- `PlanCard` and `PlanApprovalCard` show the compiled workflow and any `llm_worker` roles.
- `AgentTimeline` shows runtime execution and pending clarifications.
- `Settings` is used when a setup request needs to jump to a dedicated screen.

---

## Variant Policy

The system supports workflow variants for different data regimes.

Use variants when:

- the workflow structure is the same
- but the input data shape differs

Examples:

- `normal`
- `high_volume`
- `sparse_data`

Runtime selects the best matching variant by data signature.

If no variant matches and the workflow cannot safely continue, the system may request a recompile.

---

## Recompile Policy

Recompile is a fork, not an overwrite.

Rules:

- preserve the previous workflow version
- create a child workflow version
- reuse successful outputs when compatible
- recompile only the failed subgraph when possible
- recompile the whole workflow only when the shared contract changed

Typical triggers:

- structural failure
- policy failure

Not typical triggers:

- transient network failure
- retryable data fetch failure

---

## Validation Gate

A workflow is rejected if it has:

- invalid DSL types
- missing output schemas
- unsupported tool operations
- unresolved placeholders
- missing resource bindings
- missing branch conditions
- type mismatches
- invalid expressions
- broken DAG references

Validation happens before save and before execution.

---

## Storage and State

Compiled workflows are durable artifacts.

Runtime state persists:

- step outputs
- execution snapshots
- failure metadata
- recompile lineage
- variant selection
- resource bindings

This makes replay, auditing, and repair possible without reconstructing intent from scratch.

---

## Module Mapping

Relevant code areas in this architecture:

- `src/agent/workflow_compiler.rs` - compiler and workflow artifact generation
- `src/agent/plan_mode.rs` - user-facing compilation flow
- `src/agent/loop.rs` - runtime orchestration
- `src/agent/planner.rs` - shared plan data model only
- `src/worker/worker.rs` - recompile handoff and execution signaling
- `src/scheduler/` - task scheduling and runtime orchestration support
- `src/tools/` - tool registry and tool contracts
- `src/cognition/` - judgment and execution policy helpers
- `src/skills/` - reusable plan-mode skill hints and task patterns

---

## Final Rule

Narayan is now a deterministic workflow compiler and runtime.

- The compiler decides structure.
- The runtime executes structure.
- The LLM can work inside the DAG only when the compiler explicitly allows it.
- The system must always be fully typed, validated, and reproducible before execution.
