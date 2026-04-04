use async_trait::async_trait;
use walkdir::WalkDir;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_boolean, schema_integer, schema_array};

pub struct GlobSearchTool;

#[async_trait]
impl Tool for GlobSearchTool {
    fn name(&self) -> &str {
        "glob_search"
    }
    fn description(&self) -> &str {
        "Find files matching a glob pattern (e.g. '**/*.rs', 'src/*.toml')."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ pattern, root?, max? }. pattern is required; root defaults to workspace root.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ pattern, root, count, files }. files contains matching path metadata.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use when you need to discover files by filename or path pattern before reading or editing them.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when you already know the exact file path or when you need text/content matching instead of path matching.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("pattern", "string", "Glob pattern to match files."),
            ParameterSchema::optional("root", "string", "Directory to search from (default: workspace root)."),
            ParameterSchema::optional("max", "integer", "Maximum results to return (default: 200)."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["pattern", "root", "count", "files"],
            "properties": {
                "pattern": schema_string(),
                "root": schema_string(),
                "count": schema_integer(),
                "files": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["path", "rel_path", "is_dir"],
                    "properties": {
                        "path": schema_string(),
                        "rel_path": schema_string(),
                        "is_dir": schema_boolean(),
                        "size": serde_json::json!({ "type": ["integer", "null"] }),
                    },
                    "additionalProperties": true,
                })),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pattern = match args["pattern"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(ToolResult::err("'pattern' is required")),
        };
        let root = args["root"].as_str().unwrap_or(".").to_string();
        let max = args["max"].as_u64().unwrap_or(200) as usize;
        let glob = match glob::Pattern::new(&pattern) {
            Ok(g) => g,
            Err(e) => return Ok(ToolResult::err(format!("Invalid glob pattern: {}", e))),
        };
        let root_path = std::path::PathBuf::from(&root);
        let mut matches = Vec::new();
        for entry in WalkDir::new(&root_path).follow_links(false).into_iter().flatten() {
            if matches.len() >= max {
                break;
            }
            let path = entry.path();
            // Match against the path relative to root
            let rel = path.strip_prefix(&root_path).unwrap_or(path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if glob.matches(&rel_str) || glob.matches(entry.file_name().to_string_lossy().as_ref()) {
                matches.push(serde_json::json!({
                    "path":     path.display().to_string(),
                    "rel_path": rel_str,
                    "is_dir":   path.is_dir(),
                    "size":     entry.metadata().ok().map(|m| m.len()),
                }));
            }
        }
        Ok(ToolResult::ok(serde_json::json!({
            "pattern": pattern,
            "root":    root,
            "count":   matches.len(),
            "files":   matches,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_find_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("test.txt"), "data").unwrap();

        let tool = GlobSearchTool;
        let result = tool
            .execute(serde_json::json!({
                "pattern": "*.txt",
                "root": tmp.path().display().to_string()
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output["count"].as_u64().unwrap() >= 1, "expected at least 1 match for *.txt");
    }

    #[tokio::test]
    async fn test_no_matches() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("test.txt"), "data").unwrap();

        let tool = GlobSearchTool;
        let result = tool
            .execute(serde_json::json!({
                "pattern": "*.xyz",
                "root": tmp.path().display().to_string()
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output["count"].as_u64().unwrap(), 0, "expected 0 matches for *.xyz");
    }
}
