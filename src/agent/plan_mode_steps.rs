//! Sequential clarification step pipeline for plan mode.
//!
//! After IntentExtractor runs, `generate_steps()` builds an ordered queue of
//! `ClarificationStep` items â€” one per piece of information the system still needs.
//! Each step knows:
//!   - The exact question to ask
//!   - Which field it writes to (`StepField`)
//!   - Whether it can be skipped (e.g. trigger already at High confidence)
//!
//! `handle_clarifications` in plan_mode.rs pops steps one at a time. Each turn:
//!   1. Pop the front step from session.pending_steps
//!   2. Show its question (if it was the previous turn's question, parse the answer)
//!   3. Write the parsed answer to the target field on draft_role / draft_agent
//!   4. Push the next step's question into the reply
//!
//! Domain-specific steps come from per-category generators at the bottom of this file.
//! The domain skill registry provides execution brief text; this file provides typed steps.

use serde::{Deserialize, Serialize};

use crate::agent::definition::{
    AgentRole, CompletionCriterion, FailureRule, OutputDestination, OutputFormat, TriggerConfidence, TriggerDef,
    TriggerType,
};

// â”€â”€ Step field target â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// What field of the draft role a step's answer writes to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "field_type", rename_all = "snake_case")]
pub enum StepField {
    /// Confirm / specify the trigger (schedule cron or webhook event).
    Trigger,
    /// For WorkforceEvent triggers â€” which role/event fires this one.
    WorkforceEventFilter,
    /// For WorkforceEvent triggers â€” what data to receive from the triggering run.
    WorkforceEventInputMapping,
    /// For within-agent strict ordering â€” which role must complete first.
    DependsOnRole,
    /// Specify where the output goes (workspace / channel / email / connector).
    OutputDestination,
    /// Specify the output format (markdown / json / html).
    OutputFormat,
    /// Role split decision â€” one role or split into multiple.
    RoleSplit,
    /// A rule to add to execution_guidelines.rules.
    GuidelineRule,
    /// A failure-handling rule to add to execution_guidelines.failure_handling.
    FailureHandling { tool_scope: Option<String> },
    /// A completion criterion to add to execution_guidelines.completion_criteria.
    CompletionCriteria,
    /// A hard constraint to add to agent.constraints.
    AgentConstraint,
    /// A free-form guideline the user can provide at the end.
    UserGuidelines,
    /// Where the workflow's source of truth lives, if any.
    SourceDiscovery,
}

/// One clarification step in the sequential pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationStep {
    /// Stable identifier, e.g. "trigger", "output_dest", "failure_slack".
    pub id: String,
    /// The question shown to the user verbatim.
    pub question: String,
    /// Which field this step's answer writes to.
    pub field: StepField,
    /// If true, the step can be skipped when the answer is already known.
    pub required: bool,
    /// Optional hint to the parser about what shape the answer takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ClarificationStep {
    pub fn new(id: impl Into<String>, question: impl Into<String>, field: StepField) -> Self {
        Self { id: id.into(), question: question.into(), field, required: true, hint: None }
    }
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

// â”€â”€ Queue generation â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Build the ordered step queue from the extracted intent.
/// `existing_roles` â€” names of roles already on this agent (empty for new agents).
pub fn generate_steps(
    intent: &serde_json::Value,
    category: &str,
    _installed: &[String],
    existing_roles: &[String],
) -> Vec<ClarificationStep> {
    let mut steps: Vec<ClarificationStep> = Vec::new();

    // â”€â”€ Step 1: Multi-role split â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if intent["multi_role_suggested"].as_bool().unwrap_or(false) {
        let names: Vec<&str> = intent["responsibilities"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|r| r["name"].as_str()).collect())
            .unwrap_or_default();
        let reason = intent["multi_role_reason"].as_str().unwrap_or("they have different triggers");
        steps.push(ClarificationStep::new(
            "role_split",
            format!(
                "I see {} distinct responsibilities â€” {}.\n\n\
                 **A) One role** â€” simpler, all in one\n\
                 **B) {} separate roles** (recommended) â€” easier to debug and monitor\n\n\
                 Which do you prefer? (A or B)",
                names.len(),
                reason,
                names.len()
            ),
            StepField::RoleSplit,
        ));
    }

    // â”€â”€ Step 2: Trigger â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let trigger_hint = intent["trigger_hint"].as_str().unwrap_or("manual");
    let trigger_confidence = intent["trigger_confidence"].as_str().unwrap_or("medium");
    let is_workforce = trigger_hint == "workforce_event" || trigger_hint == "after_role";

    if is_workforce {
        // 2a: which role fires this?
        let role_hint = if !existing_roles.is_empty() {
            format!(
                "**Which role triggers this one?** Existing roles on this agent: {}\n\n\
                 e.g. 'Lead Enrichment & Drafts' â€” or describe it.",
                existing_roles.join(", ")
            )
        } else {
            "**Which role or event triggers this?** \
             e.g. 'after the enrichment role completes' or 'when the previous role finishes'."
                .into()
        };
        steps.push(ClarificationStep::new("workforce_filter", role_hint, StepField::WorkforceEventFilter));

        // 2b: what data to receive?
        steps.push(ClarificationStep::new(
            "workforce_input_mapping",
            "**What data should this role receive from that run?** \
             e.g. 'the list of lead IDs enriched', 'the output file path', or 'the error count'.\n\
             Or say 'none' if it doesn't need data from the previous run.",
            StepField::WorkforceEventInputMapping,
        ));

        // 2c: strict ordering (optional, only if existing roles present)
        if !existing_roles.is_empty() {
            steps.push(
                ClarificationStep::new(
                    "depends_on_role",
                    format!(
                        "**Should this role also enforce strict ordering** â€” i.e. block until a \
                         specific role finishes before it can start?\n\
                         Current roles: {}\n\
                         Or say 'no' â€” the workforce trigger above is enough.",
                        existing_roles.join(", ")
                    ),
                    StepField::DependsOnRole,
                )
                .optional(),
            );
        }
    } else if trigger_confidence != "high" {
        let q =
            intent["trigger_confirmation"].as_str().map(String::from).unwrap_or_else(|| build_trigger_question(intent));
        steps.push(ClarificationStep::new("trigger", q, StepField::Trigger));
    }

    // â”€â”€ Step 3: Output destination â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let dest_hint = intent["output_destination_hint"].as_str().unwrap_or("");
    if dest_hint.is_empty() {
        let hint = intent["output_hint"].as_str().unwrap_or("workspace");
        let q: String = match hint {
            "email_draft" | "email_send" =>
                "Where should the emails go â€” **drafts saved to workspace** for review, or **sent directly** via Gmail/Outlook?".into(),
            "connector_record" =>
                "Which record should I update, and which field? e.g. 'Salesforce Lead Description'".into(),
            "slack_message" =>
                "Which Slack channel should I post to? e.g. '#sales-ops'".into(),
            "report" =>
                "Where should the report go? e.g. 'workspace/reports/' or 'email to manager@co.com'".into(),
            "notification" =>
                "Where should notifications go â€” Slack channel, email, or both?".into(),
            _ =>
                "Where should the output go, and in what format? e.g. 'workspace/output.md' or '#slack-channel'".into(),
        };
        steps.push(ClarificationStep::new("output_dest", q, StepField::OutputDestination).with_hint(hint));
    }

    // â”€â”€ Steps 4+: Domain steps â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    steps.extend(domain_steps_for(category));

    // â”€â”€ Final: Completion criteria â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    steps.push(ClarificationStep::new(
        "completion",
        "What does 'done' look like for one run? e.g. 'all leads enriched, drafts saved, errors logged'. \
         Or say 'auto' for smart defaults.",
        StepField::CompletionCriteria,
    ));

    steps
}

fn build_trigger_question(intent: &serde_json::Value) -> String {
    let hint = intent["trigger_hint"].as_str().unwrap_or("manual");
    let cron = intent["trigger_cron"].as_str();
    match (hint, cron) {
        ("schedule", Some(c)) => format!(
            "I guessed: `{}` â€” is that right? Or describe it more precisely \
             (e.g. 'Every weekday at 8am New York time', 'First Monday of each month at 9am').",
            c
        ),
        ("schedule", None) => "When exactly should this run? e.g. 'Every Monday at 9am', 'Daily at midnight UTC', \
             'Every hour between 9amâ€“6pm'."
            .into(),
        ("webhook", _) => {
            let src = intent["trigger_source"].as_str().unwrap_or("the connector");
            let evt = intent["trigger_event"].as_str().unwrap_or("an event");
            format!(
                "Trigger when **{}** fires `{}`? Or describe it differently \
                 (e.g. 'when a new HubSpot contact is created', 'when a Zendesk ticket is opened').",
                src, evt
            )
        }
        _ => "When should this run?\n\
              - **Schedule**: 'Every Monday at 9am'\n\
              - **Webhook**: 'When a new Salesforce lead is created'\n\
              - **On-demand**: 'When I ask'\n\
              - **After another role**: 'After the enrichment role finishes'"
            .into(),
    }
}

// â”€â”€ Answer parsing â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Parse the user's answer for a given step and write the result to the role.
/// Returns a summary of what was written (shown to the user as confirmation).
pub fn parse_and_apply(
    step: &ClarificationStep,
    answer: &str,
    role: &mut AgentRole,
    agent_constraints: &mut Vec<String>,
    intent: &serde_json::Value,
    pending_roles_sink: &mut Option<Vec<serde_json::Value>>,
) -> String {
    let lower = answer.to_lowercase();

    match &step.field {
        StepField::Trigger => {
            let (trigger, conf) = parse_trigger_answer(answer, intent);
            let summary = format!("Trigger set: {}", trigger_summary(&trigger));
            role.trigger = trigger;
            role.trigger.confidence = conf;
            summary
        }

        StepField::WorkforceEventFilter => {
            // User named the triggering role â€” set workforce_event_filter
            let lower = answer.to_lowercase();
            let trimmed = answer.trim();
            if !lower.contains("no") && !trimmed.is_empty() {
                // Build a filter expression from the role name
                let filter = if lower.contains("any") || lower.contains("all") {
                    // Any role completion triggers this
                    "status == 'completed'".to_string()
                } else {
                    // Named role
                    let role_name = trimmed.trim_matches('"').trim_matches('\'');
                    format!("role_name == '{}' AND status == 'completed'", role_name)
                };
                role.trigger.trigger_type = crate::agent::definition::TriggerType::WorkforceEvent;
                role.trigger.workforce_event_filter = Some(filter.clone());
                format!("Trigger: runs after {}", trimmed)
            } else {
                "No specific trigger role set â€” fires on any completion.".into()
            }
        }

        StepField::WorkforceEventInputMapping => {
            let lower = answer.to_lowercase();
            if lower.contains("none") || lower.contains("no data") || lower.contains("nothing") {
                role.trigger.input_mapping = None;
                "No input mapping â€” this role starts fresh each time.".into()
            } else {
                // Parse natural language into JSONPath-style mapping
                // e.g. "the list of lead IDs enriched" â†’ { "lead_ids": "$.output_data.lead_ids" }
                let mapping = infer_input_mapping(answer);
                let summary = format!("Will receive: {}", answer.trim());
                role.trigger.input_mapping = Some(mapping);
                summary
            }
        }

        StepField::DependsOnRole => {
            let lower = answer.to_lowercase();
            if lower.contains("no") || lower.trim().len() < 3 {
                "No strict ordering set.".into()
            } else {
                // User named a role â€” store it as depends_on_role_id hint
                // (actual ID lookup happens at save time)
                let role_name = answer.trim();
                role.trigger.depends_on_role_id = Some(format!("name:{}", role_name));
                format!("Will wait for '{}' to complete first.", role_name)
            }
        }

        StepField::OutputDestination => {
            let (dest, desc) = parse_output_destination(answer, intent);
            role.output_spec.destination = dest;
            if !desc.is_empty() {
                role.output_spec.description = desc.clone();
            }
            format!("Output: {}", desc)
        }

        StepField::OutputFormat => {
            role.output_spec.format = if lower.contains("json") {
                OutputFormat::Json
            } else if lower.contains("html") {
                OutputFormat::Html
            } else {
                OutputFormat::Markdown
            };
            format!("Format: {:?}", role.output_spec.format)
        }

        StepField::RoleSplit => {
            let wants_split = lower.contains('b') && !lower.contains("best")
                || lower.contains("split")
                || lower.contains("separate")
                || lower.contains("two roles")
                || lower.contains("multiple");

            if wants_split {
                let responsibilities = intent["responsibilities"].as_array().cloned().unwrap_or_default();
                if responsibilities.len() > 1 {
                    let mut remaining = responsibilities.clone();
                    remaining.remove(0); // first is being configured now
                    *pending_roles_sink = Some(remaining.clone());
                    // Update role name to first responsibility
                    if let Some(name) = responsibilities[0]["name"].as_str() {
                        role.name = name.to_string();
                    }
                    return format!(
                        "I'll configure {} roles. Starting with: **{}**.",
                        responsibilities.len(),
                        responsibilities[0]["name"].as_str().unwrap_or("Role 1")
                    );
                }
            }
            "Keeping as one role.".into()
        }

        StepField::GuidelineRule => {
            let g = crate::agent::definition::ExecutionGuidelines::from_user_constraints(answer);
            let count = g.rules.len() + g.failure_handling.len();
            role.execution_guidelines.extend_dedup(g);
            format!("Added {} guideline item(s).", count)
        }

        StepField::FailureHandling { tool_scope } => {
            let action = crate::agent::definition::infer_failure_action(&lower);
            let rule = FailureRule { text: answer.trim().to_string(), tool_scope: tool_scope.clone(), action };
            role.execution_guidelines.add_failure(rule);
            "Failure handling saved.".into()
        }

        StepField::CompletionCriteria => {
            if lower.contains("auto") || lower.contains("default") || lower.contains("smart") {
                // Generate defaults from output spec + connectors
                let defaults = default_completion_criteria(role);
                let count = defaults.len();
                for c in defaults {
                    role.execution_guidelines.add_completion(c);
                }
                format!("Using {} default completion criteria.", count)
            } else {
                let g = crate::agent::definition::ExecutionGuidelines::from_user_constraints(answer);
                let count = g.completion_criteria.len();
                role.execution_guidelines.extend_dedup(g);
                if count == 0 {
                    // User gave free text that didn't parse as completion criteria â€” treat as custom
                    role.execution_guidelines.add_completion(CompletionCriterion::custom(answer.trim()));
                    "Completion criterion saved.".into()
                } else {
                    format!("Saved {} completion criterion/criteria.", count)
                }
            }
        }

        StepField::AgentConstraint => {
            let items: Vec<String> = answer
                .split(&[',', ';', '\n'][..])
                .map(|s| s.trim().trim_end_matches('.').to_string())
                .filter(|s| s.len() > 5)
                .collect();
            let count = items.len();
            agent_constraints.extend(items);
            format!("Added {} constraint(s).", count)
        }

        StepField::UserGuidelines => {
            let g = crate::agent::definition::ExecutionGuidelines::from_user_constraints(answer);
            let count = g.rules.len() + g.failure_handling.len() + g.completion_criteria.len();
            role.execution_guidelines.extend_dedup(g);
            format!("Added {} guideline item(s).", count)
        }

        StepField::SourceDiscovery => {
            let trimmed = answer.trim();
            let lower = trimmed.to_lowercase();
            if trimmed.is_empty()
                || lower == "none"
                || lower.contains("general knowledge")
                || lower.contains("use defaults")
            {
                "No source of truth provided - continuing with defaults.".into()
            } else {
                role.execution_guidelines.add_rule(crate::agent::definition::GuidelineRule::always(format!(
                    "Source of truth for this workflow: {}",
                    trimmed
                )));
                format!("Source of truth noted: {}", trimmed)
            }
        }
    }
}

// â”€â”€ Trigger parsing â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub fn parse_trigger_answer(answer: &str, intent: &serde_json::Value) -> (TriggerDef, TriggerConfidence) {
    use crate::agent::plan_mode::{intent_to_trigger, parse_trigger_from_text};

    // If user confirmed the auto-parsed trigger (yes/correct/right/looks good)
    let lower = answer.to_lowercase();
    let is_confirmation = lower == "yes"
        || lower == "correct"
        || lower == "right"
        || lower.contains("looks good")
        || lower.contains("that's right")
        || lower.contains("that is right");

    if is_confirmation {
        // Accept the intent's parsed trigger at high confidence
        let (mut trigger, _) = intent_to_trigger(intent);
        trigger.confidence = TriggerConfidence::High;
        return (trigger, TriggerConfidence::High);
    }

    let mut trigger = parse_trigger_from_text(answer);
    let confidence =
        if trigger_answer_is_specific(answer) { TriggerConfidence::High } else { TriggerConfidence::Medium };
    trigger.confidence = confidence.clone();
    (trigger, confidence)
}

fn trigger_answer_is_specific(answer: &str) -> bool {
    let lower = answer.to_lowercase();
    let has_day = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "daily",
        "weekday",
        "weekend",
        "hourly",
        "monthly",
    ]
    .iter()
    .any(|d| lower.contains(d));
    let has_time = lower.contains("am")
        || lower.contains("pm")
        || lower.contains(":00")
        || lower.contains("midnight")
        || lower.contains("noon");
    let has_cron = lower.contains("cron") || (lower.contains("*") && lower.contains(" "));
    let has_event = lower.contains("when")
        && (lower.contains("created")
            || lower.contains("opened")
            || lower.contains("updated")
            || lower.contains("fired"));
    (has_day && has_time) || has_cron || has_event
}

fn trigger_summary(t: &TriggerDef) -> String {
    match &t.trigger_type {
        TriggerType::Schedule => format!("Schedule {}", t.cron.as_deref().unwrap_or("(TBD)")),
        TriggerType::Webhook => format!(
            "Webhook from {} / {}",
            t.source_connector.as_deref().unwrap_or("connector"),
            t.event_filter.as_deref().unwrap_or("event")
        ),
        TriggerType::Manual => "On-demand".into(),
        TriggerType::UserMessage => "On user message".into(),
        TriggerType::WorkforceEvent => "After another role".into(),
    }
}

// â”€â”€ Output destination parsing â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub fn parse_output_destination(answer: &str, intent: &serde_json::Value) -> (OutputDestination, String) {
    let lower = answer.to_lowercase();
    let dest_hint = intent["output_destination_hint"].as_str().unwrap_or("");
    let hint_lower = dest_hint.to_lowercase();

    // Slack / channel
    if lower.contains("slack")
        || lower.contains("channel")
        || lower.starts_with('#')
        || hint_lower.contains("slack")
        || hint_lower.starts_with('#')
    {
        let channel =
            extract_channel(&answer).or_else(|| extract_channel(dest_hint)).unwrap_or_else(|| "#general".to_string());
        let connector = if lower.contains("teams") || hint_lower.contains("teams") { "outlook" } else { "slack" };
        let desc = format!("{} â†’ {}", connector, channel);
        return (OutputDestination::Channel { connector: connector.into(), channel }, desc);
    }

    // Email
    if (lower.contains("email") || hint_lower.contains("email"))
        && (lower.contains("send")
            || lower.contains("draft")
            || lower.contains("gmail")
            || lower.contains("outlook")
            || hint_lower.contains("draft"))
    {
        let connector = if lower.contains("outlook") || hint_lower.contains("outlook") { "outlook" } else { "gmail" };
        let draft = !(lower.contains("send directly") || lower.contains("auto-send") || lower.contains("send it now"));
        let desc = format!("{} {} email", connector, if draft { "draft" } else { "send" });
        return (OutputDestination::Email { connector: connector.into(), draft }, desc);
    }

    // Connector record update
    if lower.contains("salesforce")
        || lower.contains("hubspot")
        || lower.contains("crm record")
        || hint_lower.contains("salesforce")
        || hint_lower.contains("hubspot")
    {
        let connector =
            if lower.contains("hubspot") || hint_lower.contains("hubspot") { "hubspot" } else { "salesforce" };
        let field = if lower.contains("description") || hint_lower.contains("description") {
            "Description"
        } else if lower.contains("note") {
            "Notes__c"
        } else {
            "Description"
        };
        let desc = format!("{} â†’ {} field", connector, field);
        return (
            OutputDestination::Connector {
                name: connector.into(),
                record_id_field: "id".into(),
                target_field: field.into(),
            },
            desc,
        );
    }

    // Workspace â€” extract path hint
    let path = extract_workspace_path(&lower).or_else(|| extract_workspace_path(&hint_lower));
    let desc = path.as_deref().unwrap_or("workspace").to_string();
    (OutputDestination::Workspace { path }, desc)
}

/// Infer a JSONPath input_mapping from a natural language description.
/// e.g. "the list of lead IDs" â†’ { "lead_ids": "$.output_data.lead_ids" }
/// e.g. "the output file path" â†’ { "output_path": "$.output_data.output_path" }
fn infer_input_mapping(answer: &str) -> serde_json::Value {
    let lower = answer.to_lowercase();
    let mut mapping = serde_json::Map::new();

    // Common field patterns
    let patterns: &[(&str, &str, &str)] = &[
        ("lead id", "lead_ids", "$.output_data.lead_ids"),
        ("lead", "lead_ids", "$.output_data.lead_ids"),
        ("record id", "record_ids", "$.output_data.record_ids"),
        ("record", "record_ids", "$.output_data.record_ids"),
        ("file path", "output_path", "$.output_data.output_path"),
        ("file", "output_path", "$.output_data.output_path"),
        ("output", "output", "$.output_data"),
        ("count", "count", "$.output_data.processed"),
        ("error", "errors", "$.output_data.errors"),
        ("ticket", "ticket_ids", "$.output_data.ticket_ids"),
        ("contact", "contact_ids", "$.output_data.contact_ids"),
        ("account", "account_ids", "$.output_data.account_ids"),
        ("order", "order_ids", "$.output_data.order_ids"),
        ("result", "result", "$.output_data"),
    ];

    let mut matched = false;
    for (keyword, field_name, path) in patterns {
        if lower.contains(keyword) {
            mapping.insert(field_name.to_string(), serde_json::json!(path));
            matched = true;
            break;
        }
    }

    if !matched {
        // Generic fallback â€” pass the whole output_data
        mapping.insert("data".to_string(), serde_json::json!("$.output_data"));
    }

    serde_json::Value::Object(mapping)
}

fn extract_channel(text: &str) -> Option<String> {
    if let Some(pos) = text.find('#') {
        let after = &text[pos..];
        let end = after.find(|c: char| c.is_whitespace() || c == '\'' || c == '"').unwrap_or(after.len());
        if end > 1 {
            return Some(after[..end].to_lowercase());
        }
    }
    // "the sales-ops channel" pattern
    if let Some(pos) = text.to_lowercase().find(" channel") {
        let before = &text[..pos];
        let words: Vec<&str> = before.split_whitespace().collect();
        if let Some(last) = words.last() {
            return Some(format!("#{}", last.to_lowercase()));
        }
    }
    None
}

fn extract_workspace_path(lower: &str) -> Option<String> {
    for word in &["drafts/", "output/", "reports/", "results/", "emails/", "workspace/"] {
        if lower.contains(*word) {
            return Some(word.to_string());
        }
    }
    // "save to drafts" â†’ "drafts/"
    for word in &["drafts", "output", "reports", "results"] {
        if lower.contains(word) {
            return Some(format!("{}/", word));
        }
    }
    None
}

// â”€â”€ Default completion criteria â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub fn default_completion_criteria(role: &AgentRole) -> Vec<CompletionCriterion> {
    let mut criteria = Vec::new();

    // Based on connectors used
    for connector in &role.connectors {
        match connector.as_str() {
            "salesforce" | "hubspot" => criteria.push(CompletionCriterion::all_items(
                "All queried records processed (skip invalid, don't abort)",
                format!("{} query results", connector),
            )),
            "zendesk" | "intercom" | "freshdesk" => {
                criteria.push(CompletionCriterion::custom("All triggered tickets responded to or escalated"))
            }
            _ => {}
        }
    }

    // Based on output destination
    match &role.output_spec.destination {
        OutputDestination::Workspace { path } => {
            let p = path.as_deref().unwrap_or("workspace/");
            criteria.push(CompletionCriterion::output_exists(format!("Output files written to {}", p), p));
        }
        OutputDestination::Email { draft, connector } => {
            let label = if *draft { "draft saved" } else { "email sent" };
            criteria.push(CompletionCriterion::custom(format!("Email {} via {}", label, connector)));
        }
        OutputDestination::Channel { connector, channel } => {
            criteria.push(CompletionCriterion::custom(format!("Message posted to {} via {}", channel, connector)))
        }
        OutputDestination::Connector { name, .. } => {
            criteria.push(CompletionCriterion::record_updated(format!("{} record updated", name), name))
        }
        _ => {}
    }

    // Always add error log criterion
    criteria.push(CompletionCriterion::errors_logged(
        "workspace/errors.txt written (even if empty â€” proves the run completed)",
        "workspace/errors.txt",
    ));

    criteria
}

// â”€â”€ Domain step generators â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Returns domain-specific clarification steps for a category.
/// These encode the "mandatory questions" from the Superpowers-style domain skills
/// as typed steps rather than free text.
pub fn domain_steps_for(category: &str) -> Vec<ClarificationStep> {
    let source_step = source_discovery_step_for(category);
    match category {
        "customer_support" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "cs_response_mode",
                "**Response mode:** Should I draft replies for human approval, or send automatically?",
                StepField::GuidelineRule,
            ),
            ClarificationStep::new(
                "cs_sla",
                "**First-response SLA:** How fast must the first reply go out? \
                 (e.g. '15 min', '1 hour', '4 hours', 'best-effort')",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "cs_escalation",
                "**Escalation rule:** Which ticket types should always go to a human? \
                 (e.g. 'billing disputes, legal threats, VIP accounts'). Or 'none'.",
                StepField::FailureHandling { tool_scope: None },
            ),
        ],

        "sales_revops" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "sr_write_back",
                "**Write-back:** Should I update records automatically, or create tasks/notes only?",
                StepField::GuidelineRule,
            ),
            ClarificationStep::new(
                "sr_enrichment_sources",
                "**Enrichment sources:** Web search + LinkedIn, CRM data only, or specific data APIs?",
                StepField::GuidelineRule,
            ),
            ClarificationStep::new(
                "sr_outreach_mode",
                "**Outreach emails:** Save drafts to workspace for review, add to a sequence, or send directly?",
                StepField::GuidelineRule,
            ),
            ClarificationStep::new(
                "sr_skip_criteria",
                "**Skip criteria:** Which records should I skip? \
                 (e.g. 'missing email', 'already in active sequence', 'revenue < $10k'). Or 'none'.",
                StepField::FailureHandling { tool_scope: None },
            ),
        ],

        "finance_accounting" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "fa_write_access",
                "**Write access:** Read-only reporting, or can I create/update financial records?",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "fa_approval_threshold",
                "**Approval gate:** Any transaction above what amount needs human approval before posting? \
                 e.g. '$10,000' or 'none'.",
                StepField::FailureHandling { tool_scope: None },
            ),
            ClarificationStep::new(
                "fa_mismatch",
                "**On mismatch:** If a record doesn't reconcile, flag for review or block the entire run?",
                StepField::FailureHandling { tool_scope: None },
            ),
        ],

        "devops" | "it_ops_itsm" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "do_environment",
                "**Environment:** Prod, staging, or dev? (I will never default to prod.)",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "do_blast_radius",
                "**Scope:** Can I modify infrastructure, or read-only?",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "do_alert_channel",
                "**Alerts:** Which Slack channel or PagerDuty service for failure notifications? \
                 e.g. '#ops-alerts' or 'production-incidents'.",
                StepField::FailureHandling { tool_scope: None },
            ),
            ClarificationStep::new(
                "do_rollback",
                "**On failure:** Stop and alert only, or attempt automatic rollback?",
                StepField::FailureHandling { tool_scope: None },
            ),
        ],

        "hr_people_ops" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "hr_visibility",
                "**Visibility:** Who can see this agent's output? (HR only / managers / candidates / all)",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "hr_write_back",
                "**Write-back:** Update ATS records automatically, or report only?",
                StepField::GuidelineRule,
            ),
            ClarificationStep::new(
                "hr_communication",
                "**Candidate comms:** Draft offers/rejections in workspace for review, or send directly?",
                StepField::GuidelineRule,
            ),
        ],

        "legal_contract" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "lc_action_scope",
                "**Scope:** Flag issues only, or also redline and suggest edits?",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "lc_escalation_clauses",
                "**Escalation clauses:** Which clause types must always go to legal counsel? \
                 e.g. 'indemnity cap, IP assignment, non-compete'. Or 'none'.",
                StepField::FailureHandling { tool_scope: None },
            ),
            ClarificationStep::new(
                "lc_output_format",
                "**Output format:** Annotated PDF, tracked-changes summary, or plain report?",
                StepField::OutputFormat,
            ),
        ],

        "research_analyst" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "ra_depth",
                "**Evidence depth:** Quick summary of the provided material or a deeper review with supporting citations?",
                StepField::GuidelineRule,
            ),
            ClarificationStep::new(
                "ra_freshness",
                "**Evidence freshness:** How recent must the source material be? \
                 (last 7 days / 30 days / 6 months / any)",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "ra_on_no_results",
                "**If fewer than 3 references are available:** Stop and ask, broaden the scope, or proceed anyway?",
                StepField::FailureHandling { tool_scope: None },
            ),
        ],

        "brand_protection" => vec![
            source_step.clone(),
            ClarificationStep::new(
                "bp_competitors",
                "**Competitors to monitor:** Which competitor domains or handles should I track? \
                 e.g. 'competitor.com, @competitor_handle'. Or 'none - my brand only'.",
                StepField::GuidelineRule,
            ),
            ClarificationStep::new(
                "bp_channels",
                "**Channels to monitor:** Social media handles to watch? \
                 e.g. 'Twitter, LinkedIn, Instagram' or 'all'.",
                StepField::AgentConstraint,
            ),
            ClarificationStep::new(
                "bp_approval_threshold",
                "**Approval gate:** What severity level requires human review before action? \
                 e.g. 'high only', 'medium and above', or 'all alerts'.",
                StepField::FailureHandling { tool_scope: None },
            ),
            ClarificationStep::new(
                "bp_escalation_channel",
                "**Escalation channel:** Where should critical brand incidents go? \
                 e.g. '#brand-alerts', 'security@company.com', or a PagerDuty service.",
                StepField::FailureHandling { tool_scope: None },
            ),
            ClarificationStep::new(
                "bp_response_mode",
                "**Response mode:** Alerts only, auto-publish takedown requests, or draft actions for review?",
                StepField::GuidelineRule,
            ),
        ],

        _ => vec![
            source_step,
            // Generic fallback: one open constraints question
            ClarificationStep::new(
                "generic_constraints",
                "Any hard rules this agent must follow? \
                 e.g. 'Never send without approval', 'Read-only', 'Skip missing records'. \
                 Or say 'none'.",
                StepField::UserGuidelines,
            )
            .optional(),
        ],
    }
}

fn source_discovery_step_for(category: &str) -> ClarificationStep {
    let question = match category {
        "customer_support" => {
            "**Source of truth:** Where are the help docs, FAQ, policy pages, or KB articles I should use? \
             Share a URL, Notion page, doc folder, or say 'none' to continue from ticket context only."
        }
        "finance_accounting" => {
            "**Source of truth:** Where are the invoices, ledger, statements, or accounting records? \
             Share the system name, database, folder, or say 'none' if you only want a high-level draft."
        }
        "legal_contract" => {
            "**Source of truth:** Where are the contract files or policy documents stored? \
             Share Drive, DocuSign, Notion, a folder path, or say 'none' if you want a generic draft only."
        }
        "hr_people_ops" => {
            "**Source of truth:** Where are the people policies, handbook, ATS, or HR docs stored? \
             Share the system, URL, or say 'none' if you only want general guidance."
        }
        "sales_revops" => {
            "**Source of truth:** Where should I look for the account, CRM, product, or enrichment data? \
             Share the CRM, database, or docs location, or say 'none' to use connected systems only."
        }
        "devops" | "it_ops_itsm" => {
            "**Source of truth:** Where are the runbooks, incident notes, CMDB, or service docs? \
             Share a wiki, repo, dashboard, or say 'none' if you want me to rely on live systems only."
        }
        "research_analyst" => {
            "**Source of truth:** Which internal docs, approved sources, or reference list should I use first? \
             Share URLs or docs, or say 'none' if I should start from public sources."
        }
        "brand_protection" => {
            "**Source of truth:** Where are the brand guidelines, approved assets, or monitoring references? \
             Share a doc, site, or folder, or say 'none' if you want me to proceed with general monitoring rules."
        }
        _ => {
            "**Source of truth:** Where should I look first for the authoritative information for this workflow? \
             Share a URL, docs, database, folder, or say 'none' to continue with defaults."
        }
    };

    ClarificationStep::new("source_discovery", question, StepField::SourceDiscovery)
}
