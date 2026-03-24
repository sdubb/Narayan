//! `request_more_connectors` — signals that available connectors don't satisfy the need.
//!
//! Called when the LLM has reviewed the connectors in a category and none of them
//! provide the required capability. The executor responds with:
//!
//!   - More connectors if any remain un-shown in that category
//!   - The custom connector option if the category is exhausted
//!   - A clear message if nothing at all is available
//!
//! This tool is always available during connector steps so the LLM is never stuck.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct RequestMoreConnectorsTool;

#[async_trait]
impl Tool for RequestMoreConnectorsTool {
    fn name(&self) -> &str {
        "request_more_connectors"
    }

    fn description(&self) -> &str {
        "Signal that none of the connectors shown so far satisfy your requirement. \
         Provide the category and reason — the system will either show more connectors \
         in that category or offer the option to add a custom connector. \
         Use this instead of guessing with api_call or http_request."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "category",
                "string",
                "The connector category you need, e.g. 'crm', 'itsm', 'finance'.",
            ),
            ParameterSchema::required(
                "reason",
                "string",
                "Why the shown connectors don't work — helps the system find alternatives \
                 or guide the user to add a custom connector.",
            ),
            ParameterSchema::optional(
                "tried_connectors",
                "array",
                "Names of connectors you already checked and found insufficient.",
            ),
        ]
    }

    /// Fallback — executor intercepts in production.
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let category = args["category"].as_str().unwrap_or("");
        let reason   = args["reason"].as_str().unwrap_or("");
        Ok(ToolResult::ok(serde_json::json!({
            "status":   "no_more_connectors",
            "category": category,
            "reason":   reason,
            "options": [
                {
                    "action":      "create_custom_connector",
                    "description": "Add a custom connector by providing the API URL and credentials."
                }
            ],
            "note": "Executor should intercept this call and return real alternatives.",
        })))
    }
}
