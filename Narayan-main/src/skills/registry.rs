use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::agent::planner::{Plan, PlannedStep};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillStepDefinition {
    pub description: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub tool_args: Option<serde_json::Value>,
    #[serde(default)]
    pub success_criteria: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SkillStep {
    Text(String),
    Detailed(SkillStepDefinition),
}

impl SkillStep {
    pub fn description(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Detailed(step) => &step.description,
        }
    }

    pub fn to_planned_step(&self, index: usize) -> PlannedStep {
        match self {
            Self::Text(text) => PlannedStep {
                index,
                description: text.clone(),
                tool: None,
                tool_args: None,
                success_criteria: format!("step {} complete", index + 1),
                condition: None,
            },
            Self::Detailed(step) => PlannedStep {
                index,
                description: step.description.clone(),
                tool: step.tool.clone(),
                tool_args: step.tool_args.clone(),
                success_criteria: if step.success_criteria.trim().is_empty() {
                    format!("step {} complete", index + 1)
                } else {
                    step.success_criteria.clone()
                },
                condition: None,
            },
        }
    }
}

impl From<String> for SkillStep {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for SkillStep {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub steps: Vec<SkillStep>,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub version: u32,
}

impl Skill {
    pub fn new(name: impl Into<String>, description: impl Into<String>, steps: Vec<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            steps: steps.into_iter().map(SkillStep::from).collect(),
            aliases: Vec::new(),
            version: 1,
        }
    }

    pub fn structured(
        name: impl Into<String>,
        description: impl Into<String>,
        steps: Vec<SkillStepDefinition>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            steps: steps.into_iter().map(SkillStep::Detailed).collect(),
            aliases: Vec::new(),
            version: 1,
        }
    }

    pub fn with_aliases(mut self, aliases: Vec<&str>) -> Self {
        self.aliases = aliases.into_iter().map(str::to_string).collect();
        self
    }
}

pub struct SkillRegistry {
    skills: HashMap<String, Skill>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self { skills: HashMap::new() }
    }

    pub fn register(&mut self, skill: Skill) {
        self.skills.insert(skill.name.clone(), skill);
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// Find a skill whose name appears in the goal string (fuzzy match).
    pub fn find_matching(&self, goal: &str) -> Option<&Skill> {
        let lower = goal.to_lowercase();
        self.skills
            .values()
            .filter_map(|skill| {
                let mut phrases = vec![skill.name.to_lowercase()];
                phrases.extend(skill.aliases.iter().map(|alias| alias.to_lowercase()));
                let score = phrases
                    .iter()
                    .filter(|phrase| !phrase.is_empty() && lower.contains(phrase.as_str()))
                    .map(|phrase| phrase.len())
                    .max()?;
                Some((score, skill))
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, skill)| skill)
    }

    pub fn list(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    pub fn count(&self) -> usize {
        self.skills.len()
    }

    pub fn with_curated_defaults() -> Self {
        let mut registry = Self::new();
        for skill in curated_skills() {
            registry.register(skill);
        }
        registry
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Plan construction from Skill ───────────────────────────────────────────

impl Plan {
    /// Build a Plan directly from a Skill — no LLM call needed.
    pub fn from_skill(skill: &Skill) -> Self {
        let steps = skill
            .steps
            .iter()
            .enumerate()
            .map(|(i, step)| step.to_planned_step(i))
            .collect();
        Plan {
            goal: skill.description.clone(),
            job_type: Some("skill".into()),
            steps,
            rationale: format!("using pre-built skill: {}", skill.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill() -> Skill {
        Skill::new("deploy", "deploy the application", vec!["build".into(), "test".into(), "push".into()])
    }

    #[test]
    fn test_register_get() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill());
        let skill = reg.get("deploy").expect("skill should exist");
        assert_eq!(skill.name, "deploy");
        assert_eq!(reg.count(), 1);
    }

    #[test]
    fn test_find_matching() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill());
        let found = reg.find_matching("deploy app").expect("should find a match");
        assert_eq!(found.name, "deploy");
    }

    #[test]
    fn test_plan_from_skill() {
        let skill = make_skill();
        let plan = Plan::from_skill(&skill);
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.goal, "deploy the application");
        assert_eq!(plan.steps[0].description, "build");
        assert_eq!(plan.steps[2].description, "push");
    }
}

fn curated_skills() -> Vec<Skill> {
    vec![
        Skill::structured(
            "connect gmail",
            "Connect Gmail or Google securely before running email workflows.",
            vec![
                SkillStepDefinition {
                    description: "Ask the user to connect Gmail or Google in Settings before any email action.".into(),
                    tool: Some("ask_user".into()),
                    tool_args: Some(serde_json::json!({
                        "questions": [{
                            "id": "gmail_connector",
                            "prompt": "Connect your Gmail account so I can continue with the email task.",
                            "helper_text": "Open Settings and connect Gmail or Google, then come back here and confirm once it is ready.",
                            "connector_type": "gmail",
                            "action_label": "Connect Gmail in Settings",
                            "required": true,
                            "placeholder": "Type 'connected' once Gmail is ready"
                        }]
                    })),
                    success_criteria: "User is prompted to connect Gmail with a connector action card.".into(),
                },
                SkillStepDefinition {
                    description: "Verify Gmail-related credentials or connector access are available before sending, reading, or monitoring email.".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: "Agent proceeds only after Gmail access exists.".into(),
                },
                SkillStepDefinition {
                    description: "Continue with the original Gmail task using the newly connected account and do not ask for the same setup twice.".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: "Original email workflow resumes with the connected account.".into(),
                },
            ],
        )
        .with_aliases(vec!["gmail", "connect google", "google mail", "google workspace"]),
        Skill::structured(
            "database monitoring",
            "Set up database monitoring with secure credential collection and explicit approval gates.",
            vec![
                SkillStepDefinition {
                    description: "Collect the missing database details and the connection string securely before any monitoring work.".into(),
                    tool: Some("ask_user".into()),
                    tool_args: Some(serde_json::json!({
                        "questions": [
                            {
                                "id": "db_type",
                                "prompt": "Which database type should I monitor?",
                                "options": ["postgres", "mysql", "sqlite", "other"],
                                "required": true,
                                "placeholder": "postgres"
                            },
                            {
                                "id": "db_host",
                                "prompt": "What host should I check?",
                                "required": true,
                                "placeholder": "db.internal"
                            },
                            {
                                "id": "db_port",
                                "prompt": "What port should I use?",
                                "required": true,
                                "placeholder": "5432"
                            },
                            {
                                "id": "db_health_query",
                                "prompt": "What lightweight health query should I run?",
                                "required": false,
                                "placeholder": "SELECT 1"
                            },
                            {
                                "id": "db_connection",
                                "prompt": "Paste the database connection string.",
                                "helper_text": "This stays hidden and will be stored for tool use rather than shown back in chat.",
                                "secret": true,
                                "store_as_credential": "db_connection",
                                "required": true,
                                "placeholder": "postgres://user:password@host:5432/db"
                            }
                        ]
                    })),
                    success_criteria: "All missing DB inputs are requested through secure UI fields.".into(),
                },
                SkillStepDefinition {
                    description: "Inspect the current machine and database-related processes before changing anything.".into(),
                    tool: Some("process_monitor".into()),
                    tool_args: Some(serde_json::json!({
                        "action": "system"
                    })),
                    success_criteria: "Current system state is captured before monitoring changes.".into(),
                },
                SkillStepDefinition {
                    description: "Validate database connectivity with a harmless query using the stored connection string and stop if it fails.".into(),
                    tool: Some("sql_query".into()),
                    tool_args: Some(serde_json::json!({
                        "query": "SELECT 1",
                        "connection_key": "db_connection",
                        "max_rows": 1
                    })),
                    success_criteria: "Database connectivity is verified before any monitoring script or cron setup.".into(),
                },
                SkillStepDefinition {
                    description: "Request explicit user approval before adding any recurring cron job or scheduled monitor.".into(),
                    tool: Some("ask_user".into()),
                    tool_args: Some(serde_json::json!({
                        "questions": [{
                            "id": "cron_approval",
                            "prompt": "Should I add a recurring monitoring schedule now?",
                            "helper_text": "This would create a background job such as a cron entry.",
                            "options": ["yes", "no"],
                            "required": true,
                            "placeholder": "yes or no"
                        }]
                    })),
                    success_criteria: "Human approval is collected before any recurring schedule is added.".into(),
                },
            ],
        )
        .with_aliases(vec!["monitor database", "monitor db", "db monitoring", "database monitor"]),
        Skill::structured(
            "connector onboarding",
            "Guide the user through connector setup and secure credential handoff before external integrations run.",
            vec![
                SkillStepDefinition {
                    description: "Ask which external service must be connected and direct the user to Settings when a connector is required.".into(),
                    tool: Some("ask_user".into()),
                    tool_args: Some(serde_json::json!({
                        "questions": [{
                            "id": "service_name",
                            "prompt": "Which service should I connect for this task?",
                            "required": true,
                            "placeholder": "gmail, github, slack, notion..."
                        }]
                    })),
                    success_criteria: "The required external service is identified.".into(),
                },
                SkillStepDefinition {
                    description: "If an API key, password, or token is still needed, collect it with hidden fields and store it as a reusable credential.".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: "Secrets are requested only through secure hidden inputs.".into(),
                },
                SkillStepDefinition {
                    description: "Verify the connector or credential exists before any outbound API call and do not retry the same missing setup twice.".into(),
                    tool: None,
                    tool_args: None,
                    success_criteria: "The integration path is confirmed before proceeding.".into(),
                },
            ],
        )
        .with_aliases(vec!["setup connector", "connect service", "oauth setup", "connect integration"]),

        // ── Plan mode domain skills ──────────────────────────────────────────────
        // These are injected during plan mode's CapturingConstraints phase when the
        // intent category matches. Each skill encodes the mandatory questions to ask
        // for that domain AND the execution brief that goes into the role's guidelines.

        Skill::new(
            "planmode:customer_support",
            "Domain configuration for customer support agents.",
            vec![
                "MANDATORY QUESTIONS — ask all of these before confirming:\n\
                 1. Response mode: should I draft replies for human approval, or send automatically?\n\
                 2. First-response SLA: how fast must the first reply go out? (15 min / 1 hr / 4 hr / best-effort)\n\
                 3. Escalation: which ticket types should always escalate to a human? (billing, legal threats, VIP, angry tone?)\n\
                 4. Tone: formal, friendly, or match the customer?\n\
                 5. Knowledge source: is there a URL, Notion page, or help docs to search for answers?".into(),
                "EXECUTION BRIEF for the agent:\n\
                 - Always search the knowledge base before composing a reply\n\
                 - If knowledge base returns nothing: say so explicitly, do NOT hallucinate\n\
                 - On escalation: tag the ticket, add an internal note explaining why, do not send public reply\n\
                 - On SLA breach risk: warn in an internal note before the deadline, do not auto-close\n\
                 - Never share PII from one ticket with another customer".into(),
            ],
        ).with_aliases(vec!["customer_support", "support agent", "helpdesk", "zendesk", "intercom", "freshdesk"]),

        Skill::new(
            "planmode:sales_revops",
            "Domain configuration for sales, CRM, and revenue operations agents.",
            vec![
                "MANDATORY QUESTIONS — ask all of these before confirming:\n\
                 1. Data source: which CRM fields to read? (Lead, Contact, Account, Opportunity?)\n\
                 2. Write-back: update records automatically, or create tasks/notes only?\n\
                 3. Enrichment sources: web search, LinkedIn, company data APIs, or CRM only?\n\
                 4. Outreach: draft email in workspace, add to sequence, or send directly?\n\
                 5. Qualification criteria: what makes a lead worth enriching? (minimum company size, industry, title?)".into(),
                "EXECUTION BRIEF for the agent:\n\
                 - Read before writing: always fetch current record state before any update\n\
                 - Deduplication: check if a contact/company already exists before creating new records\n\
                 - On missing data: skip the record and log it, do not fill with guesses\n\
                 - On outreach: save draft to workspace first, only send if explicitly configured to auto-send\n\
                 - Never overwrite existing CRM notes — always append".into(),
            ],
        ).with_aliases(vec!["sales_revops", "crm", "salesforce", "hubspot", "lead enrichment", "outreach", "pipeline"]),

        Skill::new(
            "planmode:finance_accounting",
            "Domain configuration for finance and accounting agents.",
            vec![
                "MANDATORY QUESTIONS — ask all of these before confirming:\n\
                 1. Write access: read-only reporting, or can the agent create/update records?\n\
                 2. Approval gate: any transaction above what amount needs human approval before posting?\n\
                 3. Reconciliation window: which date range should be reconciled?\n\
                 4. Output format: QuickBooks, spreadsheet, PDF report, or Slack summary?\n\
                 5. Error handling: if a record doesn't match, flag for review or block the entire run?".into(),
                "EXECUTION BRIEF for the agent:\n\
                 - Never DELETE financial records — use void/reversal instead\n\
                 - Transactions above the approval threshold: create a pending record, notify, wait for confirmation\n\
                 - Always log the before/after state of any record you modify\n\
                 - On reconciliation mismatch: add a flagged note, do not auto-correct\n\
                 - Redact SSN, account numbers, and routing numbers from any output".into(),
            ],
        ).with_aliases(vec!["finance_accounting", "finance", "accounting", "quickbooks", "invoices", "reconciliation"]),

        Skill::new(
            "planmode:devops",
            "Domain configuration for DevOps, infrastructure, and SRE agents.",
            vec![
                "MANDATORY QUESTIONS — ask all of these before confirming:\n\
                 1. Environment: prod, staging, or dev? (Never default to prod)\n\
                 2. Blast radius: can this agent modify infrastructure, or read-only?\n\
                 3. Rollback plan: if a change fails, what's the recovery path?\n\
                 4. Alerting: which Slack channel or PagerDuty service for failure notifications?\n\
                 5. Change window: is there a maintenance window, or can changes run anytime?".into(),
                "EXECUTION BRIEF for the agent:\n\
                 - Dry-run first: always run with --dry-run or equivalent before applying changes\n\
                 - Snapshot before mutation: take a state snapshot before any destructive operation\n\
                 - On failure: stop immediately, do not attempt partial recovery — alert and wait\n\
                 - Never touch prod without an explicit confirmation in the current run\n\
                 - Log every command and its exit code to the workspace".into(),
            ],
        ).with_aliases(vec!["devops", "it_ops_itsm", "infrastructure", "kubernetes", "sre", "deployment", "pagerduty", "servicenow"]),

        Skill::new(
            "planmode:hr_people_ops",
            "Domain configuration for HR and people operations agents.",
            vec![
                "MANDATORY QUESTIONS — ask all of these before confirming:\n\
                 1. Data sensitivity: does this touch compensation, performance ratings, or termination data?\n\
                 2. Visibility: who can see the agent's output? (HR only, manager, candidate, everyone?)\n\
                 3. Write-back: update ATS records, or report only?\n\
                 4. Compliance: any GDPR/CCPA regions where candidates have data deletion rights?\n\
                 5. Communication: draft offers/rejections in workspace, or send directly?".into(),
                "EXECUTION BRIEF for the agent:\n\
                 - Compensation and performance data: never include in outbound messages or logs\n\
                 - Candidate PII: process in workspace only, do not write to external systems without explicit config\n\
                 - On rejection: use approved template language, no improvisation\n\
                 - Right-to-erasure requests: log them immediately, do not process other tasks until acknowledged\n\
                 - Never compare candidates by protected characteristics (age, gender, race, religion)".into(),
            ],
        ).with_aliases(vec!["hr_people_ops", "hr", "recruiting", "greenhouse", "hiring", "candidates", "onboarding"]),

        Skill::new(
            "planmode:legal_contract",
            "Domain configuration for legal and contract management agents.",
            vec![
                "MANDATORY QUESTIONS — ask all of these before confirming:\n\
                 1. Action scope: flag issues only, or redline and suggest edits?\n\
                 2. Governing law: which jurisdiction? (affects which clauses are standard vs. unusual)\n\
                 3. Counterparty type: customer, vendor, partner, or employee?\n\
                 4. Escalation: which clause types must always go to legal counsel? (indemnity cap, IP assignment, exclusivity?)\n\
                 5. Output: annotated PDF, tracked-changes Word doc, or summary report?".into(),
                "EXECUTION BRIEF for the agent:\n\
                 - This agent identifies and flags risks — it does NOT give legal advice\n\
                 - Always include a disclaimer: 'Review by qualified counsel before signing'\n\
                 - Escalation clauses: indemnity, IP assignment, exclusivity, non-compete — always flag, never auto-accept\n\
                 - On missing standard clause: note the gap, do not fill it with boilerplate\n\
                 - Version every document: never overwrite the original, save redlines separately".into(),
            ],
        ).with_aliases(vec!["legal_contract", "legal", "contract", "docusign", "nda", "agreement", "redline"]),

        Skill::new(
            "planmode:research_analyst",
            "Domain configuration for research and analysis agents.",
            vec![
                "MANDATORY QUESTIONS — ask all of these before confirming:\n\
                 1. Sources: web search only, or also internal documents, databases, specific URLs?\n\
                 2. Depth: quick summary (3-5 sources) or deep research (10+ sources with citations)?\n\
                 3. Output format: bullet summary, structured report, or raw data?\n\
                 4. Freshness: how recent must sources be? (last 7 days / 30 days / any)\n\
                 5. Confidence threshold: flag uncertain findings, or only report high-confidence data?".into(),
                "EXECUTION BRIEF for the agent:\n\
                 - Always cite sources with URLs — never present findings without attribution\n\
                 - Contradictory sources: present both views, do not pick one without evidence\n\
                 - On paywalled content: note the source exists but could not be accessed\n\
                 - Distinguish clearly between facts and inferences\n\
                 - If fewer than 3 sources found: say so and ask whether to proceed or broaden scope".into(),
            ],
        ).with_aliases(vec!["research_analyst", "research", "analysis", "report", "competitor analysis", "market research"]),
    ]
}
