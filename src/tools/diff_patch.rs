//! diff_patch — Compute unified diffs and apply patches using `similar`.
//! Used by code agents to show/apply changes cleanly without full file overwrites.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_boolean, schema_integer};

pub struct DiffTool;
pub struct PatchTool;

#[async_trait]
impl Tool for DiffTool {
    fn name(&self) -> &str {
        "diff"
    }
    fn description(&self) -> &str {
        "Compute a unified diff between two files or two text strings. \
         Returns the diff as a patch string that can be applied with the patch tool."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional("old_path", "string", "Path to the original file."),
            ParameterSchema::optional("new_path", "string", "Path to the new file."),
            ParameterSchema::optional("old_text", "string", "Original text (alternative to old_path)."),
            ParameterSchema::optional("new_text", "string", "New text (alternative to new_path)."),
            ParameterSchema::optional("context", "integer", "Context lines around changes (default: 3)."),
            ParameterSchema::optional("label_old", "string", "Label for old file in diff header (default: 'old')."),
            ParameterSchema::optional("label_new", "string", "Label for new file in diff header (default: 'new')."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["patch", "insertions", "deletions", "unchanged", "has_changes"],
            "properties": {
                "patch": schema_string(),
                "insertions": schema_integer(),
                "deletions": schema_integer(),
                "unchanged": schema_integer(),
                "has_changes": schema_boolean(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let old = if let Some(p) = args["old_path"].as_str() {
            tokio::fs::read_to_string(p).await.map_err(|e| anyhow::anyhow!("read old: {}", e))?
        } else {
            args["old_text"].as_str().unwrap_or("").to_string()
        };
        let new = if let Some(p) = args["new_path"].as_str() {
            tokio::fs::read_to_string(p).await.map_err(|e| anyhow::anyhow!("read new: {}", e))?
        } else {
            args["new_text"].as_str().unwrap_or("").to_string()
        };

        let ctx = args["context"].as_u64().unwrap_or(3) as usize;
        let label_old = args["label_old"].as_str().unwrap_or("old");
        let label_new = args["label_new"].as_str().unwrap_or("new");

        let diff = similar::TextDiff::from_lines(&old, &new);
        let patch = diff.unified_diff().context_radius(ctx).header(label_old, label_new).to_string();

        // Compute stats manually from diff ops (similar v2.6 has no .stats())
        let mut insertions: usize = 0;
        let mut deletions: usize = 0;
        let mut unchanged: usize = 0;
        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Insert => insertions += 1,
                similar::ChangeTag::Delete => deletions += 1,
                similar::ChangeTag::Equal => unchanged += 1,
            }
        }
        Ok(ToolResult::ok(serde_json::json!({
            "patch":       patch,
            "insertions":  insertions,
            "deletions":   deletions,
            "unchanged":   unchanged,
            "has_changes": insertions > 0 || deletions > 0,
        })))
    }
}

#[async_trait]
impl Tool for PatchTool {
    fn name(&self) -> &str {
        "patch"
    }
    fn description(&self) -> &str {
        "Apply a unified diff patch to a file or text string. \
         Returns the patched content and optionally writes it back to disk."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("patch", "string", "Unified diff patch string (output of diff tool)."),
            ParameterSchema::optional("path", "string", "File to patch in-place."),
            ParameterSchema::optional("text", "string", "Text to patch (returns result, does not write)."),
            ParameterSchema::optional("output", "string", "Write patched result to this path instead."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let patch_str = match args["patch"].as_str() {
            Some(p) => p,
            None => return Ok(ToolResult::err("'patch' required")),
        };

        let original = if let Some(p) = args["path"].as_str() {
            tokio::fs::read_to_string(p).await.map_err(|e| anyhow::anyhow!("read: {}", e))?
        } else if let Some(t) = args["text"].as_str() {
            t.to_string()
        } else {
            return Ok(ToolResult::err("'path' or 'text' required"));
        };

        // Apply unified diff patch manually (similar v2.6 has no patch-apply API)
        let result = apply_unified_diff(&original, patch_str).map_err(|e| anyhow::anyhow!("patch failed: {}", e))?;

        // Write output
        let write_to = args["output"].as_str().or(args["path"].as_str());
        if let Some(dest) = write_to {
            tokio::fs::write(dest, &result).await.map_err(|e| anyhow::anyhow!("write: {}", e))?;
        }

        Ok(ToolResult::ok(serde_json::json!({
            "patched":     true,
            "result":      result,
            "chars":       result.len(),
            "written_to":  write_to,
        })))
    }
}

/// Parse and apply a unified diff patch to the original text.
/// Handles standard unified diff format produced by `similar`'s unified_diff().
fn apply_unified_diff(original: &str, patch: &str) -> Result<String, String> {
    let orig_lines: Vec<&str> = original.lines().collect();
    let mut result_lines: Vec<String> = Vec::new();
    let mut orig_idx: usize = 0; // 0-based index into orig_lines

    let patch_lines: Vec<&str> = patch.lines().collect();
    let mut pi = 0;

    // Skip file headers (--- and +++ lines)
    while pi < patch_lines.len() {
        let line = patch_lines[pi];
        if line.starts_with("@@") {
            break;
        }
        pi += 1;
    }

    while pi < patch_lines.len() {
        let line = patch_lines[pi];
        if line.starts_with("@@") {
            // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
            let header = line;
            let parts: Vec<&str> = header.split_whitespace().collect();
            if parts.len() < 3 {
                return Err(format!("invalid hunk header: {}", header));
            }
            let old_range = parts[1]; // e.g. "-1,5"
            let old_start: usize = old_range
                .trim_start_matches('-')
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("bad old range in hunk: {}", header))?;

            // Copy lines from original that come before this hunk
            let old_start_0 = if old_start == 0 { 0 } else { old_start - 1 }; // convert to 0-based
            while orig_idx < old_start_0 && orig_idx < orig_lines.len() {
                result_lines.push(orig_lines[orig_idx].to_string());
                orig_idx += 1;
            }

            pi += 1;
            // Process hunk body
            while pi < patch_lines.len() && !patch_lines[pi].starts_with("@@") {
                let hline = patch_lines[pi];
                if let Some(content) = hline.strip_prefix('+') {
                    // Added line — include in result, don't advance orig_idx
                    result_lines.push(content.to_string());
                } else if let Some(_content) = hline.strip_prefix('-') {
                    // Removed line — skip in original
                    orig_idx += 1;
                } else if let Some(content) = hline.strip_prefix(' ') {
                    // Context line
                    result_lines.push(content.to_string());
                    orig_idx += 1;
                } else if hline == "\\ No newline at end of file" {
                    // Skip this marker
                } else {
                    // Treat bare lines as context
                    result_lines.push(hline.to_string());
                    orig_idx += 1;
                }
                pi += 1;
            }
        } else {
            pi += 1;
        }
    }

    // Copy any remaining original lines after the last hunk
    while orig_idx < orig_lines.len() {
        result_lines.push(orig_lines[orig_idx].to_string());
        orig_idx += 1;
    }

    // Preserve trailing newline if original had one
    let mut output = result_lines.join("\n");
    if original.ends_with('\n') && !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_diff_identical() {
        let diff_tool = DiffTool;
        let result = diff_tool
            .execute(serde_json::json!({
                "old_text": "hello\nworld\n",
                "new_text": "hello\nworld\n"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output["has_changes"], false);
    }

    #[tokio::test]
    async fn test_diff_changed() {
        let diff_tool = DiffTool;
        let result = diff_tool
            .execute(serde_json::json!({
                "old_text": "hello\n",
                "new_text": "world\n"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output["has_changes"], true);
        assert!(result.output["insertions"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_patch_apply() {
        let diff_tool = DiffTool;
        let old_text = "line1\nline2\nline3\n";
        let new_text = "line1\nmodified\nline3\n";

        let diff_result = diff_tool
            .execute(serde_json::json!({
                "old_text": old_text,
                "new_text": new_text
            }))
            .await
            .unwrap();
        assert!(diff_result.success);
        let patch_str = diff_result.output["patch"].as_str().unwrap();

        let patch_tool = PatchTool;
        let patch_result = patch_tool
            .execute(serde_json::json!({
                "patch": patch_str,
                "text": old_text
            }))
            .await
            .unwrap();
        assert!(patch_result.success);
        assert_eq!(patch_result.output["patched"], true);
    }

    #[tokio::test]
    async fn test_diff_patch_roundtrip() {
        let old_text = "line1\nline2\n";
        let new_text = "line1\nmodified\nline3\n";

        let diff_tool = DiffTool;
        let diff_result = diff_tool
            .execute(serde_json::json!({
                "old_text": old_text,
                "new_text": new_text
            }))
            .await
            .unwrap();
        assert!(diff_result.success);
        let patch_str = diff_result.output["patch"].as_str().unwrap();

        let patch_tool = PatchTool;
        let patch_result = patch_tool
            .execute(serde_json::json!({
                "patch": patch_str,
                "text": old_text
            }))
            .await
            .unwrap();
        assert!(patch_result.success);
        let result_text = patch_result.output["result"].as_str().unwrap();
        assert_eq!(result_text, new_text);
    }
}
