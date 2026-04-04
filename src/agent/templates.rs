//! Pre-built role templates for the plan mode template picker.
//!
//! Each template carries:
//!   - A complete `intent_cache` JSON — bypasses the IntentExtractor LLM call entirely
//!   - A pre-built `AgentRole` skeleton with typed ExecutionGuidelines, FailureRules,
//!     and CompletionCriteria specific to that workflow
//!   - A list of `pending_clarifications` — only the questions that are genuinely
//!     unknown for that user (connector credentials, Slack channel, DB name, etc.)
//!
//! When a template is selected, `start_plan_mode_session` skips CapturingIntent,
//! applies the template, and enters CapturingClarifications with a short queue
//! of only the personalisation questions. Zero redundant questions.

use serde::Serialize;

use crate::agent::definition::{
    AgentRole, CompletionCriterion, ExecutionGuidelines, FailureAction, FailureRule, GuidelineRule, TriggerDef,
    TriggerType,
};

/// A pre-built template that fully describes a role without an LLM call.
#[derive(Debug, Clone, Serialize)]
pub struct RoleTemplate {
    /// Unique slug — passed in `template_id` to `start_plan_mode_session`.
    pub id: &'static str,
    /// Short display name for the picker card.
    pub name: &'static str,
    /// One-sentence description shown under the name.
    pub description: &'static str,
    /// Persona group: "teams" | "founders" | "personal"
    pub persona: &'static str,
    /// Primary job category — maps to domain skill and segment services.
    pub category: &'static str,
    /// Emoji shown on the picker card.
    pub emoji: &'static str,
    /// Connectors this template requires. Any not installed trigger a credential step.
    #[serde(serialize_with = "serialize_static_strs")]
    pub required_connectors: &'static [&'static str],
    /// Full intent JSON — injected as `intent_cache`, bypasses IntentExtractor.
    #[serde(skip)]
    pub intent: fn() -> serde_json::Value,
    /// Pre-built role skeleton — guidelines, failure rules, completion criteria.
    #[serde(skip)]
    pub build_role: fn(agent_id: &str, tenant_id: &str) -> AgentRole,
    /// IDs of clarification steps to still ask (from `plan_mode_steps::StepField` names).
    /// Only genuinely unknown per-user values: channel names, DB names, thresholds.
    #[serde(serialize_with = "serialize_static_strs")]
    pub ask_steps: &'static [&'static str],
}

fn serialize_static_strs<S: serde::Serializer>(v: &&'static [&'static str], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(v.len()))?;
    for item in *v {
        seq.serialize_element(item)?;
    }
    seq.end()
}

// ── Template registry ────────────────────────────────────────────────────────

pub fn all_templates() -> &'static [RoleTemplate] {
    &TEMPLATES
}

pub fn find_template(id: &str) -> Option<&'static RoleTemplate> {
    TEMPLATES.iter().find(|t| t.id == id)
}

// ── Helper macro ─────────────────────────────────────────────────────────────

macro_rules! always {
    ($text:expr) => {
        GuidelineRule::always($text)
    };
}
macro_rules! before {
    ($tool:expr, $text:expr) => {
        GuidelineRule::before($tool, $text)
    };
}
macro_rules! after {
    ($tool:expr, $text:expr) => {
        GuidelineRule::after($tool, $text)
    };
}
macro_rules! skip_log {
    ($text:expr, $scope:expr) => {
        FailureRule {
            text: $text.into(),
            tool_scope: Some($scope.into()),
            action: FailureAction::SkipAndLog { log_path: "workspace/errors.txt".into() },
        }
    };
    ($text:expr) => {
        FailureRule {
            text: $text.into(),
            tool_scope: None,
            action: FailureAction::SkipAndLog { log_path: "workspace/errors.txt".into() },
        }
    };
}
macro_rules! escalate {
    ($text:expr, $channel:expr) => {
        FailureRule {
            text: $text.into(),
            tool_scope: None,
            action: FailureAction::EscalateToHuman { notify_channel: Some($channel.into()) },
        }
    };
}
macro_rules! retry {
    ($text:expr, $scope:expr) => {
        FailureRule { text: $text.into(), tool_scope: Some($scope.into()), action: FailureAction::RetryOnce }
    };
}

// ── 22 Templates ─────────────────────────────────────────────────────────────

static TEMPLATES: [RoleTemplate; 23] = [
    // ── 1. Invoice Processor ─────────────────────────────────────────────────
    RoleTemplate {
        id: "invoice_processor",
        name: "Invoice Processor",
        description: "Extract invoices from email, match to POs, post to accounting — flag anomalies for approval",
        persona: "teams",
        category: "finance_accounting",
        emoji: "🧾",
        required_connectors: &["gmail", "quickbooks"],
        intent: || {
            serde_json::json!({
                "category":              "finance_accounting",
                "trigger_hint":          "webhook",
                "trigger_confidence":    "high",
                "trigger_source":        "gmail",
                "trigger_event":         "email_received",
                "output_hint":           "connector_record",
                "output_destination_hint": "quickbooks",
                "multi_role_suggested":  false,
                "uses_external_db":      null,
                "actions": [
                    "Extract vendor name, invoice number, amount, line items from PDF attachment",
                    "Match against purchase orders in QuickBooks",
                    "Post matched invoices to QuickBooks accounts payable",
                    "Flag invoices over approval threshold for human review",
                    "Log all mismatches to workspace/reconciliation.txt"
                ],
                "workflow_outline": [
                    "read pdf attachment from email",
                    "match invoice against purchase orders in quickbooks",
                    "post matched invoice to quickbooks accounts payable",
                    "flag invoice for approval if over threshold",
                    "log result to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role =
                AgentRole::new(crate::util::new_id(), agent_id.into(), tenant_id.into(), "Invoice Processor".into());
            role.purpose = "Process incoming invoices: extract, match, post to QuickBooks, flag anomalies".into();
            role.connectors = vec!["gmail".into(), "quickbooks".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("gmail".into()),
                event_filter: Some("email_received".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(before!("pdf_read", "Only process emails with PDF attachments — skip plain text emails"));
            g.add_rule(always!("Extract: vendor name, invoice number, amount, due date, line items"));
            g.add_rule(always!("Match invoice against open POs in QuickBooks before posting"));
            g.add_rule(always!("Never post to QuickBooks without a matching PO or explicit approval"));
            g.add_rule(after!("quickbooks", "Write a one-line confirmation entry to workspace/processed.txt"));
            g.add_rule(always!("Flag invoices over $5,000 for human approval before posting"));
            g.add_failure(skip_log!("Invoice has no matching PO — log and skip", "quickbooks"));
            g.add_failure(skip_log!("Duplicate invoice number detected — log and skip"));
            g.add_failure(escalate!("Invoice amount exceeds $50,000", "#finance-alerts"));
            g.add_failure(retry!("QuickBooks API timeout", "quickbooks"));
            g.add_completion(CompletionCriterion::record_updated("quickbooks", "Invoice posted to QuickBooks"));
            g.add_completion(CompletionCriterion::errors_logged("workspace/errors.txt", "All mismatches logged"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["approval_threshold", "output_dest"],
    },
    // ── 2. Support Ticket Responder ──────────────────────────────────────────
    RoleTemplate {
        id: "support_ticket_responder",
        name: "Support Ticket Responder",
        description: "Draft replies to support tickets using your docs — escalate billing disputes to a human",
        persona: "teams",
        category: "customer_support",
        emoji: "🎫",
        required_connectors: &["zendesk"],
        intent: || {
            serde_json::json!({
                "category":              "customer_support",
                "trigger_hint":          "webhook",
                "trigger_confidence":    "high",
                "trigger_source":        "zendesk",
                "trigger_event":         "ticket_created",
                "output_hint":           "connector_record",
                "output_destination_hint": "zendesk_reply_draft",
                "multi_role_suggested":  false,
                "actions": [
                    "Search help documentation for relevant answers",
                    "Check customer's ticket history for context",
                    "Draft a personalised reply matching customer's tone",
                    "Escalate billing disputes and high-frustration tickets to human",
                    "Attach draft to ticket — never auto-send"
                ],
                "workflow_outline": [
                    "fetch customer ticket history from zendesk",
                    "search help documentation for relevant answers",
                    "draft personalised reply and attach to zendesk ticket",
                    "escalate to human queue if billing dispute or high frustration"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Support Ticket Responder".into(),
            );
            role.purpose = "Draft support replies using help docs — escalate disputes to humans".into();
            role.connectors = vec!["zendesk".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("zendesk".into()),
                event_filter: Some("ticket_created".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(before!("web_fetch", "Search help docs before composing any reply"));
            g.add_rule(always!("Check customer's last 5 tickets for context and tone"));
            g.add_rule(always!("Match the customer's communication style — formal or casual"));
            g.add_rule(always!("Always save as draft in Zendesk — never publish without human review"));
            g.add_rule(always!("If ticket mentions 'billing', 'charge', 'refund', or 'cancel' — escalate immediately"));
            g.add_rule(always!("If sentiment is highly negative — escalate to human queue"));
            g.add_failure(escalate!("Billing dispute or cancellation request detected", "#cs-escalations"));
            g.add_failure(escalate!("Customer expresses high frustration or legal threat", "#cs-escalations"));
            g.add_failure(skip_log!("Help docs search returned no results — flag for manual response"));
            g.add_failure(retry!("Zendesk API error", "zendesk"));
            g.add_completion(CompletionCriterion::record_updated("zendesk", "Draft reply attached to ticket"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["docs_url", "escalation_channel"],
    },
    // ── 3. Contract Risk Reviewer ────────────────────────────────────────────
    RoleTemplate {
        id: "contract_risk_reviewer",
        name: "Contract Risk Reviewer",
        description: "Extract clauses, flag non-standard terms, produce a one-page risk summary for legal sign-off",
        persona: "teams",
        category: "legal_contract",
        emoji: "⚖️",
        required_connectors: &[],
        intent: || {
            serde_json::json!({
                "category":              "legal_contract",
                "trigger_hint":          "user_message",
                "trigger_confidence":    "high",
                "output_hint":           "report",
                "output_destination_hint": "workspace/contract-review/",
                "multi_role_suggested":  false,
                "actions": [
                    "Extract all key clauses from the contract PDF",
                    "Identify liability caps, indemnification, IP ownership, termination, auto-renewal",
                    "Flag clauses that deviate from standard market terms",
                    "Produce a one-page risk summary with severity ratings",
                    "Never provide legal advice — flag for qualified legal review"
                ],
                "workflow_outline": [
                    "read and extract text from contract pdf",
                    "identify key clauses: liability, IP, termination, auto-renewal",
                    "flag non-standard or risky terms with severity rating",
                    "write one-page risk summary to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Contract Risk Reviewer".into(),
            );
            role.purpose = "Extract contract clauses, flag risks, produce review summary for legal sign-off".into();
            role.connectors = vec![];
            role.trigger = TriggerDef { trigger_type: TriggerType::UserMessage, ..Default::default() };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(before!("pdf_read", "Verify the uploaded file is a contract before processing"));
            g.add_rule(always!("Extract and label: liability cap, indemnification, IP ownership, termination clauses, auto-renewal, governing law, dispute resolution"));
            g.add_rule(always!("Flag: uncapped liability, broad IP assignment, one-sided termination, evergreen auto-renewal, unusual jurisdiction"));
            g.add_rule(always!("Rate each flagged clause: Low / Medium / High risk with one-line explanation"));
            g.add_rule(always!("Produce output in workspace/contract-review/{filename}-review.md"));
            g.add_rule(always!("Always end summary with: 'This is a preliminary flag — not legal advice. Have qualified counsel review before signing.'"));
            g.add_failure(escalate!(
                "Contract contains unusual clauses requiring immediate legal review",
                "#legal-team"
            ));
            g.add_failure(skip_log!("Could not extract text from PDF — may be scanned image"));
            g.add_completion(CompletionCriterion::output_exists(
                "workspace/contract-review/",
                "Review summary written",
            ));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["output_dest"],
    },
    // ── 4. New Employee Onboarding ───────────────────────────────────────────
    RoleTemplate {
        id: "employee_onboarding",
        name: "New Employee Onboarding",
        description: "When a hire is added, send their checklist, create accounts, schedule day-one meetings",
        persona: "teams",
        category: "hr_people_ops",
        emoji: "👋",
        required_connectors: &["greenhouse", "gmail"],
        intent: || {
            serde_json::json!({
                "category":           "hr_people_ops",
                "trigger_hint":       "webhook",
                "trigger_confidence": "high",
                "trigger_source":     "greenhouse",
                "trigger_event":      "candidate_hired",
                "output_hint":        "email_draft",
                "multi_role_suggested": false,
                "actions": [
                    "Send personalised onboarding checklist email to new hire",
                    "Log new hire details to HR database",
                    "Schedule day-one orientation meeting",
                    "Send welcome email to team announcing the new hire",
                    "Create follow-up check-in at day 7 and day 30"
                ],
                "workflow_outline": [
                    "fetch new hire details from greenhouse",
                    "send personalised onboarding checklist email via gmail",
                    "schedule day-one orientation and follow-up check-ins"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "New Employee Onboarding".into(),
            );
            role.purpose = "Automate new hire onboarding: checklist, accounts, day-one setup".into();
            role.connectors = vec!["greenhouse".into(), "gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("greenhouse".into()),
                event_filter: Some("candidate_hired".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Extract: name, role, start date, manager, department from Greenhouse record"));
            g.add_rule(always!("Send personalised checklist email — use new hire's name and role throughout"));
            g.add_rule(always!("CC the hiring manager on all communications"));
            g.add_rule(always!("Create a follow-up task at day 7 and day 30 using the schedule tool"));
            g.add_rule(always!("Never send emails with placeholder text like [NAME] — verify all substitutions"));
            g.add_failure(escalate!("Missing required fields in Greenhouse record", "#hr-ops"));
            g.add_failure(skip_log!("Email delivery failed — log for manual retry"));
            g.add_completion(CompletionCriterion::all_items(
                "Greenhouse new hire records",
                "All steps completed for new hire",
            ));
            g.add_completion(CompletionCriterion::errors_logged("workspace/errors.txt", "Any issues logged"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["output_dest"],
    },
    // ── 5. Compliance Deadline Monitor ───────────────────────────────────────
    RoleTemplate {
        id: "compliance_deadline_monitor",
        name: "Compliance Deadline Monitor",
        description: "Every morning check all client deadlines — email reminders, Slack escalation for overdue",
        persona: "teams",
        category: "finance_accounting",
        emoji: "📅",
        required_connectors: &["gmail", "slack"],
        intent: || {
            serde_json::json!({
                "category":           "finance_accounting",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 8 * * 1-5",
                "trigger_confidence": "high",
                "output_hint":        "notification",
                "multi_role_suggested": false,
                "uses_external_db":   null,
                "actions": [
                    "Query all active clients with upcoming deadlines",
                    "Send personalised reminder emails at 14, 7, 3, and 1 day before deadline",
                    "Escalate overdue deadlines to compliance Slack channel",
                    "Draft remediation note for overdue items",
                    "Log all actions to workspace/deadline-log.txt"
                ],
                "workflow_outline": [
                    "query active clients with upcoming deadlines",
                    "send tiered reminder emails via gmail",
                    "escalate overdue deadlines to slack channel",
                    "log all actions to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Compliance Deadline Monitor".into(),
            );
            role.purpose = "Monitor client deadlines daily and send tiered reminders and escalations".into();
            role.connectors = vec!["gmail".into(), "slack".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 8 * * 1-5".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Query deadlines: sort by days_remaining ascending"));
            g.add_rule(always!("14 days remaining → send friendly reminder email"));
            g.add_rule(always!("7 days remaining → send urgent reminder, CC manager"));
            g.add_rule(always!("3 days remaining → send urgent email + Slack DM to assigned person"));
            g.add_rule(always!("1 day remaining → all of the above + flag in Slack channel"));
            g.add_rule(always!("Overdue → post to compliance Slack channel with draft remediation note"));
            g.add_rule(always!("Log every action taken to workspace/deadline-log.txt with timestamp"));
            g.add_failure(escalate!("Client is overdue with no response after 3 reminders", "#compliance-alerts"));
            g.add_failure(retry!("Database query timeout", "external_db"));
            g.add_completion(CompletionCriterion::all_items("client deadline records", "All deadlines checked"));
            g.add_completion(CompletionCriterion::errors_logged("workspace/deadline-log.txt", "All actions logged"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["db_name", "escalation_channel"],
    },
    // ── 6. Sales Pipeline Health ─────────────────────────────────────────────
    RoleTemplate {
        id: "sales_pipeline_health",
        name: "Sales Pipeline Health",
        description: "Every Monday flag stale deals, research company news, email account owners with context",
        persona: "teams",
        category: "sales_revops",
        emoji: "📊",
        required_connectors: &["salesforce", "gmail"],
        intent: || {
            serde_json::json!({
                "category":           "sales_revops",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 8 * * 1",
                "trigger_confidence": "high",
                "output_hint":        "email_draft",
                "multi_role_suggested": false,
                "actions": [
                    "Pull Salesforce pipeline — filter deals with no activity in 14+ days",
                    "For each stale deal, search web for recent news about the company",
                    "Draft a personalised nudge email to the account owner with the news context",
                    "Update Salesforce last_reviewed_at field",
                    "Log stale deal count to workspace/pipeline-report.txt"
                ],
                "workflow_outline": [
                    "pull stale deals from salesforce",
                    "search web for recent news about each company",
                    "draft personalised nudge emails via gmail",
                    "update salesforce last_reviewed_at for each deal"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Sales Pipeline Health".into(),
            );
            role.purpose = "Weekly stale pipeline review with contextual nudge emails".into();
            role.connectors = vec!["salesforce".into(), "gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 8 * * 1".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(before!("salesforce", "Only process deals in Negotiation, Proposal, or Demo stages"));
            g.add_rule(always!("Stale = no Salesforce activity update in 14+ days"));
            g.add_rule(always!(
                "For each stale deal: search '[company name] news site:techcrunch.com OR site:reuters.com' for context"
            ));
            g.add_rule(always!("Draft email: mention the specific news item — never send a generic nudge"));
            g.add_rule(always!("Emails go to drafts — account owner reviews before sending"));
            g.add_rule(after!("salesforce", "Update last_reviewed_at in Salesforce for every processed deal"));
            g.add_failure(skip_log!("No news found for company — send generic nudge with flag", "web_search"));
            g.add_failure(retry!("Salesforce API error", "salesforce"));
            g.add_completion(CompletionCriterion::all_items("Salesforce stale deals", "All stale deals processed"));
            g.add_completion(CompletionCriterion::record_updated("salesforce", "last_reviewed_at updated"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["inactivity_days", "output_dest"],
    },
    // ── 7. Competitor Intelligence Brief ────────────────────────────────────
    RoleTemplate {
        id: "competitor_intelligence",
        name: "Competitor Intelligence Brief",
        description: "Every Friday research competitors for product changes, hiring signals, funding — post to Slack",
        persona: "teams",
        category: "research_analyst",
        emoji: "🔍",
        required_connectors: &["slack"],
        intent: || {
            serde_json::json!({
                "category":           "research_analyst",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 9 * * 5",
                "trigger_confidence": "high",
                "output_hint":        "slack_message",
                "multi_role_suggested": false,
                "actions": [
                    "Search for news and press releases about each competitor",
                    "Check competitor job postings for strategic signals",
                    "Check their website changelog or product blog for new features",
                    "Search for funding announcements or leadership changes",
                    "Synthesise into a structured brief and post to Slack"
                ],
                "workflow_outline": [
                    "search web for competitor news and announcements",
                    "fetch competitor blogs and changelogs",
                    "synthesise findings into structured brief",
                    "post brief to slack and save to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Competitor Intelligence Brief".into(),
            );
            role.purpose = "Weekly structured competitor research delivered to Slack".into();
            role.connectors = vec!["slack".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 9 * * 5".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!(
                "For each competitor: search '[name] news', '[name] new features', '[name] funding', '[name] jobs'"
            ));
            g.add_rule(always!("Check their official blog and changelog via web_fetch if URL is known"));
            g.add_rule(always!(
                "Structure output: one section per competitor with subheadings: Product, Hiring, Business"
            ));
            g.add_rule(always!("Only include developments from the last 7 days — discard older items"));
            g.add_rule(always!("If nothing significant happened for a competitor, say so explicitly — do not pad"));
            g.add_rule(always!("Cite every source with URL — no uncited claims"));
            g.add_rule(always!("Post to Slack channel as a formatted message, also save to workspace/intel/"));
            g.add_failure(skip_log!("No significant news found for competitor this week — note in brief"));
            g.add_failure(retry!("Slack API error", "slack"));
            g.add_completion(CompletionCriterion::output_exists("workspace/intel/", "Intel brief saved"));
            g.add_completion(CompletionCriterion::record_updated("slack", "Brief posted to Slack"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["competitor_names", "slack_channel"],
    },
    // ── 8. Investor Update Writer ────────────────────────────────────────────
    RoleTemplate {
        id: "investor_update_writer",
        name: "Investor Update Writer",
        description: "Every Friday pull your metrics, compare to last week, and draft an investor update for review",
        persona: "founders",
        category: "finance_accounting",
        emoji: "📈",
        required_connectors: &["gmail"],
        intent: || {
            serde_json::json!({
                "category":           "finance_accounting",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 17 * * 5",
                "trigger_confidence": "high",
                "output_hint":        "email_draft",
                "output_destination_hint": "gmail_draft",
                "multi_role_suggested": false,
                "uses_external_db":   null,
                "actions": [
                    "Pull this week's revenue, user signups, churn, and key metrics from database",
                    "Compare to previous week — calculate deltas and percentage changes",
                    "Draft investor update in concise founder voice: numbers first, narrative second",
                    "Flag any significant anomalies for the founder to address",
                    "Save as Gmail draft — never send without founder approval"
                ],
                "workflow_outline": [
                    "query key metrics from database",
                    "compare metrics to prior week and calculate deltas",
                    "draft investor update email in founder voice",
                    "save draft to gmail and workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Investor Update Writer".into(),
            );
            role.purpose = "Weekly investor update from database metrics — draft for founder review".into();
            role.connectors = vec!["gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 17 * * 5".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Pull metrics: MRR, new signups, churn, active users, key product wins this week"));
            g.add_rule(always!("Always compare to the same period last week — show absolute and percentage change"));
            g.add_rule(always!(
                "Format: 3 numbers up front, then 2-3 sentences of narrative, then asks/blockers if any"
            ));
            g.add_rule(always!("Tone: confident, direct, no filler — write as the founder would"));
            g.add_rule(always!("Save as Gmail draft to investor list — NEVER send directly"));
            g.add_rule(always!("If MRR decreased more than 5% WoW — add a flag comment for founder to explain"));
            g.add_failure(escalate!(
                "Critical metric missing from database — cannot produce accurate update",
                "#founder-alerts"
            ));
            g.add_failure(retry!("Database connection timeout", "external_db"));
            g.add_completion(CompletionCriterion::output_exists("workspace/updates/", "Draft saved to workspace"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["db_name", "metrics_table", "investor_email"],
    },
    // ── 9. Customer Churn Early Warning ─────────────────────────────────────
    RoleTemplate {
        id: "churn_early_warning",
        name: "Customer Churn Early Warning",
        description: "Daily: find customers gone quiet, draft personalised re-engagement emails for review",
        persona: "founders",
        category: "sales_revops",
        emoji: "⚠️",
        required_connectors: &["gmail"],
        intent: || {
            serde_json::json!({
                "category":           "sales_revops",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 9 * * 1-5",
                "trigger_confidence": "high",
                "output_hint":        "email_draft",
                "multi_role_suggested": false,
                "uses_external_db":   null,
                "actions": [
                    "Query customers who haven't logged in for 21+ days",
                    "Look up their account: plan, last feature used, usage history",
                    "Draft a personalised re-engagement email referencing their specific usage",
                    "Queue drafts for founder review — never auto-send",
                    "Log churn risk customers to workspace/churn-watch.csv"
                ],
                "workflow_outline": [
                    "query inactive customers from database",
                    "look up account details and last feature used",
                    "draft personalised re-engagement emails via gmail",
                    "log at-risk customers to churn watch csv"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Customer Churn Early Warning".into(),
            );
            role.purpose = "Daily churn detection with personalised re-engagement drafts".into();
            role.connectors = vec!["gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 9 * * 1-5".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Query: customers with last_login older than 21 days AND is_paying = true"));
            g.add_rule(always!("For each: look up their last feature used, their plan tier, account age"));
            g.add_rule(always!(
                "Personalise email: mention the specific feature they used last — avoid generic 'we miss you'"
            ));
            g.add_rule(always!("Subject line must reference something specific about their account"));
            g.add_rule(always!("Save all drafts to workspace/churn-emails/ — queue for review, never auto-send"));
            g.add_rule(always!("Append each at-risk customer to workspace/churn-watch.csv with reason"));
            g.add_failure(skip_log!("Customer email address missing — log to errors.txt"));
            g.add_completion(CompletionCriterion::all_items(
                "at-risk customer records",
                "All at-risk customers processed",
            ));
            g.add_completion(CompletionCriterion::output_exists(
                "workspace/churn-watch.csv",
                "Churn watch list updated",
            ));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["db_name", "inactivity_days"],
    },
    // ── 10. Job Applicant Screener ───────────────────────────────────────────
    RoleTemplate {
        id: "applicant_screener",
        name: "Job Applicant Screener",
        description: "Score new applications, research candidates online, draft invite or decline — never auto-send",
        persona: "founders",
        category: "hr_people_ops",
        emoji: "🧑‍💼",
        required_connectors: &["greenhouse", "gmail"],
        intent: || {
            serde_json::json!({
                "category":           "hr_people_ops",
                "trigger_hint":       "webhook",
                "trigger_confidence": "high",
                "trigger_source":     "greenhouse",
                "trigger_event":      "application_submitted",
                "output_hint":        "email_draft",
                "multi_role_suggested": false,
                "actions": [
                    "Score application against role requirements",
                    "Research candidate online: GitHub, LinkedIn, blog, published work",
                    "Check for relevant open source contributions or public writing",
                    "Draft personalised interview invite or respectful decline",
                    "Tag candidate profile in Greenhouse with score and research notes",
                    "Never send email without hiring manager approval"
                ],
                "workflow_outline": [
                    "score application against role requirements",
                    "search candidate online via web search",
                    "draft personalised invite or decline email",
                    "update candidate profile in greenhouse"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Job Applicant Screener".into(),
            );
            role.purpose = "Score applicants, research them online, draft responses for human approval".into();
            role.connectors = vec!["greenhouse".into(), "gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("greenhouse".into()),
                event_filter: Some("application_submitted".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Score: 1-10 on each requirement from the job spec — explain each score"));
            g.add_rule(always!("Research: search '[name] github', '[name] linkedin', '[name] blog' for signal"));
            g.add_rule(always!("Never make assessments based on name, location, or school — only work and skills"));
            g.add_rule(always!("Draft invite if score ≥ 7/10 on core requirements — draft decline otherwise"));
            g.add_rule(always!("Personalise invite: mention one specific thing from their work that impressed you"));
            g.add_rule(always!("Save draft to Greenhouse candidate profile — NEVER send directly"));
            g.add_failure(skip_log!("Candidate has no online presence to research — note in profile"));
            g.add_failure(escalate!("Application appears fraudulent or contains plagiarised content", "#hiring"));
            g.add_completion(CompletionCriterion::record_updated("greenhouse", "Candidate profile updated with score"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["job_requirements", "output_dest"],
    },
    // ── 11. Pre-Demo Sales Brief ─────────────────────────────────────────────
    RoleTemplate {
        id: "pre_demo_brief",
        name: "Pre-Demo Sales Brief",
        description: "When a demo is booked, research the company and deliver a prep brief 30 minutes before",
        persona: "founders",
        category: "sales_revops",
        emoji: "🎯",
        required_connectors: &["hubspot"],
        intent: || {
            serde_json::json!({
                "category":           "sales_revops",
                "trigger_hint":       "webhook",
                "trigger_confidence": "high",
                "trigger_source":     "hubspot",
                "trigger_event":      "meeting_booked",
                "output_hint":        "report",
                "output_destination_hint": "workspace/briefs/",
                "multi_role_suggested": false,
                "actions": [
                    "Look up company: funding stage, employee count, industry, tech stack",
                    "Find recent news and press releases about the company",
                    "Check job postings for signals about their priorities",
                    "Find the prospect's LinkedIn profile and recent activity",
                    "Check if any mutual connections exist",
                    "Produce a one-page brief: company context, likely pain points, talking points"
                ],
                "workflow_outline": [
                    "research company via web search",
                    "search for recent news and job postings",
                    "look up prospect profile and activity",
                    "write one-page prep brief to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role =
                AgentRole::new(crate::util::new_id(), agent_id.into(), tenant_id.into(), "Pre-Demo Sales Brief".into());
            role.purpose = "Research prospect company and deliver a prep brief before the call".into();
            role.connectors = vec!["hubspot".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("hubspot".into()),
                event_filter: Some("meeting_booked".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Research: company funding, size, industry, tech stack from Crunchbase/LinkedIn"));
            g.add_rule(always!("Find news from last 90 days — funding, product launches, leadership changes"));
            g.add_rule(always!(
                "Scan job postings for signals: what are they hiring for? What problems does that suggest?"
            ));
            g.add_rule(always!("Structure: (1) Company snapshot, (2) Recent news, (3) Likely pain points, (4) Suggested talking points, (5) Questions to ask"));
            g.add_rule(always!("Save to workspace/briefs/{company}-{date}.md"));
            g.add_rule(always!("Also send a Slack DM or email to the sales rep with the brief content"));
            g.add_failure(skip_log!("Company website unreachable — proceed with available data"));
            g.add_completion(CompletionCriterion::output_exists("workspace/briefs/", "Brief produced"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["delivery_channel"],
    },
    // ── 12. Monthly Expense Analyser ─────────────────────────────────────────
    RoleTemplate {
        id: "expense_analyser",
        name: "Monthly Expense Analyser",
        description: "On the 1st, pull last month's expenses, categorise them, flag anomalies vs 3-month average",
        persona: "founders",
        category: "finance_accounting",
        emoji: "💰",
        required_connectors: &["quickbooks", "gmail"],
        intent: || {
            serde_json::json!({
                "category":           "finance_accounting",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 9 1 * *",
                "trigger_confidence": "high",
                "output_hint":        "report",
                "multi_role_suggested": false,
                "actions": [
                    "Pull all expenses from QuickBooks for last calendar month",
                    "Categorise by vendor, category, and cost centre",
                    "Compare each category against 3-month rolling average",
                    "Flag categories that increased more than 20% month-over-month",
                    "Produce a one-page summary with anomalies highlighted",
                    "Email draft to founder for review"
                ],
                "workflow_outline": [
                    "pull last month expenses from quickbooks",
                    "categorise expenses and compare to 3-month average",
                    "flag anomalous categories and large new transactions",
                    "email report draft via gmail and save to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Monthly Expense Analyser".into(),
            );
            role.purpose = "Monthly expense review with anomaly detection and trend analysis".into();
            role.connectors = vec!["quickbooks".into(), "gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 9 1 * *".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Pull QuickBooks expenses for the previous complete calendar month only"));
            g.add_rule(always!("Calculate 3-month rolling average per category"));
            g.add_rule(always!(
                "Flag: any category up more than 20% MoM, any single transaction over $1,000 that is new"
            ));
            g.add_rule(always!(
                "Format: total spend, top 5 categories, anomalies table, month-over-month chart (text)"
            ));
            g.add_rule(always!("Save to workspace/finance/expenses-{month}.md and email as draft"));
            g.add_failure(retry!("QuickBooks API timeout", "quickbooks"));
            g.add_failure(escalate!("Total monthly spend exceeds budget by more than 30%", "#finance-alerts"));
            g.add_completion(CompletionCriterion::output_exists("workspace/finance/", "Expense report saved"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["output_dest"],
    },
    // ── 13. Code Review Assistant ────────────────────────────────────────────
    RoleTemplate {
        id: "code_review_assistant",
        name: "Code Review Assistant",
        description: "When a PR is opened, review the changes and post a plain-language summary to Slack",
        persona: "founders",
        category: "software_engineer",
        emoji: "👨‍💻",
        required_connectors: &["github", "slack"],
        intent: || {
            serde_json::json!({
                "category":           "software_engineer",
                "trigger_hint":       "webhook",
                "trigger_confidence": "high",
                "trigger_source":     "github",
                "trigger_event":      "pull_request_opened",
                "output_hint":        "slack_message",
                "multi_role_suggested": false,
                "actions": [
                    "Read the PR diff and changed files",
                    "Identify the purpose of the change from description and code",
                    "Flag potential issues: security risks, missing tests, breaking changes",
                    "Produce a plain-language summary for non-technical stakeholders",
                    "Post to Slack and add a review comment on the PR"
                ],
                "workflow_outline": [
                    "fetch pull request diff from github",
                    "identify purpose and flag risks in the changes",
                    "post plain-language summary to slack",
                    "add review comment on github pull request"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Code Review Assistant".into(),
            );
            role.purpose = "Summarise PRs in plain language and flag risks — post to Slack".into();
            role.connectors = vec!["github".into(), "slack".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("github".into()),
                event_filter: Some("pull_request_opened".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Read full diff — summarise what changed in 2-3 sentences for non-engineers"));
            g.add_rule(always!(
                "Flag: hardcoded secrets, SQL injection risks, missing error handling, no tests for changed code"
            ));
            g.add_rule(always!("If tests are missing for changed logic — explicitly call this out as a risk"));
            g.add_rule(always!("Post to Slack: title, what it does, risk level (Low/Medium/High), any flags"));
            g.add_rule(always!("Add a GitHub review comment with the technical detail — Slack gets the summary"));
            g.add_failure(skip_log!("PR diff too large to process in one pass — summarise by file"));
            g.add_completion(CompletionCriterion::record_updated("github", "Review comment added to PR"));
            g.add_completion(CompletionCriterion::record_updated("slack", "Summary posted to Slack"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["slack_channel"],
    },
    // ── 14. Tax Document Collector ───────────────────────────────────────────
    RoleTemplate {
        id: "tax_document_collector",
        name: "Tax Document Collector",
        description: "Guide you through collecting every document you need for your taxes — nothing missed",
        persona: "personal",
        category: "finance_accounting",
        emoji: "🗂️",
        required_connectors: &["gmail"],
        intent: || {
            serde_json::json!({
                "category":           "finance_accounting",
                "trigger_hint":       "user_message",
                "trigger_confidence": "high",
                "output_hint":        "report",
                "output_destination_hint": "workspace/tax-docs/",
                "multi_role_suggested": false,
                "actions": [
                    "Interview user to determine their income types and filing situation",
                    "Generate a personalised document checklist based on their answers",
                    "Track which documents have been provided and which are still missing",
                    "When a document is uploaded, extract key figures and confirm they are correct",
                    "Produce a final summary of all collected figures ready for filing"
                ],
                "workflow_outline": [
                    "interview user to determine filing situation",
                    "generate personalised document checklist",
                    "extract key figures from uploaded documents",
                    "save collected figures summary to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Tax Document Collector".into(),
            );
            role.purpose =
                "Guided tax document collection — personalised checklist, figure extraction, filing summary".into();
            role.connectors = vec!["gmail".into()];
            role.trigger = TriggerDef { trigger_type: TriggerType::UserMessage, ..Default::default() };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Start by asking: employment type (W2/1099/self-employed/both), investment income, rental income, dependents, student loan interest, home ownership"));
            g.add_rule(always!("Generate a personalised checklist — do not use a generic list"));
            g.add_rule(always!("For each uploaded document: confirm type, extract key figures (income, withholding, dates), confirm with user"));
            g.add_rule(always!("Track checklist completion — show what's been gathered and what's still needed"));
            g.add_rule(always!(
                "Never give tax advice — present figures only, recommend CPA or tax software for filing"
            ));
            g.add_rule(always!("Save all extracted figures to workspace/tax-docs/summary.json"));
            g.add_failure(skip_log!("Could not extract figures from document — ask user to enter manually"));
            g.add_completion(CompletionCriterion::output_exists(
                "workspace/tax-docs/summary.json",
                "All figures collected",
            ));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["tax_year"],
    },
    // ── 15. Job Application Tracker ──────────────────────────────────────────
    RoleTemplate {
        id: "job_application_tracker",
        name: "Job Application Tracker",
        description: "Track your applications — draft follow-ups automatically at the right time",
        persona: "personal",
        category: "general",
        emoji: "📋",
        required_connectors: &["gmail"],
        intent: || {
            serde_json::json!({
                "category":           "general",
                "trigger_hint":       "user_message",
                "trigger_confidence": "high",
                "output_hint":        "email_draft",
                "multi_role_suggested": false,
                "actions": [
                    "Record each new application with company, role, date, and contact",
                    "Schedule a follow-up check-in 5 business days after application",
                    "Draft a professional follow-up email if no response received",
                    "Track status updates when user reports hearing back",
                    "Maintain a summary of all applications with current status"
                ],
                "workflow_outline": [
                    "record new application details to workspace log",
                    "schedule follow-up check-in after 5 business days",
                    "draft professional follow-up email via gmail"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Job Application Tracker".into(),
            );
            role.purpose = "Track job applications and draft follow-up emails at the right time".into();
            role.connectors = vec!["gmail".into()];
            role.trigger = TriggerDef { trigger_type: TriggerType::UserMessage, ..Default::default() };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!(
                "When user adds an application: log company, role, date applied, hiring manager name/email if known"
            ));
            g.add_rule(always!(
                "Schedule follow-up for 5 business days after application date using the schedule tool"
            ));
            g.add_rule(always!(
                "Follow-up email: professional, brief, reiterate interest, ask politely about timeline"
            ));
            g.add_rule(always!("Never send email directly — always save as draft for user review"));
            g.add_rule(always!("Maintain workspace/applications.csv with all applications and statuses"));
            g.add_rule(always!("When user reports a rejection: log it, ask if they want a thank-you reply"));
            g.add_completion(CompletionCriterion::output_exists(
                "workspace/applications.csv",
                "Application log updated",
            ));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &[],
    },
    // ── 16. Weekly Research Brief ────────────────────────────────────────────
    RoleTemplate {
        id: "weekly_research_brief",
        name: "Weekly Research Brief",
        description: "Every week research your chosen topic and email you a cited 3-paragraph brief",
        persona: "personal",
        category: "research_analyst",
        emoji: "📰",
        required_connectors: &["gmail"],
        intent: || {
            serde_json::json!({
                "category":           "research_analyst",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 8 * * 1",
                "trigger_confidence": "high",
                "output_hint":        "email_draft",
                "multi_role_suggested": false,
                "actions": [
                    "Search for the latest developments in the specified topic",
                    "Find 5-8 high-quality sources from the past 7 days",
                    "Synthesise into 3 focused paragraphs: what happened, why it matters, what comes next",
                    "Include working citations for every claim",
                    "Email as a draft for review"
                ],
                "workflow_outline": [
                    "search web for topic developments from the last 7 days",
                    "synthesise findings into cited 3-paragraph brief",
                    "save brief to workspace and create gmail draft"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Weekly Research Brief".into(),
            );
            role.purpose = "Weekly cited research brief on your chosen topic delivered by email".into();
            role.connectors = vec!["gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 8 * * 1".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Search from last 7 days only — never cite older material without flagging its age"));
            g.add_rule(always!("Use at least 5 distinct sources — do not rely on a single outlet"));
            g.add_rule(always!(
                "Paragraph 1: what happened this week. Paragraph 2: why it matters. Paragraph 3: what to watch next"
            ));
            g.add_rule(always!("Every factual claim must have a citation — format: [Source Name](URL)"));
            g.add_rule(always!("Total length: 250-350 words. Concise but complete."));
            g.add_rule(always!("Save to workspace/briefs/ and create Gmail draft — never auto-send"));
            g.add_failure(skip_log!("Fewer than 3 relevant sources found this week — note in brief and send anyway"));
            g.add_completion(CompletionCriterion::output_exists("workspace/briefs/", "Brief produced and saved"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["research_topic", "output_email"],
    },
    // ── 17. Lease / Contract Plain-English Explainer ────────────────────────
    RoleTemplate {
        id: "document_explainer",
        name: "Document Plain-English Explainer",
        description: "Upload any contract or lease — get a plain-English explanation and flagged unusual clauses",
        persona: "personal",
        category: "legal_contract",
        emoji: "📄",
        required_connectors: &[],
        intent: || {
            serde_json::json!({
                "category":           "legal_contract",
                "trigger_hint":       "user_message",
                "trigger_confidence": "high",
                "output_hint":        "report",
                "output_destination_hint": "workspace/explained/",
                "multi_role_suggested": false,
                "actions": [
                    "Read the uploaded document",
                    "Explain each section in plain English a non-lawyer can understand",
                    "Flag clauses that are unusual, one-sided, or risky",
                    "Highlight key dates, amounts, obligations, and penalties",
                    "Produce a summary with a plain-language verdict"
                ],
                "workflow_outline": [
                    "read and extract text from uploaded document",
                    "explain each section in plain english",
                    "flag unusual one-sided or risky clauses",
                    "write plain-language summary to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Document Plain-English Explainer".into(),
            );
            role.purpose = "Explain any contract or document in plain language and flag risks".into();
            role.connectors = vec![];
            role.trigger = TriggerDef { trigger_type: TriggerType::UserMessage, ..Default::default() };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!("Read the full document before summarising any part"));
            g.add_rule(always!("Explain every section in 1-3 plain-English sentences — avoid legal jargon"));
            g.add_rule(always!("Flag: auto-renewal clauses, early termination penalties, unusual liability language, arbitration clauses, data sharing permissions"));
            g.add_rule(always!(
                "Highlight: key dates (start, end, renewal deadlines), key amounts (rent, fees, penalties)"
            ));
            g.add_rule(always!("End with a plain-language verdict: 'This appears standard' or 'Flagged clauses worth discussing with a professional'"));
            g.add_rule(always!(
                "Always say: this is not legal advice — consult a qualified professional before signing"
            ));
            g.add_failure(skip_log!("Could not extract text from document — may be a scanned image"));
            g.add_completion(CompletionCriterion::output_exists("workspace/explained/", "Explanation produced"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &[],
    },
    // ── 18. Options / Insurance / Mortgage Researcher ───────────────────────
    RoleTemplate {
        id: "options_researcher",
        name: "Options Researcher",
        description: "Research and compare the best options for a major financial decision — explained clearly",
        persona: "personal",
        category: "research_analyst",
        emoji: "🏦",
        required_connectors: &["gmail"],
        intent: || {
            serde_json::json!({
                "category":           "research_analyst",
                "trigger_hint":       "user_message",
                "trigger_confidence": "high",
                "output_hint":        "report",
                "output_destination_hint": "workspace/research/",
                "multi_role_suggested": false,
                "actions": [
                    "Ask user to specify what they are researching and their situation",
                    "Search for current options, rates, or products in their category",
                    "Compare top 3-5 options across relevant criteria",
                    "Explain trade-offs in plain language",
                    "Produce a comparison table and a recommendation summary"
                ],
                "workflow_outline": [
                    "clarify user decision and situation via conversation",
                    "search web for current options rates and products",
                    "compare top 3 to 5 options across key criteria",
                    "write comparison report with recommendation to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role =
                AgentRole::new(crate::util::new_id(), agent_id.into(), tenant_id.into(), "Options Researcher".into());
            role.purpose = "Research and compare options for a major financial or purchasing decision".into();
            role.connectors = vec!["gmail".into()];
            role.trigger = TriggerDef { trigger_type: TriggerType::UserMessage, ..Default::default() };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!(
                "Start by clarifying: what are they deciding, what is their situation, what matters most to them"
            ));
            g.add_rule(always!("Search for current market options — not blog posts, actual product/rate pages"));
            g.add_rule(always!("Compare minimum 3 options across: cost, key features, downsides, who it's best for"));
            g.add_rule(always!("Present as a table + one paragraph per option explaining the trade-off"));
            g.add_rule(always!("Give a clear recommendation with the single most important reason why"));
            g.add_rule(always!(
                "For financial products: include a disclaimer that this is research, not financial advice"
            ));
            g.add_failure(skip_log!("Could not find current pricing — note that rates may have changed"));
            g.add_completion(CompletionCriterion::output_exists("workspace/research/", "Comparison report produced"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &[],
    },
    // ── 19. News Monitor and Alerter ─────────────────────────────────────────
    RoleTemplate {
        id: "news_monitor",
        name: "News Monitor and Alerter",
        description:
            "Monitor news about any company, person, or topic and alert you when something significant happens",
        persona: "personal",
        category: "research_analyst",
        emoji: "🔔",
        required_connectors: &["gmail"],
        intent: || {
            serde_json::json!({
                "category":           "research_analyst",
                "trigger_hint":       "schedule",
                "trigger_cron":       "0 8 * * 1-5",
                "trigger_confidence": "high",
                "output_hint":        "email_draft",
                "multi_role_suggested": false,
                "actions": [
                    "Search for news about the specified subject from the last 24 hours",
                    "Filter for significant developments only — ignore routine coverage",
                    "If significant news found: summarise and send alert email",
                    "If nothing significant: skip — do not send noise",
                    "Log all checked searches to workspace/monitor-log.txt"
                ],
                "workflow_outline": [
                    "search web for news about subject from last 24 hours",
                    "filter results for significant developments only",
                    "send alert email via gmail if significant news found",
                    "log search run to workspace monitor log"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "News Monitor and Alerter".into(),
            );
            role.purpose = "Daily news monitoring with alert emails only when something significant happens".into();
            role.connectors = vec!["gmail".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Schedule,
                cron: Some("0 8 * * 1-5".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!(
                "Search: '[subject] news', '[subject] announcement', '[subject] update' from last 24 hours"
            ));
            g.add_rule(always!("Significant = funding, acquisition, product launch, leadership change, legal action, major partnership"));
            g.add_rule(always!(
                "Routine = earnings beats by less than 5%, generic industry roundups, republished old news"
            ));
            g.add_rule(always!(
                "Only send an email if there is at least one significant development — silence is fine"
            ));
            g.add_rule(always!("Email format: subject line states the news, body is 3-5 sentences with source link"));
            g.add_rule(always!("Log every search run to workspace/monitor-log.txt whether or not an alert was sent"));
            g.add_failure(skip_log!("Search API rate limit hit — log and skip today's check"));
            g.add_completion(CompletionCriterion::errors_logged("workspace/monitor-log.txt", "Check logged"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["monitor_subject", "output_email"],
    },
    // ── 20. Meeting / Interview Prep ─────────────────────────────────────────
    RoleTemplate {
        id: "meeting_prep",
        name: "Meeting and Interview Prep",
        description: "Tell me who you're meeting — I'll research them and have your prep brief ready before the call",
        persona: "personal",
        category: "research_analyst",
        emoji: "🤝",
        required_connectors: &[],
        intent: || {
            serde_json::json!({
                "category":           "research_analyst",
                "trigger_hint":       "user_message",
                "trigger_confidence": "high",
                "output_hint":        "report",
                "output_destination_hint": "workspace/prep/",
                "multi_role_suggested": false,
                "actions": [
                    "Research the person: LinkedIn, published work, recent news, mutual connections",
                    "Research their company: what they do, recent news, size, funding",
                    "Identify likely topics based on the meeting context",
                    "Produce a concise prep brief: who they are, context, talking points, questions to ask"
                ],
                "workflow_outline": [
                    "research person via web search",
                    "research their company via web search",
                    "identify likely topics for this meeting context",
                    "write concise prep brief to workspace"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Meeting and Interview Prep".into(),
            );
            role.purpose = "Research meeting participants and produce a prep brief".into();
            role.connectors = vec![];
            role.trigger = TriggerDef { trigger_type: TriggerType::UserMessage, ..Default::default() };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(always!(
                "Ask: who are you meeting, what is the context (interview/sales call/partnership), when is it"
            ));
            g.add_rule(always!(
                "Research person: search their name + company, LinkedIn profile, any published articles or talks"
            ));
            g.add_rule(always!("Research company: what they do, recent news, size, funding stage, key products"));
            g.add_rule(always!("Structure brief: (1) About them, (2) About the company, (3) Likely topics for this meeting, (4) Suggested questions to ask"));
            g.add_rule(always!("Keep it to one page — depth over breadth"));
            g.add_rule(always!("Save to workspace/prep/{name}-{date}.md"));
            g.add_failure(skip_log!(
                "No public information found for person — note in brief and proceed with company research"
            ));
            g.add_completion(CompletionCriterion::output_exists("workspace/prep/", "Prep brief produced"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &[],
    },
    // â”€â”€ 21. Call Center Triage â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    RoleTemplate {
        id: "call_center_triage",
        name: "Call Center Triage",
        description:
            "Handle inbound calls and texts, pull account context, and route urgent issues with a clean case note",
        persona: "teams",
        category: "customer_support",
        emoji: "📞",
        required_connectors: &["twilio", "gorgias", "zendesk", "salesforce"],
        intent: || {
            serde_json::json!({
                "category":           "customer_support",
                "trigger_hint":       "webhook",
                "trigger_confidence": "high",
                "trigger_source":     "twilio",
                "trigger_event":      "sms.received",
                "output_hint":        "connector_record",
                "output_destination_hint": "gorgias_ticket",
                "multi_role_suggested": true,
                "actions": [
                    "Capture the caller or text sender identity and classify urgency",
                    "Look up the customer in Salesforce and recent support history in Gorgias or Zendesk",
                    "Draft a short resolution note and route billing, cancellation, or escalation cases to a human queue",
                    "Send a concise follow-up SMS when appropriate",
                    "Log the final disposition and next step"
                ],
                "workflow_outline": [
                    "receive twilio inbound call or sms",
                    "look up customer context in salesforce and support tools",
                    "draft case note and escalation summary",
                    "attach note to gorgias or zendesk and log follow-up"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role =
                AgentRole::new(crate::util::new_id(), agent_id.into(), tenant_id.into(), "Call Center Triage".into());
            role.purpose = "Triage inbound calls and texts, then route or resolve with a clear case summary".into();
            role.connectors = vec!["twilio".into(), "gorgias".into(), "zendesk".into(), "salesforce".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("twilio".into()),
                event_filter: Some("sms.received".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(before!("twilio", "Confirm caller identity or phone number before sharing account details"));
            g.add_rule(always!("Pull customer context from Salesforce and the support inbox before replying"));
            g.add_rule(always!(
                "If the issue is billing, cancellation, legal, or high-frustration, route to a human queue"
            ));
            g.add_rule(always!("Keep replies short, calm, and action-oriented"));
            g.add_rule(after!("gorgias", "Attach a concise disposition note to the ticket or case"));
            g.add_rule(after!("salesforce", "Log call outcome, follow-up owner, and next step"));
            g.add_failure(escalate!("Customer requests a human or threatens churn", "#call-center-escalations"));
            g.add_failure(retry!("Twilio delivery or lookup failure", "twilio"));
            g.add_completion(CompletionCriterion::record_updated("gorgias", "Call summary attached"));
            g.add_completion(CompletionCriterion::record_updated("salesforce", "Case or contact updated"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["support_number", "escalation_channel", "default_queue"],
    },
    // â”€â”€ 22. Commerce / Dropshipping Ops â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    RoleTemplate {
        id: "commerce_fulfillment_ops",
        name: "Commerce Fulfillment Ops",
        description:
            "Manage Shopify orders, shipping exceptions, and customer updates for fast-moving ecommerce stores",
        persona: "teams",
        category: "sales_revops",
        emoji: "🛒",
        required_connectors: &["shopify", "shipstation", "gorgias", "stripe", "quickbooks"],
        intent: || {
            serde_json::json!({
                "category":           "sales_revops",
                "trigger_hint":       "webhook",
                "trigger_confidence": "high",
                "trigger_source":     "shopify",
                "trigger_event":      "orders/create",
                "output_hint":        "connector_record",
                "output_destination_hint": "shipstation_fulfillment",
                "multi_role_suggested": true,
                "actions": [
                    "Verify payment, shipping address, and fraud risk for each order",
                    "Check inventory and determine whether the order can ship immediately",
                    "Create shipping or fulfillment notes and handle exceptions cleanly",
                    "Draft customer updates for delays, refunds, or substitutions",
                    "Log financial and fulfillment outcomes for reconciliation"
                ],
                "workflow_outline": [
                    "ingest new shopify order",
                    "verify payment and shipping status",
                    "update shipstation fulfillment or exception note",
                    "notify gorgias and log the financial outcome"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Commerce Fulfillment Ops".into(),
            );
            role.purpose =
                "Manage ecommerce orders, shipping, and customer updates with strong exception handling".into();
            role.connectors =
                vec!["shopify".into(), "shipstation".into(), "gorgias".into(), "stripe".into(), "quickbooks".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("shopify".into()),
                event_filter: Some("orders/create".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(before!("shopify", "Verify address, payment status, and fraud flags before fulfillment"));
            g.add_rule(always!("If inventory is uncertain, move the order to an exception queue instead of guessing"));
            g.add_rule(always!("Use ShipStation for shipping and tracking handoff whenever possible"));
            g.add_rule(always!(
                "If a refund or replacement is needed, keep the customer informed with a clear timeline"
            ));
            g.add_rule(after!("shipstation", "Record shipping or exception outcome in the workspace log"));
            g.add_rule(after!("gorgias", "Attach the customer-facing status update to the support ticket"));
            g.add_failure(skip_log!("Order has a clear payment or address mismatch â€” flag for review", "shopify"));
            g.add_failure(retry!("Shipping provider API error", "shipstation"));
            g.add_completion(CompletionCriterion::record_updated("shopify", "Order reviewed"));
            g.add_completion(CompletionCriterion::record_updated("shipstation", "Shipment or exception updated"));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &["shop_domain", "shipping_origin", "escalation_channel"],
    },
    // ── 23. Brand Protection & Monitoring ────────────────────────────────────
    RoleTemplate {
        id: "brand_protection_monitoring",
        name: "Brand Protection & Monitoring",
        description:
            "Monitor your website, competitors, and social media for threats — escalate critical issues with evidence",
        persona: "teams",
        category: "brand_protection",
        emoji: "🛡️",
        required_connectors: &["brand_monitoring"],
        intent: || {
            serde_json::json!({
                "category":              "brand_protection",
                "trigger_hint":          "webhook",
                "trigger_confidence":    "high",
                "trigger_source":        "brand_monitoring",
                "trigger_event":         "alert",
                "output_hint":           "notification",
                "output_destination_hint": "slack_channel",
                "multi_role_suggested":  false,
                "uses_external_db":      null,
                "actions": [
                    "Monitor website for defacement, content changes, and uptime issues",
                    "Track competitor announcements and product launches",
                    "Monitor social media mentions and brand handle usage",
                    "Detect trademark violations and counterfeit activity",
                    "Escalate high-severity threats with evidence links and remediation options",
                    "Log all monitoring events for audit and trend analysis"
                ],
                "workflow_outline": [
                    "receive brand monitoring alert from external service",
                    "classify severity: low/medium/high/critical",
                    "collect evidence: screenshots, URLs, timestamps, source metadata",
                    "escalate critical issues to security/legal team with action items",
                    "log threat in workspace for trend analysis"
                ]
            })
        },
        build_role: |agent_id, tenant_id| {
            let mut role = AgentRole::new(
                crate::util::new_id(),
                agent_id.into(),
                tenant_id.into(),
                "Brand Protection & Monitoring".into(),
            );
            role.purpose = "Monitor brand threats across website, competitors, and social media — escalate critical issues with evidence".into();
            role.connectors = vec!["brand_monitoring".into()];
            role.trigger = TriggerDef {
                trigger_type: TriggerType::Webhook,
                source_connector: Some("brand_monitoring".into()),
                event_filter: Some("alert".into()),
                ..Default::default()
            };
            let mut g = ExecutionGuidelines::default();
            g.add_rule(before!(
                "brand_monitoring",
                "Verify alert authenticity — check timestamp and source reputation"
            ));
            g.add_rule(always!("Classify severity: low (typo/misspelling) / medium (unauthorized use on minor platform) / high (competitor misuse) / critical (counterfeiting, defacement, active fraud)"));
            g.add_rule(always!(
                "For all alerts except low: capture screenshots, URLs, timestamps, and IP/account metadata"
            ));
            g.add_rule(always!("Document evidence references for potential legal action"));
            g.add_rule(always!("For critical threats: immediately escalate with specific remediation options (DMCA takedown, account suspension, cease-and-desist)"));
            g.add_rule(always!("Never directly contact alleged infringers — only escalate to legal/security team"));
            g.add_rule(after!(
                "brand_monitoring",
                "Log the threat classification, evidence links, and remediation status to workspace/brand-threats.txt"
            ));
            g.add_failure(escalate!("Critical threat detected: counterfeiting or active fraud", "#security-incidents"));
            g.add_failure(escalate!("Website defacement or mass credential compromise", "#security-incidents"));
            g.add_failure(retry!("Brand monitoring service connectivity issue", "brand_monitoring"));
            g.add_completion(CompletionCriterion::record_updated("brand_monitoring", "Threat investigated"));
            g.add_completion(CompletionCriterion::errors_logged(
                "workspace/brand-threats.txt",
                "Threat logged and escalated",
            ));
            role.execution_guidelines = g;
            role
        },
        ask_steps: &[
            "bp_competitors",
            "bp_channels",
            "bp_approval_threshold",
            "bp_escalation_channel",
            "bp_response_mode",
        ],
    },
];
