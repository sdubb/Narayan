use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const HIDDEN_TOOLS: &[&str] = &["wasm_exec", "wasm_call", "wasm_compile", "wasm_inspect", "run_registered_wasm"];

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

    fn output_schema(&self) -> Option<serde_json::Value> {
        default_output_schema(self)
    }

    /// Human-readable contract text for the tool's input shape.
    /// Keep this short, explicit, and DSL-friendly.
    fn input_contract(&self) -> Option<String> {
        Some(default_input_contract(self))
    }

    /// Human-readable contract text for the tool's output shape.
    fn output_contract(&self) -> Option<String> {
        Some(default_output_contract(self))
    }

    /// When the LLM should prefer this tool.
    fn when_to_use(&self) -> Option<String> {
        Some(default_when_to_use(self))
    }

    /// When the LLM should avoid this tool.
    fn when_not_to_use(&self) -> Option<String> {
        Some(default_when_not_to_use(self))
    }

    /// Extra usage examples for plan mode and tool manifests.
    fn examples(&self) -> Vec<String> {
        Vec::new()
    }

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
        let mut names: Vec<&str> =
            self.tools.keys().map(String::as_str).filter(|name| !HIDDEN_TOOLS.contains(name)).collect();
        names.sort_unstable();
        names
    }

    /// All tool names grouped by their declared category.
    pub fn by_category(&self) -> std::collections::BTreeMap<&str, Vec<&str>> {
        let mut map: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        for (name, tool) in &self.tools {
            if HIDDEN_TOOLS.contains(&name.as_str()) {
                continue;
            }
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
            .filter(|t| {
                let name = t.name();
                !HIDDEN_TOOLS.contains(&name)
            })
            .filter(|t| t.category() == category)
            .map(|t| tool_spec_from_tool(t.as_ref()))
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
    #[allow(unused_imports)]
    pub use super::create_custom_connector::CreateCustomConnectorTool;
    #[allow(unused_imports)]
    pub use super::list_connectors_in_category::ListConnectorsInCategoryTool;
    #[allow(unused_imports)]
    pub use super::request_more_connectors::RequestMoreConnectorsTool;
    #[allow(unused_imports)]
    pub use super::request_more_tools::RequestMoreToolsTool;
}

pub mod connector_tool;
pub mod create_custom_connector;
pub mod create_workspace_tool;
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
pub mod data_engine;
pub mod data_extractor;
pub mod delegate;
pub mod diff_patch;
pub mod docker;
pub mod email;
pub mod external_api;
pub mod external_db;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod git_operations;
pub mod glob_search;
pub mod hardware;
pub mod http_request;
pub mod image_info;
pub mod image_process;
pub mod kubernetes;
pub mod mcp_session;
pub mod message_inbox;
pub mod memory_consolidate;
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
pub mod run_registered_wasm;
pub mod schedule;
pub mod screenshot;
pub mod search_mcp_registry;
pub mod send_message;
pub mod session_tasks;
pub mod shell;
pub mod skill_wrapper;
pub mod spreadsheet;
pub mod sql_query;
pub mod ssh_exec;
pub mod suggest_connectors;
pub mod tool_output;
pub mod tool_search;
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
pub mod worktree;

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
    r.register(Arc::new(data_engine::DataEngineTool));
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
    r.register(Arc::new(create_workspace_tool::CreateWorkspaceToolTool));
    r.register(Arc::new(request_more_tools::RequestMoreToolsTool));
    r.register(Arc::new(tool_search::ToolSearchTool));
    r.register(Arc::new(worktree::EnterWorktreeTool));
    r.register(Arc::new(worktree::ExitWorktreeTool));

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
    r.register(Arc::new(run_registered_wasm::RunRegisteredWasmTool::new()));
    // NOTE: vector tools are registered in main.rs (they need Arc<PgVectorStore> + Arc<dyn EmbeddingModel>)
    // NOTE: browser tools with pool are registered in main.rs (they need Arc<BrowserPool>)
    r
}

pub fn parameters_schema_to_json(parameters: &[ParameterSchema]) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": parameters.iter().fold(
            serde_json::Map::new(),
            |mut acc, p| {
                acc.insert(p.name.clone(), serde_json::json!({
                    "type":        p.param_type,
                    "description": p.description,
                }));
                acc
            }
        ),
        "required": parameters.iter()
            .filter(|p| p.required)
            .map(|p| p.name.clone())
            .collect::<Vec<_>>(),
    })
}

fn schema_array(items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "items": items,
    })
}

fn schema_string() -> serde_json::Value {
    serde_json::json!({ "type": "string" })
}

fn schema_integer() -> serde_json::Value {
    serde_json::json!({ "type": "integer" })
}

fn schema_number() -> serde_json::Value {
    serde_json::json!({ "type": "number" })
}

fn schema_boolean() -> serde_json::Value {
    serde_json::json!({ "type": "boolean" })
}

fn generic_object_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": true,
    })
}

fn any_json_schema() -> serde_json::Value {
    serde_json::json!({
        "anyOf": [
            { "type": "object", "additionalProperties": true },
            { "type": "array", "items": {} },
            { "type": "string" },
            { "type": "integer" },
            { "type": "number" },
            { "type": "boolean" },
            { "type": "null" },
        ]
    })
}

pub fn default_output_schema<T: Tool + ?Sized>(tool: &T) -> Option<serde_json::Value> {
    if tool.category().starts_with("connector/") {
        return Some(any_json_schema());
    }

    let schema = match tool.name() {
        "data_engine" => serde_json::json!({
            "type": "object",
            "required": ["records", "meta", "warnings", "errors"],
            "properties": {
                "records": {
                    "type": "array",
                    "items": { "type": "object" }
                },
                "meta": {
                    "type": "object",
                    "required": [
                        "input_count",
                        "output_count",
                        "dropped_count",
                        "derived_fields",
                        "ops_applied",
                        "execution_time_ms",
                        "used_llm",
                        "confidence",
                        "fallback_needed",
                        "missing_fields"
                    ],
                    "properties": {
                        "input_count": schema_integer(),
                        "output_count": schema_integer(),
                        "dropped_count": schema_integer(),
                        "derived_fields": schema_array(schema_string()),
                        "ops_applied": schema_array(schema_string()),
                        "execution_time_ms": schema_integer(),
                        "used_llm": schema_boolean(),
                        "confidence": schema_number(),
                        "fallback_needed": schema_boolean(),
                        "missing_fields": schema_array(schema_string()),
                    },
                    "additionalProperties": true,
                },
                "warnings": schema_array(schema_string()),
                "errors": schema_array(schema_string()),
            },
            "additionalProperties": true,
        }),
        "file_read" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["path", "is_directory", "entry_count", "entries", "hint"],
                    "properties": {
                        "path": schema_string(),
                        "is_directory": serde_json::json!({ "type": "boolean", "const": true }),
                        "entry_count": schema_integer(),
                        "entries": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["path", "name", "is_dir"],
                            "properties": {
                                "path": schema_string(),
                                "name": schema_string(),
                                "is_dir": schema_boolean(),
                                "size_bytes": serde_json::json!({ "type": ["integer", "null"] }),
                            },
                            "additionalProperties": true,
                        })),
                        "hint": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["content", "encoding", "size"],
                    "properties": {
                        "content": schema_string(),
                        "encoding": serde_json::json!({ "type": "string", "const": "base64" }),
                        "size": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["content", "path", "total_lines", "size_bytes"],
                    "properties": {
                        "content": schema_string(),
                        "path": schema_string(),
                        "total_lines": schema_integer(),
                        "size_bytes": schema_integer(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "file_write" => serde_json::json!({
            "type": "object",
            "required": ["written", "path", "bytes", "appended"],
            "properties": {
                "written": serde_json::json!({ "type": "boolean", "const": true }),
                "path": schema_string(),
                "bytes": schema_integer(),
                "appended": schema_boolean(),
            },
            "additionalProperties": true,
        }),
        "file_edit" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["edited", "path"],
                    "properties": {
                        "edited": serde_json::json!({ "type": "boolean", "const": true }),
                        "path": schema_string(),
                        "replacements": schema_integer(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "glob_search" => serde_json::json!({
            "type": "object",
            "required": ["pattern", "root", "count", "files"],
            "properties": {
                "pattern": schema_string(),
                "root": schema_string(),
                "count": schema_integer(),
                "files": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["path", "rel_path", "is_dir"],
                    "properties": {
                        "path": schema_string(),
                        "rel_path": schema_string(),
                        "is_dir": schema_boolean(),
                        "size": serde_json::json!({ "type": ["integer", "null"] }),
                    },
                    "additionalProperties": true,
                })),
            },
            "additionalProperties": true,
        }),
        "content_search" => serde_json::json!({
            "type": "object",
            "required": ["pattern", "count", "matches"],
            "properties": {
                "pattern": schema_string(),
                "count": schema_integer(),
                "matches": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["file", "line_no", "line"],
                    "properties": {
                        "file": schema_string(),
                        "line_no": schema_integer(),
                        "line": schema_string(),
                    },
                    "additionalProperties": true,
                })),
            },
            "additionalProperties": true,
        }),
        "compress" => serde_json::json!({
            "type": "object",
            "required": ["output", "format", "files", "output_bytes", "elapsed_ms"],
            "properties": {
                "output": schema_string(),
                "format": schema_string(),
                "files": schema_integer(),
                "output_bytes": schema_integer(),
                "elapsed_ms": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "decompress" => serde_json::json!({
            "type": "object",
            "required": ["extracted", "output_dir", "files", "elapsed_ms"],
            "properties": {
                "extracted": serde_json::json!({ "type": "boolean", "const": true }),
                "output_dir": schema_string(),
                "files": schema_integer(),
                "elapsed_ms": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "web_search_tool" => serde_json::json!({
            "type": "object",
            "required": ["query", "count", "results"],
            "properties": {
                "query": schema_string(),
                "count": schema_integer(),
                "results": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["title", "url", "snippet"],
                    "properties": {
                        "title": schema_string(),
                        "url": schema_string(),
                        "snippet": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "source": schema_string(),
            },
            "additionalProperties": true,
        }),
        "web_fetch" => serde_json::json!({
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
        }),
        "http_request" => serde_json::json!({
            "type": "object",
            "required": ["status", "headers", "body"],
            "properties": {
                "status": schema_integer(),
                "headers": serde_json::json!({
                    "type": "object",
                    "additionalProperties": schema_string(),
                }),
                "body": schema_string(),
            },
            "additionalProperties": true,
        }),
        "browser_open" => serde_json::json!({
            "type": "object",
            "required": ["url", "status", "reachable"],
            "properties": {
                "url": schema_string(),
                "status": schema_integer(),
                "reachable": schema_boolean(),
            },
            "additionalProperties": true,
        }),
        "pdf_read" => serde_json::json!({
            "type": "object",
            "required": ["text", "path", "char_count"],
            "properties": {
                "text": schema_string(),
                "path": schema_string(),
                "char_count": schema_integer(),
                "total_pages": serde_json::json!({ "type": ["integer", "null"] }),
            },
            "additionalProperties": true,
        }),
        "spreadsheet_read" => serde_json::json!({
            "type": "object",
            "required": ["sheet", "sheets", "headers", "rows", "row_count"],
            "properties": {
                "sheet": schema_string(),
                "sheets": schema_array(schema_string()),
                "headers": schema_array(schema_string()),
                "rows": schema_array(serde_json::json!({})),
                "row_count": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "spreadsheet_write" => serde_json::json!({
            "type": "object",
            "required": ["output", "rows", "columns", "sheet", "size_bytes"],
            "properties": {
                "output": schema_string(),
                "rows": schema_integer(),
                "columns": schema_integer(),
                "sheet": schema_string(),
                "size_bytes": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "shell" => serde_json::json!({
            "type": "object",
            "required": ["stdout", "stderr", "exit_code"],
            "properties": {
                "stdout": schema_string(),
                "stderr": schema_string(),
                "exit_code": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "code_run" => serde_json::json!({
            "type": "object",
            "required": ["stdout", "stderr", "exit_code", "elapsed_ms", "language"],
            "properties": {
                "stdout": schema_string(),
                "stderr": schema_string(),
                "exit_code": schema_integer(),
                "elapsed_ms": schema_integer(),
                "language": schema_string(),
            },
            "additionalProperties": true,
        }),
        "data_extractor" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["tables", "count"],
                    "properties": {
                        "tables": schema_array(schema_array(schema_array(schema_string()))),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["links", "count"],
                    "properties": {
                        "links": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["href", "text"],
                            "properties": {
                                "href": schema_string(),
                                "text": schema_string(),
                            },
                            "additionalProperties": true,
                        })),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["emails", "count"],
                    "properties": {
                        "emails": schema_array(schema_string()),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["prices", "count"],
                    "properties": {
                        "prices": schema_array(schema_string()),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["phones", "count"],
                    "properties": {
                        "phones": schema_array(schema_string()),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["urls", "count"],
                    "properties": {
                        "urls": schema_array(schema_string()),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["items", "count"],
                    "properties": {
                        "items": schema_array(schema_string()),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["matches", "count"],
                    "properties": {
                        "matches": schema_array(schema_string()),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "memory_store" => serde_json::json!({
            "type": "object",
            "required": ["stored", "key", "scope"],
            "properties": {
                "stored": schema_boolean(),
                "key": schema_string(),
                "scope": schema_string(),
            },
            "additionalProperties": true,
        }),
        "memory_recall" => serde_json::json!({
            "type": "object",
            "required": ["key", "value", "found"],
            "properties": {
                "key": schema_string(),
                "value": {},
                "found": schema_boolean(),
            },
            "additionalProperties": true,
        }),
        "memory_forget" => serde_json::json!({
            "type": "object",
            "required": ["deleted", "key"],
            "properties": {
                "deleted": schema_boolean(),
                "key": schema_string(),
            },
            "additionalProperties": true,
        }),
        "schedule" => serde_json::json!({
            "type": "object",
            "required": ["scheduled", "id", "goal", "run_at"],
            "properties": {
                "scheduled": schema_boolean(),
                "id": schema_string(),
                "goal": schema_string(),
                "run_at": schema_string(),
            },
            "additionalProperties": true,
        }),
        "cron_add" => serde_json::json!({
            "type": "object",
            "required": ["added", "id", "schedule"],
            "properties": {
                "added": schema_boolean(),
                "id": schema_string(),
                "schedule": schema_string(),
            },
            "additionalProperties": true,
        }),
        "cron_list" => serde_json::json!({
            "type": "object",
            "required": ["jobs", "count"],
            "properties": {
                "jobs": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["id", "schedule", "command", "enabled"],
                    "properties": {
                        "id": schema_string(),
                        "schedule": schema_string(),
                        "command": schema_string(),
                        "enabled": schema_boolean(),
                        "last_run": serde_json::json!({ "type": ["string", "null"] }),
                    },
                    "additionalProperties": true,
                })),
                "count": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "cron_remove" => serde_json::json!({
            "type": "object",
            "required": ["removed", "id"],
            "properties": {
                "removed": schema_boolean(),
                "id": schema_string(),
            },
            "additionalProperties": true,
        }),
        "cron_run" => serde_json::json!({
            "type": "object",
            "required": ["ran", "id", "success", "output", "ran_at"],
            "properties": {
                "ran": schema_boolean(),
                "id": schema_string(),
                "success": schema_boolean(),
                "output": schema_string(),
                "ran_at": schema_string(),
            },
            "additionalProperties": true,
        }),
        "cron_runs" => serde_json::json!({
            "type": "object",
            "required": ["id", "runs", "last_run"],
            "properties": {
                "id": schema_string(),
                "runs": schema_array(schema_string()),
                "last_run": serde_json::json!({ "type": ["string", "null"] }),
            },
            "additionalProperties": true,
        }),
        "cron_update" => serde_json::json!({
            "type": "object",
            "required": ["updated", "id"],
            "properties": {
                "updated": schema_boolean(),
                "id": schema_string(),
            },
            "additionalProperties": true,
        }),
        "request_credential" => serde_json::json!({
            "type": "object",
            "required": ["stored", "name", "hint"],
            "properties": {
                "stored": schema_boolean(),
                "name": schema_string(),
                "hint": schema_string(),
            },
            "additionalProperties": true,
        }),
        "register_api_tool" => serde_json::json!({
            "type": "object",
            "required": ["registered", "tool_name", "base_url"],
            "properties": {
                "registered": schema_boolean(),
                "tool_name": schema_string(),
                "base_url": schema_string(),
            },
            "additionalProperties": true,
        }),
        "tool_validation" => serde_json::json!({
            "type": "object",
            "required": ["valid", "tool", "arg_count"],
            "properties": {
                "valid": schema_boolean(),
                "tool": schema_string(),
                "arg_count": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "tool_output" => serde_json::json!({
            "type": "object",
            "required": ["formatted", "tool"],
            "properties": {
                "formatted": schema_string(),
                "tool": schema_string(),
            },
            "additionalProperties": true,
        }),
        "plane_guard" => serde_json::json!({
            "type": "object",
            "required": ["approved", "action", "risk_level", "reversible", "reason"],
            "properties": {
                "approved": schema_boolean(),
                "action": schema_string(),
                "risk_level": schema_string(),
                "reversible": schema_boolean(),
                "reason": schema_string(),
            },
            "additionalProperties": true,
        }),
        "ask_user" => serde_json::json!({
            "type": "object",
            "required": ["status", "questions", "note"],
            "properties": {
                "status": schema_string(),
                "questions": schema_array(serde_json::json!({ "type": "object", "additionalProperties": true })),
                "note": schema_string(),
            },
            "additionalProperties": true,
        }),
        "model_routing" => serde_json::json!({
            "type": "object",
            "required": ["updated"],
            "properties": {
                "updated": schema_boolean(),
            },
            "additionalProperties": true,
        }),
        "notification" => serde_json::json!({
            "type": "object",
            "required": ["sent", "provider", "status"],
            "properties": {
                "sent": schema_boolean(),
                "provider": schema_string(),
                "status": schema_string(),
            },
            "additionalProperties": true,
        }),
        "proxy_config" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["configured"],
                    "properties": { "configured": schema_boolean() },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["cleared"],
                    "properties": { "cleared": schema_boolean() },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["http_proxy", "no_proxy"],
                    "properties": {
                        "http_proxy": serde_json::json!({ "type": ["string", "null"] }),
                        "no_proxy": serde_json::json!({ "type": ["string", "null"] }),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "hardware_board_info" | "hardware_memory_map" | "hardware_memory_read" => serde_json::json!({
            "type": "object",
            "required": ["info"],
            "properties": {
                "info": schema_string(),
            },
            "additionalProperties": true,
        }),
        "git_operations" => serde_json::json!({
            "type": "object",
            "required": ["stdout", "stderr", "exit_code"],
            "properties": {
                "stdout": schema_string(),
                "stderr": schema_string(),
                "exit_code": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "diff" => serde_json::json!({
            "type": "object",
            "required": ["patch", "insertions", "deletions", "unchanged", "has_changes"],
            "properties": {
                "patch": schema_string(),
                "insertions": schema_integer(),
                "deletions": schema_integer(),
                "unchanged": schema_integer(),
                "has_changes": schema_boolean(),
            },
            "additionalProperties": true,
        }),
        "patch" => serde_json::json!({
            "type": "object",
            "required": ["patched", "result", "chars", "written_to"],
            "properties": {
                "patched": schema_boolean(),
                "result": schema_string(),
                "chars": schema_integer(),
                "written_to": serde_json::json!({ "type": ["string", "null"] }),
            },
            "additionalProperties": true,
        }),
        "api_call" => serde_json::json!({
            "type": "object",
            "required": ["status", "body"],
            "properties": {
                "status": schema_integer(),
                "body": schema_string(),
            },
            "additionalProperties": true,
        }),
        "mcp_session" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["connected", "server", "server_info", "tool_count", "tools"],
                    "properties": {
                        "connected": schema_boolean(),
                        "server": schema_string(),
                        "server_info": any_json_schema(),
                        "tool_count": schema_integer(),
                        "tools": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["name"],
                            "properties": {
                                "name": schema_string(),
                                "description": serde_json::json!({ "type": ["string", "null"] }),
                                "inputSchema": any_json_schema(),
                            },
                            "additionalProperties": true,
                        })),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["tools", "count"],
                    "properties": {
                        "tools": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["name"],
                            "properties": {
                                "name": schema_string(),
                                "description": serde_json::json!({ "type": ["string", "null"] }),
                                "inputSchema": any_json_schema(),
                            },
                            "additionalProperties": true,
                        })),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["tool", "result"],
                    "properties": {
                        "tool": schema_string(),
                        "result": any_json_schema(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "search_mcp_registry" => serde_json::json!({
            "type": "object",
            "required": ["query", "count", "servers", "tip"],
            "properties": {
                "query": schema_string(),
                "count": schema_integer(),
                "servers": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["name", "url", "description", "categories", "auth_type", "connected"],
                    "properties": {
                        "name": schema_string(),
                        "url": schema_string(),
                        "description": schema_string(),
                        "categories": schema_array(schema_string()),
                        "auth_type": schema_string(),
                        "connected": schema_boolean(),
                    },
                    "additionalProperties": true,
                })),
                "tip": schema_string(),
            },
            "additionalProperties": true,
        }),
        "suggest_connectors" => serde_json::json!({
            "type": "object",
            "required": ["suggested", "reason", "blocking", "credential_keys", "operator_message", "status", "resume_endpoint"],
            "properties": {
                "suggested": schema_array(schema_string()),
                "reason": schema_string(),
                "blocking": schema_boolean(),
                "credential_keys": schema_array(schema_string()),
                "operator_message": schema_string(),
                "status": schema_string(),
                "resume_endpoint": schema_string(),
            },
            "additionalProperties": true,
        }),
        "list_connectors_in_category" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["category", "connectors", "instruction"],
                    "properties": {
                        "category": schema_string(),
                        "connectors": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["name", "category", "summary"],
                            "properties": {
                                "name": schema_string(),
                                "category": schema_string(),
                                "summary": schema_string(),
                            },
                            "additionalProperties": true,
                        })),
                        "instruction": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["category", "connectors", "note"],
                    "properties": {
                        "category": schema_string(),
                        "connectors": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["name", "category", "summary"],
                            "properties": {
                                "name": schema_string(),
                                "category": schema_string(),
                                "summary": schema_string(),
                            },
                            "additionalProperties": true,
                        })),
                        "note": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "request_more_connectors" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["status", "message"],
                    "properties": {
                        "status": serde_json::json!({ "type": "string", "const": "more_available" }),
                        "message": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["status", "category", "reason", "options"],
                    "properties": {
                        "status": serde_json::json!({ "type": "string", "const": "exhausted" }),
                        "category": schema_string(),
                        "reason": schema_string(),
                        "options": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["action", "description"],
                            "properties": {
                                "action": schema_string(),
                                "description": schema_string(),
                            },
                            "additionalProperties": true,
                        })),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["status", "category", "reason", "options", "note"],
                    "properties": {
                        "status": serde_json::json!({ "type": "string", "const": "no_more_connectors" }),
                        "category": schema_string(),
                        "reason": schema_string(),
                        "options": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["action", "description"],
                            "properties": {
                                "action": schema_string(),
                                "description": schema_string(),
                            },
                            "additionalProperties": true,
                        })),
                        "note": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "create_custom_connector" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["status", "name", "category", "message"],
                    "properties": {
                        "status": serde_json::json!({ "type": "string", "const": "created" }),
                        "name": schema_string(),
                        "category": schema_string(),
                        "message": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["status", "name", "category", "creation_path", "note"],
                    "properties": {
                        "status": serde_json::json!({ "type": "string", "const": "pending" }),
                        "name": schema_string(),
                        "category": schema_string(),
                        "creation_path": schema_string(),
                        "note": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "create_workspace_tool" => serde_json::json!({
            "type": "object",
            "required": ["status", "message"],
            "properties": {
                "status": serde_json::json!({ "type": "string", "const": "pending_intercept" }),
                "message": schema_string(),
            },
            "additionalProperties": true,
        }),
        "request_more_tools" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["status", "requested_categories", "tools_added", "available_categories", "message"],
                    "properties": {
                        "status": serde_json::json!({ "type": "string", "const": "expanded" }),
                        "requested_categories": schema_array(schema_string()),
                        "tools_added": schema_array(schema_string()),
                        "available_categories": schema_array(schema_string()),
                        "message": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["status", "requested_categories", "note"],
                    "properties": {
                        "status": serde_json::json!({ "type": "string", "const": "expanding" }),
                        "requested_categories": schema_array(schema_string()),
                        "note": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "email" => serde_json::json!({
            "type": "object",
            "required": ["sent", "provider", "to"],
            "properties": {
                "sent": schema_boolean(),
                "provider": schema_string(),
                "to": schema_string(),
                "id": serde_json::json!({ "type": ["string", "null"] }),
                "host": serde_json::json!({ "type": ["string", "null"] }),
            },
            "additionalProperties": true,
        }),
        "acp_session" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["agents"],
                    "properties": {
                        "agents": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["sent", "to"],
                    "properties": {
                        "sent": schema_boolean(),
                        "to": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "skill_wrapper" => serde_json::json!({
            "type": "object",
            "required": ["skill", "status", "definition", "inputs"],
            "properties": {
                "skill": schema_string(),
                "status": serde_json::json!({ "type": "string", "const": "executed" }),
                "definition": schema_string(),
                "inputs": any_json_schema(),
            },
            "additionalProperties": true,
        }),
        "process_monitor" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["cpu_usage_pct", "total_memory_mb", "used_memory_mb", "total_swap_mb", "used_swap_mb", "process_count", "uptime_secs", "os", "kernel"],
                    "properties": {
                        "cpu_usage_pct": schema_number(),
                        "total_memory_mb": schema_integer(),
                        "used_memory_mb": schema_integer(),
                        "total_swap_mb": schema_integer(),
                        "used_swap_mb": schema_integer(),
                        "process_count": schema_integer(),
                        "uptime_secs": schema_integer(),
                        "os": serde_json::json!({ "type": ["string", "null"] }),
                        "kernel": serde_json::json!({ "type": ["string", "null"] }),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["processes", "count"],
                    "properties": {
                        "processes": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["pid", "name", "cpu_pct", "memory_mb", "status"],
                            "properties": {
                                "pid": schema_integer(),
                                "name": schema_string(),
                                "cpu_pct": schema_number(),
                                "memory_mb": schema_integer(),
                                "status": schema_string(),
                                "exe": serde_json::json!({ "type": ["string", "null"] }),
                            },
                            "additionalProperties": true,
                        })),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["query", "processes", "count"],
                    "properties": {
                        "query": schema_string(),
                        "processes": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["pid", "name", "cpu_pct", "memory_mb", "status"],
                            "properties": {
                                "pid": schema_integer(),
                                "name": schema_string(),
                                "cpu_pct": schema_number(),
                                "memory_mb": schema_integer(),
                                "status": schema_string(),
                                "exe": serde_json::json!({ "type": ["string", "null"] }),
                            },
                            "additionalProperties": true,
                        })),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["sort_by", "processes"],
                    "properties": {
                        "sort_by": schema_string(),
                        "processes": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["pid", "name", "cpu_pct", "memory_mb", "status"],
                            "properties": {
                                "pid": schema_integer(),
                                "name": schema_string(),
                                "cpu_pct": schema_number(),
                                "memory_mb": schema_integer(),
                                "status": schema_string(),
                                "exe": serde_json::json!({ "type": ["string", "null"] }),
                            },
                            "additionalProperties": true,
                        })),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["pid", "killed"],
                    "properties": {
                        "pid": schema_integer(),
                        "killed": schema_boolean(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "sql_query" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["rows", "row_count", "elapsed_ms"],
                    "properties": {
                        "rows": schema_array(serde_json::json!({ "type": "object", "additionalProperties": true })),
                        "row_count": schema_integer(),
                        "elapsed_ms": schema_integer(),
                    },
                    "additionalProperties": false,
                },
                {
                    "type": "object",
                    "required": ["rows", "row_count", "columns", "truncated", "elapsed_ms"],
                    "properties": {
                        "rows": schema_array(serde_json::json!({ "type": "object", "additionalProperties": true })),
                        "row_count": schema_integer(),
                        "columns": schema_array(schema_string()),
                        "truncated": schema_boolean(),
                        "elapsed_ms": schema_integer(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "delegate" => serde_json::json!({
            "type": "object",
            "required": ["child_agent_ids", "message"],
            "properties": {
                "child_agent_ids": schema_array(schema_string()),
                "message": schema_string(),
            },
            "additionalProperties": true,
        }),
        "image_process" => serde_json::json!({
            "type": "object",
            "required": ["output", "width", "height", "format", "size_bytes", "ops_applied", "elapsed_ms"],
            "properties": {
                "output": schema_string(),
                "width": schema_integer(),
                "height": schema_integer(),
                "format": schema_string(),
                "size_bytes": schema_integer(),
                "ops_applied": schema_integer(),
                "elapsed_ms": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "ssh_exec" => serde_json::json!({
            "type": "object",
            "required": ["host", "command", "stdout", "stderr", "exit_code", "elapsed_ms"],
            "properties": {
                "host": schema_string(),
                "command": schema_string(),
                "stdout": schema_string(),
                "stderr": schema_string(),
                "exit_code": schema_integer(),
                "elapsed_ms": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "docker" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["containers", "count"],
                    "properties": {
                        "containers": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["id", "image", "names", "status", "state"],
                            "properties": {
                                "id": schema_string(),
                                "image": schema_string(),
                                "names": schema_array(schema_string()),
                                "status": schema_string(),
                                "state": schema_string(),
                            },
                            "additionalProperties": true,
                        })),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["pulled", "image", "status"],
                    "properties": {
                        "pulled": schema_boolean(),
                        "image": schema_string(),
                        "status": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["started", "container_id", "detached"],
                    "properties": {
                        "started": schema_boolean(),
                        "container_id": schema_string(),
                        "detached": schema_boolean(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["exec_id", "output"],
                    "properties": {
                        "exec_id": schema_string(),
                        "output": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["container_id", "logs"],
                    "properties": {
                        "container_id": schema_string(),
                        "logs": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["stopped", "container_id"],
                    "properties": {
                        "stopped": schema_boolean(),
                        "container_id": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["removed", "container_id"],
                    "properties": {
                        "removed": schema_boolean(),
                        "container_id": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["Id"],
                    "properties": {
                        "Id": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "kubernetes" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["namespaces"],
                    "properties": {
                        "namespaces": schema_array(schema_string()),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["namespace", "pods", "count"],
                    "properties": {
                        "namespace": schema_string(),
                        "pods": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["name", "phase", "ready"],
                            "properties": {
                                "name": schema_string(),
                                "phase": serde_json::json!({ "type": ["string", "null"] }),
                                "ready": schema_boolean(),
                                "node": serde_json::json!({ "type": ["string", "null"] }),
                            },
                            "additionalProperties": true,
                        })),
                        "count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["namespace", "deployments"],
                    "properties": {
                        "namespace": schema_string(),
                        "deployments": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["name"],
                            "properties": {
                                "name": schema_string(),
                                "replicas": serde_json::json!({ "type": ["integer", "null"] }),
                                "ready_replicas": serde_json::json!({ "type": ["integer", "null"] }),
                                "available": serde_json::json!({ "type": ["integer", "null"] }),
                            },
                            "additionalProperties": true,
                        })),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["apiVersion"],
                    "properties": {},
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["pod", "logs"],
                    "properties": {
                        "pod": schema_string(),
                        "logs": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["scaled", "deployment", "replicas"],
                    "properties": {
                        "scaled": schema_boolean(),
                        "deployment": schema_string(),
                        "replicas": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["deleted", "pod"],
                    "properties": {
                        "deleted": schema_boolean(),
                        "pod": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["deployment"],
                    "properties": {
                        "deployment": schema_string(),
                        "replicas": serde_json::json!({ "type": ["integer", "null"] }),
                        "ready": serde_json::json!({ "type": ["integer", "null"] }),
                        "available": serde_json::json!({ "type": ["integer", "null"] }),
                        "updated": serde_json::json!({ "type": ["integer", "null"] }),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "pdf_create" => serde_json::json!({
            "type": "object",
            "required": ["title", "pages", "size_bytes", "pdf_b64"],
            "properties": {
                "title": schema_string(),
                "pages": schema_integer(),
                "size_bytes": schema_integer(),
                "saved_to": serde_json::json!({ "type": ["string", "null"] }),
                "pdf_b64": schema_string(),
            },
            "additionalProperties": true,
        }),
        "crypto_tool" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["algorithm", "hash", "format", "bytes"],
                    "properties": {
                        "algorithm": schema_string(),
                        "hash": schema_string(),
                        "format": schema_string(),
                        "bytes": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["signature", "format"],
                    "properties": {
                        "signature": schema_string(),
                        "format": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["ciphertext_b64", "algorithm"],
                    "properties": {
                        "ciphertext_b64": schema_string(),
                        "algorithm": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["plaintext"],
                    "properties": {
                        "plaintext": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["secret", "format", "length"],
                    "properties": {
                        "secret": schema_string(),
                        "format": schema_string(),
                        "length": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["key_b64", "salt_b64", "iterations"],
                    "properties": {
                        "key_b64": schema_string(),
                        "salt_b64": schema_string(),
                        "iterations": schema_integer(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "wasm_exec" => serde_json::json!({
            "type": "object",
            "required": [
                "exit_code",
                "success",
                "stdout",
                "stderr",
                "elapsed_ms",
                "fuel_used",
                "wasm_size_bytes",
                "memory_limit_bytes",
                "fuel_limit"
            ],
            "properties": {
                "exit_code": schema_integer(),
                "success": schema_boolean(),
                "stdout": schema_string(),
                "stderr": schema_string(),
                "elapsed_ms": schema_integer(),
                "fuel_used": serde_json::json!({ "type": ["integer", "null"] }),
                "wasm_size_bytes": schema_integer(),
                "memory_limit_bytes": schema_integer(),
                "fuel_limit": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "wasm_compile" => serde_json::json!({
            "type": "object",
            "required": ["valid", "size_bytes", "exports", "imports", "bytes_b64", "tip"],
            "properties": {
                "valid": schema_boolean(),
                "size_bytes": schema_integer(),
                "exports": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["name", "type"],
                    "properties": {
                        "name": schema_string(),
                        "type": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "imports": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["module", "name", "type"],
                    "properties": {
                        "module": schema_string(),
                        "name": schema_string(),
                        "type": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "bytes_b64": schema_string(),
                "tip": schema_string(),
            },
            "additionalProperties": true,
        }),
        "wasm_inspect" => serde_json::json!({
            "type": "object",
            "required": [
                "size_bytes",
                "interface",
                "wasi_version",
                "exports",
                "export_count",
                "imports",
                "import_count",
                "has_start",
                "has_memory",
                "tip"
            ],
            "properties": {
                "size_bytes": schema_integer(),
                "interface": schema_string(),
                "wasi_version": schema_string(),
                "exports": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["name", "kind"],
                    "properties": {
                        "name": schema_string(),
                        "kind": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "export_count": schema_integer(),
                "imports": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["module", "name", "kind"],
                    "properties": {
                        "module": schema_string(),
                        "name": schema_string(),
                        "kind": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "import_count": schema_integer(),
                "has_start": schema_boolean(),
                "has_memory": schema_boolean(),
                "tip": schema_string(),
            },
            "additionalProperties": true,
        }),
        "wasm_call" => serde_json::json!({
            "type": "object",
            "required": ["function", "result", "elapsed_ms", "fuel_used", "fuel_limit", "memory_limit_bytes", "success"],
            "properties": {
                "function": schema_string(),
                "result": any_json_schema(),
                "elapsed_ms": schema_integer(),
                "fuel_used": serde_json::json!({ "type": ["integer", "null"] }),
                "fuel_limit": schema_integer(),
                "memory_limit_bytes": schema_integer(),
                "success": schema_boolean(),
            },
            "additionalProperties": true,
        }),
        "run_registered_wasm" => any_json_schema(),
        "vector_store" => serde_json::json!({
            "type": "object",
            "required": ["stored", "doc_id", "model", "dimensions", "chars"],
            "properties": {
                "stored": schema_boolean(),
                "doc_id": schema_string(),
                "model": schema_string(),
                "dimensions": schema_integer(),
                "chars": schema_integer(),
            },
            "additionalProperties": true,
        }),
        "vector_search" => serde_json::json!({
            "type": "object",
            "required": ["query", "count", "results", "model", "scope"],
            "properties": {
                "query": schema_string(),
                "count": schema_integer(),
                "results": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["doc_id", "agent_id", "score", "content", "metadata", "stored_at"],
                    "properties": {
                        "doc_id": schema_string(),
                        "agent_id": schema_string(),
                        "score": schema_number(),
                        "content": schema_string(),
                        "metadata": any_json_schema(),
                        "stored_at": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "model": schema_string(),
                "scope": schema_string(),
            },
            "additionalProperties": true,
        }),
        "vector_delete" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["deleted", "doc_id"],
                    "properties": {
                        "deleted": schema_boolean(),
                        "doc_id": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["deleted", "agent_id"],
                    "properties": {
                        "deleted": schema_integer(),
                        "agent_id": schema_string(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        "browser" => serde_json::json!({
            "type": "object",
            "required": ["url", "title", "text", "links", "headings", "js_result"],
            "properties": {
                "url": schema_string(),
                "title": schema_string(),
                "text": schema_string(),
                "links": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["href", "text"],
                    "properties": {
                        "href": schema_string(),
                        "text": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "headings": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["level", "text"],
                    "properties": {
                        "level": schema_string(),
                        "text": schema_string(),
                    },
                    "additionalProperties": true,
                })),
                "js_result": any_json_schema(),
            },
            "additionalProperties": true,
        }),
        "browser_interact" => serde_json::json!({
            "type": "object",
            "required": ["final_url", "title", "text_preview", "actions_taken", "screenshot_b64"],
            "properties": {
                "final_url": schema_string(),
                "title": schema_string(),
                "text_preview": schema_string(),
                "actions_taken": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["type", "success"],
                    "properties": {
                        "type": schema_string(),
                        "success": schema_boolean(),
                        "selector": serde_json::json!({ "type": ["string", "null"] }),
                        "value": serde_json::json!({ "type": ["string", "null"] }),
                        "key": serde_json::json!({ "type": ["string", "null"] }),
                        "ms": serde_json::json!({ "type": ["integer", "null"] }),
                        "script": serde_json::json!({ "type": ["string", "null"] }),
                        "url": serde_json::json!({ "type": ["string", "null"] }),
                        "result": any_json_schema(),
                        "text": serde_json::json!({ "type": ["string", "null"] }),
                        "error": serde_json::json!({ "type": ["string", "null"] }),
                    },
                    "additionalProperties": true,
                })),
                "screenshot_b64": serde_json::json!({ "type": ["string", "null"] }),
            },
            "additionalProperties": true,
        }),
        "browser_network" => serde_json::json!({
            "type": "object",
            "required": ["page_url", "request_count", "requests", "tip"],
            "properties": {
                "page_url": schema_string(),
                "request_count": schema_integer(),
                "requests": schema_array(serde_json::json!({
                    "type": "object",
                    "required": ["url", "type"],
                    "properties": {
                        "url": schema_string(),
                        "type": schema_string(),
                        "method": serde_json::json!({ "type": ["string", "null"] }),
                        "status": serde_json::json!({ "type": ["integer", "null"] }),
                        "duration": serde_json::json!({ "type": ["integer", "null"] }),
                        "size": serde_json::json!({ "type": ["integer", "null"] }),
                        "error": serde_json::json!({ "type": ["string", "null"] }),
                        "ts": serde_json::json!({ "type": ["integer", "null"] }),
                    },
                    "additionalProperties": true,
                })),
                "tip": schema_string(),
            },
            "additionalProperties": true,
        }),
        "browser_pdf" => serde_json::json!({
            "type": "object",
            "required": ["url", "size_bytes", "pdf_b64"],
            "properties": {
                "url": schema_string(),
                "size_bytes": schema_integer(),
                "saved_to": serde_json::json!({ "type": ["string", "null"] }),
                "pdf_b64": schema_string(),
            },
            "additionalProperties": true,
        }),
        "screenshot" => serde_json::json!({
            "type": "object",
            "required": ["url", "format", "width", "height", "full_page", "size_bytes", "image_b64"],
            "properties": {
                "url": schema_string(),
                "format": schema_string(),
                "width": schema_integer(),
                "height": schema_integer(),
                "full_page": schema_boolean(),
                "size_bytes": schema_integer(),
                "saved_to": serde_json::json!({ "type": ["string", "null"] }),
                "image_b64": schema_string(),
            },
            "additionalProperties": true,
        }),
        "external_api" => serde_json::json!({
            "type": "object",
            "required": ["status", "success", "data", "url"],
            "properties": {
                "status": schema_integer(),
                "success": schema_boolean(),
                "data": any_json_schema(),
                "url": schema_string(),
            },
            "additionalProperties": true,
        }),
        "external_db" => serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["schema", "tables", "table_count"],
                    "properties": {
                        "schema": schema_string(),
                        "tables": schema_array(serde_json::json!({
                            "type": "object",
                            "required": ["table", "rows", "columns"],
                            "properties": {
                                "table": schema_string(),
                                "rows": schema_integer(),
                                "columns": schema_array(serde_json::json!({
                                    "type": "object",
                                    "required": ["column", "type", "nullable", "default", "primary_key"],
                                    "properties": {
                                        "column": schema_string(),
                                        "type": schema_string(),
                                        "nullable": schema_boolean(),
                                        "default": serde_json::json!({ "type": ["string", "null"] }),
                                        "primary_key": schema_boolean(),
                                    },
                                    "additionalProperties": true,
                                })),
                            },
                            "additionalProperties": true,
                        })),
                        "table_count": schema_integer(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["rows", "row_count", "truncated", "sql"],
                    "properties": {
                        "rows": schema_array(serde_json::json!({ "type": "object", "additionalProperties": true })),
                        "row_count": schema_integer(),
                        "truncated": schema_boolean(),
                        "sql": schema_string(),
                    },
                    "additionalProperties": true,
                },
                {
                    "type": "object",
                    "required": ["rows_affected", "success"],
                    "properties": {
                        "rows_affected": schema_integer(),
                        "success": schema_boolean(),
                    },
                    "additionalProperties": true,
                }
            ]
        }),
        _ => return None,
    };
    Some(schema)
}

pub fn render_output_schema(schema: &serde_json::Value) -> String {
    let text = serde_json::to_string(schema).unwrap_or_else(|_| "{}".into());
    if text.len() > 1200 {
        format!("{}...(truncated)", &text[..1200])
    } else {
        text
    }
}

pub fn validate_output_against_schema(
    tool_name: &str,
    output: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<(), String> {
    validate_json_schema(output, schema, "$").map_err(|err| format!("{tool_name}: {err}"))
}

fn validate_json_schema(value: &serde_json::Value, schema: &serde_json::Value, path: &str) -> Result<(), String> {
    let Some(schema_obj) = schema.as_object() else {
        return Err(format!("{path}: schema must be a JSON object"));
    };

    if let Some(any_of) = schema_obj.get("anyOf").and_then(|value| value.as_array()) {
        if any_of.iter().any(|subschema| validate_json_schema(value, subschema, path).is_ok()) {
            return Ok(());
        }
        return Err(format!("{path}: value did not match anyOf"));
    }

    if let Some(one_of) = schema_obj.get("oneOf").and_then(|value| value.as_array()) {
        let matches = one_of.iter().filter(|subschema| validate_json_schema(value, subschema, path).is_ok()).count();
        if matches != 1 {
            return Err(format!("{path}: value matched {matches} oneOf schemas"));
        }
    }

    if let Some(all_of) = schema_obj.get("allOf").and_then(|value| value.as_array()) {
        for subschema in all_of {
            validate_json_schema(value, subschema, path)?;
        }
    }

    if let Some(not_schema) = schema_obj.get("not") {
        if validate_json_schema(value, not_schema, path).is_ok() {
            return Err(format!("{path}: value matched forbidden not-schema"));
        }
    }

    if let Some(expected_const) = schema_obj.get("const") {
        if value != expected_const {
            return Err(format!("{path}: value must equal const {}", expected_const));
        }
    }

    if let Some(enum_values) = schema_obj.get("enum").and_then(|value| value.as_array()) {
        if !enum_values.iter().any(|candidate| candidate == value) {
            return Err(format!("{path}: value {:?} not found in enum", value));
        }
    }

    if let Some(type_spec) = schema_obj.get("type") {
        let matches_type = matches_schema_type(value, type_spec);
        if !matches_type {
            return Err(format!(
                "{path}: expected {}, found {}",
                describe_schema_type(type_spec),
                describe_value_type(value)
            ));
        }
    }

    if let Some(min_length) = schema_obj.get("minLength").and_then(|value| value.as_u64()) {
        let actual = value.as_str().map(|s| s.chars().count() as u64).unwrap_or(0);
        if actual < min_length {
            return Err(format!("{path}: string length {actual} is below minLength {min_length}"));
        }
    }

    if let Some(max_length) = schema_obj.get("maxLength").and_then(|value| value.as_u64()) {
        let actual = value.as_str().map(|s| s.chars().count() as u64).unwrap_or(0);
        if actual > max_length {
            return Err(format!("{path}: string length {actual} exceeds maxLength {max_length}"));
        }
    }

    if let Some(pattern) = schema_obj.get("pattern").and_then(|value| value.as_str()) {
        let regex = Regex::new(pattern).map_err(|err| format!("{path}: invalid schema regex: {err}"))?;
        let text = value.as_str().ok_or_else(|| format!("{path}: expected string for pattern validation"))?;
        if !regex.is_match(text) {
            return Err(format!("{path}: value does not match pattern {pattern}"));
        }
    }

    if let Some(minimum) = schema_obj.get("minimum").and_then(|value| value.as_f64()) {
        let actual = value.as_f64().ok_or_else(|| format!("{path}: expected number for minimum validation"))?;
        if actual < minimum {
            return Err(format!("{path}: number {actual} is below minimum {minimum}"));
        }
    }

    if let Some(maximum) = schema_obj.get("maximum").and_then(|value| value.as_f64()) {
        let actual = value.as_f64().ok_or_else(|| format!("{path}: expected number for maximum validation"))?;
        if actual > maximum {
            return Err(format!("{path}: number {actual} exceeds maximum {maximum}"));
        }
    }

    if let Some(object) = value.as_object() {
        if let Some(required) = schema_obj.get("required").and_then(|value| value.as_array()) {
            for entry in required {
                let Some(name) = entry.as_str() else {
                    return Err(format!("{path}: required entries must be strings"));
                };
                if !object.contains_key(name) {
                    return Err(format!("{path}: missing required property '{name}'"));
                }
            }
        }

        let properties = schema_obj.get("properties").and_then(|value| value.as_object());
        if let Some(props) = properties {
            for (name, subschema) in props {
                if let Some(child) = object.get(name) {
                    validate_json_schema(child, subschema, &format!("{path}.{name}"))?;
                }
            }
        }

        if let Some(additional) = schema_obj.get("additionalProperties") {
            match additional {
                serde_json::Value::Bool(false) => {
                    let known: std::collections::HashSet<&str> =
                        properties.map(|props| props.keys().map(String::as_str).collect()).unwrap_or_default();
                    for key in object.keys() {
                        if !known.contains(key.as_str()) {
                            return Err(format!("{path}: unexpected property '{key}'"));
                        }
                    }
                }
                serde_json::Value::Object(subschema) => {
                    let known: std::collections::HashSet<&str> =
                        properties.map(|props| props.keys().map(String::as_str).collect()).unwrap_or_default();
                    for (key, child) in object {
                        if !known.contains(key.as_str()) {
                            validate_json_schema(
                                child,
                                &serde_json::Value::Object(subschema.clone()),
                                &format!("{path}.{key}"),
                            )?;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(array) = value.as_array() {
        if let Some(items) = schema_obj.get("items") {
            match items {
                serde_json::Value::Array(tuple_schemas) => {
                    for (idx, item) in array.iter().enumerate() {
                        let Some(subschema) = tuple_schemas.get(idx) else {
                            break;
                        };
                        validate_json_schema(item, subschema, &format!("{path}[{idx}]"))?;
                    }
                }
                other => {
                    for (idx, item) in array.iter().enumerate() {
                        validate_json_schema(item, other, &format!("{path}[{idx}]"))?;
                    }
                }
            }
        }

        if let Some(min_items) = schema_obj.get("minItems").and_then(|value| value.as_u64()) {
            if array.len() < min_items as usize {
                return Err(format!("{path}: array length {} is below minItems {min_items}", array.len()));
            }
        }
        if let Some(max_items) = schema_obj.get("maxItems").and_then(|value| value.as_u64()) {
            if array.len() > max_items as usize {
                return Err(format!("{path}: array length {} exceeds maxItems {max_items}", array.len()));
            }
        }
    }

    Ok(())
}

fn describe_schema_type(type_spec: &serde_json::Value) -> String {
    match type_spec {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(types) => {
            types.iter().filter_map(|value| value.as_str()).collect::<Vec<_>>().join(" | ")
        }
        other => other.to_string(),
    }
}

fn describe_value_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn matches_schema_type(value: &serde_json::Value, type_spec: &serde_json::Value) -> bool {
    match type_spec {
        serde_json::Value::String(type_name) => matches_type_name(value, type_name),
        serde_json::Value::Array(types) => types
            .iter()
            .any(|entry| entry.as_str().map(|type_name| matches_type_name(value, type_name)).unwrap_or(false)),
        _ => true,
    }
}

fn matches_type_name(value: &serde_json::Value, type_name: &str) -> bool {
    match type_name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

pub fn parameter_summary(parameters: &[ParameterSchema]) -> String {
    if parameters.is_empty() {
        return "none".to_string();
    }

    parameters
        .iter()
        .map(|p| {
            let requirement = if p.required { "required" } else { "optional" };
            format!("{}:{} ({})", p.name, p.param_type, requirement)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn default_when_to_use<T: Tool + ?Sized>(tool: &T) -> String {
    match tool.name() {
        "shell" => "Use for workspace-local shell commands, repository inspection, and build or test commands.".into(),
        "file_read" => "Use to read existing workspace files or inspect file contents.".into(),
        "file_write" => "Use to create new files or write file contents when the final artifact should live in the workspace.".into(),
        "file_edit" => "Use for targeted edits to existing workspace files.".into(),
        "glob_search" | "content_search" => "Use to find files or text patterns across the workspace.".into(),
        "compress" | "decompress" => "Use to create or unpack archives.".into(),
        "web_search_tool" => "Use to search the web for current or public information when a web search is needed.".into(),
        "web_fetch" => "Use to fetch a known URL and read its content.".into(),
        "http_request" => "Use to call HTTP endpoints directly when an API or connector is not already available.".into(),
        "browser" | "browser_interact" | "browser_network" | "browser_pdf" | "screenshot" => {
            "Use for website inspection, browser automation, page interaction, networked browser flows, or screenshot capture.".into()
        }
        "code_run" => "Use for short deterministic code snippets, calculations, and small scripts in supported runtimes.".into(),
        "diff" => "Use to compare content changes or inspect differences between versions.".into(),
        "patch" => "Use to apply targeted text patches or focused edits.".into(),
        "git_operations" => "Use for repository-aware actions like status, commit, branch, or history operations.".into(),
        "sql_query" => "Use for database queries, schema inspection, and controlled data analysis.".into(),
        "data_engine" => "Use for deterministic record pipelines, cleaning, scoring, aggregation, ranking, and schema-aligned extraction.".into(),
        "data_extractor" => "Use to extract structured fields from documents, HTML, or semi-structured text.".into(),
        "pdf_read" => "Use to read or inspect PDF content.".into(),
        "pdf_create" => "Use to generate PDF documents from structured content.".into(),
        "spreadsheet_read" => "Use to read spreadsheet rows, sheets, and structured table data.".into(),
        "spreadsheet_write" => "Use to write or update spreadsheet rows and sheets.".into(),
        "image_info" => "Use to inspect image metadata or identify image properties.".into(),
        "image_process" => "Use to transform, resize, annotate, or otherwise process images.".into(),
        "memory_consolidate" => {
            "Use to merge successful run history into durable topic memories, update the memory index, and prune stale memory.".into()
        }
        "memory_store" => "Use to persist useful agent memory, summaries, or durable notes.".into(),
        "memory_recall" => "Use to retrieve stored memory that may help the current task.".into(),
        "memory_forget" => "Use to remove stale or incorrect stored memory.".into(),
        "vector_store" => "Use to store embeddings or searchable text chunks for later retrieval.".into(),
        "vector_search" => "Use to search stored embeddings or retrieve semantically similar content.".into(),
        "vector_delete" => "Use to delete stored vector records or embeddings that should no longer remain searchable.".into(),
        "docker" => "Use for container lifecycle, containerized workflows, and image-based execution tasks.".into(),
        "kubernetes" => "Use for Kubernetes cluster inspection and resource management.".into(),
        "ssh_exec" => "Use for remote machine administration over SSH when a host needs direct command execution.".into(),
        "process_monitor" => "Use to inspect running processes, health, and local runtime activity.".into(),
        "mcp_session" => "Use to interact with an MCP-backed service or session." .into(),
        "search_mcp_registry" => "Use to discover available MCP services or registries.".into(),
        "acp_session" => "Use to interact with an ACP-backed integration session.".into(),
        "api_call" => "Use to call a registered API tool or API-backed integration.".into(),
        "register_api_tool" => "Use to register a reusable API-backed tool definition.".into(),
        "email" | "notification" | "pushover" => "Use to send outbound communication or alerts to people or systems.".into(),
        "ask_user" => "Use to ask a human a clarifying question or request confirmation.".into(),
        "crypto_tool" => "Use for hashing, encryption, signing, verification, or secret-safe cryptographic operations.".into(),
        "plane_guard" => "Use for policy, safety, and permission checks before risky actions.".into(),
        "request_credential" => "Use when a task needs credentials or a secret that must be requested explicitly.".into(),
        "schedule" | "cron_add" | "cron_list" | "cron_remove" | "cron_run" => {
            "Use to manage recurring or delayed scheduled jobs.".into()
        }
        "delegate" => "Use when a subtask can run independently or in parallel and the result will feed back into the main goal.".into(),
        "external_db" => "Use to query or update a tenant-approved external database integration.".into(),
        "external_api" => "Use to call a tenant-approved external REST or API integration.".into(),
        "list_connectors_in_category" => "Use to inspect which connectors exist in a category before picking one.".into(),
        "request_more_connectors" => "Use to ask for more connectors when the current catalog is insufficient.".into(),
        "request_more_tools" => "Use to expand a tool category when the current tool set is insufficient.".into(),
        "create_custom_connector" => "Use to define a new tenant connector when no built-in connector fits.".into(),
        "create_workspace_tool" => "Use in plan mode when the task genuinely needs a custom workspace tool definition.".into(),
        "run_registered_wasm" => "Use to run a tenant-approved registered WASM tool with strict resource limits.".into(),
        other => match tool.category() {
            "filesystem" => format!("Use for workspace-local file and shell workflows when '{other}' fits the task."),
            "web" => format!("Use for web access, browser work, and network retrieval when '{other}' fits the task."),
            "code" => format!("Use for code execution, patching, git operations, or SQL when '{other}' fits the task."),
            "data" => format!("Use for deterministic data extraction and record transforms when '{other}' fits the task."),
            "memory" => format!("Use for persistent memory and semantic retrieval when '{other}' fits the task."),
            "infra" => format!("Use for infrastructure administration when '{other}' fits the task."),
            "integration" => format!("Use for connector or API integration when '{other}' fits the task."),
            "communication" => format!("Use for outbound communication or human interaction when '{other}' fits the task."),
            "security" => format!("Use for safety or secret-handling operations when '{other}' fits the task."),
            "automation" => format!("Use for scheduling, delegation, or workflow automation when '{other}' fits the task."),
            _ => format!("Use when the tool named '{other}' is the best available exact match for the task."),
        },
    }
}

fn default_input_contract<T: Tool + ?Sized>(tool: &T) -> String {
    match tool.name() {
        "api_call" => "Request shape: { url, credential_key, method?, auth_type?, auth_header_name?, body?, headers? }. Use a stored credential key and send JSON request data only when needed.".into(),
        "mcp_session" => "Request shape: { server_url, action, tool_name?, tool_args?, auth_token? }. action must be connect, list_tools, or call_tool; call_tool requires tool_name and tool_args.".into(),
        "search_mcp_registry" => "Request shape: { query, limit? }. Provide a short registry search query for the MCP server catalog.".into(),
        "suggest_connectors" => "Request shape: { servers, reason, blocking?, credential_keys? }. servers must be a non-empty array of connector names or URLs.".into(),
        "list_connectors_in_category" => "Request shape: { category }. Use an exact connector category such as crm, communication, project_management, finance, or all.".into(),
        "request_more_connectors" => "Request shape: { category, reason, tried_connectors? }. Use this after reviewing the connector list and still needing more options.".into(),
        "create_custom_connector" => "Request shape: { name, category, creation_path, product_name?, base_url?, auth_type?, auth_header_name?, auth_credential_key?, api_docs?, endpoints?, summary? }. Pick one creation path and provide the matching fields.".into(),
        "create_workspace_tool" => "Request shape: { name, language, code, description?, timeout_secs?, input_schema? }. Use only in plan mode for approved custom workspace logic.".into(),
        "request_more_tools" => "Request shape: { categories, reason? }. categories must be a non-empty array of core tool categories such as filesystem, web, code, data, memory, infra, security, or automation.".into(),
        "acp_session" => "Request shape: { server_url, action, message?, target_agent?, auth_token? }. action must be list_agents or send_message for the current implementation.".into(),
        "register_api_tool" => "Request shape: { name, base_url, description?, auth_type?, auth_header_name?, endpoints?, credential_key? }. Register a reusable API-backed tool definition.".into(),
        "external_api" => "Request shape: { api, method, path, params?, headers?, tenant_id? }. Use a tenant-registered REST API and keep params JSON-serializable.".into(),
        "external_db" => "Request shape: { db, operation, sql?, table?, params?, max_rows?, allow_writes? }. operation must be schema, query, execute, table_preview, or explain.".into(),
        _ if tool.category().starts_with("connector/") => {
            format!(
                "Request shape: {{ operation, params?, tenant_id?, goal_instance_id?, step_index?, idempotency_key?, auth_token? }}. operation must be one of the connector's documented operations, and params must match that operation's JSON shape."
            )
        }
        _ => "Use the parameters listed above exactly; required fields must be present.".into(),
    }
}

fn default_output_contract<T: Tool + ?Sized>(tool: &T) -> String {
    match tool.name() {
        "api_call" => "Output: { status, body }. body is the HTTP response body as text (truncated if needed). On non-2xx responses, success is false and error explains the HTTP status.".into(),
        "mcp_session" => {
            "Output depends on action: connect => { connected, server, server_info, tool_count, tools }; list_tools => { tools, count }; call_tool => { tool, result }. The result field is the raw JSON returned by the MCP server.".into()
        }
        "search_mcp_registry" => "Output: { query, count, servers, tip }. servers is a list of discovered MCP servers with names, URLs, descriptions, categories, auth type, and connection status.".into(),
        "suggest_connectors" => "Output: { suggested, reason, blocking, credential_keys, operator_message, status, resume_endpoint }. This is a human-facing connector request envelope.".into(),
        "list_connectors_in_category" => "Output: { category, connectors, instruction } or { category, connectors, note }. connectors contains names and summaries for the category.".into(),
        "request_more_connectors" => "Output depends on state: { status, message } when more connectors are available, or { status, category, reason, options } / { status, category, reason, options, note } when the category is exhausted.".into(),
        "create_custom_connector" => "Output depends on the creation path: { status: 'created', name, category, message } or { status: 'pending', name, category, creation_path, note }. The executor later turns this into a live tenant connector.".into(),
        "create_workspace_tool" => "Output: { status, message }. The runtime intercepts this path and keeps it as plan-mode-only workflow setup.".into(),
        "request_more_tools" => "Output depends on expansion state: { status, requested_categories, tools_added, available_categories, message } or { status, requested_categories, note }.".into(),
        "acp_session" => "Output depends on action: list_agents => { agents }; send_message => { sent, to }. The agents field may contain the raw remote response payload.".into(),
        "register_api_tool" => "Output: { registered, tool_name, base_url }. This confirms a reusable API-backed tool definition was saved.".into(),
        "external_api" => "Output: { status, success, data, url }. data is the parsed response JSON from the tenant API on success.".into(),
        "external_db" => "Output depends on operation: schema => { schema, tables, table_count }; query / table_preview / explain => { rows, row_count, truncated, sql }; execute => { rows_affected, success }.".into(),
        _ if tool.category().starts_with("connector/") => {
            "Output is connector-specific JSON from the selected operation. Discovery tools may return connector metadata, but the actual payload depends on the connector and operation.".into()
        }
        _ => "Output is JSON matching the Output schema section below; check success and error for status.".into(),
    }
}

fn default_when_not_to_use<T: Tool + ?Sized>(tool: &T) -> String {
    match tool.name() {
        "shell" => "Avoid for destructive system-wide commands, long-running daemons, and tasks better expressed as structured code or data transforms.".into(),
        "file_read" => "Avoid when you need to write or edit files, or when the target is not a workspace file.".into(),
        "file_write" => "Avoid when you only need to read or inspect existing content.".into(),
        "file_edit" => "Avoid when a pure read is enough, or when the change is large enough to justify a more structured transform.".into(),
        "glob_search" | "content_search" => "Avoid when you already know the exact file or text location.".into(),
        "compress" | "decompress" => "Avoid for ordinary file operations that are not archive-related.".into(),
        "web_search_tool" => "Avoid when the answer is already available locally or in the workspace.".into(),
        "web_fetch" => "Avoid when you need broad search rather than a known URL.".into(),
        "http_request" => "Avoid when a dedicated API or connector already exists and is safer to use.".into(),
        "browser" | "browser_interact" | "browser_network" | "browser_pdf" | "screenshot" => {
            "Avoid when the task can be solved with an API, local file, or deterministic transform instead of a browser.".into()
        }
        "code_run" => "Avoid for multi-file applications, long-running services, or workflows that data_engine can express more safely.".into(),
        "diff" => "Avoid when you need to change content rather than compare it.".into(),
        "patch" => "Avoid for broad refactors or binary files; use a more structured editor path instead.".into(),
        "git_operations" => "Avoid when you only need to inspect files or data, not repository state.".into(),
        "sql_query" => "Avoid for destructive or unscoped writes; prefer read-only or tightly scoped queries.".into(),
        "data_engine" => "Avoid for free-form scripts, browser automation, remote execution, or arbitrary custom code.".into(),
        "data_extractor" => "Avoid when the task is just file reading or a deterministic data transform already fits data_engine.".into(),
        "pdf_read" => "Avoid when the source is not a PDF.".into(),
        "pdf_create" => "Avoid when the output should stay as text or a workspace document instead of a PDF.".into(),
        "spreadsheet_read" => "Avoid when the source is not tabular spreadsheet data.".into(),
        "spreadsheet_write" => "Avoid when you are not updating spreadsheet-like rows or sheets.".into(),
        "image_info" => "Avoid when the source is not an image.".into(),
        "image_process" => "Avoid when you do not need to transform image content.".into(),
        "memory_store" | "memory_consolidate" | "memory_recall" | "memory_forget" | "vector_store" | "vector_search" | "vector_delete" => {
            "Avoid as a substitute for durable source-of-truth storage or user-visible output.".into()
        }
        "docker" | "kubernetes" | "ssh_exec" | "process_monitor" => {
            "Avoid when the task belongs in workspace-local execution or a higher-level built-in tool.".into()
        }
        "mcp_session" | "search_mcp_registry" | "acp_session" | "api_call" | "register_api_tool" => {
            "Avoid when an existing built-in tool, connector, or local workflow already solves the task.".into()
        }
        "email" | "notification" | "pushover" => "Avoid for drafting, analysis, or internal state updates that should stay in the workspace.".into(),
        "ask_user" => "Avoid when the answer is already inferable from the available context.".into(),
        "crypto_tool" => "Avoid for ordinary application logic or general data cleanup.".into(),
        "plane_guard" => "Avoid when the action is already known to be safe and does not need policy evaluation.".into(),
        "request_credential" => "Avoid when the credential is already configured or unnecessary.".into(),
        "schedule" | "cron_add" | "cron_list" | "cron_remove" | "cron_run" => {
            "Avoid when the action is immediate or one-off rather than scheduled.".into()
        }
        "delegate" => "Avoid when the next step depends immediately on the result or when the task is too tightly coupled.".into(),
        "external_db" => "Avoid for local file work or when no tenant-approved database is required.".into(),
        "external_api" => "Avoid for local file work or when a safer built-in or connector tool already exists.".into(),
        "list_connectors_in_category" => "Avoid when the connector name is already known.".into(),
        "request_more_connectors" => "Avoid when a current connector already satisfies the task.".into(),
        "request_more_tools" => "Avoid when the current tool set already covers the job.".into(),
        "create_custom_connector" => "Avoid when a built-in or existing connector is sufficient.".into(),
        "create_workspace_tool" => "Avoid at runtime; it belongs in plan mode or pre-approval flows.".into(),
        "run_registered_wasm" => "Avoid for ad hoc code, unregistered modules, or general-purpose script execution.".into(),
        other => match tool.category() {
            "filesystem" => format!("Avoid when the task is not a workspace file or shell operation for '{other}'."),
            "web" => format!("Avoid when the answer is local or the task is not web-related for '{other}'."),
            "code" => format!("Avoid when a structured data or file workflow is a better fit than '{other}'."),
            "data" => format!("Avoid when the task is not a deterministic record or extraction workflow for '{other}'."),
            "memory" => format!("Avoid when you need a source of truth rather than stored context for '{other}'."),
            "infra" => format!("Avoid when the task is not infrastructure or host administration for '{other}'."),
            "integration" => format!("Avoid when a built-in local tool or connector already fits '{other}'."),
            "communication" => format!("Avoid when you are not sending or requesting human-facing communication for '{other}'."),
            "security" => format!("Avoid when the task is not secret- or policy-related for '{other}'."),
            "automation" => format!("Avoid when the task is not scheduled, delegated, or automation-related for '{other}'."),
            _ => format!("Avoid when another built-in tool is a better exact match than '{other}'."),
        },
    }
}

pub fn render_tool_contract(tool: &dyn Tool) -> String {
    let mut sections = vec![format!("Purpose: {}", tool.description().trim())];
    sections.push(format!("Input parameters: {}", parameter_summary(&tool.parameters_schema())));
    sections.push("Output: ToolResult { success, output, error }".to_string());

    if let Some(text) = tool.when_to_use().filter(|value| !value.trim().is_empty()) {
        sections.push(format!("Use when: {}", text.trim()));
    }
    if let Some(text) = tool.when_not_to_use().filter(|value| !value.trim().is_empty()) {
        sections.push(format!("Avoid when: {}", text.trim()));
    }
    if let Some(text) = tool.input_contract().filter(|value| !value.trim().is_empty()) {
        sections.push(format!("Input: {}", text.trim()));
    }
    if let Some(text) = tool.output_contract().filter(|value| !value.trim().is_empty()) {
        sections.push(format!("Output detail: {}", text.trim()));
    }

    if let Some(schema) = tool.output_schema() {
        sections.push(format!("Output schema: {}", render_output_schema(&schema)));
    }

    let examples = tool.examples();
    if !examples.is_empty() {
        let rendered_examples = examples.into_iter().take(2).collect::<Vec<_>>().join(" | ");
        sections.push(format!("Examples: {}", rendered_examples));
    }

    sections.join("\n")
}

pub fn tool_spec_from_tool(tool: &dyn Tool) -> crate::providers::ToolSpec {
    crate::providers::ToolSpec {
        name: tool.name().to_string(),
        description: render_tool_contract(tool),
        parameters: parameters_schema_to_json(&tool.parameters_schema()),
        output_schema: tool.output_schema(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool {
        tool_name: String,
        category_name: &'static str,
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
        fn output_schema(&self) -> Option<serde_json::Value> {
            Some(serde_json::json!({ "type": "string" }))
        }
        fn category(&self) -> &'static str {
            self.category_name
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::text("ok"))
        }
    }

    #[test]
    fn test_registry_register_get() {
        let mut registry = ToolRegistry::new();
        let tool = Arc::new(DummyTool { tool_name: "my_tool".into(), category_name: "other" });
        registry.register(tool);
        assert!(registry.get("my_tool").is_some());
    }

    #[test]
    fn test_registry_list() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool { tool_name: "alpha".into(), category_name: "other" }));
        registry.register(Arc::new(DummyTool { tool_name: "beta".into(), category_name: "other" }));
        let names = registry.list();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn test_render_tool_contract_includes_default_sections() {
        let tool = DummyTool { tool_name: "alpha".into(), category_name: "filesystem" };
        let contract = render_tool_contract(&tool);

        assert!(contract.contains("Purpose:"));
        assert!(contract.contains("Input parameters:"));
        assert!(contract.contains("Output:"));
        assert!(contract.contains("Use when:"));
        assert!(contract.contains("Avoid when:"));
        assert!(contract.contains("Input:"));
        assert!(contract.contains("Output detail:"));
        assert!(contract.contains("Output schema:"));
    }

    #[test]
    fn test_connector_tool_contract_is_specialized() {
        let tool = DummyTool { tool_name: "salesforce".into(), category_name: "connector/crm" };

        let input = tool.input_contract().unwrap();
        let output = tool.output_contract().unwrap();

        assert!(input.contains("operation"));
        assert!(input.contains("tenant_id"));
        assert!(output.contains("connector-specific JSON"));
    }

    #[test]
    fn test_api_call_contract_is_specialized() {
        let tool = DummyTool { tool_name: "api_call".into(), category_name: "integration" };

        let input = tool.input_contract().unwrap();
        let output = tool.output_contract().unwrap();

        assert!(input.contains("url"));
        assert!(input.contains("credential_key"));
        assert!(output.contains("status, body"));
    }

    #[test]
    fn test_validate_output_against_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["ok"],
            "properties": {
                "ok": { "type": "boolean" }
            },
            "additionalProperties": true
        });
        let value = serde_json::json!({ "ok": true, "extra": "allowed" });
        assert!(validate_output_against_schema("dummy", &value, &schema).is_ok());

        let bad = serde_json::json!({ "ok": "nope" });
        assert!(validate_output_against_schema("dummy", &bad, &schema).is_err());
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
