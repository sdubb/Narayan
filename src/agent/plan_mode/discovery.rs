//! Executor meta-tool discovery service.
//!
//! Extracts the connector meta-tool interception logic from `executor.rs`
//! into free functions that the executor delegates to. This keeps the
//! discovery/expansion concern in the plan-mode layer.

use crate::{
    agent::definition::TenantConnector,
    providers::ToolSpec,
    storage::PostgresStore,
    tools::{parameters_schema_to_json, ParameterSchema, ToolRegistry, HIDDEN_TOOLS},
};

/// Names of tools that are intercepted as meta-tools by the executor
/// before reaching the normal tool registry.
pub const META_TOOL_NAMES: &[&str] = &[
    "list_connectors_in_category",
    "request_more_connectors",
    "create_custom_connector",
    "request_more_tools",
    "tool_search",
    "create_workspace_tool",
];

// ── Cache helpers ────────────────────────────────────────────────────────

fn tool_schema_cache_key(agent_id: &str) -> String {
    format!("tool_schema_cache:{}", agent_id)
}

pub fn cached_tool_schema_names(agent_id: &str) -> Vec<String> {
    crate::tools::memory_store_internal::get(&tool_schema_cache_key(agent_id))
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
}

pub fn cache_tool_schema_names(agent_id: &str, names: &[String]) {
    let mut cached = cached_tool_schema_names(agent_id);
    for name in names {
        if !cached.contains(name) {
            cached.push(name.clone());
        }
    }
    crate::tools::memory_store_internal::insert(
        tool_schema_cache_key(agent_id),
        serde_json::to_string(&cached).unwrap_or_else(|_| "[]".into()),
    );
}

// ── Connector catalogue ─────────────────────────────────────────────────

/// Returns the built-in connector catalogue as an iterator of (category_suffix, name, summary).
/// Delegates to connector_tool::ALL_CONNECTORS so there is a single source of truth.
pub fn builtin_connector_catalogue() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    crate::tools::connector_tool::catalogue_entries()
}

/// Build a ToolSpec for a TenantConnector so it can be injected into the executor's
/// live toolset during the connector expansion loop.
pub fn build_tenant_connector_spec(tc: &TenantConnector) -> ToolSpec {
    let ops: Vec<String> =
        tc.endpoints.iter().map(|e| format!("{} {} — {}", e.method, e.path, e.description)).collect();

    let ops_hint = if ops.is_empty() { format!("Custom connector at {}", tc.base_url) } else { ops.join("; ") };

    let description = format!(
        "{}. Use when: the agent needs this tenant connector. Input: {{ operation, params?, auth_token? }}; tenant_id, goal_instance_id, and step_index are injected by the executor. Output: connector-specific JSON from the selected operation. The exact fields depend on the endpoint. Operations: {}",
        tc.summary,
        &ops_hint[..ops_hint.len().min(500)],
    );

    ToolSpec {
        name: tc.name.clone(),
        description,
        parameters: parameters_schema_to_json(&[
            ParameterSchema::required("operation", "string", "The operation/endpoint to call."),
            ParameterSchema::optional("params", "object", "Operation parameters as a JSON object."),
            ParameterSchema::optional("auth_token", "string", "Optional override bearer token."),
        ]),
        output_schema: Some(serde_json::json!({
            "type": "object",
            "additionalProperties": true,
        })),
    }
}

// ── Meta-tool handler ───────────────────────────────────────────────────

/// Handle a connector meta-tool call inline, before it reaches the registry.
///
/// Mutates `tool_specs` to add newly resolved connector tools so the next
/// LLM call in the expansion loop has them available.
/// Returns a JSON value describing the result, which is injected back as
/// a synthetic tool result message.
pub async fn handle_meta_tool(
    tool_name: &str,
    args: &serde_json::Value,
    tenant_id: &str,
    agent_id: &str,
    tools: &ToolRegistry,
    store: Option<&PostgresStore>,
    tool_specs: &mut Vec<ToolSpec>,
) -> serde_json::Value {
    match tool_name {
        "list_connectors_in_category" => {
            let category = args["category"].as_str().unwrap_or("all");
            let mut connectors: Vec<serde_json::Value> = Vec::new();

            // Built-in connectors from connector_tool::ALL_CONNECTORS (single source of truth)
            for (cat_suffix, name, summary) in builtin_connector_catalogue() {
                if category == "all" || cat_suffix == category {
                    connectors.push(serde_json::json!({
                        "name":     name,
                        "category": format!("connector/{}", cat_suffix),
                        "summary":  summary,
                    }));
                }
            }

            // Tenant custom connectors
            if let Some(store) = store {
                let tenant_conns = if category == "all" {
                    store.list_tenant_connectors(tenant_id).await.unwrap_or_default()
                } else {
                    let cat = format!("connector/{}", category);
                    store.list_tenant_connectors_by_category(tenant_id, &cat).await.unwrap_or_default()
                };
                for tc in &tenant_conns {
                    connectors.push(serde_json::json!({
                        "name":     tc.name,
                        "category": tc.category,
                        "summary":  tc.summary,
                    }));
                    // Pre-inject full ToolSpec for tenant connectors
                    let already = tool_specs.iter().any(|s| s.name == tc.name);
                    if !already {
                        tool_specs.push(build_tenant_connector_spec(tc));
                    }
                }
            }

            // Pre-inject full ToolSpecs for all listed built-in connectors so the
            // LLM can call them immediately without another round-trip.
            let current_names: std::collections::HashSet<String> =
                tool_specs.iter().map(|s| s.name.clone()).collect();
            for connector_json in &connectors {
                if let Some(name) = connector_json["name"].as_str() {
                    if !current_names.contains(name) {
                        if let Some(spec) = tools.get(name) {
                            tool_specs.push(crate::tools::tool_spec_from_tool(spec.as_ref()));
                        }
                    }
                }
            }

            serde_json::json!({
                "category":    category,
                "connectors":  connectors,
                "instruction": "Pick the connector you need by name. \
                                Call it directly as a tool — its full spec is now injected.",
            })
        }

        "request_more_connectors" => {
            let category = args["category"].as_str().unwrap_or("");
            let reason = args["reason"].as_str().unwrap_or("");

            // Check if there are any tenant connectors in this category not yet in tool_specs
            let current_names: std::collections::HashSet<String> =
                tool_specs.iter().map(|s| s.name.clone()).collect();
            let full_cat = format!("connector/{}", category);
            let more_available = if let Some(store) = store {
                store
                    .list_tenant_connectors_by_category(tenant_id, &full_cat)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|tc| !current_names.contains(&tc.name))
                    .count()
                    > 0
            } else {
                false
            };

            if more_available {
                serde_json::json!({
                    "status": "more_available",
                    "message": format!("Additional {} connectors found. Use list_connectors_in_category to see them.", category),
                })
            } else {
                serde_json::json!({
                    "status": "exhausted",
                    "category": category,
                    "reason": reason,
                    "options": [
                        {
                            "action": "create_custom_connector",
                            "description": "Add a custom connector by providing the API URL, \
                                            auth details, and endpoint descriptions or docs."
                        },
                        {
                            "action": "ask_user",
                            "description": "Ask the user which service they use and how to connect to it."
                        }
                    ],
                })
            }
        }

        "create_custom_connector" => {
            let name = args["name"].as_str().unwrap_or("").to_string();
            let category_raw = args["category"].as_str().unwrap_or("custom").to_string();
            let category = if category_raw.starts_with("connector/") {
                category_raw.clone()
            } else {
                format!("connector/{}", category_raw)
            };
            let base_url = args["base_url"].as_str().unwrap_or("").to_string();
            let auth_type_str = args["auth_type"].as_str().unwrap_or("bearer");
            let cred_key = args["auth_credential_key"].as_str().map(String::from);
            let summary = args["summary"].as_str().unwrap_or(&name).to_string();
            let source_docs = args["api_docs"].as_str().map(String::from);
            let creation_path = args["creation_path"].as_str().unwrap_or("manual");

            if name.is_empty() || base_url.is_empty() {
                return serde_json::json!({
                    "error": "name and base_url are required to create a custom connector"
                });
            }

            let auth_type = match auth_type_str {
                "api_key_header" => {
                    let hname = args["auth_header_name"].as_str().unwrap_or("X-API-Key");
                    crate::agent::definition::ConnectorAuthType::ApiKeyHeader { header_name: hname.to_string() }
                }
                "basic" => crate::agent::definition::ConnectorAuthType::Basic,
                "none" => crate::agent::definition::ConnectorAuthType::None,
                _ => crate::agent::definition::ConnectorAuthType::Bearer,
            };

            let source = match creation_path {
                "known_saas" => {
                    let product = args["product_name"].as_str().unwrap_or(&name).to_string();
                    crate::agent::definition::ConnectorSource::KnownSaas { product_name: product }
                }
                "api_docs" => crate::agent::definition::ConnectorSource::ApiDocs,
                _ => crate::agent::definition::ConnectorSource::Manual,
            };

            // Parse endpoints from args if provided
            let endpoints: Vec<crate::agent::definition::EndpointDef> = args["endpoints"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            Some(crate::agent::definition::EndpointDef {
                                method: e["method"].as_str().unwrap_or("GET").to_string(),
                                path: e["path"].as_str().unwrap_or("").to_string(),
                                description: e["description"].as_str().unwrap_or("").to_string(),
                                params: Vec::new(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            let tc = crate::agent::definition::TenantConnector {
                id: uuid::Uuid::new_v4().to_string(),
                tenant_id: tenant_id.to_string(),
                name: name.clone(),
                category: category.clone(),
                base_url: base_url.clone(),
                auth_type,
                auth_credential_key: cred_key,
                source,
                source_docs,
                endpoints,
                summary: summary.clone(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };

            // Save to DB
            if let Some(store) = store {
                if let Err(e) = store.upsert_tenant_connector(&tc).await {
                    tracing::error!(error = %e, connector = %name, "failed to save custom connector");
                    return serde_json::json!({ "error": format!("failed to save connector: {}", e) });
                }
            }

            // Build a live ToolSpec for this connector and inject into tool_specs
            let spec = build_tenant_connector_spec(&tc);
            let already_there = tool_specs.iter().any(|s| s.name == spec.name);
            if !already_there {
                tool_specs.push(spec);
            }

            tracing::info!(
                tenant_id = %tenant_id,
                connector = %name,
                category  = %category,
                "custom connector created and injected"
            );

            serde_json::json!({
                "status":   "created",
                "name":     name,
                "category": category,
                "message":  format!("Connector '{}' is now available. Call it as a tool.", name),
            })
        }

        "request_more_tools" => {
            // Expand core tool categories — distinct from connector expansion.
            let categories_value: Vec<serde_json::Value> =
                args["categories"].as_array().cloned().unwrap_or_default();
            let categories: Vec<String> =
                categories_value.iter().filter_map(|v| v.as_str().map(String::from)).collect();

            if categories.is_empty() {
                return serde_json::json!({
                    "error": "'categories' must be a non-empty array"
                });
            }

            let mut added: Vec<String> = Vec::new();
            let mut current_names: std::collections::HashSet<String> =
                tool_specs.iter().map(|s| s.name.clone()).collect();

            for cat in &categories {
                let new_specs = tools.tool_specs_for_category(cat);
                for spec in new_specs {
                    if !current_names.contains(&spec.name) {
                        current_names.insert(spec.name.clone());
                        added.push(spec.name.clone());
                        tool_specs.push(spec);
                    }
                }
            }

            let mut category_names: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            for spec in tool_specs.iter() {
                if let Some(tool) = tools.get(&spec.name) {
                    category_names.entry(tool.category().to_string()).or_default().push(spec.name.clone());
                }
            }
            for tool_names in category_names.values_mut() {
                tool_names.sort();
                tool_names.dedup();
            }
            let category_preview: Vec<String> = category_names
                .into_iter()
                .map(|(category, mut names)| {
                    names.truncate(8);
                    format!("{category}: {}", names.join(", "))
                })
                .collect();

            tracing::info!(
                tenant_id  = %tenant_id,
                categories = ?categories,
                added      = ?added,
                "request_more_tools: expanded toolset"
            );

            serde_json::json!({
                "status":              "expanded",
                "requested_categories": categories,
                "tools_added":         added,
                "available_categories": category_preview,
                "message":             "Your toolset has been expanded. Use the new tools in your next action.",
            })
        }

        "tool_search" => {
            let query = args["query"].as_str().unwrap_or("").trim().to_ascii_lowercase();
            if query.is_empty() {
                return serde_json::json!({ "error": "'query' is required" });
            }

            let requested_names = args["tool_names"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            let limit = args["limit"].as_u64().unwrap_or(8).clamp(1, 20) as usize;

            let mut matches = tools
                .list()
                .into_iter()
                .filter(|name| !HIDDEN_TOOLS.contains(name))
                .filter_map(|name| {
                    let tool = tools.get(name)?;
                    let contract = format!("{} {}", tool.name(), tool.description()).to_ascii_lowercase();
                    let score = if tool.name().eq_ignore_ascii_case(&query) {
                        100
                    } else if contract.contains(&query) {
                        10
                    } else {
                        crate::tools::selector::keyword_score(
                            std::slice::from_ref(&query),
                            tool.name(),
                            tool.description(),
                        )
                    };
                    if score == 0 {
                        None
                    } else {
                        Some((
                            score,
                            tool.name().to_string(),
                            tool.category().to_string(),
                            tool.description().to_string(),
                        ))
                    }
                })
                .collect::<Vec<_>>();
            matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

            let mut loaded = Vec::new();
            let mut current_names: std::collections::HashSet<String> =
                tool_specs.iter().map(|spec| spec.name.clone()).collect();
            for name in &requested_names {
                if current_names.contains(name) {
                    loaded.push(name.clone());
                    continue;
                }
                if let Some(tool) = tools.get(name) {
                    current_names.insert(name.clone());
                    loaded.push(name.clone());
                    tool_specs.push(crate::tools::tool_spec_from_tool(tool.as_ref()));
                }
            }
            if !loaded.is_empty() {
                cache_tool_schema_names(agent_id, &loaded);
            }

            serde_json::json!({
                "status": if loaded.is_empty() { "matches" } else { "loaded" },
                "query": query,
                "matches": matches
                    .into_iter()
                    .take(limit)
                    .map(|(_, name, category, description)| {
                        let loaded_now = current_names.contains(&name);
                        serde_json::json!({
                            "name": name,
                            "category": category,
                            "description": description,
                            "loaded": loaded_now,
                        })
                    })
                    .collect::<Vec<_>>(),
                "loaded_tools": loaded,
                "message": "Use tool_names with exact matches to load the schemas you need.",
            })
        }

        "create_workspace_tool" => {
            serde_json::json!({
                "status": "blocked",
                "error": "create_workspace_tool is disabled at runtime",
                "message": "Create and test custom tools during plan mode (or pre-register tenant WASM tools), then execute only approved tools via run_registered_wasm."
            })
        }

        other => {
            serde_json::json!({ "error": format!("unknown meta-tool: {}", other) })
        }
    }
}
