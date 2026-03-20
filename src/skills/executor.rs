use super::registry::Skill;

pub fn execute_skill(skill: &Skill) {
    println!("Executing skill: {}", skill.name);
    for step in &skill.steps {
        println!("Step: {}", step);
    }
}
