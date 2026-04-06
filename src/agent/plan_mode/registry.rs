//! Connector resolution and capability-packet helpers for plan mode.
//!
//! `ConnectorResolver` maps extracted intent to specific connector names,
//! tool overrides (external_db, external_api, acp_session), and optional
//! clarifying questions when ambiguity exists.
//!
//! The delegation functions (`build_capability_packet`, etc.) forward to the
//! authoritative implementations in `crate::agent::plan_mode_registry`.

use crate::{
    agent::definition::TenantConnector,
    tools::ToolRegistry,
    tools::connector_tool::ALL_CONNECTORS as BUILTIN_CONNECTORS,
};

use super::intent::{
    intent_named_acp_peer, intent_named_external_db, intent_needs_acp_connection,
    intent_needs_database_connection,
};

// ── CapabilityPacket ─────────────────────────────────────────────────────

pub struct CapabilityPacket {
    pub capability_directory: String,
    pub registry_candidate_context: String,
}

pub fn build_capability_packet(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> CapabilityPacket {
    CapabilityPacket {
        capability_directory: crate::agent::plan_mode_registry::build_capability_directory(
            registry,
            installed,
            tenant_connectors,
        ),
        registry_candidate_context: crate::agent::plan_mode_registry::build_registry_candidate_context(
            registry,
            intent,
            installed,
            tenant_connectors,
        ),
    }
}

pub fn build_capability_directory(
    registry: &ToolRegistry,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    crate::agent::plan_mode_registry::build_capability_directory(registry, installed, tenant_connectors)
}

pub fn build_detailed_capability_context(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    crate::agent::plan_mode_registry::build_detailed_capability_context(registry, intent, installed, tenant_connectors)
}

pub fn build_registry_candidate_context(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    crate::agent::plan_mode_registry::build_registry_candidate_context(registry, intent, installed, tenant_connectors)
}

pub fn build_registry_candidate_set(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> serde_json::Value {
    crate::agent::plan_mode_registry::build_registry_candidate_set(registry, intent, installed, tenant_connectors)
}

pub fn inferred_preferred_tools(registry: &ToolRegistry, intent: &serde_json::Value) -> Vec<String> {
    crate::agent::plan_mode_registry::inferred_preferred_tools(registry, intent)
}

pub fn missing_tool_categories(intent: &serde_json::Value) -> Vec<String> {
    crate::agent::plan_mode_registry::missing_tool_categories(intent)
}

// ── ConnectorResolver ────────────────────────────────────────────────────

/// Maps extracted intent to specific connector names + tool overrides.
/// Returns (resolved_connectors, tool_overrides, clarifying_question)
/// tool_overrides are non-connector tools like external_db, external_api, or acp_session bindings
pub struct ConnectorResolver;

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

// ── Missing-connector question builder ───────────────────────────────────

pub(super) fn build_missing_connector_question(
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

// ── Local-document workflow detection ────────────────────────────────────

pub(super) fn text_mentions_local_document_workflow(text: &str) -> bool {
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

pub(super) fn intent_prefers_local_document_workflow(intent: &serde_json::Value) -> bool {
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

// ── Answer matching helpers ──────────────────────────────────────────────

pub(super) fn answer_declines_external_connector(answer_lower: &str) -> bool {
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

pub(super) fn answer_mentions_tenant_database(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category == "connector/database")
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

pub(super) fn answer_mentions_tenant_api(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category != "connector/database" && !tc.category.contains("mcp"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

pub(super) fn answer_mentions_tenant_mcp(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category.contains("mcp"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

pub(super) fn answer_mentions_tenant_acp(answer_lower: &str, tenant_connectors: &[TenantConnector]) -> Option<String> {
    tenant_connectors
        .iter()
        .filter(|tc| tc.category.contains("acp") || tc.category.contains("agent"))
        .find(|tc| contains_connector_name(answer_lower, &tc.name))
        .map(|tc| tc.name.clone())
}

// ── Connector name matching ──────────────────────────────────────────────

/// Returns true if any intent term meaningfully matches the connector's name/summary.
/// Uses proper tokenization (split on non-alphanumeric) rather than whitespace.
pub(super) fn terms_match_connector(all_terms: &[&str], tc: &TenantConnector) -> bool {
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

pub(super) fn contains_connector_name(answer_lower: &str, connector_name: &str) -> bool {
    let name = connector_name.to_ascii_lowercase();
    answer_lower.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-').any(|token| token == name)
}

// ── Tests ────────────────────────────────────────────────────────────────

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
