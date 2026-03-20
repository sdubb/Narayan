use crate::skills::registry::Skill;

pub fn evolve_skill(old: &Skill, improvements: Vec<String>) -> Skill {
    let mut steps = old.steps.clone();
    for s in improvements {
        if !steps.contains(&s) {
            steps.push(s);
        }
    }
    Skill {
        name: format!("{}-v2", old.name),
        description: format!("Improved version of {}", old.description),
        steps,
        version: old.version + 1,
    }
}
