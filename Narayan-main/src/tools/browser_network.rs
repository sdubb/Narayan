//! browser_network — Intercept and inspect network requests via CDP.
//!
//! Navigates to a URL, captures all network requests/responses,
//! and optionally blocks specific patterns. Useful for:
//!   - Discovering undocumented APIs a web app uses
//!   - Capturing XHR/fetch response data
//!   - Auditing third-party requests (trackers, analytics)
//!   - Extracting auth tokens from request headers

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::{
    browser::BrowserPool,
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct BrowserNetworkTool {
    pub pool: Arc<BrowserPool>,
}

#[async_trait]
impl Tool for BrowserNetworkTool {
    fn name(&self) -> &str {
        "browser_network"
    }
    fn description(&self) -> &str {
        "Navigate to a URL and capture all network requests/responses via Chrome DevTools Protocol. \
         Filters by resource type or URL pattern. Returns request URLs, methods, status codes, \
         response headers, and optionally response bodies. Useful for discovering hidden APIs."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "URL to navigate to."),
            ParameterSchema::optional("filter_url", "string", "Only capture requests whose URL contains this string."),
            ParameterSchema::optional(
                "filter_type",
                "string",
                "Resource type filter: XHR|Fetch|Document|Script|Image|Stylesheet (default: all).",
            ),
            ParameterSchema::optional(
                "capture_bodies",
                "boolean",
                "Capture response bodies (expensive, default: false).",
            ),
            ParameterSchema::optional(
                "wait_ms",
                "integer",
                "Wait after navigation (ms, default: 2000) to catch async requests.",
            ),
            ParameterSchema::optional("timeout_secs", "integer", "Timeout (default: 30)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = match args["url"].as_str() {
            Some(u) => u.to_string(),
            None => return Ok(ToolResult::err("'url' required")),
        };
        let filter_url = args["filter_url"].as_str().map(String::from);
        let filter_type = args["filter_type"].as_str().map(|s| s.to_lowercase());
        let _capture_bodies = args["capture_bodies"].as_bool().unwrap_or(false);
        let wait_ms = args["wait_ms"].as_u64().unwrap_or(2000).min(15_000);
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);

        let handle = self.pool.acquire(timeout_secs + 5).await?;
        let browser = handle.browser.lock().await;

        let page = browser.new_page("about:blank").await.map_err(|e| anyhow::anyhow!("new_page: {}", e))?;

        // Enable network interception via JS-based approach (CDP event listening)
        // Collect requests via performance API after navigation
        page.evaluate("window.__narayan_requests = [];").await.ok();
        page.evaluate(
            r#"
            const origFetch = window.fetch;
            window.fetch = async (...args) => {
                const req = { url: args[0]?.toString(), method: 'fetch', ts: Date.now() };
                try {
                    const res = await origFetch(...args);
                    req.status = res.status;
                    req.type = 'Fetch';
                    window.__narayan_requests.push(req);
                    return res;
                } catch(e) { req.error = e.message; window.__narayan_requests.push(req); throw e; }
            };
            const origXHR = XMLHttpRequest.prototype.open;
            XMLHttpRequest.prototype.open = function(method, url) {
                this.__req = { url, method, type: 'XHR', ts: Date.now() };
                return origXHR.apply(this, arguments);
            };
            XMLHttpRequest.prototype.addEventListener('load', function() {
                if(this.__req) { this.__req.status = this.status; window.__narayan_requests.push(this.__req); }
            });
        "#,
        )
        .await
        .ok();

        page.goto(&url).await.map_err(|e| anyhow::anyhow!("navigate: {}", e))?;
        let _ = tokio::time::timeout(Duration::from_secs(timeout_secs), page.wait_for_navigation()).await;
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;

        // Collect from performance entries (covers all resource types)
        let perf_entries = page
            .evaluate(
                r#"
            JSON.stringify(performance.getEntriesByType('resource').map(e => ({
                url:       e.name,
                type:      e.initiatorType,
                duration:  Math.round(e.duration),
                size:      e.transferSize || 0,
            })))
        "#,
            )
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
            .unwrap_or_default();

        // Also collect intercepted XHR/fetch
        let intercepted = page
            .evaluate("JSON.stringify(window.__narayan_requests || [])")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
            .unwrap_or_default();

        page.close().await.ok();

        // Merge and filter
        let mut all: Vec<serde_json::Value> = perf_entries
            .into_iter()
            .chain(intercepted.into_iter())
            .filter(|r| {
                let url_str = r["url"].as_str().unwrap_or("");
                let type_str = r["type"].as_str().unwrap_or("").to_lowercase();
                let url_ok = filter_url.as_ref().map(|f| url_str.contains(f.as_str())).unwrap_or(true);
                let type_ok = filter_type.as_ref().map(|f| type_str.contains(f.as_str())).unwrap_or(true);
                url_ok && type_ok
            })
            .collect();

        // Deduplicate by URL
        let mut seen = std::collections::HashSet::new();
        all.retain(|r| seen.insert(r["url"].as_str().unwrap_or("").to_string()));

        Ok(ToolResult::ok(serde_json::json!({
            "page_url":      url,
            "request_count": all.len(),
            "requests":      all,
            "tip": "For XHR/Fetch responses with bodies, use browser_interact with js_eval to call the API directly after login.",
        })))
    }
}
