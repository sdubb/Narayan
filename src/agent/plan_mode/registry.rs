//! Connector resolution and capability-packet helpers for plan mode.
//!
//! `ConnectorResolver` maps extracted intent to specific connector names,
//! tool overrides (external_db, external_api, acp_session), and optional
//! clarifying questions when ambiguity exists.
//!
//! Registry helper functions (capability directory, candidate sets, etc.)
//! are the authoritative implementations — previously in `plan_mode_registry.rs`.

use std::collections::BTreeMap;

use crate::{
    agent::definition::TenantConnector,
    tools::{
        connector_tool::ALL_CONNECTORS as BUILTIN_CONNECTORS,
        render_output_schema,
        toolregistry::{DslStepType, ResourceKind, ToolFamily, ToolRegistryEntry, TOOL_REGISTRY},
        ToolRegistry,
    },
};

use super::intent::{
    intent_named_acp_peer, intent_named_external_db, intent_needs_acp_connection,
    intent_needs_database_connection,
};

// ── Registry helper functions (absorbed from plan_mode_registry.rs) ─────

fn render_tool_preview(registry: &ToolRegistry, category: &str) -> String {
    let specs = registry.tool_specs_for_category(category);
    if specs.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push(format!("Tools for category '{}':", category));
    for spec in specs.into_iter().take(8) {
        let param_summary = spec.parameters["properties"]
            .as_object()
            .map(|properties| {
                let mut parameters = Vec::new();
                let required_names = spec.parameters["required"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<std::collections::HashSet<_>>())
                    .unwrap_or_default();
                for (name, value) in properties {
                    let param_type = value["type"].as_str().unwrap_or("unknown");
                    let required = if required_names.contains(name.as_str()) { "required" } else { "optional" };
                    parameters.push(format!("{}:{} ({})", name, param_type, required));
                }
                if parameters.is_empty() { "none".to_string() } else { parameters.join(", ") }
            })
            .unwrap_or_else(|| "none".to_string());
        let output_schema = spec
            .output_schema
            .as_ref()
            .map(|schema| format!("\n    output_schema: {}", render_output_schema(schema)))
            .unwrap_or_default();
        lines.push(format!(
            "  - {}:\n    {}\n    parameters: {}{}",
            spec.name, spec.description, param_summary, output_schema
        ));
    }

    let tenant_connectors: &[TenantConnector] = &[];
    let acps: Vec<&TenantConnector> =
        tenant_connectors.iter().filter(|tc| tc.category.contains("acp") || tc.category.contains("agent")).collect();
    if !acps.is_empty() {
        lines.push("ACP peers (internal agent-to-agent connections):".into());
        for acp in &acps {
            lines.push(format!("  - name='{}' \u{2014} {}", acp.name, acp.summary));
        }
    }

    let acps: Vec<&TenantConnector> =
        tenant_connectors.iter().filter(|tc| tc.category.contains("acp") || tc.category.contains("agent")).collect();
    if !acps.is_empty() {
        lines.push("ACP peers (internal agent-to-agent connections):".into());
        for acp in &acps {
            lines.push(format!("  - name='{}' \u{2014} {}", acp.name, acp.summary));
        }
    }

    lines.join("\n")
}

fn render_connector_preview(
    categories: &[String],
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    let mut lines = Vec::new();
    let mut connector_names: Vec<String> = Vec::new();

    for category in categories {
        for entry in BUILTIN_CONNECTORS {
            let cat = entry.category.strip_prefix("connector/").unwrap_or(entry.category);
            if cat == category && !connector_names.iter().any(|name| name == entry.name) {
                connector_names.push(entry.name.to_string());
            }
        }
    }

    for connector_name in connector_names {
        if let Some(entry) = BUILTIN_CONNECTORS.iter().find(|entry| entry.name == connector_name) {
            let installed_status =
                if installed.iter().any(|name| name == entry.name) { "installed" } else { "available" };
            lines.push(format!(
                "  - {}: category={} status={} summary={} operations={}",
                entry.name,
                entry.category,
                installed_status,
                entry.summary,
                entry.operations.join("; "),
            ));
        } else if let Some(connector) = tenant_connectors.iter().find(|connector| connector.name == connector_name) {
            let operations =
                connector.endpoints.iter().map(|endpoint| endpoint.path.as_str()).take(6).collect::<Vec<_>>();
            let operation_text =
                if operations.is_empty() { "custom endpoints configured".to_string() } else { operations.join(", ") };
            lines.push(format!(
                "  - {}: category={} summary={} endpoints={}",
                connector.name, connector.category, connector.summary, operation_text,
            ));
        }
    }

    lines.join("\n")
}

fn primary_tool_categories(intent: &serde_json::Value) -> Vec<String> {
    let mut categories: Vec<String> = intent["preferred_tool_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if categories.is_empty() {
        categories.push("data".into());
        categories.push("web".into());
    }
    categories.sort();
    categories.dedup();
    categories
}

fn secondary_tool_categories(intent: &serde_json::Value, primary: &[String]) -> Vec<String> {
    let mut categories = Vec::new();

    if let Some(output_hint) = intent["output_hint"].as_str() {
        match output_hint {
            "email_draft" | "email_send" | "notification" => categories.push("notification".into()),
            "report" | "workspace" => categories.push("storage".into()),
            "connector_record" => categories.push("connector".into()),
            _ => {}
        }
    }

    if categories.is_empty() {
        categories.push("web".into());
        categories.push("connector".into());
    }

    categories.retain(|cat| !primary.iter().any(|p| p == cat));
    categories.sort();
    categories.dedup();
    categories
}

fn fallback_tool_categories(primary: &[String], secondary: &[String]) -> Vec<String> {
    let mut categories = vec!["storage".into(), "code".into(), "notification".into()];
    categories.retain(|cat| !primary.iter().any(|p| p == cat) && !secondary.iter().any(|s| s == cat));
    categories
}

fn connector_categories_from_intent(intent: &serde_json::Value) -> Vec<String> {
    let mut categories: Vec<String> = intent["needed_connector_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    if categories.is_empty() {
        categories.extend(
            intent["candidate_connectors"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|name| {
                            BUILTIN_CONNECTORS
                                .iter()
                                .find(|entry| entry.name == name)
                                .map(|entry| entry.category.strip_prefix("connector/").unwrap_or(entry.category).to_string())
                                .unwrap_or_else(|| "connector".to_string())
                        })
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default(),
        );
    }

    categories.sort();
    categories.dedup();
    categories
}

fn tool_family_label(family: &ToolFamily) -> &'static str {
    match family {
        ToolFamily::Web => "web",
        ToolFamily::Database => "database",
        ToolFamily::Transform => "transform",
        ToolFamily::Connector => "connector",
        ToolFamily::Notification => "notification",
        ToolFamily::Storage => "storage",
        ToolFamily::Memory => "memory",
        ToolFamily::Code => "code",
        ToolFamily::Scheduling => "scheduling",
        ToolFamily::Security => "security",
        ToolFamily::Meta => "meta",
    }
}

fn dsl_step_type_label(step: &DslStepType) -> &'static str {
    match step {
        DslStepType::FetchRecords => "fetch_records",
        DslStepType::Filter => "filter",
        DslStepType::Compute => "compute",
        DslStepType::Aggregate => "aggregate",
        DslStepType::DetectAnomaly => "detect_anomaly",
        DslStepType::Branch => "branch",
        DslStepType::Notify => "notify",
        DslStepType::StoreResult => "store_result",
    }
}

fn resource_kind_label(kind: Option<&ResourceKind>) -> Option<&'static str> {
    kind.map(|kind| match kind {
        ResourceKind::Database => "database",
        ResourceKind::HttpEndpoint => "http_endpoint",
        ResourceKind::Connector => "connector",
        ResourceKind::AcpPeer => "acp_peer",
        ResourceKind::FileSystem => "filesystem",
        ResourceKind::ApiKey => "api_key",
        ResourceKind::SshHost => "ssh_host",
        ResourceKind::DockerDaemon => "docker_daemon",
        ResourceKind::KubeCluster => "kube_cluster",
        ResourceKind::McpServer => "mcp_server",
    })
}

fn registry_entry_for_tool(name: &str) -> Option<&'static ToolRegistryEntry> {
    TOOL_REGISTRY.iter().find(|entry| entry.name == name)
}

fn registry_tool_candidate(tool_name: &str, registry: &ToolRegistry) -> Option<serde_json::Value> {
    let spec = registry.get(tool_name)?;
    let entry = registry_entry_for_tool(tool_name);
    let contract = crate::tools::render_tool_contract(spec.as_ref());
    let parameters = crate::tools::parameters_schema_to_json(&spec.parameters_schema());
    let output_schema = spec.output_schema();

    Some(serde_json::json!({
        "name": spec.name(),
        "description": spec.description(),
        "family": entry.map(|entry| tool_family_label(&entry.family)).unwrap_or("unknown"),
        "dsl_step_types": entry
            .map(|entry| entry.dsl_step_types.iter().map(dsl_step_type_label).collect::<Vec<_>>())
            .unwrap_or_default(),
        "operations": entry.map(|entry| entry.operations.iter().copied().collect::<Vec<_>>()).unwrap_or_default(),
        "requires_resource": entry.and_then(|entry| resource_kind_label(entry.requires_resource.as_ref())),
        "read_only": entry.map(|entry| entry.read_only).unwrap_or(false),
        "requires_approval": entry.map(|entry| entry.requires_approval).unwrap_or(false),
        "priority": entry.map(|entry| entry.priority).unwrap_or_default(),
        "parameters": parameters,
        "output_schema": output_schema,
        "contract": contract,
    }))
}

fn registry_connector_candidate(
    connector_name: &str,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> Option<serde_json::Value> {
    if let Some(entry) = BUILTIN_CONNECTORS.iter().find(|entry| entry.name == connector_name) {
        return Some(serde_json::json!({
            "name": entry.name,
            "category": entry.category,
            "status": if installed.iter().any(|name| name.as_str() == entry.name) { "installed" } else { "available" },
            "summary": entry.summary,
            "operations": entry.operations,
        }));
    }

    tenant_connectors.iter().find(|connector| connector.name == connector_name).map(|connector| {
        let operations =
            connector.endpoints.iter().map(|endpoint| endpoint.path.as_str()).take(6).collect::<Vec<_>>();
        serde_json::json!({
            "name": connector.name,
            "category": connector.category,
            "status": if installed.iter().any(|name| name.as_str() == connector.name) { "installed" } else { "available" },
            "summary": connector.summary,
            "operations": operations,
        })
    })
}

fn candidate_tool_names(categories: &[String], registry: &ToolRegistry) -> Vec<String> {
    let by_category = registry.by_category();
    let mut names = Vec::new();
    for category in categories {
        if let Some(category_names) = by_category.get(category.as_str()) {
            for name in category_names {
                let name = (*name).to_string();
                if name.starts_with("request_more_")
                    || name == "list_connectors_in_category"
                    || name == "create_workspace_tool"
                {
                    continue;
                }
                if !names.iter().any(|existing| existing == &name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

fn integration_protocol_label(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "mcp_session" | "search_mcp_registry" => Some("mcp"),
        "acp_session" => Some("acp"),
        "api_call" | "register_api_tool" => Some("rest"),
        _ => None,
    }
}

fn integration_sub_operations(tool_name: &str) -> &'static [&'static str] {
    match tool_name {
        "mcp_session" => &["connect", "list_tools", "list_resources", "read_resource", "call_tool"],
        "search_mcp_registry" => &["search", "discover", "suggest_connectors"],
        "acp_session" => &["list_agents", "receive_messages", "send_message"],
        "api_call" => &["get", "post", "put", "patch", "delete"],
        "register_api_tool" => &["register"],
        _ => &[],
    }
}

fn intent_mentions_internal_agent_workflow(blob: &str) -> bool {
    blob.contains("acp")
        || blob.contains("agent-to-agent")
        || blob.contains("agent to agent")
        || blob.contains("internal agent")
        || blob.contains("internal agents")
        || blob.contains("child agent")
        || blob.contains("parent agent")
        || blob.contains("teammate agent")
        || blob.contains("peer")
        || blob.contains("send message")
        || blob.contains("receive messages")
        || blob.contains("message another agent")
}

fn integration_candidate_names(intent: &serde_json::Value) -> Vec<&'static str> {
    let mut names = vec!["mcp_session", "search_mcp_registry", "acp_session"];

    let blob = serde_json::to_string(intent).unwrap_or_default().to_lowercase();
    let wants_rest = blob.contains("api") || blob.contains("rest") || blob.contains("http");
    let wants_mcp = blob.contains("mcp") || blob.contains("connector") || blob.contains("server");
    let wants_acp = intent_mentions_internal_agent_workflow(&blob);

    if wants_rest {
        names.extend(["api_call", "register_api_tool"]);
    }
    if !wants_mcp {
        names.retain(|name| *name != "search_mcp_registry");
    }
    if !wants_acp {
        names.retain(|name| *name != "acp_session");
    }

    names.sort();
    names.dedup();
    names
}

fn integration_candidate(tool_name: &str, registry: &ToolRegistry) -> Option<serde_json::Value> {
    let spec = registry.get(tool_name)?;
    let entry = registry_entry_for_tool(tool_name);
    let contract = crate::tools::render_tool_contract(spec.as_ref());

    Some(serde_json::json!({
        "name": spec.name(),
        "description": spec.description(),
        "protocol": integration_protocol_label(tool_name),
        "family": entry.map(|entry| tool_family_label(&entry.family)).unwrap_or("unknown"),
        "operations": entry.map(|entry| entry.operations.iter().copied().collect::<Vec<_>>()).unwrap_or_default(),
        "sub_operations": integration_sub_operations(tool_name),
        "requires_resource": entry.and_then(|entry| resource_kind_label(entry.requires_resource.as_ref())),
        "read_only": entry.map(|entry| entry.read_only).unwrap_or(false),
        "requires_approval": entry.map(|entry| entry.requires_approval).unwrap_or(false),
        "priority": entry.map(|entry| entry.priority).unwrap_or_default(),
        "parameters": crate::tools::parameters_schema_to_json(&spec.parameters_schema()),
        "output_schema": spec.output_schema(),
        "contract": contract,
    }))
}

fn render_candidate_slice(
    title: &str,
    tool_categories: &[String],
    connector_categories: &[String],
    registry: &ToolRegistry,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    let mut lines = vec![format!("{}:", title)];

    if tool_categories.is_empty() {
        lines.push("  Tools: none".into());
    } else {
        lines.push("  Tools:".into());
        for category in tool_categories {
            let preview = render_tool_preview(registry, category);
            if preview.is_empty() {
                continue;
            }
            for line in preview.lines() {
                lines.push(format!("    {}", line));
            }
        }
    }

    if connector_categories.is_empty() {
        lines.push("  Connectors: none".into());
    } else {
        lines.push("  Connectors:".into());
        let preview = render_connector_preview(connector_categories, installed, tenant_connectors);
        if preview.is_empty() {
            lines.push("    none".into());
        } else {
            for line in preview.lines() {
                lines.push(format!("    {}", line));
            }
        }
    }

    lines.join("\n")
}

// ── Public registry API ─────────────────────────────────────────────────

/// Build a human-readable summary of the tenant's custom connections.
pub fn build_custom_context(_installed: &[String], tenant_connectors: &[TenantConnector]) -> String {
    if tenant_connectors.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();

    let dbs: Vec<&TenantConnector> =
        tenant_connectors.iter().filter(|tc| tc.category == "connector/database").collect();
    if !dbs.is_empty() {
        lines.push("Databases (use external_db tool, reference by name):".into());
        for db in &dbs {
            lines.push(format!("  - name='{}' \u{2014} {}", db.name, db.summary));
        }
    }

    let apis: Vec<&TenantConnector> = tenant_connectors
        .iter()
        .filter(|tc| !tc.category.contains("database") && !tc.category.contains("mcp"))
        .collect();
    if !apis.is_empty() {
        lines.push("Custom REST APIs (use external_api tool, reference by name):".into());
        for api in &apis {
            lines.push(format!("  - name='{}' \u{2014} {}", api.name, api.summary));
        }
    }

    let mcps: Vec<&TenantConnector> = tenant_connectors.iter().filter(|tc| tc.category.contains("mcp")).collect();
    if !mcps.is_empty() {
        lines.push("MCP servers (available as connector tools):".into());
        for mcp in &mcps {
            lines.push(format!("  - name='{}' \u{2014} {}", mcp.name, mcp.summary));
        }
    }

    lines.join("\n")
}

pub fn build_capability_directory(
    registry: &ToolRegistry,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    let mut lines: Vec<String> = vec![
        "Use categories first. Do not assume every connector is installed or every tool is needed.".into(),
        "Only installed connectors and registered custom connections are immediately usable.".into(),
        "If no installed connector fits, prefer missing_capabilities such as custom_db, custom_api, or connector/<category>.".into(),
        "Tool category quick map 1: filesystem=shell,file_read,file_write,file_edit,glob_search,content_search; web=web_search_tool,web_fetch,http_request,browser,browser_interact,browser_pdf".into(),
        "Tool category quick map 2: code=code_run,diff,patch,git_operations,sql_query; data=data_engine,data_extractor,pdf_read,pdf_create,spreadsheet_read,spreadsheet_write,image_process,image_info".into(),
        "Tool category quick map 3: memory=memory_store,memory_recall,memory_forget,vector_store,vector_search,vector_delete; infra=docker,kubernetes,ssh_exec,process_monitor".into(),
        "Tool category quick map 4: integration=mcp_session,search_mcp_registry,acp_session,api_call,register_api_tool; communication=email,notification,pushover,ask_user; security=crypto_tool,plane_guard,request_credential; automation=schedule,cron_add,cron_list,cron_remove,cron_run,delegate".into(),
    ];

    let mut tool_categories: Vec<(String, Vec<String>)> = registry
        .by_category()
        .into_iter()
        .filter(|(category, _)| !category.starts_with("connector/"))
        .map(|(category, names)| {
            (
                category.to_string(),
                names
                    .into_iter()
                    .filter(|name| {
                        !name.starts_with("request_more_")
                            && *name != "list_connectors_in_category"
                            && *name != "create_workspace_tool"
                    })
                    .take(4)
                    .map(String::from)
                    .collect::<Vec<String>>(),
            )
        })
        .filter(|(_, names)| !names.is_empty())
        .collect();
    tool_categories.sort_by(|a, b| a.0.cmp(&b.0));
    lines.push("Core tool categories (examples only, more detail comes later if relevant):".into());
    for (category, names) in tool_categories {
        lines.push(format!("  - {}: {}", category, names.join(", ")));
    }

    let mut connector_groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for entry in BUILTIN_CONNECTORS {
        let cat = entry.category.strip_prefix("connector/").unwrap_or(entry.category);
        let status = if installed.iter().any(|name| name == entry.name) { "installed" } else { "available" };
        connector_groups.entry(cat).or_default().push(format!("{} ({}, {})", entry.name, status, entry.summary));
    }
    lines.push("Built-in connector categories:".into());
    for (category, connectors) in connector_groups {
        let preview = connectors.into_iter().take(4).collect::<Vec<_>>();
        lines.push(format!("  - {}: {}", category, preview.join("; ")));
    }

    let custom_context = build_custom_context(installed, tenant_connectors);
    if !custom_context.is_empty() {
        lines.push("Tenant custom connections:".into());
        lines.push(custom_context);
    }

    lines.push(
        "Deterministic data workflows should use data_engine. If a workflow needs arbitrary code or a future sandbox runtime, mark that as a missing capability instead of inventing a runtime tool."
            .into(),
    );

    lines.join("\n")
}

pub fn build_detailed_capability_context(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    let primary_categories = primary_tool_categories(intent);
    let secondary_categories = secondary_tool_categories(intent, &primary_categories);
    let fallback_categories = fallback_tool_categories(&primary_categories, &secondary_categories);
    let connector_categories = connector_categories_from_intent(intent);

    let mut slices = Vec::new();
    slices.push(render_candidate_slice(
        "PRIMARY CANDIDATE SLICE",
        &primary_categories,
        &connector_categories,
        registry,
        installed,
        tenant_connectors,
    ));
    slices.push(render_candidate_slice(
        "SECONDARY CANDIDATE SLICE",
        &secondary_categories,
        &connector_categories,
        registry,
        installed,
        tenant_connectors,
    ));
    slices.push(render_candidate_slice(
        "FALLBACK CANDIDATE SLICE",
        &fallback_categories,
        &connector_categories,
        registry,
        installed,
        tenant_connectors,
    ));

    slices.join("\n\n")
}

pub fn build_registry_candidate_set(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> serde_json::Value {
    let primary_categories = primary_tool_categories(intent);
    let secondary_categories = secondary_tool_categories(intent, &primary_categories);
    let fallback_categories = fallback_tool_categories(&primary_categories, &secondary_categories);
    let connector_categories = connector_categories_from_intent(intent);

    let primary_tools = candidate_tool_names(&primary_categories, registry)
        .into_iter()
        .filter_map(|name| registry_tool_candidate(&name, registry))
        .collect::<Vec<_>>();
    let secondary_tools = candidate_tool_names(&secondary_categories, registry)
        .into_iter()
        .filter_map(|name| registry_tool_candidate(&name, registry))
        .collect::<Vec<_>>();
    let fallback_tools = candidate_tool_names(&fallback_categories, registry)
        .into_iter()
        .filter_map(|name| registry_tool_candidate(&name, registry))
        .collect::<Vec<_>>();

    let builtin_connector_names: Vec<String> = connector_categories
        .iter()
        .flat_map(|category| {
            BUILTIN_CONNECTORS.iter().filter_map(move |entry| {
                let cat = entry.category.strip_prefix("connector/").unwrap_or(entry.category);
                if cat == category {
                    Some(entry.name.to_string())
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();

    let tenant_connector_names: Vec<String> = connector_categories
        .iter()
        .flat_map(|category| {
            tenant_connectors.iter().filter_map(move |connector| {
                let cat = connector.category.strip_prefix("connector/").unwrap_or(connector.category.as_str());
                if cat == category {
                    Some(connector.name.clone())
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();

    let mut connector_names = builtin_connector_names;
    connector_names.extend(tenant_connector_names);
    connector_names.sort();
    connector_names.dedup();

    let primary_connectors = connector_names
        .iter()
        .filter_map(|name| registry_connector_candidate(name, installed, tenant_connectors))
        .collect::<Vec<_>>();

    let integration_candidates = integration_candidate_names(intent)
        .into_iter()
        .filter_map(|name| integration_candidate(name, registry))
        .collect::<Vec<_>>();

    serde_json::json!({
        "version": 1,
        "intent": {
            "preferred_tool_categories": primary_categories,
            "secondary_tool_categories": secondary_categories,
            "fallback_tool_categories": fallback_categories,
            "connector_categories": connector_categories,
        },
        "slices": [
            {
                "name": "primary",
                "tools": primary_tools,
                "connectors": primary_connectors,
            },
            {
                "name": "secondary",
                "tools": secondary_tools,
                "connectors": [],
            },
            {
                "name": "fallback",
                "tools": fallback_tools,
                "connectors": [],
            }
        ],
        "integrations": integration_candidates,
        "rules": [
            "Choose the most specific matching slice first.",
            "Select one exact tool and one exact operation from the selected slice.",
            "Use the registry metadata for operations, resource requirements, read_only, and approval policy.",
            "For integration tools, treat sub_operations as the protocol-level actions that sit underneath the tool operation.",
            "If no candidate matches, preserve missing_capabilities instead of inventing a tool.",
            "Same tool may appear multiple times in the workflow when the explicit DSL allows it."
        ]
    })
}

pub fn build_registry_candidate_context(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    let structured = build_registry_candidate_set(registry, intent, installed, tenant_connectors);
    let json = serde_json::to_string(&structured).unwrap_or_else(|_| structured.to_string());

    format!("REGISTRY CANDIDATE SET JSON:\n{}", json)
}

pub fn inferred_preferred_tools(registry: &ToolRegistry, intent: &serde_json::Value) -> Vec<String> {
    intent["preferred_tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str())
                .filter(|tool_name| registry.get(tool_name).is_some())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub fn missing_tool_categories(intent: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = intent["missing_capabilities"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str())
                .filter_map(|value| value.strip_prefix("tool/"))
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

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
        capability_directory: build_capability_directory(registry, installed, tenant_connectors),
        registry_candidate_context: build_registry_candidate_context(registry, intent, installed, tenant_connectors),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_registry_candidate_set_has_three_slices() {
        let registry = crate::tools::default_registry();
        let intent = serde_json::json!({
            "notes": "mcp and acp integration workflow"
        });
        let candidate_set = build_registry_candidate_set(&registry, &intent, &[], &[]);

        assert_eq!(candidate_set["version"], 1);
        assert!(candidate_set["slices"].as_array().is_some());
        assert_eq!(candidate_set["slices"].as_array().map(|arr| arr.len()), Some(3));
        assert!(candidate_set["rules"].as_array().map(|arr| !arr.is_empty()).unwrap_or(false));
        assert!(candidate_set["integrations"].as_array().map(|arr| !arr.is_empty()).unwrap_or(false));
        let integrations = candidate_set["integrations"].as_array().cloned().unwrap_or_default();
        assert!(integrations.iter().any(|entry| entry["name"] == "mcp_session"));
        assert!(integrations.iter().any(|entry| entry["name"] == "acp_session"));
        assert!(integrations
            .iter()
            .any(|entry| {
                entry["name"] == "acp_session"
                    && entry["sub_operations"]
                        .as_array()
                        .map(|ops| ops.iter().any(|op| op.as_str() == Some("receive_messages")))
                        .unwrap_or(false)
            }));
    }
}
