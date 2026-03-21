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
    ]
}
