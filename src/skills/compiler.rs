use super::registry::Skill;

pub fn compile_skill(name: &str, description: &str, steps: Vec<String>) -> Skill {
    Skill::new(name.to_string(), description.to_string(), steps)
}
