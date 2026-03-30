use std::collections::HashMap;

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct HttpRequestTool;

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }
    fn description(&self) -> &str {
        "Make an HTTP request to any URL. Supports GET, POST, PUT, PATCH, DELETE."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ url, method?, headers?, body?, json?, timeout?, follow_redirects? }. url is required.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ status, headers, body }. Non-2xx responses return success=false with the response payload preserved.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use for direct API calls or HTTP interactions when no dedicated connector exists.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when a dedicated API tool or connector is already available, or when the exact URL is not known.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "Request URL."),
            ParameterSchema::optional("method", "string", "HTTP method: GET|POST|PUT|PATCH|DELETE (default: GET)."),
            ParameterSchema::optional("headers", "object", "Request headers as key-value pairs."),
            ParameterSchema::optional("body", "string", "Request body (for POST/PUT/PATCH)."),
            ParameterSchema::optional(
                "json",
                "object",
                "JSON body — sets Content-Type: application/json automatically.",
            ),
            ParameterSchema::optional("timeout", "integer", "Timeout seconds (default: 30)."),
            ParameterSchema::optional("follow_redirects", "boolean", "Follow redirects (default: true)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = match args["url"].as_str() {
            Some(u) => u,
            None => return Ok(ToolResult::err("'url' is required")),
        };
        let method = args["method"].as_str().unwrap_or("GET").to_uppercase();
        let timeout = args["timeout"].as_u64().unwrap_or(30);
        let follow = args["follow_redirects"].as_bool().unwrap_or(true);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout))
            .redirect(if follow { reqwest::redirect::Policy::limited(10) } else { reqwest::redirect::Policy::none() })
            .build()?;

        let mut req = match method.as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
            other => return Ok(ToolResult::err(format!("Unknown method: '{other}'"))),
        };

        if let Some(headers) = args["headers"].as_object() {
            for (k, v) in headers {
                if let Some(val) = v.as_str() {
                    req = req.header(k.as_str(), val);
                }
            }
        }

        if !args["json"].is_null() {
            req = req.json(&args["json"]);
        } else if let Some(body) = args["body"].as_str() {
            req = req.body(body.to_string());
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::err(format!("request failed: {e}"))),
        };
        let status = resp.status().as_u16();
        let headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();
        let body = resp.text().await.unwrap_or_default();
        let ok = (200..300).contains(&status);
        let payload = serde_json::json!({
            "status":  status,
            "headers": headers,
            "body":    crate::util::truncate(&body, 8000),
        });
        if ok {
            Ok(ToolResult::ok(payload))
        } else {
            Ok(ToolResult { success: false, output: payload, error: Some(format!("HTTP {status}")) })
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;
    use crate::tools::Tool;

    async fn spawn_test_server(response: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener should bind");
        let addr = listener.local_addr().expect("local addr should be available");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("server should accept one client");
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            stream.write_all(response.as_bytes()).await.expect("response should write");
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_execute_requires_url() {
        let tool = HttpRequestTool;
        let result = tool.execute(serde_json::json!({})).await.expect("tool should return result");

        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("'url' is required"));
    }

    #[tokio::test]
    async fn test_execute_rejects_unknown_method() {
        let tool = HttpRequestTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "http://example.com",
                "method": "TRACEPLUS"
            }))
            .await
            .expect("tool should return result");

        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("Unknown method: 'TRACEPLUS'"));
    }

    #[tokio::test]
    async fn test_execute_performs_successful_get_request() {
        let url =
            spawn_test_server("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 12\r\n\r\nhello world!")
                .await;
        let tool = HttpRequestTool;

        let result = tool
            .execute(serde_json::json!({
                "url": url,
                "method": "GET",
                "follow_redirects": false
            }))
            .await
            .expect("tool should execute request");

        assert!(result.success);
        assert_eq!(result.output["status"], 200);
        assert_eq!(result.output["body"], "hello world!");
        assert_eq!(result.output["headers"]["content-type"], "text/plain");
    }

    #[tokio::test]
    async fn test_execute_returns_failed_tool_result_for_http_error_status() {
        let url = spawn_test_server(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 19\r\n\r\nservice unavailable",
        )
        .await;
        let tool = HttpRequestTool;

        let result = tool
            .execute(serde_json::json!({
                "url": url,
                "method": "GET"
            }))
            .await
            .expect("tool should execute request");

        assert!(!result.success);
        assert_eq!(result.output["status"], 503);
        assert_eq!(result.output["body"], "service unavailable");
        assert_eq!(result.error.as_deref(), Some("HTTP 503"));
    }
}
