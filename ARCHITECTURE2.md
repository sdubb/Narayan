# Narayan Architecture 2

_Registry-grounded plan mode, compiler validation, and deterministic runtime._

---

## Overview

Narayan now uses a staged pipeline:

- **Registry search** starts from the user request and asks narrow planning tools for connectors, MCP servers, or ACP peers as needed.
- **Plan mode** drafts intent, clarification steps, and a typed workflow contract.
- **Compiler validation** checks the draft against the live registry and available bindings.
- **Runtime** executes only the compiled artifact.

The plan-mode LLM is no longer expected to know the full tool surface by default. Instead, it starts small and calls search tools to pull back grounded slices for tools, connectors, MCP, ACP, DB, and API bindings.

---

## Core Principle

The system works like this:

`user intent -> plan-mode search loop -> plan-mode synthesis -> compiler validation -> compiled workflow -> runtime dispatch`

The runtime does not invent missing steps or guess at unsupported tools. If the workflow is incomplete, plan mode and the compiler collaborate to surface a bounded clarification step or setup card.

---

## What Changed From the Older Architecture

- The old always-refine plan loop is gone from initial intent capture.
- Plan mode now begins with a small planning tool surface and uses search tools before finalizing the draft.
- Tool, connector, MCP, ACP, database, and API selection all follow the same discover/select/operate pattern.
- Clarification questions are primarily model-authored, with the backend acting as a thin validator and router.
- Runtime still executes the compiled graph deterministically.

---

## Plan Mode Flow

### 1. Registry Search

Plan mode starts with the user prompt and a small planning tool surface. The LLM can then ask the backend for narrow search results such as:

- connector registry lookup
- MCP server lookup
- ACP peer lookup
- category-based connector listing

Those search results are split into narrow buckets such as:

- `tool_discovery`
- `tool_actions`
- `mcp_discovery`
- `mcp_selected`
- `mcp_actions`
- `acp`

The goal is to show the model only the capability slices that matter for the request, and only after it asks.

### 2. Workflow Synthesis

Plan mode extracts:

- goal
- constraints
- preferred tools
- candidate connectors
- missing capabilities
- clarification steps
- typed `workflow_dsl`

The LLM is encouraged to choose from the returned search results rather than inventing new tools or connector names.

### 3. Clarification Routing

Clarifications are now intentionally narrow:

- the LLM authors the clarification steps when possible
- the backend validates and normalizes those steps
- backend fallback text is minimal and acts as a safety net only

Backend-specific setup types include:

- database
- API
- MCP
- ACP

The frontend renders those as dedicated cards rather than one generic connector prompt.

### 4. Compiler Validation

The compiler validates:

- tool existence and operation support
- connector bindings
- MCP server and operation selection
- ACP peer and message contract selection
- DB/API setup requirements
- workflow structure and output schemas

If validation fails, the compiler can emit a repair reason or ask for a missing setup card.

---

## Tool and Integration Shape

Plan mode now treats all capability families in the same staged way:

1. discover the capability
2. select the concrete instance
3. choose the exact operation
4. supply the required args or bindings

This applies to:

- ordinary tools
- connectors
- MCP servers
- ACP peers
- database bindings
- API bindings

The search loop and returned search results give the model the exact names and operations available in each family.

---

## MCP And ACP

MCP is modeled as:

- family
- server
- operation
- args / output contract

ACP is modeled as:

- family
- peer
- target agent
- message / response contract

That means the model should not treat MCP or ACP as vague connector labels. They are explicit integration lanes with concrete operations and bindings.

---

## Boundary Handshakes

Boundary handshakes are the explicit safety contract for cross-enterprise and cross-team communication.

They are separate from ordinary connector setup.

### Where They Show Up

- `src/agent/plan_mode/boundary.rs` detects when a workflow needs a boundary handshake.
- `src/agent/plan_mode/clarify.rs` builds the `boundary_handshake` setup card.
- `src/boundry/mod.rs` and the boundary submodules persist, validate, audit, and govern the handshake lifecycle.
- `src/api/routes.rs` exposes accept, revoke, freeze, unfreeze, report, and audit endpoints.

### What The Flow Does

1. Plan mode detects a boundary need from the intent or from `acp_session:*` style tool usage.
2. The backend emits a structured `AskUserBoundaryHandshake` card.
3. The user supplies peer details, role, scope, and acceptance.
4. The accepted handshake is injected back into the role as a real ACP peer binding.
5. The compiler validates the draft with the boundary requirement in place.
6. Runtime can then execute the workflow knowing the handshake contract is explicit and auditable.

### What Boundary Handshakes Carry

- `peer_hint`
- `scope`
- `required`
- `peer_endpoint`
- `peer_name`
- `role`
- `accepted`

### Why This Matters

- ACP peer communication is not just a tool call.
- Cross-company and cross-team exchanges need explicit handshake semantics.
- Boundary handshakes keep those exchanges safe, visible, and auditable before runtime starts.

---

## Workflow Contract

Plan mode emits a structured `workflow_dsl` with fields like:

- `id`
- `type`
- `tool`
- `tool_operation`
- `integration_protocol`
- `integration_action`
- `integration_sub_operation`
- `resource_id`
- `resource_type`
- `server_url`
- `target_agent`
- `input_mapping`
- `output_schema`
- `depends_on`
- `next_steps`
- `loop_back_to`
- `repeat_until`
- `retry_policy`
- `success_criteria`

The compiler then canonicalizes that draft into an executable workflow artifact.

---

## Compiler And Runtime Split

### Compiler Owns

- validation of the typed contract
- binding tools and connectors
- checking search results against live capabilities
- missing-capability reporting
- repair reasoning
- versioning and lineage metadata

### Runtime Owns

- step execution
- DAG scheduling
- retries and backoff
- output persistence
- failure classification
- recompile decisions

### Plan Mode Owns

- intent capture
- clarification creation
- registry-grounded synthesis
- draft contract authoring
- bounded repair input

---

## LLM Role In The System

The LLM is the interpreter and drafter, not the sole source of truth.

It should:

- interpret the user request
- call search tools when it needs registry grounding
- choose from the returned search results
- author clarification steps
- draft the workflow contract

It should not:

- invent unsupported tools
- bypass bindings
- hide missing setup behind vague text

The backend still validates every selection.

---

## Frontend Plan Mode

The UI now mirrors the backend split:

- plan mode chat renders the current question queue
- clarification cards show stage badges like:
  - discover
  - select
  - operate
- setup cards are separate for:
  - database
  - API
  - MCP
  - ACP

The frontend does not author clarification logic. It displays the backend-generated plan state and setup requests.

---

## Module Mapping

Relevant code areas in the current architecture:

- `src/agent/plan_mode/orchestrator.rs` - thin plan-mode coordinator
- `src/agent/plan_mode/intent.rs` - intent seeding and compact snapshots
- `src/agent/plan_mode/registry.rs` - search helpers, capability directory, and search-result builders
- `src/agent/plan_mode/steps.rs` - shared workflow contract and clarification step schema
- `src/agent/plan_mode/clarify.rs` - clarification routing and backend setup handling
- `src/agent/plan_mode/review.rs` - review summary and saved-role policy defaults
- `src/agent/plan_mode/repair.rs` - compact repair loop
- `src/agent/workflow_compiler.rs` - compiler validation and binding
- `src/tools/mod.rs` - executable runtime tool registry
- `src/tools/connector_tool.rs` - connector registry vocabulary
- `src/agent/dag_engine.rs` - runtime scheduling and execution
- `src/agent/orchestrator.rs` - runtime step orchestration
- `src/agent/loop.rs` - runtime lifecycle orchestration

---

## Final Rule

Narayan should stay deterministic after plan mode finishes:

- plan mode authors the draft
- the compiler validates the draft
- the runtime executes the compiled artifact

If the draft cannot be bound safely, the system should ask for the missing piece instead of silently guessing.
