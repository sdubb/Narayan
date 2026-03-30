use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

/// Opens a URL and returns a confirmation (delegates actual content retrieval to browser/web_fetch).
pub struct BrowserOpenTool;

#[async_trait]
impl Tool for BrowserOpenTool {
    fn name(&self) -> &str {
        "browser_open"
    }
    fn description(&self) -> &str {
        "Open a URL and verify it is reachable. Returns the final URL after redirects and the HTTP status."
    }
    fn input_contract(&self) -> Option<String> {
        Some("{ url, timeout? }. url is required.".into())
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ url, status, reachable }. Confirms reachability and final URL after redirects.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some("Use to quickly verify that a URL is reachable before fetching or browser automation.".into())
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Avoid when you need the page body, browser interactions, or search/discovery first.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "URL to open."),
            ParameterSchema::optional("timeout", "integer", "Timeout seconds (default: 15)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = match args["url"].as_str() {
            Some(u) => u,
            None => return Ok(ToolResult::err("'url' is required")),
        };
        let timeout = args["timeout"].as_u64().unwrap_or(15);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout))
            .user_agent("Mozilla/5.0 (compatible; Narayan/1.0)")
            .build()?;
        match client.head(url).send().await {
            Ok(resp) => Ok(ToolResult::ok(serde_json::json!({
                "url":       resp.url().to_string(),
                "status":    resp.status().as_u16(),
                "reachable": resp.status().is_success() || resp.status().is_redirection(),
            }))),
            Err(e) => Ok(ToolResult::err(format!("cannot reach '{}': {}", url, e))),
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

    async fn spawn_server(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener should bind");
        let addr = listener.local_addr().expect("listener should have address");

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("connection should arrive");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            stream.write_all(response.as_bytes()).await.expect("response should write");
        });

        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_execute_requires_url() {
        let tool = BrowserOpenTool;

        let result = tool.execute(serde_json::json!({})).await.expect("tool should execute");

        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("'url' is required"));
    }

    #[tokio::test]
    async fn test_execute_reports_reachable_success_status() {
        let tool = BrowserOpenTool;
        let url = spawn_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_string()).await;

        let result = tool.execute(serde_json::json!({ "url": url, "timeout": 5 })).await.expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["status"], serde_json::json!(200));
        assert_eq!(result.output["reachable"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn test_execute_marks_non_success_status_as_unreachable() {
        let tool = BrowserOpenTool;
        let url = spawn_server("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string()).await;

        let result = tool.execute(serde_json::json!({ "url": url, "timeout": 5 })).await.expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["status"], serde_json::json!(404));
        assert_eq!(result.output["reachable"], serde_json::json!(false));
    }
}
