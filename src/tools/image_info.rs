use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct ImageInfoTool;

#[async_trait]
impl Tool for ImageInfoTool {
    fn name(&self) -> &str {
        "image_info"
    }
    fn description(&self) -> &str {
        "Get metadata about an image file: format, dimensions, size. Uses 'file' and 'identify' (ImageMagick) CLI tools."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![ParameterSchema::required("path", "string", "Path to the image file.")]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "type": "object", "additionalProperties": true }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = match args["path"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(ToolResult::err("'path' is required")),
        };
        if !std::path::Path::new(&path).exists() {
            return Ok(ToolResult::err(format!("File not found: '{}'", path)));
        }
        let meta = tokio::fs::metadata(&path).await.map_err(|e| anyhow::anyhow!(e))?;
        let ext = std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("unknown").to_lowercase();
        // Try ImageMagick identify
        let identify = tokio::process::Command::new("identify").arg("-verbose").arg(&path).output().await.ok();
        if let Some(out) = identify.filter(|o| o.status.success()) {
            let info = String::from_utf8_lossy(&out.stdout).into_owned();
            let width = extract_dimension(&info, "Geometry");
            let height = extract_dimension_h(&info, "Geometry");
            return Ok(ToolResult::ok(serde_json::json!({
                "path":   path, "format": ext, "size_bytes": meta.len(),
                "width":  width, "height": height,
            })));
        }
        Ok(ToolResult::ok(serde_json::json!({ "path": path, "format": ext, "size_bytes": meta.len() })))
    }
}
fn extract_dimension(info: &str, key: &str) -> Option<u32> {
    info.lines()
        .find(|l| l.trim().starts_with(key))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().split('x').next())
        .and_then(|s| s.split('+').next())
        .and_then(|s| s.trim().parse().ok())
}
fn extract_dimension_h(info: &str, key: &str) -> Option<u32> {
    info.lines()
        .find(|l| l.trim().starts_with(key))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().split('x').nth(1))
        .and_then(|s| s.split('+').next())
        .and_then(|s| s.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_image_info_nonexistent() {
        let tool = ImageInfoTool;
        let result = tool
            .execute(serde_json::json!({
                "path": "/tmp/narayan_test_nonexistent_image_xyz.png"
            }))
            .await
            .unwrap();
        assert!(!result.success, "expected error for nonexistent image file");
    }
}
