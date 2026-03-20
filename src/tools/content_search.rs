use async_trait::async_trait;
use walkdir::WalkDir;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct ContentSearchTool;

#[async_trait]
impl Tool for ContentSearchTool {
    fn name(&self) -> &str {
        "content_search"
    }
    fn description(&self) -> &str {
        "Search for a text pattern inside files (like grep). Returns matching file paths, line numbers, and matched lines."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("pattern", "string", "Text or regex pattern to search for."),
            ParameterSchema::optional("path", "string", "File or directory to search (default: current dir)."),
            ParameterSchema::optional("glob", "string", "File glob filter, e.g. '*.rs' (default: all files)."),
            ParameterSchema::optional("max_results", "integer", "Max matching lines to return (default: 100)."),
            ParameterSchema::optional("case_insensitive", "boolean", "Case-insensitive match (default: false)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let pat = match args["pattern"].as_str() {
            Some(p) => p,
            None => return Ok(ToolResult::err("'pattern' is required")),
        };
        let search_path = args["path"].as_str().unwrap_or(".");
        let max = args["max_results"].as_u64().unwrap_or(100) as usize;
        let ci = args["case_insensitive"].as_bool().unwrap_or(false);
        let glob_pat = args["glob"].as_str();
        let glob_filter = glob_pat.and_then(|g| glob::Pattern::new(g).ok());

        let re = {
            let mut rb = regex::RegexBuilder::new(pat);
            rb.case_insensitive(ci);
            match rb.build() {
                Ok(r) => r,
                Err(e) => return Ok(ToolResult::err(format!("invalid pattern: {e}"))),
            }
        };

        let mut results: Vec<serde_json::Value> = Vec::new();
        let target = std::path::Path::new(search_path);

        let entries: Vec<_> = if target.is_file() {
            vec![target.to_path_buf()]
        } else {
            WalkDir::new(target)
                .follow_links(false)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
                .map(|e| e.path().to_path_buf())
                .collect()
        };

        'outer: for file_path in &entries {
            if let Some(ref gf) = glob_filter {
                let fname = file_path.file_name().unwrap_or_default().to_string_lossy();
                if !gf.matches(&fname) {
                    continue;
                }
            }
            // Skip binary files
            if is_likely_binary(file_path) {
                continue;
            }
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for (i, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    results.push(serde_json::json!({
                        "file":    file_path.display().to_string(),
                        "line_no": i + 1,
                        "line":    crate::util::truncate(line, 300),
                    }));
                    if results.len() >= max {
                        break 'outer;
                    }
                }
            }
        }

        Ok(ToolResult::ok(serde_json::json!({
            "pattern": pat,
            "count":   results.len(),
            "matches": results,
        })))
    }
}

fn is_likely_binary(path: &std::path::Path) -> bool {
    let binary_exts =
        ["png", "jpg", "jpeg", "gif", "bmp", "pdf", "zip", "tar", "gz", "bin", "exe", "so", "dylib", "wasm"];
    path.extension().and_then(|e| e.to_str()).map(|e| binary_exts.contains(&e.to_lowercase().as_str())).unwrap_or(false)
}
