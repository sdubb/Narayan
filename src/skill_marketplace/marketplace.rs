use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct MarketplaceSkill {
    pub name: String,
    pub author: String,
    pub description: String,
    pub steps: Vec<String>,
}

pub struct SkillMarketplace {
    skills: HashMap<String, MarketplaceSkill>,
}

impl SkillMarketplace {
    pub fn new() -> Self {
        Self { skills: HashMap::new() }
    }

    pub fn upload(&mut self, skill: MarketplaceSkill) {
        self.skills.insert(skill.name.clone(), skill);
    }

    pub fn list(&self) -> Vec<&MarketplaceSkill> {
        self.skills.values().collect()
    }

    pub fn get(&self, name: &str) -> Option<&MarketplaceSkill> {
        self.skills.get(name)
    }
}
