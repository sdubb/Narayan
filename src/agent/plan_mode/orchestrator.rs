//! Plan mode — the agent configuration conversation.
//!
//! Plan mode is the one-time setup phase where a user describes what an agent
//! should do in plain business language. The LLM infers the workflow, and the
//! plan-mode flow either asks the next missing question or turns structured
//! setup needs into inline cards. There is no separate planning service the user has
//! to interact with.
//! The user never sees tool names or connector IDs.
//!
//! ## Flow
//!
//!   POST /plan-mode/sessions          → create PlanModeSession
//!   POST /plan-mode/sessions/:id/turn → send user message, get assistant reply
//!   POST /plan-mode/sessions/:id/save → save AgentDefinition + AgentRole, close session
//!
//! ## Phases
//!
//!   CapturingIntent        → "What should this agent do?"
//!   ResolvingConnectors    → structured setup gate (DB / MCP / REST API / API key)
//!   CapturingClarifications → combined: trigger confirm + output questions + multi-role suggestion
//!   CapturingConstraints   → domain skill mandatory questions + user constraints
//!   Reviewing              → show the full config for user confirmation
//!   Complete               → save and close

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use base64::Engine as _;
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    agent::definition::{
        AgentDefinition, AgentDefinitionStatus, AgentRole, ExecutionStrategy, PermissionMode, PlanModeMessage,
        PlanModeCompilerStage, PlanModePhase, PlanModePreflightResult, PlanModeSandboxResult, PlanModeSession,
        PlanModeTestCheck,
        PlanModeTestConfidence, PlanModeTestResult, PlanModeTestStatus, PlanModeTestStepResult, RoleCategory,
        RoleStatus, TenantConnector, ToolPool, TriggerDef, TriggerType,
    },
    agent::planner::AdaptiveResearchMemo,
    agent::workflow_compiler::{CompilerResult, WorkflowCompiler},
    connectors::ConnectorInstallStore,
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    state::{SessionTask, SessionTaskOutput, SessionTaskResultStatus, SessionTaskStatus},
    storage::PostgresStore,
    tools::{toolregistry::dsl_generation_prompt_fragment, ToolRegistry},
    agent::plan_mode::registry::{
        build_capability_directory, build_detailed_capability_context, inferred_preferred_tools,
        missing_tool_categories,
    },
};

// ── Delegate to submodules ─────────────────────────────────────────────────
use crate::tools::connector_tool::ALL_CONNECTORS as BUILTIN_CONNECTORS;

// Import from decomposed modules
use super::intent::{compact_intent_snapshot, intent_named_external_db, intent_named_acp_peer,
    intent_needs_database_connection, intent_needs_api_connection,
    intent_needs_mcp_connection, intent_needs_acp_connection, AGENT_SUBSYSTEMS};
use super::registry::ConnectorResolver;

// ── IntentExtractor ────────────────────────────────────────────────────────

/// Extracts structured intent from a free-form business description.
/// Returns a JSON object with: data_sources, actions, trigger_hint, output_hint, constraints
pub struct IntentExtractor {
    gateway: Arc<dyn LlmGateway>,
}

impl IntentExtractor {
    pub fn new(gateway: Arc<dyn LlmGateway>) -> Self {
        Self { gateway }
    }

    pub async fn extract_initial(
        &self,
        session_id: &str,
        tenant_id: &str,
        description: &str,
        capability_directory: &str,
    ) -> Result<serde_json::Value> {
        let system =
            super::steps::intent_extractor_system_prompt(capability_directory, dsl_generation_prompt_fragment());
        let user = format!("Configure an agent to do:\n\n{}", description);

        let first_pass = GatewayRequest::new(
            session_id.to_string(),
            tenant_id.to_string(),
            TaskComplexity::Medium,
            vec![Message::system(system), Message::user(user)],
        );

        self.parse_json_response(self.gateway.chat(first_pass).await?.content.unwrap_or_default())
    }

    pub async fn refine(
        &self,
        session_id: &str,
        tenant_id: &str,
        description: &str,
        initial: &serde_json::Value,
        detailed_context: &str,
    ) -> Result<serde_json::Value> {
        let refine_system = format!(
            r#"You are repairing a previously inferred compiler draft.
Use the detailed capability context below to keep what was right, correct what was vague,
and choose exact tools/connectors where supported.
The detailed capability context is organized as three candidate slices. Prefer the most specific matching slice and only widen when necessary.
If a REGISTRY CANDIDATE SET JSON block is present, treat it as authoritative and choose tools/connectors only from those slices.

Return ONLY valid JSON with the exact same schema as before.

Detailed capability context:
{}

Rules:
- Preserve the original business intent unless the detailed context proves it impossible
- Fill preferred_tools with exact tool names only when the tool is clearly relevant
- Prefer data_engine for deterministic record workflows instead of inventing custom runtime code
- Tool contracts in the detailed context are authoritative; read Purpose, Use when, Avoid when, Input, Output, and Output schema before choosing a tool
- Use data_extractor first when the task is extracting fields from HTML/text/PDF-like content; use data_engine afterward for transforms and scoring
- Fill candidate_connectors with exact names only when the connector is clearly relevant
- Uploaded documents, local files, and workspace-only summaries remain connector-free unless the detailed context explicitly requires a connector.
- Keep missing_capabilities accurate if no installed/custom option satisfies the need
- Keep workflow_dsl ordered, typed, and practical
{}
"#,
            detailed_context,
            dsl_generation_prompt_fragment()
        );

        let refine_user = format!(
            "Original request:\n{}\n\nPreliminary inference summary:\n{}",
            description,
            serde_json::to_string(&compact_intent_snapshot(initial)).unwrap_or_else(|_| initial.to_string())
        );

        let second_pass = GatewayRequest::new(
            session_id.to_string(),
            tenant_id.to_string(),
            TaskComplexity::Medium,
            vec![Message::system(refine_system), Message::user(refine_user)],
        );

        self.parse_json_response(self.gateway.chat(second_pass).await?.content.unwrap_or_default())
    }

    fn parse_json_response(&self, raw: String) -> Result<serde_json::Value> {
        let cleaned = clean_json_markdown_response(&raw);

        serde_json::from_str(&cleaned).map_err(|e| {
            anyhow::anyhow!("intent extraction returned invalid JSON: {} — raw: {}", e, &raw[..raw.len().min(200)])
        })
    }
}

// ConnectorResolver is now in super::registry
// Intent helpers (intent_needs_*, intent_named_*, etc.) are now in super::intent
// ConnectorResolver::resolve, answer helpers, and intent helpers have been
// moved to super::registry and super::intent respectively.
// The orchestrator now imports them via `use super::registry::ConnectorResolver`.

// Re-export helpers that PlanModeManager methods still call directly.
use super::registry::{
    answer_declines_external_connector, answer_mentions_tenant_database,
    answer_mentions_tenant_api, answer_mentions_tenant_mcp, answer_mentions_tenant_acp,
    contains_connector_name, intent_prefers_local_document_workflow,
};

fn persist_selected_external_db(intent: &mut serde_json::Value, db_name: &str) {
    if let Some(intent_object) = intent.as_object_mut() {
        intent_object.insert("uses_external_db".into(), serde_json::json!(db_name));
    }
}

fn persist_selected_acp_peer(intent: &mut serde_json::Value, peer_name: &str) {
    if let Some(intent_object) = intent.as_object_mut() {
        intent_object.insert("uses_acp_peer".into(), serde_json::json!(peer_name));
    }
}

// Remaining intent helpers needed by handle_connector_clarification that weren't
// moved to intent.rs (they reference orchestrator-local state).
fn intent_text_for_keys(intent: &serde_json::Value, keys: &[&str]) -> String {
    super::intent::intent_text_for_keys_internal(intent, keys)
}

// ── PlanModeManager ────────────────────────────────────────────────────────
#[allow(dead_code)]
fn _removed_connector_resolver_impl() {}  // placeholder so git diff shows the removal

impl ConnectorResolver {
    /// Resolve which connectors and special tools are needed for the extracted intent.
    pub async fn resolve(
        intent: &serde_json::Value,
        installed: &[String],
        tenant_connectors: &[TenantConnector],
    ) -> (Vec<String>, Vec<String>, Option<String>) {
        let sources: Vec<String> = intent["data_sources"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
            .unwrap_or_default();
        let writes: Vec<String> = intent["write_targets"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
            .unwrap_or_default();
        let actions: Vec<String> = intent["actions"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
            .unwrap_or_default();

        let all_terms: Vec<&str> =
            sources.iter().chain(writes.iter()).chain(actions.iter()).map(String::as_str).collect();

        if intent_prefers_local_document_workflow(intent) {
            return (Vec::new(), Vec::new(), None);
        }

        let candidate_connectors: Vec<String> = intent["candidate_connectors"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let needed_connector_categories: Vec<String> = intent["needed_connector_categories"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let missing_capabilities: Vec<String> = intent["missing_capabilities"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        // ── Tool overrides for external_db, external_api, and ACP ─────────────
        let needs_db_connection = intent_needs_database_connection(intent);
        let needs_acp_connection = intent_needs_acp_connection(intent);
        let mut tool_overrides: Vec<String> = Vec::new();
        let database_connectors: Vec<&TenantConnector> =
            tenant_connectors.iter().filter(|tc| tc.category == "connector/database").collect();
        let acp_connectors: Vec<&TenantConnector> =
            tenant_connectors.iter().filter(|tc| tc.category.contains("acp") || tc.category.contains("agent")).collect();
        let explicit_db_name = intent_named_external_db(intent)
            .filter(|db_name| database_connectors.iter().any(|connector| connector.name == *db_name));
        let explicit_acp_name = intent_named_acp_peer(intent)
            .filter(|peer_name| acp_connectors.iter().any(|connector| connector.name == *peer_name));

        // If the intent explicitly named an external_db
        if let Some(db_name) = explicit_db_name.as_ref() {
            if !db_name.is_empty() && db_name != "null" {
                tool_overrides.push(format!("external_db:{}", db_name));
            }
        }
        // If the intent explicitly named an external_api
        if let Some(api_name) = intent["uses_external_api"].as_str() {
            if !api_name.is_empty() && api_name != "null" {
                tool_overrides.push(format!("external_api:{}", api_name));
            }
        }
        if let Some(peer_name) = explicit_acp_name.as_ref() {
            if !peer_name.is_empty() && peer_name != "null" {
                tool_overrides.push(format!("acp_session:{}", peer_name));
            }
        }

        // If the workflow needs a database and the tenant has multiple saved databases,
        // ask the user to choose one instead of silently enabling both.
        if explicit_db_name.is_none() && needs_db_connection {
            match database_connectors.as_slice() {
                [] => {}
                [only_db] => {
                    tool_overrides.push(format!("external_db:{}", only_db.name));
                }
                multiple => {
                    let names = multiple.iter().map(|tc| tc.name.clone()).collect::<Vec<_>>();
                    let question = format!(
                        "You have multiple database connections installed: {}. Which one should this agent use?",
                        names.join(", ")
                    );
                    return (Vec::new(), Vec::new(), Some(question));
                }
            }
        }

        if explicit_acp_name.is_none() && needs_acp_connection {
            match acp_connectors.as_slice() {
                [] => {
                    return (
                        Vec::new(),
                        Vec::new(),
                        Some(
                            "This workflow needs an ACP peer for internal agent-to-agent communication. Use the inline ACP connection card to add it, or tell me the exact saved ACP peer name if it already exists."
                                .into(),
                        ),
                    );
                }
                [only_acp] => {
                    tool_overrides.push(format!("acp_session:{}", only_acp.name));
                }
                multiple => {
                    let names = multiple.iter().map(|tc| tc.name.clone()).collect::<Vec<_>>();
                    let question = format!(
                        "You have multiple ACP peers installed for internal agent-to-agent communication: {}. Which one should this agent use?",
                        names.join(", ")
                    );
                    return (Vec::new(), Vec::new(), Some(question));
                }
            }
        }

        // ── Score built-in connectors ────────────────────────────────────
        let scored: Vec<(usize, &crate::tools::connector_tool::ConnectorDef)> = {
            let mut v: Vec<(usize, &crate::tools::connector_tool::ConnectorDef)> = BUILTIN_CONNECTORS
                .iter()
                .map(|entry| {
                    let score = entry
                        .keywords
                        .iter()
                        .filter(|kw| all_terms.iter().any(|t| t.contains(**kw) || kw.contains(t)))
                        .count();
                    (score, entry)
                })
                .filter(|(score, _)| *score > 0)
                .collect();
            v.sort_by(|a, b| b.0.cmp(&a.0));
            v
        };

        let mut resolved: Vec<String> = Vec::new();
        let mut ambiguous_categories: Vec<(&str, Vec<&str>)> = Vec::new();
        let mut resolved_categories: std::collections::HashSet<&str> = Default::default();

        if let Some(peer_name) = explicit_acp_name.as_ref() {
            if !resolved.iter().any(|name| name == peer_name) {
                resolved.push(peer_name.clone());
            }
        }
        if needs_acp_connection && explicit_acp_name.is_none() && acp_connectors.len() == 1 {
            let only_peer = acp_connectors[0].name.clone();
            if !resolved.iter().any(|name| name == &only_peer) {
                resolved.push(only_peer);
            }
        }

        for requested in &candidate_connectors {
            if installed.iter().any(|name| name == requested)
                || tenant_connectors.iter().any(|tc| tc.name == *requested)
            {
                resolved.push(requested.clone());
                if let Some(entry) = BUILTIN_CONNECTORS.iter().find(|entry| entry.name == requested.as_str()) {
                    resolved_categories.insert(entry.category);
                }
            }
        }

        for (_, entry) in &scored {
            let is_installed = installed.iter().any(|i| i == entry.name);
            if !is_installed {
                continue;
            }

            if resolved_categories.contains(entry.category) {
                if let Some(cat_entry) = ambiguous_categories.iter_mut().find(|(c, _)| *c == entry.category) {
                    cat_entry.1.push(entry.name);
                }
                continue;
            }
            resolved_categories.insert(entry.category);
            resolved.push(entry.name.to_string());
            ambiguous_categories.push((entry.category, vec![entry.name]));
        }

        // Add matching tenant custom connectors (non-database ones)
        for tc in tenant_connectors {
            if tc.category == "connector/database" {
                continue;
            } // handled as tool_override above
            if terms_match_connector(&all_terms, tc) && !resolved.contains(&tc.name) {
                resolved.push(tc.name.clone());
            }
        }

        // Build clarifying question if multiple connectors in same category
        let clarifying = ambiguous_categories
            .iter()
            .find(|(_, names)| names.len() > 1)
            .map(|(cat, names)| {
                let display_cat = cat.strip_prefix("connector/").unwrap_or(cat);
                format!(
                    "You have multiple {} integrations installed: {}. Which one should this agent use?",
                    display_cat,
                    names.join(", ")
                )
            })
            .or_else(|| {
                if explicit_db_name.is_none() && needs_db_connection && tool_overrides.iter().all(|spec| !spec.starts_with("external_db:")) {
                    Some(
                        "I think this workflow needs a database connection. Use the inline connection card to add it, or tell me the exact saved database name to use.".into(),
                    )
                } else {
                    None
                }
            })
            .or_else(|| {
                build_missing_connector_question(
                    &needed_connector_categories,
                    &missing_capabilities,
                    installed,
                    tenant_connectors,
                )
            });

        resolved.sort();
        resolved.dedup();
        tool_overrides.sort();
        tool_overrides.dedup();

        (resolved, tool_overrides, clarifying)
    }
}

fn build_missing_connector_question(
    needed_connector_categories: &[String],
    missing_capabilities: &[String],
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> Option<String> {
    for category in needed_connector_categories {
        let full_category = format!("connector/{}", category);
        let installed_builtin: Vec<&str> = BUILTIN_CONNECTORS
            .iter()
            .filter(|entry| entry.category == full_category)
            .filter(|entry| installed.iter().any(|name| name == entry.name))
            .map(|entry| entry.name)
            .collect();
        let installed_tenant: Vec<&str> = tenant_connectors
            .iter()
            .filter(|connector| connector.category == full_category)
            .map(|connector| connector.name.as_str())
            .collect();

        if installed_builtin.is_empty() && installed_tenant.is_empty() {
            let suggestions: Vec<&str> = BUILTIN_CONNECTORS
                .iter()
                .filter(|entry| entry.category == full_category)
                .map(|entry| entry.name)
                .take(3)
                .collect();
            let suggestion_text =
                if suggestions.is_empty() { "a custom connector".to_string() } else { suggestions.join(", ") };
            return Some(format!(
                "This sounds like it needs a {} connector, but none is installed. Should we use a custom database/API, or should you connect {}?",
                category,
                suggestion_text,
            ));
        }
    }

    if missing_capabilities.iter().any(|value| value == "custom_db") {
        return Some(
            "This may need a database connection. Use the inline connection card to add it, or tell me the exact saved database name if it already exists.".into()
        );
    }
    if missing_capabilities.iter().any(|value| value == "custom_api") {
        return Some(
            "This may need a custom REST API connection. Use the inline connection card to add it, or tell me the exact saved API name if it already exists.".into()
        );
    }
    if missing_capabilities.iter().any(|value| value == "connector/mcp") {
        return Some(
            "This may need an MCP server connection. Use the inline connection card to add it, or tell me the exact saved MCP server name if it already exists.".into()
        );
    }
    if missing_capabilities.iter().any(|value| value == "connector/acp") {
        return Some(
            "This may need an ACP peer connection for internal agent-to-agent communication. Use the inline connection card to add it, or tell me the exact saved ACP peer name if it already exists.".into()
        );
    }

    None
}

fn text_mentions_local_document_workflow(text: &str) -> bool {
    let lower = text.to_lowercase();
    let has_document_terms =
        ["document", "documents", "file", "files", "pdf", "csv", "spreadsheet", "attachment", "uploaded", "upload"]
            .iter()
            .any(|term| lower.contains(term));
    let has_read_terms =
        ["read", "review", "analyze", "analyse", "summarize", "summarise", "extract", "inspect", "highlight", "report"]
            .iter()
            .any(|term| lower.contains(term));
    has_document_terms && has_read_terms
}

fn intent_text_for_keys(intent: &serde_json::Value, keys: &[&str]) -> String {
    let mut text = String::new();

    for key in keys {
        if let Some(values) = intent[*key].as_array() {
            for value in values {
                if let Some(text_value) = value.as_str() {
                    text.push_str(text_value);
                    text.push(' ');
                } else if let Some(object) = value.as_object() {
                    if let Some(text_value) = object.get("description").and_then(|v| v.as_str()) {
                        text.push_str(text_value);
                        text.push(' ');
                    }
                    if let Some(text_value) = object.get("type").and_then(|v| v.as_str()) {
                        text.push_str(text_value);
                        text.push(' ');
                    }
                    if let Some(text_value) = object.get("tool_hint").or_else(|| object.get("tool")).and_then(|v| v.as_str()) {
                        text.push_str(text_value);
                        text.push(' ');
                    }
                    if let Some(text_value) = object.get("resource_hint").or_else(|| object.get("resource")).and_then(|v| v.as_str()) {
                        text.push_str(text_value);
                        text.push(' ');
                    }
                }
            }
        }
    }

    if let Some(steps) = intent["workflow_dsl"].as_array() {
        for value in steps {
            if let Some(object) = value.as_object() {
                if let Some(text_value) = object.get("description").and_then(|v| v.as_str()) {
                    text.push_str(text_value);
                    text.push(' ');
                }
                if let Some(text_value) = object.get("type").and_then(|v| v.as_str()) {
                    text.push_str(text_value);
                    text.push(' ');
                }
                if let Some(text_value) = object.get("tool_hint").or_else(|| object.get("tool")).and_then(|v| v.as_str()) {
                    text.push_str(text_value);
                    text.push(' ');
                }
                if let Some(text_value) = object.get("resource_hint").or_else(|| object.get("resource")).and_then(|v| v.as_str()) {
                    text.push_str(text_value);
                    text.push(' ');
                }
            }
        }
    }

    text
}

fn intent_prefers_local_document_workflow(intent: &serde_json::Value) -> bool {
    let mut text = intent_text_for_keys(intent, &["data_sources", "actions", "workflow_dsl"]);
    if let Some(output_hint) = intent["output_hint"].as_str() {
        text.push_str(output_hint);
        text.push(' ');
    }

    let write_targets_empty = intent["write_targets"].as_array().map(|arr| arr.is_empty()).unwrap_or(true);
    let output_hint = intent["output_hint"].as_str().unwrap_or("").to_lowercase();
    let local_output_hint = matches!(output_hint.as_str(), "" | "workspace" | "report") || output_hint.contains("chat");

    write_targets_empty && local_output_hint && text_mentions_local_document_workflow(&text)
}

fn intent_contains_database_terms(intent: &serde_json::Value) -> bool {
    let text = intent_text_for_keys(intent, &["data_sources", "write_targets", "actions", "workflow_dsl"]);
    let lower = text.to_lowercase();
    [
        "database",
        "postgres",
        "mysql",
        "sqlite",
        "sql",
        "schema",
        "table",
        "tables",
        "row",
        "rows",
        "connection string",
        "db connection",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

fn intent_contains_api_terms(intent: &serde_json::Value) -> bool {
    let text = intent_text_for_keys(intent, &["data_sources", "write_targets", "actions", "workflow_dsl"]);
    let lower = text.to_lowercase();
    ["rest api", "api", "endpoint", "endpoints", "backend", "http", "web service", "service api", "internal api"]
        .iter()
        .any(|term| lower.contains(term))
}

fn intent_contains_mcp_terms(intent: &serde_json::Value) -> bool {
    let text = intent_text_for_keys(intent, &["data_sources", "write_targets", "actions", "workflow_dsl"]);
    let lower = text.to_lowercase();
    ["mcp", "model context protocol", "tools/list", "tools/call", "json-rpc", "json rpc", "mcp server"]
        .iter()
        .any(|term| lower.contains(term))
}

fn intent_contains_acp_terms(intent: &serde_json::Value) -> bool {
    let text = intent_text_for_keys(intent, &["data_sources", "write_targets", "actions", "workflow_dsl"]);
    let lower = text.to_lowercase();
    [
        "acp",
        "agent communication protocol",
        "agent-to-agent",
        "agent to agent",
        "internal agent",
        "internal agents",
        "child agent",
        "parent agent",
        "teammate agent",
        "peer",
        "send message",
        "receive messages",
        "message another agent",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

pub(crate) fn intent_named_external_db(intent: &serde_json::Value) -> Option<String> {
    if let Some(db_name) = intent["uses_external_db"].as_str() {
        let trimmed = db_name.trim();
        if !trimmed.is_empty() && trimmed != "null" {
            return Some(trimmed.to_string());
        }
    }

    None
}

pub(crate) fn intent_named_acp_peer(intent: &serde_json::Value) -> Option<String> {
    if let Some(peer_name) = intent["uses_acp_peer"].as_str() {
        let trimmed = peer_name.trim();
        if !trimmed.is_empty() && trimmed != "null" {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn persist_selected_external_db(intent: &mut serde_json::Value, db_name: &str) {
    if let Some(intent_object) = intent.as_object_mut() {
        intent_object.insert("uses_external_db".into(), serde_json::json!(db_name));
    }
}

fn persist_selected_acp_peer(intent: &mut serde_json::Value, peer_name: &str) {
    if let Some(intent_object) = intent.as_object_mut() {
        intent_object.insert("uses_acp_peer".into(), serde_json::json!(peer_name));
    }
}

pub(crate) fn intent_needs_database_connection(intent: &serde_json::Value) -> bool {
    intent["missing_capabilities"]
        .as_array()
        .map(|arr| arr.iter().any(|value| value.as_str() == Some("custom_db")))
        .unwrap_or(false)
        || intent["uses_external_db"].as_bool().unwrap_or(false)
        || intent_contains_database_terms(intent)
}

pub(crate) fn intent_needs_api_connection(intent: &serde_json::Value) -> bool {
    intent["missing_capabilities"]
        .as_array()
        .map(|arr| arr.iter().any(|value| value.as_str() == Some("custom_api")))
        .unwrap_or(false)
        || intent["uses_external_api"].as_bool().unwrap_or(false)
        || intent["uses_external_api"]
            .as_str()
            .map(|value| {
                let trimmed = value.trim();
                !trimmed.is_empty() && trimmed != "null"
            })
            .unwrap_or(false)
        || intent_contains_api_terms(intent)
}

pub(crate) fn intent_needs_mcp_connection(intent: &serde_json::Value) -> bool {
    intent["missing_capabilities"]
        .as_array()
        .map(|arr| arr.iter().any(|value| value.as_str() == Some("connector/mcp")))
        .unwrap_or(false)
        || intent["needed_connector_categories"]
        .as_array()
        .map(|arr| arr.iter().any(|value| value.as_str() == Some("mcp")))
        .unwrap_or(false)
        || intent_contains_mcp_terms(intent)
}

pub(crate) fn intent_needs_acp_connection(intent: &serde_json::Value) -> bool {
    intent["missing_capabilities"]
        .as_array()
        .map(|arr| arr.iter().any(|value| value.as_str() == Some("connector/acp")))
        .unwrap_or(false)
        || intent["needed_connector_categories"]
            .as_array()
            .map(|arr| arr.iter().any(|value| value.as_str() == Some("acp")))
            .unwrap_or(false)
        || intent["uses_acp_peer"].as_bool().unwrap_or(false)
        || intent["uses_acp_peer"]
            .as_str()
            .map(|value| {
                let trimmed = value.trim();
                !trimmed.is_empty() && trimmed != "null"
            })
            .unwrap_or(false)
        || intent_contains_acp_terms(intent)
}

#[cfg(test)]
fn text_prefers_local_document_workflow(text: &str) -> bool {
    let lower = text.to_lowercase();
    text_mentions_local_document_workflow(&lower)
        && (lower.contains("no external")
            || lower.contains("never send")
            || lower.contains("never sends")
            || lower.contains("never write")
            || lower.contains("never writes")
            || lower.contains("read-only")
            || lower.contains("read only"))
}

fn answer_declines_external_connector(answer_lower: &str) -> bool {
    [
        "none",
        "no connector",
        "no external connector",
        "no external connectors",
        "built-in",
        "builtin",
        "local",
        "local only",
        "read-only",
        "read only",
        "workspace",
        "document",
        "documents",
        "file",
        "files",
        "uploaded file",
        "uploaded documents",
    ]
    .iter()
    .any(|phrase| answer_lower.contains(phrase))
}

fn answer_mentions_tenant_database(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category == "connector/database")
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

fn answer_mentions_tenant_api(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category != "connector/database" && !tc.category.contains("mcp"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

fn answer_mentions_tenant_mcp(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category.contains("mcp"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

fn answer_mentions_tenant_acp(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category.contains("acp") || tc.category.contains("agent"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

/// Returns true if any intent term meaningfully matches the connector's name/summary.
/// Uses proper tokenization (split on non-alphanumeric) rather than whitespace.
fn terms_match_connector(all_terms: &[&str], tc: &TenantConnector) -> bool {
    // Tokenize the summary into words
    let summary_words: Vec<String> =
        tc.summary.split(|c: char| !c.is_alphanumeric()).filter(|s| s.len() > 2).map(|s| s.to_lowercase()).collect();

    // Also include the connector name itself
    let name_lower = tc.name.to_lowercase();

    all_terms.iter().any(|term| {
        let term_lower = term.to_lowercase();
        // Exact name match
        term_lower == name_lower ||
        name_lower.contains(&term_lower) ||
        term_lower.contains(&name_lower) ||
        // Summary word match (both directions, min 4 chars to avoid noise)
        (term_lower.len() >= 4 && summary_words.iter().any(|w| {
            w.contains(&term_lower) || term_lower.contains(w.as_str())
        }))
    })
}

fn contains_connector_name(answer_lower: &str, connector_name: &str) -> bool {
    let name = connector_name.to_ascii_lowercase();
    answer_lower.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-').any(|token| token == name)
}

// ── PlanModeManager ────────────────────────────────────────────────────────

/// Manages the multi-turn plan mode conversation.
/// Each turn advances the session through PlanModePhase states.
pub struct PlanModeManager {
    gateway: Arc<dyn LlmGateway>,
    store: Arc<PostgresStore>,
    installs: Arc<ConnectorInstallStore>,
    tools: Arc<ToolRegistry>,
    workspace_root: PathBuf,
    extractor: IntentExtractor,
    skill_registry: Option<Arc<tokio::sync::RwLock<crate::skills::registry::SkillRegistry>>>,
}

impl PlanModeManager {
    pub fn new(
        gateway: Arc<dyn LlmGateway>,
        store: Arc<PostgresStore>,
        installs: Arc<ConnectorInstallStore>,
        tools: Arc<ToolRegistry>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        let extractor = IntentExtractor::new(Arc::clone(&gateway));
        Self { gateway, store, installs, tools, workspace_root: workspace_root.into(), extractor, skill_registry: None }
    }

    pub fn with_skill_registry(
        mut self,
        registry: Arc<tokio::sync::RwLock<crate::skills::registry::SkillRegistry>>,
    ) -> Self {
        self.skill_registry = Some(registry);
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Build the clarification step queue for the given intent, store it in the
    /// session, and return the first question. Shared by handle_intent and
    /// handle_connector_clarification.
    async fn build_step_queue_and_ask(&self, session: &mut PlanModeSession, intent: &serde_json::Value) -> String {
        let installed: Vec<String> = self
            .installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();

        // Load existing roles on this agent so the step pipeline can ask
        // about workforce event filters and depends_on ordering
        let existing_role_names: Vec<String> = self
            .store
            .list_roles_for_agent(&session.tenant_id, &session.draft_agent.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.name)
            .collect();

        let steps = super::steps::generate_steps(
            intent,
            intent["category"].as_str().unwrap_or("general"),
            &installed,
            &existing_role_names,
        );

        session.pending_steps = steps.iter().filter_map(|s| serde_json::to_value(s).ok()).collect();

        steps.first().map(|s| s.question.clone()).unwrap_or_else(|| "Any constraints or rules for this agent?".into())
    }

    fn build_clarification_refinement_context(&self, session: &PlanModeSession) -> String {
        let mut parts = Vec::new();

        let history: Vec<&PlanModeMessage> = session.conversation.iter().rev().take(8).collect();
        if !history.is_empty() {
            parts.push("PLAN MODE CONVERSATION (most recent last):".into());
            for message in history.into_iter().rev() {
                parts.push(format!("{}: {}", message.role, message.content));
            }
        }

        if let Some(role) = session.draft_role.as_ref() {
            parts.push("CURRENT DRAFT SNAPSHOT:".into());
            parts.push(format!("category: {}", role.role_category.as_str()));
            parts.push(format!(
                "connectors: {}",
                if role.connectors.is_empty() { "none".into() } else { role.connectors.join(", ") }
            ));
            parts.push(format!("tools: {}", if role.tools.is_empty() { "none".into() } else { role.tools.join(", ") }));
            parts.push(format!("trigger: {}", crate::agent::agent_chat::trigger_summary(&role.trigger)));
            parts.push(format!(
                "constraints: {}",
                if session.draft_agent.constraints.is_empty() {
                    "none".into()
                } else {
                    session.draft_agent.constraints.join("; ")
                }
            ));
        }

        if !session.pending_steps.is_empty() {
            let step_summaries: Vec<String> = session
                .pending_steps
                .iter()
                .filter_map(|value| {
                    serde_json::from_value::<super::steps::ClarificationStep>(value.clone()).ok()
                })
                .map(|step| format!("{} -> {}", step.id, step.question))
                .collect();
            if !step_summaries.is_empty() {
                parts.push("UNANSWERED CLARIFICATIONS:".into());
                parts.extend(step_summaries.into_iter().take(6));
            }
        }

        parts.join("\n")
    }

    async fn ensure_research_memo(&self, session: &mut PlanModeSession) -> Result<Option<AdaptiveResearchMemo>> {
        let Some(intent_value) = session.intent_cache.as_ref() else {
            return Ok(None);
        };
        let Some(intent_object) = intent_value.as_object() else {
            return Ok(None);
        };
        if let Some(existing) = intent_object.get("_adaptive_research_memo") {
            if let Ok(memo) = serde_json::from_value::<AdaptiveResearchMemo>(existing.clone()) {
                return Ok(Some(memo));
            }
        }

        let Some(role) = session.draft_role.as_ref() else {
            return Ok(None);
        };

        let research_context = self.build_plan_mode_research_context(session, role);
        let system = "You are synthesizing a plan-mode research memo for an automation role. \
Return strict JSON only. Do not produce executable runtime output. \
The memo must help compile the configuration into a deterministic workflow artifact.\n\n\
Required JSON shape:\n{\n  \"summary\": \"...\",\n  \"findings\": [\"...\"],\n  \"assumptions\": [\"...\"],\n  \"risks\": [\"...\"],\n  \"workflow_hints\": [\"...\"]\n}\n\n\
Rules:\n\
- Capture only durable planning signal, not chatty prose\n\
- workflow_hints must be short, ordered, and directly usable to shape a deterministic workflow\n\
- risks should focus on missing capabilities, approvals, connector gaps, or validation concerns\n\
- assumptions should name anything still inferred rather than confirmed"
            .to_string();
        let user = format!("Goal: {}\n\nPlanning research context:\n{}", role.purpose, research_context);

        let request = GatewayRequest::new(
            session.id.clone(),
            session.tenant_id.clone(),
            TaskComplexity::Medium,
            vec![Message::system(system), Message::user(user)],
        );
        let raw = self.gateway.chat(request).await?.content.unwrap_or_default();
        let cleaned = clean_json_markdown_response(&raw);
        let memo = match serde_json::from_str::<AdaptiveResearchMemo>(&cleaned) {
            Ok(memo) => memo,
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    error = %error,
                    "plan mode research memo failed to parse, using fallback synthesis"
                );
                fallback_plan_mode_research_memo(role, intent_value)
            }
        };

        if let Some(intent_object) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
            intent_object.insert("_adaptive_research_memo".into(), serde_json::to_value(&memo)?);
        }
        Ok(Some(memo))
    }

    fn build_plan_mode_research_context(&self, session: &PlanModeSession, role: &AgentRole) -> String {
        let mut parts = Vec::new();
        parts.push(format!("Category: {}", role.role_category.as_str()));
        parts.push(format!("Trigger: {}", crate::agent::agent_chat::trigger_summary(&role.trigger)));
        parts.push(format!(
            "Connectors: {}",
            if role.connectors.is_empty() { "none".into() } else { role.connectors.join(", ") }
        ));
        parts.push(format!("Tools: {}", if role.tools.is_empty() { "none".into() } else { role.tools.join(", ") }));

        if let Some(intent) = session.intent_cache.as_ref() {
            parts.push(format!(
                "Intent JSON:\n{}",
                serde_json::to_string_pretty(intent).unwrap_or_else(|_| intent.to_string())
            ));
        }
        if !session.attachment_context.trim().is_empty() {
            parts.push(format!("Attachment context:\n{}", session.attachment_context));
        }
        if !session.draft_agent.constraints.is_empty() {
            parts.push(format!("Constraints:\n- {}", session.draft_agent.constraints.join("\n- ")));
        }
        let history: Vec<&PlanModeMessage> = session.conversation.iter().rev().take(10).collect();
        if !history.is_empty() {
            parts.push("Recent plan-mode conversation:".into());
            for message in history.into_iter().rev() {
                parts.push(format!("{}: {}", message.role, message.content));
            }
        }
        parts.join("\n\n")
    }

    async fn refine_plan_after_clarifications(&self, session: &mut PlanModeSession) -> Result<Option<String>> {
        let Some(initial_intent) = session.intent_cache.clone() else {
            return Ok(None);
        };
        let description = session
            .conversation
            .iter()
            .find(|message| message.role == "user")
            .map(|message| message.content.clone())
            .or_else(|| session.draft_role.as_ref().map(|role| role.purpose.clone()))
            .unwrap_or_default();

        let detail_context = self.build_clarification_refinement_context(session);
        let store = self.store.as_ref();
        let Some(role) = session.draft_role.as_mut() else {
            return Ok(None);
        };

        let mut refined = self
            .extractor
            .refine(&session.id, &session.tenant_id, &description, &initial_intent, &detail_context)
            .await?;
        session.intent_cache = Some(refined.clone());
        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
            intent.remove("_adaptive_research_memo");
        }

        let installed: Vec<String> = self
            .installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();
        let tenant_connectors: Vec<TenantConnector> =
            store.list_tenant_connectors(&session.tenant_id).await.unwrap_or_default();

        let previous_category = role.role_category.clone();
        let previous_limits = role.execution_limits.clone();
        let previous_memory_scope = role.memory_scope.clone();
        role.role_category = RoleCategory::from_slug(refined["category"].as_str().unwrap_or("general"));
        if previous_memory_scope == previous_category.default_memory_scope() {
            role.memory_scope = role.role_category.default_memory_scope();
        }
        if previous_limits == previous_category.default_execution_limits() {
            role.execution_limits = role.role_category.default_execution_limits();
        }
        crate::agent::plan_mode::review::apply_role_policy_defaults(&mut session.draft_agent, role);

        let (resolved_connectors, tool_overrides, clarifying_q) =
            ConnectorResolver::resolve(&refined, &installed, &tenant_connectors).await;
        session.draft_agent.connectors = resolved_connectors.clone();
        role.connectors = resolved_connectors;
        if let Some(db_name) = tool_overrides.iter().find_map(|spec| spec.strip_prefix("external_db:")) {
            persist_selected_external_db(&mut refined, db_name);
        }

        let mut inferred_tools = inferred_preferred_tools(&self.tools, &refined);
        for tool_override in &tool_overrides {
            if !inferred_tools.contains(tool_override) {
                inferred_tools.push(tool_override.clone());
            }
        }
        if !inferred_tools.is_empty() {
            for tool_name in inferred_tools {
                if !role.tools.iter().any(|tool| tool == &tool_name) {
                    role.tools.push(tool_name);
                }
            }
            role.tools.sort();
            role.tools.dedup();
        }

        role.execution_guidelines.compiled_workflow = None;
        apply_execution_hints(role, &refined);
        // Only overwrite the trigger if the user hasn't already confirmed it
        // during the clarification step (confidence == High). Otherwise the
        // refinement LLM response may omit or null-out trigger_cron, discarding
        // the user's explicit answer (e.g. "every minute" → "* * * * *").
        let user_confirmed_trigger = role.trigger.confidence == crate::agent::definition::TriggerConfidence::High
            && role.trigger.trigger_type != TriggerType::Manual;
        if !user_confirmed_trigger {
            let (trigger, confidence) = intent_to_trigger(&refined);
            role.trigger = trigger;
            role.trigger.confidence = confidence;
        } else {
            // Sync the confirmed trigger's cron back into the refined intent
            // so downstream consumers (fingerprint, review summary) stay consistent.
            if let Some(obj) = refined.as_object_mut() {
                if let Some(cron) = role.trigger.cron.as_deref() {
                    obj.insert("trigger_cron".into(), serde_json::json!(cron));
                }
                obj.insert(
                    "trigger_hint".into(),
                    serde_json::json!(match role.trigger.trigger_type {
                        TriggerType::Schedule => "schedule",
                        TriggerType::Webhook => "webhook",
                        TriggerType::UserMessage => "user_message",
                        TriggerType::WorkforceEvent => "workforce_event",
                        TriggerType::Manual => "manual",
                    }),
                );
                obj.insert("trigger_confidence".into(), serde_json::json!("high"));
            }
            session.intent_cache = Some(refined.clone());
        }
        session.goal_fingerprint = Some(compute_plan_mode_goal_fingerprint(&description, &refined, role));

        Ok(clarifying_q)
    }

    /// Look up the domain plan-mode skill for the given intent category.
    async fn domain_skill_text(&self, category: &str) -> Option<String> {
        let reg = self.skill_registry.as_ref()?.read().await;
        // Domain skills are named "planmode:<category>"
        let key = format!("planmode:{}", category);
        if let Some(skill) = reg.get(&key) {
            let text = skill.steps.iter().map(|s| s.description()).collect::<Vec<_>>().join("\n\n");
            return Some(text);
        }
        // Fallback: fuzzy match via aliases
        reg.find_matching(category)
            .map(|skill| skill.steps.iter().map(|s| s.description()).collect::<Vec<_>>().join("\n\n"))
    }

    async fn superpowers_guidance_text(&self, phase: &PlanModePhase) -> Option<String> {
        let names = superpowers_skill_names_for_phase(phase);
        if names.is_empty() {
            return None;
        }

        let reg = self.skill_registry.as_ref()?.read().await;
        let mut sections = Vec::new();

        for name in names {
            if let Some(skill) = reg.get(name) {
                let body =
                    skill.steps.iter().map(|step| step.description().to_string()).collect::<Vec<_>>().join("\n- ");
                sections.push(format!("{}:\n- {}", skill.name, body));
            }
        }

        if sections.is_empty() {
            None
        } else {
            Some(sections.join("\n\n"))
        }
    }

    pub async fn test(&self, session: &PlanModeSession) -> Result<PlanModeTestResult> {
        let mut role = match session.draft_role.as_ref() {
            Some(role) => role.clone(),
            None => {
                return Ok(PlanModeTestResult {
                    status: PlanModeTestStatus::Fail,
                    confidence: PlanModeTestConfidence::Low,
                    preflight: PlanModePreflightResult {
                        status: PlanModeTestStatus::Fail,
                        checks: vec![PlanModeTestCheck {
                            label: "draft role exists".into(),
                            success: false,
                            detail: Some("no draft role is available for testing".into()),
                        }],
                        summary: "No draft role exists yet, so the workflow cannot be tested.".into(),
                    },
                    sandbox: PlanModeSandboxResult {
                        status: PlanModeTestStatus::Fail,
                        steps: Vec::new(),
                        summary: "Sandbox skipped because the draft role is missing.".into(),
                    },
                    steps: Vec::new(),
                    criteria_checks: vec![],
                    summary: "No draft role exists yet, so the workflow cannot be tested.".into(),
                });
            }
        };

        if role.execution_guidelines.compiled_workflow.is_none() {
            let intent = session.intent_cache.as_ref().cloned().unwrap_or_else(|| serde_json::json!({}));
            match WorkflowCompiler::compile(&role, &intent, &self.tools) {
                Ok(CompilerResult::Ready(compiled)) => {
                    role.execution_guidelines.compiled_workflow = Some(compiled);
                }
                Ok(CompilerResult::NeedsCard(card)) => {
                    let summary = format!(
                        "Compiler needs setup card before the draft can be tested: {} ({})",
                        card.card_type, card.binding_target
                    );
                    return Ok(PlanModeTestResult {
                        status: PlanModeTestStatus::Fail,
                        confidence: PlanModeTestConfidence::Low,
                        preflight: PlanModePreflightResult {
                            status: PlanModeTestStatus::Fail,
                            checks: vec![PlanModeTestCheck {
                                label: "compiled workflow available".into(),
                                success: false,
                                detail: Some(summary.clone()),
                            }],
                            summary: summary.clone(),
                        },
                        sandbox: PlanModeSandboxResult {
                            status: PlanModeTestStatus::Fail,
                            steps: Vec::new(),
                            summary: "Sandbox skipped because the compiler needs setup first.".into(),
                        },
                        steps: Vec::new(),
                        criteria_checks: vec![],
                        summary,
                    });
                }
                Err(error) => {
                    let summary = format!("Compiler failed before sandboxing: {}", error);
                    return Ok(PlanModeTestResult {
                        status: PlanModeTestStatus::Fail,
                        confidence: PlanModeTestConfidence::Low,
                        preflight: PlanModePreflightResult {
                            status: PlanModeTestStatus::Fail,
                            checks: vec![PlanModeTestCheck {
                                label: "compiled workflow available".into(),
                                success: false,
                                detail: Some(summary.clone()),
                            }],
                            summary: summary.clone(),
                        },
                        sandbox: PlanModeSandboxResult {
                            status: PlanModeTestStatus::Fail,
                            steps: Vec::new(),
                            summary: "Sandbox skipped because compiler validation failed.".into(),
                        },
                        steps: Vec::new(),
                        criteria_checks: vec![],
                        summary,
                    });
                }
            }
        }

        let workspace_root = session.session_workspace.as_ref().map(PathBuf::from).unwrap_or_else(|| {
            plan_mode_workspace_root(&self.workspace_root, &session.tenant_id, &session.draft_agent.id)
        });
        tokio::fs::create_dir_all(workspace_root.join("files")).await?;
        tokio::fs::create_dir_all(workspace_root.join("artifacts")).await?;
        tokio::fs::create_dir_all(workspace_root.join("logs")).await?;
        let sandbox_input_path = workspace_root.join("artifacts").join("sandbox_input.txt");
        let sandbox_output_path = workspace_root.join("artifacts").join("sandbox_output.txt");
        let _ = tokio::fs::write(
            &sandbox_input_path,
            b"Sandbox fixture input. This file exists so read-only steps can be exercised safely.",
        )
        .await;
        let _ = tokio::fs::write(&sandbox_output_path, b"").await;

        let _synthetic_input = synthetic_input_data_for_role(&role, session, &workspace_root);
        let plan = match role.execution_guidelines.compiled_workflow.as_ref() {
            Some(compiled) => crate::agent::planner::Plan::from_compiled_workflow(compiled, &role),
            None => {
                return Ok(PlanModeTestResult {
                    status: PlanModeTestStatus::Fail,
                    confidence: PlanModeTestConfidence::Low,
                    preflight: PlanModePreflightResult {
                        status: PlanModeTestStatus::Fail,
                        checks: vec![PlanModeTestCheck {
                            label: "compiled workflow".into(),
                            success: false,
                            detail: Some("compiler preview did not produce a compiled workflow artifact".into()),
                        }],
                        summary: "No compiled workflow artifact is available for testing.".into(),
                    },
                    sandbox: PlanModeSandboxResult {
                        status: PlanModeTestStatus::Fail,
                        steps: Vec::new(),
                        summary: "Sandbox skipped because no compiled workflow was available.".into(),
                    },
                    steps: Vec::new(),
                    criteria_checks: vec![],
                    summary: "No compiled workflow artifact is available for testing.".into(),
                });
            }
        };

        let preflight = self.preflight_workflow(&plan, &role).await;
        let sandbox = if matches!(preflight.status, PlanModeTestStatus::Fail) {
            PlanModeSandboxResult {
                status: PlanModeTestStatus::Fail,
                steps: Vec::new(),
                summary: "Sandbox skipped because preflight failed.".into(),
            }
        } else {
            self.run_sandbox(&plan, &role, &workspace_root).await
        };

        let status = combine_test_status(&preflight.status, &sandbox.status);
        let confidence = match status {
            PlanModeTestStatus::Pass => PlanModeTestConfidence::High,
            PlanModeTestStatus::Partial => PlanModeTestConfidence::Partial,
            PlanModeTestStatus::Fail => PlanModeTestConfidence::Low,
        };

        let mut criteria_checks = preflight.checks.clone();
        criteria_checks.extend(sandbox.steps.iter().map(|step| PlanModeTestCheck {
            label: format!("step {}: {}", step.step + 1, step.description),
            success: step.success && !step.blocked,
            detail: step.error.clone().or_else(|| Some(step.output.to_string())),
        }));

        let mut summary_parts = vec![preflight.summary.clone(), sandbox.summary.clone()];
        if let Some(guidance) = self.superpowers_guidance_text(&PlanModePhase::Reviewing).await {
            if !guidance.trim().is_empty() {
                summary_parts.push(format!("Review guidance:\n{}", guidance));
            }
        }

        Ok(PlanModeTestResult {
            status,
            confidence,
            preflight,
            sandbox: sandbox.clone(),
            steps: sandbox.steps,
            criteria_checks,
            summary: summary_parts.join("\n\n"),
        })
    }

    /// Feed a failing/partial test result back through plan mode so the LLM can repair the draft.
    pub async fn revise_from_test_result(
        &self,
        mut session: PlanModeSession,
        test_result: &PlanModeTestResult,
    ) -> Result<(String, PlanModeSession)> {
        session.phase = PlanModePhase::Reviewing;
        let prompt = build_revision_prompt_from_test_result(test_result);
        self.turn(session, &prompt).await
    }

    /// Create a new plan mode session for a tenant.
    pub fn new_session(&self, tenant_id: &str, agent_name: &str) -> PlanModeSession {
        let session_id = Uuid::new_v4().to_string();
        let agent_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let session_workspace =
            plan_mode_workspace_root(&self.workspace_root, tenant_id, &agent_id).display().to_string();

        let mut draft_agent = AgentDefinition::new(agent_id, tenant_id.to_string(), agent_name.to_string());
        draft_agent.memory_ref = format!("agent:{}", &draft_agent.id[..8]);

        PlanModeSession {
            id: session_id.clone(),
            tenant_id: tenant_id.to_string(),
            draft_agent,
            draft_role: None,
            conversation: Vec::new(),
            attachments: Vec::new(),
            attachment_context: String::new(),
            session_workspace: Some(session_workspace),
            goal_fingerprint: None,
            repair_version: 1,
            reused_from_session_id: None,
            repair_root_session_id: Some(session_id.clone()),
            phase: PlanModePhase::CapturingIntent,
            compiler_stage: crate::agent::definition::PlanModeCompilerStage::Intent,
            compiler_repair_passes: 0,
            compiler_validation_issues: Vec::new(),
            intent_cache: None,
            pending_steps: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Process one user turn.  Returns the assistant's reply and the updated session.
    pub async fn turn(&self, mut session: PlanModeSession, user_message: &str) -> Result<(String, PlanModeSession)> {
        session.conversation.push(PlanModeMessage { role: "user".into(), content: user_message.to_string() });

        let reply = match session.phase {
            PlanModePhase::CapturingIntent => self.handle_intent(&mut session, user_message).await?,
            PlanModePhase::ResolvingConnectors => {
                // User answered the connector clarification question
                self.handle_connector_clarification(&mut session, user_message).await?
            }
            PlanModePhase::CapturingClarifications => {
                // User answered the combined trigger/output/multi-role questions
                self.handle_clarifications(&mut session, user_message).await?
            }
            PlanModePhase::CapturingConstraints => {
                // Compatibility fallback phase: newer flows generally capture
                // constraints inside the clarification step pipeline.
                self.handle_constraints(&mut session, user_message).await?
            }
            PlanModePhase::Reviewing => self.handle_review(&mut session, user_message).await?,
            PlanModePhase::Complete => "This session is complete. The agent has been saved.".into(),
        };

        session.conversation.push(PlanModeMessage { role: "assistant".into(), content: reply.clone() });
        session.updated_at = Utc::now();

        Ok((reply, session))
    }

    /// Persist uploaded documents for this plan-mode session and build a short
    /// extracted context block for the LLM.
    pub async fn ingest_attachments(
        &self,
        session: &mut PlanModeSession,
        uploads: Vec<crate::agent::definition::PlanModeAttachmentUpload>,
        file_cap_bytes: Option<u64>,
        workspace_cap_bytes: Option<u64>,
    ) -> Result<Vec<crate::agent::definition::PlanModeAttachment>> {
        let root = self.ensure_session_workspace(session).await?;
        if uploads.is_empty() {
            return Ok(Vec::new());
        }

        let files_root = root.join("files");
        tokio::fs::create_dir_all(&files_root).await?;
        let mut current_workspace_bytes = directory_size_bytes(&files_root)?;

        let mut created = Vec::new();
        let mut snippets = Vec::new();

        for upload in uploads {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(upload.content_base64.trim())
                .map_err(|e| anyhow::anyhow!("failed to decode attachment '{}': {}", upload.name, e))?;

            let size_bytes = decoded.len() as u64;
            if let Some(limit) = file_cap_bytes {
                if size_bytes > limit {
                    return Err(anyhow::anyhow!(
                        "attachment '{}' is too large ({} bytes, max {} bytes)",
                        upload.name,
                        size_bytes,
                        limit
                    ));
                }
            }
            if let Some(limit) = workspace_cap_bytes {
                let projected = current_workspace_bytes.saturating_add(size_bytes);
                if projected > limit {
                    return Err(anyhow::anyhow!(
                        "workspace storage cap reached ({} bytes used, {} byte file would exceed max {})",
                        current_workspace_bytes,
                        size_bytes,
                        limit
                    ));
                }
            }

            let name = sanitise_attachment_name(&upload.name);
            let path = unique_session_attachment_path(&files_root, &name).await?;
            tokio::fs::write(&path, &decoded).await?;
            current_workspace_bytes = current_workspace_bytes.saturating_add(size_bytes);

            let kind = infer_plan_mode_attachment_kind(&path, upload.mime_type.as_deref());
            let preview = self
                .extract_attachment_preview(&path, &kind)
                .await
                .unwrap_or_else(|e| format!("extraction failed: {}", e));
            let preview = crate::util::truncate(&preview, 4000).to_string();
            let size_bytes = decoded.len() as u64;

            let relative_path = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().to_string();

            let attachment = crate::agent::definition::PlanModeAttachment {
                name: name.clone(),
                path: relative_path,
                mime_type: upload.mime_type.clone(),
                size_bytes,
                kind: kind.clone(),
                extracted_preview: preview.clone(),
                uploaded_at: Utc::now(),
            };

            snippets.push(format!(
                "Attachment: {} ({}, {} bytes)\n{}",
                attachment.name,
                attachment_kind_label(&attachment.kind),
                attachment.size_bytes,
                preview
            ));

            created.push(attachment);
        }

        session.attachments.extend(created.clone());
        if !snippets.is_empty() {
            if !session.attachment_context.is_empty() {
                session.attachment_context.push_str("\n\n");
            }
            session.attachment_context.push_str(&snippets.join("\n\n"));
        }

        Ok(created)
    }

    async fn ensure_session_workspace(&self, session: &mut PlanModeSession) -> Result<PathBuf> {
        let root = session.session_workspace.as_ref().map(PathBuf::from).unwrap_or_else(|| {
            plan_mode_workspace_root(&self.workspace_root, &session.tenant_id, &session.draft_agent.id)
        });

        tokio::fs::create_dir_all(root.join("files")).await?;
        tokio::fs::create_dir_all(root.join("artifacts")).await?;
        tokio::fs::create_dir_all(root.join("logs")).await?;

        if session.session_workspace.is_none() {
            session.session_workspace = Some(root.display().to_string());
        }

        Ok(root)
    }

    async fn extract_attachment_preview(
        &self,
        path: &Path,
        kind: &crate::agent::definition::PlanModeAttachmentKind,
    ) -> Result<String> {
        let path_str = path.display().to_string();
        let tool_name = match kind {
            crate::agent::definition::PlanModeAttachmentKind::Pdf => "pdf_read",
            crate::agent::definition::PlanModeAttachmentKind::Spreadsheet => "spreadsheet_read",
            crate::agent::definition::PlanModeAttachmentKind::Csv => "file_read",
            crate::agent::definition::PlanModeAttachmentKind::Text => "file_read",
            crate::agent::definition::PlanModeAttachmentKind::Binary => "",
            crate::agent::definition::PlanModeAttachmentKind::Unknown => "file_read",
        };

        if tool_name.is_empty() {
            return Ok(format!("binary attachment saved at {}", path_str));
        }

        let mut args = serde_json::json!({ "path": path_str });
        if tool_name == "file_read" {
            if matches!(kind, crate::agent::definition::PlanModeAttachmentKind::Csv) {
                args["start_line"] = serde_json::json!(1);
                args["end_line"] = serde_json::json!(60);
            } else {
                args["start_line"] = serde_json::json!(1);
                args["end_line"] = serde_json::json!(200);
            }
        } else if tool_name == "spreadsheet_read" {
            args["max_rows"] = serde_json::json!(50);
            args["header_row"] = serde_json::json!(true);
        }

        let Some(tool) = self.tools.get(tool_name) else {
            return Ok(format!("extraction tool '{}' is unavailable; file saved at {}", tool_name, path_str));
        };

        let result = tool.execute(args).await?;
        if !result.success {
            return Ok(format!(
                "extraction tool '{}' failed for {}: {}",
                tool_name,
                path_str,
                result.error.unwrap_or_else(|| "unknown error".into())
            ));
        }

        Ok(serde_json::to_string_pretty(&result.output).unwrap_or_else(|_| result.output.to_string()))
    }

    // ── Phase handlers ─────────────────────────────────────────────────────

    async fn handle_intent(&self, session: &mut PlanModeSession, description: &str) -> Result<String> {
        // Load tenant's custom connections upfront — used for both context injection
        // and connector resolution
        let installed: Vec<String> = self
            .installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();

        let user_description = description.trim().to_string();
        let mut description = combine_user_message_with_attachment_context(description, &session.attachment_context);
        if let Some(guidance) = self.superpowers_guidance_text(&PlanModePhase::CapturingIntent).await {
            if !guidance.trim().is_empty() {
                description.push_str("\n\nINTERNAL PLANNING GUIDANCE:\n");
                description.push_str(&guidance);
            }
        }

        let tenant_connectors: Vec<TenantConnector> =
            self.store.list_tenant_connectors(&session.tenant_id).await.unwrap_or_default();
        let capability_directory =
            crate::agent::plan_mode::registry::build_capability_directory(&self.tools, &installed, &tenant_connectors);
        let initial_intent = self
            .extractor
            .extract_initial(&session.id, &session.tenant_id, &description, &capability_directory)
            .await?;
        let detail_context = crate::agent::plan_mode::registry::build_detailed_capability_context(
            &self.tools,
            &initial_intent,
            &installed,
            &tenant_connectors,
        );
        let intent = if detail_context.trim().is_empty() {
            initial_intent
        } else {
            self.extractor
                .refine(&session.id, &session.tenant_id, &description, &initial_intent, &detail_context)
                .await?
        };

        // Store intent in the draft role
        let role_id = Uuid::new_v4().to_string();
        let mut role =
            AgentRole::new(role_id, session.draft_agent.id.clone(), session.tenant_id.clone(), "Primary Role".into());
        role.purpose = user_description.clone();
        role.role_category = RoleCategory::from_slug(intent["category"].as_str().unwrap_or("general"));
        crate::agent::plan_mode::review::apply_role_policy_defaults(&mut session.draft_agent, &mut role);

        // Resolve connectors and tool overrides
        let (resolved_connectors, tool_overrides, clarifying_q) =
            ConnectorResolver::resolve(&intent, &installed, &tenant_connectors).await;

        // Set connectors on agent (allowed universe) and role (relevant subset)
        session.draft_agent.connectors = resolved_connectors.clone();
        role.connectors = resolved_connectors.clone();

        let mut inferred_tools = inferred_preferred_tools(&self.tools, &intent);
        for tool_override in &tool_overrides {
            if !inferred_tools.contains(tool_override) {
                inferred_tools.push(tool_override.clone());
            }
        }
        if !inferred_tools.is_empty() {
            for tool_name in inferred_tools {
                if !role.tools.iter().any(|tool| tool == &tool_name) {
                    role.tools.push(tool_name);
                }
            }
            role.tools.sort();
            role.tools.dedup();
        }

        // Build execution guidelines from actions
        let mut guidelines: Vec<String> = Vec::new();
        if let Some(actions) = intent["actions"].as_array() {
            guidelines.extend(actions.iter().filter_map(|a| a.as_str().map(String::from)));
        }
        // Add tool hints for external connections
        for override_spec in &tool_overrides {
            if let Some(db_name) = override_spec.strip_prefix("external_db:") {
                guidelines.push(format!(
                    "Use tool external_db with db='{}'. Start by calling operation='schema' to discover tables.",
                    db_name
                ));
            } else if let Some(api_name) = override_spec.strip_prefix("external_api:") {
                guidelines
                    .push(format!("Use tool external_api with api='{}' for all HTTP calls to this backend.", api_name));
            }
        }
        // Populate structured ExecutionGuidelines from extracted actions + tool overrides
        for item in guidelines {
            role.execution_guidelines.add_rule(crate::agent::definition::GuidelineRule::always(item));
        }
        apply_execution_hints(&mut role, &intent);

        // Apply trigger from intent (with confidence) — will be confirmed in clarifications phase
        let (parsed_trigger, confidence) = intent_to_trigger(&intent);
        role.trigger = parsed_trigger;
        role.trigger.confidence = confidence;

        let goal_fingerprint = compute_plan_mode_goal_fingerprint(&user_description, &intent, &role);
        session.goal_fingerprint = Some(goal_fingerprint.clone());
        let reused_snapshot =
            self.store.get_latest_plan_mode_session_by_goal_fingerprint(&session.tenant_id, &goal_fingerprint).await?;

        let pending_custom_tool_categories = missing_tool_categories(&intent)
            .into_iter()
            .filter(|category| !category.trim().is_empty())
            .collect::<Vec<_>>();
        let custom_tool_resolution_pending = false;

        session.draft_role = Some(role.clone());

        // Cache the extracted intent — used throughout all subsequent phases
        let mut cached_intent = intent.clone();
        if let Some(object) = cached_intent.as_object_mut() {
            if clarifying_q.is_some() {
                object.insert("_pending_connector_resolution".into(), serde_json::json!(true));
            }
            if custom_tool_resolution_pending {
                object.insert(
                    "_pending_custom_tool_categories".into(),
                    serde_json::json!(pending_custom_tool_categories),
                );
            }
        }
        if let Some(db_name) = tool_overrides.iter().find_map(|spec| spec.strip_prefix("external_db:")) {
            persist_selected_external_db(&mut cached_intent, db_name);
        }
        session.intent_cache = Some(cached_intent.clone());

        if let Some(previous) = reused_snapshot {
            if previous.id != session.id {
                tracing::info!(
                    agent_id = %session.draft_agent.id,
                    goal_fingerprint = %goal_fingerprint,
                    source_session = %previous.id,
                    version = previous.repair_version + 1,
                    "reusing prior repaired plan-mode snapshot for matching goal"
                );

                session.repair_version = previous.repair_version.saturating_add(1);
                session.reused_from_session_id = Some(previous.id.clone());
                session.repair_root_session_id =
                    previous.repair_root_session_id.clone().or_else(|| Some(previous.id.clone()));

                if let Some(mut previous_role) = previous.draft_role.clone() {
                    previous_role.id = role.id.clone();
                    previous_role.agent_id = session.draft_agent.id.clone();
                    previous_role.tenant_id = session.tenant_id.clone();
                    previous_role.status = RoleStatus::Draft;
                    previous_role.version = previous_role.version.saturating_add(1);
                    previous_role.created_at = Utc::now();
                    previous_role.updated_at = Utc::now();
                    role = previous_role;
                    session.draft_agent.connectors = role.connectors.clone();
                }

                if !previous.attachment_context.trim().is_empty() {
                    session.attachment_context = previous.attachment_context.clone();
                }
                if session.attachments.is_empty() && !previous.attachments.is_empty() {
                    session.attachments = previous.attachments.clone();
                }
                if session.pending_steps.is_empty() && !previous.pending_steps.is_empty() {
                    session.pending_steps = previous.pending_steps.clone();
                }
                if previous.intent_cache.is_some() {
                    session.intent_cache = previous.intent_cache.clone();
                    cached_intent = previous.intent_cache.clone().unwrap_or(cached_intent);
                }
                if phase_rank(&previous.phase) > phase_rank(&session.phase) {
                    session.phase = phase_for_reuse(&previous.phase);
                }
            }
        }

        session.draft_role = Some(role);

        if clarifying_q.is_some() {
            session.phase = PlanModePhase::ResolvingConnectors;
            let mut questions: Vec<String> = Vec::new();
            if let Some(q) = clarifying_q {
                questions.push(q);
            }
            return Ok(questions.join("\n\n"));
        }

        // Move to the combined clarifications phase — steps queue drives it
        let draft_role = session.draft_role.clone().unwrap();
        let (repaired_intent, compiler_question) = self
            .validate_and_repair_compiler_draft(
                session,
                &draft_role,
                cached_intent.clone(),
                &installed,
                &tenant_connectors,
            )
            .await?;
        cached_intent = repaired_intent;
        session.intent_cache = Some(cached_intent.clone());
        if let Some(question) = compiler_question {
            return Ok(question);
        }
        session.phase = PlanModePhase::CapturingClarifications;
        Ok(self.build_step_queue_and_ask(session, &cached_intent).await)
    }
    async fn validate_and_repair_compiler_draft(
        &self,
        session: &mut PlanModeSession,
        role: &AgentRole,
        intent: serde_json::Value,
        installed: &[String],
        tenant_connectors: &[TenantConnector],
    ) -> Result<(serde_json::Value, Option<String>)> {
        let mut current_intent = intent;
        let mut repair_passes = session.compiler_repair_passes;
        let mut last_issues: Vec<String> = Vec::new();

        loop {
            session.compiler_stage = PlanModeCompilerStage::Validate;
            session.compiler_validation_issues = last_issues.clone();

            match WorkflowCompiler::compile(role, &current_intent, &self.tools) {
                Ok(CompilerResult::Ready(_compiled)) => {
                    if let Some(role) = session.draft_role.as_mut() {
                        role.execution_guidelines.compiled_workflow = Some(_compiled.clone());
                    }
                    session.compiler_stage = PlanModeCompilerStage::Bind;
                    session.compiler_repair_passes = repair_passes;
                    session.compiler_validation_issues.clear();
                    return Ok((current_intent, None));
                }
                Ok(CompilerResult::NeedsCard(card)) => {
                    session.compiler_stage = PlanModeCompilerStage::Review;
                    session.compiler_repair_passes = repair_passes;
                    let question = match card.card_type.as_str() {
                        "database" => format!(
                            "The compiler needs a database connection before it can finish this workflow.\nPlease open the database card for `{}` and then reply with the saved database name.",
                            card.binding_target
                        ),
                        "api_auth" => format!(
                            "The compiler needs API auth before it can finish this workflow.\nPlease open the API card for `{}` and then reply once the connection is saved.",
                            card.binding_target
                        ),
                        "mcp" => format!(
                            "The compiler needs an MCP connection before it can finish this workflow.\nPlease open the MCP card for `{}` and then reply once the server is saved.",
                            card.binding_target
                        ),
                        _ => format!(
                            "The compiler needs additional setup before it can finish this workflow: {}",
                            card.card_type
                        ),
                    };
                    session.compiler_validation_issues = vec![question.clone()];
                    return Ok((current_intent, Some(question)));
                }
                Err(error) => {
                    let issue = error.to_string();
                    last_issues = vec![issue.clone()];
                    session.compiler_validation_issues = last_issues.clone();
                    if repair_passes >= 2 {
                        session.compiler_stage = PlanModeCompilerStage::Review;
                        session.compiler_repair_passes = repair_passes;
                        return Ok((current_intent, Some(self.compiler_followup_question(&last_issues))));
                    }

                    repair_passes = repair_passes.saturating_add(1);
                    session.compiler_stage = PlanModeCompilerStage::Repair;
                    session.compiler_repair_passes = repair_passes;

                    let detail_context = format!(
                        "{}\n\n{}",
                        crate::agent::plan_mode::repair::compact_repair_context(&last_issues, &current_intent),
                        crate::agent::plan_mode::registry::build_registry_candidate_context(
                            &self.tools,
                            &current_intent,
                            installed,
                            tenant_connectors,
                        )
                    );
                    current_intent = self
                        .extractor
                        .refine(&session.id, &session.tenant_id, &role.purpose, &current_intent, &detail_context)
                        .await?;
                }
            }
        }
    }

    fn compiler_followup_question(&self, issues: &[String]) -> String {
        let mut unique = Vec::new();
        for issue in issues {
            let trimmed = issue.trim();
            if !trimmed.is_empty() && !unique.iter().any(|existing: &String| existing.eq_ignore_ascii_case(trimmed)) {
                unique.push(trimmed.to_string());
            }
        }

        if unique.is_empty() {
            return "I still need one more compiler detail before I can finish the workflow draft.".into();
        }

        format!(
            "I still need a bit more detail before I can finish the workflow draft:\n- {}\n\nPlease clarify the missing step or setup detail, then I’ll recompile.",
            unique.join("\n- ")
        )
    }

    async fn handle_connector_clarification(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        let answer_lower = answer.to_lowercase();
        let mut pending_connector_resolution = false;
        let mut pending_custom_tool_categories: Vec<String> = Vec::new();
        let tenant_connectors = self.store.list_tenant_connectors(&session.tenant_id).await.unwrap_or_default();
        if let Some(intent) = session.intent_cache.as_ref() {
            pending_connector_resolution = intent["_pending_connector_resolution"].as_bool().unwrap_or(false);
            pending_custom_tool_categories = intent["_pending_custom_tool_categories"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|value| value.as_str().map(String::from)).collect())
                .unwrap_or_default();
        }
        let local_document_workflow =
            session.intent_cache.as_ref().map(intent_prefers_local_document_workflow).unwrap_or(false);
        let needs_db_connection = session.intent_cache.as_ref().map(intent_needs_database_connection).unwrap_or(false);
        let needs_acp_connection = session.intent_cache.as_ref().map(intent_needs_acp_connection).unwrap_or(false);

        if let Some(role) = session.draft_role.as_mut() {
            if !pending_custom_tool_categories.is_empty() {
                if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                    intent.remove("_pending_custom_tool_categories");
                }
                session.phase = PlanModePhase::ResolvingConnectors;
                pending_custom_tool_categories.clear();
                return Ok(
                    "Deterministic custom logic should use data_engine in plan mode. If you need arbitrary code later, mark it as a missing capability for the future sandbox runtime."
                        .into(),
                );
            }

            if pending_connector_resolution {
                let matched: Vec<&crate::tools::connector_tool::ConnectorDef> = BUILTIN_CONNECTORS
                    .iter()
                    .filter(|entry| contains_connector_name(&answer_lower, entry.name))
                    .collect();
                let matched_db_name = answer_mentions_tenant_database(&answer_lower, &tenant_connectors);
                let matched_api_name = answer_mentions_tenant_api(&answer_lower, &tenant_connectors);
                let matched_mcp_name = answer_mentions_tenant_mcp(&answer_lower, &tenant_connectors);
                let matched_acp_name = answer_mentions_tenant_acp(&answer_lower, &tenant_connectors);

                if !needs_db_connection
                    && !needs_acp_connection
                    && (answer_declines_external_connector(&answer_lower)
                        || (local_document_workflow && matched.is_empty()))
                {
                    if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                        intent.remove("_pending_connector_resolution");
                    }
                    pending_connector_resolution = false;
                }

                if pending_connector_resolution {
                    if matched.len() > 1 {
                        let choices = matched.iter().map(|entry| entry.name).collect::<Vec<_>>().join(", ");
                        session.phase = PlanModePhase::ResolvingConnectors;
                        return Ok(format!(
                            "I found multiple connector names in your answer: {}. Please reply with one exact connector name.",
                            choices
                        ));
                    }

                    if let Some(entry) = matched.first().copied() {
                        role.connectors.retain(|connector_name| {
                            BUILTIN_CONNECTORS
                                .iter()
                                .find(|candidate| candidate.name == connector_name.as_str())
                                .map(|candidate| candidate.category != entry.category)
                                .unwrap_or(true)
                        });
                        role.connectors.push(entry.name.to_string());
                        role.connectors.sort();
                        role.connectors.dedup();
                        session.draft_agent.connectors = role.connectors.clone();
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        pending_connector_resolution = false;
                    } else if let Some(db_name) = matched_db_name {
                        if !role.connectors.iter().any(|connector_name| connector_name == &db_name) {
                            role.connectors.push(db_name.clone());
                            role.connectors.sort();
                            role.connectors.dedup();
                            session.draft_agent.connectors = role.connectors.clone();
                        }
                        if !role.tools.iter().any(|tool| tool == &format!("external_db:{}", db_name)) {
                            role.tools.push(format!("external_db:{}", db_name));
                            role.tools.sort();
                            role.tools.dedup();
                        }
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        if let Some(intent) = session.intent_cache.as_mut() {
                            persist_selected_external_db(intent, &db_name);
                        }
                        pending_connector_resolution = false;
                    } else if let Some(api_name) = matched_api_name {
                        if !role.connectors.iter().any(|connector_name| connector_name == &api_name) {
                            role.connectors.push(api_name.clone());
                            role.connectors.sort();
                            role.connectors.dedup();
                            session.draft_agent.connectors = role.connectors.clone();
                        }
                        if !role.tools.iter().any(|tool| tool == &format!("external_api:{}", api_name)) {
                            role.tools.push(format!("external_api:{}", api_name));
                            role.tools.sort();
                            role.tools.dedup();
                        }
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        pending_connector_resolution = false;
                    } else if let Some(mcp_name) = matched_mcp_name {
                        if !role.connectors.iter().any(|connector_name| connector_name == &mcp_name) {
                            role.connectors.push(mcp_name.clone());
                            role.connectors.sort();
                            role.connectors.dedup();
                            session.draft_agent.connectors = role.connectors.clone();
                        }
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        pending_connector_resolution = false;
                    } else if let Some(acp_name) = matched_acp_name {
                        if !role.connectors.iter().any(|connector_name| connector_name == &acp_name) {
                            role.connectors.push(acp_name.clone());
                            role.connectors.sort();
                            role.connectors.dedup();
                            session.draft_agent.connectors = role.connectors.clone();
                        }
                        if !role.tools.iter().any(|tool| tool == &format!("acp_session:{}", acp_name)) {
                            role.tools.push(format!("acp_session:{}", acp_name));
                            role.tools.sort();
                            role.tools.dedup();
                        }
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        if let Some(intent) = session.intent_cache.as_mut() {
                            persist_selected_acp_peer(intent, &acp_name);
                        }
                        pending_connector_resolution = false;
                    } else if needs_db_connection {
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        session.phase = PlanModePhase::ResolvingConnectors;
                        return Ok(
                            "Please add the database using the inline connection card, then reply with the saved database name so I can continue.".into(),
                        );
                    } else if needs_acp_connection {
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        session.phase = PlanModePhase::ResolvingConnectors;
                        return Ok(
                            "Please add the ACP peer using the inline connection card, then reply with the saved peer name so I can continue.".into(),
                        );
                    } else if local_document_workflow {
                        if let Some(intent) = session.intent_cache.as_mut().and_then(|value| value.as_object_mut()) {
                            intent.remove("_pending_connector_resolution");
                        }
                        pending_connector_resolution = false;
                    } else {
                        session.phase = PlanModePhase::ResolvingConnectors;
                        return Ok(
                            "Please reply with the exact connector name to use (for example: salesforce, hubspot, zendesk)."
                                .into()
                        );
                    }
                }
            }
        }

        if !pending_custom_tool_categories.is_empty() || pending_connector_resolution {
            session.phase = PlanModePhase::ResolvingConnectors;
            if needs_db_connection {
                return Ok("Please add the database using the inline connection card, then reply with the saved database name so I can continue.".into());
            }
            return Ok("Please confirm the pending connector/custom-tool setup first.".into());
        }

        // Regenerate the step queue now that the connector is confirmed
        let intent = session.intent_cache.clone().unwrap_or_else(|| serde_json::json!({ "trigger_hint": "manual" }));
        session.phase = PlanModePhase::CapturingClarifications;
        Ok(self.build_step_queue_and_ask(session, &intent).await)
    }

    async fn handle_clarifications(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        use super::steps::{parse_and_apply, ClarificationStep};

        // Pop the front step — that's the one we're answering now
        let current_step: Option<ClarificationStep> = if !session.pending_steps.is_empty() {
            let raw = session.pending_steps.remove(0);
            serde_json::from_value(raw).ok()
        } else {
            None
        };

        if let Some(step) = current_step {
            // Parse and apply the answer for this step
            let mut agent_constraints = session.draft_agent.constraints.clone();
            let mut pending_roles: Option<Vec<serde_json::Value>> = None;

            let summary = if let Some(role) = session.draft_role.as_mut() {
                parse_and_apply(
                    &step,
                    answer,
                    role,
                    &mut agent_constraints,
                    session.intent_cache.as_ref().unwrap_or(&serde_json::json!({})),
                    &mut pending_roles,
                )
            } else {
                "Step processed.".into()
            };

            session.draft_agent.constraints = agent_constraints;

            // If user chose to split roles, stash pending responsibilities
            if let Some(remaining) = pending_roles {
                if !session.draft_agent.memory_ref.contains("|pending_roles:") {
                    let meta = session.draft_agent.memory_ref.clone();
                    session.draft_agent.memory_ref =
                        format!("{}|pending_roles:{}", meta, serde_json::to_string(&remaining).unwrap_or_default());
                }
            }

            // Advance to next step or move to review
            if let Some(next_raw) = session.pending_steps.first() {
                if let Ok(next_step) = serde_json::from_value::<ClarificationStep>(next_raw.clone()) {
                    // Show confirmation + next question
                    return Ok(format!("✓ {}\n\n{}", summary, next_step.question));
                }
            }

            // No more steps — inject domain skill execution brief then go to constraints
            if let Some(question) = self.refine_plan_after_clarifications(session).await? {
                session.phase = PlanModePhase::ResolvingConnectors;
                return Ok(format!("OK {}\n\n{}", summary, question));
            }

            let category = session.intent_cache.as_ref().and_then(|i| i["category"].as_str()).unwrap_or("general");

            if let Some(skill_text) = self.domain_skill_text(category).await {
                let brief: String =
                    skill_text.lines().skip_while(|l| !l.starts_with("EXECUTION BRIEF")).collect::<Vec<_>>().join("\n");
                if !brief.is_empty() {
                    if let Some(role) = session.draft_role.as_mut() {
                        let parsed = crate::agent::definition::ExecutionGuidelines::from_skill_text(&brief);
                        role.execution_guidelines.extend_dedup(parsed);
                    }
                }
                // Also auto-generate default completion criteria if none yet
                if let Some(role) = session.draft_role.as_mut() {
                    if role.execution_guidelines.completion_criteria.is_empty() {
                        let defaults = super::steps::default_completion_criteria(role);
                        for c in defaults {
                            role.execution_guidelines.add_completion(c);
                        }
                    }
                }
            }

            let _ = self.ensure_research_memo(session).await?;
            session.phase = PlanModePhase::Reviewing;
            return Ok(format!("✓ {}\n\n{}", summary, self.build_review_summary(session).await));
        }

        // pending_steps was already empty — go straight to review
        let _ = self.ensure_research_memo(session).await?;
        session.phase = PlanModePhase::Reviewing;
        Ok(self.build_review_summary(session).await)
    }
    async fn handle_constraints(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        let lower = answer.to_lowercase();
        let is_empty = lower.contains("no constraint")
            || lower.contains("none")
            || lower.contains("n/a")
            || lower.contains("defaults")
            || answer.trim().len() < 4;

        if !is_empty {
            // Parse domain skill answers + user constraints into structured guidelines
            let from_user = crate::agent::definition::ExecutionGuidelines::from_user_constraints(answer);
            if let Some(role) = session.draft_role.as_mut() {
                role.execution_guidelines.extend_dedup(from_user);
            }

            // Also parse plain constraint strings into agent.constraints
            // (for hard rules that should be visible in the review card)
            let constraint_items: Vec<String> = answer
                .split(&[',', ';', '\n'][..])
                .map(|s| s.trim().trim_end_matches('.').to_string())
                .filter(|s| s.len() > 8)
                .filter(|s| {
                    let l = s.to_lowercase();
                    !l.starts_with("mandatory") && !l.starts_with("before confirm") && !l.starts_with("execution brief")
                })
                .collect();
            session.draft_agent.constraints.extend(constraint_items);
        }

        let _ = self.ensure_research_memo(session).await?;
        session.phase = PlanModePhase::Reviewing;
        Ok(self.build_review_summary(session).await)
    }

    /// Public wrapper for build_review_summary — used by the template fast-path in routes.rs
    pub async fn build_review_summary_pub(&self, session: &mut PlanModeSession) -> String {
        let _ = self.ensure_research_memo(session).await;
        self.build_review_summary(session).await
    }

    async fn sync_review_scaffold_tasks(&self, session: &PlanModeSession) -> Vec<SessionTask> {
        let specs = crate::agent::plan_mode::review::plan_mode_scaffold_specs(session);
        let mut tasks = Vec::new();

        for (task_id, subject, description, status, metadata, output) in specs {
            let mut task =
                self.store.get_session_task(&session.tenant_id, &task_id).await.ok().flatten().unwrap_or_else(|| {
                    SessionTask::new(
                        task_id.clone(),
                        session.tenant_id.clone(),
                        session.draft_agent.id.clone(),
                        subject.clone(),
                        description.clone(),
                    )
                });

            task.subject = subject;
            task.description = description;
            task.metadata = metadata;
            task.set_status(status);
            task.output = output;
            let _ = self.store.upsert_session_task(&task).await;
            tasks.push(task);
        }

        tasks
    }

    async fn build_review_summary(&self, session: &PlanModeSession) -> String {
        let agent = &session.draft_agent;
        let preview_role = match session.draft_role.as_ref() {
            Some(r) => r.clone(),
            None => return "Configuration incomplete — no role defined.".into(),
        };
        let role = &preview_role;
        let scaffold_tasks = self.sync_review_scaffold_tasks(session).await;
        let research_memo = session
            .intent_cache
            .as_ref()
            .and_then(|intent| intent.get("_adaptive_research_memo"))
            .and_then(|value| serde_json::from_value::<AdaptiveResearchMemo>(value.clone()).ok());

        let trigger_desc = match &role.trigger.trigger_type {
            TriggerType::Webhook => format!(
                "triggered by {} {}",
                role.trigger.source_connector.as_deref().unwrap_or("external event"),
                role.trigger.event_filter.as_deref().unwrap_or("")
            ),
            TriggerType::Schedule => format!("runs on schedule: {}", role.trigger.cron.as_deref().unwrap_or("daily")),
            TriggerType::UserMessage => "runs when you ask it to".into(),
            TriggerType::Manual => "runs on-demand".into(),
            TriggerType::WorkforceEvent => {
                match &role.trigger.workforce_event_filter {
                    Some(f) if f.contains("role_name") => {
                        // Extract the role name from the filter expression
                        let name = f
                            .split("role_name == '")
                            .nth(1)
                            .and_then(|s| s.split('\'').next())
                            .unwrap_or("another role");
                        format!("runs after '{}' completes", name)
                    }
                    Some(f) => format!("runs on workforce event: {}", f),
                    None => "runs after another role completes".into(),
                }
            }
        };

        let connectors = if role.connectors.is_empty() {
            "none (uses built-in tools only)".into()
        } else {
            role.connectors.join(", ")
        };

        // Show external databases and APIs from tool overrides
        let tools_section = if role.tools.is_empty() {
            String::new()
        } else {
            let mut parts: Vec<String> = Vec::new();
            for t in &role.tools {
                if let Some(db_name) = t.strip_prefix("external_db:") {
                    parts.push(format!("database '{}'", db_name));
                } else if let Some(api_name) = t.strip_prefix("external_api:") {
                    parts.push(format!("REST API '{}'", api_name));
                } else if let Some(peer_name) = t.strip_prefix("acp_session:") {
                    parts.push(format!("ACP peer '{}'", peer_name));
                } else {
                    parts.push(t.clone());
                }
            }
            format!("\n**Your connections:** {}", parts.join(", "))
        };

        let constraints = if agent.constraints.is_empty() { "none".into() } else { agent.constraints.join("; ") };

        let attachments = if session.attachments.is_empty() {
            "none".into()
        } else {
            session
                .attachments
                .iter()
                .map(|attachment| {
                    format!(
                        "{} ({}, {} bytes)",
                        attachment.name,
                        attachment_kind_label(&attachment.kind),
                        attachment.size_bytes
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        // Show which compliance services will be active for this category
        let services_line = {
            let category = session.intent_cache.as_ref().and_then(|i| i["category"].as_str()).unwrap_or("general");
            let services = active_services_for_category(category);
            if services.is_empty() {
                String::new()
            } else {
                format!("\n**Active services:** {}", services.join(", "))
            }
        };

        let subsystem_line = format!(
            "\n**Agent subsystems:** {}",
            crate::agent::plan_mode::subsystems::subsystem_names().join(", ")
        );

        let boundary_line =
            if role.tools.iter().any(|tool| tool.starts_with("acp_session:")) || role.connectors.iter().any(|connector| {
                connector.contains("acp") || connector.contains("agent")
            }) {
                format!("\n**Boundary:** {}", crate::agent::plan_mode::boundary::planning_hint())
            } else {
                String::new()
            };

        let review_focus = if self.superpowers_guidance_text(&PlanModePhase::Reviewing).await.is_some() {
            format!(
                "\n**Review checklist:** validate the compiler draft, run the sandbox test, then save only if the result is clear. {}",
                crate::agent::plan_mode::subsystems::subsystem_setup_prompt()
            )
        } else {
            String::new()
        };

        let save_guardrail = if session
            .intent_cache
            .as_ref()
            .map_or(true, |intent| crate::agent::plan_mode::review::workflow_hints_for_compilation(intent).is_empty())
        {
            "\n\n⚠️ **Save guardrail:** this role still does not have a runnable compiler draft. Please add or clarify the missing workflow steps before saving.".to_string()
        } else {
            String::new()
        };

        let runtime_policy = format!(
            "\n**Runtime policy:** execution={} | tool pool={} | permission mode={}",
            match role.execution_guidelines.execution_strategy {
                ExecutionStrategy::DeterministicWorkflow => "deterministic_workflow",
                ExecutionStrategy::AdaptivePlanning => "adaptive_planning -> compile into compiled_workflow",
                ExecutionStrategy::CoordinatorShell => "coordinator_shell -> research / synthesize / verify",
            },
            match role.execution_guidelines.tool_pool {
                ToolPool::Worker => "worker",
                ToolPool::Plan => "plan",
                ToolPool::Coordinator => "coordinator",
                ToolPool::Verification => "verification",
                ToolPool::Teammate => "teammate",
            },
            role.execution_guidelines.permission_mode.as_str(),
        );

        let scaffold = if scaffold_tasks.is_empty() {
            String::new()
        } else {
            let lines = scaffold_tasks
                .iter()
                .map(|task| {
                    format!(
                        "- [{}] {}",
                        match task.status {
                            SessionTaskStatus::Completed => "done",
                            SessionTaskStatus::InProgress => "active",
                            SessionTaskStatus::Blocked => "blocked",
                            SessionTaskStatus::Failed => "failed",
                            SessionTaskStatus::Stopped => "stopped",
                            SessionTaskStatus::Pending => "pending",
                        },
                        task.subject
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n**Planning scaffold:**\n{}", lines)
        };

        let tooling_notes = {
            let notes = shared_plan_mode_tooling_notes(role);
            if notes.is_empty() {
                String::new()
            } else {
                format!("\n**Shared planning/runtime tools:** {}", notes.join(" | "))
            }
        };

        let research_summary = research_memo.as_ref().map_or_else(String::new, |memo| {
            let mut lines = Vec::new();
            if !memo.summary.trim().is_empty() {
                lines.push(format!("summary: {}", memo.summary));
            }
            if !memo.findings.is_empty() {
                lines.push(format!("findings: {}", memo.findings.join(" | ")));
            }
            if !memo.risks.is_empty() {
                lines.push(format!("risks: {}", memo.risks.join(" | ")));
            }
            if lines.is_empty() {
                String::new()
            } else {
                format!("\n**Research synthesis:** {}", lines.join("\n"))
            }
        });

        let compiler_state = {
            let issues = if session.compiler_validation_issues.is_empty() {
                "none".to_string()
            } else {
                session.compiler_validation_issues.join(" | ")
            };
            let stage = match session.compiler_stage {
                PlanModeCompilerStage::Intent => "intent",
                PlanModeCompilerStage::Dsl => "dsl",
                PlanModeCompilerStage::Validate => "validate",
                PlanModeCompilerStage::Repair => "repair",
                PlanModeCompilerStage::Bind => "bind",
                PlanModeCompilerStage::Review => "review",
            };
            format!(
                "\n**Compiler stage:** {} | repair passes: {} | validation issues: {}",
                stage, session.compiler_repair_passes, issues
            )
        };

        format!(
            "Here's what I've configured:\n\n\
            **Agent:** {name}\n\
            **Role:** {purpose}\n\
            **Trigger:** {trigger}\n\
            **Connectors:** {connectors}{tools}\n\
            **Output:** {output}\n\
            **Uploaded docs:** {attachments}\n\
            **Constraints:** {constraints}{services}{subsystems}{boundary}{compiler_state}{runtime_policy}{tooling_notes}{research_summary}{scaffold}{review_focus}{save_guardrail}\n\n\
            Does this look right? Say **yes** to save, or tell me what to change.",
            name = agent.name,
            purpose = role.purpose,
            trigger = trigger_desc,
            connectors = connectors,
            tools = tools_section,
            output = role.output_spec.description,
            attachments = attachments,
            constraints = constraints,
            services = services_line,
            subsystems = subsystem_line,
            boundary = boundary_line,
            compiler_state = compiler_state,
            runtime_policy = runtime_policy,
            tooling_notes = tooling_notes,
            research_summary = research_summary,
            scaffold = scaffold,
            review_focus = review_focus,
        )
    }

    async fn handle_review(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        if is_explicit_review_confirmation(answer) {
            session.phase = PlanModePhase::Complete;
            return Ok("✓ Agent saved. You can find it in your agent list. \
                       Add more roles anytime from the agent settings page."
                .into());
        }

        // User wants to change something — re-extract from their correction
        session.phase = PlanModePhase::CapturingIntent;
        let reply = self.handle_intent(session, answer).await?;
        Ok(format!("Updated. Let me reconfigure based on your correction.\n\n{}", reply))
    }

    /// Finalise and save the session — creates AgentDefinition + AgentRole in DB.
    pub async fn save(&self, mut session: PlanModeSession) -> Result<(AgentDefinition, AgentRole)> {
        session.phase = PlanModePhase::Complete;
        let _ = self.ensure_research_memo(&mut session).await?;
        let mut agent = session.draft_agent.clone();
        agent.status = AgentDefinitionStatus::Active;
        agent.updated_at = Utc::now();

        self.store.upsert_agent_definition(&agent).await?;

        let role = match session.draft_role.take() {
            Some(mut r) => {
                r.status = RoleStatus::Active;
                r.updated_at = Utc::now();

                // Compile the draft into the immutable workflow artifact.
                let intent = session.intent_cache.as_ref().cloned().unwrap_or_else(|| serde_json::json!({}));
                match WorkflowCompiler::compile(&r, &intent, &self.tools) {
                    Ok(CompilerResult::Ready(mut compiled)) => {
                        if let Some(metadata) = compiled.metadata.as_object_mut() {
                            metadata.insert("plan_mode_session_id".into(), serde_json::json!(session.id.clone()));
                            metadata.insert(
                                "plan_mode_goal_fingerprint".into(),
                                serde_json::json!(session.goal_fingerprint.clone()),
                            );
                            metadata
                                .insert("plan_mode_repair_version".into(), serde_json::json!(session.repair_version));
                            metadata.insert(
                                "plan_mode_repair_root_session_id".into(),
                                serde_json::json!(session.repair_root_session_id.clone()),
                            );
                            metadata.insert(
                                "plan_mode_reused_from_session_id".into(),
                                serde_json::json!(session.reused_from_session_id.clone()),
                            );
                            metadata.insert(
                                "compiler_stage".into(),
                                serde_json::json!(format!("{:?}", session.compiler_stage).to_lowercase()),
                            );
                            metadata.insert(
                                "compiler_repair_passes".into(),
                                serde_json::json!(session.compiler_repair_passes),
                            );
                            metadata.insert(
                                "compiler_validation_issues".into(),
                                serde_json::json!(session.compiler_validation_issues.clone()),
                            );
                        }
                        r.execution_guidelines.compiled_workflow = Some(compiled.clone());
                    }
                    Ok(CompilerResult::NeedsCard(card)) => {
                        anyhow::bail!(
                            "workflow compilation requires setup card: {} (target={}, resume={})",
                            card.card_type,
                            card.binding_target,
                            card.resume_token
                        );
                    }
                    Err(error) => {
                        anyhow::bail!("workflow compilation failed: {}", error);
                    }
                }
                if r.execution_guidelines.compiled_workflow.is_none() {
                    anyhow::bail!("workflow compiler did not produce a runnable artifact before save");
                }
                crate::agent::plan_mode::review::finalize_saved_role_execution_strategy(&mut r);

                // Resolve "name:Role Name" hints in depends_on_role_id to actual IDs
                if let Some(hint) = r.trigger.depends_on_role_id.clone() {
                    if let Some(name) = hint.strip_prefix("name:") {
                        let existing =
                            self.store.list_roles_for_agent(&agent.tenant_id, &agent.id).await.unwrap_or_default();
                        if let Some(found) = existing.iter().find(|er| er.name.to_lowercase() == name.to_lowercase()) {
                            r.trigger.depends_on_role_id = Some(found.id.clone());
                        } else {
                            // Named role not found — clear the hint rather than save a bad ref
                            r.trigger.depends_on_role_id = None;
                            tracing::warn!(
                                role_name = %name,
                                "depends_on_role_id: named role not found — cleared"
                            );
                        }
                    }
                }

                self.store.upsert_agent_role(&r).await?;

                // Sync workforce event subscription if needed
                crate::events::workforce::sync_subscriptions_for_role(&r, &self.store).await?;
                r
            }
            None => {
                anyhow::bail!("cannot save plan mode session with no role defined")
            }
        };

        let _ = self.sync_review_scaffold_tasks(&session).await;

        Ok((agent, role))
    }

    async fn preflight_workflow(
        &self,
        plan: &crate::agent::planner::Plan,
        role: &AgentRole,
    ) -> PlanModePreflightResult {
        let mut checks = Vec::new();
        if plan.steps.is_empty() {
            return PlanModePreflightResult {
                status: PlanModeTestStatus::Fail,
                checks: vec![PlanModeTestCheck {
                    label: "compiled workflow".into(),
                    success: false,
                    detail: Some("compiled workflow artifact is empty".into()),
                }],
                summary: "No compiled workflow artifact was drafted, so there is nothing to preflight.".into(),
            };
        }

        let mut has_failure = false;
        let mut has_partial = false;

        for step in &plan.steps {
            let label = format!("step {}: {}", step.index + 1, step.description);
            match step.tool.as_deref() {
                None => {
                    checks.push(PlanModeTestCheck {
                        label,
                        success: true,
                        detail: Some("llm_worker; conceptual step only".into()),
                    });
                }
                Some(tool_name) => {
                    let Some(tool) = self.tools.get(tool_name) else {
                        has_failure = true;
                        checks.push(PlanModeTestCheck {
                            label,
                            success: false,
                            detail: Some(format!("tool '{}' not found", tool_name)),
                        });
                        continue;
                    };

                    let mut args = step.tool_args.clone().unwrap_or_else(|| serde_json::json!({}));
                    materialize_validation_tool_args(
                        tool_name,
                        &step.description,
                        &mut args,
                        &self.workspace_root,
                        role,
                    );
                    if value_contains_placeholder(&args) {
                        has_failure = true;
                        checks.push(PlanModeTestCheck {
                            label,
                            success: false,
                            detail: Some("args still contain unresolved placeholders".into()),
                        });
                        continue;
                    }

                    let schema = tool.parameters_schema();
                    let missing = missing_required_args_for_schema(&args, &schema);
                    if !missing.is_empty() {
                        has_failure = true;
                        checks.push(PlanModeTestCheck {
                            label,
                            success: false,
                            detail: Some(format!("missing required args: {}", missing.join(", "))),
                        });
                        continue;
                    }

                    match sandbox_tool_policy(tool_name, &args) {
                        SandboxPolicy::Block(reason) => {
                            has_partial = true;
                            checks.push(PlanModeTestCheck { label, success: true, detail: Some(reason) });
                        }
                        SandboxPolicy::NoOp(reason) => {
                            has_partial = true;
                            checks.push(PlanModeTestCheck { label, success: true, detail: Some(reason) });
                        }
                        SandboxPolicy::Allow => {
                            checks.push(PlanModeTestCheck {
                                label,
                                success: true,
                                detail: Some("preflight checks passed".into()),
                            });
                        }
                    }
                }
            }
        }

        let status = if has_failure {
            PlanModeTestStatus::Fail
        } else if has_partial {
            PlanModeTestStatus::Partial
        } else {
            PlanModeTestStatus::Pass
        };

        let summary = match status {
            PlanModeTestStatus::Pass => format!("Preflight passed for {} step(s).", checks.len()),
            PlanModeTestStatus::Partial => format!("Preflight completed with {} warning(s).", checks.len()),
            PlanModeTestStatus::Fail => "Preflight failed; fix the draft before relying on the sandbox.".into(),
        };

        let _ = role; // role-specific checks can expand here without changing the API.

        PlanModePreflightResult { status, checks, summary }
    }

    async fn run_sandbox(
        &self,
        plan: &crate::agent::planner::Plan,
        role: &AgentRole,
        workspace_root: &Path,
    ) -> PlanModeSandboxResult {
        let mut steps = Vec::new();
        let mut has_failure = false;
        let mut has_partial = false;

        for step in &plan.steps {
            let result = self.run_sandbox_step(step, role, workspace_root).await;
            if result.blocked {
                has_partial = true;
            }
            if !result.success && !result.blocked {
                has_failure = true;
            }
            steps.push(result);
        }

        let status = if has_failure {
            PlanModeTestStatus::Fail
        } else if has_partial {
            PlanModeTestStatus::Partial
        } else {
            PlanModeTestStatus::Pass
        };

        let summary = match status {
            PlanModeTestStatus::Pass => format!("Sandbox executed {} step(s) successfully.", steps.len()),
            PlanModeTestStatus::Partial => format!("Sandbox executed {} step(s) with safety no-ops.", steps.len()),
            PlanModeTestStatus::Fail => "Sandbox encountered at least one hard failure.".into(),
        };

        PlanModeSandboxResult { status, steps, summary }
    }

    async fn run_sandbox_step(
        &self,
        step: &crate::agent::planner::PlannedStep,
        role: &AgentRole,
        workspace_root: &Path,
    ) -> PlanModeTestStepResult {
        let label = step.description.clone();
        let Some(tool_name) = step.tool.as_deref() else {
            return PlanModeTestStepResult {
                step: step.index,
                description: label,
                tool: Some(conceptual_step_tool_name().into()),
                success: true,
                output: serde_json::json!({
                    "mode": "llm_worker",
                    "reason": "conceptual step handled by the model",
                }),
                error: None,
                blocked: false,
            };
        };

        if tool_name == conceptual_step_tool_name() {
            return PlanModeTestStepResult {
                step: step.index,
                description: label,
                tool: Some(tool_name.to_string()),
                success: true,
                output: serde_json::json!({
                    "mode": "llm_worker",
                    "reason": "conceptual LLM worker step handled by the compiler/runtime contract",
                }),
                error: None,
                blocked: false,
            };
        }

        let mut args = step.tool_args.clone().unwrap_or_else(|| serde_json::json!({}));
        materialize_validation_tool_args(tool_name, &step.description, &mut args, workspace_root, role);
        match sandbox_tool_policy(tool_name, &args) {
            SandboxPolicy::NoOp(reason) => {
                return PlanModeTestStepResult {
                    step: step.index,
                    description: label,
                    tool: Some(tool_name.to_string()),
                    success: true,
                    output: serde_json::json!({
                        "mode": "log_only",
                        "reason": reason,
                    }),
                    error: None,
                    blocked: true,
                };
            }
            SandboxPolicy::Block(reason) => {
                return PlanModeTestStepResult {
                    step: step.index,
                    description: label,
                    tool: Some(tool_name.to_string()),
                    success: true,
                    output: serde_json::json!({
                        "mode": "blocked",
                        "reason": reason,
                    }),
                    error: None,
                    blocked: true,
                };
            }
            SandboxPolicy::Allow => {}
        }

        let Some(tool) = self.tools.get(tool_name) else {
            return PlanModeTestStepResult {
                step: step.index,
                description: label,
                tool: Some(tool_name.to_string()),
                success: false,
                output: serde_json::Value::Null,
                error: Some(format!("tool '{}' is not available", tool_name)),
                blocked: false,
            };
        };

        let mut resolved_args = step.tool_args.clone().unwrap_or_else(|| serde_json::json!({}));
        normalize_sandbox_tool_args(tool_name, &mut resolved_args, workspace_root, role);

        if value_contains_placeholder(&resolved_args) {
            return PlanModeTestStepResult {
                step: step.index,
                description: label,
                tool: Some(tool_name.to_string()),
                success: false,
                output: serde_json::Value::Null,
                error: Some("unresolved placeholders remain after sandbox rendering".into()),
                blocked: false,
            };
        }

        match tool.execute(resolved_args).await {
            Ok(result) => PlanModeTestStepResult {
                step: step.index,
                description: label,
                tool: Some(tool_name.to_string()),
                success: result.success,
                output: result.output,
                error: result.error,
                blocked: false,
            },
            Err(error) => PlanModeTestStepResult {
                step: step.index,
                description: label,
                tool: Some(tool_name.to_string()),
                success: false,
                output: serde_json::Value::Null,
                error: Some(error.to_string()),
                blocked: false,
            },
        }
    }
}

// ── Free helper functions ───────────────────────────────────────────────────

fn conceptual_step_tool_name() -> &'static str {
    "llm_worker"
}

fn apply_role_policy_defaults(agent: &mut AgentDefinition, role: &mut AgentRole) {
    if agent.persona.trim().is_empty() {
        agent.persona = role.role_category.default_persona().to_string();
    }

    role.memory_scope = role.role_category.default_memory_scope();

    if role.execution_limits == crate::agent::definition::ExecutionLimits::default() {
        role.execution_limits = role.role_category.default_execution_limits();
    }

    role.execution_guidelines.permission_mode = role.role_category.default_permission_mode();
    if matches!(role.role_category, RoleCategory::SoftwareEngineer) {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::AdaptivePlanning;
        role.execution_guidelines.tool_pool = ToolPool::Worker;
    } else if matches!(role.role_category, RoleCategory::ResearchAnalyst) {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::CoordinatorShell;
        role.execution_guidelines.tool_pool = ToolPool::Coordinator;
    } else if role.execution_guidelines.execution_strategy == ExecutionStrategy::AdaptivePlanning {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::DeterministicWorkflow;
    }

    if matches!(role.execution_guidelines.execution_strategy, ExecutionStrategy::CoordinatorShell)
        && matches!(role.execution_guidelines.tool_pool, ToolPool::Worker)
    {
        role.execution_guidelines.tool_pool = ToolPool::Coordinator;
    }
}

fn plan_mode_scaffold_specs(
    session: &PlanModeSession,
) -> Vec<(String, String, String, SessionTaskStatus, serde_json::Value, Option<SessionTaskOutput>)> {
    let phase = phase_rank(&session.phase);
    let mut specs = Vec::new();

    let intent_output = session.intent_cache.as_ref().map(|intent| SessionTaskOutput {
        status: SessionTaskResultStatus::Complete,
        artifacts: Vec::new(),
        findings: workflow_hints_for_compilation(intent).into_iter().take(4).collect(),
        confidence: 1.0,
        note: Some("intent, workflow shape, and operating category captured".into()),
    });
    specs.push((
        format!("planmode:{}:intent", session.id),
        "Capture intent and workflow shape".into(),
        "Lock down the business goal, compiler draft, trigger guess, and output direction before execution design."
            .into(),
        if session.intent_cache.is_some() { SessionTaskStatus::Completed } else { SessionTaskStatus::InProgress },
        serde_json::json!({
            "phase": "capturing_intent",
            "recommended_tools": ["ask_user:clarification", "task_create", "task_update"],
        }),
        intent_output,
    ));

    let resources_complete = phase > phase_rank(&PlanModePhase::ResolvingConnectors);
    specs.push((
        format!("planmode:{}:resources", session.id),
        "Resolve systems, resources, and access".into(),
        "Confirm connectors, databases, MCP servers, and any deferred capabilities before the workflow is finalized.".into(),
        if resources_complete {
            SessionTaskStatus::Completed
        } else if phase >= phase_rank(&PlanModePhase::ResolvingConnectors) {
            SessionTaskStatus::InProgress
        } else {
            SessionTaskStatus::Pending
        },
        serde_json::json!({
            "phase": "resolving_connectors",
            "recommended_tools": ["tool_search", "mcp_session:list_resources", "mcp_session:read_resource", "request_more_tools"],
        }),
        resources_complete.then(|| SessionTaskOutput {
            status: SessionTaskResultStatus::Complete,
            artifacts: Vec::new(),
            findings: session
                .draft_role
                .as_ref()
                .map(|role| role.connectors.clone())
                .unwrap_or_default(),
            confidence: 1.0,
            note: Some("connector and capability requirements resolved".into()),
        }),
    ));

    let research_memo = session
        .intent_cache
        .as_ref()
        .and_then(|intent| intent.get("_adaptive_research_memo"))
        .and_then(|value| serde_json::from_value::<AdaptiveResearchMemo>(value.clone()).ok());
    let research_complete = research_memo.is_some();
    specs.push((
        format!("planmode:{}:research", session.id),
        "Research and compile execution contract".into(),
        "Synthesize findings, assumptions, and risks into compile-ready workflow hints before deterministic execution is saved.".into(),
        if research_complete {
            SessionTaskStatus::Completed
        } else if session.pending_steps.is_empty() && session.intent_cache.is_some() {
            SessionTaskStatus::InProgress
        } else {
            SessionTaskStatus::Pending
        },
        serde_json::json!({
            "phase": "research_compile",
            "recommended_tools": ["task_update", "tool_search", "ask_user:decision"],
        }),
        research_memo.map(|memo| SessionTaskOutput {
            status: SessionTaskResultStatus::Complete,
            artifacts: Vec::new(),
            findings: memo.workflow_hints.into_iter().take(5).collect(),
            confidence: 1.0,
            note: Some(memo.summary),
        }),
    ));

    let review_status = if phase >= phase_rank(&PlanModePhase::Reviewing) {
        SessionTaskStatus::InProgress
    } else if session.pending_steps.is_empty() && session.intent_cache.is_some() {
        SessionTaskStatus::InProgress
    } else {
        SessionTaskStatus::Pending
    };
    specs.push((
        format!("planmode:{}:review", session.id),
        "Review, preflight, and sandbox the draft".into(),
        format!(
            "Use the checklist to validate workflow steps, required arguments, sandbox behavior, and agent subsystems ({}) before approval.",
            crate::agent::plan_mode::subsystems::AGENT_SUBSYSTEMS.join(", ")
        ),
        review_status,
        serde_json::json!({
            "phase": "reviewing",
            "recommended_tools": ["task_list", "task_output", "ask_user:decision", "tool_search"],
        }),
        None,
    ));

    let save_status = if session.phase == PlanModePhase::Complete {
        SessionTaskStatus::Completed
    } else {
        SessionTaskStatus::Pending
    };
    specs.push((
        format!("planmode:{}:save", session.id),
        "Save or revise the final draft".into(),
        "Approval stays separate from clarifications: revise if needed, otherwise save the draft as the execution contract.".into(),
        save_status,
        serde_json::json!({
            "phase": "save",
            "recommended_tools": ["ask_user:approval", "task_output"],
        }),
        (session.phase == PlanModePhase::Complete).then(|| SessionTaskOutput {
            status: SessionTaskResultStatus::Complete,
            artifacts: Vec::new(),
            findings: vec!["plan approved and ready to save".into()],
            confidence: 1.0,
            note: Some("plan mode reached completion".into()),
        }),
    ));

    specs
}

fn shared_plan_mode_tooling_notes(role: &AgentRole) -> Vec<String> {
    let mut notes = vec![
        "task_* keeps planning and execution state durable".into(),
        "tool_search lazy-loads optional schemas instead of expanding whole categories".into(),
        "ask_user stays structured: clarification vs decision vs approval".into(),
    ];

    if role.tools.iter().any(|tool| tool == "mcp_session")
        || role.connectors.iter().any(|connector| connector.contains("mcp"))
    {
        notes.push("use mcp_session list_resources/read_resource to ground MCP-backed workflows".into());
    }
    if matches!(role.execution_guidelines.permission_mode, PermissionMode::WorkspaceWrite | PermissionMode::TrustedAuto)
    {
        notes.push("runtime writes stay scoped by permission mode and workspace boundary checks".into());
    }
    if matches!(role.execution_guidelines.execution_strategy, ExecutionStrategy::AdaptivePlanning) {
        notes.push("adaptive planning must compile back into compiled_workflow before deterministic execution".into());
    } else if matches!(role.execution_guidelines.execution_strategy, ExecutionStrategy::CoordinatorShell) {
        notes.push("coordinator-shell runs task-first research -> synthesis -> implementation -> verification".into());
    }

    notes
}

fn plan_mode_workspace_root(workspace_root: &Path, tenant_id: &str, agent_id: &str) -> PathBuf {
    workspace_root.join(tenant_id).join("agents").join(agent_id)
}

#[derive(Debug, Clone)]
enum SandboxPolicy {
    Allow,
    NoOp(String),
    Block(String),
}

fn superpowers_skill_names_for_phase(phase: &PlanModePhase) -> &'static [&'static str] {
    match phase {
        PlanModePhase::CapturingIntent => &["brainstorming"],
        PlanModePhase::CapturingClarifications => &["writing-plans"],
        PlanModePhase::Reviewing => &["verification-before-completion", "receiving-code-review"],
        _ => &[],
    }
}

fn combine_test_status(preflight: &PlanModeTestStatus, sandbox: &PlanModeTestStatus) -> PlanModeTestStatus {
    if matches!(preflight, PlanModeTestStatus::Fail) || matches!(sandbox, PlanModeTestStatus::Fail) {
        PlanModeTestStatus::Fail
    } else if matches!(preflight, PlanModeTestStatus::Partial) || matches!(sandbox, PlanModeTestStatus::Partial) {
        PlanModeTestStatus::Partial
    } else {
        PlanModeTestStatus::Pass
    }
}

fn materialize_validation_tool_args(
    tool_name: &str,
    step_description: &str,
    args: &mut serde_json::Value,
    workspace_root: &Path,
    role: &AgentRole,
) {
    let Some(object) = args.as_object_mut() else {
        return;
    };

    let input_file = workspace_root.join("artifacts").join("sandbox_input.txt").display().to_string();
    let output_file = workspace_root.join("artifacts").join("sandbox_output.txt").display().to_string();
    let workspace_dir = workspace_root.display().to_string();
    let workspace_files = workspace_root.join("files").display().to_string();

    let set_if_missing =
        |object: &mut serde_json::Map<String, serde_json::Value>, key: &str, value: serde_json::Value| {
            let missing = match object.get(key) {
                None | Some(serde_json::Value::Null) => true,
                Some(serde_json::Value::String(text)) => text.trim().is_empty(),
                Some(serde_json::Value::Array(items)) => items.is_empty(),
                _ => false,
            };
            if missing {
                object.insert(key.to_string(), value);
            }
        };

    match tool_name {
        "file_read" | "pdf_read" | "spreadsheet_read" => {
            set_if_missing(object, "path", serde_json::json!(input_file));
        }
        "file_write" | "pdf_create" | "browser_pdf" | "screenshot" => {
            set_if_missing(object, "path", serde_json::json!(output_file));
        }
        "content_search" => {
            set_if_missing(object, "path", serde_json::json!(workspace_dir));
        }
        "glob_search" => {
            set_if_missing(object, "root", serde_json::json!(workspace_dir));
        }
        "data_extractor" => {
            set_if_missing(
                object,
                "content",
                serde_json::json!("Sandbox document content. Key points: summarize uploaded documents, highlight action items, and surface risks."),
            );
        }
        "schedule" => {
            set_if_missing(object, "goal", serde_json::json!(step_description.trim()));
            set_if_missing(
                object,
                "run_at",
                serde_json::json!((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
            );
        }
        "external_db" => {
            let selected_db = role
                .tools
                .iter()
                .find_map(|tool| tool.strip_prefix("external_db:").map(String::from))
                .unwrap_or_else(|| "sandbox_db".into());
            let operation = infer_external_db_operation(step_description);
            set_if_missing(object, "db", serde_json::json!(selected_db));
            set_if_missing(object, "operation", serde_json::json!(operation));
            if matches!(operation, "query" | "explain" | "execute") {
                set_if_missing(object, "sql", serde_json::json!("SELECT 1 AS sandbox_check"));
            }
            if operation == "table_preview" {
                set_if_missing(object, "table", serde_json::json!("users"));
            }
        }
        "data_engine" => {
            set_if_missing(
                object,
                "records",
                serde_json::json!([
                    {
                        "id": "sandbox-user-1",
                        "email": "user@example.com",
                        "processed": false,
                        "created_at": "2026-03-31T00:00:00Z"
                    }
                ]),
            );
        }
        _ => {}
    }

    if object.get("path").is_none() && object.get("root").is_none() && tool_name == "content_search" {
        object.insert("path".into(), serde_json::json!(workspace_files));
    }
}

fn infer_external_db_operation(step_description: &str) -> &'static str {
    let lower = step_description.to_lowercase();
    if lower.contains("schema") || lower.contains("inspect") || lower.contains("discover") {
        "schema"
    } else if lower.contains("update")
        || lower.contains("write")
        || lower.contains("insert")
        || lower.contains("delete")
    {
        "execute"
    } else if lower.contains("preview") {
        "table_preview"
    } else if lower.contains("explain") {
        "explain"
    } else {
        "query"
    }
}

pub(crate) fn build_revision_prompt_from_test_result(test_result: &PlanModeTestResult) -> String {
    let rendered = serde_json::to_string_pretty(test_result).unwrap_or_else(|_| test_result.summary.clone());
    format!(
        "The deterministic plan test failed or only partially passed.\n\
Please repair the current draft using the structured test result below.\n\
Keep the workflow deterministic and only change what is needed so the plan will pass the next test run.\n\n\
TEST RESULT:\n{}\n",
        rendered
    )
}

fn is_explicit_review_confirmation(answer: &str) -> bool {
    let normalized = answer.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "y" | "yes"
            | "save"
            | "save it"
            | "save now"
            | "confirm"
            | "confirmed"
            | "approve"
            | "approved"
            | "ok"
            | "okay"
            | "looks good"
            | "looks good to me"
    )
}

fn synthetic_input_data_for_role(
    role: &AgentRole,
    session: &PlanModeSession,
    workspace_root: &Path,
) -> serde_json::Value {
    let mut values = serde_json::Map::new();
    let input_fixture = workspace_root.join("artifacts").join("sandbox_input.txt").display().to_string();
    let output_fixture = workspace_root.join("artifacts").join("sandbox_output.txt").display().to_string();

    values.insert("file_path".into(), serde_json::json!(input_fixture));
    values.insert(
        "path".into(),
        serde_json::json!(workspace_root.join("artifacts").join("sandbox_input.txt").display().to_string()),
    );
    values.insert("output_path".into(), serde_json::json!(output_fixture));
    values.insert("output".into(), serde_json::json!(output_fixture));
    values.insert(
        "content".into(),
        serde_json::json!("Sandbox document content. Key points: summarize uploaded documents, highlight action items, and surface risks."),
    );
    values.insert("code".into(), serde_json::json!("print('sandbox ok')"));
    values.insert("key".into(), serde_json::json!("sandbox-key"));
    values.insert("value".into(), serde_json::json!("sandbox value"));
    values.insert("sub_goal_1".into(), serde_json::json!("Read the uploaded document"));
    values.insert("sub_goal_2".into(), serde_json::json!("Summarize key points and risks"));
    values.insert("selector".into(), serde_json::json!(".content"));
    values.insert("attribute".into(), serde_json::json!("href"));
    values.insert("pattern".into(), serde_json::json!(".*"));
    values.insert("url".into(), serde_json::json!("https://example.com"));
    values.insert("topic".into(), serde_json::json!("sandbox test"));
    values.insert("query".into(), serde_json::json!("sandbox test query"));
    values.insert("message".into(), serde_json::json!("sandbox message"));
    values.insert("body".into(), serde_json::json!("sandbox message body"));
    values.insert("subject".into(), serde_json::json!("sandbox test subject"));
    values.insert("recipient".into(), serde_json::json!("test@example.com"));
    values.insert("input".into(), serde_json::json!({ "file_path": input_fixture, "output_path": output_fixture }));
    values.insert("workspace".into(), serde_json::json!(workspace_root.display().to_string()));

    if let Some(intent) = session.intent_cache.as_ref().and_then(|value| value.as_object()) {
        for (key, value) in intent {
            values.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }

    if let Some(compiled) = role.execution_guidelines.compiled_workflow.as_ref() {
        for step in &compiled.steps {
            let template = crate::agent::workflow_compiler::legacy_args_template_from_compiled_step(step);
            collect_template_placeholders(&template, &mut values, workspace_root);
        }
    }

    serde_json::Value::Object(values)
}

fn collect_template_placeholders(
    template: &serde_json::Value,
    values: &mut serde_json::Map<String, serde_json::Value>,
    workspace_root: &Path,
) {
    match template {
        serde_json::Value::String(text) => {
            for key in extract_input_placeholders(text) {
                values.entry(key.clone()).or_insert_with(|| synthetic_placeholder_value(&key, workspace_root));
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_template_placeholders(item, values, workspace_root);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_template_placeholders(value, values, workspace_root);
            }
        }
        _ => {}
    }
}

fn extract_input_placeholders(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut search_from = 0usize;
    while let Some(start_rel) = text[search_from..].find("{input.") {
        let start = search_from + start_rel + 7;
        let Some(end_rel) = text[start..].find('}') else {
            break;
        };
        let key = text[start..start + end_rel].trim();
        if !key.is_empty() {
            keys.push(key.to_string());
        }
        search_from = start + end_rel + 1;
    }
    keys
}

fn synthetic_placeholder_value(key: &str, workspace_root: &Path) -> serde_json::Value {
    let lower = key.to_ascii_lowercase();
    if lower.contains("file") || lower.contains("path") {
        return serde_json::json!(workspace_root.join("artifacts").join("sandbox_input.txt").display().to_string());
    }
    if lower.contains("url") {
        return serde_json::json!("https://example.com");
    }
    if lower.contains("email") || lower.contains("recipient") {
        return serde_json::json!("test@example.com");
    }
    if lower.contains("subject") {
        return serde_json::json!("sandbox test subject");
    }
    if lower.contains("body") || lower.contains("message") {
        return serde_json::json!("sandbox message body");
    }
    if lower.contains("query") || lower.contains("topic") {
        return serde_json::json!("sandbox test query");
    }
    serde_json::json!(format!("sandbox_{}", key.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_")))
}

fn value_contains_placeholder(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            lower.contains("{input.") || lower.contains("{{result_of_step_") || lower.contains("replace_me")
        }
        serde_json::Value::Array(items) => items.iter().any(value_contains_placeholder),
        serde_json::Value::Object(map) => map.values().any(value_contains_placeholder),
        _ => false,
    }
}

fn missing_required_args_for_schema(args: &serde_json::Value, schema: &[crate::tools::ParameterSchema]) -> Vec<String> {
    let Some(object) = args.as_object() else {
        return schema
            .iter()
            .filter(|parameter| parameter.required)
            .filter(|parameter| !is_runtime_injected_schema_field(&parameter.name))
            .map(|parameter| parameter.name.clone())
            .collect();
    };

    schema
        .iter()
        .filter(|parameter| parameter.required)
        .filter(|parameter| !is_runtime_injected_schema_field(&parameter.name))
        .filter(|parameter| match object.get(&parameter.name) {
            None | Some(serde_json::Value::Null) => true,
            Some(serde_json::Value::String(text)) => text.trim().is_empty(),
            Some(serde_json::Value::Array(items)) => items.is_empty(),
            _ => false,
        })
        .map(|parameter| parameter.name.clone())
        .collect()
}

fn is_runtime_injected_schema_field(name: &str) -> bool {
    matches!(name, "tenant_id" | "agent_id" | "parent_agent_id" | "role_id" | "goal_instance_id")
}

fn normalize_sandbox_tool_args(tool_name: &str, args: &mut serde_json::Value, workspace_root: &Path, role: &AgentRole) {
    let Some(object) = args.as_object_mut() else {
        return;
    };

    let absolutize_key = |object: &mut serde_json::Map<String, serde_json::Value>, key: &str| {
        if let Some(path) = object.get(key).and_then(|value| value.as_str()) {
            let resolved = if Path::new(path).is_absolute() {
                path.to_string()
            } else {
                workspace_root.join(path).display().to_string()
            };
            object.insert(key.to_string(), serde_json::Value::String(resolved));
        }
    };

    match tool_name {
        "file_read" | "file_write" | "file_edit" | "pdf_read" | "decompress" | "pdf_create" => {
            absolutize_key(object, "path");
        }
        "spreadsheet_read" | "spreadsheet_write" => {
            absolutize_key(object, "path");
        }
        "web_fetch" | "web_search_tool" => {}
        "http_request" | "api_call" => {
            if let Some(method) = object.get("method").and_then(|value| value.as_str()) {
                object.insert("method".into(), serde_json::Value::String(method.to_ascii_uppercase()));
            }
        }
        "external_db" | "external_api" => {
            object.entry("tenant_id").or_insert_with(|| serde_json::json!(role.tenant_id));
        }
        _ => {
            absolutize_key(object, "path");
        }
    }
}

fn sandbox_tool_policy(tool_name: &str, args: &serde_json::Value) -> SandboxPolicy {
    let lower = tool_name.to_ascii_lowercase();
    match lower.as_str() {
        "email" | "notification" | "pushover" => {
            SandboxPolicy::NoOp("outbound communication is log-only in sandbox".into())
        }
        "ask_user" => SandboxPolicy::NoOp("interactive step logged only in sandbox".into()),
        "file_read" | "pdf_read" | "spreadsheet_read" | "data_extractor" | "content_search" | "glob_search"
        | "memory_recall" | "vector_search" | "web_search_tool" | "web_fetch" | "browser_pdf" | "browser_open" => {
            SandboxPolicy::Allow
        }
        "external_db" | "external_api" => {
            if tool_args_is_read_only(args) {
                SandboxPolicy::Allow
            } else {
                SandboxPolicy::Block("database and API writes are blocked in sandbox".into())
            }
        }
        "http_request" | "api_call" => {
            if tool_args_is_read_only(args) {
                SandboxPolicy::Allow
            } else {
                SandboxPolicy::Block("HTTP writes are blocked in sandbox".into())
            }
        }
        "file_write"
        | "file_edit"
        | "pdf_create"
        | "decompress"
        | "create_workspace_tool"
        | "delegate"
        | "git_operations"
        | "sql_query" => {
            if lower == "sql_query" && tool_args_is_read_only(args) {
                SandboxPolicy::Allow
            } else {
                SandboxPolicy::Block("write or destructive actions are blocked in sandbox".into())
            }
        }
        _ => {
            if tool_args_is_read_only(args) {
                SandboxPolicy::Allow
            } else {
                SandboxPolicy::Block("unclassified tool is blocked in sandbox by default".into())
            }
        }
    }
}

fn tool_args_is_read_only(args: &serde_json::Value) -> bool {
    if let Some(method) = args.get("method").and_then(|value| value.as_str()) {
        return matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD" | "OPTIONS");
    }

    if let Some(operation) = args.get("operation").and_then(|value| value.as_str()) {
        let lower = operation.to_ascii_lowercase();
        return matches!(
            lower.as_str(),
            "read"
                | "get"
                | "list"
                | "query"
                | "fetch"
                | "schema"
                | "inspect"
                | "search"
                | "preview"
                | "describe"
                | "status"
                | "read_only"
        );
    }

    if let Some(query) = args.get("query").and_then(|value| value.as_str()) {
        let lower = query.to_ascii_lowercase();
        if lower.contains("insert")
            || lower.contains("update")
            || lower.contains("delete")
            || lower.contains("drop")
            || lower.contains("alter")
            || lower.contains("create ")
            || lower.contains("truncate")
            || lower.contains("replace")
        {
            return false;
        }
        return true;
    }

    true
}

fn combine_user_message_with_attachment_context(message: &str, attachment_context: &str) -> String {
    let trimmed = message.trim();
    let ctx = attachment_context.trim();
    if ctx.is_empty() {
        trimmed.to_string()
    } else {
        format!("{}\n\nUPLOADED DOCUMENT CONTEXT:\n{}", trimmed, ctx)
    }
}

fn sanitise_attachment_name(name: &str) -> String {
    let fallback = "attachment";
    let raw = Path::new(name).file_name().and_then(|s| s.to_str()).unwrap_or(fallback).trim();

    let cleaned: String = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') { ch } else { '_' })
        .collect();

    let cleaned = cleaned.trim_matches('_').to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

async fn unique_session_attachment_path(root: &Path, file_name: &str) -> Result<PathBuf> {
    let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("attachment");
    let ext = Path::new(file_name).extension().and_then(|s| s.to_str()).unwrap_or("");

    for index in 0usize.. {
        let candidate = if index == 0 {
            root.join(file_name)
        } else if ext.is_empty() {
            root.join(format!("{}-{}", stem, index))
        } else {
            root.join(format!("{}-{}.{}", stem, index, ext))
        };

        if tokio::fs::metadata(&candidate).await.is_err() {
            return Ok(candidate);
        }
    }

    unreachable!("attachment path generation should always terminate")
}

fn directory_size_bytes(root: &Path) -> Result<u64> {
    if !root.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    Ok(total)
}

fn infer_plan_mode_attachment_kind(
    path: &Path,
    mime_type: Option<&str>,
) -> crate::agent::definition::PlanModeAttachmentKind {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    let mime = mime_type.unwrap_or("").to_ascii_lowercase();

    match ext.as_str() {
        "pdf" => crate::agent::definition::PlanModeAttachmentKind::Pdf,
        "xls" | "xlsx" | "ods" => crate::agent::definition::PlanModeAttachmentKind::Spreadsheet,
        "csv" => crate::agent::definition::PlanModeAttachmentKind::Csv,
        "txt" | "md" | "markdown" | "json" | "jsonl" | "log" | "html" | "htm" | "xml" | "yaml" | "yml" | "rst"
        | "toml" => crate::agent::definition::PlanModeAttachmentKind::Text,
        _ if mime.contains("pdf") => crate::agent::definition::PlanModeAttachmentKind::Pdf,
        _ if mime.contains("csv") => crate::agent::definition::PlanModeAttachmentKind::Csv,
        _ if mime.contains("sheet") || mime.contains("excel") || mime.contains("spreadsheet") => {
            crate::agent::definition::PlanModeAttachmentKind::Spreadsheet
        }
        _ if mime.starts_with("text/") || mime.contains("json") || mime.contains("xml") || mime.contains("html") => {
            crate::agent::definition::PlanModeAttachmentKind::Text
        }
        _ if mime.is_empty() => crate::agent::definition::PlanModeAttachmentKind::Unknown,
        _ => crate::agent::definition::PlanModeAttachmentKind::Binary,
    }
}

fn attachment_kind_label(kind: &crate::agent::definition::PlanModeAttachmentKind) -> &'static str {
    match kind {
        crate::agent::definition::PlanModeAttachmentKind::Pdf => "pdf",
        crate::agent::definition::PlanModeAttachmentKind::Spreadsheet => "spreadsheet",
        crate::agent::definition::PlanModeAttachmentKind::Csv => "csv",
        crate::agent::definition::PlanModeAttachmentKind::Text => "text",
        crate::agent::definition::PlanModeAttachmentKind::Binary => "binary",
        crate::agent::definition::PlanModeAttachmentKind::Unknown => "unknown",
    }
}

fn apply_execution_hints(role: &mut AgentRole, intent: &serde_json::Value) {
    const TOOL_CATEGORY_RULE_PREFIX: &str = "Prefer these tool categories when relevant:";
    const CONNECTOR_CATEGORY_RULE_PREFIX: &str = "Prefer connectors from these categories when relevant:";

    // Clear old hint-derived rules so refreshes/reconfiguration do not leave stale copies.
    role.execution_guidelines.remove_rules_with_prefix(TOOL_CATEGORY_RULE_PREFIX);
    role.execution_guidelines.remove_rules_with_prefix(CONNECTOR_CATEGORY_RULE_PREFIX);
    role.execution_guidelines.remove_priority_prefix("step: ");

    let workflow_hints = crate::agent::plan_mode::review::workflow_hints_for_compilation(intent);
    for item in workflow_hints.into_iter().take(5) {
        role.execution_guidelines.add_priority(format!("step: {}", item.trim()));
    }

    let tool_categories: Vec<String> = intent["preferred_tool_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if !tool_categories.is_empty() {
        role.execution_guidelines.add_rule(crate::agent::definition::GuidelineRule::always(format!(
            "Prefer these tool categories when relevant: {}.",
            tool_categories.join(", ")
        )));
    }

    let connector_categories: Vec<String> = intent["needed_connector_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if !connector_categories.is_empty() {
        role.execution_guidelines.add_rule(crate::agent::definition::GuidelineRule::always(format!(
            "Prefer connectors from these categories when relevant: {}.",
            connector_categories.join(", ")
        )));
    }
}

fn finalize_saved_role_execution_strategy(role: &mut AgentRole) {
    if !matches!(role.execution_guidelines.execution_strategy, ExecutionStrategy::CoordinatorShell) {
        role.execution_guidelines.execution_strategy = ExecutionStrategy::DeterministicWorkflow;
    }
}

fn workflow_hints_for_compilation(intent: &serde_json::Value) -> Vec<String> {
    let mut hints: Vec<String> = intent
        .get("workflow_dsl")
        .and_then(|value| value.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|value| {
                    if let Some(text) = value.as_str() {
                        Some(text.trim().to_string())
                    } else {
                        value.as_object().and_then(|object| {
                            object
                                .get("description")
                                .or_else(|| object.get("type"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.trim().to_string())
                        })
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if hints.is_empty() {
        if let Some(actions) = intent["actions"].as_array() {
            hints.extend(actions.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_string())));
        }
    }

    if let Some(memo) = intent
        .get("_adaptive_research_memo")
        .and_then(|value| serde_json::from_value::<AdaptiveResearchMemo>(value.clone()).ok())
    {
        hints.extend(memo.workflow_hints);
    }

    let mut merged = Vec::new();
    for hint in hints {
        let normalized = hint.trim();
        if normalized.is_empty() {
            continue;
        }
        if !merged.iter().any(|existing: &String| existing.eq_ignore_ascii_case(normalized)) {
            merged.push(normalized.to_string());
        }
    }
    merged
}

fn clean_json_markdown_response(raw: &str) -> String {
    let trimmed = raw.trim();

    if let Some(start) = trimmed.find("```json") {
        let after_fence = trimmed[start + "```json".len()..].trim_start();
        if let Some(end) = after_fence.find("```") {
            let candidate = after_fence[..end].trim();
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
    }

    if let Some(start) = trimmed.find("```") {
        let after_fence = trimmed[start + 3..].trim_start_matches("json").trim_start();
        if let Some(end) = after_fence.find("```") {
            let candidate = after_fence[..end].trim();
            if !candidate.is_empty() {
                return candidate.to_string();
            }
        }
    }

    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                return trimmed[start..=end].trim().to_string();
            }
        }
    }

    trimmed.to_string()
}

fn fallback_plan_mode_research_memo(role: &AgentRole, intent: &serde_json::Value) -> AdaptiveResearchMemo {
    AdaptiveResearchMemo {
        summary: format!("Plan-mode research fallback for {}", role.purpose),
        findings: crate::agent::plan_mode::review::workflow_hints_for_compilation(intent).into_iter().take(4).collect(),
        assumptions: Vec::new(),
        risks: vec![
            "Research synthesis fell back to intent-derived hints because the memo response was not valid JSON.".into(),
        ],
        workflow_hints: crate::agent::plan_mode::review::workflow_hints_for_compilation(intent).into_iter().take(6).collect(),
    }
}

fn compute_plan_mode_goal_fingerprint(description: &str, intent: &serde_json::Value, role: &AgentRole) -> String {
    let payload = serde_json::json!({
        "goal": normalize_fingerprint_text(description),
        "category": normalize_fingerprint_text(intent["category"].as_str().unwrap_or("general")),
        "trigger_hint": normalize_fingerprint_text(intent["trigger_hint"].as_str().unwrap_or("manual")),
        "output_hint": normalize_fingerprint_text(intent["output_hint"].as_str().unwrap_or("workspace")),
        "actions": normalize_fingerprint_strings(
            intent["actions"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|value| value.as_str().map(normalize_fingerprint_text)).collect::<Vec<_>>())
                .unwrap_or_default(),
        ),
        "workflow_dsl": intent
            .get("workflow_dsl")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|value| value.as_object())
                    .map(|step| {
                        serde_json::json!({
                            "id": normalize_fingerprint_text(step.get("id").and_then(|value| value.as_str()).unwrap_or("")),
                            "type": normalize_fingerprint_text(step.get("type").and_then(|value| value.as_str()).unwrap_or("")),
                            "description": normalize_fingerprint_text(step.get("description").and_then(|value| value.as_str()).unwrap_or("")),
                            "resource_hint": normalize_fingerprint_text(step.get("resource_hint").and_then(|value| value.as_str()).unwrap_or("")),
                            "tool_hint": normalize_fingerprint_text(step.get("tool_hint").and_then(|value| value.as_str()).unwrap_or("")),
                            "success_criteria": step
                                .get("success_criteria")
                                .and_then(|value| value.as_array())
                                .map(|arr| normalize_fingerprint_strings(
                                    arr.iter().filter_map(|value| value.as_str().map(String::from)).collect::<Vec<_>>()
                                ))
                                .unwrap_or_default(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "connectors": normalize_fingerprint_strings(role.connectors.clone()),
        "tools": normalize_fingerprint_strings(role.tools.clone()),
    });

    let digest = Sha256::digest(serde_json::to_vec(&payload).unwrap_or_default());
    format!("pmg_{}", hex::encode(digest))
}

fn normalize_fingerprint_text(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| part.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_'))
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_fingerprint_strings(values: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> =
        values.into_iter().map(|value| normalize_fingerprint_text(&value)).filter(|value| !value.is_empty()).collect();
    out.sort();
    out.dedup();
    out
}

fn phase_rank(phase: &PlanModePhase) -> u8 {
    match phase {
        PlanModePhase::CapturingIntent => 0,
        PlanModePhase::ResolvingConnectors => 1,
        PlanModePhase::CapturingClarifications => 2,
        PlanModePhase::CapturingConstraints => 3,
        PlanModePhase::Reviewing => 4,
        PlanModePhase::Complete => 5,
    }
}

fn phase_for_reuse(phase: &PlanModePhase) -> PlanModePhase {
    match phase {
        PlanModePhase::Complete => PlanModePhase::Reviewing,
        other => other.clone(),
    }
}

/// Map a prose workflow hint to the best matching tool name and build an arg template.
/// Returns (tool_name, args_template).
fn resolve_tool_for_hint(
    hint: &str,
    connectors: &[String],
    role_tools: &[String],
) -> (Option<String>, Option<serde_json::Value>) {
    let lower = hint.to_lowercase();

    if is_schedule_trigger_hint(&lower) {
        return (None, None);
    }

    // Attempt to extract inline JSON arguments if the AI or User provided static tool parameters!
    let mut explicit_args = None;
    if let Some(start) = hint.find('{') {
        if let Some(end) = hint.rfind('}') {
            match serde_json::from_str::<serde_json::Value>(&hint[start..=end]) {
                Ok(params) if params.is_object() => {
                    explicit_args = Some(params);
                }
                _ => {}
            }
        }
    }

    // 1. Check for exact connector name match first
    for conn in connectors {
        if lower.contains(&conn.to_lowercase()) {
            let args = explicit_args.or_else(|| {
                let op = infer_connector_operation(&lower);
                Some(serde_json::json!({ "operation": op }))
            });
            return (Some(conn.clone()), args);
        }
    }

    // 2. Check for explicit role tool matches
    for tool in role_tools {
        if lower.contains(&tool.to_lowercase()) {
            return (Some(tool.clone()), explicit_args);
        }
    }

    // 3. Keep summary-style document review steps conceptual so the model can
    // summarize the content rather than forcing a brittle extraction tool.
    if lower.contains("key points")
        || lower.contains("action items")
        || lower.contains("risks")
        || lower.contains("main points")
        || lower.contains("summary")
        || lower.contains("summarize")
    {
        return (None, None);
    }

    if (lower.contains("read") || lower.contains("load") || lower.contains("open") || lower.contains("inspect"))
        && (lower.contains("uploaded")
            || lower.contains("attachment")
            || lower.contains("document")
            || lower.contains("file"))
    {
        return (Some("file_read".into()), Some(serde_json::json!({ "path": "{input.file_path}" })));
    }

    if lower.contains("extract") || lower.contains("parse") || lower.contains("pull data") {
        if let Some(args) = resolve_data_extractor_args(&lower) {
            return (Some("data_extractor".into()), Some(args));
        }
    }

    // 4. Keyword-based tool matching
    let tool_keywords: &[(&[&str], &str, Option<serde_json::Value>)] = &[
        (
            &["search", "find news", "look up", "research", "latest"],
            "web_search_tool",
            Some(serde_json::json!({ "query": "{input.topic}" })),
        ),
        (
            &["fetch", "scrape", "download page", "get url", "crawl"],
            "web_fetch",
            Some(serde_json::json!({ "url": "{input.url}" })),
        ),
        (
            &["email", "send email", "notify via email", "mail"],
            "email",
            Some(
                serde_json::json!({ "to": "{input.recipient}", "subject": "{input.subject}", "body": "{input.body}" }),
            ),
        ),
        (
            &["notify", "alert", "send notification", "push"],
            "notification",
            Some(serde_json::json!({ "message": "{input.message}" })),
        ),
        (
            &["write file", "save to file", "create file", "output file"],
            "file_write",
            Some(serde_json::json!({ "path": "{input.output_path}" })),
        ),
        (
            &["read file", "load file", "open file"],
            "file_read",
            Some(serde_json::json!({ "path": "{input.file_path}" })),
        ),
        (
            &["run code", "execute", "script", "calculate"],
            "code_run",
            Some(serde_json::json!({ "language": "python", "code": "{input.code}" })),
        ),
        (&["read pdf", "pdf"], "pdf_read", Some(serde_json::json!({ "path": "{input.file_path}" }))),
        (
            &["create pdf", "generate pdf"],
            "pdf_create",
            Some(serde_json::json!({ "content": "{input.content}", "path": "{input.output_path}" })),
        ),
        (
            &["spreadsheet", "csv", "excel"],
            "spreadsheet_read",
            Some(serde_json::json!({ "path": "{input.file_path}" })),
        ),
        (
            &["remember", "store memory", "save context"],
            "memory_store",
            Some(serde_json::json!({ "key": "{input.key}", "value": "{input.value}" })),
        ),
        (
            &["recall", "retrieve memory", "past context"],
            "memory_recall",
            Some(serde_json::json!({ "key": "{input.key}" })),
        ),
        (
            &["vector search", "similar", "semantic search"],
            "vector_search",
            Some(serde_json::json!({ "query": "{input.query}" })),
        ),
        (
            &["delegate", "spawn", "paralleli"],
            "delegate",
            Some(serde_json::json!({ "sub_goals": ["{input.sub_goal_1}", "{input.sub_goal_2}"] })),
        ),
        (&["api call", "http request", "rest api"], "http_request", None),
    ];

    for (keywords, tool_name, default_args) in tool_keywords {
        if keywords.iter().any(|kw| lower.contains(kw)) {
            return (Some((*tool_name).into()), default_args.clone());
        }
    }

    // 5. No tool match — explicit LLM worker step
    (
        Some(conceptual_step_tool_name().into()),
        Some(serde_json::json!({
            "instruction": hint,
            "response_format": "text",
        })),
    )
}

fn is_schedule_trigger_hint(lower: &str) -> bool {
    let schedule_terms = [
        "every ",
        "daily",
        "weekly",
        "monthly",
        "hourly",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "midnight",
        "noon",
        "morning",
        "evening",
        "at ",
        "am",
        "pm",
        "cron",
    ];

    let trigger_terms = [
        "trigger",
        "schedule",
        "runs on schedule",
        "run on schedule",
        "scheduled run",
        "recurring",
        "hourly agent",
        "daily agent",
    ];

    schedule_terms.iter().any(|kw| lower.contains(kw)) && trigger_terms.iter().any(|kw| lower.contains(kw))
}

fn resolve_data_extractor_args(lower: &str) -> Option<serde_json::Value> {
    let extract = if lower.contains("email") || lower.contains("emails") {
        Some("emails")
    } else if lower.contains("url") || lower.contains("urls") {
        Some("urls")
    } else if lower.contains("link") || lower.contains("links") {
        Some("links")
    } else if lower.contains("price") || lower.contains("cost") || lower.contains("amount") {
        Some("prices")
    } else if lower.contains("phone") || lower.contains("phones") {
        Some("phones")
    } else if lower.contains("table") || lower.contains("tables") || lower.contains("row") || lower.contains("csv") {
        Some("tables")
    } else if lower.contains("selector") || lower.contains("css") {
        Some("selector")
    } else if lower.contains("regex") || lower.contains("pattern") {
        Some("regex")
    } else {
        None
    }?;

    let mut args = serde_json::json!({
        "content": "{input.content}",
        "extract": extract,
    });
    if extract == "selector" {
        args["selector"] = serde_json::json!("{input.selector}");
        args["attribute"] = serde_json::json!("{input.attribute}");
    }
    if extract == "regex" {
        args["pattern"] = serde_json::json!("{input.pattern}");
    }

    Some(args)
}

/// Infer the connector operation from a workflow hint.
fn infer_connector_operation(hint: &str) -> &'static str {
    if hint.contains("update") || hint.contains("write") || hint.contains("post") {
        "update_record"
    } else if hint.contains("create") || hint.contains("add") || hint.contains("insert") {
        "create_record"
    } else if hint.contains("delete") || hint.contains("remove") {
        "delete_record"
    } else if hint.contains("list") || hint.contains("fetch") || hint.contains("get") || hint.contains("query") {
        "query_records"
    } else if hint.contains("send") || hint.contains("reply") || hint.contains("message") {
        "send_message"
    } else {
        "query_records"
    }
}

/// Parse a natural-language trigger description into a `TriggerDef`.
pub(crate) fn parse_trigger_from_text(answer: &str) -> TriggerDef {
    let lower = answer.to_lowercase();

    // Workforce event — "after another role", "when X finishes/completes"
    if lower.contains("after") && (lower.contains("role") || lower.contains("finish") || lower.contains("complet")) {
        return TriggerDef {
            trigger_type: TriggerType::WorkforceEvent,
            cron: None,
            source_connector: None,
            event_filter: None,
            input_mapping: None,
            ..Default::default()
        };
    }

    // Schedule — contains time/day keywords
    let schedule_keywords = [
        "every",
        "daily",
        "weekly",
        "monthly",
        "hourly",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "midnight",
        "noon",
        "morning",
        "evening",
        "at ",
        "am",
        "pm",
        "cron",
    ];
    if schedule_keywords.iter().any(|kw| lower.contains(kw)) {
        let cron = natural_to_cron(&lower);
        return TriggerDef {
            trigger_type: TriggerType::Schedule,
            cron: Some(cron),
            source_connector: None,
            event_filter: None,
            input_mapping: None,
            ..Default::default()
        };
    }

    // Webhook — "when X happens", "on new Y", connector name mentioned
    let webhook_keywords = [
        "when ",
        "on new",
        "on a new",
        "webhook",
        "salesforce",
        "hubspot",
        "github",
        "zendesk",
        "stripe",
        "intercom",
        "freshdesk",
        "pagerduty",
        "created",
        "updated",
        "received",
    ];
    if webhook_keywords.iter().any(|kw| lower.contains(kw)) {
        // Try to detect the source connector
        let connector_names = [
            "salesforce",
            "hubspot",
            "github",
            "zendesk",
            "slack",
            "jira",
            "notion",
            "gmail",
            "stripe",
            "intercom",
            "freshdesk",
            "pagerduty",
            "servicenow",
            "greenhouse",
            "docusign",
            "quickbooks",
            "dbt_cloud",
            "outlook",
        ];
        let source_connector = connector_names.iter().find(|&&c| lower.contains(c)).map(|&c| c.to_string());

        // Extract event filter (e.g. "lead created" → "lead_created")
        let event_filter = extract_event_filter(&lower);

        return TriggerDef {
            trigger_type: TriggerType::Webhook,
            cron: None,
            source_connector,
            event_filter,
            input_mapping: None,
            ..Default::default()
        };
    }

    // User message / on-demand
    if lower.contains("ask") || lower.contains("message") || lower.contains("chat") {
        return TriggerDef {
            trigger_type: TriggerType::UserMessage,
            cron: None,
            source_connector: None,
            event_filter: None,
            input_mapping: None,
            ..Default::default()
        };
    }

    // Default: manual
    TriggerDef {
        trigger_type: TriggerType::Manual,
        cron: None,
        source_connector: None,
        event_filter: None,
        input_mapping: None,
        ..Default::default()
    }
}

/// Convert natural-language schedule descriptions to cron expressions.
pub(crate) fn natural_to_cron(text: &str) -> String {
    let lower = text.to_lowercase();

    // Specific time extraction: "at 9am", "at 14:00", "at 3pm"
    let hour = if let Some(h) = extract_hour(&lower) { h } else { 9u32 };

    // Minute-level schedules (must be before "every hour")
    if lower.contains("every min") || lower.contains("every minute") {
        return "* * * * *".into();
    }
    if lower.contains("every 5 min") {
        return "*/5 * * * *".into();
    }
    if lower.contains("every 10 min") {
        return "*/10 * * * *".into();
    }
    if lower.contains("every 15 min") {
        return "*/15 * * * *".into();
    }

    if lower.contains("every hour") || lower.contains("hourly") {
        return format!("0 * * * *");
    }
    if lower.contains("every 30 min") || lower.contains("every half hour") {
        return format!("*/30 * * * *");
    }
    // Generic "every N min/minutes" (must be after specific checks above)
    if lower.contains("every") && lower.contains("min") {
        if let Some(n) = extract_number(&lower) {
            return format!("*/{} * * * *", n);
        }
        return "* * * * *".into();
    }
    if lower.contains("midnight") {
        return "0 0 * * *".into();
    }
    if lower.contains("noon") {
        return "0 12 * * *".into();
    }

    // Day of week
    let day = if lower.contains("monday") {
        Some("1")
    } else if lower.contains("tuesday") {
        Some("2")
    } else if lower.contains("wednesday") {
        Some("3")
    } else if lower.contains("thursday") {
        Some("4")
    } else if lower.contains("friday") {
        Some("5")
    } else if lower.contains("saturday") {
        Some("6")
    } else if lower.contains("sunday") {
        Some("0")
    } else {
        None
    };

    if let Some(d) = day {
        return format!("0 {} * * {}", hour, d);
    }
    if lower.contains("weekly") {
        return format!("0 {} * * 1", hour);
    }
    if lower.contains("daily") || lower.contains("every day") {
        return format!("0 {} * * *", hour);
    }
    if lower.contains("monthly") || lower.contains("every month") {
        return format!("0 {} 1 * *", hour);
    }
    if lower.contains("every") && lower.contains("hour") {
        if let Some(n) = extract_number(&lower) {
            return format!("0 */{} * * *", n);
        }
    }

    // Default: daily at 9am
    format!("0 {} * * *", hour)
}

fn extract_hour(text: &str) -> Option<u32> {
    // Match "9am", "9 am", "14:00", "3pm", "3 pm"
    let re_12h = regex::Regex::new(r"(\d{1,2})\s*(am|pm)").ok()?;
    let re_24h = regex::Regex::new(r"(\d{1,2}):(\d{2})").ok()?;

    if let Some(cap) = re_24h.captures(text) {
        let h: u32 = cap[1].parse().ok()?;
        return Some(h);
    }
    if let Some(cap) = re_12h.captures(text) {
        let h: u32 = cap[1].parse().ok()?;
        let is_pm = &cap[2] == "pm";
        return Some(if is_pm && h != 12 {
            h + 12
        } else if !is_pm && h == 12 {
            0
        } else {
            h
        });
    }
    None
}

fn extract_number(text: &str) -> Option<u32> {
    text.split_whitespace().find_map(|w| w.parse::<u32>().ok())
}

fn extract_event_filter(text: &str) -> Option<String> {
    // "lead created" → "lead_created", "pr opened" → "pr_opened", etc.
    let patterns = [
        ("lead created", "lead_created"),
        ("lead updated", "lead_updated"),
        ("opportunity", "opportunity_updated"),
        ("ticket created", "ticket_created"),
        ("ticket updated", "ticket_updated"),
        ("pr opened", "pull_request"),
        ("pull request", "pull_request"),
        ("issue created", "issues"),
        ("payment failed", "payment_intent.payment_failed"),
        ("subscription cancelled", "customer.subscription.deleted"),
        ("invoice failed", "invoice.payment_failed"),
        ("dispute", "charge.dispute.created"),
    ];
    patterns.iter().find(|(pattern, _)| text.contains(pattern)).map(|(_, filter)| filter.to_string())
}

// ── Intent-to-trigger converter ────────────────────────────────────────────

/// Build a TriggerDef from the IntentExtractor JSON output.
/// Returns the trigger and its confidence level.
pub(crate) fn intent_to_trigger(
    intent: &serde_json::Value,
) -> (TriggerDef, crate::agent::definition::TriggerConfidence) {
    use crate::agent::definition::TriggerConfidence;

    let confidence = match intent["trigger_confidence"].as_str().unwrap_or("medium") {
        "high" => TriggerConfidence::High,
        "low" => TriggerConfidence::Low,
        _ => TriggerConfidence::Medium,
    };

    let trigger_type = match intent["trigger_hint"].as_str().unwrap_or("manual") {
        "schedule" => TriggerType::Schedule,
        "webhook" => TriggerType::Webhook,
        "user_message" => TriggerType::UserMessage,
        _ => TriggerType::Manual,
    };

    let trigger = match trigger_type {
        TriggerType::Schedule => TriggerDef {
            trigger_type: TriggerType::Schedule,
            cron: intent["trigger_cron"].as_str().map(String::from),
            source_connector: None,
            event_filter: None,
            input_mapping: None,
            confidence: confidence.clone(),
            ..Default::default()
        },
        TriggerType::Webhook => TriggerDef {
            trigger_type: TriggerType::Webhook,
            source_connector: intent["trigger_source"].as_str().map(String::from),
            event_filter: intent["trigger_event"].as_str().map(String::from),
            cron: None,
            input_mapping: None,
            confidence: confidence.clone(),
            ..Default::default()
        },
        other => TriggerDef {
            trigger_type: other,
            cron: None,
            source_connector: None,
            event_filter: None,
            input_mapping: None,
            confidence: confidence.clone(),
            ..Default::default()
        },
    };

    (trigger, confidence)
}

/// Build the combined clarification question shown after intent extraction.
/// Covers trigger confirmation (if needed), output questions, and multi-role suggestion.
pub(crate) fn build_clarification_question(intent: &serde_json::Value) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Multi-role suggestion
    if intent["multi_role_suggested"].as_bool().unwrap_or(false) {
        if let Some(reason) = intent["multi_role_reason"].as_str() {
            let names: Vec<&str> = intent["responsibilities"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|r| r["name"].as_str()).collect())
                .unwrap_or_default();
            parts.push(format!(
                "**I see {} distinct responsibilities** — {}\n\n\
                 • **A) One role** — simpler, but mixes concerns\n\
                 • **B) {} separate roles** (recommended) — cleaner, easier to debug\n\
                 Which do you prefer?",
                names.len(),
                reason,
                names.len(),
            ));
        }
    }

    // Trigger confirmation (only if not high confidence)
    let confidence = intent["trigger_confidence"].as_str().unwrap_or("medium");
    if confidence != "high" {
        if let Some(q) = intent["trigger_confirmation"].as_str() {
            parts.push(q.to_string());
        } else {
            // Fallback: build confirmation from what we parsed
            let trigger_hint = intent["trigger_hint"].as_str().unwrap_or("manual");
            let cron = intent["trigger_cron"].as_str();
            match (trigger_hint, cron) {
                ("schedule", Some(c)) => {
                    parts.push(format!(
                        "**When should this run?** I guessed: `{}` — is that right? \
                         Or describe it differently (e.g. 'Every weekday at 8am London time').",
                        c
                    ));
                }
                ("schedule", None) => {
                    parts.push("**When should this run?** e.g. 'Every Monday at 9am', 'Daily at midnight'.".into());
                }
                ("webhook", _) => {
                    let src = intent["trigger_source"].as_str().unwrap_or("a connector");
                    let evt = intent["trigger_event"].as_str().unwrap_or("an event");
                    parts.push(format!(
                        "**Trigger confirmation:** Run when {} fires `{}`? Or describe the trigger.",
                        src, evt
                    ));
                }
                _ => {
                    parts.push("**When should this run?** Schedule / webhook / on-demand / after another role?".into());
                }
            }
        }
    }

    // Output questions
    // FIX: use an explicit has_output_questions flag so we always fall through
    // to the fallback when the LLM returns an empty array
    let output_questions: Vec<&str> = intent["output_questions"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|q| q.as_str()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    if !output_questions.is_empty() {
        parts.push(format!(
            "**Output details:**\n{}",
            output_questions.iter().map(|q| format!("- {}", q)).collect::<Vec<_>>().join("\n")
        ));
    } else {
        // Fallback — always ask if LLM returned no output questions or empty array
        let hint = intent["output_hint"].as_str().unwrap_or("workspace");
        let dest = intent["output_destination_hint"].as_str().unwrap_or("");
        if dest.is_empty() {
            let q = match hint {
                "email_draft" | "email_send" => {
                    "Where should the emails go — drafts in workspace, or sent via Gmail/Outlook?"
                }
                "connector_record" => "Which record should I update, and which field?",
                "slack_message" => "Which Slack channel?",
                "report" => "Where should the report be saved? (e.g. workspace/reports/ or email to stakeholders)",
                "notification" => "Where should notifications go — Slack, email, or in-app?",
                _ => "Where should the output go, and in what format?",
            };
            parts.push(format!("**Output:** {}", q));
        }
        // If dest is known, no question needed — output is clear enough
    }

    if parts.is_empty() {
        "How should this run and where should the output go?".into()
    } else {
        parts.join("\n\n")
    }
}

/// Returns the list of compliance/platform services that will be automatically
/// active for a given job category. Shown in the review summary so users know
/// what's running on their behalf.
fn active_services_for_category(category: &str) -> Vec<&'static str> {
    match category {
        "customer_support" => {
            vec!["SLA tracking (1hr first-response)", "PII redaction", "Citation recording", "Human review queue"]
        }
        "sales_revops" => vec!["PII redaction", "Citation recording", "Human review queue"],
        "finance_accounting" => vec!["PII redaction", "Citation recording", "Evidence packaging", "Human review queue"],
        "legal_contract" => vec!["PII redaction", "Citation recording", "Evidence packaging", "Human review queue"],
        "hr_people_ops" => vec!["PII redaction", "Citation recording", "Human review queue"],
        "devops" | "it_ops_itsm" => {
            vec!["SLA tracking", "Citation recording", "Evidence packaging", "Human review queue"]
        }
        "research_analyst" => vec!["PII redaction", "Citation recording", "Evidence packaging", "Human review queue"],
        "software_engineer" => vec!["Human review queue"],
        _ => vec![],
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::ConnectorAuthType;
    use crate::agent::TenantConnector;

    #[test]
    fn test_connector_resolver_matches_salesforce() {
        let intent = serde_json::json!({
            "data_sources": ["Salesforce CRM leads"],
            "write_targets": ["Salesforce"],
            "actions": ["query lead records", "update lead description"],
        });
        let installed = vec!["salesforce".into(), "slack".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, clarifying) = rt.block_on(ConnectorResolver::resolve(&intent, &installed, &[]));
        assert!(resolved.contains(&"salesforce".to_string()));
        assert!(clarifying.is_none());
    }

    #[test]
    fn test_connector_resolver_no_installed_match() {
        let intent = serde_json::json!({
            "data_sources": ["Salesforce"],
            "write_targets": [],
            "actions": ["query records"],
        });
        let installed: Vec<String> = vec!["slack".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, _) = rt.block_on(ConnectorResolver::resolve(&intent, &installed, &[]));
        assert!(!resolved.contains(&"salesforce".to_string()));
    }

    #[test]
    fn test_connector_resolver_tenant_connector_matched() {
        let intent = serde_json::json!({
            "data_sources": ["Acme ERP orders"],
            "write_targets": [],
            "actions": ["query orders"],
        });
        let installed: Vec<String> = vec![];
        let tc = TenantConnector {
            id: "tc-1".into(),
            tenant_id: "t-1".into(),
            name: "acme_erp".into(),
            category: "connector/erp".into(),
            base_url: "https://erp.acme.com".into(),
            auth_type: ConnectorAuthType::Bearer,
            auth_credential_key: None,
            source: crate::agent::definition::ConnectorSource::Manual,
            source_docs: None,
            endpoints: Vec::new(),
            summary: "Acme ERP: orders inventory customers".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, _) = rt.block_on(ConnectorResolver::resolve(&intent, &installed, &[tc]));
        assert!(resolved.contains(&"acme_erp".to_string()));
    }

    #[test]
    fn test_db_connector_returns_tool_override() {
        let intent = serde_json::json!({
            "data_sources": ["our production postgres"],
            "uses_external_db": "prod_db",
            "write_targets": [],
            "actions": ["query leads table"],
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_resolved, tools, _) = rt.block_on(ConnectorResolver::resolve(&intent, &[], &[]));
        assert!(tools.contains(&"external_db:prod_db".to_string()));
    }

    #[test]
    fn test_selected_db_name_is_persisted_in_intent() {
        let mut intent = serde_json::json!({
            "data_sources": ["monitor new users activity"],
            "uses_external_db": true,
            "write_targets": [],
            "actions": ["watch for new rows"],
        });

        persist_selected_external_db(&mut intent, "mainnarayan");

        assert_eq!(intent_named_external_db(&intent).as_deref(), Some("mainnarayan"));
        assert_eq!(intent["uses_external_db"].as_str(), Some("mainnarayan"));
    }

    #[test]
    fn test_db_connector_prompts_for_registration_when_name_is_unknown() {
        let intent = serde_json::json!({
            "data_sources": ["monitor new users activity"],
            "uses_external_db": true,
            "write_targets": [],
            "actions": ["watch for new rows"],
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_resolved, _tools, clarifying) = rt.block_on(ConnectorResolver::resolve(&intent, &[], &[]));
        let question = clarifying.expect("should ask to connect a database");
        let lower = question.to_lowercase();
        assert!(lower.contains("settings") || lower.contains("database connection"));
    }

    #[test]
    fn test_db_connector_prompts_for_selection_when_multiple_databases_exist() {
        let intent = serde_json::json!({
            "data_sources": ["monitor new users activity"],
            "uses_external_db": true,
            "write_targets": [],
            "actions": ["watch for new rows"],
        });
        let tenant_connectors = vec![
            TenantConnector {
                id: "db-1".into(),
                tenant_id: "t-1".into(),
                name: "mainnarayan".into(),
                category: "connector/database".into(),
                base_url: String::new(),
                auth_type: ConnectorAuthType::ConnectionString,
                auth_credential_key: None,
                source: crate::agent::definition::ConnectorSource::Manual,
                source_docs: None,
                endpoints: Vec::new(),
                summary: "Primary database".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            TenantConnector {
                id: "db-2".into(),
                tenant_id: "t-1".into(),
                name: "analytics".into(),
                category: "connector/database".into(),
                base_url: String::new(),
                auth_type: ConnectorAuthType::ConnectionString,
                auth_credential_key: None,
                source: crate::agent::definition::ConnectorSource::Manual,
                source_docs: None,
                endpoints: Vec::new(),
                summary: "Analytics database".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_resolved, _tools, clarifying) = rt.block_on(ConnectorResolver::resolve(&intent, &[], &tenant_connectors));
        let question = clarifying.expect("should ask which database to use");
        let lower = question.to_lowercase();
        assert!(lower.contains("multiple database connections installed"));
        assert!(lower.contains("mainnarayan"));
        assert!(lower.contains("analytics"));
    }

    #[test]
    fn test_db_connector_treats_tool_placeholder_as_unresolved() {
        let intent = serde_json::json!({
            "data_sources": ["monitor new users activity"],
            "uses_external_db": "external_db",
            "write_targets": [],
            "actions": ["watch for new rows"],
        });
        let tenant_connectors = vec![
            TenantConnector {
                id: "db-1".into(),
                tenant_id: "t-1".into(),
                name: "mainnarayan".into(),
                category: "connector/database".into(),
                base_url: String::new(),
                auth_type: ConnectorAuthType::ConnectionString,
                auth_credential_key: None,
                source: crate::agent::definition::ConnectorSource::Manual,
                source_docs: None,
                endpoints: Vec::new(),
                summary: "Primary database".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            TenantConnector {
                id: "db-2".into(),
                tenant_id: "t-1".into(),
                name: "analytics".into(),
                category: "connector/database".into(),
                base_url: String::new(),
                auth_type: ConnectorAuthType::ConnectionString,
                auth_credential_key: None,
                source: crate::agent::definition::ConnectorSource::Manual,
                source_docs: None,
                endpoints: Vec::new(),
                summary: "Analytics database".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_resolved, tools, clarifying) = rt.block_on(ConnectorResolver::resolve(&intent, &[], &tenant_connectors));
        assert!(tools.iter().all(|tool| !tool.starts_with("external_db:external_db")));
        let question = clarifying.expect("should ask which database to use when only a placeholder was provided");
        let lower = question.to_lowercase();
        assert!(lower.contains("multiple database connections installed"));
        assert!(lower.contains("mainnarayan"));
        assert!(lower.contains("analytics"));
    }

    #[test]
    fn test_api_detection_triggers_on_workflow_language() {
        let intent = serde_json::json!({
            "data_sources": ["internal rest api"],
            "write_targets": [],
            "actions": ["enrich user records via backend"],
        });
        assert!(intent_needs_api_connection(&intent));
    }

    #[test]
    fn test_mcp_detection_triggers_on_workflow_language() {
        let intent = serde_json::json!({
            "data_sources": ["mcp server"],
            "write_targets": [],
            "actions": ["update records through tools/call"],
        });
        assert!(intent_needs_mcp_connection(&intent));
    }

    #[test]
    fn test_acp_detection_triggers_on_internal_agent_language() {
        let intent = serde_json::json!({
            "data_sources": ["internal agent-to-agent workflow"],
            "write_targets": [],
            "actions": ["send a message to the internal agent"],
            "uses_acp_peer": "ops_acp",
        });
        assert!(intent_needs_acp_connection(&intent));
    }

    #[test]
    fn test_custom_connector_answer_matching_handles_api_mcp_and_acp() {
        let tenant_connectors = vec![
            TenantConnector {
                id: "api-1".into(),
                tenant_id: "t-1".into(),
                name: "acme_backend".into(),
                category: "connector/custom".into(),
                base_url: "https://api.acme.com".into(),
                auth_type: ConnectorAuthType::Bearer,
                auth_credential_key: None,
                source: crate::agent::definition::ConnectorSource::Manual,
                source_docs: None,
                endpoints: Vec::new(),
                summary: "Acme backend REST API".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            TenantConnector {
                id: "mcp-1".into(),
                tenant_id: "t-1".into(),
                name: "ops_mcp".into(),
                category: "connector/mcp".into(),
                base_url: "https://mcp.example.com".into(),
                auth_type: ConnectorAuthType::Bearer,
                auth_credential_key: None,
                source: crate::agent::definition::ConnectorSource::Manual,
                source_docs: None,
                endpoints: Vec::new(),
                summary: "Ops MCP server".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            TenantConnector {
                id: "acp-1".into(),
                tenant_id: "t-1".into(),
                name: "ops_acp".into(),
                category: "connector/acp".into(),
                base_url: "https://acp.example.com".into(),
                auth_type: ConnectorAuthType::Bearer,
                auth_credential_key: None,
                source: crate::agent::definition::ConnectorSource::Manual,
                source_docs: None,
                endpoints: Vec::new(),
                summary: "Internal agent exchange peer".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];

        assert_eq!(
            answer_mentions_tenant_api("Connected acme_backend", &tenant_connectors),
            Some("acme_backend".into())
        );
        assert_eq!(answer_mentions_tenant_mcp("Connected ops_mcp", &tenant_connectors), Some("ops_mcp".into()));
        assert_eq!(answer_mentions_tenant_acp("Connected ops_acp", &tenant_connectors), Some("ops_acp".into()));
    }

    #[test]
    fn test_connector_resolver_uses_candidate_connector_hint() {
        let intent = serde_json::json!({
            "data_sources": ["customer data"],
            "write_targets": [],
            "actions": ["sync records"],
            "candidate_connectors": ["hubspot"],
        });
        let installed = vec!["hubspot".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, clarifying) = rt.block_on(ConnectorResolver::resolve(&intent, &installed, &[]));
        assert!(resolved.contains(&"hubspot".to_string()));
        assert!(clarifying.is_none());
    }

    #[test]
    fn test_connector_resolver_prompts_for_missing_connector_category() {
        let intent = serde_json::json!({
            "data_sources": ["pipeline data"],
            "write_targets": [],
            "actions": ["update CRM records"],
            "needed_connector_categories": ["crm"],
        });
        let installed: Vec<String> = vec!["slack".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_resolved, _tools, clarifying) = rt.block_on(ConnectorResolver::resolve(&intent, &installed, &[]));
        let question = clarifying.expect("should ask for missing crm connector");
        assert!(question.to_lowercase().contains("crm connector"));
    }

    #[test]
    fn test_connector_resolver_skips_connector_clarification_for_local_document_workflow() {
        let intent = serde_json::json!({
            "data_sources": ["uploaded documents", "local files"],
            "write_targets": [],
            "actions": ["read uploaded documents", "summarize main points", "highlight action items"],
            "output_hint": "report",
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, tools, clarifying) = rt.block_on(ConnectorResolver::resolve(&intent, &[], &[]));
        assert!(resolved.is_empty());
        assert!(tools.is_empty());
        assert!(clarifying.is_none(), "local document workflows should not ask for connectors");
    }

    #[test]
    fn test_attachment_context_combines_message_and_documents() {
        let message = "Analyze these files.";
        let context = "Attachment: report.pdf (pdf, 1200 bytes)\n{\"text\":\"hello\"}";

        let combined = combine_user_message_with_attachment_context(message, context);
        assert!(combined.contains("Analyze these files."));
        assert!(combined.contains("UPLOADED DOCUMENT CONTEXT"));
        assert!(combined.contains("report.pdf"));
    }

    #[test]
    fn test_attachment_kind_detection_prefers_extension() {
        let pdf = infer_plan_mode_attachment_kind(Path::new("invoice.pdf"), None);
        let sheet = infer_plan_mode_attachment_kind(Path::new("data.xlsx"), None);
        let csv = infer_plan_mode_attachment_kind(Path::new("rows.csv"), None);
        let text = infer_plan_mode_attachment_kind(Path::new("notes.md"), None);

        assert_eq!(pdf, crate::agent::definition::PlanModeAttachmentKind::Pdf);
        assert_eq!(sheet, crate::agent::definition::PlanModeAttachmentKind::Spreadsheet);
        assert_eq!(csv, crate::agent::definition::PlanModeAttachmentKind::Csv);
        assert_eq!(text, crate::agent::definition::PlanModeAttachmentKind::Text);
    }

    #[test]
    fn test_goal_fingerprint_is_stable_for_normalized_goal_text() {
        let intent = serde_json::json!({
            "category": "general",
            "trigger_hint": "manual",
            "output_hint": "workspace",
            "actions": ["collect inputs", "draft summary"],
        });

        let role = AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary".into());
        let fp_a = compute_plan_mode_goal_fingerprint("   Draft   a summary  ", &intent, &role);
        let fp_b = compute_plan_mode_goal_fingerprint("draft a summary", &intent, &role);
        assert_eq!(fp_a, fp_b);
    }

    #[test]
    fn test_phase_for_reuse_caps_completed_sessions_at_reviewing() {
        assert_eq!(phase_for_reuse(&PlanModePhase::Complete), PlanModePhase::Reviewing);
        assert_eq!(phase_for_reuse(&PlanModePhase::Reviewing), PlanModePhase::Reviewing);
    }

    #[test]
    fn test_resolve_tool_for_hint_reads_uploaded_document_with_file_read() {
        let (tool, args) = resolve_tool_for_hint("Agent reads the uploaded file", &[], &[]);
        assert_eq!(tool.as_deref(), Some("file_read"));
        assert_eq!(args.unwrap()["path"], serde_json::json!("{input.file_path}"));
    }

    #[test]
    fn test_resolve_tool_for_hint_keeps_summary_extraction_conceptual() {
        let (tool, args) = resolve_tool_for_hint("Agent extracts key points, action items, and risks", &[], &[]);
        assert_eq!(tool.as_deref(), Some("llm_worker"));
        assert!(args.is_some());
    }

    #[test]
    fn test_resolve_tool_for_hint_keeps_schedule_trigger_conceptual() {
        let (tool, args) = resolve_tool_for_hint(
            "Schedule an hourly trigger for this agent",
            &["schedule".into()],
            &["schedule".into()],
        );
        assert_eq!(tool.as_deref(), Some("llm_worker"));
        assert!(args.is_some());
    }

    #[test]
    fn test_resolve_tool_for_hint_builds_data_extractor_args_for_emails() {
        let (tool, args) = resolve_tool_for_hint("Extract emails from the HTML", &[], &[]);
        assert_eq!(tool.as_deref(), Some("data_extractor"));
        let args = args.expect("expected args for data_extractor");
        assert_eq!(args["content"], serde_json::json!("{input.content}"));
        assert_eq!(args["extract"], serde_json::json!("emails"));
    }

    #[test]
    fn test_materialize_validation_tool_args_fills_missing_file_read_path() {
        let tmp = std::env::temp_dir().join("narayan-plan-mode-test");
        let role = AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary".into());
        let mut args = serde_json::json!({});
        materialize_validation_tool_args("file_read", "read the uploaded file", &mut args, &tmp, &role);
        assert_eq!(
            args["path"],
            serde_json::json!(tmp.join("artifacts").join("sandbox_input.txt").display().to_string())
        );
    }

    #[test]
    fn test_materialize_validation_tool_args_fills_schedule_db_and_data_engine_defaults() {
        let tmp = std::env::temp_dir().join("narayan-plan-mode-test");
        let mut role = AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary".into());
        role.tools.push("external_db:mainnarayan".into());

        let mut schedule_args = serde_json::json!({});
        materialize_validation_tool_args("schedule", "Schedule an hourly trigger", &mut schedule_args, &tmp, &role);
        assert!(schedule_args["goal"].as_str().unwrap_or_default().contains("Schedule an hourly trigger"));
        assert!(schedule_args["run_at"].as_str().is_some());

        let mut db_args = serde_json::json!({});
        materialize_validation_tool_args("external_db", "Query the database for new users", &mut db_args, &tmp, &role);
        assert_eq!(db_args["db"], serde_json::json!("mainnarayan"));
        assert_eq!(db_args["operation"], serde_json::json!("query"));

        let mut data_engine_args = serde_json::json!({});
        materialize_validation_tool_args(
            "data_engine",
            "Filter records before processing",
            &mut data_engine_args,
            &tmp,
            &role,
        );
        assert!(data_engine_args["records"].as_array().map(|arr| !arr.is_empty()).unwrap_or(false));
    }

    #[test]
    fn test_missing_required_args_ignores_runtime_injected_fields() {
        let args = serde_json::json!({ "content": "hello" });
        let schema = vec![
            crate::tools::ParameterSchema::required("content", "string", "Content"),
            crate::tools::ParameterSchema::required("tenant_id", "string", "Tenant ID"),
            crate::tools::ParameterSchema::required("agent_id", "string", "Agent ID"),
            crate::tools::ParameterSchema::required("parent_agent_id", "string", "Parent agent"),
        ];

        let missing = missing_required_args_for_schema(&args, &schema);
        assert!(missing.is_empty(), "runtime injected fields should be ignored in preflight");
    }

    #[test]
    fn test_parse_trigger_schedule() {
        let trigger = parse_trigger_from_text("every friday at 9am");
        assert_eq!(trigger.trigger_type, TriggerType::Schedule);
        assert_eq!(trigger.cron.as_deref(), Some("0 9 * * 5"));
    }

    #[test]
    fn test_parse_trigger_webhook() {
        let trigger = parse_trigger_from_text("when a new Salesforce lead is created");
        assert_eq!(trigger.trigger_type, TriggerType::Webhook);
        assert_eq!(trigger.source_connector.as_deref(), Some("salesforce"));
    }

    #[test]
    fn test_parse_trigger_manual() {
        let trigger = parse_trigger_from_text("on demand");
        assert_eq!(trigger.trigger_type, TriggerType::Manual);
    }

    #[test]
    fn test_parse_trigger_workforce_event() {
        let trigger = parse_trigger_from_text("after the lead enrichment role completes");
        assert_eq!(trigger.trigger_type, TriggerType::WorkforceEvent);
    }

    #[test]
    fn test_natural_to_cron_friday() {
        assert_eq!(natural_to_cron("every friday"), "0 9 * * 5");
    }

    #[test]
    fn test_natural_to_cron_midnight() {
        assert_eq!(natural_to_cron("daily at midnight"), "0 0 * * *");
    }

    #[test]
    fn test_every_minute_cron() {
        assert_eq!(natural_to_cron("every min"), "* * * * *");
        assert_eq!(natural_to_cron("every minute"), "* * * * *");
    }

    #[test]
    fn test_every_n_minutes_cron() {
        assert_eq!(natural_to_cron("every 5 min"), "*/5 * * * *");
        assert_eq!(natural_to_cron("every 10 min"), "*/10 * * * *");
        assert_eq!(natural_to_cron("every 15 min"), "*/15 * * * *");
        assert_eq!(natural_to_cron("every 7 minutes"), "*/7 * * * *");
    }

    #[test]
    fn test_build_custom_context_empty() {
        let ctx = super::registry::build_custom_context(&[], &[]);
        assert!(ctx.is_empty());
    }

    #[test]
    fn test_contains_connector_name_matches_token_only() {
        assert!(contains_connector_name("please use hubspot for this", "hubspot"));
        assert!(!contains_connector_name("please use hubspots for this", "hubspot"));
    }

    #[test]
    fn test_apply_execution_hints_replaces_old_category_rules_and_round_trips() {
        let mut role = AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary Role".into());
        role.execution_guidelines.add_rule(crate::agent::definition::GuidelineRule::always(
            "Prefer these tool categories when relevant: web.",
        ));
        role.execution_guidelines.add_rule(crate::agent::definition::GuidelineRule::always(
            "Prefer connectors from these categories when relevant: crm.",
        ));
        role.execution_guidelines.add_priority("step: old sequencing");

        let intent = serde_json::json!({
            "preferred_tool_categories": ["data", "web"],
            "needed_connector_categories": ["support", "crm"],
            "workflow_dsl": [
                {
                    "id": "step_1",
                    "type": "fetch_records",
                    "description": "fetch source records",
                    "resource_hint": "database",
                    "tool_hint": "sql_query",
                    "args_hint": {},
                    "success_criteria": ["records fetched"]
                },
                {
                    "id": "step_2",
                    "type": "compute",
                    "description": "transform records",
                    "resource_hint": null,
                    "tool_hint": "data_engine",
                    "args_hint": {},
                    "success_criteria": ["records transformed"]
                }
            ]
        });
        apply_execution_hints(&mut role, &intent);

        assert_eq!(role.execution_guidelines.preferred_tool_categories(), vec!["data".to_string(), "web".to_string()]);
        assert_eq!(
            role.execution_guidelines.preferred_connector_categories(),
            vec!["crm".to_string(), "support".to_string()]
        );
        assert_eq!(
            role.execution_guidelines.workflow_hints(),
            vec!["fetch source records".to_string(), "transform".to_string(), "write destination".to_string(),]
        );
    }

    #[test]
    fn test_workflow_hints_for_compilation_merges_research_memo_hints() {
        let intent = serde_json::json!({
            "workflow_dsl": [
                {
                    "id": "step_1",
                    "type": "compute",
                    "description": "inspect repository",
                    "resource_hint": null,
                    "tool_hint": null,
                    "args_hint": {},
                    "success_criteria": ["inspection complete"]
                },
                {
                    "id": "step_2",
                    "type": "llm_worker",
                    "description": "compile plan",
                    "resource_hint": null,
                    "tool_hint": null,
                    "args_hint": {},
                    "success_criteria": ["plan compiled"]
                }
            ],
            "_adaptive_research_memo": {
                "summary": "research summary",
                "findings": ["existing CI failures"],
                "assumptions": [],
                "risks": ["tests may require credentials"],
                "workflow_hints": ["inspect repository", "verify behavior independently"]
            }
        });

        let hints = crate::agent::plan_mode::review::workflow_hints_for_compilation(&intent);
        assert_eq!(
            hints,
            vec![
                "inspect repository".to_string(),
                "compile plan".to_string(),
                "verify behavior independently".to_string()
            ]
        );
    }

    #[test]
    fn test_finalize_saved_role_execution_strategy_normalizes_to_deterministic() {
        let mut role = AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary Role".into());
        role.execution_guidelines.execution_strategy = ExecutionStrategy::DeterministicWorkflow;

        crate::agent::plan_mode::review::finalize_saved_role_execution_strategy(&mut role);
        assert_eq!(role.execution_guidelines.execution_strategy, ExecutionStrategy::DeterministicWorkflow);

        role.execution_guidelines.execution_strategy = ExecutionStrategy::AdaptivePlanning;
        crate::agent::plan_mode::review::finalize_saved_role_execution_strategy(&mut role);
        assert_eq!(role.execution_guidelines.execution_strategy, ExecutionStrategy::DeterministicWorkflow);
    }

    #[test]
    fn test_plan_mode_scaffold_specs_tracks_research_stage() {
        let session = PlanModeSession {
            id: "pm-1".into(),
            tenant_id: "tenant-1".into(),
            draft_agent: AgentDefinition::new("agent-1".into(), "tenant-1".into(), "Planner".into()),
            draft_role: Some(AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary".into())),
            conversation: vec![],
            attachments: vec![],
            attachment_context: String::new(),
            session_workspace: None,
            goal_fingerprint: None,
            repair_version: 1,
            reused_from_session_id: None,
            repair_root_session_id: None,
            phase: PlanModePhase::Reviewing,
            compiler_stage: crate::agent::definition::PlanModeCompilerStage::Review,
            compiler_repair_passes: 0,
            compiler_validation_issues: Vec::new(),
            intent_cache: Some(serde_json::json!({
                "workflow_dsl": [
                    {
                        "id": "step_1",
                        "type": "compute",
                        "description": "inspect repository",
                        "resource_hint": null,
                        "tool_hint": null,
                        "args_hint": {},
                        "success_criteria": ["repository inspected"]
                    }
                ],
                "_adaptive_research_memo": {
                    "summary": "research summary",
                    "findings": [],
                    "assumptions": [],
                    "risks": [],
                    "workflow_hints": ["verify behavior independently"]
                }
            })),
            pending_steps: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let specs = crate::agent::plan_mode::review::plan_mode_scaffold_specs(&session);
        let research = specs
            .into_iter()
            .find(|(task_id, _, _, _, _, _)| task_id.ends_with(":research"))
            .expect("research scaffold should exist");
        assert_eq!(research.3, SessionTaskStatus::Completed);
    }
}



