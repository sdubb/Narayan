use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

const MAX_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }
    fn description(&self) -> &str {
        "Read the contents of a file. Returns UTF-8 text. \
         Supports optional line range. Capped at 10 MiB."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("path", "string", "Absolute or workspace-relative file path."),
            ParameterSchema::optional("start_line", "integer", "First line to read (1-based, inclusive)."),
            ParameterSchema::optional("end_line", "integer", "Last line to read (1-based, inclusive)."),
            ParameterSchema::optional("encoding", "string", "'utf8' (default) or 'base64' for binary files."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = match args["path"].as_str() {
            Some(p) => std::path::PathBuf::from(p),
            None => return Ok(ToolResult::err("'path' is required")),
        };
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => return Ok(ToolResult::err(format!("cannot stat '{}': {}", path.display(), e))),
        };
        if meta.len() > MAX_SIZE {
            return Ok(ToolResult::err(format!("file too large ({} bytes, max 10 MiB)", meta.len())));
        }

        let encoding = args["encoding"].as_str().unwrap_or("utf8");
        if encoding == "base64" {
            let bytes = tokio::fs::read(&path).await.map_err(|e| anyhow::anyhow!(e))?;
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(ToolResult::ok(serde_json::json!({
                "content":  b64,
                "encoding": "base64",
                "size":     bytes.len(),
            })));
        }

        let content =
            tokio::fs::read_to_string(&path).await.map_err(|e| anyhow::anyhow!("read '{}': {}", path.display(), e))?;

        let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize;
        let end = args["end_line"].as_u64().map(|v| v as usize);
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let sliced = if start > 1 || end.is_some() {
            let s = (start - 1).min(total);
            let e = end.unwrap_or(total).min(total);
            lines[s..e].join("\n")
        } else {
            content.clone()
        };

        Ok(ToolResult::ok(serde_json::json!({
            "content":     sliced,
            "path":        path.display().to_string(),
            "total_lines": total,
            "size_bytes":  meta.len(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_read_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("test.txt");
        std::fs::write(&file_path, "hello from test").unwrap();

        let tool = FileReadTool;
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.display().to_string()
            }))
            .await
            .unwrap();
        assert!(result.success);
        let content = result.output["content"].as_str().unwrap();
        assert!(content.contains("hello from test"));
    }

    #[tokio::test]
    async fn test_read_missing() {
        let tool = FileReadTool;
        let result = tool
            .execute(serde_json::json!({
                "path": "/tmp/narayan_test_nonexistent_file_xyz.txt"
            }))
            .await
            .unwrap();
        assert!(!result.success, "expected error for nonexistent file");
    }
}
