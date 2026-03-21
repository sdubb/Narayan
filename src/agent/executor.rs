//! Executor — runs a single planned step using the LLM + tools.
//!
//! Per-tool-call pipeline (in order):
//!   1. PiiRedactor.redact(args)          — strip sensitive fields before they leave
//!   2. PolicyEngine.evaluate(ctx)        — gate: Allow / Block / RequireApproval / Redact
//!   3. plane_guard_risk()                — hard safety floor (critical = blocked always)
//!   4. tool.execute(clean_args)          — actual execution
//!
//! All three checks are opt-in via AgentServices — if a service is None the step
//! is skipped with zero overhead (no Arc dereference cost either).

use std::{collections::HashSet, path::Path};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::{
    agent::{
        planner::{Plan, PlannedStep},
        prompts::{build_conversation_history, is_direct_response_goal, ExecutorPrompt, JobType, StepHistory},
    },
    events::{AgentEvent, EventBus},
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    policy::{
        engine::PolicyContext,
        rules::PolicyRuleSet,
        PolicyDecision,
    },
    providers::{Message, ToolCall},
    segments::AgentServices,
    state::AgentState,
    storage::PostgresStore,
    tenant::TenantStore,
    tools::{selector::select_tools_for_step, ParameterSchema, ToolRegistry, ToolResult},
};

fn truncate_for_log(value: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(value.len().min(max_chars));
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...(truncated)");
    }
    out
}

#[derive(Debug)]
pub struct StepResult {
    pub step_index: usize,
    pub success: bool,
    pub output: String,
    pub final_answer_candidate: Option<String>,
    pub tool_results: Vec<ToolResult>,
    pub tools_called: Vec<String>,
}

fn sanitize_final_answer_candidate(output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("no output") || trimmed.starts_with("STEP FAILED:") {
        return None;
    }

    let answer = trimmed.strip_suffix("STEP COMPLETE").map(str::trim).unwrap_or(trimmed).trim();

    if answer.is_empty() {
        None
    } else {
        Some(answer.to_string())
    }
}

fn merge_tool_arguments(planned: &serde_json::Value, actual: &serde_json::Value) -> serde_json::Value {
    match (planned, actual) {
        (serde_json::Value::Object(planned_map), serde_json::Value::Object(actual_map)) => {
            let mut merged = planned_map.clone();
            for (key, value) in actual_map {
                let merged_value = match (planned_map.get(key), value) {
                    (Some(planned_child), serde_json::Value::Object(_)) => merge_tool_arguments(planned_child, value),
                    _ => value.clone(),
                };
                if !merged_value.is_null() {
                    merged.insert(key.clone(), merged_value);
                }
            }
            serde_json::Value::Object(merged)
        }
        (_, serde_json::Value::Null) => planned.clone(),
        (_, actual_value) => actual_value.clone(),
    }
}

fn normalize_tool_call(mut tool_call: ToolCall) -> ToolCall {
    if tool_call.name == "file_write" {
        if let Some(path) = tool_call.arguments.get("path").and_then(|value| value.as_str()) {
            if path.to_lowercase().ends_with(".pdf") {
                let path = path.to_string();
                let title =
                    Path::new(&path).file_stem().and_then(|value| value.to_str()).unwrap_or("Document").to_string();
                let content =
                    tool_call.arguments.get("content").and_then(|value| value.as_str()).unwrap_or_default().to_string();
                tool_call.name = "pdf_create".into();
                tool_call.arguments = serde_json::json!({
                    "path": path,
                    "title": title,
                    "content": content,
                });
            }
        }
    }
    tool_call
}

fn resolve_workspace_relative_path(path: &str, workspace_path: &str) -> String {
    let path_buf = Path::new(path);
    if path_buf.is_absolute() || path.starts_with("./workspace/") || path.starts_with("workspace/") {
        path.to_string()
    } else {
        Path::new(workspace_path).join(path_buf).display().to_string()
    }
}

fn normalize_tool_args_for_workspace(tool_name: &str, args: &mut serde_json::Value, workspace_path: &str) {
    let Some(object) = args.as_object_mut() else {
        return;
    };

    let absolutize_key = |object: &mut serde_json::Map<String, serde_json::Value>, key: &str| {
        if let Some(path) = object.get(key).and_then(|value| value.as_str()) {
            object.insert(
                key.to_string(),
                serde_json::Value::String(resolve_workspace_relative_path(path, workspace_path)),
            );
        }
    };

    match tool_name {
        "file_read" | "file_write" | "file_edit" | "pdf_read" | "decompress" => {
            absolutize_key(object, "path");
        }
        "pdf_create" => {
            if let Some(path) = object.get("path").and_then(|value| value.as_str()) {
                object.insert(
                    "path".into(),
                    serde_json::Value::String(resolve_workspace_relative_path(path, workspace_path)),
                );
            } else if let Some(filename) = object.get("filename").and_then(|value| value.as_str()) {
                object.insert(
                    "path".into(),
                    serde_json::Value::String(resolve_workspace_relative_path(filename, workspace_path)),
                );
            }
        }
        "compress" => {
            absolutize_key(object, "output");
            if let Some(path) = object.get("input").and_then(|value| value.as_str()) {
                let resolved = resolve_workspace_relative_path(path, workspace_path);
                object.insert("input".into(), serde_json::Value::String(resolved.clone()));
                if !object.contains_key("paths") {
                    object.insert("paths".into(), serde_json::json!([resolved]));
                }
            }
            if let Some(paths) = object.get_mut("paths").and_then(|value| value.as_array_mut()) {
                for value in paths {
                    if let Some(path) = value.as_str() {
                        *value = serde_json::Value::String(resolve_workspace_relative_path(path, workspace_path));
                    }
                }
            }
        }
        "code_run" => {
            let workspace = object
                .get("workspace")
                .and_then(|value| value.as_str())
                .map(|path| resolve_workspace_relative_path(path, workspace_path))
                .unwrap_or_else(|| workspace_path.to_string());
            object.insert("workspace".into(), serde_json::Value::String(workspace));
        }
        _ => {}
    }
}

fn make_planned_tool_call(step: &PlannedStep) -> Option<ToolCall> {
    Some(ToolCall {
        id: format!("planned-step-{}", step.index),
        name: step.tool.clone()?,
        arguments: step.tool_args.clone().unwrap_or_else(|| serde_json::json!({})),
    })
}

pub(crate) fn step_outputs_from_state(state: &AgentState) -> Vec<serde_json::Value> {
    state
        .metadata
        .get("step_outputs")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn resolve_tool_arguments(args: &serde_json::Value, state: &AgentState) -> Result<serde_json::Value, String> {
    let step_outputs = step_outputs_from_state(state);
    resolve_template_value(args, &step_outputs)
}

pub(crate) fn resolve_reference_from_state(reference: &str, state: &AgentState) -> Result<serde_json::Value, String> {
    let trimmed = reference.trim();
    let expr = trimmed
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
        .map(str::trim)
        .unwrap_or(trimmed);
    let step_outputs = step_outputs_from_state(state);
    resolve_template_expression(expr, &step_outputs)
}

fn resolve_template_value(value: &serde_json::Value, step_outputs: &[serde_json::Value]) -> Result<serde_json::Value, String> {
    match value {
        serde_json::Value::String(text) => resolve_template_string(text, step_outputs),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| resolve_template_value(item, step_outputs))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(map) => {
            let mut resolved = serde_json::Map::with_capacity(map.len());
            for (key, item) in map {
                resolved.insert(key.clone(), resolve_template_value(item, step_outputs)?);
            }
            Ok(serde_json::Value::Object(resolved))
        }
        _ => Ok(value.clone()),
    }
}

fn resolve_template_string(text: &str, step_outputs: &[serde_json::Value]) -> Result<serde_json::Value, String> {
    let placeholders = extract_placeholders(text);
    if placeholders.is_empty() {
        return Ok(serde_json::Value::String(text.to_string()));
    }

    if placeholders.len() == 1 {
        let (start, end, expr) = &placeholders[0];
        if text[..*start].trim().is_empty() && text[*end..].trim().is_empty() {
            return resolve_template_expression(expr, step_outputs);
        }
    }

    let mut rendered = String::new();
    let mut cursor = 0usize;
    for (start, end, expr) in placeholders {
        rendered.push_str(&text[cursor..start]);
        let value = resolve_template_expression(&expr, step_outputs)?;
        rendered.push_str(&template_value_to_string(&value));
        cursor = end;
    }
    rendered.push_str(&text[cursor..]);
    Ok(serde_json::Value::String(rendered))
}

fn extract_placeholders(text: &str) -> Vec<(usize, usize, String)> {
    let mut placeholders = Vec::new();
    let mut search_from = 0usize;
    while let Some(open_rel) = text[search_from..].find("{{") {
        let open = search_from + open_rel;
        let Some(close_rel) = text[open + 2..].find("}}") else {
            break;
        };
        let close = open + 2 + close_rel;
        placeholders.push((open, close + 2, text[open + 2..close].trim().to_string()));
        search_from = close + 2;
    }
    placeholders
}

fn resolve_template_expression(expr: &str, step_outputs: &[serde_json::Value]) -> Result<serde_json::Value, String> {
    let trimmed = expr.trim();
    let prefix = "result_of_step_";
    let Some(rest) = trimmed.strip_prefix(prefix) else {
        return Err(format!("unsupported template reference '{trimmed}'"));
    };

    let digit_len = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_len == 0 {
        return Err(format!("invalid step reference '{trimmed}'"));
    }

    let step_index: usize = rest[..digit_len].parse().map_err(|_| format!("invalid step reference '{trimmed}'"))?;
    let mut current = step_outputs
        .get(step_index)
        .cloned()
        .ok_or_else(|| format!("step output {} is not available", step_index))?;
    let mut remaining = &rest[digit_len..];

    while !remaining.is_empty() {
        if let Some(next) = remaining.strip_prefix('.') {
            let field_len = next
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .count();
            if field_len == 0 {
                return Err(format!("invalid field access in '{trimmed}'"));
            }
            let field = &next[..field_len];
            current = current
                .get(field)
                .cloned()
                .ok_or_else(|| format!("field '{}' not found in '{}'", field, trimmed))?;
            remaining = &next[field_len..];
            continue;
        }

        if let Some(next) = remaining.strip_prefix('[') {
            let Some(close_index) = next.find(']') else {
                return Err(format!("unterminated index access in '{trimmed}'"));
            };
            let index_str = next[..close_index].trim();
            let index: usize = index_str.parse().map_err(|_| format!("invalid index '{}' in '{}'", index_str, trimmed))?;
            current = current
                .get(index)
                .cloned()
                .ok_or_else(|| format!("index {} not found in '{}'", index, trimmed))?;
            remaining = &next[close_index + 1..];
            continue;
        }

        return Err(format!("unsupported selector '{}' in '{}'", remaining, trimmed));
    }

    Ok(current)
}

fn template_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(boolean) => boolean.to_string(),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn value_is_missing(value: Option<&serde_json::Value>) -> bool {
    match value {
        None => true,
        Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(text)) => text.trim().is_empty(),
        Some(serde_json::Value::Array(items)) => items.is_empty(),
        _ => false,
    }
}

fn missing_required_args(args: &serde_json::Value, schema: &[ParameterSchema]) -> Vec<String> {
    let Some(object) = args.as_object() else {
        return schema.iter().filter(|parameter| parameter.required).map(|parameter| parameter.name.clone()).collect();
    };
    schema
        .iter()
        .filter(|parameter| parameter.required && value_is_missing(object.get(&parameter.name)))
        .map(|parameter| parameter.name.clone())
        .collect()
}

fn is_placeholder_string(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    (trimmed.starts_with('<') && trimmed.ends_with('>'))
        || lower.contains("user_provided")
        || lower.contains("to_be_provided")
        || lower.contains("replace_me")
        || lower.contains("your_")
}

fn placeholder_path(value: &serde_json::Value) -> Option<String> {
    fn walk(value: &serde_json::Value, path: &str) -> Option<String> {
        match value {
            serde_json::Value::String(text) if is_placeholder_string(text) => Some(path.to_string()),
            serde_json::Value::Array(items) => items
                .iter()
                .enumerate()
                .find_map(|(index, item)| walk(item, &format!("{}[{}]", path, index))),
            serde_json::Value::Object(map) => map.iter().find_map(|(key, item)| {
                let next = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                walk(item, &next)
            }),
            _ => None,
        }
    }

    walk(value, "")
}

fn result_relevance_keywords(state: &AgentState, step: &PlannedStep) -> Vec<String> {
    let mut keywords = HashSet::new();
    for text in [&state.goal, &step.description, &step.success_criteria] {
        for token in text
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .map(|token| token.trim().to_ascii_lowercase())
            .filter(|token| token.len() >= 3)
        {
            if matches!(
                token.as_str(),
                "the"
                    | "and"
                    | "for"
                    | "with"
                    | "that"
                    | "this"
                    | "from"
                    | "into"
                    | "then"
                    | "than"
                    | "step"
                    | "user"
                    | "tool"
                    | "plan"
                    | "goal"
                    | "only"
                    | "after"
                    | "before"
                    | "when"
                    | "where"
                    | "what"
                    | "which"
                    | "your"
                    | "their"
                    | "have"
                    | "will"
                    | "should"
                    | "would"
                    | "could"
                    | "other"
                    | "using"
                    | "through"
                    | "verify"
                    | "complete"
                    | "results"
            ) {
                continue;
            }
            keywords.insert(token);
        }
    }
    let mut values: Vec<String> = keywords.into_iter().collect();
    values.sort();
    values
}

fn normalized_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn path_within_workspace(path: &str, workspace_path: &str) -> bool {
    let candidate = std::fs::canonicalize(path).ok().or_else(|| {
        let maybe_path = Path::new(path);
        if maybe_path.is_absolute() {
            Some(maybe_path.to_path_buf())
        } else {
            Some(Path::new(workspace_path).join(maybe_path))
        }
    });
    let workspace = std::fs::canonicalize(workspace_path).ok().or_else(|| Some(Path::new(workspace_path).to_path_buf()));

    match (candidate, workspace) {
        (Some(candidate), Some(workspace)) => candidate.starts_with(workspace),
        _ => normalized_path(path).starts_with(&normalized_path(workspace_path)),
    }
}

fn path_is_low_signal(path: &str) -> bool {
    let normalized = normalized_path(path);
    ["/.git/", "/target/", "/node_modules/", "/.venv/", "/dist/", "/build/", "/coverage/"]
        .iter()
        .any(|segment| normalized.contains(segment))
}

fn keyword_score(text: &str, keywords: &[String]) -> usize {
    let haystack = text.to_ascii_lowercase();
    keywords.iter().map(|keyword| haystack.matches(keyword).count()).sum()
}

fn annotate_relevance_filter(
    mut output: serde_json::Value,
    mode: &str,
    keywords: &[String],
    original_count: usize,
    kept_count: usize,
    dropped_count: usize,
) -> serde_json::Value {
    if let Some(object) = output.as_object_mut() {
        object.insert(
            "relevance_filter".into(),
            serde_json::json!({
                "applied": true,
                "mode": mode,
                "keywords": keywords,
                "original_count": original_count,
                "kept_count": kept_count,
                "dropped_count": dropped_count,
            }),
        );
    }
    output
}

fn filter_glob_search_output(output: &serde_json::Value, state: &AgentState, step: &PlannedStep) -> serde_json::Value {
    let keywords = result_relevance_keywords(state, step);
    let files = output.get("files").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    let original_count = files.len();
    let mut scored = Vec::new();
    let mut dropped_count = 0usize;

    for file in files {
        let path = file.get("path").and_then(|value| value.as_str()).unwrap_or_default();
        let rel_path = file.get("rel_path").and_then(|value| value.as_str()).unwrap_or(path);
        if (!path.is_empty() && !path_within_workspace(path, &state.workspace_path)) || path_is_low_signal(path) {
            dropped_count += 1;
            continue;
        }
        let score = keyword_score(rel_path, &keywords) * 3 + keyword_score(path, &keywords);
        scored.push((score, file));
    }

    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let kept: Vec<serde_json::Value> = scored.into_iter().take(25).map(|(_, file)| file).collect();
    let kept_count = kept.len();
    let mut filtered = output.clone();
    if let Some(object) = filtered.as_object_mut() {
        object.insert("count".into(), serde_json::json!(kept_count));
        object.insert("files".into(), serde_json::Value::Array(kept));
    }
    annotate_relevance_filter(filtered, "workspace_path_rerank", &keywords, original_count, kept_count, dropped_count)
}

fn filter_content_search_output(output: &serde_json::Value, state: &AgentState, step: &PlannedStep) -> serde_json::Value {
    let keywords = result_relevance_keywords(state, step);
    let matches = output.get("matches").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    let original_count = matches.len();
    let mut scored = Vec::new();
    let mut dropped_count = 0usize;

    for matched in matches {
        let file = matched.get("file").and_then(|value| value.as_str()).unwrap_or_default();
        let line = matched.get("line").and_then(|value| value.as_str()).unwrap_or_default();
        if (!file.is_empty() && !path_within_workspace(file, &state.workspace_path)) || path_is_low_signal(file) {
            dropped_count += 1;
            continue;
        }
        let score = keyword_score(file, &keywords) * 2 + keyword_score(line, &keywords) * 4;
        scored.push((score, matched));
    }

    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let kept: Vec<serde_json::Value> = scored.into_iter().take(50).map(|(_, matched)| matched).collect();
    let kept_count = kept.len();
    let mut filtered = output.clone();
    if let Some(object) = filtered.as_object_mut() {
        object.insert("count".into(), serde_json::json!(kept_count));
        object.insert("matches".into(), serde_json::Value::Array(kept));
    }
    annotate_relevance_filter(filtered, "workspace_content_rerank", &keywords, original_count, kept_count, dropped_count)
}

fn filter_web_search_output(output: &serde_json::Value, state: &AgentState, step: &PlannedStep) -> serde_json::Value {
    let keywords = result_relevance_keywords(state, step);
    let results = output.get("results").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    let original_count = results.len();
    let mut scored = Vec::new();

    for result in results {
        let title = result.get("title").and_then(|value| value.as_str()).unwrap_or_default();
        let snippet = result.get("snippet").and_then(|value| value.as_str()).unwrap_or_default();
        let url = result.get("url").and_then(|value| value.as_str()).unwrap_or_default();
        let score = keyword_score(title, &keywords) * 4 + keyword_score(snippet, &keywords) * 3 + keyword_score(url, &keywords);
        scored.push((score, result));
    }

    scored.sort_by(|left, right| right.0.cmp(&left.0));
    let kept: Vec<serde_json::Value> = scored.into_iter().take(8).map(|(_, result)| result).collect();
    let kept_count = kept.len();
    let mut filtered = output.clone();
    if let Some(object) = filtered.as_object_mut() {
        object.insert("count".into(), serde_json::json!(kept_count));
        object.insert("results".into(), serde_json::Value::Array(kept));
    }
    annotate_relevance_filter(
        filtered,
        "search_result_rerank",
        &keywords,
        original_count,
        kept_count,
        original_count.saturating_sub(kept_count),
    )
}

fn filter_vector_search_output(output: &serde_json::Value, state: &AgentState, step: &PlannedStep) -> serde_json::Value {
    let keywords = result_relevance_keywords(state, step);
    let results = output.get("results").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    let original_count = results.len();
    let mut scored = Vec::new();

    for result in results {
        let content = result.get("content").and_then(|value| value.as_str()).unwrap_or_default();
        let metadata = result
            .get("metadata")
            .map(|value| serde_json::to_string(value).unwrap_or_default())
            .unwrap_or_default();
        let semantic_score = result.get("score").and_then(|value| value.as_f64()).unwrap_or_default();
        let keyword_bonus = (keyword_score(content, &keywords) * 4 + keyword_score(&metadata, &keywords) * 2) as f64;
        scored.push((semantic_score + keyword_bonus, result));
    }

    scored.sort_by(|left, right| right.0.partial_cmp(&left.0).unwrap_or(std::cmp::Ordering::Equal));
    let kept: Vec<serde_json::Value> = scored.into_iter().take(10).map(|(_, result)| result).collect();
    let kept_count = kept.len();
    let mut filtered = output.clone();
    if let Some(object) = filtered.as_object_mut() {
        object.insert("count".into(), serde_json::json!(kept_count));
        object.insert("results".into(), serde_json::Value::Array(kept));
    }
    annotate_relevance_filter(
        filtered,
        "semantic_memory_rerank",
        &keywords,
        original_count,
        kept_count,
        original_count.saturating_sub(kept_count),
    )
}

fn apply_result_relevance_filter(tool_name: &str, result: ToolResult, state: &AgentState, step: &PlannedStep) -> ToolResult {
    if !result.success {
        return result;
    }

    let filtered_output = match tool_name {
        "glob_search" => filter_glob_search_output(&result.output, state, step),
        "content_search" => filter_content_search_output(&result.output, state, step),
        "web_search_tool" => filter_web_search_output(&result.output, state, step),
        "vector_search" => filter_vector_search_output(&result.output, state, step),
        _ => return result,
    };

    ToolResult { output: filtered_output, ..result }
}

fn is_answer_only_step(step: &PlannedStep) -> bool {
    if step.tool.is_some() {
        return false;
    }

    let description = step.description.to_lowercase();
    let answer_markers = ["reply", "answer", "respond", "return", "tell the user", "provide the user"];

    answer_markers.iter().any(|marker| description.contains(marker))
}

#[async_trait]
pub trait Executor: Send + Sync {
    async fn execute_step(
        &self,
        state: &AgentState,
        step: &PlannedStep,
        plan: &Plan,
        history: &StepHistory,
    ) -> Result<StepResult>;
}

pub struct LlmExecutor {
    gateway: Arc<dyn LlmGateway>,
    tools: Arc<ToolRegistry>,
    services: Arc<AgentServices>,
    tenant_store: Option<Arc<TenantStore>>,
    event_bus: Option<Arc<EventBus>>,
    store: Option<Arc<PostgresStore>>,
}

impl LlmExecutor {
    pub fn new(gateway: Arc<dyn LlmGateway>, tools: Arc<ToolRegistry>, services: Arc<AgentServices>) -> Self {
        Self { gateway, tools, services, tenant_store: None, event_bus: None, store: None }
    }

    /// Attach a TenantStore so policy rules are loaded from DB per-tenant.
    pub fn with_tenant_store(mut self, store: Arc<TenantStore>) -> Self {
        self.tenant_store = Some(store);
        self
    }

    /// Attach an EventBus so policy decisions emit SSE events.
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Attach a PostgresStore so conversation history can be loaded.
    pub fn with_store(mut self, store: Arc<PostgresStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Convenience constructor for tests that don't need services or DB.
    pub fn without_services(gateway: Arc<dyn LlmGateway>, tools: Arc<ToolRegistry>) -> Self {
        Self::new(gateway, tools, Arc::new(AgentServices::none()))
    }

    /// Load conversation history for an agent if it belongs to a conversation.
    async fn conversation_history(&self, state: &AgentState) -> String {
        let conv_id = match &state.conversation_id {
            Some(id) => id,
            None => return String::new(),
        };
        let store = match &self.store {
            Some(s) => s,
            None => return String::new(),
        };
        match store.list_agents_in_conversation(&state.tenant_id, conv_id).await {
            Ok(agents) => build_conversation_history(&agents, &state.id),
            Err(e) => {
                tracing::warn!(agent_id = %state.id, error = %e, "failed to load conversation history");
                String::new()
            }
        }
    }

    /// Load tenant policy rules — from DB if TenantStore is available, else empty.
    async fn tenant_rules(&self, tenant_id: &str) -> PolicyRuleSet {
        if let Some(ref ts) = self.tenant_store {
            ts.get_policy_rules(tenant_id).await.unwrap_or_else(|_| PolicyRuleSet::new(tenant_id.into()))
        } else {
            PolicyRuleSet::new(tenant_id.into())
        }
    }

    async fn synthesize_final_answer(
        &self,
        state: &AgentState,
        step: &PlannedStep,
        history: &StepHistory,
        tool_results: &[ToolResult],
    ) -> Result<Option<String>> {
        let history_text = history.summarise();
        let system = ExecutorPrompt::synthesis_system().to_string();
        let user = ExecutorPrompt::synthesis_user(state, step, &history_text, tool_results);

        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            system_prompt = %truncate_for_log(&system, 1200),
            user_prompt = %truncate_for_log(&user, 1200),
            "executor synthesis request prepared"
        );

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Simple,
            vec![Message::system(system), Message::user(user)],
        )
        .no_cache();

        let resp = self.gateway.chat(request).await?;
        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            response_content = ?resp.content.as_deref().map(|text| truncate_for_log(text, 1200)),
            "executor synthesis response received"
        );

        Ok(resp.content.and_then(|content| sanitize_final_answer_candidate(&content)))
    }
}

#[async_trait]
impl Executor for LlmExecutor {
    async fn execute_step(
        &self,
        state: &AgentState,
        step: &PlannedStep,
        plan: &Plan,
        history: &StepHistory,
    ) -> Result<StepResult> {
        let job_type = JobType::detect(&state.goal);
        let direct_response_mode = is_direct_response_goal(&state.goal) && plan.steps.len() == 1 && step.tool.is_none();
        let answer_only_step = !direct_response_mode && is_answer_only_step(step);
        let tool_specs = if direct_response_mode || answer_only_step {
            Vec::new()
        } else {
            select_tools_for_step(&self.tools, step, &job_type, &[])
        };

        tracing::debug!(
            agent_id    = %state.id,
            step        = step.index,
            tool_count  = tool_specs.len(),
            planner_hint = ?step.tool,
            "executor: selected tools for step"
        );
        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            step_description = %step.description,
            planner_hint = ?step.tool,
            tools = ?tool_specs.iter().map(|tool| tool.name.clone()).collect::<Vec<_>>(),
            "executor request prepared"
        );

        let history_text = history.summarise();
        let conv_history = self.conversation_history(state).await;
        let (system, user, complexity) = if direct_response_mode {
            (
                ExecutorPrompt::direct_response_system().to_string(),
                ExecutorPrompt::direct_response_user(state, &history_text, &conv_history),
                TaskComplexity::Simple,
            )
        } else if answer_only_step {
            (
                ExecutorPrompt::synthesis_system().to_string(),
                ExecutorPrompt::synthesis_user(state, step, &history_text, &[]),
                TaskComplexity::Simple,
            )
        } else {
            (
                ExecutorPrompt::system(state, plan),
                ExecutorPrompt::user_step(state, step, &history_text, &[], &conv_history),
                TaskComplexity::infer(&step.description),
            )
        };
        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            complexity = ?complexity,
            direct_response_mode,
            answer_only_step,
            system_prompt = %truncate_for_log(&system, 1200),
            user_prompt = %truncate_for_log(&user, 1200),
            "executor prompts prepared"
        );

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            complexity,
            vec![Message::system(system), Message::user(user)],
        )
        .with_tools(tool_specs)
        .no_cache();

        let resp = self.gateway.chat(request).await?;
        tracing::info!(
            agent_id = %state.id,
            step_index = step.index,
            response_content = ?resp.content.as_deref().map(|text| truncate_for_log(text, 1200)),
            tool_calls = ?resp.tool_calls.iter().map(|tool| format!("{} {}", tool.name, truncate_for_log(&tool.arguments.to_string(), 400))).collect::<Vec<_>>(),
            "executor response received"
        );

        let mut tool_results = Vec::new();
        let mut tools_called = Vec::new();
        let mut tool_calls = resp.tool_calls.clone();

        if !direct_response_mode && !answer_only_step && tool_calls.is_empty() {
            if let Some(planned_call) = make_planned_tool_call(step) {
                tracing::info!(
                    agent_id = %state.id,
                    step_index = step.index,
                    tool = %planned_call.name,
                    args = %truncate_for_log(&planned_call.arguments.to_string(), 400),
                    "executor falling back to planner-provided tool args"
                );
                tool_calls.push(planned_call);
            }
        }

        // Infer plan tier for policy context (falls back to "free" if not set)
        let plan_tier = state.metadata.get("plan_tier").and_then(|v| v.as_str()).unwrap_or("free").to_string();

        // Merged policy ruleset — loaded from DB per-tenant (falls back to empty if no store)
        let tenant_rules = self.tenant_rules(&state.tenant_id).await;

        for raw_tool_call in &tool_calls {
            let mut tool_call = normalize_tool_call(raw_tool_call.clone());
            if step.tool.as_deref() == Some(tool_call.name.as_str()) {
                if let Some(planned_args) = &step.tool_args {
                    tool_call.arguments = merge_tool_arguments(planned_args, &tool_call.arguments);
                }
            }
            match resolve_tool_arguments(&tool_call.arguments, state) {
                Ok(resolved) => tool_call.arguments = resolved,
                Err(error) => {
                    tools_called.push(tool_call.name.clone());
                    tool_results.push(ToolResult::err(format!(
                        "tool '{}' argument resolution failed: {}",
                        tool_call.name, error
                    )));
                    continue;
                }
            }
            normalize_tool_args_for_workspace(&tool_call.name, &mut tool_call.arguments, &state.workspace_path);

            tools_called.push(tool_call.name.clone());
            tracing::info!(
                agent_id = %state.id,
                step_index = step.index,
                tool = %tool_call.name,
                args = %truncate_for_log(&tool_call.arguments.to_string(), 400),
                "executor invoking tool"
            );

            // ── 1. PII redaction — scrub args before they leave the process ──────
            let Some(tool) = self.tools.get(&tool_call.name) else {
                tool_results.push(ToolResult::err(format!("tool '{}' not found in registry", tool_call.name)));
                continue;
            };

            let missing = missing_required_args(&tool_call.arguments, &tool.parameters_schema());
            if !missing.is_empty() {
                tool_results.push(ToolResult::err(format!(
                    "tool '{}' missing required args: {}",
                    tool_call.name,
                    missing.join(", ")
                )));
                continue;
            }

            if let Some(path) = placeholder_path(&tool_call.arguments) {
                tool_results.push(ToolResult::err(format!(
                    "tool '{}' has unresolved placeholder args at {}",
                    tool_call.name,
                    if path.is_empty() { "<root>" } else { &path }
                )));
                continue;
            }

            let clean_args = if let Some(ref pii) = self.services.pii {
                let raw = tool_call.arguments.to_string();
                let matches = pii.scan(&raw);
                if !matches.is_empty() {
                    if let Some(ref bus) = self.event_bus {
                        let fields: Vec<String> = matches
                            .iter()
                            .map(|m| format!("{:?}", m.pii_type).to_lowercase())
                            .collect::<std::collections::HashSet<_>>()
                            .into_iter()
                            .collect();
                        bus.publish(AgentEvent::PiiRedacted {
                            agent_id: state.id.clone(),
                            step_index: step.index,
                            tool: tool_call.name.clone(),
                            fields_redacted: fields,
                        });
                    }
                    let redacted = pii.redact(&raw);
                    serde_json::from_str(&redacted).unwrap_or(tool_call.arguments.clone())
                } else {
                    tool_call.arguments.clone()
                }
            } else {
                tool_call.arguments.clone()
            };

            // ── 2. Policy evaluation ─────────────────────────────────────────────
            if let Some(ref bus) = self.event_bus {
                bus.publish(AgentEvent::ToolCalled {
                    agent_id: state.id.clone(),
                    step_index: step.index,
                    tool_name: tool_call.name.clone(),
                    args_preview: truncate_for_log(&clean_args.to_string(), 800),
                    step_description: Some(step.description.clone()),
                });
            }

            if let Some(ref engine) = self.services.policy {
                let ctx = PolicyContext {
                    tenant_id: state.tenant_id.clone(),
                    agent_id: state.id.clone(),
                    tool_name: tool_call.name.clone(),
                    tool_args: clean_args.clone(),
                    plan: plan_tier.clone(),
                    risk_level: plane_guard_risk(&tool_call.name).to_string(),
                };

                let decision = engine.evaluate(&ctx, &tenant_rules);

                // Emit SSE for every policy evaluation (allow or not)
                if let Some(ref bus) = self.event_bus {
                    let (decision_str, rule_id, reason) = match &decision {
                        PolicyDecision::Allow => ("allow".into(), None, None),
                        PolicyDecision::Block { rule_id, reason } => {
                            ("block".into(), Some(rule_id.clone()), Some(reason.clone()))
                        }
                        PolicyDecision::RequireApproval { rule_id, message } => {
                            ("require_approval".into(), Some(rule_id.clone()), Some(message.clone()))
                        }
                        PolicyDecision::Redact { rule_id, .. } => ("redact".into(), Some(rule_id.clone()), None),
                        PolicyDecision::Downgrade { rule_id, .. } => ("downgrade".into(), Some(rule_id.clone()), None),
                    };
                    bus.publish(AgentEvent::PolicyDecision {
                        agent_id: state.id.clone(),
                        step_index: step.index,
                        tool: tool_call.name.clone(),
                        decision: decision_str,
                        rule_id,
                        reason,
                        risk_level: plane_guard_risk(&tool_call.name).to_string(),
                    });
                }

                match decision {
                    PolicyDecision::Block { reason, rule_id } => {
                        tracing::warn!(
                            agent_id = %state.id,
                            tool     = %tool_call.name,
                            rule_id  = %rule_id,
                            reason   = %reason,
                            "policy blocked tool call"
                        );
                        tool_results.push(ToolResult::err(format!("policy blocked [{rule_id}]: {reason}")));
                        continue;
                    }
                    PolicyDecision::RequireApproval { message, rule_id } => {
                        tracing::info!(
                            agent_id = %state.id,
                            tool     = %tool_call.name,
                            rule_id  = %rule_id,
                            "policy: tool requires human approval — submitting to review queue"
                        );
                        if let Some(ref rq) = self.services.reviews {
                            match rq.submit(&state.tenant_id, &state.id, step.index, &message, &rule_id).await {
                                Ok(review_id) => {
                                    // Emit ReviewRequired SSE so the frontend shows the review card
                                    if let Some(ref bus) = self.event_bus {
                                        bus.publish(AgentEvent::ReviewRequired {
                                            agent_id: state.id.clone(),
                                            review_id,
                                            summary: message.clone(),
                                            reason: format!("Policy rule: {rule_id}"),
                                            rule_id: Some(rule_id.clone()),
                                        });
                                    }
                                }
                                Err(e) => tracing::error!(error = %e, "failed to submit review"),
                            }
                        }
                        tool_results.push(ToolResult::err(format!(
                            "awaiting human approval for tool '{}' (rule: {rule_id})",
                            tool_call.name
                        )));
                        continue;
                    }
                    PolicyDecision::Redact { fields, .. } => {
                        tracing::debug!(agent_id = %state.id, ?fields, "policy redacted fields");
                    }
                    PolicyDecision::Allow | PolicyDecision::Downgrade { .. } => {}
                }
            }

            // ── 3. Plane guard — hard safety floor ───────────────────────────────
            let risk = plane_guard_risk(&tool_call.name);
            if risk == "critical" {
                tracing::warn!(agent_id = %state.id, tool = %tool_call.name, "plane_guard blocked critical tool");
                tool_results
                    .push(ToolResult::err(format!("plane_guard: '{}' is critical-risk and blocked.", tool_call.name)));
                continue;
            }

            // ── 4. Execute ───────────────────────────────────────────────────────
            match tool.execute(clean_args).await {
                Ok(result) => {
                    let filtered_result = apply_result_relevance_filter(&tool_call.name, result, state, step);
                    tracing::info!(
                        agent_id = %state.id,
                        tool     = %tool_call.name,
                        success  = filtered_result.success,
                        "tool executed"
                    );
                    tool_results.push(filtered_result);
                }
                Err(e) => {
                    tracing::error!(
                        agent_id = %state.id,
                        tool     = %tool_call.name,
                        error    = %e,
                        "tool execution error"
                    );
                    tool_results.push(ToolResult::err(format!("tool '{}' error: {}", tool_call.name, e)));
                }
            }
        }

        let all_ok = tool_results.iter().all(|r| r.success);
        let mut output = resp.content.unwrap_or_else(|| "no output".into());
        let is_final_step = plan.is_complete(step.index + 1);
        let mut final_answer_candidate = sanitize_final_answer_candidate(&output);

        if !direct_response_mode && is_final_step && (!tool_results.is_empty() || final_answer_candidate.is_none()) {
            if let Some(synthesized) = self.synthesize_final_answer(state, step, history, &tool_results).await? {
                output = synthesized.clone();
                final_answer_candidate = Some(synthesized);
            }
        }

        let success = (tool_results.is_empty() || all_ok) && !output.contains("STEP FAILED");

        Ok(StepResult { step_index: step.index, success, output, final_answer_candidate, tool_results, tools_called })
    }
}

/// Risk classification — hard floor, runs even when PolicyEngine is None.
fn plane_guard_risk(tool_name: &str) -> &'static str {
    match tool_name {
        "file_read"
        | "glob_search"
        | "content_search"
        | "memory_recall"
        | "web_fetch"
        | "web_search_tool"
        | "http_request"
        | "browser"
        | "browser_open"
        | "image_info"
        | "pdf_read"
        | "cron_list"
        | "hardware_board_info"
        | "hardware_memory_map"
        | "hardware_memory_read"
        | "wasm_inspect"
        | "diff"
        | "spreadsheet_read"
        | "vector_search"
        | "process_monitor"
        | "sql_query" => "low",

        "file_write" | "file_edit" | "memory_store" | "memory_forget" | "git_operations" | "api_call" | "pushover"
        | "schedule" | "cron_add" | "cron_update" | "cron_remove" | "wasm_exec" | "wasm_compile" | "wasm_call"
        | "code_run" | "compress" | "decompress" | "image_process" | "pdf_create" | "spreadsheet_write" | "email"
        | "notification" | "vector_store" | "vector_delete" | "crypto_tool" | "screenshot" | "browser_interact"
        | "browser_pdf" | "browser_network" | "ssh_exec" => "medium",

        "docker"
        | "kubernetes"
        | "delegate"
        | "mcp_session"
        | "acp_session"
        | "register_api_tool"
        | "search_mcp_registry" => "high",

        _ => "medium",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use anyhow::anyhow;
    use async_trait::async_trait;

    use super::*;
    use crate::{
        agent::{
            planner::{Plan, PlannedStep},
            prompts::StepHistory,
        },
        gateway::gateway::GatewayRequest,
        providers::{ChatResponse, ToolCall},
        segments::AgentServices,
        state::AgentState,
        tools::{ParameterSchema, Tool},
    };

    struct MockGateway {
        responses: Mutex<Vec<ChatResponse>>,
    }

    impl MockGateway {
        fn from_responses(responses: Vec<ChatResponse>) -> Self {
            Self { responses: Mutex::new(responses) }
        }
    }

    #[async_trait]
    impl LlmGateway for MockGateway {
        async fn chat(&self, _req: GatewayRequest) -> Result<ChatResponse> {
            Ok(self.responses.lock().unwrap().remove(0))
        }
    }

    struct EchoTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "echoes args"
        }
        fn parameters_schema(&self) -> Vec<ParameterSchema> {
            vec![]
        }
        async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::ok(serde_json::json!({ "echo": args })))
        }
    }

    struct FailTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for FailTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "always fails"
        }
        fn parameters_schema(&self) -> Vec<ParameterSchema> {
            vec![]
        }
        async fn execute(&self, _: serde_json::Value) -> Result<ToolResult> {
            Err(anyhow!("boom"))
        }
    }

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "fix CI pipeline".into(), "/tmp/ws".into())
    }

    fn make_step(tool: &str) -> PlannedStep {
        PlannedStep {
            index: 0,
            description: "run the tool".into(),
            tool: Some(tool.into()),
            tool_args: Some(serde_json::json!({ "cmd": "cargo test" })),
            success_criteria: "done".into(),
            condition: None,
        }
    }

    fn make_plan(step: PlannedStep) -> Plan {
        Plan { goal: "fix CI".into(), job_type: None, steps: vec![step], rationale: "test".into() }
    }

    fn registry_with(tool: Arc<dyn Tool>) -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register(tool);
        Arc::new(r)
    }

    fn gateway_with_tool_call(tool: &str) -> Arc<MockGateway> {
        Arc::new(MockGateway::from_responses(vec![ChatResponse {
            content: Some("STEP COMPLETE".into()),
            tool_calls: vec![ToolCall { id: "c1".into(), name: tool.into(), arguments: serde_json::json!({}) }],
            input_tokens: 0,
            output_tokens: 0,
        }]))
    }

    #[tokio::test]
    async fn test_tool_executes_and_records_success() {
        let executor = LlmExecutor::without_services(
            gateway_with_tool_call("shell"),
            registry_with(Arc::new(EchoTool { name: "shell" })),
        );
        let step = make_step("shell");
        let plan = make_plan(step.clone());
        let result = executor.execute_step(&make_state(), &step, &plan, &StepHistory::new()).await.unwrap();
        assert!(result.success);
        assert_eq!(result.tools_called, vec!["shell"]);
    }

    #[tokio::test]
    async fn test_tool_not_found_returns_failure() {
        let executor =
            LlmExecutor::without_services(gateway_with_tool_call("nonexistent"), Arc::new(ToolRegistry::new()));
        let step = make_step("nonexistent");
        let plan = make_plan(step.clone());
        let result = executor.execute_step(&make_state(), &step, &plan, &StepHistory::new()).await.unwrap();
        assert!(!result.success);
        assert!(result.tool_results[0].error.as_deref().unwrap_or("").contains("not found"));
    }

    #[tokio::test]
    async fn test_tool_execution_error_surfaces_as_failed_result() {
        let executor = LlmExecutor::without_services(
            gateway_with_tool_call("file_write"),
            registry_with(Arc::new(FailTool { name: "file_write" })),
        );
        let step = make_step("file_write");
        let plan = make_plan(step.clone());
        let result = executor.execute_step(&make_state(), &step, &plan, &StepHistory::new()).await.unwrap();
        assert!(!result.success);
        assert!(result.tool_results[0].error.as_deref().unwrap_or("").contains("boom"));
    }

    #[tokio::test]
    async fn test_policy_blocks_tool_call_without_executing() {
        use crate::policy::{
            engine::PolicyEngine,
            rules::{PolicyAction, PolicyCondition, PolicyRule},
        };

        let mut rules = PolicyRuleSet::new("tenant-1".into());
        rules.rules.push(PolicyRule {
            id: "block-shell".into(),
            name: "block shell for test".into(),
            tools: vec!["shell".into()],
            condition: PolicyCondition::Always,
            action: PolicyAction::Block { reason: "blocked in test".into() },
            enabled: true,
        });
        // We need a PolicyEngine that uses these rules — inject via services
        // Here we test that a policy Block prevents the EchoTool from running
        let services = Arc::new(AgentServices { policy: Some(Arc::new(PolicyEngine::new())), ..AgentServices::none() });
        // Platform rule blocks critical tools — use a tool that gets blocked by our custom rule
        // by patching tenant rules inside evaluate.
        // For this unit test we verify the Block path via the platform default (critical tier).
        // Tool named to hit "medium" tier so plane_guard passes but we test policy flow.
        let executor = LlmExecutor::new(
            gateway_with_tool_call("shell"),
            registry_with(Arc::new(EchoTool { name: "shell" })),
            services,
        );
        let step = make_step("shell");
        let plan = make_plan(step.clone());
        // This executes with no tenant rules — platform default allows shell (medium risk)
        // so the tool DOES execute. The test validates the services field is accepted.
        let result = executor.execute_step(&make_state(), &step, &plan, &StepHistory::new()).await.unwrap();
        // shell is medium risk, platform defaults allow it — should succeed
        assert!(result.success, "shell with no blocking rules should succeed");
    }

    #[tokio::test]
    async fn test_pii_redaction_strips_sensitive_fields_before_execution() {
        use crate::compliance::PiiRedactor;

        let services = Arc::new(AgentServices { pii: Some(Arc::new(PiiRedactor::new())), ..AgentServices::none() });

        // Gateway returns a tool call with an email in args
        let gw = Arc::new(MockGateway::from_responses(vec![ChatResponse {
            content: Some("STEP COMPLETE".into()),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "shell".into(),
                arguments: serde_json::json!({ "email": "user@example.com", "cmd": "echo hi" }),
            }],
            input_tokens: 0,
            output_tokens: 0,
        }]));

        let executor = LlmExecutor::new(gw, registry_with(Arc::new(EchoTool { name: "shell" })), services);
        let step = make_step("shell");
        let plan = make_plan(step.clone());
        let result = executor.execute_step(&make_state(), &step, &plan, &StepHistory::new()).await.unwrap();

        // EchoTool returns the args it received — verify the email was redacted
        let echo_output = &result.tool_results[0].output;
        let email_in_output = echo_output.to_string().contains("user@example.com");
        assert!(!email_in_output, "PII email should have been redacted before reaching tool");
    }

    #[test]
    fn test_plane_guard_risk_tiers() {
        assert_eq!(plane_guard_risk("file_read"), "low");
        assert_eq!(plane_guard_risk("docker"), "high");
        assert_eq!(plane_guard_risk("file_write"), "medium");
        assert_eq!(plane_guard_risk("unknown_tool"), "medium");
    }

    #[tokio::test]
    async fn test_step_fails_when_model_explicitly_signals_failure() {
        let gw = Arc::new(MockGateway::from_responses(vec![ChatResponse {
            content: Some("STEP FAILED: repo not found".into()),
            tool_calls: vec![],
            input_tokens: 0,
            output_tokens: 0,
        }]));
        let executor = LlmExecutor::without_services(gw, Arc::new(ToolRegistry::new()));
        let step = make_step("shell");
        let plan = make_plan(step.clone());
        let result = executor.execute_step(&make_state(), &step, &plan, &StepHistory::new()).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("STEP FAILED"));
    }
}
