use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Core types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(output: serde_json::Value) -> Self {
        Self { success: true, output, error: None }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self { success: false, output: serde_json::Value::Null, error: Some(msg.into()) }
    }
    pub fn text(s: impl Into<String>) -> Self {
        Self::ok(serde_json::json!({ "text": s.into() }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
    pub required: bool,
}

impl ParameterSchema {
    pub fn required(name: &str, param_type: &str, description: &str) -> Self {
        Self { name: name.into(), param_type: param_type.into(), description: description.into(), required: true }
    }
    pub fn optional(name: &str, param_type: &str, description: &str) -> Self {
        Self { name: name.into(), param_type: param_type.into(), description: description.into(), required: false }
    }
}

// ── Tool trait ─────────────────────────────────────────────────────────────

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Vec<ParameterSchema>;
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;

    /// Category this tool belongs to.  Used by the selector and by
    /// `request_more_tools` to let the LLM ask for a whole category at once.
    ///
    /// Convention: use slash-namespaced strings —
    ///   "filesystem", "web", "code", "data", "memory",
    ///   "infra", "integration", "communication", "security",
    ///   "automation", "connector/crm", "connector/devtools",
    ///   "connector/project_management", "connector/communication",
    ///   "connector/finance", "connector/hr", "connector/itsm", "other"
    fn category(&self) -> &'static str {
        "other"
    }
}

// ── Registry ───────────────────────────────────────────────────────────────

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }
    pub fn list(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.tools.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// All tool names grouped by their declared category.
    pub fn by_category(&self) -> std::collections::BTreeMap<&str, Vec<&str>> {
        let mut map: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        for (name, tool) in &self.tools {
            map.entry(tool.category()).or_default().push(name.as_str());
        }
        for names in map.values_mut() {
            names.sort_unstable();
        }
        map
    }

    /// Full ToolSpec list for every tool in the given category.
    /// Used by the `request_more_tools` meta-tool to expand the executor's toolset.
    pub fn tool_specs_for_category(&self, category: &str) -> Vec<crate::providers::ToolSpec> {
        self.tools
            .values()
            .filter(|t| t.category() == category)
            .map(|t| crate::providers::ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": t.parameters_schema().iter().fold(
                        serde_json::Map::new(),
                        |mut acc, p| {
                            acc.insert(p.name.clone(), serde_json::json!({
                                "type":        p.param_type,
                                "description": p.description,
                            }));
                            acc
                        }
                    ),
                    "required": t.parameters_schema().iter()
                        .filter(|p| p.required)
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>(),
                }),
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Sub-modules ────────────────────────────────────────────────────────────

pub mod connector_meta {
    pub use super::create_custom_connector::CreateCustomConnectorTool;
    pub use super::list_connectors_in_category::ListConnectorsInCategoryTool;
    pub use super::request_more_connectors::RequestMoreConnectorsTool;
    pub use super::request_more_tools::RequestMoreToolsTool;
}

pub mod connector_tool;
pub mod create_custom_connector;
pub mod credential_requirements;
pub mod list_connectors_in_category;
pub mod memory_store_internal;
pub mod request_more_connectors;
pub mod request_more_tools;
pub mod selector;

pub mod acp_session;
pub mod api_call;
pub mod ask_user;
pub mod browser;
pub mod browser_interact;
pub mod browser_network;
pub mod browser_open;
pub mod browser_pdf;
pub mod code_run;
pub mod compress;
pub mod content_search;
pub mod cron;
pub mod crypto_tool;
pub mod data_extractor;
pub mod delegate;
pub mod diff_patch;
pub mod docker;
pub mod email;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod git_operations;
pub mod external_api;
pub mod external_db;
pub mod glob_search;
pub mod hardware;
pub mod http_request;
pub mod image_info;
pub mod image_process;
pub mod kubernetes;
pub mod mcp_session;
pub mod memory_forget;
pub mod memory_recall;
pub mod memory_store;
pub mod model_routing;
pub mod notification;
pub mod pdf_create;
pub mod pdf_read;
pub mod plane_guard;
pub mod process_monitor;
pub mod proxy_config;
pub mod pushover;
pub mod register_api_tool;
pub mod request_credential;
pub mod schedule;
pub mod screenshot;
pub mod search_mcp_registry;
pub mod shell;
pub mod skill_wrapper;
pub mod spreadsheet;
pub mod sql_query;
pub mod ssh_exec;
pub mod suggest_connectors;
pub mod tool_output;
pub mod tool_validation;
pub mod vector_delete;
pub mod vector_search;
pub mod vector_store;
pub mod wasm_call;
pub mod wasm_compile;
pub mod wasm_exec;
pub mod wasm_inspect;
pub mod web_fetch;
pub mod web_search_tool;

pub use delegate::DelegateTool;

// ── Default registry factory ───────────────────────────────────────────────
// DelegateTool is NOT registered here — it needs store + workspace_base
// and is registered in main.rs after those are available.

pub fn default_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(shell::ShellTool::new()));
    r.register(Arc::new(file_read::FileReadTool));
    r.register(Arc::new(file_write::FileWriteTool));
    r.register(Arc::new(file_edit::FileEditTool));
    r.register(Arc::new(glob_search::GlobSearchTool));
    r.register(Arc::new(content_search::ContentSearchTool));
    r.register(Arc::new(git_operations::GitOperationsTool));
    r.register(Arc::new(web_fetch::WebFetchTool));
    r.register(Arc::new(web_search_tool::WebSearchTool::new()));
    r.register(Arc::new(http_request::HttpRequestTool));
    r.register(Arc::new(external_api::ExternalApiTool::new()));
    r.register(Arc::new(external_db::ExternalDbTool::new()));
    // NOTE: BrowserTool and ScreenshotTool require Arc<BrowserPool> and are registered
    // in main.rs when a browser pool is available.
    r.register(Arc::new(browser_open::BrowserOpenTool));
    r.register(Arc::new(memory_store::MemoryStoreTool));
    r.register(Arc::new(memory_recall::MemoryRecallTool));
    r.register(Arc::new(memory_forget::MemoryForgetTool));
    r.register(Arc::new(data_extractor::DataExtractorTool));
    r.register(Arc::new(pdf_read::PdfReadTool));
    r.register(Arc::new(image_info::ImageInfoTool));
    r.register(Arc::new(api_call::ApiCallTool));
    r.register(Arc::new(pushover::PushoverTool));
    r.register(Arc::new(request_credential::RequestCredentialTool));
    r.register(Arc::new(register_api_tool::RegisterApiTool));
    r.register(Arc::new(plane_guard::PlaneGuardTool));
    r.register(Arc::new(tool_validation::ToolValidationTool));
    r.register(Arc::new(tool_output::ToolOutputTool));
    r.register(Arc::new(skill_wrapper::SkillWrapperTool));
    r.register(Arc::new(schedule::ScheduleTool));
    r.register(Arc::new(cron::CronAddTool));
    r.register(Arc::new(cron::CronListTool));
    r.register(Arc::new(cron::CronRemoveTool));
    r.register(Arc::new(cron::CronRunTool));
    r.register(Arc::new(cron::CronRunsTool));
    r.register(Arc::new(cron::CronUpdateTool));
    r.register(Arc::new(mcp_session::McpSessionTool::new()));
    r.register(Arc::new(search_mcp_registry::SearchMcpRegistryTool));
    r.register(Arc::new(suggest_connectors::SuggestConnectorsTool));
    r.register(Arc::new(list_connectors_in_category::ListConnectorsInCategoryTool));
    r.register(Arc::new(request_more_connectors::RequestMoreConnectorsTool));
    r.register(Arc::new(create_custom_connector::CreateCustomConnectorTool));
    r.register(Arc::new(request_more_tools::RequestMoreToolsTool));

    // Register all built-in connector tools (salesforce, github, slack, etc.)
    // install_store=None here — callers that have a ConnectorInstallStore should
    // call connector_tool::register_all_connectors(registry, Some(store)) after
    // default_registry() to wire in OAuth token injection.
    connector_tool::register_all_connectors(&mut r, None);
    r.register(Arc::new(email::EmailTool));
    r.register(Arc::new(acp_session::AcpSessionTool));
    r.register(Arc::new(proxy_config::ProxyConfigTool));
    r.register(Arc::new(model_routing::ModelRoutingTool));
    r.register(Arc::new(ask_user::AskUserTool));
    r.register(Arc::new(hardware::HardwareBoardInfoTool));
    r.register(Arc::new(hardware::HardwareMemoryMapTool));
    r.register(Arc::new(hardware::HardwareMemoryReadTool));
    r.register(Arc::new(sql_query::SqlQueryTool));
    r.register(Arc::new(diff_patch::DiffTool));
    r.register(Arc::new(diff_patch::PatchTool));
    r.register(Arc::new(code_run::CodeRunTool));
    r.register(Arc::new(notification::NotificationTool));
    r.register(Arc::new(compress::CompressTool));
    r.register(Arc::new(compress::DecompressTool));
    r.register(Arc::new(image_process::ImageProcessTool));
    r.register(Arc::new(ssh_exec::SshExecTool));
    r.register(Arc::new(docker::DockerTool));
    r.register(Arc::new(spreadsheet::SpreadsheetReadTool));
    r.register(Arc::new(spreadsheet::SpreadsheetWriteTool));
    r.register(Arc::new(process_monitor::ProcessMonitorTool));
    r.register(Arc::new(kubernetes::KubernetesTool));
    r.register(Arc::new(pdf_create::PdfCreateTool));
    r.register(Arc::new(crypto_tool::CryptoTool));
    // WASM tools
    r.register(Arc::new(wasm_exec::WasmExecTool));
    r.register(Arc::new(wasm_compile::WasmCompileTool));
    r.register(Arc::new(wasm_inspect::WasmInspectTool));
    r.register(Arc::new(wasm_call::WasmCallTool));
    // NOTE: vector tools are registered in main.rs (they need Arc<PgVectorStore> + Arc<dyn EmbeddingModel>)
    // NOTE: browser tools with pool are registered in main.rs (they need Arc<BrowserPool>)
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool {
        tool_name: String,
    }

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn parameters_schema(&self) -> Vec<ParameterSchema> {
            vec![]
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::text("ok"))
        }
    }

    #[test]
    fn test_registry_register_get() {
        let mut registry = ToolRegistry::new();
        let tool = Arc::new(DummyTool { tool_name: "my_tool".into() });
        registry.register(tool);
        assert!(registry.get("my_tool").is_some());
    }

    #[test]
    fn test_registry_list() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool { tool_name: "alpha".into() }));
        registry.register(Arc::new(DummyTool { tool_name: "beta".into() }));
        let names = registry.list();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn test_registry_get_missing() {
        let registry = ToolRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_default_registry_not_empty() {
        let registry = default_registry();
        assert!(registry.list().len() > 30, "expected >30 tools, got {}", registry.list().len());
    }
}
