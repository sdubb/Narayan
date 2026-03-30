use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct FileWriteTool;

fn normalize_text_content(content: &str) -> String {
    if content.contains('\n') || content.contains('\r') || !content.contains("\\") {
        return content.to_string();
    }

    content.replace("\\r\\n", "\r\n").replace("\\n", "\n").replace("\\t", "\t")
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }
    fn description(&self) -> &str {
        "Write or overwrite a file with the given content. \
         Creates parent directories as needed. Supports append mode."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ path, content, append?, encoding? }. path and content are required.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ written, path, bytes, appended }. Indicates the file write result.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use when the final artifact should be stored as a workspace file or when appending to an existing file.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when you only need to read content, or when the output should be a structured transform rather than a file write.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("path", "string", "File path to write."),
            ParameterSchema::required("content", "string", "Content to write."),
            ParameterSchema::optional("append", "boolean", "Append instead of overwrite (default: false)."),
            ParameterSchema::optional("encoding", "string", "'utf8' (default) or 'base64' for binary content."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = match args["path"].as_str() {
            Some(p) => std::path::PathBuf::from(p),
            None => return Ok(ToolResult::err("'path' is required")),
        };
        let content = match args["content"].as_str() {
            Some(c) => c.to_string(),
            None => return Ok(ToolResult::err("'content' is required")),
        };

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| anyhow::anyhow!("create dirs: {e}"))?;
            }
        }

        let append = args["append"].as_bool().unwrap_or(false);
        let encoding = args["encoding"].as_str().unwrap_or("utf8");

        let bytes: Vec<u8> = if encoding == "base64" {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(&content)
                .map_err(|e| anyhow::anyhow!("base64 decode: {e}"))?
        } else {
            normalize_text_content(&content).into_bytes()
        };

        if append {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|e| anyhow::anyhow!("open: {e}"))?;
            file.write_all(&bytes).await.map_err(|e| anyhow::anyhow!("write: {e}"))?;
        } else {
            tokio::fs::write(&path, &bytes).await.map_err(|e| anyhow::anyhow!("write '{}': {}", path.display(), e))?;
        }

        Ok(ToolResult::ok(serde_json::json!({
            "written":  true,
            "path":     path.display().to_string(),
            "bytes":    bytes.len(),
            "appended": append,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_create() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("new_file.txt");

        let tool = FileWriteTool;
        let result = tool
            .execute(serde_json::json!({
                "path": file_path.display().to_string(),
                "content": "hello world"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_write_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("overwrite.txt");

        let tool = FileWriteTool;
        tool.execute(serde_json::json!({
            "path": file_path.display().to_string(),
            "content": "first content"
        }))
        .await
        .unwrap();

        tool.execute(serde_json::json!({
            "path": file_path.display().to_string(),
            "content": "second content"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "second content");
    }

    #[test]
    fn test_normalize_escaped_newlines() {
        let content = normalize_text_content("alpha\\nbeta\\ngamma");
        assert_eq!(content, "alpha\nbeta\ngamma");
    }
}
