//! Named connector tools — one per supported external integration.
//!
//! ## Role in the architecture
//!
//! The LLM discovers connectors lazily:
//!   1. Planner writes a step with a connector category hint ("crm")
//!   2. Executor always includes `list_connectors_in_category` in the toolset
//!   3. LLM calls `list_connectors_in_category { category: "crm" }`
//!   4. Executor intercepts → returns names + summaries ("salesforce", "hubspot")
//!   5. LLM decides it needs "salesforce"
//!   6. Executor finds `ConnectorTool { name: "salesforce" }` in the registry,
//!      builds its ToolSpec, and injects it into the next LLM call
//!   7. LLM calls `salesforce { operation: "query_records", params: {...} }`
//!   8. ConnectorTool delegates to mcp_session with the right server URL
//!
//! ## Adding a new connector
//!
//! Add one `ConnectorDef` entry to `ALL_CONNECTORS` below. Nothing else changes.
//! credential_requirements.rs derives its checks from this list automatically.
//! plan_mode.rs uses `keywords` from this list for intent matching.
//! The executor's connector catalogue is also derived from this list.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::time::sleep;

use crate::tools::{mcp_session::McpSessionTool, ParameterSchema, Tool, ToolResult};

// ── ConnectorDef ───────────────────────────────────────────────────────────

/// Static definition of one external connector integration.
pub struct ConnectorDef {
    /// Tool name exposed to the LLM — also the credential provider name.
    pub name: &'static str,
    /// Slash-namespaced category, e.g. "connector/crm".
    pub category: &'static str,
    /// MCP server URL the tool routes through.
    pub mcp_url: &'static str,
    /// One-line summary shown in the connector directory manifest.
    pub summary: &'static str,
    /// Detailed description shown when the LLM receives the full ToolSpec.
    pub description: &'static str,
    /// Available operations — shown to the LLM to guide correct usage.
    pub operations: &'static [&'static str],
    /// Keywords used by ConnectorResolver in plan_mode to match user intent.
    pub keywords: &'static [&'static str],
}

/// All built-in connector definitions — the single source of truth.
///
/// This list drives:
///   - Which tools get registered in the ToolRegistry (register_all_connectors)
///   - Which connectors show up in list_connectors_in_category
///   - Which tool names get red confidence dots when credentials are missing
///   - Which connectors ConnectorResolver can match during plan mode
///   - The KNOWN_CONNECTORS set on the frontend (keep in sync manually)
///
/// To add a new connector: add one entry here. Nothing else changes.
pub static ALL_CONNECTORS: &[ConnectorDef] = &[
    // ── CRM ────────────────────────────────────────────────────────────────
    ConnectorDef {
        name: "salesforce",
        category: "connector/crm",
        mcp_url: "https://mcp.salesforce.com/sse",
        summary: "Salesforce CRM: query leads/contacts/opportunities, update records",
        description: "Interact with Salesforce CRM. Supports SOQL queries on any object \
                       (Lead, Contact, Account, Opportunity, Case), creating and updating \
                       records, logging activity notes, and creating follow-up tasks.",
        operations: &[
            "query_records  — SOQL query, e.g. SELECT Id,Name FROM Lead WHERE Status='New'",
            "get_record     — fetch a single record by Id and object type",
            "create_record  — create Lead, Contact, Opportunity, Task, etc.",
            "update_record  — update fields on an existing record",
            "log_note       — create a Chatter note or activity on a record",
        ],
        keywords: &["crm", "lead", "contact", "opportunity", "account", "salesforce", "deal", "pipeline"],
    },
    ConnectorDef {
        name: "hubspot",
        category: "connector/crm",
        mcp_url: "https://mcp.hubapi.com/sse",
        summary: "HubSpot CRM: contacts, deals, companies, activities",
        description: "Interact with HubSpot CRM. Create or update contacts, companies, \
                       and deals; add notes and activities; search by any property.",
        operations: &[
            "search_contacts — find contacts by email, name, or property",
            "create_contact  — create a new contact",
            "update_deal     — update deal stage or properties",
            "add_note        — add a note to a contact or deal",
        ],
        keywords: &["crm", "hubspot", "contact", "deal", "company", "marketing", "inbound"],
    },
    // ── Customer support ───────────────────────────────────────────────────
    ConnectorDef {
        name: "zendesk",
        category: "connector/support",
        mcp_url: "https://mcp.zendesk.com/sse",
        summary: "Zendesk: tickets, agents, customers, macros",
        description: "Interact with Zendesk Support. Query and update tickets, add comments, \
                       assign agents, apply macros, look up customers.",
        operations: &[
            "list_tickets    — list tickets with optional status/priority filter",
            "get_ticket      — fetch a ticket by ID",
            "create_ticket   — open a new support ticket",
            "update_ticket   — update status, assignee, priority, or fields",
            "add_comment     — add a public or internal comment to a ticket",
        ],
        keywords: &["zendesk", "ticket", "support", "helpdesk", "customer service", "agent", "issue"],
    },
    ConnectorDef {
        name: "intercom",
        category: "connector/support",
        mcp_url: "https://api.intercom.io/mcp/sse",
        summary: "Intercom: conversations, contacts, articles",
        description: "Interact with Intercom. Query and reply to conversations, look up contacts, \
                       add notes, create tickets, search the help centre.",
        operations: &[
            "list_conversations — list open or unassigned conversations",
            "get_conversation   — fetch a conversation by ID",
            "reply              — send a reply in a conversation",
            "create_note        — add an internal note to a conversation",
            "search_contacts    — find contacts by email or name",
        ],
        keywords: &["intercom", "conversation", "support", "chat", "customer", "helpdesk", "ticket", "inbox"],
    },
    ConnectorDef {
        name: "freshdesk",
        category: "connector/support",
        mcp_url: "https://mcp.freshdesk.com/sse",
        summary: "Freshdesk: tickets, agents, contacts",
        description: "Interact with Freshdesk. Create and update tickets, add notes, \
                       assign agents, look up contacts and companies.",
        operations: &[
            "list_tickets   — list tickets with filters",
            "create_ticket  — open a new ticket",
            "update_ticket  — update status, priority, or assignee",
            "add_note       — add a private or public note",
            "get_contact    — look up a contact by email",
        ],
        keywords: &["freshdesk", "ticket", "support", "helpdesk", "fresh", "customer service"],
    },
    // ── Developer tools ─────────────────────────────────────────────────────
    ConnectorDef {
        name: "github",
        category: "connector/devtools",
        mcp_url: "https://api.githubcopilot.com/mcp/sse",
        summary: "GitHub: repos, PRs, issues, commits, CI workflows",
        description: "Interact with GitHub. Read file contents, list/create/update issues, \
                       open/review/merge pull requests, push commits, trigger workflows.",
        operations: &[
            "get_file       — read a file from a repo at a given path/ref",
            "list_issues    — list open issues with optional label/milestone filters",
            "create_issue   — open a new issue",
            "create_pr      — open a pull request",
            "merge_pr       — merge a pull request",
            "push_commit    — push file changes as a commit",
            "run_workflow   — trigger a GitHub Actions workflow",
        ],
        keywords: &["github", "pr", "pull request", "issue", "code", "repo", "commit", "ci", "git"],
    },
    // ── Project management ──────────────────────────────────────────────────
    ConnectorDef {
        name: "jira",
        category: "connector/project_management",
        mcp_url: "https://mcp.atlassian.com/sse",
        summary: "Jira: issues, sprints, boards, comments",
        description: "Interact with Jira. Search issues with JQL, create bugs/stories/tasks, \
                       update status/assignee/priority, add comments, manage sprints.",
        operations: &[
            "search_issues  — JQL search, e.g. project=ENG AND status=Open",
            "get_issue      — fetch issue details by key (e.g. ENG-123)",
            "create_issue   — create a new bug, story, or task",
            "update_issue   — change status, assignee, priority, or fields",
            "add_comment    — add a comment to an issue",
        ],
        keywords: &["jira", "ticket", "issue", "sprint", "board", "task", "bug", "story", "atlassian"],
    },
    ConnectorDef {
        name: "notion",
        category: "connector/project_management",
        mcp_url: "https://mcp.notion.com/sse",
        summary: "Notion: pages, databases, wiki",
        description: "Interact with Notion. Search pages, read and append content, \
                       create database entries, update page properties.",
        operations: &[
            "search_pages   — search by title or content keyword",
            "get_page       — read a page's full content blocks",
            "create_page    — create a new page or database entry",
            "append_block   — append content blocks to a page",
            "update_props   — update database entry properties",
        ],
        keywords: &["notion", "page", "database", "wiki", "doc", "knowledge", "workspace"],
    },
    ConnectorDef {
        name: "asana",
        category: "connector/project_management",
        mcp_url: "https://mcp.asana.com/sse",
        summary: "Asana: tasks, projects, sections",
        description: "Interact with Asana. List, create and update tasks; add comments; \
                       manage project sections and due dates.",
        operations: &[
            "list_tasks     — list tasks in a project or assigned to a user",
            "create_task    — create a new task with optional due date",
            "update_task    — update status, assignee, or due date",
            "add_comment    — add a comment to a task",
        ],
        keywords: &["asana", "task", "project", "milestone", "team", "workflow"],
    },
    // ── Communication ───────────────────────────────────────────────────────
    ConnectorDef {
        name: "linear",
        category: "connector/project_management",
        mcp_url: "https://mcp.linear.app/mcp",
        summary: "Linear: issues, projects, comments, triage",
        description: "Interact with Linear. List issues, create and update issues, add comments, \
                       and inspect project work in a fast issue-tracking workflow.",
        operations: &[
            "list_issues   â€” list recent issues, optionally filter by team or text",
            "create_issue  â€” create a new issue in a team",
            "update_issue  â€” update title or description of an issue",
            "add_comment   â€” add a comment to an issue",
        ],
        keywords: &["linear", "issue", "project", "product", "bug", "task", "triage", "roadmap"],
    },
    ConnectorDef {
        name: "monday",
        category: "connector/project_management",
        mcp_url: "https://mcp.monday.com/sse",
        summary: "monday.com: boards, items, updates, workflows",
        description: "Interact with monday.com. List boards, create items, update item columns, \
                       and add updates to keep teams aligned.",
        operations: &[
            "list_boards  â€” list accessible boards and their names",
            "create_item  â€” create a new item on a board",
            "update_item  â€” update multiple column values on an item",
            "add_update   â€” add an update/comment to an item",
        ],
        keywords: &["monday", "board", "item", "workflow", "project", "task", "tracker", "crm"],
    },
    ConnectorDef {
        name: "slack",
        category: "connector/communication",
        mcp_url: "https://mcp.slack.com/sse",
        summary: "Slack: send messages, read channels, DMs",
        description: "Interact with Slack. Send messages to channels or users, \
                       read recent messages, create threads, look up user info.",
        operations: &[
            "send_message   — post a message to a channel or DM",
            "list_messages  — read recent messages from a channel",
            "reply_thread   — reply in a thread",
            "lookup_user    — find a user by name or email",
        ],
        keywords: &["slack", "message", "channel", "notify", "alert", "dm", "chat", "notification"],
    },
    ConnectorDef {
        name: "gmail",
        category: "connector/communication",
        mcp_url: "https://gmail.mcp.claude.com/mcp",
        summary: "Gmail: read, send, and organise email",
        description: "Interact with Gmail. Read inbox and threads, send emails, \
                       search messages, apply labels, manage drafts.",
        operations: &[
            "list_messages  — list inbox messages with optional query",
            "get_message    — read a message and its thread",
            "send_email     — send a new email",
            "create_draft   — create a draft without sending",
            "search         — search messages with Gmail query syntax",
        ],
        keywords: &["gmail", "email", "inbox", "send", "draft", "google mail"],
    },
    ConnectorDef {
        name: "outlook",
        category: "connector/communication",
        mcp_url: "https://graph.microsoft.com/mcp/sse",
        summary: "Outlook / Microsoft 365: email, calendar, contacts",
        description: "Interact with Outlook via Microsoft Graph. Read and send email, \
                       manage calendar events, look up contacts.",
        operations: &[
            "list_messages  — list inbox or folder messages",
            "send_email     — send a new email",
            "create_draft   — save a draft",
            "list_events    — list calendar events",
            "create_event   — create a calendar event",
        ],
        keywords: &["outlook", "email", "microsoft", "office 365", "m365", "exchange", "calendar"],
    },
    // ── Finance ─────────────────────────────────────────────────────────────
    ConnectorDef {
        name: "quickbooks",
        category: "connector/finance",
        mcp_url: "https://mcp.intuit.com/quickbooks/sse",
        summary: "QuickBooks: invoices, expenses, P&L reports",
        description: "Interact with QuickBooks Online. Query invoices, bills, expenses, \
                       and customers; create invoices; pull financial reports.",
        operations: &[
            "query          — run a QuickBooks query, e.g. SELECT * FROM Invoice WHERE Balance > 0",
            "create_invoice — create a new customer invoice",
            "get_report     — pull ProfitAndLoss, BalanceSheet, or CashFlow report",
        ],
        keywords: &["quickbooks", "invoice", "expense", "accounting", "billing", "payment", "finance"],
    },
    ConnectorDef {
        name: "stripe",
        category: "connector/finance",
        mcp_url: "https://mcp.stripe.com/sse",
        summary: "Stripe: payments, customers, subscriptions, invoices",
        description: "Interact with Stripe. Query charges, customers, subscriptions, \
                       and invoices; create payment links; look up failed payments.",
        operations: &[
            "list_charges      — list recent charges with optional filters",
            "get_customer      — fetch a customer by ID or email",
            "list_invoices     — list invoices for a customer",
            "list_subscriptions— list active subscriptions",
            "create_payment_link— generate a payment link",
        ],
        keywords: &["stripe", "payment", "charge", "subscription", "invoice", "billing", "revenue"],
    },
    // ── IT service management ────────────────────────────────────────────────
    ConnectorDef {
        name: "servicenow",
        category: "connector/itsm",
        mcp_url: "https://mcp.service-now.com/sse",
        summary: "ServiceNow: incidents, change requests, CMDB",
        description: "Interact with ServiceNow. Query incidents, changes, and config items; \
                       create and update records; add work notes.",
        operations: &[
            "query_records   — query any table with optional sys_class filter",
            "get_record      — fetch a record by sys_id",
            "create_incident — open a new incident",
            "update_record   — update fields on any record",
            "add_work_note   — add a work note to an incident or change",
        ],
        keywords: &["servicenow", "incident", "change", "itsm", "ticket", "service desk", "cmdb"],
    },
    ConnectorDef {
        name: "pagerduty",
        category: "connector/itsm",
        mcp_url: "https://mcp.pagerduty.com/sse",
        summary: "PagerDuty: incidents, alerts, on-call schedules",
        description: "Interact with PagerDuty. List and acknowledge incidents, trigger \
                       new alerts, check on-call schedules, add notes.",
        operations: &[
            "list_incidents   — list triggered or acknowledged incidents",
            "trigger_incident — create a new incident",
            "acknowledge      — acknowledge an incident",
            "add_note         — add a note to an incident",
            "get_oncall       — get current on-call person for a schedule",
        ],
        keywords: &["pagerduty", "alert", "oncall", "incident", "escalate", "page", "on-call"],
    },
    // ── HR ───────────────────────────────────────────────────────────────────
    ConnectorDef {
        name: "greenhouse",
        category: "connector/hr",
        mcp_url: "https://harvest.greenhouse.io/mcp/sse",
        summary: "Greenhouse ATS: jobs, candidates, applications",
        description: "Interact with Greenhouse ATS. List open jobs and candidates, \
                       get application details, add notes, advance or reject candidates.",
        operations: &[
            "list_jobs        — list open job postings",
            "list_candidates  — list candidates for a job",
            "get_application  — get full application + interview details",
            "add_note         — add a note to a candidate profile",
            "advance_stage    — move candidate to next interview stage",
        ],
        keywords: &["greenhouse", "recruit", "candidate", "hiring", "ats", "job", "hr", "applicant"],
    },
    // ── Legal ────────────────────────────────────────────────────────────────
    ConnectorDef {
        name: "docusign",
        category: "connector/legal",
        mcp_url: "https://mcp.docusign.net/sse",
        summary: "DocuSign: envelopes, signatures, status tracking",
        description: "Interact with DocuSign. Create signature envelopes, add recipients \
                       and fields, send for signature, check status.",
        operations: &[
            "create_envelope — create a new envelope with documents",
            "send_envelope   — send an envelope to recipients",
            "get_status      — check envelope status and signer actions",
            "void_envelope   — void an in-progress envelope",
        ],
        keywords: &["docusign", "signature", "contract", "envelope", "sign", "esign", "legal"],
    },
    // ── Data pipelines ───────────────────────────────────────────────────────
    ConnectorDef {
        name: "dbt_cloud",
        category: "connector/data",
        mcp_url: "https://cloud.getdbt.com/mcp/sse",
        summary: "dbt Cloud: trigger runs, check status, list models",
        description: "Interact with dbt Cloud. List jobs and runs, trigger a job run, \
                       get run logs and results, check model status.",
        operations: &[
            "list_jobs   — list all jobs in a project",
            "trigger_run — trigger a specific job to run",
            "get_run     — get status and logs of a run",
            "list_models — list models and their last run status",
        ],
        keywords: &["dbt", "transform", "pipeline", "datawarehouse", "model", "run", "analytics"],
    },
];

/// Return the `(category_suffix, name, summary)` tuple used by the executor's
/// connector catalogue for fast, allocation-free lookups.
/// Derives from ALL_CONNECTORS so there is a single source of truth.
pub fn catalogue_entries() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    ALL_CONNECTORS.iter().map(|def| {
        let cat_suffix = def.category.strip_prefix("connector/").unwrap_or(def.category);
        (cat_suffix, def.name, def.summary)
    })
}

/// Look up a `ConnectorDef` by tool name. O(n) — catalogue is small.
pub fn find_by_name(name: &str) -> Option<&'static ConnectorDef> {
    ALL_CONNECTORS.iter().find(|d| d.name == name)
}

// ── REST execution router ──────────────────────────────────────────────────
//
// Routes `operation` strings to real REST API calls for each connector.
// Each connector section handles all operations declared in ALL_CONNECTORS.
// Returns serde_json::Value of the API response body.

fn with_idempotency(builder: reqwest::RequestBuilder, idempotency_key: Option<&str>) -> reqwest::RequestBuilder {
    match idempotency_key.filter(|value| !value.trim().is_empty()) {
        Some(key) => builder.header("Idempotency-Key", key),
        None => builder,
    }
}

fn derive_idempotency_key(
    connector: &str,
    tenant_id: &str,
    goal_instance_id: Option<&str>,
    step_index: Option<u64>,
    operation: &str,
    params: &serde_json::Value,
) -> String {
    let normalized = serde_json::json!({
        "connector": connector,
        "tenant_id": tenant_id,
        "goal_instance_id": goal_instance_id,
        "step_index": step_index,
        "operation": operation,
        "params": params,
    });
    let bytes = serde_json::to_vec(&normalized).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    format!("narayan-{}", hex::encode(digest))
}

fn retryable_graphql_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("temporarily unavailable")
        || lower.contains("service unavailable")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
}

#[derive(Debug, Clone, Copy)]
enum GraphqlAuthMode {
    AuthorizationHeader,
}

async fn graphql_execute_with_retry(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    auth_mode: GraphqlAuthMode,
    query: &str,
    variables: serde_json::Value,
    idempotency_key: Option<&str>,
    connector: &str,
    operation: &str,
    api_version: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    let payload = serde_json::json!({
        "query": query,
        "variables": variables,
    });

    let mut attempt = 0usize;
    let mut delay = Duration::from_millis(250);
    loop {
        attempt += 1;

        let mut request = http.post(url);
        request = match auth_mode {
            GraphqlAuthMode::AuthorizationHeader => request.header("Authorization", token),
        };
        if let Some(version) = api_version {
            request = request.header("API-Version", version);
        }
        request = with_idempotency(request, idempotency_key);

        let response = match request.json(&payload).send().await {
            Ok(resp) => resp,
            Err(err) if attempt < 4 && retryable_graphql_error(&err.to_string()) => {
                sleep(delay).await;
                delay = delay.saturating_mul(2);
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) && attempt < 4 {
            sleep(delay).await;
            delay = delay.saturating_mul(2);
            continue;
        }

        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "{} {}: invalid GraphQL response (HTTP {}): {} ({})",
                    connector,
                    operation,
                    status,
                    text,
                    err
                ));
            }
        };

        if let Some(errors) = json.get("errors") {
            let error_text = serde_json::to_string(errors).unwrap_or_default();
            if attempt < 4 && retryable_graphql_error(&error_text) {
                sleep(delay).await;
                delay = delay.saturating_mul(2);
                continue;
            }
            return Err(anyhow::anyhow!("{} {} GraphQL error: {}", connector, operation, error_text));
        }

        if !status.is_success() {
            return Err(anyhow::anyhow!("{} {} HTTP {}: {}", connector, operation, status, text));
        }

        return Ok(json.get("data").cloned().unwrap_or_default());
    }
}

async fn rest_execute(
    http: &reqwest::Client,
    connector: &str,
    token: &str,
    operation: &str,
    params: &serde_json::Value,
    settings: &serde_json::Value,
    idempotency_key: Option<&str>,
) -> anyhow::Result<serde_json::Value> {
    match connector {
        // ── Salesforce ─────────────────────────────────────────────────────
        "salesforce" => {
            let instance = settings["instance_url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Salesforce: missing instance_url in settings"))?;
            let base = format!("{}/services/data/v58.0", instance);

            match operation {
                "query_records" => {
                    let soql = params["soql"].as_str().unwrap_or("SELECT Id,Name FROM Lead LIMIT 10");
                    let url = format!("{}/query?q={}", base, urlencoding::encode(soql));
                    let r = http.get(&url).bearer_auth(token).send().await?;
                    Ok(r.json().await?)
                }
                "get_record" => {
                    let id = params["id"].as_str().ok_or_else(|| anyhow::anyhow!("id required"))?;
                    let obj = params["object_type"].as_str().unwrap_or("Lead");
                    let url = format!("{}/sobjects/{}/{}", base, obj, id);
                    Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
                }
                "create_record" => {
                    let obj = params["object_type"].as_str().unwrap_or("Lead");
                    let body = params.get("fields").cloned().unwrap_or_default();
                    let url = format!("{}/sobjects/{}", base, obj);
                    Ok(http.post(&url).bearer_auth(token).json(&body).send().await?.json().await?)
                }
                "update_record" => {
                    let id = params["id"].as_str().ok_or_else(|| anyhow::anyhow!("id required"))?;
                    let obj = params["object_type"].as_str().unwrap_or("Lead");
                    let body = params.get("fields").cloned().unwrap_or_default();
                    let url = format!("{}/sobjects/{}/{}", base, obj, id);
                    http.patch(&url).bearer_auth(token).json(&body).send().await?;
                    Ok(serde_json::json!({"updated": true, "id": id}))
                }
                "log_note" => {
                    let parent_id = params["id"].as_str().ok_or_else(|| anyhow::anyhow!("id required"))?;
                    let body_text = params["body"].as_str().unwrap_or("");
                    let url = format!("{}/sobjects/Note", base);
                    let r = http
                        .post(&url)
                        .bearer_auth(token)
                        .json(&serde_json::json!({"ParentId": parent_id, "Title": "Narayan Note", "Body": body_text}))
                        .send()
                        .await?;
                    Ok(r.json().await?)
                }
                _ => anyhow::bail!("Salesforce: unknown operation '{}'", operation),
            }
        }

        // ── HubSpot ────────────────────────────────────────────────────────
        "hubspot" => {
            let base = "https://api.hubapi.com/crm/v3";
            match operation {
                "search_contacts" => {
                    let q = params["query"].as_str().unwrap_or("");
                    let url = format!("{}/objects/contacts/search", base);
                    let r = http
                        .post(&url)
                        .bearer_auth(token)
                        .json(&serde_json::json!({"query": q, "limit": 10}))
                        .send()
                        .await?;
                    Ok(r.json().await?)
                }
                "create_contact" => {
                    let url = format!("{}/objects/contacts", base);
                    let props = params.get("properties").cloned().unwrap_or_default();
                    Ok(http
                        .post(&url)
                        .bearer_auth(token)
                        .json(&serde_json::json!({"properties": props}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "update_deal" => {
                    let id = params["id"].as_str().ok_or_else(|| anyhow::anyhow!("id required"))?;
                    let props = params.get("properties").cloned().unwrap_or_default();
                    let url = format!("{}/objects/deals/{}", base, id);
                    Ok(http
                        .patch(&url)
                        .bearer_auth(token)
                        .json(&serde_json::json!({"properties": props}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "add_note" => {
                    let url = format!("{}/objects/notes", base);
                    let body = serde_json::json!({
                        "properties": {
                            "hs_note_body": params["body"].as_str().unwrap_or(""),
                            "hs_timestamp": chrono::Utc::now().timestamp_millis(),
                        },
                        "associations": params.get("associations").cloned().unwrap_or_default(),
                    });
                    Ok(http.post(&url).bearer_auth(token).json(&body).send().await?.json().await?)
                }
                _ => anyhow::bail!("HubSpot: unknown operation '{}'", operation),
            }
        }

        // ── GitHub ─────────────────────────────────────────────────────────
        "github" => {
            let repo = settings["repo"].as_str().unwrap_or(params["repo"].as_str().unwrap_or(""));
            let base = format!("https://api.github.com/repos/{}", repo);

            match operation {
                "get_file" => {
                    let path = params["path"].as_str().ok_or_else(|| anyhow::anyhow!("path required"))?;
                    let r = http
                        .get(&format!("{}/contents/{}", base, path))
                        .bearer_auth(token)
                        .header("Accept", "application/vnd.github.v3+json")
                        .header("User-Agent", "narayan-agent")
                        .send()
                        .await?;
                    Ok(r.json().await?)
                }
                "list_issues" => {
                    let state = params["state"].as_str().unwrap_or("open");
                    let labels = params["labels"].as_str().unwrap_or("");
                    let url = format!("{}/issues?state={}&labels={}", base, state, labels);
                    Ok(http
                        .get(&url)
                        .bearer_auth(token)
                        .header("User-Agent", "narayan-agent")
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "create_issue" => {
                    let body = serde_json::json!({
                        "title": params["title"].as_str().unwrap_or("New issue"),
                        "body":  params["body"].as_str().unwrap_or(""),
                        "labels": params.get("labels").cloned().unwrap_or_default(),
                    });
                    Ok(http
                        .post(&format!("{}/issues", base))
                        .bearer_auth(token)
                        .header("User-Agent", "narayan-agent")
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "create_pr" => {
                    let body = serde_json::json!({
                        "title": params["title"],
                        "head":  params["head"],
                        "base":  params.get("base").and_then(|v| v.as_str()).unwrap_or("main"),
                        "body":  params.get("body").and_then(|v| v.as_str()).unwrap_or(""),
                    });
                    Ok(http
                        .post(&format!("{}/pulls", base))
                        .bearer_auth(token)
                        .header("User-Agent", "narayan-agent")
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "merge_pr" => {
                    let pr = params["pr_number"].as_u64().ok_or_else(|| anyhow::anyhow!("pr_number required"))?;
                    Ok(http
                        .put(&format!("{}/pulls/{}/merge", base, pr))
                        .bearer_auth(token)
                        .header("User-Agent", "narayan-agent")
                        .json(&serde_json::json!({"commit_title": params.get("commit_title")}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "push_commit" => {
                    // Create/update a file via the GitHub Contents API
                    let path = params["path"].as_str().ok_or_else(|| anyhow::anyhow!("path required"))?;
                    let content = params["content"].as_str().ok_or_else(|| anyhow::anyhow!("content required"))?;
                    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, content);
                    let sha = params.get("sha").and_then(|v| v.as_str());
                    let mut body = serde_json::json!({
                        "message": params.get("message").and_then(|v| v.as_str()).unwrap_or("Update via Narayan"),
                        "content": encoded,
                    });
                    if let Some(s) = sha {
                        body["sha"] = serde_json::json!(s);
                    }
                    Ok(http
                        .put(&format!("{}/contents/{}", base, path))
                        .bearer_auth(token)
                        .header("User-Agent", "narayan-agent")
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "run_workflow" => {
                    let workflow =
                        params["workflow_id"].as_str().ok_or_else(|| anyhow::anyhow!("workflow_id required"))?;
                    let r#ref = params.get("ref").and_then(|v| v.as_str()).unwrap_or("main");
                    http.post(&format!("{}/actions/workflows/{}/dispatches", base, workflow))
                        .bearer_auth(token)
                        .header("User-Agent", "narayan-agent")
                        .json(&serde_json::json!({"ref": r#ref}))
                        .send()
                        .await?;
                    Ok(serde_json::json!({"dispatched": true}))
                }
                _ => anyhow::bail!("GitHub: unknown operation '{}'", operation),
            }
        }

        // ── Jira ───────────────────────────────────────────────────────────
        "jira" => {
            let cloud_url = settings["cloud_url"]
                .as_str()
                .or_else(|| params["cloud_url"].as_str())
                .unwrap_or("https://your-domain.atlassian.net");
            let base = format!("{}/rest/api/3", cloud_url);

            match operation {
                "search_issues" => {
                    let jql = params["jql"].as_str().unwrap_or("project is not EMPTY ORDER BY updated DESC");
                    let url = format!("{}/search?jql={}&maxResults=20", base, urlencoding::encode(jql));
                    Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
                }
                "get_issue" => {
                    let key = params["key"].as_str().ok_or_else(|| anyhow::anyhow!("key required"))?;
                    Ok(http.get(&format!("{}/issue/{}", base, key)).bearer_auth(token).send().await?.json().await?)
                }
                "create_issue" => {
                    let body = serde_json::json!({
                        "fields": {
                            "project":     {"key": params["project"].as_str().unwrap_or("")},
                            "summary":     params["summary"].as_str().unwrap_or(""),
                            "description": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": params["description"].as_str().unwrap_or("")}]}]},
                            "issuetype":   {"name": params.get("issue_type").and_then(|v| v.as_str()).unwrap_or("Task")},
                        }
                    });
                    Ok(http
                        .post(&format!("{}/issue", base))
                        .bearer_auth(token)
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "update_issue" => {
                    let key = params["key"].as_str().ok_or_else(|| anyhow::anyhow!("key required"))?;
                    let fields = params.get("fields").cloned().unwrap_or_default();
                    http.put(&format!("{}/issue/{}", base, key))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"fields": fields}))
                        .send()
                        .await?;
                    Ok(serde_json::json!({"updated": true, "key": key}))
                }
                "add_comment" => {
                    let key = params["key"].as_str().ok_or_else(|| anyhow::anyhow!("key required"))?;
                    let text = params["body"].as_str().unwrap_or("");
                    let body = serde_json::json!({"body": {"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]}});
                    Ok(http
                        .post(&format!("{}/issue/{}/comment", base, key))
                        .bearer_auth(token)
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Jira: unknown operation '{}'", operation),
            }
        }

        // ── Slack ──────────────────────────────────────────────────────────
        "linear" => {
            let url = "https://api.linear.app/graphql";
            match operation {
                "list_issues" => {
                    let query = params["query"].as_str().unwrap_or("").trim().to_string();
                    let team_id = params["team_id"].as_str().unwrap_or("").trim().to_string();
                    let limit = params["limit"].as_u64().unwrap_or(20) as usize;
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        query {
                          issues {
                            nodes {
                              id
                              identifier
                              title
                              description
                              url
                              priority
                              team { id key name }
                            }
                          }
                        }
                        "#,
                        serde_json::json!({}),
                        idempotency_key,
                        "Linear",
                        "list_issues",
                        None,
                    )
                    .await?;
                    let mut issues = data
                        .get("issues")
                        .and_then(|value| value.get("nodes"))
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if !team_id.is_empty() {
                        issues.retain(|issue| {
                            let team = issue.get("team").and_then(|v| v.as_object());
                            let matches_id =
                                team.and_then(|team| team.get("id")).and_then(|v| v.as_str()) == Some(team_id.as_str());
                            let matches_key = team.and_then(|team| team.get("key")).and_then(|v| v.as_str())
                                == Some(team_id.as_str());
                            matches_id || matches_key
                        });
                    }
                    if !query.is_empty() {
                        let needle = query.to_ascii_lowercase();
                        issues.retain(|issue| {
                            let haystack = [
                                issue.get("identifier").and_then(|v| v.as_str()).unwrap_or(""),
                                issue.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                issue.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                            ]
                            .join(" ")
                            .to_ascii_lowercase();
                            haystack.contains(&needle)
                        });
                    }
                    if issues.len() > limit {
                        issues.truncate(limit);
                    }
                    Ok(serde_json::json!({
                        "issues": issues,
                        "count": issues.len(),
                        "team_id": team_id,
                        "query": query,
                    }))
                }
                "create_issue" => {
                    let team_id = params["team_id"].as_str().ok_or_else(|| anyhow::anyhow!("team_id required"))?;
                    let title = params["title"].as_str().unwrap_or("New issue");
                    let description = params["description"].as_str().unwrap_or("");
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        mutation($teamId: String!, $title: String!, $description: String) {
                          issueCreate(input: { teamId: $teamId, title: $title, description: $description }) {
                            success
                            issue { id identifier title url }
                          }
                        }
                        "#,
                        serde_json::json!({
                            "teamId": team_id,
                            "title": title,
                            "description": description,
                        }),
                        idempotency_key,
                        "Linear",
                        "create_issue",
                        None,
                    )
                    .await?;
                    Ok(serde_json::json!({
                        "created": true,
                        "issue": data.get("issueCreate").and_then(|v| v.get("issue")).cloned().unwrap_or_default(),
                        "result": data.get("issueCreate").cloned().unwrap_or_default(),
                    }))
                }
                "update_issue" => {
                    let issue_id = params["issue_id"].as_str().ok_or_else(|| anyhow::anyhow!("issue_id required"))?;
                    let title = params["title"].as_str().unwrap_or("");
                    let description = params["description"].as_str().unwrap_or("");
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        mutation($issueId: String!, $title: String, $description: String) {
                          issueUpdate(id: $issueId, input: { title: $title, description: $description }) {
                            success
                            issue { id identifier title url }
                          }
                        }
                        "#,
                        serde_json::json!({
                            "issueId": issue_id,
                            "title": title,
                            "description": description,
                        }),
                        idempotency_key,
                        "Linear",
                        "update_issue",
                        None,
                    )
                    .await?;
                    Ok(serde_json::json!({
                        "updated": true,
                        "issue": data.get("issueUpdate").and_then(|v| v.get("issue")).cloned().unwrap_or_default(),
                        "result": data.get("issueUpdate").cloned().unwrap_or_default(),
                    }))
                }
                "add_comment" => {
                    let issue_id = params["issue_id"].as_str().ok_or_else(|| anyhow::anyhow!("issue_id required"))?;
                    let body = params["body"].as_str().unwrap_or("");
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        mutation($issueId: String!, $body: String!) {
                          commentCreate(input: { issueId: $issueId, body: $body }) {
                            success
                            comment { id body }
                          }
                        }
                        "#,
                        serde_json::json!({
                            "issueId": issue_id,
                            "body": body,
                        }),
                        idempotency_key,
                        "Linear",
                        "add_comment",
                        None,
                    )
                    .await?;
                    Ok(serde_json::json!({
                        "added": true,
                        "comment": data.get("commentCreate").and_then(|v| v.get("comment")).cloned().unwrap_or_default(),
                        "result": data.get("commentCreate").cloned().unwrap_or_default(),
                    }))
                }
                _ => anyhow::bail!("Linear: unknown operation '{}'", operation),
            }
        }

        "monday" => {
            let url = "https://api.monday.com/v2";
            match operation {
                "list_boards" => {
                    let query = params["query"].as_str().unwrap_or("").trim().to_string();
                    let limit = params["limit"].as_u64().unwrap_or(20) as usize;
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        query {
                          boards {
                            id
                            name
                            description
                            state
                          }
                        }
                        "#,
                        serde_json::json!({}),
                        idempotency_key,
                        "monday.com",
                        "list_boards",
                        Some("2025-10"),
                    )
                    .await?;
                    let mut boards = data.get("boards").and_then(|value| value.as_array()).cloned().unwrap_or_default();
                    if !query.is_empty() {
                        let needle = query.to_ascii_lowercase();
                        boards.retain(|board| {
                            let haystack = [
                                board.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                board.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                            ]
                            .join(" ")
                            .to_ascii_lowercase();
                            haystack.contains(&needle)
                        });
                    }
                    if boards.len() > limit {
                        boards.truncate(limit);
                    }
                    Ok(serde_json::json!({
                        "boards": boards,
                        "count": boards.len(),
                        "query": query,
                    }))
                }
                "create_item" => {
                    let board_id = params["board_id"].as_str().ok_or_else(|| anyhow::anyhow!("board_id required"))?;
                    let item_name = params["item_name"].as_str().unwrap_or("New item");
                    let group_id = params["group_id"].as_str().unwrap_or("");
                    let column_values = params.get("column_values").cloned().unwrap_or_default();
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        mutation($boardId: ID!, $itemName: String!, $groupId: String, $columnValues: JSON) {
                          create_item(board_id: $boardId, item_name: $itemName, group_id: $groupId, column_values: $columnValues) {
                            id
                            name
                          }
                        }
                        "#,
                        serde_json::json!({
                            "boardId": board_id,
                            "itemName": item_name,
                            "groupId": if group_id.is_empty() { serde_json::Value::Null } else { serde_json::json!(group_id) },
                            "columnValues": if column_values.is_null() {
                                serde_json::Value::Null
                            } else if let Some(s) = column_values.as_str() {
                                serde_json::json!(s)
                            } else {
                                serde_json::json!(column_values)
                            },
                        }),
                        idempotency_key,
                        "monday.com",
                        "create_item",
                        Some("2025-10"),
                    )
                    .await?;
                    Ok(serde_json::json!({
                        "created": true,
                        "item": data.get("create_item").cloned().unwrap_or_default(),
                        "result": data.get("create_item").cloned().unwrap_or_default(),
                    }))
                }
                "update_item" => {
                    let board_id = params["board_id"].as_str().ok_or_else(|| anyhow::anyhow!("board_id required"))?;
                    let item_id = params["item_id"].as_str().ok_or_else(|| anyhow::anyhow!("item_id required"))?;
                    let column_values = params.get("column_values").cloned().unwrap_or_default();
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        mutation($boardId: ID!, $itemId: ID!, $columnValues: JSON) {
                          change_multiple_column_values(board_id: $boardId, item_id: $itemId, column_values: $columnValues) {
                            id
                          }
                        }
                        "#,
                        serde_json::json!({
                            "boardId": board_id,
                            "itemId": item_id,
                            "columnValues": if column_values.is_null() {
                                serde_json::Value::Null
                            } else if let Some(s) = column_values.as_str() {
                                serde_json::json!(s)
                            } else {
                                serde_json::json!(column_values)
                            },
                        }),
                        idempotency_key,
                        "monday.com",
                        "update_item",
                        Some("2025-10"),
                    )
                    .await?;
                    Ok(serde_json::json!({
                        "updated": true,
                        "item": data.get("change_multiple_column_values").cloned().unwrap_or_default(),
                        "result": data.get("change_multiple_column_values").cloned().unwrap_or_default(),
                    }))
                }
                "add_update" => {
                    let item_id = params["item_id"].as_str().ok_or_else(|| anyhow::anyhow!("item_id required"))?;
                    let body = params["body"].as_str().unwrap_or("");
                    let data = graphql_execute_with_retry(
                        http,
                        url,
                        token,
                        GraphqlAuthMode::AuthorizationHeader,
                        r#"
                        mutation($itemId: ID!, $body: String!) {
                          create_update(item_id: $itemId, body: $body) {
                            id
                            body
                          }
                        }
                        "#,
                        serde_json::json!({
                            "itemId": item_id,
                            "body": body,
                        }),
                        idempotency_key,
                        "monday.com",
                        "add_update",
                        Some("2025-10"),
                    )
                    .await?;
                    Ok(serde_json::json!({
                        "added": true,
                        "update": data.get("create_update").cloned().unwrap_or_default(),
                        "result": data.get("create_update").cloned().unwrap_or_default(),
                    }))
                }
                _ => anyhow::bail!("monday: unknown operation '{}'", operation),
            }
        }

        "slack" => match operation {
            "send_message" => {
                let channel = params["channel"].as_str().unwrap_or("#general");
                let text = params["text"].as_str().unwrap_or("");
                let r = http
                    .post("https://slack.com/api/chat.postMessage")
                    .bearer_auth(token)
                    .json(&serde_json::json!({"channel": channel, "text": text}))
                    .send()
                    .await?;
                Ok(r.json().await?)
            }
            "list_messages" => {
                let channel = params["channel"].as_str().ok_or_else(|| anyhow::anyhow!("channel required"))?;
                let limit = params["limit"].as_u64().unwrap_or(20);
                let url = format!("https://slack.com/api/conversations.history?channel={}&limit={}", channel, limit);
                Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
            }
            "reply_thread" => {
                let channel = params["channel"].as_str().ok_or_else(|| anyhow::anyhow!("channel required"))?;
                let thread_ts = params["thread_ts"].as_str().ok_or_else(|| anyhow::anyhow!("thread_ts required"))?;
                let text = params["text"].as_str().unwrap_or("");
                Ok(http
                    .post("https://slack.com/api/chat.postMessage")
                    .bearer_auth(token)
                    .json(&serde_json::json!({"channel": channel, "thread_ts": thread_ts, "text": text}))
                    .send()
                    .await?
                    .json()
                    .await?)
            }
            "lookup_user" => {
                let email = params["email"].as_str();
                if let Some(e) = email {
                    let url = format!("https://slack.com/api/users.lookupByEmail?email={}", e);
                    Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
                } else {
                    let url = format!("https://slack.com/api/users.list?limit=100");
                    Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
                }
            }
            _ => anyhow::bail!("Slack: unknown operation '{}'", operation),
        },

        // ── Notion ─────────────────────────────────────────────────────────
        "notion" => {
            let base = "https://api.notion.com/v1";
            match operation {
                "search_pages" => {
                    let q = params["query"].as_str().unwrap_or("");
                    Ok(http
                        .post(&format!("{}/search", base))
                        .bearer_auth(token)
                        .header("Notion-Version", "2022-06-28")
                        .json(&serde_json::json!({"query": q}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_page" => {
                    let id = params["page_id"].as_str().ok_or_else(|| anyhow::anyhow!("page_id required"))?;
                    Ok(http
                        .get(&format!("{}/blocks/{}/children", base, id))
                        .bearer_auth(token)
                        .header("Notion-Version", "2022-06-28")
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "create_page" => {
                    let body = params.get("body").cloned().unwrap_or_else(|| serde_json::json!({"parent": {"page_id": params["parent_id"]}, "properties": {"title": [{"text": {"content": params["title"].as_str().unwrap_or("")}}]}}));
                    Ok(http
                        .post(&format!("{}/pages", base))
                        .bearer_auth(token)
                        .header("Notion-Version", "2022-06-28")
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "append_block" => {
                    let block_id = params["block_id"].as_str().ok_or_else(|| anyhow::anyhow!("block_id required"))?;
                    let children = params.get("children").cloned().unwrap_or(serde_json::json!([]));
                    Ok(http
                        .patch(&format!("{}/blocks/{}/children", base, block_id))
                        .bearer_auth(token)
                        .header("Notion-Version", "2022-06-28")
                        .json(&serde_json::json!({"children": children}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "update_props" => {
                    let id = params["page_id"].as_str().ok_or_else(|| anyhow::anyhow!("page_id required"))?;
                    let props = params.get("properties").cloned().unwrap_or_default();
                    Ok(http
                        .patch(&format!("{}/pages/{}", base, id))
                        .bearer_auth(token)
                        .header("Notion-Version", "2022-06-28")
                        .json(&serde_json::json!({"properties": props}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Notion: unknown operation '{}'", operation),
            }
        }

        // ── Zendesk ────────────────────────────────────────────────────────
        "zendesk" => {
            let subdomain = settings["subdomain"]
                .as_str()
                .or_else(|| params["subdomain"].as_str())
                .ok_or_else(|| anyhow::anyhow!("Zendesk: missing subdomain in settings"))?;
            let base = format!("https://{}.zendesk.com/api/v2", subdomain);

            match operation {
                "list_tickets" => {
                    let status = params["status"].as_str().unwrap_or("open");
                    Ok(http
                        .get(&format!("{}/tickets?status={}", base, status))
                        .bearer_auth(token)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_ticket" => {
                    let id = params["ticket_id"].as_str().ok_or_else(|| anyhow::anyhow!("ticket_id required"))?;
                    Ok(http.get(&format!("{}/tickets/{}", base, id)).bearer_auth(token).send().await?.json().await?)
                }
                "create_ticket" => {
                    let ticket = params.get("ticket").cloned().unwrap_or_default();
                    Ok(http
                        .post(&format!("{}/tickets", base))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"ticket": ticket}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "update_ticket" => {
                    let id = params["ticket_id"].as_str().ok_or_else(|| anyhow::anyhow!("ticket_id required"))?;
                    let ticket = params.get("ticket").cloned().unwrap_or_default();
                    Ok(http
                        .put(&format!("{}/tickets/{}", base, id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"ticket": ticket}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "add_comment" => {
                    let id = params["ticket_id"].as_str().ok_or_else(|| anyhow::anyhow!("ticket_id required"))?;
                    let body = params["body"].as_str().unwrap_or("");
                    let public = params["public"].as_bool().unwrap_or(false);
                    Ok(http
                        .put(&format!("{}/tickets/{}", base, id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"ticket": {"comment": {"body": body, "public": public}}}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Zendesk: unknown operation '{}'", operation),
            }
        }

        // ── Intercom ───────────────────────────────────────────────────────
        "intercom" => {
            let base = "https://api.intercom.io";
            match operation {
                "list_conversations" => Ok(http
                    .get(&format!("{}/conversations?display_as=plaintext", base))
                    .bearer_auth(token)
                    .header("Accept", "application/json")
                    .header("Intercom-Version", "2.10")
                    .send()
                    .await?
                    .json()
                    .await?),
                "get_conversation" => {
                    let id = params["conversation_id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("conversation_id required"))?;
                    Ok(http
                        .get(&format!("{}/conversations/{}", base, id))
                        .bearer_auth(token)
                        .header("Intercom-Version", "2.10")
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "reply" => {
                    let id = params["conversation_id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("conversation_id required"))?;
                    let body_txt = params["body"].as_str().unwrap_or("");
                    let admin_id = params
                        .get("admin_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| settings["admin_id"].as_str())
                        .unwrap_or("");
                    let body =
                        serde_json::json!({"message_type":"reply","type":"admin","admin_id":admin_id,"body":body_txt});
                    Ok(http
                        .post(&format!("{}/conversations/{}/reply", base, id))
                        .bearer_auth(token)
                        .header("Intercom-Version", "2.10")
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "create_note" => {
                    let id = params["conversation_id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("conversation_id required"))?;
                    let body_txt = params["body"].as_str().unwrap_or("");
                    let admin_id = params
                        .get("admin_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| settings["admin_id"].as_str())
                        .unwrap_or("");
                    let body =
                        serde_json::json!({"message_type":"note","type":"admin","admin_id":admin_id,"body":body_txt});
                    Ok(http
                        .post(&format!("{}/conversations/{}/reply", base, id))
                        .bearer_auth(token)
                        .header("Intercom-Version", "2.10")
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "search_contacts" => {
                    let email = params["email"].as_str().unwrap_or("");
                    let body = serde_json::json!({"query":{"field":"email","operator":"=","value":email}});
                    Ok(http
                        .post(&format!("{}/contacts/search", base))
                        .bearer_auth(token)
                        .header("Intercom-Version", "2.10")
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Intercom: unknown operation '{}'", operation),
            }
        }

        // ── Freshdesk ──────────────────────────────────────────────────────
        "freshdesk" => {
            let domain = settings["domain"]
                .as_str()
                .or_else(|| params["domain"].as_str())
                .ok_or_else(|| anyhow::anyhow!("Freshdesk: missing domain in settings"))?;
            let base = format!("https://{}.freshdesk.com/api/v2", domain);
            // Freshdesk uses basic auth: api_key:X
            let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{}:X", token));
            let auth_header = format!("Basic {}", encoded);

            match operation {
                "list_tickets" => Ok(http
                    .get(&format!("{}/tickets", base))
                    .header("Authorization", &auth_header)
                    .send()
                    .await?
                    .json()
                    .await?),
                "create_ticket" => {
                    let ticket = params.get("ticket").cloned().unwrap_or_default();
                    Ok(http
                        .post(&format!("{}/tickets", base))
                        .header("Authorization", &auth_header)
                        .json(&ticket)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "update_ticket" => {
                    let id = params["ticket_id"].as_str().ok_or_else(|| anyhow::anyhow!("ticket_id required"))?;
                    let ticket = params.get("ticket").cloned().unwrap_or_default();
                    Ok(http
                        .put(&format!("{}/tickets/{}", base, id))
                        .header("Authorization", &auth_header)
                        .json(&ticket)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "add_note" => {
                    let id = params["ticket_id"].as_str().ok_or_else(|| anyhow::anyhow!("ticket_id required"))?;
                    let body = params["body"].as_str().unwrap_or("");
                    let private = params["private"].as_bool().unwrap_or(true);
                    Ok(http
                        .post(&format!("{}/tickets/{}/notes", base, id))
                        .header("Authorization", &auth_header)
                        .json(&serde_json::json!({"body": body, "private": private}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_contact" => {
                    let email = params["email"].as_str().ok_or_else(|| anyhow::anyhow!("email required"))?;
                    Ok(http
                        .get(&format!("{}/contacts?email={}", base, urlencoding::encode(email)))
                        .header("Authorization", &auth_header)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Freshdesk: unknown operation '{}'", operation),
            }
        }

        // ── Stripe ─────────────────────────────────────────────────────────
        "stripe" => {
            let base = "https://api.stripe.com/v1";
            match operation {
                "list_charges" => {
                    let limit = params["limit"].as_u64().unwrap_or(10);
                    let status = params.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let url = if status.is_empty() {
                        format!("{}/charges?limit={}", base, limit)
                    } else {
                        // Stripe doesn't filter charges by status in list — filter by failure_code
                        format!("{}/charges?limit={}", base, limit)
                    };
                    Ok(http.get(&url).basic_auth(token, Option::<&str>::None).send().await?.json().await?)
                }
                "get_customer" => {
                    let id_or_email = params["id"]
                        .as_str()
                        .or_else(|| params["email"].as_str())
                        .ok_or_else(|| anyhow::anyhow!("id or email required"))?;
                    if id_or_email.starts_with("cus_") {
                        Ok(http
                            .get(&format!("{}/customers/{}", base, id_or_email))
                            .basic_auth(token, Option::<&str>::None)
                            .send()
                            .await?
                            .json()
                            .await?)
                    } else {
                        Ok(http
                            .get(&format!("{}/customers?email={}&limit=1", base, urlencoding::encode(id_or_email)))
                            .basic_auth(token, Option::<&str>::None)
                            .send()
                            .await?
                            .json()
                            .await?)
                    }
                }
                "list_invoices" => {
                    let customer = params["customer"].as_str().unwrap_or("");
                    Ok(http
                        .get(&format!("{}/invoices?customer={}&limit=10", base, customer))
                        .basic_auth(token, Option::<&str>::None)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "list_subscriptions" => {
                    let customer = params.get("customer").and_then(|v| v.as_str()).unwrap_or("");
                    let url = if customer.is_empty() {
                        format!("{}/subscriptions?status=active&limit=10", base)
                    } else {
                        format!("{}/subscriptions?customer={}&status=active", base, customer)
                    };
                    Ok(http.get(&url).basic_auth(token, Option::<&str>::None).send().await?.json().await?)
                }
                "create_payment_link" => {
                    let price_id = params["price_id"].as_str().ok_or_else(|| anyhow::anyhow!("price_id required"))?;
                    let quantity = params["quantity"].as_u64().unwrap_or(1);
                    Ok(http
                        .post(&format!("{}/payment_links", base))
                        .basic_auth(token, Option::<&str>::None)
                        .form(&[("line_items[0][price]", price_id), ("line_items[0][quantity]", &quantity.to_string())])
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Stripe: unknown operation '{}'", operation),
            }
        }

        // ── QuickBooks ─────────────────────────────────────────────────────
        "quickbooks" => {
            let realm_id = settings["realm_id"]
                .as_str()
                .or_else(|| params["realm_id"].as_str())
                .ok_or_else(|| anyhow::anyhow!("QuickBooks: missing realm_id in settings"))?;
            let base = format!("https://quickbooks.api.intuit.com/v3/company/{}", realm_id);

            match operation {
                "query" => {
                    let q = params["query"].as_str().ok_or_else(|| anyhow::anyhow!("query required"))?;
                    let url = format!("{}/query?query={}", base, urlencoding::encode(q));
                    Ok(http
                        .get(&url)
                        .bearer_auth(token)
                        .header("Accept", "application/json")
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "create_invoice" => {
                    let invoice = params.get("invoice").cloned().unwrap_or_default();
                    Ok(http
                        .post(&format!("{}/invoice", base))
                        .bearer_auth(token)
                        .header("Content-Type", "application/json")
                        .json(&invoice)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_report" => {
                    let report = params["report_type"].as_str().unwrap_or("ProfitAndLoss");
                    Ok(http
                        .get(&format!("{}/reports/{}", base, report))
                        .bearer_auth(token)
                        .header("Accept", "application/json")
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("QuickBooks: unknown operation '{}'", operation),
            }
        }

        // ── ServiceNow ─────────────────────────────────────────────────────
        "servicenow" => {
            let instance = settings["instance_url"]
                .as_str()
                .or_else(|| params["instance_url"].as_str())
                .ok_or_else(|| anyhow::anyhow!("ServiceNow: missing instance_url in settings"))?;
            let base = format!("{}/api/now/v2", instance.trim_end_matches('/'));

            match operation {
                "query_records" => {
                    let table = params["table"].as_str().unwrap_or("incident");
                    let filter = params.get("sysparm_query").and_then(|v| v.as_str()).unwrap_or("active=true");
                    let limit = params["sysparm_limit"].as_u64().unwrap_or(10);
                    Ok(http
                        .get(&format!(
                            "{}/table/{}?sysparm_query={}&sysparm_limit={}",
                            base,
                            table,
                            urlencoding::encode(filter),
                            limit
                        ))
                        .bearer_auth(token)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_record" => {
                    let table = params["table"].as_str().unwrap_or("incident");
                    let sys_id = params["sys_id"].as_str().ok_or_else(|| anyhow::anyhow!("sys_id required"))?;
                    Ok(http
                        .get(&format!("{}/table/{}/{}", base, table, sys_id))
                        .bearer_auth(token)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "create_incident" => {
                    let body = params.get("record").cloned().unwrap_or_else(
                        || serde_json::json!({"short_description": params["short_description"].as_str().unwrap_or("")}),
                    );
                    Ok(http
                        .post(&format!("{}/table/incident", base))
                        .bearer_auth(token)
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "update_record" => {
                    let table = params["table"].as_str().unwrap_or("incident");
                    let sys_id = params["sys_id"].as_str().ok_or_else(|| anyhow::anyhow!("sys_id required"))?;
                    let body = params.get("fields").cloned().unwrap_or_default();
                    Ok(http
                        .patch(&format!("{}/table/{}/{}", base, table, sys_id))
                        .bearer_auth(token)
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "add_work_note" => {
                    let sys_id = params["sys_id"].as_str().ok_or_else(|| anyhow::anyhow!("sys_id required"))?;
                    let note = params["work_notes"].as_str().unwrap_or("");
                    Ok(http
                        .patch(&format!("{}/table/incident/{}", base, sys_id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"work_notes": note}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("ServiceNow: unknown operation '{}'", operation),
            }
        }

        // ── PagerDuty ──────────────────────────────────────────────────────
        "pagerduty" => {
            let base = "https://api.pagerduty.com";
            match operation {
                "list_incidents" => {
                    let status = params.get("status").and_then(|v| v.as_str()).unwrap_or("triggered,acknowledged");
                    Ok(http
                        .get(&format!("{}/incidents?statuses[]={}", base, status))
                        .header("Authorization", format!("Token token={}", token))
                        .header("Accept", "application/vnd.pagerduty+json;version=2")
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "trigger_incident" => {
                    let routing_key = settings["routing_key"].as_str().unwrap_or("");
                    let body = serde_json::json!({"routing_key": routing_key, "event_action": "trigger",
                        "payload": {"summary": params["summary"].as_str().unwrap_or("Incident"), "severity": params.get("severity").and_then(|v| v.as_str()).unwrap_or("critical"), "source": "narayan"}});
                    Ok(http.post("https://events.pagerduty.com/v2/enqueue").json(&body).send().await?.json().await?)
                }
                "acknowledge" => {
                    let id = params["incident_id"].as_str().ok_or_else(|| anyhow::anyhow!("incident_id required"))?;
                    Ok(http
                        .put(&format!("{}/incidents/{}", base, id))
                        .header("Authorization", format!("Token token={}", token))
                        .header("Accept", "application/vnd.pagerduty+json;version=2")
                        .header(
                            "From",
                            params.get("from_email").and_then(|v| v.as_str()).unwrap_or("narayan@example.com"),
                        )
                        .json(&serde_json::json!({"incident":{"type":"incident_reference","status":"acknowledged"}}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "add_note" => {
                    let id = params["incident_id"].as_str().ok_or_else(|| anyhow::anyhow!("incident_id required"))?;
                    let note = params["content"].as_str().unwrap_or("");
                    Ok(http
                        .post(&format!("{}/incidents/{}/notes", base, id))
                        .header("Authorization", format!("Token token={}", token))
                        .header("Accept", "application/vnd.pagerduty+json;version=2")
                        .header(
                            "From",
                            params.get("from_email").and_then(|v| v.as_str()).unwrap_or("narayan@example.com"),
                        )
                        .json(&serde_json::json!({"note":{"content":note}}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_oncall" => {
                    let schedule_id =
                        params["schedule_id"].as_str().ok_or_else(|| anyhow::anyhow!("schedule_id required"))?;
                    Ok(http
                        .get(&format!("{}/schedules/{}/users", base, schedule_id))
                        .header("Authorization", format!("Token token={}", token))
                        .header("Accept", "application/vnd.pagerduty+json;version=2")
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("PagerDuty: unknown operation '{}'", operation),
            }
        }

        // ── Greenhouse ─────────────────────────────────────────────────────
        "greenhouse" => {
            let base = "https://harvest.greenhouse.io/v1";
            match operation {
                "list_jobs" => Ok(http
                    .get(&format!("{}/jobs?status=open", base))
                    .basic_auth(token, Option::<&str>::None)
                    .send()
                    .await?
                    .json()
                    .await?),
                "list_candidates" => {
                    let job_id = params.get("job_id").and_then(|v| v.as_str()).unwrap_or("");
                    let url = if job_id.is_empty() {
                        format!("{}/candidates", base)
                    } else {
                        format!("{}/candidates?job_id={}", base, job_id)
                    };
                    Ok(http.get(&url).basic_auth(token, Option::<&str>::None).send().await?.json().await?)
                }
                "get_application" => {
                    let id =
                        params["application_id"].as_str().ok_or_else(|| anyhow::anyhow!("application_id required"))?;
                    Ok(http
                        .get(&format!("{}/applications/{}", base, id))
                        .basic_auth(token, Option::<&str>::None)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "add_note" => {
                    let candidate_id =
                        params["candidate_id"].as_str().ok_or_else(|| anyhow::anyhow!("candidate_id required"))?;
                    let user_id = params["user_id"].as_str().unwrap_or("0");
                    let body = serde_json::json!({"user_id": user_id, "body": params["body"].as_str().unwrap_or("")});
                    Ok(http
                        .post(&format!("{}/candidates/{}/activity_feed/notes", base, candidate_id))
                        .basic_auth(token, Option::<&str>::None)
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "advance_stage" => {
                    let application_id =
                        params["application_id"].as_str().ok_or_else(|| anyhow::anyhow!("application_id required"))?;
                    let from_stage_id =
                        params["from_stage_id"].as_str().ok_or_else(|| anyhow::anyhow!("from_stage_id required"))?;
                    let body = serde_json::json!({"from_stage_id": from_stage_id});
                    Ok(http
                        .post(&format!("{}/applications/{}/advance", base, application_id))
                        .basic_auth(token, Option::<&str>::None)
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Greenhouse: unknown operation '{}'", operation),
            }
        }

        // ── DocuSign ───────────────────────────────────────────────────────
        "docusign" => {
            let account_id = settings["account_id"]
                .as_str()
                .or_else(|| params["account_id"].as_str())
                .ok_or_else(|| anyhow::anyhow!("DocuSign: missing account_id in settings"))?;
            let base_url = settings["base_url"].as_str().unwrap_or("https://demo.docusign.net");
            let base = format!("{}/restapi/v2.1/accounts/{}", base_url, account_id);

            match operation {
                "create_envelope" => {
                    let envelope = params.get("envelope").cloned().unwrap_or_default();
                    Ok(http
                        .post(&format!("{}/envelopes", base))
                        .bearer_auth(token)
                        .json(&envelope)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "send_envelope" => {
                    let id = params["envelope_id"].as_str().ok_or_else(|| anyhow::anyhow!("envelope_id required"))?;
                    Ok(http
                        .put(&format!("{}/envelopes/{}", base, id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"status":"sent"}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_status" => {
                    let id = params["envelope_id"].as_str().ok_or_else(|| anyhow::anyhow!("envelope_id required"))?;
                    Ok(http.get(&format!("{}/envelopes/{}", base, id)).bearer_auth(token).send().await?.json().await?)
                }
                "void_envelope" => {
                    let id = params["envelope_id"].as_str().ok_or_else(|| anyhow::anyhow!("envelope_id required"))?;
                    let reason = params.get("reason").and_then(|v| v.as_str()).unwrap_or("Voided by Narayan");
                    Ok(http
                        .put(&format!("{}/envelopes/{}", base, id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"status":"voided","voidedReason":reason}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("DocuSign: unknown operation '{}'", operation),
            }
        }

        // ── dbt Cloud ──────────────────────────────────────────────────────
        "dbt_cloud" => {
            let account_id = settings["account_id"]
                .as_str()
                .or_else(|| params["account_id"].as_str())
                .ok_or_else(|| anyhow::anyhow!("dbt Cloud: missing account_id in settings"))?;
            let base = format!("https://cloud.getdbt.com/api/v2/accounts/{}", account_id);

            match operation {
                "list_jobs" => Ok(http.get(&format!("{}/jobs/", base)).bearer_auth(token).send().await?.json().await?),
                "trigger_run" => {
                    let job_id = params["job_id"].as_str().ok_or_else(|| anyhow::anyhow!("job_id required"))?;
                    let cause = params.get("cause").and_then(|v| v.as_str()).unwrap_or("Triggered by Narayan");
                    Ok(http
                        .post(&format!("{}/jobs/{}/run/", base, job_id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"cause": cause}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_run" => {
                    let run_id = params["run_id"].as_str().ok_or_else(|| anyhow::anyhow!("run_id required"))?;
                    Ok(http.get(&format!("{}/runs/{}/", base, run_id)).bearer_auth(token).send().await?.json().await?)
                }
                "list_models" => {
                    let project_id = params.get("project_id").and_then(|v| v.as_str()).unwrap_or("");
                    let url = format!(
                        "https://cloud.getdbt.com/api/v3/accounts/{}/projects/{}/models/",
                        account_id, project_id
                    );
                    Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
                }
                _ => anyhow::bail!("dbt Cloud: unknown operation '{}'", operation),
            }
        }

        // ── Gmail ──────────────────────────────────────────────────────────
        "gmail" => {
            let base = "https://gmail.googleapis.com/gmail/v1/users/me";
            match operation {
                "list_messages" => {
                    let q = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let max = params["max_results"].as_u64().unwrap_or(10);
                    Ok(http
                        .get(&format!("{}/messages?q={}&maxResults={}", base, urlencoding::encode(q), max))
                        .bearer_auth(token)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "get_message" => {
                    let id = params["message_id"].as_str().ok_or_else(|| anyhow::anyhow!("message_id required"))?;
                    Ok(http.get(&format!("{}/messages/{}", base, id)).bearer_auth(token).send().await?.json().await?)
                }
                "send_email" | "create_draft" => {
                    let to = params["to"].as_str().unwrap_or("");
                    let subject = params["subject"].as_str().unwrap_or("");
                    let body = params["body"].as_str().unwrap_or("");
                    let raw = base64::Engine::encode(
                        &base64::engine::general_purpose::URL_SAFE,
                        format!("To: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\n{body}"),
                    );
                    if operation == "create_draft" {
                        Ok(http
                            .post(&format!("{}/drafts", base))
                            .bearer_auth(token)
                            .json(&serde_json::json!({"message":{"raw":raw}}))
                            .send()
                            .await?
                            .json()
                            .await?)
                    } else {
                        Ok(http
                            .post(&format!("{}/messages/send", base))
                            .bearer_auth(token)
                            .json(&serde_json::json!({"raw":raw}))
                            .send()
                            .await?
                            .json()
                            .await?)
                    }
                }
                "search" => {
                    let q = params["query"].as_str().ok_or_else(|| anyhow::anyhow!("query required"))?;
                    let max = params["max_results"].as_u64().unwrap_or(10);
                    Ok(http
                        .get(&format!("{}/messages?q={}&maxResults={}", base, urlencoding::encode(q), max))
                        .bearer_auth(token)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Gmail: unknown operation '{}'", operation),
            }
        }

        // ── Outlook ────────────────────────────────────────────────────────
        "outlook" => {
            let base = "https://graph.microsoft.com/v1.0/me";
            match operation {
                "list_messages" => {
                    let top = params["top"].as_u64().unwrap_or(10);
                    let filter = params.get("filter").and_then(|v| v.as_str()).unwrap_or("");
                    let url = if filter.is_empty() {
                        format!("{}/messages?$top={}", base, top)
                    } else {
                        format!("{}/messages?$top={}&$filter={}", base, top, urlencoding::encode(filter))
                    };
                    Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
                }
                "send_email" => {
                    let body = serde_json::json!({
                        "message": {
                            "subject": params["subject"].as_str().unwrap_or(""),
                            "body": {"contentType":"Text","content": params["body"].as_str().unwrap_or("")},
                            "toRecipients":[{"emailAddress":{"address":params["to"].as_str().unwrap_or("")}}],
                        }
                    });
                    http.post(&format!("{}/sendMail", base)).bearer_auth(token).json(&body).send().await?;
                    Ok(serde_json::json!({"sent": true}))
                }
                "create_draft" => {
                    let body = serde_json::json!({
                        "subject": params["subject"].as_str().unwrap_or(""),
                        "body": {"contentType":"Text","content": params["body"].as_str().unwrap_or("")},
                        "toRecipients":[{"emailAddress":{"address":params["to"].as_str().unwrap_or("")}}],
                    });
                    Ok(http
                        .post(&format!("{}/messages", base))
                        .bearer_auth(token)
                        .json(&body)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "list_events" => {
                    Ok(http.get(&format!("{}/events?$top=10", base)).bearer_auth(token).send().await?.json().await?)
                }
                "create_event" => {
                    let event = params.get("event").cloned().unwrap_or_default();
                    Ok(http
                        .post(&format!("{}/events", base))
                        .bearer_auth(token)
                        .json(&event)
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Outlook: unknown operation '{}'", operation),
            }
        }

        // ── Asana ──────────────────────────────────────────────────────────
        "asana" => {
            let base = "https://app.asana.com/api/1.0";
            match operation {
                "list_tasks" => {
                    let project = params.get("project").and_then(|v| v.as_str()).unwrap_or("");
                    let url = format!("{}/tasks?project={}&opt_fields=name,completed,due_on,assignee", base, project);
                    Ok(http.get(&url).bearer_auth(token).send().await?.json().await?)
                }
                "create_task" => {
                    let task = params.get("task").cloned().unwrap_or(serde_json::json!({
                        "name": params["name"].as_str().unwrap_or("New task"),
                        "projects": params.get("projects").cloned().unwrap_or_default(),
                    }));
                    Ok(http
                        .post(&format!("{}/tasks", base))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"data":task}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "update_task" => {
                    let id = params["task_id"].as_str().ok_or_else(|| anyhow::anyhow!("task_id required"))?;
                    let data = params.get("data").cloned().unwrap_or_default();
                    Ok(http
                        .put(&format!("{}/tasks/{}", base, id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"data":data}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                "add_comment" => {
                    let id = params["task_id"].as_str().ok_or_else(|| anyhow::anyhow!("task_id required"))?;
                    let text = params["text"].as_str().unwrap_or("");
                    Ok(http
                        .post(&format!("{}/tasks/{}/stories", base, id))
                        .bearer_auth(token)
                        .json(&serde_json::json!({"data":{"text":text}}))
                        .send()
                        .await?
                        .json()
                        .await?)
                }
                _ => anyhow::bail!("Asana: unknown operation '{}'", operation),
            }
        }

        _ => anyhow::bail!("No REST implementation for connector '{}'. Use auth_token for MCP fallback.", connector),
    }
}

// ── ConnectorTool ──────────────────────────────────────────────────────────
//
// Execution strategy:
//   1. If the LLM provides an auth_token override → use mcp_session (best effort MCP)
//   2. If install_store is set → load stored token → execute via real REST API
//   3. Fallback → try mcp_session with the def.mcp_url (may fail for fantasy URLs)
//
// This means real production connector calls go through the same authenticated
// REST paths that the Connector framework uses — no fantasy MCP servers needed.

pub struct ConnectorTool {
    def: &'static ConnectorDef,
    mcp: McpSessionTool,
    install_store: Option<Arc<crate::connectors::ConnectorInstallStore>>,
    http: reqwest::Client,
}

impl ConnectorTool {
    pub fn new(def: &'static ConnectorDef) -> Self {
        Self { def, mcp: McpSessionTool::new(), install_store: None, http: reqwest::Client::new() }
    }

    pub fn with_install_store(
        def: &'static ConnectorDef,
        store: Arc<crate::connectors::ConnectorInstallStore>,
    ) -> Self {
        Self {
            def,
            mcp: McpSessionTool::new().with_install_store(Arc::clone(&store)),
            install_store: Some(store),
            http: reqwest::Client::new(),
        }
    }

    /// Fetch the stored access token for this connector + tenant.
    async fn stored_token(&self, tenant_id: &str) -> Option<String> {
        let store = self.install_store.as_ref()?;
        let install = store.get(tenant_id, self.def.name).await.ok()??;
        store.decrypt_token(&install)
    }

    /// Execute a real REST API call for this connector.
    /// Routes the `operation` string to the correct endpoint for each connector.
    async fn execute_rest(
        &self,
        token: &str,
        operation: &str,
        params: &serde_json::Value,
        tenant_id: &str,
        idempotency_key: Option<&str>,
    ) -> anyhow::Result<ToolResult> {
        // Get any stored settings (e.g. Salesforce instance_url, Zendesk subdomain)
        let settings = if let Some(store) = &self.install_store {
            store.get(tenant_id, self.def.name).await.ok().flatten().map(|i| i.settings.clone()).unwrap_or_default()
        } else {
            serde_json::json!({})
        };

        let result =
            rest_execute(&self.http, self.def.name, token, operation, params, &settings, idempotency_key).await?;

        Ok(ToolResult::ok(result))
    }
}

#[async_trait]
impl Tool for ConnectorTool {
    fn name(&self) -> &str {
        self.def.name
    }
    fn description(&self) -> &str {
        self.def.description
    }
    fn category(&self) -> &'static str {
        self.def.category
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        let ops_hint = self.def.operations.join("; ");
        vec![
            ParameterSchema::required(
                "operation",
                "string",
                Box::leak(format!("Operation to perform. Available: {}", ops_hint).into_boxed_str()),
            ),
            ParameterSchema::optional("params", "object", "Operation-specific parameters as a JSON object."),
            ParameterSchema::optional("tenant_id", "string", "Tenant ID for credential lookup (injected by executor)."),
            ParameterSchema::optional(
                "goal_instance_id",
                "string",
                "Goal instance ID injected by the executor to keep retries idempotent.",
            ),
            ParameterSchema::optional(
                "step_index",
                "integer",
                "Current plan step index injected by the executor for stable retries.",
            ),
            ParameterSchema::optional(
                "idempotency_key",
                "string",
                "Stable idempotency key injected by the executor; derived automatically when omitted.",
            ),
            ParameterSchema::optional(
                "auth_token",
                "string",
                "Bearer token override. Omit to use the stored tenant credential.",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let operation = match args["operation"].as_str() {
            Some(op) => op.to_string(),
            None => return Ok(ToolResult::err("'operation' is required")),
        };
        let params = args.get("params").cloned().unwrap_or_default();
        let tenant_id = args["tenant_id"].as_str().unwrap_or("").to_string();
        let goal_instance_id = args["goal_instance_id"].as_str().filter(|value| !value.trim().is_empty());
        let step_index = args["step_index"].as_u64();
        let idempotency_key = args
            .get("idempotency_key")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(String::from)
            .unwrap_or_else(|| {
                derive_idempotency_key(self.def.name, &tenant_id, goal_instance_id, step_index, &operation, &params)
            });

        // 1. Explicit token override → try MCP session
        if let Some(token) = args["auth_token"].as_str() {
            let mcp_args = serde_json::json!({
                "server_url": self.def.mcp_url,
                "action":     "call_tool",
                "tool_name":  operation,
                "tool_args":  params,
                "auth_token": token,
            });
            return self.mcp.execute(mcp_args).await;
        }

        // 2. Load stored token → real REST API
        if !tenant_id.is_empty() {
            if let Some(token) = self.stored_token(&tenant_id).await {
                return self.execute_rest(&token, &operation, &params, &tenant_id, Some(&idempotency_key)).await;
            }
        }

        // 3. Fallback: mcp_session with def.mcp_url (works if a real MCP server exists)
        let mcp_args = serde_json::json!({
            "server_url": self.def.mcp_url,
            "action":     "call_tool",
            "tool_name":  operation,
            "tool_args":  params,
        });
        self.mcp.execute(mcp_args).await
    }
}

// ── Registration ───────────────────────────────────────────────────────────

pub fn register_all_connectors(
    registry: &mut crate::tools::ToolRegistry,
    install_store: Option<Arc<crate::connectors::ConnectorInstallStore>>,
) {
    for def in ALL_CONNECTORS {
        let tool: Arc<dyn Tool> = match &install_store {
            Some(store) => Arc::new(ConnectorTool::with_install_store(def, Arc::clone(store))),
            None => Arc::new(ConnectorTool::new(def)),
        };
        registry.register(tool);
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_connectors_have_unique_names() {
        let mut names = std::collections::HashSet::new();
        for def in ALL_CONNECTORS {
            assert!(names.insert(def.name), "duplicate connector name: {}", def.name);
        }
    }

    #[test]
    fn test_all_connectors_have_category_prefix() {
        for def in ALL_CONNECTORS {
            assert!(
                def.category.starts_with("connector/"),
                "connector '{}' category '{}' must start with 'connector/'",
                def.name,
                def.category
            );
        }
    }

    #[test]
    fn test_all_connectors_have_operations() {
        for def in ALL_CONNECTORS {
            assert!(!def.operations.is_empty(), "connector '{}' needs at least one operation", def.name);
        }
    }

    #[test]
    fn test_all_connectors_have_keywords() {
        for def in ALL_CONNECTORS {
            assert!(!def.keywords.is_empty(), "connector '{}' needs at least one keyword", def.name);
        }
    }

    #[test]
    fn test_linear_and_monday_are_registered() {
        assert!(find_by_name("linear").is_some(), "linear must be in the connector catalogue");
        assert!(find_by_name("monday").is_some(), "monday must be in the connector catalogue");
    }

    #[test]
    fn test_idempotency_key_is_stable_for_same_payload() {
        let params = serde_json::json!({ "title": "hello" });
        let key_a = derive_idempotency_key("linear", "tenant-1", Some("gi-1"), Some(4), "create_issue", &params);
        let key_b = derive_idempotency_key("linear", "tenant-1", Some("gi-1"), Some(4), "create_issue", &params);
        let key_c = derive_idempotency_key("linear", "tenant-1", Some("gi-1"), Some(5), "create_issue", &params);

        assert_eq!(key_a, key_b, "same payload must yield the same key");
        assert_ne!(key_a, key_c, "different step should produce a different key");
    }

    #[test]
    fn test_find_by_name_salesforce() {
        let def = find_by_name("salesforce").expect("salesforce should exist");
        assert_eq!(def.category, "connector/crm");
    }

    #[test]
    fn test_find_by_name_intercom() {
        let def = find_by_name("intercom").expect("intercom should exist");
        assert_eq!(def.category, "connector/support");
        assert!(def.keywords.contains(&"intercom"));
    }

    #[test]
    fn test_find_by_name_zendesk() {
        let def = find_by_name("zendesk").expect("zendesk should exist");
        assert_eq!(def.category, "connector/support");
    }

    #[test]
    fn test_find_by_name_gmail() {
        let def = find_by_name("gmail").expect("gmail should exist");
        assert_eq!(def.category, "connector/communication");
    }

    #[test]
    fn test_find_by_name_stripe() {
        let def = find_by_name("stripe").expect("stripe should exist");
        assert_eq!(def.category, "connector/finance");
    }

    #[test]
    fn test_find_by_name_missing() {
        assert!(find_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_catalogue_entries_count_matches() {
        assert_eq!(catalogue_entries().count(), ALL_CONNECTORS.len());
    }

    #[test]
    fn test_catalogue_entries_strip_prefix() {
        for (cat, name, _) in catalogue_entries() {
            assert!(
                !cat.starts_with("connector/"),
                "catalogue_entries should strip prefix, got '{}' for '{}'",
                cat,
                name
            );
        }
    }

    #[tokio::test]
    async fn test_connector_tool_requires_operation() {
        let def = find_by_name("salesforce").unwrap();
        let tool = ConnectorTool::new(def);
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("'operation'"));
    }
}
