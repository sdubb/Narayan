# CLAUDE.md

## Project Overview

Narayan is a distributed AI employee platform for running autonomous agents across any industry vertical. Agents are state machines driven by a scheduler — they are not long-running processes:

```
scheduler wakes agent → worker loads state → agent executes one step → state saved → reschedule
```

**Segment plugin system** — each vertical (Engineering, Sales, Legal, etc.) is a self-contained plugin that contributes connectors, compliance services, policy rules, and SLA policies. Adding a new segment = writing one file in `src/segments/`.

---

## Build & Development Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run all tests
cargo test <test_name>         # Run a single test by name
cargo test -- --nocapture      # Show stdout in tests
cargo clippy -- -W clippy::all # Lint (CI enforces RUSTFLAGS="-D warnings")
cargo fmt                      # Format code
cargo fmt -- --check           # Check formatting only
make all                       # fmt-check + lint + test
```

---

## REST API Reference

**Base URL:** `http://your-server:8080`

**Auth:** All protected routes require `Authorization: Bearer <token>` — either the raw API key (`nar_xxx_...`) or a JWT from `POST /auth/token`.

**Admin routes** require `Authorization: Bearer <NARAYAN_ADMIN_TOKEN>` (separate token, not a tenant JWT).

---

### Authentication

#### `POST /auth/register` — no auth
Create a tenant. API key shown once — save immediately.
```json
// Request
{ "name": "Acme Corp", "email": "admin@acme.com" }

// Response 201
{
  "api_key":    "nar_abc123_xxxxxxxxxxxxxxxx",
  "key_prefix": "abc123",
  "tenant_id":  "uuid"
}
```

#### `POST /auth/token` — no auth
Exchange API key for a short-lived JWT (24h).
```json
// Request
{ "api_key": "nar_abc123_xxxxxxxxxxxxxxxx" }

// Response 200
{ "token": "eyJhbGci...", "tenant_id": "uuid" }
```

---

### Credentials & Routing (BYOK)

#### `PUT /credentials`
Store a provider API key. Encrypted with AES-256-GCM at rest. Auto-configures routing if first key.
```json
// Request
{
  "provider": "anthropic",   // anthropic | openai | gemini | ollama | openrouter | copilot | glm | novita | sglang | compatible
  "api_key":  "sk-ant-...",
  "model":    "claude-sonnet-4-20250514",
  "label":    "Production key"
}
// Response 200
{ "saved": true, "provider": "anthropic", "routing_updated": true }
```

#### `GET /credentials`
List configured providers. Never returns secret values.
```json
// Response 200
{
  "credentials": [
    { "provider": "anthropic", "model": "claude-sonnet-4-20250514", "label": "Production key", "enabled": true }
  ]
}
```

#### `DELETE /credentials/:provider`
```json
// Response 200
{ "deleted": true }
```

#### `PUT /routing`
Manually set which provider handles which complexity tier.
```json
// Request
{
  "simple":   "openai",      // evaluator, preflight, clarifier
  "medium":   "anthropic",   // reflector
  "complex":  "anthropic",   // planner
  "fallback": "openai"       // if preferred provider fails
}
// Response 200
{ "updated": true }
```

---

### Goals & Agents

#### `POST /goals`
Create a goal and its root agent. Agent starts immediately. Returns 402 if spend limit exceeded, 429 if agent limit reached.
```json
// Request
{ "description": "Research the top 5 competitors to Stripe and write a comparison report" }

// Response 201
{ "goal_id": "uuid", "agent_id": "uuid" }
```

#### `GET /agents`
List all agents for this tenant.
```json
// Response 200
{
  "agents": [
    {
      "id":           "uuid",
      "goal":         "Research top 5 competitors...",
      "status":       "running",
      "current_step": 3,
      "next_run":     "2026-03-18T10:05:30Z",
      "created_at":   "2026-03-18T10:00:00Z",
      "updated_at":   "2026-03-18T10:05:00Z"
    }
  ]
}
```

Status values: `pending | preflight | clarifying | running | waiting | delegating | paused | completed | failed`

#### `GET /agents/:id`
Full agent detail including metadata.
```json
// Response 200
{
  "id":             "uuid",
  "goal":           "Research top 5 competitors...",
  "status":         "waiting",
  "current_step":   3,
  "workspace_path": "/var/narayan/workspaces/tenant/agents/uuid",
  "next_run":       "2026-03-18T10:05:30Z",
  "created_at":     "2026-03-18T10:00:00Z",
  "updated_at":     "2026-03-18T10:05:00Z",
  "metadata": {
    "last_reflection": "Found Stripe's main competitors are...",
    "key_findings":    ["Stripe fee: 2.9%", "Square fee: 2.6%"]
  }
}
```

#### `GET /agents/:id/logs`
Raw workspace log file.
```
// Response 200  text/plain
Step 1: searching web for "Stripe competitors 2026"...
```

#### `POST /agents/:id/pause`
```json
// Response 200
{ "paused": true }
```

#### `POST /agents/:id/resume`
```json
// Response 200
{ "resumed": true }
```

#### `POST /agents/:id/clarify`
Submit answers when agent is in `clarifying` status.
```json
// Request
{
  "answers":  ["Q3 2026", "Include EU markets"],
  "freeform": "Focus on European markets in Q3"
}
// Response 200
{ "acknowledged": true }
```

#### `GET /agents/:id/replay`
Step-by-step debug recording.
```json
// Response 200
{
  "agent_id": "uuid",
  "count": 5,
  "steps": [
    { "step_index": 0, "action": "search web for competitors", "result": "{...}", "timestamp": "..." }
  ]
}
```

#### `GET /agents/:id/stream`
**Server-Sent Events** — real-time agent execution stream. See [SSE Events Reference](#sse-events-reference) below.

Stream closes automatically on `goal_complete`, `goal_failed`, or `preflight_failed`.

#### `GET /agents/:id/citations`
All citations recorded for a specific agent.
```json
// Response 200
{
  "citations": [
    {
      "id":          "uuid",
      "agent_id":    "uuid",
      "tenant_id":   "uuid",
      "step_index":  2,
      "claim":       "Stripe charges 2.9% + $0.30 per transaction",
      "source_type": "tool_output",
      "source_ref":  "web_search_tool",
      "excerpt":     "According to Stripe's pricing page...",
      "confidence":  0.95,
      "created_at":  "2026-03-18T10:03:00Z"
    }
  ],
  "count": 1
}
```

---

### Citations (cross-agent)

#### `GET /citations`
All citations for this tenant across all agents. Returns last 200 ordered by `created_at DESC`.
```json
// Response 200
{ "citations": [...], "count": 42 }
```

---

### Reviews

#### `GET /reviews`
List review items. Optional query param `?status=pending` to filter to pending only (default returns all).
```json
// Response 200
{
  "reviews": [
    {
      "id":             "uuid",
      "tenant_id":      "uuid",
      "agent_id":       "uuid",
      "step_index":     3,
      "summary":        "Agent wants to call external API with user PII",
      "reason":         "external_api_pii",
      "status":         "pending",
      "reviewer_notes": null,
      "created_at":     "2026-03-18T10:04:00Z",
      "reviewed_at":    null
    }
  ],
  "count": 1
}
```

Status values: `pending | approved | rejected | changes_requested`

#### `POST /reviews/:id/resolve`
Resolve a single review item. The UI term `auto_approved` is accepted and maps to `approved` on the wire.
```json
// Request
{
  "status": "approved",   // approved | auto_approved | rejected | changes_requested
  "notes":  "Looks safe — proceed"
}
// Response 200
{ "resolved": true }
```

#### `POST /reviews/resolve-all`
Bulk-resolve all pending reviews for this tenant.
```json
// Request
{ "status": "approved", "notes": "Bulk approved from admin" }
// Response 200
{ "resolved": 7 }
```

---

### Auto-Approvals

Rules that suppress the review queue for a specific policy `rule_id`. Stored in-process (survives restarts via re-evaluation on next trigger).

#### `GET /auto-approvals`
```json
// Response 200
{
  "rules": [
    {
      "rule_id":    "web_search_external",
      "tenant_id":  "uuid",
      "notes":      "Always safe for this tenant",
      "created_at": "2026-03-18T10:00:00Z"
    }
  ],
  "count": 1
}
```

#### `POST /auto-approvals`
```json
// Request
{ "rule_id": "web_search_external", "notes": "Always safe for this tenant" }
// Response 201
{ "saved": true, "rule": { "rule_id": "...", "tenant_id": "...", ... } }
```

#### `DELETE /auto-approvals/:rule_id`
```json
// Response 200
{ "deleted": true }
```

---

### Outbound Webhooks

#### `POST /webhooks`
Register a webhook endpoint. Payloads are signed with HMAC-SHA256.
```json
// Request
{
  "url":    "https://your-server.com/webhook",
  "events": ["goal_complete", "goal_failed"],
  "secret": "optional-signing-secret"
}
// Response 201
{
  "id":     "uuid",
  "url":    "https://your-server.com/webhook",
  "secret": "auto-generated-if-not-provided",
  "events": ["goal_complete", "goal_failed"]
}
```

#### `GET /webhooks`
```json
// Response 200
{
  "webhooks": [
    { "id": "uuid", "url": "...", "events": [...], "enabled": true, "failure_count": 0 }
  ],
  "count": 1
}
```

#### `DELETE /webhooks/:id`
```json
// Response 200
{ "deleted": true }
```

---

### Skills Marketplace

#### `POST /skills/upload`
```json
// Request
{
  "name":        "github_pr_creator",
  "description": "Clone repo, make changes, commit, open PR",
  "steps":       ["clone the repository", "modify the file", "commit", "open a pull request"],
  "author":      "narayan"
}
// Response 201
{ "uploaded": true, "name": "github_pr_creator" }
```

#### `GET /skills`
List all marketplace skills.

#### `POST /skills/install`
```json
// Request
{ "name": "github_pr_creator" }
// Response 200
{ "installed": true, "name": "github_pr_creator" }
```

#### `GET /skills/registry`
List installed (active) skills.

---

### Connectors (Inbound Webhooks)

#### `POST /connectors/:type/webhook`
Receive inbound events from external systems. The connector parses the payload, generates a goal string, and creates an agent.

Supported types: `github | zendesk | servicenow | salesforce | quickbooks | docusign | pagerduty | hubspot | notion | greenhouse | dbt_cloud`

```json
// Response 200
{
  "received":      true,
  "connector":     "github",
  "agent_created": true,
  "agent_id":      "uuid",
  "goal_id":       "uuid"
}
```

---

### Audit Log

#### `GET /audit`
Query the immutable audit log for this tenant.

Query params: `agent_id`, `action`, `from` (ISO-8601), `to` (ISO-8601), `limit` (default 100), `offset`

```json
// Response 200
{
  "entries": [
    {
      "id":         "uuid",
      "tenant_id":  "uuid",
      "agent_id":   "uuid",
      "action":     "goal_created",
      "detail":     { "goal_id": "...", "description": "..." },
      "ip_address": "1.2.3.4",
      "created_at": "2026-03-18T10:00:00Z"
    }
  ],
  "count": 1
}
```

Audit action values: `tenant_registered | token_issued | credential_set | credential_deleted | routing_updated | goal_created | agent_paused | agent_resumed | agent_clarified | step_started | step_completed | tool_executed | tool_blocked | llm_call_completed | spend_limit_exceeded | spend_limit_warning | tenant_suspended | tenant_plan_changed | webhook_registered | webhook_delivered | webhook_failed | custom`

---

### Observability

#### `GET /metrics`
Platform-level counters.
```json
// Response 200
{
  "steps_total":         892,
  "agents_running":      4,
  "goals_total":         142,
  "llm_calls_total":     8941,
  "llm_cache_hits":      1205,
  "input_tokens_total":  4291000,
  "output_tokens_total": 891000,
  "uptime_secs":         86400,
  // Frontend-friendly aliases
  "agents_started":      142,
  "agents_finished":     138,
  "steps_completed":     892,
  "steps_per_minute":    14
}
```

#### `GET /costs`
Token usage and spend for this tenant.
```json
// Response 200
{
  "tenant_id":         "uuid",
  "spend_limit_usd":   500.00,
  "current_spend_usd": 4.27,
  "pct_used":          0.854,
  "total_input_tokens":  1840000,
  "total_output_tokens": 382000,
  "total_requests":      412,
  // Per-provider breakdown (frontend UsageTab key)
  "total_usd": 4.27,
  "usage": {
    "anthropic": { "input_tokens": 1840000, "output_tokens": 382000, "usd": 4.27 }
  }
}
```

#### `GET /swarm/status`
```json
// Response 200
{
  "queue_depth":  7,
  "pool_size":    32,
  "queue_backed": true
}
```

`queue_backed: true` means Redis, `false` means in-memory.

#### `GET /health` — no auth
```json
{ "status": "ok", "service": "narayan" }
```

---

### Admin API

All routes require `Authorization: Bearer <NARAYAN_ADMIN_TOKEN>`.

| Method | Path | Description |
|--------|------|-------------|
| `GET`  | `/admin/info` | System info (version, build) |
| `GET`  | `/admin/health/ready` | Readiness check (DB ping) |
| `GET`  | `/admin/health/live` | Liveness check |
| `GET`  | `/admin/metrics` | Platform-wide counters |
| `GET`  | `/admin/tenants` | List all tenants with plan + spend |
| `POST` | `/admin/tenants/:id/suspend` | Suspend a tenant |
| `POST` | `/admin/tenants/:id/activate` | Re-activate a tenant |
| `PUT`  | `/admin/tenants/:id/plan` | Change plan: `{ "plan": "pro" }` |
| `GET`  | `/admin/spend` | Cross-tenant spend report |
| `GET`  | `/admin/audit` | Cross-tenant audit log query |

---

## SSE Events Reference

Connect with:
```
GET /agents/:id/stream
Authorization: Bearer <token>
```

Use `fetch` + `ReadableStream` — **not** `EventSource` (which cannot send `Authorization` headers).

Each event is a JSON object on a `data:` line. All events include `"event": "<type>"` as the discriminant field (Rust `#[serde(tag = "event", rename_all = "snake_case")]`).

The stream closes automatically when `goal_complete`, `goal_failed`, or `preflight_failed` is received.

---

### Preflight & Clarification

| Event | Fields | When emitted |
|-------|--------|-------------|
| `preflight_started` | `agent_id` | Before feasibility check |
| `preflight_passed` | `agent_id` | Goal is achievable |
| `preflight_failed` | `agent_id`, `reason` | Goal not achievable — **terminal** |
| `clarification_needed` | `agent_id`, `questions: string[]` | Agent needs user input before planning |
| `clarification_received` | `agent_id` | User answers submitted, agent resuming |

```json
{ "event": "clarification_needed", "agent_id": "uuid", "questions": ["Which repo?", "Which branch?"] }
```

---

### Planning

| Event | Fields | When emitted |
|-------|--------|-------------|
| `planning_started` | `agent_id` | LLM planning call begins |
| `plan_created` | `agent_id`, `step_count`, `rationale` | Plan ready |

```json
{ "event": "plan_created", "agent_id": "uuid", "step_count": 6, "rationale": "Starting with research before writing..." }
```

---

### Step Execution

| Event | Fields | When emitted |
|-------|--------|-------------|
| `step_started` | `agent_id`, `step_index`, `description` | Step begins |
| `tool_called` | `agent_id`, `step_index`, `tool_name`, `args_preview` | Tool call dispatched |
| `tool_result` | `agent_id`, `step_index`, `tool_name`, `success`, `output_preview` | Tool returned |
| `step_completed` | `agent_id`, `step_index`, `success`, `summary` | Step done |
| `step_retrying` | `agent_id`, `step_index`, `delay_secs`, `reason` | Transient failure, will retry |

```json
{ "event": "tool_result", "agent_id": "uuid", "step_index": 2, "tool_name": "web_search_tool", "success": true, "output_preview": "Found 8 results for..." }
```

---

### Policy & Compliance

| Event | Fields | When emitted |
|-------|--------|-------------|
| `policy_decision` | `agent_id`, `step_index`, `tool`, `decision`, `rule_id?`, `reason?`, `risk_level` | Every tool call through policy engine |
| `pii_redacted` | `agent_id`, `step_index`, `tool`, `fields_redacted: string[]` | PII stripped from tool args |
| `sla_check` | `agent_id`, `pct_elapsed`, `message`, `action?`, `deadline?` | SLA threshold crossed |
| `citation_recorded` | `agent_id`, `step_index`, `claim`, `source_ref`, `source_type`, `confidence` | Citation stored per tool call |
| `evidence_packaged` | `agent_id`, `citations`, `audit_entries` | Full evidence bundle assembled |
| `review_required` | `agent_id`, `review_id`, `summary`, `reason`, `rule_id?` | Policy rule requires human approval — agent paused |

`decision` values: `"allow" | "block" | "require_approval" | "redact" | "downgrade"`

`action` values (SLA): `"escalate" | "notify"`

`source_type` values: `"tool_output" | "document" | "url" | "memory" | "user_input"`

```json
{ "event": "policy_decision", "agent_id": "uuid", "step_index": 3, "tool": "shell", "decision": "block", "rule_id": "no_shell_production", "reason": "Shell execution not permitted in production", "risk_level": "high" }

{ "event": "pii_redacted", "agent_id": "uuid", "step_index": 3, "tool": "email", "fields_redacted": ["email", "ssn"] }

{ "event": "review_required", "agent_id": "uuid", "review_id": "rev-123", "summary": "Agent wants to call Stripe API with card number", "reason": "external_api_pii", "rule_id": "external_api_pii" }

{ "event": "citation_recorded", "agent_id": "uuid", "step_index": 2, "claim": "Stripe charges 2.9% per transaction", "source_ref": "web_search_tool", "source_type": "tool_output", "confidence": 0.95 }
```

---

### Connector Triggers

| Event | Fields | When emitted |
|-------|--------|-------------|
| `connector_trigger` | `agent_id`, `connector_type`, `event_type`, `external_id?` | Agent created from inbound connector webhook |

```json
{ "event": "connector_trigger", "agent_id": "uuid", "connector_type": "github", "event_type": "pull_request.opened", "external_id": "pr-42" }
```

---

### Delegation

| Event | Fields | When emitted |
|-------|--------|-------------|
| `child_spawned` | `agent_id`, `child_agent_id`, `sub_goal` | Sub-agent created |
| `children_complete` | `agent_id`, `child_ids: string[]` | All sub-agents done, parent resumes |

```json
{ "event": "child_spawned", "agent_id": "uuid", "child_agent_id": "uuid2", "sub_goal": "Research Stripe pricing page" }
```

---

### Terminal Events

| Event | Fields | Notes |
|-------|--------|-------|
| `goal_complete` | `agent_id`, `summary` | Success — stream closes |
| `goal_failed` | `agent_id`, `reason` | Unrecoverable failure — stream closes |

---

### Infrastructure Events

| Event | Fields | Notes |
|-------|--------|-------|
| `lag` | `agent_id`, `missed` | Subscriber fell behind — `missed` events were dropped |

```json
{ "event": "lag", "agent_id": "uuid", "missed": 12 }
```

When `lag` is received the frontend should warn the user that some events were missed and offer to load the replay log.

---

### Complete SSE Event → Frontend Card Mapping

| SSE event type | Frontend component | Phase group |
|---|---|---|
| `preflight_started` | `StepRow` (gray) | Preflight |
| `preflight_passed` | `StepRow` (green) | Preflight |
| `preflight_failed` | `StepRow` (red) | Preflight |
| `clarification_needed` | `ClarifyCard` (inline form) | Preflight |
| `clarification_received` | `StepRow` (green) | Preflight |
| `planning_started` | `StepRow` (blue) | Planning |
| `plan_created` | `PlanCard` (collapsible) | Planning |
| `step_started` | `StepRow` (gray) | Step N |
| `tool_called` | `StepRow` (amber) | Step N |
| `tool_result` | `StepRow` (green/amber) | Step N |
| `step_completed` | `StepRow` (green) | Step N |
| `step_retrying` | `StepRow` (amber) | Step N |
| `policy_decision` | `PolicyCard` | Step N |
| `pii_redacted` | `PiiCard` | Step N |
| `sla_check` | `SlaCard` | Step N |
| `citation_recorded` | `CitationCard` | Step N |
| `evidence_packaged` | `EvidenceCard` | Completion |
| `review_required` | `ReviewQueueCard` (4-option approval) | Step N |
| `connector_trigger` | `ConnectorTriggerCard` | (prepended) |
| `child_spawned` | `StepRow` (violet) | Delegation |
| `children_complete` | `StepRow` (violet) | Delegation |
| `goal_complete` | Banner + `AgentResultView` | Completion |
| `goal_failed` | Banner + `AgentResultView` | Completion |
| `lag` | `StepRow` (amber warning) | (inline) |

---

## Architecture

### Core Execution Loop

```
Preflight → Clarifier → Planner → Executor → EvaluateAndReflect → (loop or complete)
```

All cognitive components are LLM-backed. `AgentLoop` (`agent/loop.rs`) orchestrates them.

**Per-step pipeline in Executor (for each tool call):**
1. `PiiRedactor.redact(args)` — strip sensitive fields, emit `pii_redacted` SSE if any found
2. `PolicyEngine.evaluate(ctx)` — emit `policy_decision` SSE; block/approve/redact/allow
3. `plane_guard_risk()` — hard safety floor (critical = always blocked)
4. `tool.execute(clean_args)` — actual execution
5. On `RequireApproval`: submit to `ReviewQueue`, emit `review_required` SSE, agent pauses

**LLM calls per agent lifetime:**
- Preflight step: 2 calls (Preflight + Clarifier)
- Planning step: 1 call (Planner)
- Each execution step: 2 calls (Executor + combined EvaluateAndReflect)

### Segment Plugin System (`src/segments/`)

```
main.rs
  └── SegmentRegistry::builder()
        .add(engineering::plugin(&deps, tenant_id))
        .add(customer_support::plugin(&deps, tenant_id))
        .add(legal_contract::plugin(&deps, tenant_id))
        ...
        .build()
              ├── merged ConnectorRegistry  (all inbound webhook routes)
              ├── merged AgentServices      (union of all active service flags)
              ├── merged SlaTracker         (all SLA policies combined)
              └── merged PolicyRuleSet      (all tenant rules combined)
```

### Active Segments & Their Connectors

| Segment | Plugin file | Connectors | Services active |
|---|---|---|---|
| Engineering Maintenance | `segments/engineering.rs` | GitHub | policy, reviews |
| Customer Support | `segments/customer_support.rs` | Zendesk | policy, citations, reviews, pii |
| Compliance Ops | `segments/compliance_ops.rs` | ServiceNow | policy, citations, reviews, evidence, pii |
| Sales & RevOps | `segments/sales_revops.rs` | Salesforce | policy, citations, reviews, pii |
| Finance & Accounting | `segments/finance_accounting.rs` | QuickBooks | policy, citations, reviews, evidence, pii |
| HR & People Ops | `segments/hr_people_ops.rs` | Greenhouse | policy, citations, reviews, pii |
| Legal & Contract Ops | `segments/legal_contract.rs` | DocuSign | policy, citations, reviews, evidence, pii |
| IT Ops & ITSM | `segments/it_ops_itsm.rs` | ServiceNow + PagerDuty | policy, citations, reviews, evidence |
| Research & Intelligence | `segments/research_intelligence.rs` | Notion | policy, citations, reviews, evidence, pii |
| Data & Analytics Ops | `segments/data_analytics.rs` | dbt Cloud | policy, citations, reviews, pii |
| Marketing & Growth | `segments/marketing_growth.rs` | HubSpot | policy, citations, reviews, pii |

### Adding a New Segment

Create `src/segments/my_segment.rs`:

```rust
pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    SegmentPlugin {
        id:   "my_segment",
        name: "My Segment",
        connectors: vec![Arc::new(MyConnector::new())],
        services: SegmentServices {
            policy:    Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews:   Some(deps.review_queue.clone()),
            evidence:  None,
            pii:       None,
            sla:       None,
        },
        policy_rules: PolicyRuleSet::new(tenant_id.into()),
        sla_policies: vec![],
    }
}
```

Add `pub mod my_segment;` to `segments/mod.rs` and `.add(segments::my_segment::plugin(...))` to `main.rs`.

### Connector Reference

| Connector | Trigger events | Delivery methods |
|---|---|---|
| GitHub | `pr_opened`, `issue_created`, `check_run`, `@mention` | pr_review, issue_comment |
| Zendesk | `ticket_created`, `ticket_updated` | internal note, public reply |
| ServiceNow | `incident_created`, `change_request` | work_notes |
| Salesforce | `lead_created`, `opportunity_stage_changed`, `renewal_alert` | note, field_update, task |
| QuickBooks | `invoice_overdue`, `expense_batch_ready`, `month_end_close` | invoice_note |
| DocuSign | `envelope_sent`, `envelope_completed`, `envelope_declined` | envelope_note |
| PagerDuty | `incident.triggered`, `incident.resolved`, `service.degraded` | incident_note, status_update |
| HubSpot | `contact.propertyChange`, `deal.stageChange`, `form.submission` | note, task |
| Notion | `database_item_created`, `research_request` | page block append |
| Greenhouse | `application`, `interview`, `offer` | candidate note |
| dbt Cloud | `job.run.errored`, `job.run.completed`, `source.freshness.error` | run annotation |

### Module Map

| Module | Key File(s) | Purpose |
|--------|------------|---------|
| `agent/` | `loop.rs`, `executor.rs`, `planner.rs`, `prompts.rs` | Core state machine |
| `api/` | `routes.rs`, `server.rs`, `admin/routes.rs`, `stream.rs` | Axum REST + SSE server |
| `audit/` | `log.rs` | Append-only PostgreSQL audit log |
| `auth/` | `apikey.rs`, `jwt.rs`, `middleware.rs` | API key hashing, JWT, middleware |
| `browser/` | `pool.rs` | Chromium pool for browser automation |
| `cognition/` | `control_loop.rs` | Step limit + wall-clock timeout guard |
| `compliance/` | `citations.rs`, `pii.rs`, `evidence.rs`, `reviewer.rs`, `sla.rs` | Full compliance stack |
| `connectors/` | `framework.rs` + 11 connectors | Inbound webhook routing → agent goals |
| `debug/` | `recorder.rs`, `replay.rs` | Step recording and replay |
| `events/` | `bus.rs` | Per-agent broadcast channels, 23 event types |
| `gateway/` | `gateway.rs`, `cost.rs`, `cache.rs`, `limiter.rs`, `router.rs` | BYOK LLM routing, cost tracking |
| `knowledge/` | `graph.rs` | Entity extraction from reflections |
| `memory/` | `embeddings.rs`, `vector.rs`, `store.rs` | pgvector + Redis memory |
| `policy/` | `engine.rs`, `rules.rs` | Per-tool-call policy evaluation |
| `scheduler/` | `scheduler.rs`, `queue.rs` | DB polling + Redis/in-memory queue |
| `segments/` | `mod.rs`, `registry.rs`, 11 segment files | Plugin system |
| `skills/` | `registry.rs`, `compiler.rs`, `executor.rs` | Skill registry (skips LLM planning) |
| `state/` | `agent_state.rs` | `AgentState` struct |
| `storage/` | `postgres.rs` | `PostgresStore` — all DB queries |
| `swarm/` | `mod.rs` | `Arc<Swarm>` wraps `Arc<dyn Queue>` |
| `tools/` | `mod.rs`, `selector.rs`, 60+ tool files | 80+ tools |
| `worker/` | `pool.rs`, `worker.rs` | Worker pool + evidence SSE emission |
| `workspace/` | `manager.rs`, `local.rs`, `remote.rs` | Hybrid local/S3 workspace |

### Startup Sequence (`main.rs`)

```
Config → DB pool → TenantStore → AuditLog → WebhookStore → CitationTracker → ReviewQueue
→ SharedDeps (policy, pii, evidence built once)
→ SegmentRegistry::builder() — all 11 plugins registered, merged into:
     ConnectorRegistry + AgentServices + SlaTracker + PolicyRuleSet
→ Queue (Redis/in-memory)
→ RedisMemoryStore (7-day TTL) → Embedder + pgvector → WorkspaceManager
→ LLM Gateway (CostTracker, RateLimiter, ResponseCache)
→ Browser pool → Tool registry
→ Agent runtime: Planner / Executor(+event_bus) / Evaluator / Reflector / Preflight / Clarifier
→ AgentLoop(+event_bus) → AgentManager → Scheduler → WorkerPool(+event_bus)
→ API Server (AppState includes citation_tracker, auto_approvals, event_bus_handle)
```

### Database Tables

| Table | Notes |
|-------|-------|
| `agents` | `created_at`, `updated_at`, `started_at` (wall-clock timeout) |
| `goals` | Goal tracking |
| `tenants` / `tenant_configs` | Multi-tenancy, AES-256-GCM credential encryption |
| `audit_log` | Immutable (trigger-enforced). Indexed on tenant_id, agent_id, action, created_at |
| `webhooks` / `webhook_deliveries` | HMAC-SHA256 signed, retry with backoff, auto-disable after 10 failures |
| `citations` | Per-step source attribution. Linked to agent_id, step_index |
| `review_queue` | Human review items. Pending blocks agent progression |
| `vector_documents` | pgvector HNSW index. Auto-populated from reflection summaries |
| `workspaces` | Workspace metadata |

### AppState Fields (routes.rs)

| Field | Type | Purpose |
|-------|------|---------|
| `store` | `Arc<PostgresStore>` | All agent/goal DB queries |
| `tenant_store` | `Arc<TenantStore>` | Tenant + credential queries |
| `manager` | `Arc<AgentManager>` | Goal creation |
| `cost_tracker` | `Arc<CostTracker>` | Spend tracking |
| `metrics` | `Arc<Metrics>` | Atomic counters |
| `skill_registry` | `Arc<RwLock<SkillRegistry>>` | Installed skills |
| `marketplace` | `Arc<Mutex<SkillMarketplace>>` | Marketplace skills |
| `audit_log` | `Arc<AuditLog>` | Append-only audit writes |
| `webhook_store` | `Arc<WebhookStore>` | Webhook registration |
| `webhook_dispatcher` | `Arc<WebhookDispatcher>` | Outbound delivery |
| `review_queue` | `Arc<ReviewQueue>` | Human review items |
| `swarm` | `Arc<Swarm>` | Queue depth query |
| `connector_registry` | `Arc<ConnectorRegistry>` | Inbound webhook routing |
| `citation_tracker` | `Option<Arc<CitationTracker>>` | Cross-agent citation queries |
| `auto_approvals` | `Arc<AutoApprovalStore>` | In-process auto-approval rules |
| `event_bus_handle` | `Arc<EventBus>` | Publish SSE from HTTP handlers (connectors) |

---

## Configuration

```
NARAYAN__DATABASE__URL         PostgreSQL connection string
NARAYAN__REDIS__URL            Redis connection string
NARAYAN__REDIS__ENABLED        true | false
NARAYAN__WORKER__POOL_SIZE     Worker pool size (default: 32)
NARAYAN_JWT_SECRET             JWT signing key
NARAYAN_ENCRYPT_KEY            AES-256-GCM passphrase
NARAYAN_ADMIN_TOKEN            Admin API token
NARAYAN_EMBED_PROVIDER         openai | anthropic | ollama | stub
NARAYAN_EMBED_API_KEY          Embedding API key
NARAYAN_EMBED_MODEL            Embedding model name
NARAYAN_BROWSER_POOL_SIZE      0 to disable browser tools
```

---

## Tool Security (plane_guard_risk in executor.rs)

| Risk level | Example tools | Behaviour |
|---|---|---|
| `low` | `file_read`, `web_fetch`, `vector_search`, `sql_query` (read) | Allowed, no gate |
| `medium` | `file_write`, `git_operations`, `code_run`, `email`, `ssh_exec` | Policy engine gate |
| `high` | `docker`, `kubernetes`, `delegate`, `mcp_session` | Policy engine gate |
| `critical` | (hardcoded list) | Always blocked, no override |

`PolicyEngine` (from active segments) adds tenant-configurable rules on top of the platform defaults.

---

## Spend Limits

- Free: $5 / Pro: $500 / Enterprise: unlimited
- Gateway pre-checks every LLM call. `SpendCheck::Exceeded` → bail. Warning at 80%.
- `POST /goals` returns 402 if limit exceeded.
- `GET /costs` returns `pct_used` for frontend progress bar rendering.

---

## Code Conventions

- Rust edition 2021, max line width 120 chars
- `crate::util::new_id()` for UUID v4 IDs
- `anyhow::Result` everywhere, `anyhow::bail!()` for early returns
- `#[cfg(test)] mod tests` at file bottom, `#[tokio::test]` for async
- **Never store plan in `metadata` JSONB** — use `state.plan`
- **Never use `InMemoryStore` in production** — use `RedisMemoryStore`
- **Never call `crate::swarm::push/next` free functions** — use `Arc<Swarm>`
- **Never construct `AgentServices` manually in main.rs** — use `SegmentRegistry::builder()`
- **Never add a compliance/policy call directly to executor/loop** — add it as a segment plugin
- **Always publish SSE via `event_bus.publish()`** — never write raw SSE strings

---

## Supported LLM Providers

`anthropic | openai | gemini | ollama | openrouter | copilot | glm | novita | sglang | compatible`

---

## Billing API

Step-based billing — Narayan charges for platform execution steps, not LLM tokens (BYOK).
`spend_limit_usd` in `GET /costs` is **informational only** — it shows the tenant's own LLM spend.

### Plans

| Plan | Price | Steps/month | Concurrent agents | Connectors | Compliance |
|------|-------|-------------|-------------------|------------|------------|
| Free | $0 | 1,000 | 3 | All 20 | Full stack |
| Go | $15/mo | 20,000 | 20 | All 20 | Full stack |
| Pro | $79/mo | 150,000 | 200 | All 20 | Full stack |
| Enterprise | custom | unlimited | unlimited | All 20 + custom | Full stack |

Credit top-ups: **$8 = 5,000 extra steps** (any paid plan, purchased as one-time PayPal order).

Everyone gets all connectors and the full compliance stack — the only differentiator is scale.

### Billing routes

#### `POST /billing/checkout`
Create a hosted checkout session (PayPal or Stripe).
```json
// Request
{ "plan": "go", "provider": "paypal", "success_url": "...", "cancel_url": "..." }

// Response 200
{
  "session_id":   "PAYID-xxx",
  "provider":     "paypal",
  "redirect_url": "https://www.paypal.com/checkoutnow?token=...",
  "plan":         "go",
  "amount_usd":   15.0,
  "expires_at":   "2026-03-19T11:00:00Z"
}
```

#### `GET /billing/subscription`
```json
// Response 200 (active subscriber)
{
  "id":                       "uuid",
  "provider":                 "paypal",
  "provider_subscription_id": "I-xxx",
  "plan":                     "go",
  "status":                   "active",
  "current_period_start":     "2026-03-01T00:00:00Z",
  "current_period_end":       "2026-04-01T00:00:00Z"
}

// Response 200 (free / no subscription)
{ "plan": "free", "status": "active" }
```

#### `POST /billing/subscription/cancel`
```json
// Response 200
{ "cancelled": true }
```

#### `GET /billing/invoices`
```json
// Response 200
{
  "invoices": [
    {
      "id":              "uuid",
      "provider":        "paypal",
      "provider_inv_id": "SALE-xxx",
      "amount_usd":      15.0,
      "status":          "paid",
      "period_start":    "2026-03-01T00:00:00Z",
      "period_end":      "2026-04-01T00:00:00Z",
      "pdf_url":         null,
      "created_at":      "2026-03-01T00:01:00Z"
    }
  ],
  "count": 1
}
```

#### `GET /billing/credits`
```json
// Response 200
{
  "tenant_id":      "uuid",
  "extra_steps":    5000,
  "pack_price_usd": 8.0,
  "pack_steps":     5000
}
```

#### `POST /billing/credits/purchase`
Creates a one-time PayPal order for a credit top-up pack.
```json
// Response 200
{
  "session_id":   "PAYID-xxx",
  "redirect_url": "https://www.paypal.com/checkoutnow?token=...",
  "steps":        5000,
  "amount_usd":   8.0
}
```

#### `POST /billing/webhooks/:provider` — no auth (signature-verified internally)
Receives PayPal and Stripe webhook events. Provider name in path: `paypal` | `stripe`.
Always returns 200 to prevent retries. Returns 400 only on signature failure.

### Adding a new billing provider

1. Create `src/billing/razorpay.rs` implementing `BillingProvider` trait
2. Add `pub mod razorpay;` to `src/billing/mod.rs`
3. In `main.rs`: `.register(Arc::new(RazorpayProvider::from_env()))`
4. The webhook route `/billing/webhooks/razorpay` works automatically

Required env vars per provider:
```
# PayPal
PAYPAL_CLIENT_ID=...
PAYPAL_CLIENT_SECRET=...
PAYPAL_WEBHOOK_ID=...           # from PayPal dashboard
PAYPAL_SANDBOX=true             # omit for production

# Stripe
STRIPE_SECRET_KEY=sk_live_...
STRIPE_WEBHOOK_SECRET=whsec_...
```

---

## Connector System

### Three connector types

**Type 1 — Inbound webhook (push):** External system posts to Narayan → agent created.
User pastes Narayan's webhook URL + secret into the external system's settings.

**Type 2 — MCP tool use (pull by agents):** Agent calls MCP server tools during execution.
User connects once via OAuth or API key → token stored → all agents use it automatically.

**Type 3 — Both:** Triggers agents on new events AND agents use it as a tool.

### All 20 connectors

| Connector | Auth | Triggers agents | Agents use as tool |
|-----------|------|-----------------|-------------------|
| GitHub | API key or OAuth | Poll issues/PRs | ✅ MCP |
| Zendesk | API key | Poll tickets | ❌ |
| ServiceNow | API key + URL | Poll incidents | ❌ |
| Salesforce | OAuth | Poll opportunities | ✅ MCP |
| QuickBooks | OAuth | Poll invoices | ❌ |
| DocuSign | OAuth | Poll envelopes | ❌ |
| PagerDuty | API key | Poll incidents | ❌ |
| HubSpot | OAuth | Poll deals | ✅ MCP |
| Notion | OAuth | Poll database | ✅ MCP |
| Greenhouse | API key | Poll applications | ❌ |
| dbt Cloud | API key + account_id | Poll failed runs | ❌ |
| Slack | OAuth | Poll channels | ✅ MCP |
| Gmail | OAuth | Poll inbox | ✅ MCP |
| Outlook | OAuth | Poll inbox | ✅ MCP (Graph API) |
| Google Sheets | OAuth | ❌ | ✅ MCP |
| Google Docs | OAuth | ❌ | ✅ MCP |
| Teams | OAuth | Poll channels | ✅ MCP (Graph API) |
| Jira | OAuth | Poll issues | ✅ MCP |
| Confluence | OAuth | ❌ | ✅ MCP |
| Linear | API key | Poll issues | ✅ MCP |

### Connector install API

#### `GET /connectors`
List all installed connectors for this tenant.
```json
// Response 200
{
  "connectors": [
    {
      "id":             "uuid",
      "connector_type": "slack",
      "auth_type":      "oauth",
      "connected":      true,
      "settings":       { "team_id": "T123" },
      "last_polled_at": "2026-03-18T10:00:00Z",
      "created_at":     "2026-03-01T00:00:00Z"
    }
  ],
  "count": 1
}
```

#### `GET /auth/oauth/:provider/start` — requires auth
Redirect the user to the provider's OAuth consent page.
Supported providers: `slack` | `gmail` | `google` | `outlook` | `microsoft` | `salesforce` | `hubspot` | `jira` | `atlassian` | `notion` | `github` | `quickbooks` | `docusign`

Required env vars per provider (e.g. Slack):
```
SLACK_CLIENT_ID=...
SLACK_CLIENT_SECRET=...
NARAYAN_BASE_URL=https://your-narayan.com   # for callback URL
NARAYAN_UI_URL=https://your-ui.com          # for post-OAuth redirect
```

#### `GET /auth/oauth/:provider/callback` — public (called by OAuth provider)
Exchanges the authorization code, encrypts and stores the token, then redirects to the UI.
On success: `→ {NARAYAN_UI_URL}/settings/connectors?connected=slack`
On failure: `→ {NARAYAN_UI_URL}/settings/connectors?error=...`

#### `POST /connectors/:type/install`
Install an API-key connector (GitHub, Linear, PagerDuty, dbt Cloud, etc.)
```json
// Request
{
  "api_key":  "ghp_xxxxxxxxxxxx",
  "settings": { "repo": "acme/backend" }
}
// Response 201
{ "installed": true, "id": "uuid", "connector": "github" }
```

#### `POST /connectors/:type/webhook-install`
Install a webhook-push connector. Returns the URL and secret to paste into the external system.
```json
// Request
{ "settings": { "subdomain": "acme" } }

// Response 201
{
  "installed":      true,
  "id":             "uuid",
  "connector":      "zendesk",
  "webhook_url":    "https://your-narayan.com/connectors/zendesk/webhook",
  "webhook_secret": "nar_whsec_xxxxxxxxxxxxxxxxxxxx",
  "note":           "Paste the webhook_url and webhook_secret into the external system's webhook settings."
}
```

#### `DELETE /connectors/:type`
```json
// Response 200
{ "uninstalled": true }
```

### How polling works

The `ConnectorPoller` runs alongside the agent scheduler. Every 60 seconds it checks which installed connectors are due for a poll based on their interval:

| Interval | Connectors |
|----------|-----------|
| 2 min | GitHub, Jira/Atlassian, Linear, Zendesk |
| 3 min | ServiceNow, Greenhouse |
| 5 min | Slack, Gmail, Microsoft/Outlook, Salesforce, HubSpot, PagerDuty |
| 10 min | Notion, dbt Cloud |
| 15 min | QuickBooks, DocuSign |

For each connector due for polling, it calls the provider API, finds new events since `last_polled_at`, and calls `AgentManager::create_goal()` for each one.

### MCP auto-token injection

When an agent calls `mcp_session` with a known MCP server URL, the tool automatically looks up the stored OAuth token for the matching connector (no need for the agent to know the token):

```
Agent calls: mcp_session(server_url="https://slack.mcp.claude.ai/mcp", action="call_tool", ...)
  → McpSessionTool looks up connector_type "slack" for this tenant
  → Finds stored OAuth token in connector_installs table
  → Injects token into Authorization header automatically
  → Agent never sees the token
```

URL → connector type mappings in `src/tools/mcp_session.rs :: mcp_url_to_connector()`.

### Keeping tenant plan in sync

When a PayPal/Stripe webhook fires `SubscriptionActivated` or `SubscriptionCancelled`:
1. `BillingStore::process_event()` updates `subscriptions` table
2. Calls `UPDATE tenants SET plan=$1` to sync the tenant's active plan
3. Next JWT issued will reflect the new plan (JWTs are 24h — tenant may need to re-authenticate for immediate plan enforcement)

---

## Environment Variables (complete)

```
# Core
NARAYAN__DATABASE__URL         PostgreSQL connection string
NARAYAN__REDIS__URL            Redis connection string
NARAYAN__REDIS__ENABLED        true | false
NARAYAN__WORKER__POOL_SIZE     Worker pool size (default: 32)
NARAYAN_JWT_SECRET             JWT signing key (min 32 chars)
NARAYAN_ENCRYPT_KEY            AES-256-GCM passphrase (min 32 chars)
NARAYAN_ADMIN_TOKEN            Admin API token
NARAYAN_BASE_URL               Public URL of this Narayan instance (for OAuth callbacks)
NARAYAN_UI_URL                 Frontend URL (for post-OAuth redirects)

# Embeddings
NARAYAN_EMBED_PROVIDER         openai | anthropic | ollama | stub
NARAYAN_EMBED_API_KEY          Embedding API key
NARAYAN_EMBED_MODEL            Embedding model name

# Browser
NARAYAN_BROWSER_POOL_SIZE      0 to disable browser tools

# Billing — PayPal
PAYPAL_CLIENT_ID               PayPal REST API client ID
PAYPAL_CLIENT_SECRET           PayPal REST API client secret
PAYPAL_WEBHOOK_ID              Webhook ID from PayPal dashboard
PAYPAL_SANDBOX                 true for sandbox, false (default) for live

# Billing — Stripe
STRIPE_SECRET_KEY              Stripe secret key (sk_live_... or sk_test_...)
STRIPE_WEBHOOK_SECRET          Stripe webhook signing secret (whsec_...)

# OAuth connectors (PROVIDER_CLIENT_ID + PROVIDER_CLIENT_SECRET for each)
SLACK_CLIENT_ID / SLACK_CLIENT_SECRET
GMAIL_CLIENT_ID / GMAIL_CLIENT_SECRET         (or GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)
OUTLOOK_CLIENT_ID / OUTLOOK_CLIENT_SECRET     (or MICROSOFT_CLIENT_ID / MICROSOFT_CLIENT_SECRET)
SALESFORCE_CLIENT_ID / SALESFORCE_CLIENT_SECRET
HUBSPOT_CLIENT_ID / HUBSPOT_CLIENT_SECRET
JIRA_CLIENT_ID / JIRA_CLIENT_SECRET           (or ATLASSIAN_CLIENT_ID / ATLASSIAN_CLIENT_SECRET)
NOTION_CLIENT_ID / NOTION_CLIENT_SECRET
GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET
QUICKBOOKS_CLIENT_ID / QUICKBOOKS_CLIENT_SECRET
DOCUSIGN_CLIENT_ID / DOCUSIGN_CLIENT_SECRET
```
