//! `external_api` — call the user's own REST API.
//!
//! Lets agents interact with any HTTP API that the tenant has registered.
//! API definitions are stored in `tenant_connectors` as `ConnectorSource::Manual`
//! or `ConnectorSource::ApiDocs`.
//!
//! ## How it works
//!
//! The tenant registers their API:
//!   Name:      acme_backend
//!   Base URL:  https://api.acme.com/v2
//!   Auth:      bearer  (stored in connector_installs)
//!   Endpoints: (auto-discovered from OpenAPI or entered manually)
//!
//! The LLM calls:
//!   external_api {
//!     "api":     "acme_backend",
//!     "method":  "GET",
//!     "path":    "/orders",
//!     "params":  { "status": "pending", "limit": 20 }
//!   }
//!
//! The executor looks up the API definition from tenant_connectors,
//! fetches the stored token, and makes the real HTTP request.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub const TOOL_NAME: &str = "external_api";

pub struct ExternalApiTool {
    http:          reqwest::Client,
    install_store: Option<Arc<crate::connectors::ConnectorInstallStore>>,
    store:         Option<Arc<crate::storage::PostgresStore>>,
}

impl ExternalApiTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            install_store: None,
            store: None,
        }
    }

    pub fn with_stores(
        mut self,
        install_store: Arc<crate::connectors::ConnectorInstallStore>,
        store: Arc<crate::storage::PostgresStore>,
    ) -> Self {
        self.install_store = Some(install_store);
        self.store = Some(store);
        self
    }

    async fn load_api(&self, tenant_id: &str, api_name: &str)
        -> Result<(String, String, Option<String>), String>
    {
        // Load the connector definition (base_url, auth_type)
        let store = self.store.as_ref()
            .ok_or_else(|| "Store not configured".to_string())?;

        let tc = store.get_tenant_connector(tenant_id, api_name).await
            .map_err(|e| format!("Connector lookup failed: {e}"))?
            .ok_or_else(|| format!(
                "API '{api_name}' not found. Register it in Settings → Connections → REST APIs."
            ))?;

        // Load stored token
        let token = if let Some(inst) = self.install_store.as_ref() {
            inst.get(tenant_id, api_name).await
                .ok()
                .flatten()
                .and_then(|i| inst.decrypt_token(&i))
        } else {
            None
        };

        // Determine auth header name from auth_type
        let auth_header = match &tc.auth_type {
            crate::agent::definition::ConnectorAuthType::ApiKeyHeader { header_name } =>
                header_name.clone(),
            _ => "Authorization".to_string(),
        };

        Ok((tc.base_url.clone(), auth_header, token))
    }
}

#[async_trait]
impl Tool for ExternalApiTool {
    fn name(&self) -> &str { TOOL_NAME }

    fn description(&self) -> &str {
        "Call an endpoint on an external REST API that has been registered by the tenant. \
         Use 'api' to name which registered API to use, 'method' for the HTTP verb (GET/POST/PUT/PATCH/DELETE), \
         'path' for the endpoint path relative to the base URL, 'params' for query parameters or request body. \
         The API's authentication is handled automatically using stored credentials."
    }

    fn category(&self) -> &'static str { "integration" }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "api",
                "string",
                "Name of the registered API (as set in Settings → Connections → REST APIs).",
            ),
            ParameterSchema::required(
                "method",
                "string",
                "HTTP method: GET, POST, PUT, PATCH, DELETE.",
            ),
            ParameterSchema::required(
                "path",
                "string",
                "Endpoint path relative to the API base URL, e.g. '/orders' or '/users/123'.",
            ),
            ParameterSchema::optional(
                "params",
                "object",
                "For GET: query parameters. For POST/PUT/PATCH: request body as JSON object.",
            ),
            ParameterSchema::optional(
                "headers",
                "object",
                "Additional HTTP headers to include (auth headers are added automatically).",
            ),
            ParameterSchema::optional(
                "tenant_id",
                "string",
                "Injected by executor — tenant for credential lookup.",
            ),
        ]
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let api_name  = args["api"].as_str().unwrap_or("").to_string();
        let method    = args["method"].as_str().unwrap_or("GET").to_uppercase();
        let path      = args["path"].as_str().unwrap_or("").to_string();
        let tenant_id = args["tenant_id"].as_str().unwrap_or("").to_string();
        let params    = args.get("params").cloned().unwrap_or_default();
        let headers   = args.get("headers").and_then(|v| v.as_object()).cloned();

        if api_name.is_empty() { return Ok(ToolResult::err("'api' is required")); }
        if path.is_empty()     { return Ok(ToolResult::err("'path' is required")); }

        let (base_url, auth_header, token) =
            match self.load_api(&tenant_id, &api_name).await {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult::err(e)),
            };

        // Build full URL
        let full_url = format!("{}{}", base_url.trim_end_matches('/'), path);

        // Build request
        let mut req = match method.as_str() {
            "GET"    => self.http.get(&full_url),
            "POST"   => self.http.post(&full_url),
            "PUT"    => self.http.put(&full_url),
            "PATCH"  => self.http.patch(&full_url),
            "DELETE" => self.http.delete(&full_url),
            other    => return Ok(ToolResult::err(format!("Unknown HTTP method '{other}'"))),
        };

        // Auth
        if let Some(ref tok) = token {
            req = match auth_header.as_str() {
                "Authorization" => req.bearer_auth(tok),
                other           => req.header(other, tok),
            };
        }

        // Extra headers
        if let Some(hmap) = headers {
            for (k, v) in &hmap {
                if let Some(s) = v.as_str() {
                    req = req.header(k.as_str(), s);
                }
            }
        }

        // Params / body
        req = if method == "GET" || method == "DELETE" {
            if let Some(obj) = params.as_object() {
                let qp: Vec<(String, String)> = obj.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect();
                req.query(&qp)
            } else {
                req
            }
        } else {
            req.json(&params)
        };

        // Execute
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            req.send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Request timed out"))?
        .map_err(|e| anyhow::anyhow!("Request failed: {e}"))?;

        let status = resp.status();
        let status_u16 = status.as_u16();

        // Parse response
        let body: Value = resp.json().await
            .unwrap_or_else(|_| Value::String(String::new()));

        if status.is_success() {
            Ok(ToolResult::ok(serde_json::json!({
                "status":  status_u16,
                "success": true,
                "data":    body,
                "url":     full_url,
            })))
        } else {
            Ok(ToolResult::err(format!(
                "API returned {status_u16}: {}",
                body.to_string().chars().take(500).collect::<String>()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name() {
        let t = ExternalApiTool::new();
        assert_eq!(t.name(), TOOL_NAME);
        assert_eq!(t.category(), "integration");
    }

    #[tokio::test]
    async fn test_missing_api_name() {
        let t = ExternalApiTool::new();
        let r = t.execute(serde_json::json!({"api": "", "method": "GET", "path": "/test"})).await.unwrap();
        assert!(!r.success);
    }

    #[tokio::test]
    async fn test_missing_path() {
        let t = ExternalApiTool::new();
        let r = t.execute(serde_json::json!({"api": "test", "method": "GET", "path": ""})).await.unwrap();
        assert!(!r.success);
    }
}
