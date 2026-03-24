//! Tool selector — pick the right subset of tools for each step.
//!
//! Sending all 65 tools to the LLM on every call is wasteful and harmful:
//!   - wastes ~5,000 tokens per call on tool schemas
//!   - degrades model decision quality (too many choices = worse selection)
//!   - inflates cost proportionally
//!
//! Strategy (applied in order, stops when enough tools found):
//!   1. Always include: the tool named in the plan step (planner hint)
//!   2. Always include: a small core set every agent needs
//!   3. Job-type preferred tools (from JobType::preferred_tools)
//!   4. Keyword match: step description words against tool names + descriptions
//!   5. Cap at MAX_TOOLS (20) — drop lowest-relevance tools if over cap
//!
//! Result: 8-18 highly relevant tools per step instead of 65.

use crate::{
    agent::{planner::PlannedStep, prompts::JobType},
    providers::ToolSpec,
    tools::ToolRegistry,
};

/// Maximum tools sent to any single LLM call.
pub const MAX_TOOLS: usize = 20;
const MAX_ROLE_CATEGORY_TOOLS: usize = 4;
const RUNTIME_BLOCKED_TOOLS: &[&str] = &["create_workspace_tool"];

/// Tools always included regardless of job type or step.
/// Every agent needs these for basic operation.
const ALWAYS_INCLUDE: &[&str] = &[
    "shell",
    "file_read",
    "file_write",
    "memory_recall",
    "memory_store",
    "ask_user",
    "delegate",
    "plane_guard",
    "vector_search",
    "list_connectors_in_category",
    "request_more_connectors",
    "create_custom_connector",
    "request_more_tools",
];

/// Select the best tool subset for a given step.
pub fn select_tools_for_step(
    registry: &ToolRegistry,
    step: &PlannedStep,
    job_type: &JobType,
    role_tools: &[String],
    role_tool_categories: &[String],
) -> Vec<ToolSpec> {
    let mut selected: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let add = |name: &str, selected: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if RUNTIME_BLOCKED_TOOLS.contains(&name) {
            return;
        }
        if seen.insert(name.to_string()) && registry.get(name).is_some() {
            selected.push(name.to_string());
        }
    };

    // ── 1. Planner hint — always include the tool the planner specified ──────
    if let Some(ref tool_name) = step.tool {
        add(tool_name, &mut selected, &mut seen);
    }

    // ── 2. Core always-available tools ────────────────────────────────────────
    for name in ALWAYS_INCLUDE {
        add(name, &mut selected, &mut seen);
    }

    // ── 3. Role-scoped tool preferences ───────────────────────────────────────
    for name in role_tools {
        if selected.len() >= MAX_TOOLS {
            break;
        }
        add(name, &mut selected, &mut seen);
    }

    // ── 4. Job-type preferred tools ───────────────────────────────────────────
    let tools_by_category = registry.by_category();
    for category in role_tool_categories {
        if selected.len() >= MAX_TOOLS {
            break;
        }
        if let Some(names) = tools_by_category.get(category.as_str()) {
            for name in names.iter().take(MAX_ROLE_CATEGORY_TOOLS) {
                if selected.len() >= MAX_TOOLS {
                    break;
                }
                add(name, &mut selected, &mut seen);
            }
        }
    }

    for name in job_type.preferred_tools() {
        if selected.len() >= MAX_TOOLS {
            break;
        }
        add(name, &mut selected, &mut seen);
    }

    // ── 5. Keyword relevance matching ─────────────────────────────────────────
    // Extract meaningful words from the step description
    let step_words = extract_keywords(&step.description);

    if selected.len() < MAX_TOOLS {
        // Score every tool by keyword overlap
        let mut scored: Vec<(usize, String)> = registry
            .list()
            .into_iter()
            .filter(|name| !seen.contains(*name))
            .filter_map(|name| {
                let tool = registry.get(name)?;
                let score = keyword_score(&step_words, tool.name(), tool.description());
                if score > 0 {
                    Some((score, name.to_string()))
                } else {
                    None
                }
            })
            .collect();

        // Highest score first
        scored.sort_by(|a, b| b.0.cmp(&a.0));

        for (_, name) in scored {
            if selected.len() >= MAX_TOOLS {
                break;
            }
            add(&name, &mut selected, &mut seen);
        }
    }

    // ── 6. Build ToolSpec list ────────────────────────────────────────────────
    selected
        .into_iter()
        .filter_map(|name| registry.get(&name))
        .map(|t| ToolSpec {
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

/// Extract meaningful keywords from a step description.
/// Strips stop words, lowercases, deduplicates.
pub(crate) fn extract_keywords(text: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by", "from", "as", "is",
        "it", "its", "this", "that", "be", "do", "have", "has", "had", "will", "would", "could", "should", "may",
        "might", "then", "after", "before", "step", "now", "next", "using", "use", "the", "create", "get", "set",
        "run", "make", "find", "read", "write", "check", "all", "any", "each", "every", "into", "out", "new", "old",
        "current",
    ];
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(String::from)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect()
}

/// Score a tool by keyword overlap with the step description.
pub(crate) fn keyword_score(step_words: &[String], tool_name: &str, tool_desc: &str) -> usize {
    let haystack = format!("{} {}", tool_name, tool_desc).to_lowercase();
    step_words.iter()
        .filter(|w| haystack.contains(w.as_str()))
        .count()
        // Bonus: exact tool name match in step description
        + if step_words.iter().any(|w| tool_name.contains(w.as_str())) { 3 } else { 0 }
}

/// Return a grouped summary of ALL tools for the planner and preflight.
/// Doesn't send full schemas — just names grouped by category.
/// Keeps the planner informed without overwhelming it.
pub fn tool_manifest(registry: &ToolRegistry) -> String {
    let groups: &[(&str, &[&str])] = &[
        (
            "filesystem",
            &[
                "shell",
                "file_read",
                "file_write",
                "file_edit",
                "glob_search",
                "content_search",
                "compress",
                "decompress",
            ],
        ),
        (
            "web",
            &[
                "web_search_tool",
                "web_fetch",
                "http_request",
                "browser",
                "browser_interact",
                "browser_pdf",
                "browser_network",
                "screenshot",
            ],
        ),
        (
            "code",
            &[
                "code_run",
                "wasm_exec",
                "wasm_compile",
                "wasm_inspect",
                "wasm_call",
                "run_registered_wasm",
                "diff",
                "patch",
                "git_operations",
                "sql_query",
            ],
        ),
        (
            "data",
            &[
                "data_extractor",
                "pdf_read",
                "pdf_create",
                "spreadsheet_read",
                "spreadsheet_write",
                "image_process",
                "image_info",
            ],
        ),
        (
            "memory",
            &["memory_store", "memory_recall", "memory_forget", "vector_store", "vector_search", "vector_delete"],
        ),
        ("infra", &["docker", "kubernetes", "ssh_exec", "process_monitor"]),
        (
            "integration",
            &["mcp_session", "search_mcp_registry", "acp_session", "api_call", "http_request", "register_api_tool"],
        ),
        ("communication", &["email", "notification", "pushover", "ask_user"]),
        ("security", &["crypto_tool", "plane_guard", "request_credential"]),
        ("automation", &["schedule", "cron_add", "cron_list", "cron_remove", "cron_run", "delegate"]),
    ];

    let mut lines = vec!["Available tool categories (use exact names in tool_args):".to_string()];
    for (group, names) in groups {
        let available: Vec<&str> = names.iter().filter(|n| registry.get(n).is_some()).copied().collect();
        if !available.is_empty() {
            lines.push(format!("  {}: {}", group, available.join(", ")));
        }
    }

    // Add any tools not in the known groups
    let all_known: std::collections::HashSet<&str> =
        groups.iter().flat_map(|(_, names)| names.iter().copied()).collect();
    let extras: Vec<&str> = registry.list().into_iter().filter(|n| !all_known.contains(n)).collect();
    if !extras.is_empty() {
        lines.push(format!("  other: {}", extras.join(", ")));
    }

    lines.join("\n")
}

/// Build a grouped manifest from a slice of tool names (for preflight + planner).
/// Same grouping as tool_manifest but works from a &[&str] instead of registry.
pub fn tool_manifest_from_names(names: &[&str]) -> String {
    let set: std::collections::HashSet<&&str> = names.iter().collect();
    let groups: &[(&str, &[&str])] = &[
        (
            "filesystem",
            &[
                "shell",
                "file_read",
                "file_write",
                "file_edit",
                "glob_search",
                "content_search",
                "compress",
                "decompress",
            ],
        ),
        (
            "web",
            &[
                "web_search_tool",
                "web_fetch",
                "http_request",
                "browser",
                "browser_interact",
                "browser_pdf",
                "browser_network",
                "screenshot",
            ],
        ),
        (
            "code",
            &[
                "code_run",
                "wasm_exec",
                "wasm_compile",
                "wasm_inspect",
                "wasm_call",
                "run_registered_wasm",
                "diff",
                "patch",
                "git_operations",
                "sql_query",
            ],
        ),
        (
            "data",
            &[
                "data_extractor",
                "pdf_read",
                "pdf_create",
                "spreadsheet_read",
                "spreadsheet_write",
                "image_process",
                "image_info",
            ],
        ),
        (
            "memory",
            &["memory_store", "memory_recall", "memory_forget", "vector_store", "vector_search", "vector_delete"],
        ),
        ("infra", &["docker", "kubernetes", "ssh_exec", "process_monitor"]),
        ("integration", &["mcp_session", "search_mcp_registry", "api_call", "http_request", "register_api_tool"]),
        ("communication", &["email", "notification", "pushover", "ask_user"]),
        ("security", &["crypto_tool", "plane_guard", "request_credential"]),
        ("automation", &["schedule", "cron_add", "cron_list", "cron_remove", "delegate"]),
    ];

    let all_known: std::collections::HashSet<&str> = groups.iter().flat_map(|(_, ns)| ns.iter().copied()).collect();

    let mut lines = vec!["Available tool categories:".to_string()];
    for (group, group_names) in groups {
        let available: Vec<&str> = group_names.iter().filter(|n| set.contains(n)).copied().collect();
        if !available.is_empty() {
            lines.push(format!("  {}: {}", group, available.join(", ")));
        }
    }
    let extras: Vec<&&str> = names.iter().filter(|n| !all_known.contains(**n)).collect();
    if !extras.is_empty() {
        lines.push(format!("  other: {}", extras.iter().map(|n| **n).collect::<Vec<_>>().join(", ")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::{planner::PlannedStep, prompts::JobType},
        tools::default_registry,
    };

    #[test]
    fn test_always_include_present() {
        let registry = default_registry();
        let step = PlannedStep {
            index: 0,
            description: "do something generic".into(),
            tool: None,
            tool_args: None,
            success_criteria: String::new(),
            condition: None,
        };
        let job_type = JobType::detect("build a web app");
        let specs = select_tools_for_step(&registry, &step, &job_type, &[], &[]);
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"shell"), "expected 'shell' in selected tools");
        assert!(names.contains(&"file_read"), "expected 'file_read' in selected tools");
    }

    #[test]
    fn test_planner_hint_included() {
        let registry = default_registry();
        let step = PlannedStep {
            index: 0,
            description: "encrypt data".into(),
            tool: Some("crypto_tool".into()),
            tool_args: None,
            success_criteria: String::new(),
            condition: None,
        };
        let job_type = JobType::detect("encrypt some data");
        let specs = select_tools_for_step(&registry, &step, &job_type, &[], &[]);
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"crypto_tool"), "expected 'crypto_tool' in selected tools");
    }

    #[test]
    fn test_max_tools_cap() {
        let registry = default_registry();
        let step = PlannedStep {
            index: 0,
            description:
                "do everything with docker kubernetes ssh web browser file compression image pdf crypto spreadsheet"
                    .into(),
            tool: None,
            tool_args: None,
            success_criteria: String::new(),
            condition: None,
        };
        let job_type = JobType::detect("build a web app");
        let specs = select_tools_for_step(&registry, &step, &job_type, &[], &[]);
        assert!(specs.len() <= MAX_TOOLS, "expected <= {} tools, got {}", MAX_TOOLS, specs.len());
    }

    #[test]
    fn test_role_scoped_tools_are_honored() {
        let registry = default_registry();
        let step = PlannedStep {
            index: 0,
            description: "fetch a page and summarize it".into(),
            tool: None,
            tool_args: None,
            success_criteria: String::new(),
            condition: None,
        };
        let job_type = JobType::General;
        let specs = select_tools_for_step(&registry, &step, &job_type, &["web_fetch".into()], &[]);
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"web_fetch"), "expected role-scoped tool 'web_fetch' in selected tools");
    }

    #[test]
    fn test_role_tool_categories_expand_toolset() {
        let registry = default_registry();
        let step = PlannedStep {
            index: 0,
            description: "inspect an api response".into(),
            tool: None,
            tool_args: None,
            success_criteria: String::new(),
            condition: None,
        };
        let specs = select_tools_for_step(&registry, &step, &JobType::General, &[], &["integration".into()]);
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"api_call") || names.contains(&"http_request"));
    }

    #[test]
    fn test_role_tool_category_respects_per_category_cap() {
        let registry = default_registry();
        let step = PlannedStep {
            index: 0,
            description: "handle integrations broadly".into(),
            tool: None,
            tool_args: None,
            success_criteria: String::new(),
            condition: None,
        };
        let specs = select_tools_for_step(&registry, &step, &JobType::General, &[], &["integration".into()]);
        let integration_count = specs
            .iter()
            .filter(|spec| registry.get(&spec.name).map(|tool| tool.category() == "integration").unwrap_or(false))
            .count();
        assert!(integration_count <= MAX_ROLE_CATEGORY_TOOLS);
    }

    #[test]
    fn test_keyword_scoring() {
        let words = vec!["docker".to_string(), "container".to_string()];
        let score = keyword_score(&words, "docker", "manage docker containers");
        assert!(score > 0, "expected positive score for docker/container keywords");
    }

    #[test]
    fn test_extract_keywords_filters_stopwords() {
        let keywords = extract_keywords("the quick brown fox");
        assert!(!keywords.contains(&"the".to_string()), "'the' should be filtered as a stop word");
    }
}
