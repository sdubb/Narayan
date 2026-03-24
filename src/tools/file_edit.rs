use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct FileEditTool;

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }
    fn description(&self) -> &str {
        "Edit a file by replacing occurrences of a string or regex pattern with a new string. \
         Fails if the 'old' string is not found, preventing silent no-ops."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("path", "string", "File path to edit."),
            ParameterSchema::required("old", "string", "Exact text to find and replace."),
            ParameterSchema::required("new", "string", "Replacement text."),
            ParameterSchema::optional("count", "integer", "Max replacements to make (0 = all, default)."),
            ParameterSchema::optional("use_regex", "boolean", "Treat 'old' as a regex pattern (default: false)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = match args["path"].as_str() {
            Some(p) => std::path::PathBuf::from(p),
            None => return Ok(ToolResult::err("'path' is required")),
        };
        let old = match args["old"].as_str() {
            Some(o) if !o.is_empty() => o.to_string(),
            _ => return Ok(ToolResult::err("'old' is required and must not be empty")),
        };
        let new = args["new"].as_str().unwrap_or("").to_string();
        let count = args["count"].as_u64().unwrap_or(0) as usize;
        let use_re = args["use_regex"].as_bool().unwrap_or(false);

        let original =
            tokio::fs::read_to_string(&path).await.map_err(|e| anyhow::anyhow!("read '{}': {}", path.display(), e))?;

        if use_re {
            let re = regex::Regex::new(&old).map_err(|e| anyhow::anyhow!("invalid regex: {}", e))?;
            if !re.is_match(&original) {
                return Ok(ToolResult::err(format!("Pattern not found in '{}'", path.display())));
            }
            let replaced = if count == 0 {
                re.replace_all(&original, new.as_str()).into_owned()
            } else {
                let mut result = original.clone();
                let mut n = 0;
                while n < count {
                    if let Some(m) = re.find(&result.clone()) {
                        let s = m.start();
                        let e = m.end();
                        result = format!("{}{}{}", &result[..s], &new, &result[e..]);
                        n += 1;
                    } else {
                        break;
                    }
                }
                result
            };
            tokio::fs::write(&path, &replaced).await.map_err(|e| anyhow::anyhow!("write: {e}"))?;
            Ok(ToolResult::ok(serde_json::json!({"edited": true, "path": path.display().to_string()})))
        } else {
            if !original.contains(&old) {
                return Ok(ToolResult::err(format!(
                    "String not found in '{}': {:?}",
                    path.display(),
                    crate::util::truncate(&old, 80)
                )));
            }
            let replaced = if count == 0 {
                original.replace(&old, &new)
            } else {
                let mut result = original.clone();
                for _ in 0..count {
                    if let Some(pos) = result.find(&old) {
                        result.replace_range(pos..pos + old.len(), &new);
                    } else {
                        break;
                    }
                }
                result
            };
            let replacements = if count == 0 { original.matches(&old).count() } else { count };
            tokio::fs::write(&path, &replaced).await.map_err(|e| anyhow::anyhow!("write: {e}"))?;
            Ok(ToolResult::ok(
                serde_json::json!({"edited": true, "path": path.display().to_string(), "replacements": replacements}),
            ))
        }
    }
}
