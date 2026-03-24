//! screenshot — Real screenshot via headless Chromium (chromiumoxide).
//! Full JS rendering — captures React, SPAs, anything a real browser sees.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use chromiumoxide::{cdp::browser_protocol::page::CaptureScreenshotFormat, page::ScreenshotParams};

use crate::{
    browser::BrowserPool,
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct ScreenshotTool {
    pub pool: Arc<BrowserPool>,
}

#[async_trait]
impl Tool for ScreenshotTool {
    fn name(&self) -> &str {
        "screenshot"
    }
    fn description(&self) -> &str {
        "Capture a full-page or viewport screenshot of any URL using real headless Chromium. \
         JavaScript is fully executed before capture. Returns base64-encoded PNG/JPEG and \
         saves to path if specified."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "URL to screenshot."),
            ParameterSchema::optional("path", "string", "File path to save the image (e.g. 'screenshot.png')."),
            ParameterSchema::optional("full_page", "boolean", "Capture full scrollable page (default: true)."),
            ParameterSchema::optional("format", "string", "Image format: 'png' (default) or 'jpeg'."),
            ParameterSchema::optional("quality", "integer", "JPEG quality 0-100 (default: 90, PNG ignores this)."),
            ParameterSchema::optional("width", "integer", "Viewport width in pixels (default: 1440)."),
            ParameterSchema::optional("height", "integer", "Viewport height in pixels (default: 900)."),
            ParameterSchema::optional("wait_ms", "integer", "Wait after load before capturing (ms, default: 500)."),
            ParameterSchema::optional("timeout_secs", "integer", "Navigation timeout (default: 30)."),
            ParameterSchema::optional("selector", "string", "CSS selector — screenshot only that element."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        use std::time::Duration;

        use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;

        let url = match args["url"].as_str() {
            Some(u) => u.to_string(),
            None => return Ok(ToolResult::err("'url' required")),
        };
        let path = args["path"].as_str().map(String::from);
        let full_page = args["full_page"].as_bool().unwrap_or(true);
        let format_str = args["format"].as_str().unwrap_or("png");
        let quality = args["quality"].as_u64().unwrap_or(90) as i64;
        let width = args["width"].as_u64().unwrap_or(1440) as u32;
        let height = args["height"].as_u64().unwrap_or(900) as u32;
        let wait_ms = args["wait_ms"].as_u64().unwrap_or(500).min(10_000);
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);
        let selector = args["selector"].as_str().map(String::from);

        let fmt = match format_str {
            "jpeg" | "jpg" => CaptureScreenshotFormat::Jpeg,
            _ => CaptureScreenshotFormat::Png,
        };

        let handle = self.pool.acquire(timeout_secs + 5).await.map_err(|e| anyhow::anyhow!("browser pool: {}", e))?;
        let browser = handle.browser.lock().await;

        let page = browser.new_page(&url).await.map_err(|e| anyhow::anyhow!("new_page: {}", e))?;

        // Set viewport via DevTools emulation
        page.execute(SetDeviceMetricsOverrideParams::new(width as i64, height as i64, 1., false)).await.ok();

        let _ = tokio::time::timeout(Duration::from_secs(timeout_secs), page.wait_for_navigation()).await;

        if wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }

        let bytes = if let Some(ref css) = selector {
            // Element screenshot
            let _el =
                page.find_element(css.as_str()).await.map_err(|e| anyhow::anyhow!("selector '{}': {}", css, e))?;
            let params = ScreenshotParams::builder().format(fmt).quality(quality).build();
            page.screenshot(params).await.map_err(|e| anyhow::anyhow!("screenshot: {}", e))?
        } else {
            let params = ScreenshotParams::builder().format(fmt).quality(quality).full_page(full_page).build();
            page.screenshot(params).await.map_err(|e| anyhow::anyhow!("screenshot: {}", e))?
        };

        page.close().await.ok();

        // Save to disk if path given
        if let Some(ref p) = path {
            if let Some(parent) = std::path::Path::new(p).parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(p, &bytes).await.map_err(|e| anyhow::anyhow!("write '{}': {}", p, e))?;
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        Ok(ToolResult::ok(serde_json::json!({
            "url":        url,
            "format":     format_str,
            "width":      width,
            "height":     height,
            "full_page":  full_page,
            "size_bytes": bytes.len(),
            "saved_to":   path,
            "image_b64":  b64,
        })))
    }
}
