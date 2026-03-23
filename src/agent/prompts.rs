//! Prompt construction for all agent runtime components.
//!
//! Centralised here — tuning one place affects the entire runtime.
//! Every function returns a (system, user) pair.
//!
//! DESIGN PRINCIPLES:
//!   - Prompts are specific, not generic — job-type specialisation throughout
//!   - StepHistory uses tiered compression (recent = full, old = summary)
//!   - Planner receives grouped tool manifest (not a flat list of 65 names)
//!   - Evaluator sees actual tool names called (not just ok/fail counts)
//!   - All JSON output formats specify exact field names and types

use crate::{
    agent::{
        executor::StepResult,
        planner::{Plan, PlannedStep},
    },
    state::AgentState,
    tools::ToolResult,
};

// ── Job type detection ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobType {
    SoftwareEngineer,
    ResearchAnalyst,
    CustomerSupport,
    DevOps,
    Marketing,
    DataExtraction,
    // ── New segments ──────────────────────────────────────────────────────
    /// Prospect research, CRM enrichment, pipeline reporting, outreach
    SalesRevOps,
    /// Invoice processing, reconciliation, expense categorisation, close
    FinanceAccounting,
    /// Candidate screening, onboarding, policy Q&A, performance data
    HRPeopleOps,
    /// Contract review, clause extraction, redlining, due diligence
    LegalContract,
    /// Incident runbooks, change advisory, ITSM workflows, health checks
    ITOpsITSM,
    General,
}

pub fn is_direct_response_goal(goal: &str) -> bool {
    let trimmed = goal.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_lowercase();

    // Only bypass the planner for trivial greetings and pure arithmetic.
    // Everything else goes through normal planning so the LLM can decide
    // whether tools/connectors are needed.

    let greetings = [
        "hi",
        "hello",
        "hey",
        "yo",
        "good morning",
        "good afternoon",
        "good evening",
        "thanks",
        "thank you",
        "bye",
        "goodbye",
    ];
    if greetings.iter().any(|g| lower == *g) {
        return true;
    }

    // Pure arithmetic expressions like "2+2" or "15 * 3"
    if lower.chars().all(|ch| ch.is_ascii_digit() || " +-*/().=".contains(ch)) {
        return true;
    }

    false
}

impl JobType {
    pub fn detect(goal: &str) -> Self {
        let g = goal.to_lowercase();
        let is = |kw: &[&str]| kw.iter().any(|k| g.contains(k));

        if is(&[
            "fix",
            "bug",
            "code",
            "commit",
            "pull request",
            "repo",
            "compile",
            "lint",
            "refactor",
            "deploy",
            "ci",
            "pipeline",
            "test",
            "function",
            "class",
            "implement",
            "write code",
            "typescript",
            "python",
        ]) {
            Self::SoftwareEngineer
        } else if is(&[
            "research",
            "analyze",
            "analyse",
            "competitor",
            "pricing",
            "market",
            "report",
            "survey",
            "comparison",
            "benchmark",
            "investigate",
            "study",
            "findings",
        ]) {
            Self::ResearchAnalyst
        } else if is(&[
            "ticket",
            "support",
            "customer",
            "email",
            "respond",
            "resolve",
            "helpdesk",
            "complaint",
            "inquiry",
            "reply",
        ]) {
            Self::CustomerSupport
        } else if is(&[
            "monitor",
            "infrastructure",
            "alert",
            "server",
            "cpu",
            "memory",
            "disk",
            "kubernetes",
            "docker",
            "logs",
            "incident",
            "uptime",
            "scale",
            "deploy",
            "container",
            "pod",
        ]) {
            Self::DevOps
        } else if is(&[
            "campaign",
            "marketing",
            "seo",
            "content",
            "social",
            "advertisement",
            "copy",
            "brand",
            "audience",
            "post",
        ]) {
            Self::Marketing
        } else if is(&[
            "scrape",
            "extract",
            "csv",
            "spreadsheet",
            "collect",
            "crawl",
            "parse",
            "structured",
            "harvest",
            "dataset",
        ]) {
            Self::DataExtraction
        } else if is(&[
            "prospect",
            "outreach",
            "crm",
            "lead",
            "pipeline",
            "deal",
            "quota",
            "sales",
            "account",
            "salesforce",
            "hubspot",
            "opportunity",
            "enrichment",
        ]) {
            Self::SalesRevOps
        } else if is(&[
            "invoice",
            "reconcile",
            "reconciliation",
            "expense",
            "accounting",
            "payable",
            "receivable",
            "balance sheet",
            "ledger",
            "journal entry",
            "month-end",
            "close",
            "quickbooks",
            "xero",
        ]) {
            Self::FinanceAccounting
        } else if is(&[
            "candidate",
            "hiring",
            "onboard",
            "onboarding",
            "recruit",
            "recruiting",
            "employee",
            "interview",
            "payroll",
            "performance review",
            "hr",
            "people ops",
            "job description",
            "offer letter",
        ]) {
            Self::HRPeopleOps
        } else if is(&[
            "contract",
            "clause",
            "agreement",
            "legal",
            "redline",
            "nda",
            "liability",
            "due diligence",
            "review contract",
            "terms of service",
            "statement of work",
            "sow",
            "obligation",
        ]) {
            Self::LegalContract
        } else if is(&[
            "runbook",
            "itsm",
            "change request",
            "change advisory",
            "servicenow",
            "pagerduty",
            "on-call",
            "postmortem",
            "root cause",
            "maintenance window",
            "cmdb",
            "asset management",
        ]) {
            Self::ITOpsITSM
        } else {
            Self::General
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::SoftwareEngineer => "software_engineer",
            Self::ResearchAnalyst => "research_analyst",
            Self::CustomerSupport => "customer_support",
            Self::DevOps => "devops",
            Self::Marketing => "marketing",
            Self::DataExtraction => "data_extraction",
            Self::SalesRevOps => "sales_revops",
            Self::FinanceAccounting => "finance_accounting",
            Self::HRPeopleOps => "hr_people_ops",
            Self::LegalContract => "legal_contract",
            Self::ITOpsITSM => "it_ops_itsm",
            Self::General => "general",
        }
    }

    /// Primary tools for this job type — given selection priority in executor.
    pub fn preferred_tools(&self) -> &'static [&'static str] {
        match self {
            Self::SoftwareEngineer => &[
                "shell",
                "file_read",
                "file_write",
                "file_edit",
                "content_search",
                "glob_search",
                "git_operations",
                "diff",
                "patch",
                "code_run",
                "wasm_exec",
                "sql_query",
            ],
            Self::ResearchAnalyst => &[
                "web_search_tool",
                "web_fetch",
                "browser",
                "browser_interact",
                "data_extractor",
                "vector_store",
                "vector_search",
                "file_write",
                "memory_store",
                "pdf_read",
                "spreadsheet_write",
            ],
            Self::CustomerSupport => &[
                "memory_recall",
                "vector_search",
                "email",
                "notification",
                "api_call",
                "request_credential",
                "file_read",
                "ask_user",
            ],
            Self::DevOps => &[
                "shell",
                "docker",
                "kubernetes",
                "ssh_exec",
                "process_monitor",
                "http_request",
                "api_call",
                "notification",
                "memory_store",
                "file_read",
                "cron_add",
            ],
            Self::Marketing => &[
                "web_search_tool",
                "web_fetch",
                "browser",
                "screenshot",
                "file_write",
                "image_process",
                "pdf_create",
                "notification",
            ],
            Self::DataExtraction => &[
                "web_fetch",
                "browser",
                "browser_interact",
                "data_extractor",
                "content_search",
                "spreadsheet_write",
                "file_write",
                "sql_query",
                "compress",
            ],
            Self::SalesRevOps => &[
                "web_search_tool",
                "web_fetch",
                "browser",
                "data_extractor",
                "email",
                "spreadsheet_write",
                "vector_search",
                "api_call",
                "memory_store",
                "memory_recall",
                "http_request",
            ],
            Self::FinanceAccounting => &[
                "pdf_read",
                "spreadsheet_write",
                "sql_query",
                "data_extractor",
                "file_read",
                "file_write",
                "api_call",
                "content_search",
                "memory_store",
                "crypto_tool",
            ],
            Self::HRPeopleOps => &[
                "pdf_read",
                "email",
                "vector_search",
                "schedule",
                "ask_user",
                "file_read",
                "file_write",
                "memory_store",
                "memory_recall",
                "spreadsheet_write",
            ],
            Self::LegalContract => &[
                "pdf_read",
                "diff_patch",
                "content_search",
                "file_read",
                "file_write",
                "vector_search",
                "memory_store",
                "data_extractor",
                "pdf_create",
            ],
            Self::ITOpsITSM => &[
                "shell",
                "ssh_exec",
                "docker",
                "kubernetes",
                "process_monitor",
                "http_request",
                "api_call",
                "notification",
                "file_read",
                "memory_store",
                "schedule",
            ],
            Self::General => &[
                "shell",
                "file_read",
                "file_write",
                "web_search_tool",
                "memory_store",
                "memory_recall",
                "http_request",
                "code_run",
            ],
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CONVERSATION HISTORY
// ═══════════════════════════════════════════════════════════════════════════

/// Build a conversation history string from prior agents in the same conversation.
/// Takes the list of agents ordered by created_at ASC.
/// Only includes agents that came *before* the current agent (by created_at).
/// Limits to the most recent 10 prior messages to avoid context overflow.
pub fn build_conversation_history(prior_agents: &[AgentState], current_agent_id: &str) -> String {
    let priors: Vec<&AgentState> = prior_agents.iter().filter(|a| a.id != current_agent_id).collect();

    if priors.is_empty() {
        return String::new();
    }

    // Take last 10
    let start = priors.len().saturating_sub(10);
    let recent = &priors[start..];

    let mut history = String::from("CONVERSATION HISTORY (prior messages in this thread):\n");
    for (i, agent) in recent.iter().enumerate() {
        let answer = agent.final_answer().unwrap_or("(still in progress)").chars().take(500).collect::<String>();
        history.push_str(&format!("[Message {}] User: {}\nNarayan: {}\n\n", i + 1, agent.goal, answer,));
    }
    history
}

fn clarification_context(state: &AgentState) -> Option<String> {
    let last_input = state
        .metadata
        .get("last_user_input_context")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    last_input.or_else(|| {
        state
            .metadata
            .get("clarification_answers")
            .and_then(|value| serde_json::from_value::<crate::agent::clarifier::ClarificationAnswers>(value.clone()).ok())
            .and_then(|answers| {
                if let Some(freeform) = answers.freeform.filter(|value| !value.trim().is_empty()) {
                    Some(freeform)
                } else {
                    let joined = answers
                        .answers
                        .into_iter()
                        .filter(|answer| !answer.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if joined.is_empty() { None } else { Some(joined) }
                }
            })
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// PLANNER PROMPTS
// ═══════════════════════════════════════════════════════════════════════════

pub struct PlannerPrompt;

impl PlannerPrompt {
    pub fn system(job_type: &JobType) -> String {
        let job_guidance = match job_type {
            JobType::SoftwareEngineer => {
                "\
You are planning work for a software engineer agent.
PLANNING RULES:
- Explore and read relevant files BEFORE making any changes
- Run existing tests before editing; re-run after editing to verify
- Each step is one atomic change — never bundle unrelated edits
- Commit and open a PR as the final steps
- Use diff tool to preview changes before applying
STEP SEQUENCE: explore → read_tests → read_source → edit → run_tests → fix_if_failing → commit → open_pr"
            }

            JobType::ResearchAnalyst => {
                "\
You are planning work for a research analyst agent.
PLANNING RULES:
- Define scope and sources before fetching anything
- Use parallel sub-agents (delegate) for independent research tracks
- Store intermediate findings to vector_store after each batch
- Synthesise only after all sources are gathered — never from memory alone
- Final step must write a structured markdown report to a file
STEP SEQUENCE: define_scope → map_sources → fetch_batch → extract → vector_store → synthesise → write_report"
            }

            JobType::CustomerSupport => {
                "\
You are planning work for a customer support agent.
PLANNING RULES:
- Read the full ticket context before searching anything
- Use vector_search to find similar resolved cases first
- Always draft a response before sending — never send on first pass
- Log the resolution outcome after sending
STEP SEQUENCE: read_ticket → vector_search_history → draft → validate → send → log_resolution"
            }

            JobType::DevOps => {
                "\
You are planning work for a DevOps agent.
PLANNING RULES:
- Always observe current state before taking any action
- Never modify infrastructure without reading its current config first
- Alert humans (notification tool) before any destructive action
- Verify health after every change — if health check fails, stop and report
STEP SEQUENCE: observe → diagnose → plan_change → notify_if_destructive → execute → verify → report"
            }

            JobType::Marketing => {
                "\
You are planning work for a marketing agent.
PLANNING RULES:
- Research audience and competitors before drafting anything
- Produce multiple content variants — never stop at first draft
- Save all drafts as workspace artifacts for human review
- Never publish directly — final step is always save_for_review
STEP SEQUENCE: research_audience → research_competitors → draft_variants → refine → save_artifacts"
            }

            JobType::DataExtraction => {
                "\
You are planning work for a data extraction agent.
PLANNING RULES:
- Map all target sources before fetching any
- Process in batches and store partial results after each batch
- Validate extracted schema against expected structure before writing output
- Always write both raw JSON and cleaned CSV output
STEP SEQUENCE: map_sources → batch_fetch → extract → validate_schema → clean → write_csv → write_summary"
            }

            JobType::General => {
                "\
You are planning work for a general-purpose agent.
PLANNING RULES:
- Break the goal into the smallest independently verifiable steps
- Prefer reversible actions over irreversible ones
- Store meaningful progress to memory after each significant step
- Final step must produce a tangible output: file, report, or confirmation"
            }

            JobType::SalesRevOps => {
                "\
You are planning work for a sales and revenue operations agent.
PLANNING RULES:
- Research the target (company, person, market) before writing any outreach
- Use vector_search to pull past interactions and context before contacting
- Enrich data from multiple public sources — never rely on a single signal
- Save all structured outputs to spreadsheet AND vector_store for recall
- Never send outreach without first saving a draft for review
STEP SEQUENCE: research_target → enrich_data → vector_search_history → draft_outreach → save_draft → review → send → log_to_crm"
            }

            JobType::FinanceAccounting => {
                "\
You are planning work for a finance and accounting agent.
PLANNING RULES:
- Read source documents in full before extracting any figures
- Cross-reference extracted values against at least one secondary source
- Every monetary calculation must show its inputs and formula explicitly
- Write outputs to both structured spreadsheet AND a human-readable summary
- Flag discrepancies for human review — never silently resolve them
STEP SEQUENCE: read_source → extract_values → cross_reference → calculate → validate → write_structured → write_summary → flag_discrepancies"
            }

            JobType::HRPeopleOps => {
                "\
You are planning work for an HR and people operations agent.
PLANNING RULES:
- Read the job description or policy document before taking any action
- Mask all PII in outputs unless explicitly authorised to include it
- Any decision affecting a person (screening, offer, termination) must route to human review
- Store resolutions in memory for consistent handling of similar cases
STEP SEQUENCE: read_context → vector_search_policy → draft_action → pii_check → route_to_review → execute → log"
            }

            JobType::LegalContract => {
                "\
You are planning work for a legal and contract operations agent.
PLANNING RULES:
- Read the full document before identifying any specific clause or issue
- Every finding must cite the exact section, page, and original language
- Produce a structured issues register — not a prose summary
- Redlines must show original language AND proposed alternative
- Never draw legal conclusions — surface findings for attorney review
STEP SEQUENCE: read_full_document → index_sections → identify_issues → cite_sections → draft_redlines → write_issues_register → route_to_review"
            }

            JobType::ITOpsITSM => {
                "\
You are planning work for an IT operations and ITSM agent.
PLANNING RULES:
- Read the runbook or change record before any execution
- Observe current system state FIRST — never act without a baseline
- Log every command executed with its timestamp and output
- Any change with risk > low must notify on-call before proceeding
- Verify system health after every change — stop immediately if health fails
STEP SEQUENCE: read_runbook → observe_state → notify_oncall_if_needed → execute_steps → verify_health → update_ticket → write_postmortem"
            }
        };

        format!(
            r#"{job_guidance}

OUTPUT FORMAT — return ONLY valid JSON, no markdown fences, no other text:
{{
  "goal": "restate the goal in one sentence",
  "job_type": "{label}",
  "steps": [
    {{
      "index": 0,
      "description": "specific, concrete description of what this step does",
      "tool": "exact_tool_name_or_null",
      "tool_args": {{}},
      "condition": {{
        "reference": "result_of_step_0.output.count",
        "operator": "gt",
        "value": 0
      }}
    }}
  ],
  "rationale": "one sentence: why this sequence achieves the goal"
}}

CONSTRAINTS:
- Maximum 12 steps. If more are needed, the goal is too broad — narrow it.
- Each step must be independently executable and its completion verifiable
- tool must be null only if the step is pure LLM reasoning with no external calls
- tool_args must include every required parameter for the named tool
- condition is optional; when present it must use one of: exists, not_exists, truthy, falsy, equals, not_equals, contains, nonempty, empty, gt, gte, lt, lte
- condition.reference must point to a concrete prior-step field such as result_of_step_0.output.count or result_of_step_0.tool_results[0].output.files[0].path
- Use condition for branching and skipping steps; do not describe IF/ELSE behavior only in prose
- If a later step depends on a prior step's structured output, reference it with an explicit template like {{result_of_step_0.tool_results[0].output.files[0].path}} or {{result_of_step_0.output}}
- Never use vague placeholders like {{result_of_step_0[0]}} — reference the real field path you need
- tool names must be exact — use the tool manifest provided
- If any step uses tools to gather, create, or transform information for the user, add a final step with tool=null that answers the user directly from the verified results
- The final step should be a user-facing answer step unless the goal is explicitly only background automation with no human reply needed
- Use pdf_create for PDF generation; never use file_write to create .pdf files
- Use compress/decompress for archives; never use file_write to create .zip, .tar.gz, or other archive files
- Prefer code_run for calculations or short executable snippets; only create a script file first when the user explicitly asks for a saved script
- Never plan a step you cannot verify as complete or failed"#,
            job_guidance = job_guidance,
            label = job_type.label(),
        )
    }

    /// User message for initial plan creation.
    /// Uses tool_manifest (grouped categories) instead of a flat list.
    pub fn user_create(
        state: &AgentState,
        context: &str,
        tool_manifest: &str,
        conversation_history: &str,
        role_context: Option<&str>,
    ) -> String {
        let conv_ctx =
            if conversation_history.is_empty() { String::new() } else { format!("\n{conversation_history}\n") };
        let clarification_ctx = clarification_context(state)
            .map(|value| format!("\nLATEST USER INPUT:\n{value}\n"))
            .unwrap_or_default();
        let role_ctx = role_context
            .filter(|s| !s.is_empty())
            .map(|s| format!("\nROLE CONTEXT (follow these guidelines):\n{s}\n"))
            .unwrap_or_default();
        format!(
            "{conv_ctx}GOAL: {goal}\n\nWORKSPACE: {ws}{clarification_ctx}{role_ctx}\nADDITIONAL CONTEXT:\n{ctx}\n\n{manifest}\n\nCreate the plan now.",
            conv_ctx          = conv_ctx,
            goal              = state.goal,
            ws                = state.workspace_path,
            clarification_ctx = clarification_ctx,
            role_ctx          = role_ctx,
            ctx               = if context.is_empty() { "none" } else { context },
            manifest          = tool_manifest,
        )
    }

    pub fn user_revise(plan: &Plan, feedback: &str, state: &AgentState) -> String {
        format!(
            "ORIGINAL PLAN:\n{plan}\n\nREVISION FEEDBACK:\n{feedback}\n\nGOAL: {goal}\nCOMPLETED UP TO STEP: {done}\n\nRevise ONLY the remaining steps (index >= {done}). Do not alter completed steps.",
            plan     = serde_json::to_string_pretty(plan).unwrap_or_default(),
            feedback = feedback,
            goal     = state.goal,
            done     = state.current_step,
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// EXECUTOR PROMPTS
// ═══════════════════════════════════════════════════════════════════════════

pub struct ExecutorPrompt;

impl ExecutorPrompt {
    pub fn direct_response_system() -> &'static str {
        r#"You are Narayan, a helpful chat assistant.

Reply directly to the user's message.
- Give the final answer in natural language.
- Do not describe internal planning, tools, or policies.
- Do not invent tool usage or say that a tool is required.
- If the user greeting is simple, answer simply and warmly."#
    }

    pub fn direct_response_user(state: &AgentState, history_summary: &str, conversation_history: &str) -> String {
        let mut parts = Vec::new();
        if !conversation_history.is_empty() {
            parts.push(conversation_history.to_string());
        }
        if !history_summary.trim().is_empty() {
            parts.push(format!("Step context:\n{history_summary}"));
        }
        if parts.is_empty() {
            state.goal.clone()
        } else {
            parts.push(format!("Latest user message:\n{}", state.goal));
            parts.join("\n\n")
        }
    }

    pub fn system(state: &AgentState, plan: &Plan) -> String {
        let job_type = JobType::detect(&state.goal);

        let execution_style = match job_type {
            JobType::SoftwareEngineer => {
                "\
EXECUTION STYLE:
- Read a file fully before editing any part of it
- Shell commands: always check exit code and stderr
- If a test fails, read the full error before attempting a fix — do not guess
- Never edit a file you have not read in this session"
            }

            JobType::ResearchAnalyst => {
                "\
EXECUTION STYLE:
- Fetch full page content — do not rely on snippets or metadata alone
- Store every significant finding immediately with vector_store
- When drafting reports: structured markdown, clear H2 sections, data-backed claims only
- Cite sources in the report — include URLs"
            }

            JobType::CustomerSupport => {
                "\
EXECUTION STYLE:
- Verify every factual claim against the knowledge base before including it in a response
- Empathetic but precise language — no promises about timelines you cannot guarantee
- Always log the resolution: what was done, when, by which agent"
            }

            JobType::DevOps => {
                "\
EXECUTION STYLE:
- Log every state-modifying shell command before executing it
- After any infrastructure change: run a health check immediately
- If health check fails after a change: stop, do not proceed, report the failure
- Use notification tool to alert humans before destructive operations"
            }

            JobType::Marketing => {
                "\
EXECUTION STYLE:
- Produce complete, polished content — not outlines or placeholders
- Save every draft as a named workspace file so humans can choose between them
- Include specific details, numbers, and examples — not generic copy"
            }

            JobType::DataExtraction => {
                "\
EXECUTION STYLE:
- Process every source in the list — do not skip similar-looking sources
- Validate field presence after each extraction batch before continuing
- If a source fails to load: log it and continue with the rest, report at the end"
            }

            JobType::General => {
                "\
EXECUTION STYLE:
- Complete each step fully before starting the next
- When a tool returns an error: read it carefully, understand the cause, fix it specifically
- Do not retry the same action with the same parameters — change something first"
            }

            JobType::SalesRevOps => {
                "\
EXECUTION STYLE:
- Verify company/contact existence before building any enrichment profile
- When using web_search: collect at least 3 distinct sources before synthesising
- Email drafts: personalise with specific details from research — no generic templates
- Log every CRM update with the data source and confidence level"
            }

            JobType::FinanceAccounting => {
                "\
EXECUTION STYLE:
- Every figure extracted from a document must include its source location (page, section)
- Calculations: show the full formula and all input values in the step output
- If extracted values conflict: surface both, do not choose — flag for human resolution
- Spreadsheet outputs: include a data dictionary tab explaining every column"
            }

            JobType::HRPeopleOps => {
                "\
EXECUTION STYLE:
- Redact PII from all intermediate outputs before storing to memory or files
- Screening decisions must cite specific criteria from the job description
- Use consistent scoring rubrics — store the rubric in memory at the start of each batch
- Never directly communicate a hiring decision to a candidate — prepare for human delivery"
            }

            JobType::LegalContract => {
                "\
EXECUTION STYLE:
- Quote the exact contract language before any analysis of it
- Issues register format: clause_number | original_text | issue_type | recommended_action
- Redlines: always show the original text struck-through and the proposed replacement
- Never paraphrase legal language in findings — use exact quotes with section references"
            }

            JobType::ITOpsITSM => {
                "\
EXECUTION STYLE:
- Every shell command executed must be logged with: timestamp, command, stdout, exit_code
- Health checks: define pass/fail criteria BEFORE running them, not after seeing results
- If a step fails: do not proceed — stop and report current state immediately
- Change advisory: always compare current config against known-good baseline before change"
            }
        };

        format!(
            r#"You are an autonomous AI agent executing a real-world task.

GOAL: {goal}
JOB TYPE: {jt}
WORKSPACE: {ws}
PLAN LENGTH: {n} steps
You are executing one step from the plan. The current step, retry context, and any user clarifications are provided in the user message.

{style}

EXECUTION RULES:
- Execute ONLY the current step shown in the user message — do not skip ahead
- Call the tool specified in the plan; only deviate if you have a concrete reason
- file_read is for files, not directories; if a path is a directory, inspect the listing and then switch to a concrete child file or use glob_search/content_search
- After every tool call, state what you observed and whether it achieved the step's intent
- If the step is complete, end your response with exactly: STEP COMPLETE
- If the step failed and cannot be recovered without a plan change, end with: STEP FAILED: <concise reason>
- Never fabricate tool results — if you cannot call a tool, say so"#,
            goal = state.goal,
            jt = job_type.label(),
            ws = state.workspace_path,
            n = plan.steps.len(),
            style = execution_style,
        )
    }

    pub fn user_step(
        state: &AgentState,
        step: &PlannedStep,
        history_summary: &str,
        previous_tool_results: &[&str],
        conversation_history: &str,
    ) -> String {
        let conv_ctx =
            if conversation_history.is_empty() { String::new() } else { format!("{conversation_history}\n") };

        let history_ctx = if history_summary.is_empty() {
            String::new()
        } else {
            format!("\nCOMPLETED STEPS SUMMARY:\n{}\n", history_summary)
        };

        let tool_ctx = if previous_tool_results.is_empty() {
            String::new()
        } else {
            format!(
                "\nTOOL RESULTS THIS STEP:\n{}\n",
                previous_tool_results
                    .iter()
                    .enumerate()
                    .map(|(i, r)| format!("[{}] {}", i + 1, truncate(r, 1200)))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let planned_tool = step
            .tool
            .as_ref()
            .map(|t| format!("\nPLANNED TOOL: {} — use this unless you have a concrete reason not to", t))
            .unwrap_or_default();
        let planned_tool_args = step
            .tool_args
            .as_ref()
            .map(|args| {
                format!(
                    "\nPLANNED TOOL ARGS:\n{}",
                    truncate(&serde_json::to_string_pretty(args).unwrap_or_default(), 1600)
                )
            })
            .unwrap_or_default();

        // Include previous attempt error so the LLM can fix its approach
        let retry_ctx = match (
            state.metadata.get("retry_count").and_then(|v| v.as_u64()).unwrap_or(0),
            state.metadata.get("last_step_error").and_then(|v| v.as_str()),
        ) {
            (count, Some(error)) if count > 0 => format!(
                "\n\nPREVIOUS ATTEMPT FAILED (retry {count}/{max}):\n{error}\nYou MUST use a different approach or fix the error. Do NOT repeat the same call with the same arguments.",
                count = count,
                max = 3,
                error = truncate(error, 500),
            ),
            _ => String::new(),
        };
        let clarification_ctx = clarification_context(state)
            .map(|value| format!("\nLATEST USER INPUT:\n{}\n", truncate(&value, 1200)))
            .unwrap_or_default();

        format!(
            "{conv_ctx}USER GOAL:\n{goal}{clarification_ctx}\nCURRENT STEP [{idx}]: {desc}{planned_tool}{planned_tool_args}{retry_ctx}{history}{tools}\n\nExecute this step now.",
            conv_ctx = conv_ctx,
            goal = state.goal,
            clarification_ctx = clarification_ctx,
            idx = step.index,
            desc = step.description,
            planned_tool = planned_tool,
            planned_tool_args = planned_tool_args,
            retry_ctx = retry_ctx,
            history = history_ctx,
            tools = tool_ctx,
        )
    }

    pub fn synthesis_system() -> &'static str {
        r#"You are Narayan, producing the final user-visible answer after internal tools have already run.

Your job is to answer the user using the verified execution results.
- Give only the user-facing answer
- Do not mention internal planning, tools, steps, policies, or agent state
- If the user asked for "filename only", "URL only", or similar, obey exactly
- If the result is missing or failed, say so plainly and briefly
- Do not invent facts that are not present in the provided results"#
    }

    pub fn synthesis_user(
        state: &AgentState,
        step: &PlannedStep,
        history_summary: &str,
        tool_results: &[ToolResult],
    ) -> String {
        let tool_output = if tool_results.is_empty() {
            "none".to_string()
        } else {
            tool_results
                .iter()
                .enumerate()
                .map(|(index, result)| {
                    format!(
                        "[{}] success={}\n{}",
                        index + 1,
                        result.success,
                        truncate(&serde_json::to_string_pretty(result).unwrap_or_default(), 2000)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        format!(
            "USER GOAL:\n{goal}\n\nCURRENT STEP:\n{step}\n\nCOMPLETED STEP HISTORY:\n{history}\n\nLATEST TOOL RESULTS:\n{tool_results}\n\nWrite the final answer for the user now.",
            goal = state.goal,
            step = step.description,
            history = if history_summary.trim().is_empty() { "none" } else { history_summary },
            tool_results = tool_output,
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// EVALUATOR PROMPTS
// ═══════════════════════════════════════════════════════════════════════════

pub struct EvaluatorPrompt;

impl EvaluatorPrompt {
    pub fn system() -> &'static str {
        r#"You are a step evaluator for an autonomous AI agent platform.

Classify the outcome of a completed step as exactly one of: CONTINUE, RETRY, or ABORT

CONTINUE — step succeeded, move to next step
  Conditions (all must hold):
  - All tool calls returned success=true
  - Output contains the expected result for this step
  - Response ends with STEP COMPLETE
  - No unresolved errors remain

RETRY — step failed but is recoverable (max 3 retries per step)
  Use RETRY for:
  - Transient network/service errors (timeout, 503, 429 rate limit, connection refused)
  - File not found where the path may be wrong (agent can correct it)
  - Partial output where a retry would complete it
  - LLM produced no tool call when one was clearly needed
  Do NOT retry:
  - The same error appearing on retry 3 or higher
  - Auth failures (401, 403) — credentials do not self-heal
  - Tool does not exist in registry — configuration error

ABORT — step failed permanently
  Use ABORT for:
  - Auth failure (401, 403) on any tool call
  - Tool not found in registry
  - File permission denied (structural, not fixable by agent)
  - Goal is provably impossible given available tools
  - 3 or more consecutive retries with identical failure patterns
  - Response ends with STEP FAILED

Respond with EXACTLY:
Line 1: CONTINUE | RETRY | ABORT
Line 2: One sentence explaining why (mention the specific tool name and error if applicable)"#
    }

    pub fn user(state: &AgentState, step: &PlannedStep, result: &StepResult, retry_count: u32) -> String {
        let failures: Vec<String> = result
            .tool_results
            .iter()
            .filter(|r| !r.success)
            .map(|r| r.error.clone().unwrap_or_else(|| "unknown error".into()))
            .collect();

        // ← Now includes actual tool names, not just ok/fail counts
        let tool_summary = if result.tools_called.is_empty() {
            "no tools called".into()
        } else {
            result
                .tools_called
                .iter()
                .zip(result.tool_results.iter().chain(std::iter::repeat(&ToolResult {
                    success: false,
                    output: serde_json::Value::Null,
                    error: None,
                })))
                .map(|(name, r)| format!("{} → {}", name, if r.success { "✓" } else { "✗" }))
                .collect::<Vec<_>>()
                .join(", ")
        };

        format!(
            "GOAL: {goal}\nSTEP [{idx}]: {desc}\nRETRY #{retry}\n\nTOOLS CALLED: {tools}\nSTEP OUTPUT:\n{output}\nFAILURE DETAILS: {failures}",
            goal     = state.goal,
            idx      = step.index,
            desc     = step.description,
            retry    = retry_count,
            tools    = tool_summary,
            output   = truncate(&result.output, 800),
            failures = if failures.is_empty() { "none".into() } else { failures.join(" | ") },
        )
    }

    /// Combined evaluate + reflect system prompt.
    /// Produces one JSON response that replaces two separate LLM calls.
    pub fn combined_system() -> &'static str {
        r#"You are a step evaluator and reflection assistant for an autonomous AI agent.

Given a completed step, respond with a SINGLE JSON object that covers both
evaluation (verdict) and reflection (summary, findings, plan revision).

VERDICT must be exactly one of:
  CONTINUE  — step succeeded, proceed to next step
  RETRY     — step failed but is recoverable (transient error, wrong path, partial output)
  ABORT     — step failed permanently (auth error, tool not found, 3+ identical failures)
  COMPLETE  — this was the final step and the goal is done

REVISE the plan only when:
  - A future step is now provably wrong or impossible
  - A key dependency does not exist
  - The goal scope changed significantly

RESPOND with exactly this JSON — no markdown, no extra text:
{
  "verdict":       "CONTINUE | RETRY | ABORT | COMPLETE",
  "summary":       "one sentence: what happened (max 140 chars)",
  "key_findings":  ["concrete fact 1", "concrete fact 2"],
  "revise":        false,
  "feedback":      ""
}

key_findings: 0-3 concrete facts discovered this step (e.g. "node version pinned to 14").
feedback: only populated when revise=true — specific instruction for plan revision (max 300 chars)."#
    }

    /// Combined evaluate + reflect user prompt.
    pub fn combined_user(
        state: &AgentState,
        plan: &Plan,
        step: &PlannedStep,
        result: &StepResult,
        retry_count: u32,
    ) -> String {
        let failures: Vec<String> = result
            .tool_results
            .iter()
            .filter(|r| !r.success)
            .map(|r| r.error.clone().unwrap_or_else(|| "unknown error".into()))
            .collect();

        let tool_summary = if result.tools_called.is_empty() {
            "no tools called".into()
        } else {
            result
                .tools_called
                .iter()
                .zip(result.tool_results.iter().chain(std::iter::repeat(&ToolResult {
                    success: false,
                    output: serde_json::Value::Null,
                    error: None,
                })))
                .map(|(name, r)| format!("{} → {}", name, if r.success { "✓" } else { "✗" }))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let remaining = plan.steps.len().saturating_sub(step.index + 1);

        format!(
            "GOAL: {goal}\nSTEP [{idx}/{total}]: {desc}\nSUCCESS: {ok}\nRETRY #{retry}\nREMAINING STEPS: {rem}\n\nTOOLS CALLED: {tools}\nSTEP OUTPUT:\n{output}\nFAILURE DETAILS: {failures}",
            goal     = state.goal,
            idx      = step.index,
            total    = plan.steps.len().saturating_sub(1),
            desc     = step.description,
            ok       = result.success,
            retry    = retry_count,
            rem      = remaining,
            tools    = tool_summary,
            output   = truncate(&result.output, 800),
            failures = if failures.is_empty() { "none".into() } else { failures.join(" | ") },
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// REFLECTOR PROMPTS
// ═══════════════════════════════════════════════════════════════════════════

pub struct ReflectorPrompt;

impl ReflectorPrompt {
    pub fn system() -> &'static str {
        r#"You are a reflection assistant for an autonomous AI agent.

After each step completes, produce a brief reflection and decide if the remaining plan needs revision.

REVISE when:
- The step revealed that a future planned step is impossible or wrong (e.g. wrong path, API unavailable, unexpected data shape)
- The output changed the goal's scope or direction significantly
- A dependency assumed by future steps does not exist

DO NOT REVISE when:
- The step succeeded normally — unnecessary overhead
- A step needed a retry but the plan itself is still correct
- Minor deviations that do not affect future steps

RESPOND with exactly this JSON — no other text, no markdown fences:
{
  "summary": "one sentence: what happened and what it means for the goal (max 140 chars)",
  "key_findings": ["finding 1", "finding 2"],
  "revise": false,
  "feedback": ""
}

Or if revision is needed:
{
  "summary": "one sentence: what happened (max 140 chars)",
  "key_findings": ["finding that triggered revision"],
  "revise": true,
  "feedback": "specific instruction: what to change in the remaining plan and why (max 300 chars)"
}

key_findings: 0-3 concrete facts discovered this step worth storing in memory.
Return ONLY the JSON."#
    }

    pub fn user(state: &AgentState, plan: &Plan, result: &StepResult) -> String {
        let step = plan.steps.get(result.step_index);
        let remaining = plan.steps.len().saturating_sub(result.step_index + 1);

        // Include tool names in reflector context so it can reason about what actually ran
        let tools_ctx = if result.tools_called.is_empty() {
            String::new()
        } else {
            format!("\nTOOLS USED: {}", result.tools_called.join(", "))
        };

        format!(
            "GOAL: {goal}\nSTEP [{idx}]: {desc}\nSUCCESS: {ok}{tools}\nOUTPUT:\n{output}\nREMAINING STEPS: {rem}",
            goal = state.goal,
            idx = result.step_index,
            desc = step.map(|s| s.description.as_str()).unwrap_or("unknown"),
            ok = result.success,
            tools = tools_ctx,
            output = truncate(&result.output, 600),
            rem = remaining,
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PREFLIGHT PROMPT
// ═══════════════════════════════════════════════════════════════════════════

pub struct PreflightPrompt;

impl PreflightPrompt {
    pub fn system() -> &'static str {
        r#"You are a capability validator for an autonomous AI agent.

Given a goal and the available tool categories, determine if the goal is achievable.

OUTPUT — valid JSON only, no markdown fences:
{ "feasible": true, "missing_tools": [], "reason": "" }

or:
{ "feasible": false, "missing_tools": ["specific_tool_needed"], "reason": "one sentence why not" }

RULES:
1. Mark feasible=false ONLY if the goal fundamentally requires a capability not present
2. Do NOT mark infeasible because the task is hard or multi-step
3. Do NOT mark infeasible if the goal can be approximated with available tools
4. If feasible=true, reason and missing_tools must be empty"#
    }

    /// Uses tool_manifest (grouped) instead of 65 individual names.
    pub fn user(goal: &str, tool_manifest: &str) -> String {
        format!(
            "GOAL: {goal}\n\n{manifest}\n\nIs this goal achievable with these tools?",
            goal = goal,
            manifest = tool_manifest,
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STEP HISTORY — tiered compression
// ═══════════════════════════════════════════════════════════════════════════

/// Builds a compact summary of completed steps for the executor's context window.
///
/// Tiered compression strategy:
///   - Last 3 steps: full output (up to 600 chars each) — most relevant for current step
///   - Steps 4-8: medium summary (up to 200 chars each)
///   - Older steps: header only (step index + description + success)
///
/// This keeps the context window bounded regardless of how many steps run,
/// while keeping the most recent and relevant information fully intact.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StepHistory {
    entries: Vec<StepEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StepEntry {
    index: usize,
    desc: String,
    success: bool,
    output: String, // full, untruncated — we truncate at render time
}

impl StepHistory {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn push(&mut self, index: usize, desc: String, success: bool, output: &str) {
        self.entries.push(StepEntry { index, desc, success, output: output.to_string() });
    }

    pub fn inject_facts(&mut self, facts: &str) {
        if facts.is_empty() {
            return;
        }
        self.entries.push(StepEntry {
            index: 0,
            desc: "knowledge_graph".into(),
            success: true,
            output: facts.to_string(),
        });
    }

    /// Render history with tiered compression.
    pub fn summarise(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let n = self.entries.len();
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let icon = if e.success { "✓" } else { "✗" };
                let age = n - 1 - i; // 0 = most recent
                let output = if age < 3 {
                    // Recent steps: up to 600 chars
                    truncate(&e.output, 600).to_string()
                } else if age < 8 {
                    // Mid steps: up to 200 chars
                    truncate(&e.output, 200).to_string()
                } else {
                    // Old steps: header only
                    String::new()
                };

                if output.is_empty() {
                    format!("[{}] {} {}", e.index, icon, e.desc)
                } else {
                    format!("[{}] {} {} → {}", e.index, icon, e.desc, output)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for StepHistory {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

pub fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        None => s,
        Some((i, _)) => &s[..i],
    }
}
