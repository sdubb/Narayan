//! Real MCP (Model Context Protocol) implementation.
//!
//! Implements the full JSON-RPC 2.0 over SSE transport as specified at
//! https://spec.modelcontextprotocol.io — compatible with Claude.ai MCP servers,
//! GitHub MCP, Slack MCP, Stripe MCP, Gmail MCP, and any compliant server.
//!
//! Protocol flow:
//!   1. POST /  → {"jsonrpc":"2.0","method":"initialize","id":1,...}
//!   2. POST /  → {"jsonrpc":"2.0","method":"notifications/initialized"}
//!   3. POST /  → {"jsonrpc":"2.0","method":"tools/list","id":2}
//!   4. POST /  → {"jsonrpc":"2.0","method":"tools/call","id":3,"params":{...}}

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::{ParameterSchema, Tool, ToolResult};

// ── JSON-RPC types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    method: String,
    id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    message: String,
    #[serde(default)]
    code: i64,
}

// ── MCP tool descriptor returned by tools/list ─────────────────────────────

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

// ── Shared request ID counter ──────────────────────────────────────────────

static REQ_ID: AtomicU64 = AtomicU64::new(1);
fn next_id() -> u64 {
    REQ_ID.fetch_add(1, Ordering::Relaxed)
}

// ── Core MCP client ────────────────────────────────────────────────────────

struct McpClient {
    server_url: String,
    auth_token: Option<String>,
    client: reqwest::Client,
}

impl McpClient {
    fn new(server_url: String, auth_token: Option<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;
        Ok(Self { server_url: server_url.trim_end_matches('/').to_string(), auth_token, client })
    }

    /// Send a JSON-RPC request to the MCP server and return the parsed response.
    async fn rpc(&self, method: &str, params: Option<Value>, id: Option<u64>) -> anyhow::Result<JsonRpcResponse> {
        let payload = JsonRpcRequest { jsonrpc: "2.0", method: method.to_string(), id, params };

        let mut req = self
            .client
            .post(&self.server_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(ref token) = self.auth_token {
            req = req.bearer_auth(token);
        }

        let resp = req.json(&payload).send().await.map_err(|e| anyhow::anyhow!("MCP request failed: {}", e))?;

        if !resp.status().is_success() && resp.status().as_u16() != 202 {
            anyhow::bail!("MCP server returned HTTP {}", resp.status());
        }

        let ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();

        // Handle SSE response — read first data: line
        if ct.contains("text/event-stream") {
            return self.read_sse_response(resp).await;
        }

        // Standard JSON response
        let body = resp.text().await?;
        if body.trim().is_empty() {
            return Ok(JsonRpcResponse { result: Some(Value::Null), error: None });
        }
        Ok(serde_json::from_str::<JsonRpcResponse>(&body).unwrap_or(JsonRpcResponse {
            result: Some(serde_json::from_str(&body).unwrap_or(Value::Null)),
            error: None,
        }))
    }

    /// Parse SSE stream — extract first `data:` line containing JSON-RPC response.
    async fn read_sse_response(&self, resp: reqwest::Response) -> anyhow::Result<JsonRpcResponse> {
        use futures_util::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| anyhow::anyhow!("SSE read error: {}", e))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Parse complete SSE events (terminated by \n\n)
            while let Some(event_end) = buffer.find("\n\n") {
                let event = buffer[..event_end].to_string();
                buffer = buffer[event_end + 2..].to_string();

                for line in event.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data.trim() == "[DONE]" {
                            continue;
                        }
                        if let Ok(rpc) = serde_json::from_str::<JsonRpcResponse>(data) {
                            return Ok(rpc);
                        }
                        // Might be a plain value
                        if let Ok(v) = serde_json::from_str::<Value>(data) {
                            return Ok(JsonRpcResponse { result: Some(v), error: None });
                        }
                    }
                }
            }
        }

        Ok(JsonRpcResponse { result: Some(Value::Null), error: None })
    }

    /// Step 1: Initialize the MCP session (required before any other call).
    async fn initialize(&self) -> anyhow::Result<Value> {
        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "roots":    { "listChanged": false },
                "sampling": {}
            },
            "clientInfo": {
                "name":    "narayan",
                "version": "2.0.0"
            }
        });

        let resp = self.rpc("initialize", Some(params), Some(next_id())).await?;
        if let Some(err) = resp.error {
            anyhow::bail!("MCP initialize failed: {} (code {})", err.message, err.code);
        }

        // Send initialized notification (no response expected)
        let _ = self.rpc("notifications/initialized", None, None).await;

        Ok(resp.result.unwrap_or(Value::Null))
    }

    /// Step 2: List all tools exposed by this MCP server.
    async fn list_tools(&self) -> anyhow::Result<Vec<McpToolInfo>> {
        let resp = self.rpc("tools/list", None, Some(next_id())).await?;
        if let Some(err) = resp.error {
            anyhow::bail!("tools/list failed: {}", err.message);
        }
        let tools: Vec<McpToolInfo> = resp
            .result
            .as_ref()
            .and_then(|v| v.get("tools"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(tools)
    }

    /// Step 3: Call a specific tool.
    async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        let params = serde_json::json!({
            "name":      name,
            "arguments": arguments,
        });
        let resp = self.rpc("tools/call", Some(params), Some(next_id())).await?;
        if let Some(err) = resp.error {
            anyhow::bail!("tools/call '{}' failed: {}", name, err.message);
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }
}

// ── Tool connector registry (in-memory) ───────────────────────────────────
// Stores discovered MCP tools so agents can call them by name later.

/// Map a known MCP server URL to the connector_type stored in connector_installs.
/// These must match exactly what users pass to POST /connectors/:type/install.
/// Google services (Gmail, Drive, Sheets, Docs, Calendar) all share one "google" install.
fn mcp_url_to_connector(server_url: &str) -> Option<&'static str> {
    let url = server_url.to_lowercase();
    // Google — all Google services share one OAuth install under "google"
    if url.contains("gmail.mcp")
        || url.contains("gcal.mcp")
        || url.contains("gdrive.mcp")
        || url.contains("googleapis.com")
    {
        return Some("google");
    }
    // Slack
    if url.contains("slack.mcp") || url.contains("slack.com/api") {
        return Some("slack");
    }
    // Notion
    if url.contains("notion.mcp") || url.contains("api.notion.com") {
        return Some("notion");
    }
    // Atlassian — Jira and Confluence share one OAuth install under "atlassian"
    if url.contains("atlassian.com") {
        return Some("atlassian");
    }
    // Salesforce
    if url.contains("salesforce.com") {
        return Some("salesforce");
    }
    // HubSpot
    if url.contains("hubapi.com") || url.contains("hubspot.mcp") {
        return Some("hubspot");
    }
    // GitHub
    if url.contains("github") || url.contains("githubcopilot.com") {
        return Some("github");
    }
    // Linear
    if url.contains("linear.app") {
        return Some("linear");
    }
    // Microsoft — Teams, Outlook, and Graph API share one OAuth install under "microsoft"
    if url.contains("graph.microsoft") || url.contains("microsoftonline") {
        return Some("microsoft");
    }
    // Stripe
    if url.contains("mcp.stripe.com") {
        return Some("stripe");
    }
    // Shopify
    if url.contains("mcp.shopify.com") {
        return Some("shopify");
    }
    None
}

fn connector_key(server_url: &str, tool_name: &str) -> String {
    format!(
        "mcp_connector:{}:{}",
        server_url.replace("https://", "").replace("http://", "").replace('/', "_"),
        tool_name
    )
}

// ── McpSessionTool ─────────────────────────────────────────────────────────

pub struct McpSessionTool {
    /// Optional connector install store — auto-injects stored tokens when
    /// server_url matches a connected MCP server. Set via with_install_store().
    install_store: Option<Arc<crate::connectors::ConnectorInstallStore>>,
}

impl McpSessionTool {
    pub fn new() -> Self {
        Self { install_store: None }
    }

    pub fn with_install_store(mut self, store: Arc<crate::connectors::ConnectorInstallStore>) -> Self {
        self.install_store = Some(store);
        self
    }

    /// Look up a stored access token for a given MCP server URL.
    /// Matches known server URLs to connector types.
    async fn stored_token(&self, server_url: &str, tenant_id: &str) -> Option<String> {
        let store = self.install_store.as_ref()?;
        // Map MCP server URL → connector type
        let connector_type = mcp_url_to_connector(server_url)?;
        let install = store.get(tenant_id, connector_type).await.ok()??;
        store.decrypt_token(&install)
    }
}

#[async_trait]
impl Tool for McpSessionTool {
    fn name(&self) -> &str {
        "mcp_session"
    }

    fn description(&self) -> &str {
        "Connect to any MCP (Model Context Protocol) server using the full JSON-RPC 2.0 protocol. \
         Supports initialize handshake, tool discovery, and tool execution. \
         Compatible with Claude.ai MCP servers (Gmail, GitHub, Slack, Stripe, etc.)."
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("server_url", "string", "MCP server endpoint URL."),
            ParameterSchema::required("action", "string", "Action: 'connect' | 'list_tools' | 'call_tool'"),
            ParameterSchema::optional("tool_name", "string", "Tool name to call (for call_tool)."),
            ParameterSchema::optional("tool_args", "object", "Tool arguments as JSON object (for call_tool)."),
            ParameterSchema::optional("auth_token", "string", "Bearer auth token (OAuth or API key)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let server_url = match args["server_url"].as_str() {
            Some(u) => u.to_string(),
            None => return Ok(ToolResult::err("'server_url' is required")),
        };
        let action = match args["action"].as_str() {
            Some(a) => a.to_string(),
            None => return Ok(ToolResult::err("'action' is required")),
        };

        // Auto-inject stored OAuth/API-key token if tenant has connected this MCP server.
        // Falls back to explicit auth_token arg, then unauthenticated.
        let tenant_id = args["tenant_id"].as_str().unwrap_or("");
        let auth_token = if let Some(explicit) = args["auth_token"].as_str() {
            Some(explicit.to_string())
        } else if !tenant_id.is_empty() {
            self.stored_token(&server_url, tenant_id).await
        } else {
            None
        };

        let mcp = match McpClient::new(server_url.clone(), auth_token) {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::err(format!("failed to build MCP client: {}", e))),
        };

        match action.as_str() {
            // ── connect: initialize + list tools ──────────────────────────
            "connect" => {
                let server_info = match mcp.initialize().await {
                    Ok(v) => v,
                    Err(e) => return Ok(ToolResult::err(format!("MCP initialize failed: {}", e))),
                };

                let tools = match mcp.list_tools().await {
                    Ok(t) => t,
                    Err(e) => return Ok(ToolResult::err(format!("tools/list failed: {}", e))),
                };

                // Register discovered tools in connector registry
                for tool in &tools {
                    let key = connector_key(&server_url, &tool.name);
                    let val = serde_json::json!({
                        "server_url":   server_url,
                        "tool_name":    tool.name,
                        "description":  tool.description,
                        "input_schema": tool.input_schema,
                    });
                    crate::tools::memory_store_internal::insert(key, val.to_string());
                }

                tracing::info!(
                    server   = %server_url,
                    tools    = tools.len(),
                    "MCP session connected"
                );

                Ok(ToolResult::ok(serde_json::json!({
                    "connected":    true,
                    "server":       server_url,
                    "server_info":  server_info,
                    "tool_count":   tools.len(),
                    "tools": tools.iter().map(|t| serde_json::json!({
                        "name":        t.name,
                        "description": t.description,
                    })).collect::<Vec<_>>(),
                })))
            }

            // ── list_tools: initialize then list ──────────────────────────
            "list_tools" => {
                if let Err(e) = mcp.initialize().await {
                    return Ok(ToolResult::err(format!("MCP initialize failed: {}", e)));
                }
                match mcp.list_tools().await {
                    Ok(tools) => Ok(ToolResult::ok(serde_json::json!({
                        "tools": tools,
                        "count": tools.len(),
                    }))),
                    Err(e) => Ok(ToolResult::err(format!("list_tools failed: {}", e))),
                }
            }

            // ── call_tool: initialize, then call a specific tool ──────────
            "call_tool" => {
                let tool_name = match args["tool_name"].as_str() {
                    Some(t) => t.to_string(),
                    None => return Ok(ToolResult::err("'tool_name' is required for action='call_tool'")),
                };
                let tool_args = args["tool_args"].clone();

                if let Err(e) = mcp.initialize().await {
                    return Ok(ToolResult::err(format!("MCP initialize failed: {}", e)));
                }

                tracing::info!(server = %server_url, tool = %tool_name, "MCP tool call");

                match mcp.call_tool(&tool_name, tool_args).await {
                    Ok(result) => Ok(ToolResult::ok(serde_json::json!({
                        "tool":   tool_name,
                        "result": result,
                    }))),
                    Err(e) => Ok(ToolResult::err(format!("MCP call_tool '{}' failed: {}", tool_name, e))),
                }
            }

            other => Ok(ToolResult::err(format!("Unknown action '{}'. Use: connect | list_tools | call_tool", other))),
        }
    }
}
