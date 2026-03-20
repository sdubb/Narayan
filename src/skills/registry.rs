use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::agent::planner::{Plan, PlannedStep};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub steps: Vec<String>,
    pub version: u32,
}

impl Skill {
    pub fn new(name: impl Into<String>, description: impl Into<String>, steps: Vec<String>) -> Self {
        Self { name: name.into(), description: description.into(), steps, version: 1 }
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
        self.skills.values().find(|s| lower.contains(&s.name.to_lowercase()))
    }

    pub fn list(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    pub fn count(&self) -> usize {
        self.skills.len()
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
            .map(|(i, s)| PlannedStep {
                index: i,
                description: s.clone(),
                tool: None,
                tool_args: None,
                success_criteria: format!("step {} complete", i + 1),
            })
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
