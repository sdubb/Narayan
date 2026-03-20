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

## REST API Routes

**Auth:** `Authorization: Bearer <token>` (API key `nar_xxx_...` or JWT from `POST /auth/token`). Admin routes use `NARAYAN_ADMIN_TOKEN`.

| Method | Path | Auth | Purpose |
|--------|------|------|---------|
| `POST` | `/auth/register` | none | Create tenant → returns `api_key` (shown once) |
| `POST` | `/auth/token` | none | Exchange API key → JWT (24h) |
| `PUT` | `/credentials` | tenant | Store provider API key (AES-256-GCM at rest) |
| `GET` | `/credentials` | tenant | List configured providers (no secrets) |
| `DELETE` | `/credentials/:provider` | tenant | Remove provider credential |
| `PUT` | `/routing` | tenant | Set complexity tier routing (`simple`/`medium`/`complex`/`fallback`) |
| `POST` | `/goals` | tenant | Create goal + root agent (402 if spend exceeded, 429 if agent limit) |
| `GET` | `/agents` | tenant | List all agents |
| `GET` | `/agents/:id` | tenant | Full agent detail + metadata |
| `GET` | `/agents/:id/logs` | tenant | Raw workspace log (text/plain) |
| `POST` | `/agents/:id/pause` | tenant | Pause agent |
| `POST` | `/agents/:id/resume` | tenant | Resume agent |
| `POST` | `/agents/:id/clarify` | tenant | Submit clarification answers (`answers[]`, `freeform`) |
| `GET` | `/agents/:id/replay` | tenant | Step-by-step debug recording |
| `GET` | `/agents/:id/stream` | tenant | SSE real-time stream |
| `GET` | `/agents/:id/citations` | tenant | Citations for this agent |
| `GET` | `/citations` | tenant | All citations across agents (last 200) |
| `GET` | `/reviews` | tenant | List reviews (optional `?status=pending`) |
| `POST` | `/reviews/:id/resolve` | tenant | Resolve review (`approved`/`auto_approved`/`rejected`/`changes_requested`) |
| `POST` | `/reviews/resolve-all` | tenant | Bulk-resolve all pending reviews |
| `GET` | `/auto-approvals` | tenant | List auto-approval rules |
| `POST` | `/auto-approvals` | tenant | Create auto-approval for a `rule_id` |
| `DELETE` | `/auto-approvals/:rule_id` | tenant | Remove auto-approval |
| `POST` | `/webhooks` | tenant | Register outbound webhook (HMAC-SHA256 signed) |
| `GET` | `/webhooks` | tenant | List webhooks |
| `DELETE` | `/webhooks/:id` | tenant | Remove webhook |
| `POST` | `/skills/upload` | tenant | Upload skill to marketplace |
| `GET` | `/skills` | tenant | List marketplace skills |
| `POST` | `/skills/install` | tenant | Install a skill |
| `GET` | `/skills/registry` | tenant | List installed skills |
| `POST` | `/connectors/:type/webhook` | varies | Inbound connector webhook → creates agent |
| `GET` | `/audit` | tenant | Query audit log (`agent_id`, `action`, `from`, `to`, `limit`, `offset`) |
| `GET` | `/metrics` | tenant | Platform counters |
| `GET` | `/costs` | tenant | Token usage + spend (`pct_used` for progress bar) |
| `GET` | `/swarm/status` | tenant | Queue depth + pool size |
| `GET` | `/health` | none | Health check |

**Agent statuses:** `pending | preflight | clarifying | running | waiting | delegating | paused | completed | failed`

**Audit actions:** `tenant_registered | token_issued | credential_set | credential_deleted | routing_updated | goal_created | agent_paused | agent_resumed | agent_clarified | step_started | step_completed | tool_executed | tool_blocked | llm_call_completed | spend_limit_exceeded | spend_limit_warning | tenant_suspended | tenant_plan_changed | webhook_registered | webhook_delivered | webhook_failed | custom`

### Admin API

All routes require `Authorization: Bearer <NARAYAN_ADMIN_TOKEN>`.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/admin/info` | System info (version, build) |
| `GET` | `/admin/health/ready` | Readiness check (DB ping) |
| `GET` | `/admin/health/live` | Liveness check |
| `GET` | `/admin/metrics` | Platform-wide counters |
| `GET` | `/admin/tenants` | List all tenants with plan + spend |
| `POST` | `/admin/tenants/:id/suspend` | Suspend a tenant |
| `POST` | `/admin/tenants/:id/activate` | Re-activate a tenant |
| `PUT` | `/admin/tenants/:id/plan` | Change plan: `{ "plan": "pro" }` |
| `GET` | `/admin/spend` | Cross-tenant spend report |
| `GET` | `/admin/audit` | Cross-tenant audit log query |

---

## SSE Events Reference

Connect: `GET /agents/:id/stream` with `Authorization: Bearer <token>`. Use `fetch` + `ReadableStream` (not `EventSource`). Stream closes on `goal_complete`, `goal_failed`, or `preflight_failed`.

All events are JSON on `data:` lines. Discriminant: `"event": "<type>"` (Rust `#[serde(tag = "event", rename_all = "snake_case")]`).

| Event | Key fields | Phase | Terminal? |
|-------|-----------|-------|-----------|
| `preflight_started` | `agent_id` | Preflight | |
| `preflight_passed` | `agent_id` | Preflight | |
| `preflight_failed` | `agent_id`, `reason` | Preflight | yes |
| `clarification_needed` | `agent_id`, `questions: string[]` | Preflight | |
| `clarification_received` | `agent_id` | Preflight | |
| `planning_started` | `agent_id` | Planning | |
| `plan_created` | `agent_id`, `step_count`, `rationale` | Planning | |
| `step_started` | `agent_id`, `step_index`, `description` | Step N | |
| `tool_called` | `agent_id`, `step_index`, `tool_name`, `args_preview` | Step N | |
| `tool_result` | `agent_id`, `step_index`, `tool_name`, `success`, `output_preview` | Step N | |
| `step_completed` | `agent_id`, `step_index`, `success`, `summary` | Step N | |
| `step_retrying` | `agent_id`, `step_index`, `delay_secs`, `reason` | Step N | |
| `policy_decision` | `agent_id`, `step_index`, `tool`, `decision`, `rule_id?`, `reason?`, `risk_level` | Step N | |
| `pii_redacted` | `agent_id`, `step_index`, `tool`, `fields_redacted: string[]` | Step N | |
| `sla_check` | `agent_id`, `pct_elapsed`, `message`, `action?`, `deadline?` | Step N | |
| `citation_recorded` | `agent_id`, `step_index`, `claim`, `source_ref`, `source_type`, `confidence` | Step N | |
| `evidence_packaged` | `agent_id`, `citations`, `audit_entries` | Completion | |
| `review_required` | `agent_id`, `review_id`, `summary`, `reason`, `rule_id?` | Step N | |
| `connector_trigger` | `agent_id`, `connector_type`, `event_type`, `external_id?` | (prepended) | |
| `child_spawned` | `agent_id`, `child_agent_id`, `sub_goal` | Delegation | |
| `children_complete` | `agent_id`, `child_ids: string[]` | Delegation | |
| `goal_complete` | `agent_id`, `summary` | Completion | yes |
| `goal_failed` | `agent_id`, `reason` | Completion | yes |
| `lag` | `agent_id`, `missed` | (inline) | |

**Enum values:** `decision`: allow/block/require_approval/redact/downgrade. `source_type`: tool_output/document/url/memory/user_input. `sla action`: escalate/notify.

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
# Core
NARAYAN__DATABASE__URL         PostgreSQL connection string
NARAYAN__REDIS__URL            Redis connection string
NARAYAN__REDIS__ENABLED        true | false
NARAYAN__WORKER__POOL_SIZE     Worker pool size (default: 32)
NARAYAN_JWT_SECRET             JWT signing key (min 32 chars)
NARAYAN_ENCRYPT_KEY            AES-256-GCM passphrase (min 32 chars)
NARAYAN_ADMIN_TOKEN            Admin API token
NARAYAN_BASE_URL               Public URL (for OAuth callbacks)
NARAYAN_UI_URL                 Frontend URL (for post-OAuth redirects)

# Embeddings
NARAYAN_EMBED_PROVIDER         openai | anthropic | ollama | stub
NARAYAN_EMBED_API_KEY          Embedding API key
NARAYAN_EMBED_MODEL            Embedding model name

# Browser
NARAYAN_BROWSER_POOL_SIZE      0 to disable browser tools

# Billing
PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET / PAYPAL_WEBHOOK_ID / PAYPAL_SANDBOX
STRIPE_SECRET_KEY / STRIPE_WEBHOOK_SECRET

# OAuth connectors (PROVIDER_CLIENT_ID + PROVIDER_CLIENT_SECRET for each)
# Providers: SLACK, GMAIL/GOOGLE, OUTLOOK/MICROSOFT, SALESFORCE, HUBSPOT, JIRA/ATLASSIAN, NOTION, GITHUB, QUICKBOOKS, DOCUSIGN
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

### Plans

| Plan | Price | Steps/month | Concurrent agents |
|------|-------|-------------|-------------------|
| Free | $0 | 1,000 | 3 |
| Go | $15/mo | 20,000 | 20 |
| Pro | $79/mo | 150,000 | 200 |
| Enterprise | custom | unlimited | unlimited |

All plans get all 20 connectors + full compliance stack. Credit top-ups: **$8 = 5,000 extra steps**.

### Billing routes

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/billing/checkout` | Create checkout session (PayPal/Stripe) → `redirect_url` |
| `GET` | `/billing/subscription` | Current subscription details |
| `POST` | `/billing/subscription/cancel` | Cancel subscription |
| `GET` | `/billing/invoices` | List invoices |
| `GET` | `/billing/credits` | Check extra step credits |
| `POST` | `/billing/credits/purchase` | Buy credit top-up → `redirect_url` |
| `POST` | `/billing/webhooks/:provider` | Receive PayPal/Stripe webhooks (no auth, signature-verified) |

### Adding a new billing provider

1. Create `src/billing/razorpay.rs` implementing `BillingProvider` trait
2. Add `pub mod razorpay;` to `src/billing/mod.rs`
3. In `main.rs`: `.register(Arc::new(RazorpayProvider::from_env()))`
4. The webhook route `/billing/webhooks/razorpay` works automatically

---

## Connector System

### Three connector types

- **Type 1 — Inbound webhook (push):** External system posts to Narayan → agent created
- **Type 2 — MCP tool use (pull):** Agent calls MCP server tools during execution (OAuth token auto-injected)
- **Type 3 — Both:** Triggers agents AND agents use as tool

### All 20 connectors

| Connector | Auth | Triggers agents | MCP tool |
|-----------|------|-----------------|----------|
| GitHub | API key or OAuth | Poll issues/PRs | yes |
| Zendesk | API key | Poll tickets | |
| ServiceNow | API key + URL | Poll incidents | |
| Salesforce | OAuth | Poll opportunities | yes |
| QuickBooks | OAuth | Poll invoices | |
| DocuSign | OAuth | Poll envelopes | |
| PagerDuty | API key | Poll incidents | |
| HubSpot | OAuth | Poll deals | yes |
| Notion | OAuth | Poll database | yes |
| Greenhouse | API key | Poll applications | |
| dbt Cloud | API key + account_id | Poll failed runs | |
| Slack | OAuth | Poll channels | yes |
| Gmail | OAuth | Poll inbox | yes |
| Outlook | OAuth | Poll inbox | yes |
| Google Sheets | OAuth | | yes |
| Google Docs | OAuth | | yes |
| Teams | OAuth | Poll channels | yes |
| Jira | OAuth | Poll issues | yes |
| Confluence | OAuth | | yes |
| Linear | API key | Poll issues | yes |

### Connector install routes

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/connectors` | List installed connectors |
| `GET` | `/auth/oauth/:provider/start` | Redirect to OAuth consent page |
| `GET` | `/auth/oauth/:provider/callback` | OAuth callback → store token → redirect to UI |
| `POST` | `/connectors/:type/install` | Install API-key connector |
| `POST` | `/connectors/:type/webhook-install` | Install webhook connector → returns URL + secret |
| `DELETE` | `/connectors/:type` | Uninstall connector |

OAuth providers: `slack | gmail | google | outlook | microsoft | salesforce | hubspot | jira | atlassian | notion | github | quickbooks | docusign`

### Polling intervals

| Interval | Connectors |
|----------|-----------|
| 2 min | GitHub, Jira/Atlassian, Linear, Zendesk |
| 3 min | ServiceNow, Greenhouse |
| 5 min | Slack, Gmail, Microsoft/Outlook, Salesforce, HubSpot, PagerDuty |
| 10 min | Notion, dbt Cloud |
| 15 min | QuickBooks, DocuSign |

### MCP auto-token injection

When an agent calls `mcp_session` with a known MCP server URL, the tool looks up the stored OAuth token for the matching connector and injects it automatically. URL → connector mappings in `src/tools/mcp_session.rs :: mcp_url_to_connector()`.

### Keeping tenant plan in sync

When a PayPal/Stripe webhook fires `SubscriptionActivated` or `SubscriptionCancelled`:
1. `BillingStore::process_event()` updates `subscriptions` table
2. Calls `UPDATE tenants SET plan=$1` to sync the tenant's active plan
3. Next JWT issued will reflect the new plan
