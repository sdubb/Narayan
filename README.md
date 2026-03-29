# Narayan

Narayan is a B2B autonomous agent platform for running AI workers on top of your existing systems. Deploy agents as an intelligence layer over your backend, databases, APIs, and SaaS tools.

It is designed to do two things well:

1. Help a user configure an agent conversationally in plan mode.
2. Run that agent deterministically and safely in production — monitoring, detecting, and acting on your data.

## What Narayan Does

**Don't replace your backend. Make it smarter.**

- **Bring Your Own Backend** — Connect any API, database, or REST endpoint. Agents fetch data and trigger actions on your systems.
- **Webhook Ingestion** — Real-time events from Zendesk, Salesforce, GitHub, ServiceNow, and 17+ more platforms trigger agents instantly.
- **Monitoring Agents** — Detect anomalies, flag exceptions, and track SLA deadlines on your data.
- **Action Agents** — Fetch context, make decisions, and call your APIs to update records or notify teams.
- **Bring-Your-Own Databases** — Query PostgreSQL, execute SQL directly, or use external databases as agent memory.
- **MCP Server Support** — Use Model Context Protocol servers for extensible tool integrations into any system.

For the full system design, see [ARCHITECTURE.md](ARCHITECTURE.md).

## High-Level Architecture

```text
Frontend (React + Vite)
  -> HTTP API (Axum routes + SSE bus)
  -> Plan Mode
  -> Deterministic Planner
  -> Agent Runtime
  -> Tools / Connectors / LLMs
  -> Storage / Memory / Workspace
```

```text
user intent
  -> intent extraction
  -> clarification steps
  -> review draft
  -> test / revise
  -> save agent + roles
```

```text
workflow_outline
  -> Plan::from_workflow_outline()
  -> executable steps
```

```text
worker picks task
  -> AgentLoop runs one step
  -> preflight checks
  -> execute tool or LLM call
  -> evaluate result
  -> reflect / retry / continue
  -> persist state and requeue if needed
```

## What Narayan Is Made Of

Narayan is not one big agent. It is a set of cooperating subsystems:

| Component | Responsibility |
|---|---|
| `src/api` | HTTP routes for plan mode, agents, runs, connectors, billing, and SSE delivery |
| `src/agent` | Plan mode, runtime loop, planner, evaluator, executor, prompts, role chat |
| `src/segments` | Domain bundles that define sector-specific connectors, policies, SLA behavior, and judgment tuning |
| `src/tools` | The tool registry and all built-in tools: filesystem, browser, connector tools, wasm, search, memory, and more |
| `src/connectors` | OAuth, webhook, polling, and connector install/credential management |
| `src/workspace` | Per-agent workspace creation, local/remote storage, archiving, and path resolution |
| `src/storage` | Postgres persistence for agents, roles, sessions, runs, credentials, and workspace metadata |
| `src/gateway` | LLM routing, provider selection, cost tracking, and request limiting |
| `src/events` | In-process event bus used for UI streaming and internal event dispatch |
| `narayan-v5/` | Frontend app that visualizes setup, execution, cards, timeline, and run details |

## End-to-End Flow

### 1. Plan Mode

Plan mode is the conversational setup system.

What it does:

- turns plain language into an `AgentDefinition` and one or more `AgentRole`s
- asks clarifying questions when details are missing
- infers triggers, outputs, connectors, tools, and constraints
- builds a `workflow_outline`
- runs deterministic test validation
- can revise the draft before save

Plan mode is where the system learns what the agent should do.

### 2. Deterministic Planning

Narayan now uses the saved `workflow_outline` as the source of truth for runtime execution.

The planner module does not invent steps for a configured role. Instead, it translates the saved workflow outline into a runtime `Plan`.

```text
workflow_outline
  -> Plan::from_workflow_outline()
  -> executable steps
```

This makes the runtime predictable, testable, and easier to repair.

### 3. Runtime Execution

The runtime is driven by the worker pool and the agent loop.

```text
worker picks task
  -> AgentLoop runs one step
  -> preflight checks
  -> execute tool or LLM call
  -> evaluate result
  -> reflect / retry / continue
  -> persist state and requeue if needed
```

The loop is not a forever-running process. It performs one unit of work, saves state, and yields back to the scheduler.

### 4. Tools and Connectors

Tools are the execution layer.

Examples:

- `file_read`, `file_write`, `file_edit`
- `glob_search`, `content_search`
- `browser`, `web_fetch`, `web_search_tool`
- `shell`, `git_operations`, `code_run`
- `memory_store`, `memory_recall`
- `external_db`, `external_api`
- connector tools such as Salesforce, Zendesk, Slack, GitHub, QuickBooks, and others

Connectors are the integrations that let agents move data in and out of SaaS systems.

They can be:

- built-in connector tools
- installed tenant connectors
- custom external databases
- custom REST APIs
- MCP servers

### 5. Segments and Domains

Segments are the domain layer.

Each segment packages:

- canonical domain identity
- policy rules
- SLA behavior
- judgment tuning
- the connectors that make sense for that domain

Examples include:

- `finance_accounting`
- `legal_contract`
- `hr_people_ops`
- `it_ops_itsm`
- `customer_support`
- `sales_revops`
- `research_intelligence`
- `procurement_vendor_ops`
- `security_ops_grc`
- `customer_success_renewals`

This is what makes Narayan feel enterprise-aware instead of generic.

### 6. Workspace Layer

Each agent or plan-mode session works inside its own workspace.

That workspace stores:

- uploaded documents
- sandbox artifacts
- logs
- generated files
- temporary outputs

The workspace layer gives the agent a safe local boundary for file operations and testing.

## Core Backend Subsystems

### API

The Axum API exposes:

- plan mode session creation and turns
- plan-mode test and revise endpoints
- save/deploy flows
- agent creation and management
- connector and credential endpoints
- replay/debug views
- webhook ingestion and event delivery

### Gateway

The gateway is the LLM traffic layer.

It handles:

- model/provider selection
- rate limiting
- caching
- cost tracking
- event publication

### Storage

PostgreSQL stores:

- agent definitions
- agent roles
- plan mode sessions
- run history
- workspace metadata
- connector installs
- credentials
- billing data
- review/evidence records

pgvector is used for memory and retrieval-style features.

### Events

The event bus connects backend activity to the UI.

It streams things like:

- step events
- judgement signals
- run updates
- webhook deliveries
- replay/debug events

## Frontend

The `narayan-v5` app is the operator console.

It shows:

- plan mode conversations
- agent cards
- run timelines
- step-by-step execution
- judgment and review signals
- completed run details

The frontend is tied to the event stream so users can watch agents work in near real time.

## Deployment View

```text
Postgres + Redis + Object Storage + LLM Providers
  -> Narayan Backend
  -> API + Scheduler + Worker Pool
  -> Frontend Dashboard
```

## Mental Model

If you want the simplest way to think about Narayan:

- `plan mode` decides what the agent should be
- `workflow_outline` defines what the agent is allowed to do
- `planner` converts that outline into runtime steps
- `worker + loop` execute the steps
- `tools + connectors` touch the real world
- `segments` decide how strict and domain-aware the agent should be
- `workspace` keeps file-based work isolated and traceable
- `events + frontend` make the system visible

## Quick Start

```bash
cp .env.example .env
cargo build --release
./target/release/narayan
```

## Key Docs

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [CLAUDE.md](CLAUDE.md)


