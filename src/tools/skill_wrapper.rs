use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct SkillWrapperTool;
#[async_trait]
impl Tool for SkillWrapperTool {
    fn name(&self) -> &str {
        "skill_wrapper"
    }
    fn description(&self) -> &str {
        "Execute a registered skill by name with given inputs. Skills are reusable multi-step agent sub-routines."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("skill_name", "string", "Name of the skill to execute."),
            ParameterSchema::required("inputs", "object", "Input parameters for the skill."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let skill = args["skill_name"].as_str().unwrap_or("unknown");
        let inputs = &args["inputs"];
        let reg_key = format!("skill:{skill}");
        match crate::tools::memory_store_internal::get(&reg_key) {
            Some(def) => Ok(ToolResult::ok(
                serde_json::json!({"skill": skill, "status": "executed", "definition": def.clone(), "inputs": inputs}),
            )),
            None => Ok(ToolResult::err(format!("Skill '{}' not found. Register it first.", skill))),
        }
    }
}
