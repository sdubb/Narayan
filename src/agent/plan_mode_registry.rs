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
            lines.push(format!("  - name='{}' Ã¢â‚¬â€ {}", acp.name, acp.summary));
        }
    }

    let acps: Vec<&TenantConnector> =
        tenant_connectors.iter().filter(|tc| tc.category.contains("acp") || tc.category.contains("agent")).collect();
    if !acps.is_empty() {
        lines.push("ACP peers (internal agent-to-agent connections):".into());
        for acp in &acps {
            lines.push(format!("  - name='{}' Ã¢â‚¬â€ {}", acp.name, acp.summary));
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
            lines.push(format!("  - name='{}' â€” {}", db.name, db.summary));
        }
    }

    let apis: Vec<&TenantConnector> = tenant_connectors
        .iter()
        .filter(|tc| !tc.category.contains("database") && !tc.category.contains("mcp"))
        .collect();
    if !apis.is_empty() {
        lines.push("Custom REST APIs (use external_api tool, reference by name):".into());
        for api in &apis {
            lines.push(format!("  - name='{}' â€” {}", api.name, api.summary));
        }
    }

    let mcps: Vec<&TenantConnector> = tenant_connectors.iter().filter(|tc| tc.category.contains("mcp")).collect();
    if !mcps.is_empty() {
        lines.push("MCP servers (available as connector tools):".into());
        for mcp in &mcps {
            lines.push(format!("  - name='{}' â€” {}", mcp.name, mcp.summary));
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
    let json = serde_json::to_string_pretty(&structured).unwrap_or_else(|_| structured.to_string());

    format!(
        "{}\n\nREGISTRY CANDIDATE SET JSON:\n{}",
        build_detailed_capability_context(registry, intent, installed, tenant_connectors),
        json
    )
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
