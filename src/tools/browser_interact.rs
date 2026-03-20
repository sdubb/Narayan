//! browser_interact — Multi-step browser automation with real Chromium.
//!
//! Supports: click, fill form fields, submit, scroll, select dropdown,
//! wait for element, evaluate JS, extract after interaction, hover, press key.
//! All actions run in sequence on the same page — enables full login flows,
//! form submissions, and multi-step UI automation.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;

use crate::{
    browser::BrowserPool,
    tools::{ParameterSchema, Tool, ToolResult},
};

pub struct BrowserInteractTool {
    pub pool: Arc<BrowserPool>,
}

/// Single browser action step.
#[derive(serde::Deserialize, Debug)]
struct Action {
    #[serde(rename = "type")]
    kind: String,
    selector: Option<String>,
    value: Option<String>,
    key: Option<String>,
    ms: Option<u64>,
    script: Option<String>,
}

#[async_trait]
impl Tool for BrowserInteractTool {
    fn name(&self) -> &str {
        "browser_interact"
    }
    fn description(&self) -> &str {
        "Execute a sequence of browser actions on a page using real headless Chromium. \
         Supports: navigate, click, fill, submit, select, scroll, wait, key_press, \
         hover, js_eval, screenshot. Enables login flows, form submissions, and complex \
         multi-step UI automation. Returns final page state after all actions."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "Starting URL."),
            ParameterSchema::required(
                "actions",
                "array",
                "Sequence of actions to perform. Each: {type, selector?, value?, key?, ms?, script?}",
            ),
            ParameterSchema::optional("timeout_secs", "integer", "Per-action timeout seconds (default: 15)."),
            ParameterSchema::optional(
                "cookies",
                "array",
                "Cookies to inject before starting: [{name, value, domain}].",
            ),
            ParameterSchema::optional(
                "screenshot_after",
                "boolean",
                "Take screenshot after all actions (default: false).",
            ),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        use chromiumoxide::cdp::browser_protocol::network::CookieParam;

        let url = match args["url"].as_str() {
            Some(u) => u.to_string(),
            None => return Ok(ToolResult::err("'url' required")),
        };
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(15);
        let screenshot = args["screenshot_after"].as_bool().unwrap_or(false);

        let actions: Vec<Action> = match serde_json::from_value(args["actions"].clone()) {
            Ok(a) => a,
            Err(e) => return Ok(ToolResult::err(format!("invalid actions: {}", e))),
        };

        let handle = self.pool.acquire(timeout_secs + 10).await?;
        let browser = handle.browser.lock().await;

        let page = browser.new_page(&url).await.map_err(|e| anyhow::anyhow!("new_page: {}", e))?;

        // Inject cookies
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

        let _ = tokio::time::timeout(Duration::from_secs(timeout_secs), page.wait_for_navigation()).await;

        let mut action_log: Vec<serde_json::Value> = Vec::new();

        // Execute each action
        for action in &actions {
            let result = tokio::time::timeout(Duration::from_secs(timeout_secs), execute_action(&page, action)).await;

            match result {
                Ok(Ok(entry)) => action_log.push(entry),
                Ok(Err(e)) => {
                    action_log.push(serde_json::json!({
                        "type":    &action.kind,
                        "success": false,
                        "error":   e.to_string(),
                    }));
                    // Continue on non-fatal errors
                    tracing::warn!(action = ?action.kind, error = %e, "browser action failed");
                }
                Err(_) => {
                    action_log.push(serde_json::json!({
                        "type":    &action.kind,
                        "success": false,
                        "error":   format!("timed out after {}s", timeout_secs),
                    }));
                }
            }
        }

        // Final page state
        let title = page.get_title().await.unwrap_or_default().unwrap_or_default();
        let final_url = page.url().await.unwrap_or_default().unwrap_or_default();
        let text = page
            .evaluate("document.body?.innerText?.slice(0, 5000) || ''")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .unwrap_or_default();

        // Optional screenshot
        let screenshot_b64: Option<String> = if screenshot {
            let params = chromiumoxide::page::ScreenshotParams::builder()
                .format(chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png)
                .build();
            page.screenshot(params).await.ok().map(|b| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&b)
            })
        } else {
            None
        };

        page.close().await.ok();

        Ok(ToolResult::ok(serde_json::json!({
            "final_url":      final_url,
            "title":          title,
            "text_preview":   text,
            "actions_taken":  action_log,
            "screenshot_b64": screenshot_b64,
        })))
    }
}

async fn execute_action(page: &chromiumoxide::Page, action: &Action) -> anyhow::Result<serde_json::Value> {
    let sel = action.selector.as_deref().unwrap_or("body");

    match action.kind.as_str() {
        "click" => {
            page.find_element(sel)
                .await
                .map_err(|e| anyhow::anyhow!("find '{}': {}", sel, e))?
                .click()
                .await
                .map_err(|e| anyhow::anyhow!("click: {}", e))?;
            Ok(serde_json::json!({"type": "click", "selector": sel, "success": true}))
        }

        "fill" => {
            let val = action.value.as_deref().unwrap_or("");
            let el = page.find_element(sel).await.map_err(|e| anyhow::anyhow!("find '{}': {}", sel, e))?;
            el.click().await?;
            // Clear then type
            page.evaluate(format!("document.querySelector({:?}).value = ''", sel)).await.ok();
            el.type_str(val).await.map_err(|e| anyhow::anyhow!("type: {}", e))?;
            Ok(serde_json::json!({"type": "fill", "selector": sel, "value": val, "success": true}))
        }

        "submit" => {
            page.find_element(sel).await.map_err(|e| anyhow::anyhow!("find '{}': {}", sel, e))?.click().await?;
            let _ = tokio::time::timeout(Duration::from_secs(15), page.wait_for_navigation()).await;
            Ok(serde_json::json!({"type": "submit", "success": true}))
        }

        "select" => {
            let val = action.value.as_deref().unwrap_or("");
            page.evaluate(format!(
                "let s=document.querySelector({:?}); if(s){{s.value={:?}; s.dispatchEvent(new Event('change',{{bubbles:true}}))}}",
                sel, val
            )).await?;
            Ok(serde_json::json!({"type": "select", "selector": sel, "value": val, "success": true}))
        }

        "wait" => {
            let ms = action.ms.unwrap_or(1000).min(10_000);
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok(serde_json::json!({"type": "wait", "ms": ms, "success": true}))
        }

        "wait_for" => {
            // Poll for selector to appear
            for _ in 0..30 {
                let found = page
                    .evaluate(format!("!!document.querySelector({:?})", sel))
                    .await
                    .ok()
                    .and_then(|v| v.into_value::<bool>().ok())
                    .unwrap_or(false);
                if found {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Ok(serde_json::json!({"type": "wait_for", "selector": sel, "success": true}))
        }

        "scroll" => {
            let y = action.value.as_deref().unwrap_or("500").parse::<i64>().unwrap_or(500);
            page.evaluate(format!("window.scrollBy(0, {})", y)).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(serde_json::json!({"type": "scroll", "y": y, "success": true}))
        }

        "scroll_to_bottom" => {
            page.evaluate("window.scrollTo(0, document.body.scrollHeight)").await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(serde_json::json!({"type": "scroll_to_bottom", "success": true}))
        }

        "hover" => {
            page.find_element(sel)
                .await
                .map_err(|e| anyhow::anyhow!("find '{}': {}", sel, e))?
                .hover()
                .await
                .map_err(|e| anyhow::anyhow!("hover: {}", e))?;
            Ok(serde_json::json!({"type": "hover", "selector": sel, "success": true}))
        }

        "key_press" => {
            let key = action.key.as_deref().unwrap_or("Enter");
            // Use CDP Input.dispatchKeyEvent via JS since Page has no press_key
            page.evaluate(format!(
                "document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', {{key: {:?}, bubbles: true}})); \
                 document.activeElement?.dispatchEvent(new KeyboardEvent('keyup', {{key: {:?}, bubbles: true}}))",
                key, key
            ))
            .await
            .map_err(|e| anyhow::anyhow!("key_press '{}': {}", key, e))?;
            Ok(serde_json::json!({"type": "key_press", "key": key, "success": true}))
        }

        "js_eval" => {
            let script = action.script.as_deref().unwrap_or("null");
            let result = page.evaluate(script).await.map_err(|e| anyhow::anyhow!("js_eval: {}", e))?;
            let val = result.into_value::<serde_json::Value>().ok();
            Ok(serde_json::json!({"type": "js_eval", "result": val, "success": true}))
        }

        "navigate" => {
            let target_url = action.value.as_deref().unwrap_or("");
            page.goto(target_url).await.map_err(|e| anyhow::anyhow!("navigate '{}': {}", target_url, e))?;
            let _ = tokio::time::timeout(Duration::from_secs(15), page.wait_for_navigation()).await;
            Ok(serde_json::json!({"type": "navigate", "url": target_url, "success": true}))
        }

        "extract" => {
            let text = page
                .evaluate(format!(
                    "(document.querySelector({:?}) || document.body)?.innerText?.slice(0, 5000) || ''",
                    sel
                ))
                .await
                .ok()
                .and_then(|v| v.into_value::<String>().ok())
                .unwrap_or_default();
            Ok(serde_json::json!({"type": "extract", "selector": sel, "text": text, "success": true}))
        }

        other => Err(anyhow::anyhow!(
            "unknown action type '{}'. Valid: click, fill, submit, select, wait, wait_for, scroll, \
             scroll_to_bottom, hover, key_press, js_eval, navigate, extract",
            other
        )),
    }
}
