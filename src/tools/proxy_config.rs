use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct ProxyConfigTool;
#[async_trait]
impl Tool for ProxyConfigTool {
    fn name(&self) -> &str {
        "proxy_config"
    }
    fn description(&self) -> &str {
        "Configure HTTP proxy settings for outbound requests made by this agent."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("action", "string", "Action: 'set'|'clear'|'get'."),
            ParameterSchema::optional("proxy_url", "string", "Proxy URL, e.g. 'http://proxy.example.com:8080'."),
            ParameterSchema::optional("no_proxy", "string", "Comma-separated list of hosts to bypass proxy."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = match args["action"].as_str() {
            Some(a) => a,
            None => return Ok(ToolResult::err("'action' required")),
        };
        match action {
            "set" => {
                if let Some(url) = args["proxy_url"].as_str() {
                    crate::tools::memory_store_internal::insert("proxy:http".into(), url.to_string());
                    crate::tools::memory_store_internal::insert("proxy:https".into(), url.to_string());
                }
                if let Some(no_proxy) = args["no_proxy"].as_str() {
                    crate::tools::memory_store_internal::insert("proxy:no_proxy".into(), no_proxy.to_string());
                }
                Ok(ToolResult::ok(serde_json::json!({"configured": true})))
            }
            "clear" => {
                crate::tools::memory_store_internal::remove("proxy:http");
                crate::tools::memory_store_internal::remove("proxy:https");
                crate::tools::memory_store_internal::remove("proxy:no_proxy");
                Ok(ToolResult::ok(serde_json::json!({"cleared": true})))
            }
            "get" => {
                let http = crate::tools::memory_store_internal::get("proxy:http").map(|v| v.clone());
                let no_prx = crate::tools::memory_store_internal::get("proxy:no_proxy").map(|v| v.clone());
                Ok(ToolResult::ok(serde_json::json!({"http_proxy": http, "no_proxy": no_prx})))
            }
            other => Ok(ToolResult::err(format!("Unknown action: '{other}'"))),
        }
    }
}
