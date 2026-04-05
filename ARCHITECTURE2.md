# Narayan Architecture 2

_Compiler-first workflow runtime, replacing the old runtime loop._

---

## Overview

Narayan now follows a strict compiler/runtime split:

- **Plan mode** is the drafting compiler.
- **Workflow compilation** is validated and canonicalized by the compiler layer.
- **Runtime** is a deterministic executor.
- **LLM use during execution** is allowed only as an explicit compiled worker step.
- **Setup gaps** are resolved through frontend cards triggered by `ask_user`.

The goal is not a conversational agent loop. The goal is a workflow system that compiles human intent into a fully typed, validated, executable DAG.

---

## Core Principle

The system works like this:

`user intent -> compiler -> compiled workflow artifact -> runtime scheduler -> tool/LLM workers`

The runtime does not invent missing steps, repair broken structure, or guess what should happen next.

If a workflow cannot be executed safely, the failure is handled by:

- retries for transient issues
- resume policies for recoverable interruptions
- recompile/fork for structural or policy failures

---

## What Changed From the Old Architecture

- The old runtime planning loop is removed.
- The old outline fallback path is removed.
- The compiled workflow artifact is now the source of truth.
- `Plan` and `PlannedStep` remain as shared execution data structures, not as an LLM decision layer.
- The old planning service no longer owns runtime behavior.

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

Plan mode stores this draft as typed `workflow_dsl`, and the compiler consumes that draft directly.

### Phase 2.5: Registry Candidate Repair Loop

Plan mode now has a dedicated registry-candidate repair path for explicit tool and connector selection.

The loop is:

1. Plan mode extracts intent and drafts the workflow contract.
2. The plan-mode registry helper builds a filtered candidate set from the tool registry and tenant connectors.
3. The LLM receives three ordered slices:
   - primary
   - secondary
   - fallback
4. The LLM chooses exact tools, operations, resources, mappings, and control-flow fields from those slices.
5. The compiler validates the result and either accepts it or returns a structured repair reason.

Rules:

- the LLM should choose from the candidate slices instead of inventing tools or connectors
- same tool may appear multiple times in a workflow
- workflows may loop back to earlier steps when `loop_back_to` or `repeat_until` makes that explicit
- if a workflow still cannot be expressed, the result must preserve `missing_capabilities`

### Phase 3: Deterministic Tool Binding

The DSL is then bound to exact tools and operations using:

- tool registry
- capability validation
- deterministic binding rules
- explicit resource bindings

This stage produces the compiled workflow artifact that runtime executes.

### Phase 4: Bounded Repair

If the compiler cannot validate the typed workflow on the first pass, it may attempt a limited repair loop.

Repair rules:

- maximum `2` repair passes per draft
- each repair pass must be driven by validation failures or missing bindings
- if the draft is still invalid after the second repair pass, the compiler must stop and emit `ask_user`
- `ask_user` should surface a structured clarification question or setup card, not another hidden repair loop

This keeps compilation iterative enough for complex workflows without turning the compiler into a soft agentic loop.

### Compiler Boundary Today

Plan mode now produces a much more explicit workflow draft, but the compiler still owns the safety boundary.

Plan mode provides:

- typed `workflow_dsl`
- explicit tool and operation hints
- explicit resource ids and resource types
- explicit input and output mappings
- explicit control-flow fields for branching and revisits
- explicit retry policy and success criteria

The compiler still validates and canonicalizes:

- tool existence and operation support
- resource bindings and resource kinds
- read-only and approval policy
- DAG references and revisit references
- workflow versioning metadata
- variant policy and recompile policy
- execution snapshot and lineage metadata

The compiler is no longer the place where tool intent should be guessed, but it is still the place where the draft becomes a safe executable artifact.

---

## Plan Mode vs Compiler vs Lifecycle Ownership

The clean split is:

- **Plan mode** authors the explicit workflow contract.
- **The compiler** validates, canonicalizes, versions, and binds that contract.
- **The runtime and lifecycle layer** execute the workflow and manage child runs.

### Plan Mode Owns

- goal and intent extraction
- explicit step ordering
- explicit tool and operation selection
- explicit input and output mappings
- explicit resource references
- explicit loop and revisit structure
- explicit retry policy hints
- explicit `llm_worker` placement when reasoning is required inside the workflow
- candidate selection from the three-slice registry payload during repair
- preserving `missing_capabilities` when no candidate is sufficient

### Compiler Owns

- tool existence and operation support checks
- resource binding checks
- DAG reference validation
- schema and expression validation
- versioning metadata
- variant and recompile policy
- execution snapshot and lineage metadata
- validation of the explicit registry candidate choice
- repair reasons when a contract cannot be bound exactly

### Runtime / Lifecycle Owns

- step execution
- child workflow spawning
- retry/backoff enforcement
- resume and continue behavior
- failure action application
- runtime variant selection
- runtime execution of the compiled `llm_worker` step when the workflow includes one

The compiler is the safety gate. The runtime is the executor. Plan mode is the author.

---

## Implemented Codex Plan

The recent plan-mode refactor implemented the contract-first direction discussed with Codex.

What changed:

- `plan_mode_steps.rs` now owns the shared workflow contract schema used by plan mode and repair prompts.
- `plan_mode_registry.rs` now owns capability directory generation and the structured three-slice registry repair payload.
- `plan_mode_registry.rs` also emits an `integrations` payload so MCP and ACP can be chosen like first-class integration lanes.
- `plan_mode_registry.rs` now treats ACP as an explicit internal-agent lane and exposes `receive_messages` alongside `list_agents` and `send_message`.
- `plan_mode.rs` now uses the shared helpers instead of carrying the registry and contract logic inline.
- `workflow_compiler.rs` continues to validate and canonicalize the explicit contract rather than guessing tool intent.
- the repair flow now carries an explicit `REGISTRY CANDIDATE SET JSON` block so the LLM can choose from a validated candidate set.
- MCP and ACP now appear as explicit integration candidates, with protocol sub-operations exposed separately from normal tool operations.
- same-tool reuse, multi-operation workflows, and explicit loop-back paths are now part of the contract shape.

Why this matters:

- the LLM still classifies intent, but it must now choose from a narrowed registry slice when selecting tools and connectors
- the LLM must also choose from a narrowed integration slice when selecting MCP or ACP lanes
- ACP can be used for internal agent-to-agent communication when the workflow needs a peer channel rather than a database, API, or external MCP server
- the compiler is reduced to validation, binding, versioning, and repair reasoning
- plan mode is now the place where the draft becomes nearly executable before compilation

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

The current registry logic is split into two layers:

- `src/tools/toolregistry.rs` holds the semantic registry and binding rules used by the compiler to validate explicit contracts
- `src/tools/mod.rs` still holds the executable runtime tool registry and tool implementations

Plan mode now also has a separate registry helper module:

- `src/agent/plan_mode_registry.rs` builds the capability directory
- it renders the three candidate slices used during repair
- it emits the structured `REGISTRY CANDIDATE SET JSON` payload that the plan-mode LLM can use to choose exact tools and connectors
- MCP and ACP are treated as explicit integration lanes in that payload, with protocol-level sub-operations separate from normal connector/tool selection

The shared plan-mode contract text lives in:

- `src/agent/plan_mode_steps.rs`

That file owns the explicit contract schema and prompt fragment. `plan_mode.rs` uses it, but does not duplicate the contract definition.

This means the registry file is not just comments. It defines the binding contract that plan mode and compiler now use to stay aligned.

The important split is:

- `toolregistry.rs` defines the semantic binding rules and registry metadata
- `plan_mode_registry.rs` turns those rules into plan-mode capability directories and repair slices
- `plan_mode_steps.rs` defines the shared workflow contract shape
- `plan_mode.rs` orchestrates the plan-mode conversation and repair flow
- `workflow_compiler.rs` validates and canonicalizes the draft into an executable artifact

---

## Integration Lanes

Plan mode now treats MCP and ACP as first-class integration lanes, not as vague connector fallbacks.

MCP is the transport lane for remote tool servers.
ACP is the transport lane for agent communication, including internal agent-to-agent workflows when one agent should coordinate with another.

This matches the broader protocol design used by the codebase:

- MCP: https://spec.modelcontextprotocol.io/
- MCP docs: https://modelcontextprotocol.io/
- ACP: https://agentclientprotocol.com/

### What The Registry Now Emits

`src/agent/plan_mode_registry.rs` builds a structured `integrations` payload inside the registry candidate set.

That payload can include:

- `mcp_session`
- `search_mcp_registry`
- `acp_session`
- `api_call`
- `register_api_tool`

It also attaches protocol-level `sub_operations`, so MCP and ACP are treated as explicit choices instead of opaque blobs.

For ACP specifically, the registry candidate set now exposes:

- `list_agents`
- `receive_messages`
- `send_message`

That makes ACP usable for polling an internal agent inbox as well as sending messages to another agent.

The same three-slice repair flow still applies:

- primary
- secondary
- fallback

### What The Plan Contract Now Carries

`src/agent/plan_mode_steps.rs` adds explicit integration fields to the workflow contract:

- `integration_protocol`
- `integration_action`
- `integration_sub_operation`
- `server_url`
- `target_agent`

The rules now tell plan mode to do the following:

- use `mcp_session` for MCP-backed workflows when that candidate is present
- use `acp_session` for ACP-backed workflows when that candidate is present
- treat `integration_protocol: "mcp"` as a protocol-lane contract, not a normal connector action
- treat `integration_protocol: "acp"` the same way for ACP, including internal agent-to-agent and receive-message flows

The resource label used for ACP connections is `acp_peer`.

### How To Think About It

- `tool` means an ordinary tool name or connector tool name
- `tool_operation` means the operation on that tool
- `mcp_session` is the MCP transport lane
- `acp_session` is the ACP transport lane
- `sub_operations` are the protocol-level actions underneath those lanes

So the plan-mode LLM can now choose:

- a normal tool
- a connector tool
- an MCP transport action
- an ACP transport action
- an ACP peer setup card when the connection is missing

This is the same contract-first pattern we use for ordinary tools, just extended to protocol-backed integration lanes.

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
- `type`
- `tool`
- `operation`
- `tool_operation`
- `args`
- `resource_id`
- `resource_type`
- `input_mapping`
- `output_mapping`
- `output_schema`
- `read_only`
- `retry_policy`
- `execution_policy`
- `locks`
- `depends_on`
- `next_steps`
- `branch_condition`
- `repeat_until`
- `fallback_step`
- `loop_back_to`
- `success_criteria`

### What the Compiler Adds

Even when plan mode emits a nearly complete contract, the compiler still adds or normalizes:

- `workflow_id`
- `version` and `workflow_version`
- `parent_workflow_version`
- `recompile_reason`
- `dsl_version`, `binding_version`, `runtime_version`
- `tool_registry_version`
- `entry_step`
- `state_schema`
- `resources`
- `permissions`
- `tool_capabilities`
- `binding_rules`
- `variant_policy`
- `recompile_policy`
- `execution_snapshot`
- loop and revisit references
- explicit binding validation results

The compiled artifact is still the canonical runtime input, even when the plan-mode contract is already nearly complete.

That metadata is what makes the workflow auditable, reproducible, and resumable.

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

## Versioning, Variants, and Child Runs

Workflow versioning is spread across the compiler and lifecycle layers.

Compiler-owned version fields:

- `workflow_id`
- `version`
- `workflow_version`
- `parent_workflow_version`
- `recompile_reason`
- `tool_registry_version`
- `execution_snapshot`

Variant selection:

- lives in `workflow_compiler.rs` as `WorkflowVariantPolicy`
- chooses a variant from the current data signature
- falls back to `Recompile` when no variant matches and the workflow cannot safely continue

Recompile / child-run behavior:

- `RecompileMode::Fork` keeps lineage and creates a child workflow version
- `RecompileMode::InPlace` keeps the artifact identity but updates the compiled contract
- the compiler preserves lineage metadata so the runtime and manager can track v1/v2 style revisions

Runtime / lifecycle ownership:

- `manager.rs` copies compiled version metadata into agent runtime state
- `loop.rs` selects workflow variants from current input signatures and marks workflows for recompile when needed
- `dag.rs` handles step retry, retry backoff, and step completion state

The important point is that plan mode should not own lineage or version branching. It should author the draft. The compiler and lifecycle layer decide how that draft becomes a versioned, resumable artifact.

---

## Failure Handling

Failure policy is split by responsibility.

Modeling layer:

- `src/agent/definition.rs` defines `FailureAction`, `FailureRule`, and `infer_failure_action`
- this is where human-readable rules are turned into deterministic actions

Compiler layer:

- `src/agent/workflow_compiler.rs` defines `FailureKind`, `RecompileMode`, `RecompilePolicy`, and `RetryPolicy`
- the compiler validates whether the workflow can be resumed, retried, or forked

Runtime / DAG layer:

- `src/agent/dag.rs` enforces step retry attempts and backoff
- `src/agent/orchestrator.rs` applies failure actions like abort or escalate
- `src/agent/loop.rs` handles failure classification and recompile decisions during lifecycle management

Typical behavior:

- transient or retryable step failures stay inside the DAG retry policy
- structural or policy failures trigger recompile/fork behavior
- human escalation uses the failure-action rules

The compiler should not hide failure policy inside planning. It should validate that the failure policy is explicit and executable.

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

- invent missing steps
- guess missing arguments
- modify the workflow artifact
- open setup cards
- switch tools on its own

---

## LLM as Worker

LLM reasoning is still allowed, but only as an explicit compiled worker step. It is not a plan-mode responsibility because it is part of the executable workflow, not the drafting phase.

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
- `PlanApprovalCard` shows the compiled workflow artifact, compiler stage, validation issues, and any `llm_worker` roles and budgets.
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
- `PlanCard` and `PlanApprovalCard` show the compiled workflow artifact, compiler stage, validation issues, and any `llm_worker` roles.
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
- `src/agent/plan_mode.rs` - user-facing compilation flow and explicit draft generation
- `src/agent/plan_mode_steps.rs` - shared plan-mode workflow contract and prompt fragment
- `src/agent/plan_mode_registry.rs` - capability directory and three-slice registry repair payloads
- `src/agent/manager.rs` - compiled workflow handoff into runtime state
- `src/agent/loop.rs` - variant selection, recompile marking, and runtime lifecycle orchestration
- `src/agent/dag.rs` - DAG scheduling, retries, and state transitions
- `src/agent/orchestrator.rs` - failure action orchestration and escalation
- `src/agent/definition.rs` - failure action modeling, role definitions, and execution rules
- `src/agent/planner.rs` - shared plan data model only
- `src/scheduler/` - task scheduling and runtime orchestration support
- `src/tools/toolregistry.rs` - semantic tool registry, explicit binding rules, and DSL prompt fragment
- `src/tools/mod.rs` - executable runtime tool registry and tool implementations
- `src/cognition/` - judgment and execution policy helpers
- `src/skills/` - reusable plan-mode skill hints and task patterns

---

## Final Rule

Narayan is now a deterministic workflow compiler and runtime.

- Plan mode authoring is explicit and typed.
- Plan mode now uses a dedicated registry-candidate repair loop with three slices.
- The compiler validates, canonicalizes, versions, and binds the draft into a safe artifact.
- The runtime executes structure.
- The LLM can work inside the DAG only when the compiler explicitly allows it.
- The system must always be fully typed, validated, and reproducible before execution.
