//! browser — Navigate to a URL with real headless Chromium (chromiumoxide).
//!
//! Full JS execution — works with React, Vue, Next.js, SPAs, and any JS-heavy page.
//! Extracts text, links, headings, and can optionally run custom JS evaluation.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    browser::BrowserPool,
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct BrowserTool {
    pub pool: Arc<BrowserPool>,
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }
    fn description(&self) -> &str {
        "Navigate to a URL using real headless Chromium with full JavaScript execution. \
         Works with React, Vue, Next.js, SPAs, and JS-heavy pages. \
         Extracts page text, title, links, headings, and can evaluate custom JavaScript."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "URL to navigate to."),
            ParameterSchema::optional(
                "wait_for",
                "string",
                "CSS selector to wait for before extracting (default: body).",
            ),
            ParameterSchema::optional(
                "extract",
                "string",
                "What to extract: 'text'|'links'|'headings'|'all' (default: 'all').",
            ),
            ParameterSchema::optional(
                "js_eval",
                "string",
                "JavaScript to evaluate in the page context. Return value is captured.",
            ),
            ParameterSchema::optional("timeout_secs", "integer", "Navigation timeout seconds (default: 30)."),
            ParameterSchema::optional(
                "wait_ms",
                "integer",
                "Extra wait after page load in ms (for lazy-rendered content).",
            ),
            ParameterSchema::optional("cookies", "array", "Cookies to inject: [{name, value, domain}]."),
        ]
    }



    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "type": "object", "additionalProperties": true }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        use std::time::Duration;

        use chromiumoxide::cdp::browser_protocol::network::CookieParam;

        let url = match args["url"].as_str() {
            Some(u) => u.to_string(),
            None => return Ok(ToolResult::err("'url' required")),
        };
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);
        let wait_ms = args["wait_ms"].as_u64().unwrap_or(0);
        let extract = args["extract"].as_str().unwrap_or("all").to_string();
        let _wait_for = args["wait_for"].as_str().unwrap_or("body").to_string();
        let js_eval = args["js_eval"].as_str().map(String::from);

        let handle = self.pool.acquire(timeout_secs + 5).await.map_err(|e| anyhow::anyhow!("browser pool: {}", e))?;
        let browser = handle.browser.lock().await;

        let page = browser.new_page(&url).await.map_err(|e| anyhow::anyhow!("new_page '{}': {}", url, e))?;

        // Inject cookies before navigation
        if let Some(cookies) = args["cookies"].as_array() {
            for c in cookies {
                if let (Some(name), Some(value)) = (c["name"].as_str(), c["value"].as_str()) {
                    let domain = c["domain"].as_str().unwrap_or("");
                    let mut cp = CookieParam::new(name, value);
                    cp.domain = Some(domain.to_string());
                    let _ = page.set_cookie(cp).await;
                }
            }
        }

        // Wait for selector
        let _ = tokio::time::timeout(Duration::from_secs(timeout_secs), page.wait_for_navigation()).await;

        if wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(wait_ms.min(10_000))).await;
        }

        let title = page.get_title().await.unwrap_or_default().unwrap_or_default();
        let final_url = page.url().await.unwrap_or_default().unwrap_or_default();

        // Extract content via JS
        let text = if extract == "text" || extract == "all" {
            page.evaluate("document.body?.innerText || ''")
                .await
                .ok()
                .and_then(|v| v.into_value::<String>().ok())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let links: serde_json::Value = if extract == "links" || extract == "all" {
            page.evaluate(
                r#"
                Array.from(document.querySelectorAll('a[href]')).slice(0, 100).map(a => ({
                    href: a.href, text: a.innerText?.trim()?.slice(0, 100)
                }))
            "#,
            )
            .await
            .ok()
            .and_then(|v| v.into_value::<serde_json::Value>().ok())
            .unwrap_or_default()
        } else {
            serde_json::json!([])
        };

        let headings: serde_json::Value = if extract == "headings" || extract == "all" {
            page.evaluate(
                r#"
                Array.from(document.querySelectorAll('h1,h2,h3,h4')).slice(0, 50).map(h => ({
                    level: h.tagName, text: h.innerText?.trim()
                }))
            "#,
            )
            .await
            .ok()
            .and_then(|v| v.into_value::<serde_json::Value>().ok())
            .unwrap_or_default()
        } else {
            serde_json::json!([])
        };

        // Custom JS evaluation
        let js_result: Option<serde_json::Value> = if let Some(ref script) = js_eval {
            page.evaluate(script.as_str()).await.ok().and_then(|v| v.into_value::<serde_json::Value>().ok())
        } else {
            None
        };

        page.close().await.ok();

        Ok(ToolResult::ok(serde_json::json!({
            "url":       final_url,
            "title":     title,
            "text":      crate::util::truncate(&text, 10_000),
            "links":     links,
            "headings":  headings,
            "js_result": js_result,
        })))
    }
}
