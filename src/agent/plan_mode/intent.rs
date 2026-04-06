use serde_json::Value;

use crate::agent::definition::TriggerDef;

/// Compact snapshot used by repair and summary prompts.
pub fn compact_intent_snapshot(initial: &Value) -> Value {
    let workflow_dsl = initial
        .get("workflow_dsl")
        .and_then(|value| value.as_array())
        .map(|steps| {
            steps
                .iter()
                .take(5)
                .map(|step| {
                    serde_json::json!({
                        "id": step.get("id").cloned().unwrap_or(Value::Null),
                        "type": step.get("type").cloned().unwrap_or(Value::Null),
                        "description": step.get("description").cloned().unwrap_or(Value::Null),
                        "tool": step.get("tool").cloned().unwrap_or(Value::Null),
                        "tool_operation": step.get("tool_operation").cloned().unwrap_or(Value::Null),
                        "resource_type": step.get("resource_type").cloned().unwrap_or(Value::Null),
                        "read_only": step.get("read_only").cloned().unwrap_or(Value::Null),
                        "depends_on": step.get("depends_on").cloned().unwrap_or(Value::Null),
                        "next_steps": step.get("next_steps").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    serde_json::json!({
        "category": initial.get("category").cloned().unwrap_or(Value::Null),
        "actions": initial.get("actions").cloned().unwrap_or(Value::Null),
        "data_sources": initial.get("data_sources").cloned().unwrap_or(Value::Null),
        "write_targets": initial.get("write_targets").cloned().unwrap_or(Value::Null),
        "output_hint": initial.get("output_hint").cloned().unwrap_or(Value::Null),
        "preferred_tool_categories": initial.get("preferred_tool_categories").cloned().unwrap_or(Value::Null),
        "preferred_tools": initial.get("preferred_tools").cloned().unwrap_or(Value::Null),
        "candidate_connectors": initial.get("candidate_connectors").cloned().unwrap_or(Value::Null),
        "missing_capabilities": initial.get("missing_capabilities").cloned().unwrap_or(Value::Null),
        "workflow_dsl": workflow_dsl,
    })
}

/// First-class subsystems that the final agent definition should make explicit.
pub const AGENT_SUBSYSTEMS: &[&str] = &[
    "memory",
    "knowledge",
    "swarm",
    "scheduler",
    "skills",
    "storage",
    "workspace",
];

fn text_mentions_local_document_workflow(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "local document",
        "workspace file",
        "uploaded document",
        "uploaded file",
        "workspace summary",
        "private file",
        "local file",
        "repository file",
    ]
    .iter()
    .any(|term| lower.contains(term))
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

pub fn intent_named_external_db(intent: &Value) -> Option<String> {
    intent
        .get("uses_external_db")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null")
        .map(String::from)
}

pub fn intent_named_acp_peer(intent: &Value) -> Option<String> {
    intent
        .get("uses_acp_peer")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null")
        .map(String::from)
}

pub fn intent_needs_database_connection(intent: &Value) -> bool {
    intent["missing_capabilities"]
        .as_array()
        .map(|arr| arr.iter().any(|value| value.as_str() == Some("custom_db")))
        .unwrap_or(false)
        || intent["uses_external_db"].as_bool().unwrap_or(false)
        || intent_contains_database_terms(intent)
}

pub fn intent_needs_api_connection(intent: &Value) -> bool {
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

pub fn intent_needs_mcp_connection(intent: &Value) -> bool {
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

pub fn intent_needs_acp_connection(intent: &Value) -> bool {
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

pub fn intent_to_trigger(_intent: &Value) -> TriggerDef {
    crate::agent::plan_mode::parse_trigger_from_text("on demand")
}

fn intent_text_for_keys(intent: &Value, keys: &[&str]) -> String {
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

fn intent_contains_database_terms(intent: &Value) -> bool {
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

fn intent_contains_api_terms(intent: &Value) -> bool {
    let text = intent_text_for_keys(intent, &["data_sources", "write_targets", "actions", "workflow_dsl"]);
    let lower = text.to_lowercase();
    ["rest api", "api", "endpoint", "endpoints", "backend", "http", "web service", "service api", "internal api"]
        .iter()
        .any(|term| lower.contains(term))
}

fn intent_contains_mcp_terms(intent: &Value) -> bool {
    let text = intent_text_for_keys(intent, &["data_sources", "write_targets", "actions", "workflow_dsl"]);
    let lower = text.to_lowercase();
    ["mcp", "model context protocol", "tools/list", "tools/call", "json-rpc", "json rpc", "mcp server"]
        .iter()
        .any(|term| lower.contains(term))
}

fn intent_contains_acp_terms(intent: &Value) -> bool {
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
