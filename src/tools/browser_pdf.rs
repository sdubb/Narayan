//! browser_pdf — Print a page to PDF using headless Chromium.
//!
//! Renders the full page with JS executed then exports as PDF.
//! Supports custom margins, page size, header/footer templates.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use base64::Engine;
use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;

use crate::{
    browser::BrowserPool,
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct BrowserPdfTool {
    pub pool: Arc<BrowserPool>,
}

#[async_trait]
impl Tool for BrowserPdfTool {
    fn name(&self) -> &str {
        "browser_pdf"
    }
    fn description(&self) -> &str {
        "Export any web page to PDF using real headless Chromium. \
         JavaScript is fully executed before export. Returns base64 PDF \
         and saves to path if specified."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "URL to export as PDF."),
            ParameterSchema::optional("path", "string", "File path to save the PDF."),
            ParameterSchema::optional("landscape", "boolean", "Landscape orientation (default: false)."),
            ParameterSchema::optional(
                "print_background",
                "boolean",
                "Include background colors/images (default: true).",
            ),
            ParameterSchema::optional("scale", "number", "Page scale 0.1–2.0 (default: 1.0)."),
            ParameterSchema::optional("margin_top", "number", "Top margin in inches (default: 0.4)."),
            ParameterSchema::optional("margin_bottom", "number", "Bottom margin (default: 0.4)."),
            ParameterSchema::optional("margin_left", "number", "Left margin (default: 0.4)."),
            ParameterSchema::optional("margin_right", "number", "Right margin (default: 0.4)."),
            ParameterSchema::optional("wait_ms", "integer", "Wait after load before export (ms, default: 500)."),
            ParameterSchema::optional("timeout_secs", "integer", "Navigation timeout (default: 30)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = match args["url"].as_str() {
            Some(u) => u.to_string(),
            None => return Ok(ToolResult::err("'url' required")),
        };
        let path = args["path"].as_str().map(String::from);
        let landscape = args["landscape"].as_bool().unwrap_or(false);
        let print_bg = args["print_background"].as_bool().unwrap_or(true);
        let scale = args["scale"].as_f64().unwrap_or(1.0).clamp(0.1, 2.0);
        let margin_top = args["margin_top"].as_f64().unwrap_or(0.4);
        let margin_bottom = args["margin_bottom"].as_f64().unwrap_or(0.4);
        let margin_left = args["margin_left"].as_f64().unwrap_or(0.4);
        let margin_right = args["margin_right"].as_f64().unwrap_or(0.4);
        let wait_ms = args["wait_ms"].as_u64().unwrap_or(500).min(10_000);
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);

        let handle = self.pool.acquire(timeout_secs + 5).await?;
        let browser = handle.browser.lock().await;

        let page = browser.new_page(&url).await.map_err(|e| anyhow::anyhow!("new_page: {}", e))?;

        let _ = tokio::time::timeout(Duration::from_secs(timeout_secs), page.wait_for_navigation()).await;
        if wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }

        let params = PrintToPdfParams {
            landscape: Some(landscape),
            print_background: Some(print_bg),
            scale: Some(scale),
            margin_top: Some(margin_top),
            margin_bottom: Some(margin_bottom),
            margin_left: Some(margin_left),
            margin_right: Some(margin_right),
            ..Default::default()
        };

        let pdf_bytes = page.pdf(params).await.map_err(|e| anyhow::anyhow!("pdf export: {}", e))?;

        page.close().await.ok();

        if let Some(ref p) = path {
            if let Some(parent) = std::path::Path::new(p).parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(p, &pdf_bytes).await.map_err(|e| anyhow::anyhow!("write '{}': {}", p, e))?;
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(&pdf_bytes);

        Ok(ToolResult::ok(serde_json::json!({
            "url":        url,
            "size_bytes": pdf_bytes.len(),
            "saved_to":   path,
            "pdf_b64":    b64,
        })))
    }
}
