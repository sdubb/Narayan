//! `request_more_tools` — meta-tool for explicit core tool category expansion.
//!
//! ## Purpose
//!
//! Distinct from the connector discovery tools (`list_connectors_in_category`,
//! `request_more_connectors`), this tool handles **non-connector** tool expansion.
//!
//! The executor starts each step with 10-15 carefully selected core tools.
//! If the LLM determines mid-step that it needs tools from a category that
//! wasn't included (e.g. it needs `wasm_exec` but only `code_run` was sent),
//! it calls `request_more_tools { categories: ["code"] }` and the executor
//! injects the full category toolset before re-calling the LLM.
//!
//! ## Distinction from connector tools
//!
//! | Situation | Use |
//! |---|---|
//! | Need Salesforce, Jira, Slack, etc. | `list_connectors_in_category` |
//! | Need more shell/file/web/code tools | `request_more_tools` |
//! | Exhausted all connectors in a category | `request_more_connectors` |
//! | Need an API not in the catalogue | `create_custom_connector` |
//!
//! ## Executor interception
//!
//! The executor intercepts this call before the normal tool dispatch path:
//!   1. Parse `categories` array from args
//!   2. Call `registry.tool_specs_for_category(cat)` for each
//!   3. Merge specs into current `tool_specs` (deduped by name)
//!   4. Re-call LLM with expanded toolset — same step, same context
//!   5. Not recorded in `tools_called` (transparent to the agent loop)

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub const TOOL_NAME: &str = "request_more_tools";

pub struct RequestMoreToolsTool;

#[async_trait]
impl Tool for RequestMoreToolsTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Request additional tools from a specific category when the current toolset \
         doesn't include what you need. This is for core tool categories (filesystem, \
         web, code, data, memory, infra, security, automation) — not for external \
         service connectors. For connectors like Salesforce or Slack, use \
         list_connectors_in_category instead. The expanded toolset is injected \
         immediately and you can use the new tools in your next action."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "categories",
                "array",
                "Core tool categories to expand. Available: \
                 filesystem (shell, file_read, file_write, glob_search, compress...), \
                 web (web_search_tool, web_fetch, http_request, browser...), \
                 code (code_run, wasm_exec, wasm_compile, sql_query, diff, patch...), \
                 data (data_extractor, pdf_read, pdf_create, spreadsheet...), \
                 memory (vector_store, vector_search, memory_store...), \
                 infra (docker, kubernetes, ssh_exec, process_monitor), \
                 security (crypto_tool, request_credential), \
                 automation (schedule, cron_add, delegate).",
            ),
            ParameterSchema::optional(
                "reason",
                "string",
                "Why you need these tools — helps the system optimise future requests.",
            ),
        ]
    }

    /// This execute() is the fallback path — in production the executor intercepts
    /// this call before it reaches the registry dispatch.
    /// Implemented so the tool is unit-testable in isolation.
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let categories: Vec<String> = args["categories"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if categories.is_empty() {
            return Ok(ToolResult::err(
                "'categories' must be a non-empty array. \
                 Example: [\"code\"] or [\"web\", \"data\"]",
            ));
        }

        // Validate against known categories
        const KNOWN: &[&str] = &[
            "filesystem", "web", "code", "data", "memory",
            "infra", "security", "automation", "integration", "communication",
        ];
        let unknown: Vec<&str> = categories
            .iter()
            .filter(|c| !KNOWN.contains(&c.as_str()))
            .map(String::as_str)
            .collect();

        if !unknown.is_empty() {
            return Ok(ToolResult::err(format!(
                "unknown tool categories: {}. Known: {}",
                unknown.join(", "),
                KNOWN.join(", "),
            )));
        }

        Ok(ToolResult::ok(serde_json::json!({
            "status":               "expanding",
            "requested_categories": categories,
            "note":                 "Executor will inject tools from these categories and re-invoke the LLM.",
        })))
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_categories_returns_error() {
        let tool = RequestMoreToolsTool;
        let result = tool.execute(serde_json::json!({ "categories": [] })).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn test_valid_category_returns_ok() {
        let tool = RequestMoreToolsTool;
        let result = tool.execute(serde_json::json!({ "categories": ["code"] })).await.unwrap();
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_unknown_category_returns_error() {
        let tool = RequestMoreToolsTool;
        let result = tool
            .execute(serde_json::json!({ "categories": ["connector/crm"] }))
            .await
            .unwrap();
        // connector/ categories are handled by list_connectors_in_category, not here
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_multiple_valid_categories() {
        let tool = RequestMoreToolsTool;
        let result = tool
            .execute(serde_json::json!({ "categories": ["web", "data"], "reason": "need scraping" }))
            .await
            .unwrap();
        assert!(result.success);
    }

    #[test]
    fn test_tool_name_is_constant() {
        let tool = RequestMoreToolsTool;
        assert_eq!(tool.name(), TOOL_NAME);
        assert_eq!(TOOL_NAME, "request_more_tools");
    }

    #[test]
    fn test_category_is_meta() {
        let tool = RequestMoreToolsTool;
        assert_eq!(tool.category(), "meta");
    }
}
