use crate::skills::registry::Skill;

pub fn evolve_skill(old: &Skill, improvements: Vec<String>) -> Skill {
    let mut steps = old.steps.clone();
    for s in improvements {
        if !steps.iter().any(|step| step.description() == s) {
            steps.push(crate::skills::registry::SkillStep::from(s));
        }
    }
    Skill {
        name: format!("{}-v2", old.name),
        description: format!("Improved version of {}", old.description),
        steps,
        aliases: old.aliases.clone(),
        version: old.version + 1,
    }
}
