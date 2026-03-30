use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

/// PDF reader — uses pdftotext CLI (poppler) if available, otherwise base64 encodes the file.
pub struct PdfReadTool;

#[async_trait]
impl Tool for PdfReadTool {
    fn name(&self) -> &str {
        "pdf_read"
    }
    fn description(&self) -> &str {
        "Extract text content from a PDF file. Requires pdftotext (poppler-utils) to be installed."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ path, start_page?, end_page? }. path is required and must point to a PDF file.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ text, path, char_count, total_pages }. Returns extracted text plus PDF metadata.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use when the source is a PDF and you need its text or page metadata.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when the source is not a PDF or when you need structured record transforms after extraction.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("path", "string", "Path to the PDF file."),
            ParameterSchema::optional("start_page", "integer", "First page to extract (1-based, default: 1)."),
            ParameterSchema::optional("end_page", "integer", "Last page to extract (default: all)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path = match args["path"].as_str() {
            Some(p) => p.to_string(),
            None => return Ok(ToolResult::err("'path' is required")),
        };
        if !std::path::Path::new(&path).exists() {
            return Ok(ToolResult::err(format!("File not found: '{}'", path)));
        }
        let start = args["start_page"].as_u64().unwrap_or(1);
        let end = args["end_page"].as_u64();
        let mut cmd_str = format!("pdftotext -f {start}");
        if let Some(e) = end {
            cmd_str.push_str(&format!(" -l {e}"));
        }
        cmd_str.push_str(&format!(" {} -", shell_quote(&path)));
        let out = tokio::process::Command::new("sh").arg("-c").arg(&cmd_str).output().await;
        match out {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout).into_owned();
                let info_out = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("pdfinfo {} 2>/dev/null", shell_quote(&path)))
                    .output()
                    .await
                    .ok();
                let info = info_out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
                let pages = info
                    .lines()
                    .find(|l| l.starts_with("Pages:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|s| s.trim().parse::<u64>().ok());
                Ok(ToolResult::ok(serde_json::json!({
                    "text":       crate::util::truncate(&text, 50_000),
                    "path":       path,
                    "char_count": text.len(),
                    "total_pages": pages,
                })))
            }
            _ => Ok(ToolResult::err(
                "pdftotext not found. Install with: apt install poppler-utils OR brew install poppler",
            )),
        }
    }
}
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
