//! Core data model for the multi-role agent system.
//!
//! ## Hierarchy
//!
//!   AgentDefinition   — the employee: identity, allowed connectors, constraints
//!       └── AgentRole — a role the employee plays: trigger, purpose, guidelines,
//!                       scoped connectors, output spec, execution limits
//!           └── GoalInstance (in goal_instance.rs) — one run of a role
//!
//! ## Design decisions
//!
//! - Connectors are declared at TWO levels:
//!     AgentDefinition.connectors  = allowed universe (security boundary)
//!     AgentRole.connectors        = relevant subset for this role (planner scope)
//!   A role cannot use a connector that isn't in the agent's allowed list.
//!   Validated on save, enforced at execution time.
//!
//! - Roles are versioned. Running GoalInstances snapshot the role_version they
//!   started with so a mid-flight edit never corrupts an in-progress run.
//!
//! - WorkforceEvent triggers enable cross-agent chaining via a pub/sub bus.
//!   No central orchestrator needed — each role declares what event it listens to.
//!
//! - RoleStatus::Testing lets you validate a role against sandbox data before
//!   going live. Testing-mode GoalInstances are flagged and never write to
//!   real external systems.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── AgentDefinition ────────────────────────────────────────────────────────

/// The agent's persistent identity — the "employee record".
/// Created once during plan mode, referenced by every role and goal instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub id: String,
    pub tenant_id: String,

    /// Human-readable name, e.g. "Sales Ops Agent".
    pub name: String,

    /// System-prompt persona injected at the start of every execution.
    /// Written in second person: "You are a senior RevOps analyst..."
    pub persona: String,

    /// Allowed connector universe for this agent.
    /// Acts as a security boundary — roles can only use a subset of these.
    /// e.g. ["salesforce", "slack", "web_search"]
    pub connectors: Vec<String>,

    /// Hard constraints that apply to every role.
    /// e.g. ["never send emails without approval", "never delete CRM records"]
    pub constraints: Vec<String>,

    /// Persistent memory key prefix for this agent.
    /// Agent-scoped memory is shared across all roles.
    pub memory_ref: String,

    pub status: AgentDefinitionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDefinitionStatus {
    /// Being configured in plan mode — not yet deployable.
    Draft,
    /// Fully configured and accepting goal instances.
    Active,
    /// Temporarily disabled — no new goal instances created.
    Paused,
    /// Archived — hidden from UI, no new instances.
    Archived,
}

impl AgentDefinition {
    pub fn new(id: String, tenant_id: String, name: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            tenant_id,
            name,
            persona: String::new(),
            connectors: Vec::new(),
            constraints: Vec::new(),
            memory_ref: String::new(),
            status: AgentDefinitionStatus::Draft,
            created_at: now,
            updated_at: now,
        }
    }

    /// Validate that all role connectors are a subset of the agent's allowed list.
    /// Returns the names of any connectors the role references that aren't allowed.
    pub fn validate_role_connectors(&self, role_connectors: &[String]) -> Vec<String> {
        role_connectors
            .iter()
            .filter(|c| !self.connectors.contains(c))
            .cloned()
            .collect()
    }
}

// ── AgentRole ──────────────────────────────────────────────────────────────

/// One role an agent plays — a reusable template that generates GoalInstances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRole {
    pub id: String,
    pub agent_id: String,
    pub tenant_id: String,

    /// Monotonically increasing version number.
    /// Incremented on every save. GoalInstances snapshot this at creation time.
    pub version: u32,

    pub status: RoleStatus,

    /// Human-readable name, e.g. "Lead Enrichment".
    pub name: String,

    /// How this role gets triggered.
    pub trigger: TriggerDef,

    /// Business-language description of what this role does.
    /// e.g. "Enrich inbound Salesforce leads and draft personalised outreach"
    pub purpose: String,

    /// Structured guidelines for the planner — happy path AND failure handling.
    /// The planner injects these verbatim into its system prompt.
    ///
    /// Example:
    ///   - Always attempt company inference if company name is missing
    ///   - Perform at least 2 independent web searches per lead
    ///   - If Salesforce update fails, save result to workspace and notify Slack
    ///   - Skip leads with no valid email address
    /// Structured execution guidelines — rules, failure handling, and priorities.
    /// Stored as JSON in the DB and injected verbatim into the planner prompt.
    #[serde(default)]
    pub execution_guidelines: ExecutionGuidelines,

    /// Connectors this role uses — must be a subset of AgentDefinition.connectors.
    /// Only these connectors are shown to the planner and executor.
    pub connectors: Vec<String>,

    /// Specific non-connector tools this role uses.
    /// If empty, the selector uses category-based defaults for the job type.
    pub tools: Vec<String>,

    /// Structured output specification.
    pub output_spec: OutputSpec,

    /// Memory isolation scope for this role.
    pub memory_scope: MemoryScope,

    /// Execution safety limits.
    pub execution_limits: ExecutionLimits,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleStatus {
    /// Being configured — not yet accepting goal instances.
    Draft,
    /// Runs against sandbox/synthetic data. Instances flagged, no real writes.
    Testing,
    /// Live — accepts real goal instances from triggers.
    Active,
    /// Trigger disabled — existing instances continue, no new ones created.
    Paused,
    /// Soft-deleted — hidden, no new instances.
    Archived,
}

impl AgentRole {
    pub fn new(id: String, agent_id: String, tenant_id: String, name: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            agent_id,
            tenant_id,
            version: 1,
            status: RoleStatus::Draft,
            name,
            trigger: TriggerDef::default(),
            purpose: String::new(),
            execution_guidelines: ExecutionGuidelines::default(),
            connectors: Vec::new(),
            tools: Vec::new(),
            output_spec: OutputSpec::default(),
            memory_scope: MemoryScope::Agent,
            execution_limits: ExecutionLimits::default(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Bump version and updated_at on every save.
    pub fn bump_version(&mut self) {
        self.version += 1;
        self.updated_at = Utc::now();
    }

    pub fn is_live(&self) -> bool {
        self.status == RoleStatus::Active
    }
}

// ── TriggerDef ─────────────────────────────────────────────────────────────

// ── Execution guideline types ──────────────────────────────────────────────

/// When a rule applies relative to a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RulePhase { Before, After, #[default] Always }

/// A single behavioural rule — verb-led, tool-scoped where possible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuidelineRule {
    pub text:       String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_scope: Option<String>,
    #[serde(default)]
    pub phase:      RulePhase,
}
impl GuidelineRule {
    pub fn always(text: impl Into<String>) -> Self {
        Self { text: text.into(), tool_scope: None, phase: RulePhase::Always }
    }
    pub fn before(tool: impl Into<String>, text: impl Into<String>) -> Self {
        Self { text: text.into(), tool_scope: Some(tool.into()), phase: RulePhase::Before }
    }
    pub fn after(tool: impl Into<String>, text: impl Into<String>) -> Self {
        Self { text: text.into(), tool_scope: Some(tool.into()), phase: RulePhase::After }
    }
}

/// What to do when a specific failure occurs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FailureAction {
    SkipAndLog          { log_path: String },
    SkipSilently,
    RetryOnce,
    EscalateToHuman     { notify_channel: Option<String> },
    Abort,
}

/// A failure-handling rule, optionally tool-scoped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureRule {
    pub text:       String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_scope: Option<String>,
    pub action:     FailureAction,
}
impl FailureRule {
    pub fn skip_and_log(tool: Option<&str>, text: impl Into<String>, log_path: impl Into<String>) -> Self {
        Self { text: text.into(), tool_scope: tool.map(String::from),
               action: FailureAction::SkipAndLog { log_path: log_path.into() } }
    }
    pub fn retry_once(tool: Option<&str>, text: impl Into<String>) -> Self {
        Self { text: text.into(), tool_scope: tool.map(String::from), action: FailureAction::RetryOnce }
    }
    pub fn escalate(channel: Option<&str>, text: impl Into<String>) -> Self {
        Self { text: text.into(), tool_scope: None,
               action: FailureAction::EscalateToHuman { notify_channel: channel.map(String::from) } }
    }
}

/// What type of assertion is checked at completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionCheck {
    AllItemsProcessed { collection_hint: String },
    OutputExists      { path_hint: String },
    RecordUpdated     { connector: String },
    CountMatches      { source: String, target: String },
    ErrorsLogged      { log_hint: String },
    Custom            { assertion: String },
}

/// One criterion the evaluator checks to declare a role run complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionCriterion {
    pub description: String,
    pub check:       CompletionCheck,
}
impl CompletionCriterion {
    pub fn all_items(desc: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { description: desc.into(), check: CompletionCheck::AllItemsProcessed { collection_hint: hint.into() } }
    }
    pub fn output_exists(desc: impl Into<String>, path: impl Into<String>) -> Self {
        Self { description: desc.into(), check: CompletionCheck::OutputExists { path_hint: path.into() } }
    }
    pub fn errors_logged(desc: impl Into<String>, log: impl Into<String>) -> Self {
        Self { description: desc.into(), check: CompletionCheck::ErrorsLogged { log_hint: log.into() } }
    }
    pub fn record_updated(desc: impl Into<String>, connector: impl Into<String>) -> Self {
        Self { description: desc.into(), check: CompletionCheck::RecordUpdated { connector: connector.into() } }
    }
    pub fn custom(desc: impl Into<String>) -> Self {
        let d = desc.into();
        Self { check: CompletionCheck::Custom { assertion: d.clone() }, description: d }
    }
}

/// Infer a FailureAction from the text of a failure rule.
pub fn infer_failure_action(lower: &str) -> FailureAction {
    if lower.contains("retry")   { return FailureAction::RetryOnce; }
    if lower.contains("abort") || lower.contains("stop all") { return FailureAction::Abort; }
    if lower.contains("escalat") || lower.contains("human") || lower.contains("handoff") {
        let ch = if lower.contains("slack") { Some("#ops-alerts".into()) } else { None };
        return FailureAction::EscalateToHuman { notify_channel: ch };
    }
    if lower.contains("silent") || lower.contains("ignore") { return FailureAction::SkipSilently; }
    FailureAction::SkipAndLog { log_path: "workspace/errors.txt".into() }
}

/// Complete, typed execution guidelines for an agent role.
/// Composed from: domain skill (static) + intent extraction (LLM) +
/// clarification steps (user answers) + connector overrides (derived).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionGuidelines {
    #[serde(default)] pub rules:               Vec<GuidelineRule>,
    #[serde(default)] pub failure_handling:    Vec<FailureRule>,
    #[serde(default)] pub priorities:          Vec<String>,
    #[serde(default)] pub completion_criteria: Vec<CompletionCriterion>,
}

impl ExecutionGuidelines {
    const MAX_RULES:      usize = 12;
    const MAX_FAILURE:    usize = 8;
    const MAX_PRIORITIES: usize = 5;
    const MAX_COMPLETION: usize = 6;

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.failure_handling.is_empty()
            && self.priorities.is_empty() && self.completion_criteria.is_empty()
    }

    /// Render a structured, numbered prompt block — LLMs follow numbered lists more reliably.
    pub fn to_prompt(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        if !self.rules.is_empty() {
            let items: Vec<String> = self.rules.iter().take(Self::MAX_RULES).enumerate().map(|(i, r)| {
                let prefix = match (&r.tool_scope, &r.phase) {
                    (Some(t), RulePhase::Before) => format!("[BEFORE {}] ", t),
                    (Some(t), RulePhase::After)  => format!("[AFTER {}] ", t),
                    (Some(t), _)                 => format!("[{}] ", t),
                    (None, _)                    => String::new(),
                };
                format!("{}. {}{}", i + 1, prefix, r.text)
            }).collect();
            parts.push(format!("RULES (apply in order):\n{}", items.join("\n")));
        }

        if !self.failure_handling.is_empty() {
            let items: Vec<String> = self.failure_handling.iter().take(Self::MAX_FAILURE).enumerate().map(|(i, f)| {
                let scope = f.tool_scope.as_deref().map(|t| format!("[{} fails] ", t)).unwrap_or_default();
                let act = match &f.action {
                    FailureAction::SkipAndLog { log_path }           => format!("→ Skip, log to {}", log_path),
                    FailureAction::SkipSilently                      => "→ Skip silently".into(),
                    FailureAction::RetryOnce                         => "→ Retry once".into(),
                    FailureAction::EscalateToHuman { notify_channel: Some(ch) } => format!("→ Escalate, notify {}", ch),
                    FailureAction::EscalateToHuman { notify_channel: None }     => "→ Escalate to human".into(),
                    FailureAction::Abort                             => "→ Abort run".into(),
                };
                format!("{}. {}{} {}", i + 1, scope, f.text, act)
            }).collect();
            parts.push(format!("FAILURE HANDLING:\n{}", items.join("\n")));
        }

        if !self.priorities.is_empty() {
            let items: Vec<String> = self.priorities.iter().take(Self::MAX_PRIORITIES)
                .enumerate().map(|(i, p)| format!("{}. {}", i + 1, p)).collect();
            parts.push(format!("PRIORITIES:\n{}", items.join("\n")));
        }

        if !self.completion_criteria.is_empty() {
            let items: Vec<String> = self.completion_criteria.iter().take(Self::MAX_COMPLETION)
                .enumerate().map(|(i, c)| format!("{}. [ ] {}", i + 1, c.description)).collect();
            parts.push(format!("DONE WHEN ALL OF:\n{}", items.join("\n")));
        }

        parts.join("\n\n")
    }

    pub fn add_rule(&mut self, r: GuidelineRule) {
        if self.rules.len() < Self::MAX_RULES && !self.rules.iter().any(|x| x.text == r.text) {
            self.rules.push(r);
        }
    }
    pub fn add_failure(&mut self, r: FailureRule) {
        if self.failure_handling.len() < Self::MAX_FAILURE
            && !self.failure_handling.iter().any(|x| x.text == r.text) {
            self.failure_handling.push(r);
        }
    }
    pub fn add_priority(&mut self, p: impl Into<String>) {
        let s = p.into();
        if self.priorities.len() < Self::MAX_PRIORITIES && !self.priorities.contains(&s) {
            self.priorities.push(s);
        }
    }
    pub fn add_completion(&mut self, c: CompletionCriterion) {
        if self.completion_criteria.len() < Self::MAX_COMPLETION
            && !self.completion_criteria.iter().any(|x| x.description == c.description) {
            self.completion_criteria.push(c);
        }
    }
    pub fn extend_dedup(&mut self, other: ExecutionGuidelines) {
        for r in other.rules             { self.add_rule(r); }
        for f in other.failure_handling  { self.add_failure(f); }
        for p in other.priorities        { self.add_priority(p); }
        for c in other.completion_criteria { self.add_completion(c); }
    }

    /// Parse a domain skill EXECUTION BRIEF text section into typed guidelines.
    pub fn from_skill_text(text: &str) -> Self {
        let mut out = Self::default();
        enum Sec { Rules, Failure, Priorities, Completion }
        let mut sec = Sec::Rules;

        for line in text.lines() {
            let t = line.trim().trim_start_matches("- ").trim_start_matches("• ").trim_start_matches("* ");
            if t.is_empty() { continue; }
            let l = t.to_lowercase();
            if l.starts_with("execution brief") || l.starts_with("rules:") { sec = Sec::Rules; continue; }
            if l.starts_with("on failure") || l.starts_with("failure:") || l.starts_with("on error") { sec = Sec::Failure; continue; }
            if l.starts_with("priorit") { sec = Sec::Priorities; continue; }
            if l.starts_with("done when") || l.starts_with("completion") || l.starts_with("complete when") { sec = Sec::Completion; continue; }
            if l.starts_with("mandatory") || l.starts_with("before confirm") { continue; }
            // Skip numbered question items
            if t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && t.contains('.') { if matches!(sec, Sec::Rules) { continue; } }
            if t.len() < 8 { continue; }

            match sec {
                Sec::Rules => {
                    let fkws = ["skip","if fail","on error","retry","escalate","notify","if missing","fallback","when missing"];
                    if fkws.iter().any(|k| l.contains(k)) {
                        out.add_failure(FailureRule { text: t.into(), tool_scope: None, action: infer_failure_action(&l) });
                    } else {
                        out.add_rule(GuidelineRule::always(t));
                    }
                }
                Sec::Failure    => { out.add_failure(FailureRule { text: t.into(), tool_scope: None, action: infer_failure_action(&l) }); }
                Sec::Priorities => { out.add_priority(t); }
                Sec::Completion => { out.add_completion(CompletionCriterion::custom(t)); }
            }
        }
        out
    }

    /// Parse user free-text constraints into typed guidelines.
    pub fn from_user_constraints(text: &str) -> Self {
        let mut out = Self::default();
        for part in text.split(&[',', ';', '\n'][..]) {
            let t = part.trim().trim_end_matches('.');
            if t.len() < 6 { continue; }
            let l = t.to_lowercase();
            if l.starts_with("no constraint") || l == "none" || l == "n/a" || l == "defaults" { continue; }
            let fkws = ["skip","if fail","on error","retry","notify","escalate","if missing","when missing","fallback"];
            let ckws = ["when done","complete when","done when","all processed","once all","after all"];
            if ckws.iter().any(|k| l.contains(k)) {
                out.add_completion(CompletionCriterion::custom(t));
            } else if fkws.iter().any(|k| l.contains(k)) {
                out.add_failure(FailureRule { text: t.into(), tool_scope: None, action: infer_failure_action(&l) });
            } else {
                out.add_rule(GuidelineRule::always(t));
            }
        }
        out
    }
}

impl From<&str> for ExecutionGuidelines {
    fn from(s: &str) -> Self {
        // Try JSON parse first — allows passing a full serialised ExecutionGuidelines
        if let Ok(parsed) = serde_json::from_str::<ExecutionGuidelines>(s) {
            return parsed;
        }

        // Try parsing as structured text via the existing helper
        let from_text = ExecutionGuidelines::from_skill_text(s);
        if !from_text.is_empty() {
            return from_text;
        }

        // Fallback: treat the whole string as a single always-rule
        let mut g = ExecutionGuidelines::default();
        if !s.trim().is_empty() {
            g.add_rule(GuidelineRule::always(s.trim()));
        }
        g
    }
}

/// How confident the trigger parser is in its interpretation.
/// Used to decide whether to ask for explicit confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TriggerConfidence {
    /// Parsed unambiguously — no confirmation needed.
    High,
    /// Parsed with reasonable confidence but ambiguous details — ask to confirm.
    #[default]
    Medium,
    /// Could not parse reliably — must ask.
    Low,
}

/// Detected sub-responsibility within a user's description.
/// Used to suggest splitting into multiple roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleResponsibility {
    pub name:    String,
    pub actions: Vec<String>,
    pub trigger_hint: String,
}

/// Fully-specified trigger for a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDef {
    pub trigger_type: TriggerType,

    // ── Webhook fields ─────────────────────────────────────────────────────
    /// Connector that delivers the webhook, e.g. "salesforce".
    pub source_connector: Option<String>,
    /// Event name to match, e.g. "lead_created".
    pub event_filter: Option<String>,

    // ── Schedule fields ────────────────────────────────────────────────────
    /// Cron expression, e.g. "0 9 * * 5" for Friday 9am.
    pub cron: Option<String>,
    /// IANA timezone, e.g. "America/New_York". Defaults to UTC.
    pub timezone: Option<String>,

    // ── UserMessage fields ─────────────────────────────────────────────────
    /// If set, only these user IDs can invoke this role via message.
    /// Empty vec means any authenticated user.
    pub allowed_users: Option<Vec<String>>,
    /// Keywords that help the router match this role to an incoming message.
    /// e.g. ["enrich", "lead", "prospect"] for a lead enrichment role.
    pub intent_keywords: Option<Vec<String>>,

    // ── WorkforceEvent fields ──────────────────────────────────────────────
    /// JSONPath-style filter on the workforce event, e.g.:
    ///   "role_name == 'Lead Enrichment' AND status == 'completed'"
    pub workforce_event_filter: Option<String>,
    /// JSONPath mappings from the event payload to this role's input_data.
    /// e.g. { "lead_id": "$.output_data.lead_id" }
    pub input_mapping: Option<serde_json::Value>,

    // ── AgentCompletion (within-agent chaining) ────────────────────────────
    /// Role ID within this agent that must complete before this role fires.
    pub depends_on_role_id: Option<String>,

    /// How confident the system is in this trigger's interpretation.
    /// Medium/Low means the user was asked to confirm before saving.
    #[serde(default)]
    pub confidence: TriggerConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    /// Fires when an external system sends a webhook.
    Webhook,
    /// Fires on a cron schedule.
    Schedule,
    /// Fires when a user sends a message that matches this role's intent.
    UserMessage,
    /// Manually triggered via API or UI. Never fires automatically.
    Manual,
    /// Fires when any goal instance emits a matching WorkforceEvent.
    WorkforceEvent,
}

impl Default for TriggerDef {
    fn default() -> Self {
        Self {
            trigger_type:            TriggerType::Manual,
            source_connector:        None,
            event_filter:            None,
            cron:                    None,
            timezone:                None,
            allowed_users:           None,
            intent_keywords:         None,
            workforce_event_filter:  None,
            input_mapping:           None,
            depends_on_role_id:      None,
            confidence:              TriggerConfidence::default(),
        }
    }
}

// ── OutputSpec ─────────────────────────────────────────────────────────────

/// Structured description of what this role produces and where it goes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSpec {
    /// Output format.
    pub format: OutputFormat,

    /// Human-readable description of the output content.
    /// e.g. "Summary of enriched lead: company size, news, tech stack, outreach draft"
    pub description: String,

    /// Optional JSON schema for structured outputs.
    /// When present, the executor validates the output before writing.
    pub schema: Option<serde_json::Value>,

    /// Where the output is delivered.
    pub destination: OutputDestination,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    Markdown,
    Json,
    Html,
}

/// Where the role's output is delivered when execution completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputDestination {
    /// Write to a file in the agent's workspace. Path is relative.
    Workspace { path: Option<String> },
    /// Update a field on a connector record.
    /// e.g. Connector { name: "salesforce", record_id_field: "lead_id", target_field: "Description" }
    Connector {
        name: String,
        record_id_field: String,
        target_field: String,
    },
    /// Post a message to a Slack-like channel via a connector.
    Channel {
        connector: String,
        channel: String,
    },
    /// Send an email via a connector (draft only if draft: true).
    Email {
        connector: String,
        draft: bool,
    },
    /// Emit a workforce event that other roles can subscribe to.
    /// The output becomes the event's output_data payload.
    WorkforceEvent { event_name: String },
    /// Return as a conversational reply in the chat UI.
    ConversationReply,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            format: OutputFormat::Markdown,
            description: String::new(),
            schema: None,
            destination: OutputDestination::Workspace { path: None },
        }
    }
}

// ── MemoryScope ────────────────────────────────────────────────────────────

/// Controls what memory this role can read and write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Shared across all agents for this tenant.
    /// Use for reference data: company info, preferences, shared knowledge.
    Global,
    /// Shared across all roles of this agent.
    /// Default — most roles should use this.
    Agent,
    /// Isolated to this role only.
    /// Use for roles that produce noisy, role-specific intermediate data
    /// that would pollute other roles if shared.
    Role,
}

// ── ExecutionLimits ────────────────────────────────────────────────────────

/// Safety limits applied to every GoalInstance of this role.
/// Prevents runaway agents and controls cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLimits {
    /// Maximum number of planner steps. Default 15.
    pub max_steps: u32,
    /// Maximum retry attempts per step. Default 2.
    pub max_retries: u32,
    /// Wall-clock timeout per goal instance in seconds. Default 600 (10 min).
    pub timeout_secs: u64,
    /// Maximum LLM cost per goal instance in USD.
    /// None = no limit (use with caution on research-heavy roles).
    pub max_cost_usd: Option<f64>,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_steps: 15,
            max_retries: 2,
            timeout_secs: 600,
            max_cost_usd: None,
        }
    }
}

// ── Plan mode conversation state ────────────────────────────────────────────

/// Tracks the state of an in-progress plan mode configuration session.
/// Stored temporarily while the user is answering questions.
/// Discarded once the AgentDefinition + first AgentRole are saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanModeSession {
    pub id: String,
    pub tenant_id: String,

    /// Partial agent definition being built.
    pub draft_agent: AgentDefinition,

    /// Partial role being configured (first role is built during plan mode).
    pub draft_role: Option<AgentRole>,

    /// Conversation history so the LLM has context across turns.
    pub conversation: Vec<PlanModeMessage>,

    /// Which step of the plan mode flow we're on.
    pub phase: PlanModePhase,

    /// Cached intent extracted in CapturingIntent phase.
    #[serde(default)]
    pub intent_cache: Option<serde_json::Value>,

    /// Sequential clarification step queue.
    /// Each element is a serialised ClarificationStep (opaque to definition.rs).
    /// Consumed one per turn in CapturingClarifications phase.
    #[serde(default)]
    pub pending_steps: Vec<serde_json::Value>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanModeMessage {
    pub role: String,   // "user" | "assistant"
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanModePhase {
    /// Gathering what the agent should do.
    CapturingIntent,
    /// System is internally resolving which connectors are needed.
    /// User may be asked one clarifying question if ambiguous.
    ResolvingConnectors,
    /// Combined phase: trigger confirmation, output questions, multi-role suggestion.
    /// Replaces the old CapturingTrigger + CapturingOutput phases.
    /// One LLM-context-aware turn covers everything.
    CapturingClarifications,
    /// Gathering constraints and guidelines (domain skill mandatory questions).
    CapturingConstraints,
    /// Reviewing the complete configuration with the user before saving.
    Reviewing,
    /// Configuration saved — session complete.
    Complete,
}

// ── TenantConnector ────────────────────────────────────────────────────────

/// A user-defined custom connector, specific to one tenant.
/// Created when the LLM discovers a needed connector isn't built-in.
/// Persists permanently for the tenant after creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantConnector {
    pub id: String,
    pub tenant_id: String,

    /// Connector name as it appears in role definitions, e.g. "acme_erp".
    pub name: String,

    /// Category this connector belongs to, e.g. "connector/erp".
    pub category: String,

    /// Base URL for API calls, e.g. "https://erp.acme.com/api".
    pub base_url: String,

    /// Auth mechanism.
    pub auth_type: ConnectorAuthType,

    /// Key name in the tenant's credential store that holds the token/key.
    pub auth_credential_key: Option<String>,

    /// How this connector's endpoints were derived.
    pub source: ConnectorSource,

    /// Raw documentation content (OpenAPI spec, markdown, etc.) if uploaded.
    pub source_docs: Option<String>,

    /// Derived endpoint definitions — either from docs parsing or manual input.
    pub endpoints: Vec<EndpointDef>,

    /// One-line summary shown in the connector directory.
    pub summary: String,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAuthType {
    Bearer,
    ApiKeyHeader { header_name: String },
    Basic,
    OAuth2,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorSource {
    /// Built by the LLM from knowledge of a known SaaS product.
    KnownSaas { product_name: String },
    /// Derived by parsing an uploaded OpenAPI/Swagger spec.
    OpenApiSpec,
    /// Derived by parsing uploaded API documentation (PDF, markdown, HTML).
    ApiDocs,
    /// Manually specified by the user endpoint by endpoint.
    Manual,
}

/// A single API endpoint on a custom connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointDef {
    /// HTTP method: GET, POST, PUT, PATCH, DELETE.
    pub method: String,
    /// Path relative to base_url, e.g. "/customers/{id}".
    pub path: String,
    /// Human-readable description of what this endpoint does.
    pub description: String,
    /// Parameters this endpoint accepts.
    pub params: Vec<EndpointParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointParam {
    pub name: String,
    pub location: ParamLocation,
    pub param_type: String,
    pub description: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamLocation {
    Path,
    Query,
    Body,
    Header,
}

// ── WorkforceEvent (subscription model) ────────────────────────────────────

/// Persisted subscription record — "role X listens for event matching filter Y".
/// Created when a role with TriggerType::WorkforceEvent is saved.
/// Polled by the scheduler to fire new GoalInstances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkforceEventSubscription {
    pub id: String,
    pub tenant_id: String,

    /// The role that fires when a matching event arrives.
    pub subscriber_role_id: String,
    pub subscriber_agent_id: String,

    /// Filter expression evaluated against incoming GoalCompleted/GoalFailed events.
    /// Simple equality expressions: "role_name == 'Lead Enrichment'"
    /// Combined with AND: "role_name == 'Lead Enrichment' AND status == 'completed'"
    pub event_filter: String,

    /// JSONPath mappings from the event payload to the new GoalInstance's input_data.
    /// e.g. { "lead_id": "$.output_data.lead_id", "company": "$.output_data.company" }
    pub input_mapping: serde_json::Value,

    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// An event emitted by a completed or failed GoalInstance.
/// Published to the workforce event bus and matched against subscriptions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkforceEventPayload {
    pub tenant_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub role_id: String,
    pub role_name: String,
    pub goal_instance_id: String,

    /// "completed" or "failed"
    pub status: String,

    /// The result data from the completed GoalInstance.
    /// Used by input_mapping to populate the next GoalInstance's input_data.
    pub output_data: serde_json::Value,

    /// Failure reason, populated when status == "failed".
    pub failure_reason: Option<String>,

    pub emitted_at: DateTime<Utc>,
}

impl WorkforceEventPayload {
    /// Evaluate a simple filter expression against this event.
    /// Supports: field == 'value', field == 'value' AND field2 == 'value2'
    /// Fields: role_name, agent_name, status, role_id, agent_id
    pub fn matches_filter(&self, filter: &str) -> bool {
        filter.split(" AND ").all(|clause| {
            let parts: Vec<&str> = clause.trim().splitn(2, " == ").collect();
            if parts.len() != 2 {
                return false;
            }
            let field = parts[0].trim();
            let value = parts[1].trim().trim_matches('\'').trim_matches('"');
            match field {
                "role_name"  => self.role_name == value,
                "agent_name" => self.agent_name == value,
                "status"     => self.status == value,
                "role_id"    => self.role_id == value,
                "agent_id"   => self.agent_id == value,
                _            => false,
            }
        })
    }

    /// Apply input_mapping JSONPath expressions to extract fields for a new GoalInstance.
    /// Supports simple dot-path expressions: "$.output_data.lead_id"
    pub fn apply_mapping(&self, mapping: &serde_json::Value) -> serde_json::Value {
        let obj = match mapping.as_object() {
            Some(m) => m,
            None    => return serde_json::Value::Object(Default::default()),
        };

        let self_json = serde_json::to_value(self).unwrap_or_default();
        let mut result = serde_json::Map::new();

        for (key, path_val) in obj {
            if let Some(path) = path_val.as_str() {
                if let Some(extracted) = resolve_jsonpath(&self_json, path) {
                    result.insert(key.clone(), extracted);
                }
            }
        }

        serde_json::Value::Object(result)
    }
}

/// Minimal JSONPath resolver — supports "$.field.subfield" dot notation only.
/// Full JSONPath (arrays, filters) is out of scope for V1.
fn resolve_jsonpath(value: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current.clone())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent() -> AgentDefinition {
        let mut a = AgentDefinition::new("ag-1".into(), "t-1".into(), "Sales Ops Agent".into());
        a.connectors = vec!["salesforce".into(), "slack".into(), "web_search".into()];
        a
    }

    fn make_role() -> AgentRole {
        AgentRole::new("role-1".into(), "ag-1".into(), "t-1".into(), "Lead Enrichment".into())
    }

    // ── AgentDefinition ────────────────────────────────────────────────────

    #[test]
    fn test_agent_defaults_to_draft() {
        let a = make_agent();
        assert_eq!(a.status, AgentDefinitionStatus::Draft);
    }

    #[test]
    fn test_validate_role_connectors_ok() {
        let agent = make_agent();
        let violations = agent.validate_role_connectors(&["salesforce".into(), "slack".into()]);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_validate_role_connectors_violation() {
        let agent = make_agent();
        let violations = agent.validate_role_connectors(&["salesforce".into(), "github".into()]);
        assert_eq!(violations, vec!["github"]);
    }

    #[test]
    fn test_validate_role_connectors_empty_ok() {
        let agent = make_agent();
        let violations = agent.validate_role_connectors(&[]);
        assert!(violations.is_empty());
    }

    // ── AgentRole ──────────────────────────────────────────────────────────

    #[test]
    fn test_role_defaults_to_draft() {
        let r = make_role();
        assert_eq!(r.status, RoleStatus::Draft);
        assert_eq!(r.version, 1);
    }

    #[test]
    fn test_role_bump_version() {
        let mut r = make_role();
        r.bump_version();
        assert_eq!(r.version, 2);
        r.bump_version();
        assert_eq!(r.version, 3);
    }

    #[test]
    fn test_role_not_live_when_draft() {
        let r = make_role();
        assert!(!r.is_live());
    }

    #[test]
    fn test_role_is_live_when_active() {
        let mut r = make_role();
        r.status = RoleStatus::Active;
        assert!(r.is_live());
    }

    // ── ExecutionLimits ────────────────────────────────────────────────────

    #[test]
    fn test_execution_limits_defaults() {
        let l = ExecutionLimits::default();
        assert_eq!(l.max_steps, 15);
        assert_eq!(l.max_retries, 2);
        assert_eq!(l.timeout_secs, 600);
        assert!(l.max_cost_usd.is_none());
    }

    // ── WorkforceEventPayload ──────────────────────────────────────────────

    fn make_event() -> WorkforceEventPayload {
        WorkforceEventPayload {
            tenant_id: "t-1".into(),
            agent_id: "ag-1".into(),
            agent_name: "Sales Ops Agent".into(),
            role_id: "role-1".into(),
            role_name: "Lead Enrichment".into(),
            goal_instance_id: "gi-1".into(),
            status: "completed".into(),
            output_data: serde_json::json!({ "lead_id": "L-1234", "company": "Acme Corp" }),
            failure_reason: None,
            emitted_at: Utc::now(),
        }
    }

    #[test]
    fn test_event_matches_single_clause() {
        let ev = make_event();
        assert!(ev.matches_filter("role_name == 'Lead Enrichment'"));
        assert!(!ev.matches_filter("role_name == 'Weekly Report'"));
    }

    #[test]
    fn test_event_matches_and_clause() {
        let ev = make_event();
        assert!(ev.matches_filter("role_name == 'Lead Enrichment' AND status == 'completed'"));
        assert!(!ev.matches_filter("role_name == 'Lead Enrichment' AND status == 'failed'"));
    }

    #[test]
    fn test_event_matches_status() {
        let ev = make_event();
        assert!(ev.matches_filter("status == 'completed'"));
        assert!(!ev.matches_filter("status == 'failed'"));
    }

    #[test]
    fn test_event_unknown_field_no_match() {
        let ev = make_event();
        assert!(!ev.matches_filter("unknown_field == 'value'"));
    }

    #[test]
    fn test_apply_mapping_extracts_fields() {
        let ev = make_event();
        let mapping = serde_json::json!({
            "lead_id": "$.output_data.lead_id",
            "company": "$.output_data.company"
        });
        let result = ev.apply_mapping(&mapping);
        assert_eq!(result["lead_id"], "L-1234");
        assert_eq!(result["company"], "Acme Corp");
    }

    #[test]
    fn test_apply_mapping_missing_path_skipped() {
        let ev = make_event();
        let mapping = serde_json::json!({
            "lead_id": "$.output_data.lead_id",
            "missing": "$.output_data.does_not_exist"
        });
        let result = ev.apply_mapping(&mapping);
        assert_eq!(result["lead_id"], "L-1234");
        assert!(result.get("missing").is_none());
    }

    #[test]
    fn test_apply_mapping_empty_returns_empty() {
        let ev = make_event();
        let result = ev.apply_mapping(&serde_json::json!({}));
        assert!(result.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_resolve_jsonpath_nested() {
        let v = serde_json::json!({ "a": { "b": { "c": 42 } } });
        let result = resolve_jsonpath(&v, "$.a.b.c");
        assert_eq!(result, Some(serde_json::json!(42)));
    }

    #[test]
    fn test_resolve_jsonpath_missing() {
        let v = serde_json::json!({ "a": 1 });
        let result = resolve_jsonpath(&v, "$.a.b.c");
        assert!(result.is_none());
    }

    // ── Serialisation round-trips ──────────────────────────────────────────

    #[test]
    fn test_trigger_def_serialises() {
        let t = TriggerDef {
            trigger_type: TriggerType::WorkforceEvent,
            workforce_event_filter: Some("role_name == 'Lead Enrichment'".into()),
            input_mapping: Some(serde_json::json!({ "lead_id": "$.output_data.lead_id" })),
            ..Default::default()
        };
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["trigger_type"], "workforce_event");
        assert!(json["workforce_event_filter"].is_string());
    }

    #[test]
    fn test_output_destination_connector_serialises() {
        let dest = OutputDestination::Connector {
            name: "salesforce".into(),
            record_id_field: "lead_id".into(),
            target_field: "Description".into(),
        };
        let json = serde_json::to_value(&dest).unwrap();
        assert_eq!(json["type"], "connector");
        assert_eq!(json["name"], "salesforce");
    }

    #[test]
    fn test_memory_scope_serialises() {
        let s = MemoryScope::Role;
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json, "role");
    }
}
