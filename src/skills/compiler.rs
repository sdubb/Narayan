use super::registry::Skill;

pub fn compile_skill(name: &str, description: &str, steps: Vec<String>) -> Skill {
    Skill { name: name.to_string(), description: description.to_string(), steps, version: 1 }
}
