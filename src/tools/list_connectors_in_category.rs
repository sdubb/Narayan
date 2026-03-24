//! `list_connectors_in_category` — returns names + summaries for a connector category.
//!
//! This is the first step in the lazy connector discovery protocol:
//!
//!   1. Planner puts connector_category in the step (e.g. "crm")
//!   2. Executor injects this tool + the category's connectors summaries
//!   3. LLM calls list_connectors_in_category to see what's available
//!   4. LLM picks one → executor injects its full ToolSpec → LLM executes
//!
//! The tool returns both built-in connectors and tenant-specific custom connectors
//! so the LLM sees the full picture in one call.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct ListConnectorsInCategoryTool;

#[async_trait]
impl Tool for ListConnectorsInCategoryTool {
    fn name(&self) -> &str {
        "list_connectors_in_category"
    }

    fn description(&self) -> &str {
        "List all available connectors in a given category with their names and summaries. \
         Call this when you know the type of integration you need (e.g. 'crm', 'communication') \
         but haven't yet picked a specific connector. After reviewing the list, \
         use the connector's exact name as a tool to execute operations on it."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![ParameterSchema::required(
            "category",
            "string",
            "Connector category to list. Examples: crm, devtools, project_management, \
                 communication, finance, itsm, hr, legal, data. \
                 Use 'all' to see every available connector.",
        )]
    }

    /// In production the executor intercepts this call and returns the real list.
    /// This fallback runs in unit tests.
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let category = args["category"].as_str().unwrap_or("all");
        Ok(ToolResult::ok(serde_json::json!({
            "category": category,
            "connectors": [],
            "note": "Executor should intercept this call and return real connector list.",
        })))
    }
}
