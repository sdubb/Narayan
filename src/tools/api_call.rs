use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct ApiCallTool;
#[async_trait]
impl Tool for ApiCallTool {
    fn name(&self) -> &str {
        "api_call"
    }
    fn description(&self) -> &str {
        "Make an authenticated API call using a stored credential. Combines with request_credential for secure key management."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("url", "string", "API endpoint URL."),
            ParameterSchema::required("credential_key", "string", "Key name of the stored credential to use for auth."),
            ParameterSchema::optional("method", "string", "HTTP method (default: GET)."),
            ParameterSchema::optional(
                "auth_type",
                "string",
                "Auth type: 'bearer'|'api_key_header'|'basic' (default: bearer).",
            ),
            ParameterSchema::optional(
                "auth_header_name",
                "string",
                "Header name for api_key_header auth (default: X-API-Key).",
            ),
            ParameterSchema::optional("body", "object", "JSON request body."),
            ParameterSchema::optional("headers", "object", "Additional headers."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = match args["url"].as_str() {
            Some(u) => u,
            None => return Ok(ToolResult::err("'url' required")),
        };
        let cred_key = match args["credential_key"].as_str() {
            Some(k) => k,
            None => return Ok(ToolResult::err("'credential_key' required")),
        };
        // Retrieve credential from memory store
        let full_key = format!("credential:{cred_key}");
        let cred_val = match crate::tools::memory_store_internal::get(&full_key) {
            Some(v) => v.clone(),
            None => {
                return Ok(ToolResult::err(format!(
                    "Credential '{}' not found. Use request_credential to store it first.",
                    cred_key
                )))
            }
        };
        let method = args["method"].as_str().unwrap_or("GET").to_uppercase();
        let auth_type = args["auth_type"].as_str().unwrap_or("bearer");
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        let mut req = match method.as_str() {
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
            _ => client.get(url),
        };
        req = match auth_type {
            "bearer" => req.bearer_auth(&cred_val),
            "api_key_header" => {
                let hname = args["auth_header_name"].as_str().unwrap_or("X-API-Key");
                req.header(hname, &cred_val)
            }
            "basic" => req.basic_auth(&cred_key, Some(&cred_val)),
            _ => req.bearer_auth(&cred_val),
        };
        if let Some(headers) = args["headers"].as_object() {
            for (k, v) in headers {
                if let Some(val) = v.as_str() {
                    req = req.header(k.as_str(), val);
                }
            }
        }
        if !args["body"].is_null() {
            req = req.json(&args["body"]);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                let ok = (200..300).contains(&status);
                let out = serde_json::json!({"status": status, "body": crate::util::truncate(&body, 8000)});
                if ok {
                    Ok(ToolResult::ok(out))
                } else {
                    Ok(ToolResult { success: false, output: out, error: Some(format!("HTTP {status}")) })
                }
            }
            Err(e) => Ok(ToolResult::err(format!("API call failed: {e}"))),
        }
    }
}
