use async_trait::async_trait;
use scraper::{Html, Selector};

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_integer};

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }
    fn description(&self) -> &str {
        "Fetch the content of a URL. Returns extracted text by default (strips HTML tags). \
         Set 'raw' to true to get the full HTML."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ url, raw?, timeout?, headers? }. url is required; raw returns HTML instead of extracted text.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ text | html, title, url, status, content_type, char_count }. Non-2xx HTTP statuses return success=false.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use when the exact URL is known and you need the page content or metadata.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when you need search/discovery first or when a browser interaction flow is required.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "URL to fetch."),
            ParameterSchema::optional("raw", "boolean", "Return raw HTML instead of extracted text (default: false)."),
            ParameterSchema::optional("timeout", "integer", "Request timeout in seconds (default: 30)."),
            ParameterSchema::optional("headers", "object", "Additional HTTP headers as key-value pairs."),
        ]
    }


    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["html", "url", "status", "content_type"],
                    "properties": {
                        "html": schema_string(),
                        "url": schema_string(),
                        "status": schema_integer(),
                        "content_type": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["text", "title", "url", "status", "content_type", "char_count"],
                    "properties": {
                        "text": schema_string(),
                        "title": schema_string(),
                        "url": schema_string(),
                        "status": schema_integer(),
                        "content_type": schema_string(),
                        "char_count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["status", "url"],
                    "properties": {
                        "status": schema_integer(),
                        "url": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = match args["url"].as_str() {
            Some(u) => u,
            None => return Ok(ToolResult::err("'url' is required")),
        };
        let timeout = args["timeout"].as_u64().unwrap_or(30);
        let raw = args["raw"].as_bool().unwrap_or(false);

        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout))
            .user_agent("Narayan/1.0 (+https://narayan.ai)")
            .build()?
            .get(url);

        if let Some(headers) = args["headers"].as_object() {
            for (k, v) in headers {
                if let Some(val) = v.as_str() {
                    builder = builder.header(k.as_str(), val);
                }
            }
        }

        let resp = match builder.send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("fetch failed: {e}"))),
        };

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let content_type =
            resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("unknown").to_string();

        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::err(format!("read body failed: {e}"))),
        };

        if status >= 400 {
            return Ok(ToolResult {
                success: false,
                output: serde_json::json!({"status": status, "url": final_url}),
                error: Some(format!("HTTP {}", status)),
            });
        }

        if raw {
            return Ok(ToolResult::ok(serde_json::json!({
                "html":         body,
                "url":          final_url,
                "status":       status,
                "content_type": content_type,
            })));
        }

        // Extract text from HTML
        let text = extract_text_from_html(&body);
        let title = extract_title(&body);
        Ok(ToolResult::ok(serde_json::json!({
            "text":         crate::util::truncate(&text, 8000),
            "title":        title,
            "url":          final_url,
            "status":       status,
            "content_type": content_type,
            "char_count":   text.len(),
        })))
    }
}

fn extract_text_from_html(html: &str) -> String {
    let doc = Html::parse_document(html);
    // Remove script/style elements first (by collecting their text selectively)
    let body_sel = Selector::parse("body").ok();
    let mut text = String::new();
    if let Some(sel) = body_sel {
        for el in doc.select(&sel) {
            for node in el.text() {
                let t = node.trim();
                if !t.is_empty() {
                    text.push_str(t);
                    text.push(' ');
                }
            }
        }
    }
    if text.is_empty() {
        doc.root_element().text().collect::<Vec<_>>().join(" ")
    } else {
        text
    }
}

fn extract_title(html: &str) -> String {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("title").ok();
    sel.and_then(|s| doc.select(&s).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;
    use crate::tools::Tool;

    async fn spawn_server(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have address");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("connection should arrive");
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf).await;
            stream.write_all(response.as_bytes()).await.expect("response should write");
        });

        format!("http://{}", addr)
    }

    fn http_response(status_line: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn test_execute_extracts_title_and_visible_text_from_html() {
        let tool = WebFetchTool;
        let body = "<html><head><title>Example</title></head><body>Hello <b>world</b></body></html>";
        let url = spawn_server(http_response("200 OK", "text/html; charset=utf-8", body)).await;

        let result = tool.execute(serde_json::json!({ "url": url, "timeout": 5 })).await.expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["title"], serde_json::json!("Example"));
        assert_eq!(result.output["status"], serde_json::json!(200));
        assert!(result.output["text"].as_str().unwrap_or_default().contains("Hello world"));
        assert!(result.output["char_count"].as_u64().unwrap_or_default() >= 11);
    }

    #[tokio::test]
    async fn test_execute_returns_raw_html_when_requested() {
        let tool = WebFetchTool;
        let body = "<html><body><p>Raw body</p></body></html>";
        let url = spawn_server(http_response("200 OK", "text/html", body)).await;

        let result = tool
            .execute(serde_json::json!({ "url": url, "raw": true, "timeout": 5 }))
            .await
            .expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["html"], serde_json::json!(body));
        assert_eq!(result.output["status"], serde_json::json!(200));
    }

    #[tokio::test]
    async fn test_execute_surfaces_http_error_status_with_metadata() {
        let tool = WebFetchTool;
        let url = spawn_server(http_response("503 Service Unavailable", "text/plain", "temporarily unavailable")).await;

        let result = tool.execute(serde_json::json!({ "url": url, "timeout": 5 })).await.expect("tool should execute");

        assert!(!result.success);
        assert_eq!(result.output["status"], serde_json::json!(503));
        assert!(result.error.unwrap_or_default().contains("HTTP 503"));
    }
}
