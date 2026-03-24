//! `create_custom_connector` — creates a tenant-specific connector on the fly.
//!
//! Called when the LLM discovers the built-in connectors don't cover a needed
//! integration. Three creation paths:
//!
//!   KnownSaas   — product name provided, LLM fills endpoints from training knowledge
//!   OpenApiSpec — user uploads an OpenAPI/Swagger JSON or YAML spec
//!   Manual      — user provides base URL + endpoint descriptions
//!
//! The executor intercepts this call, saves the TenantConnector to DB,
//! and immediately injects the new connector as a live ToolSpec for the
//! current step — no restart required.
//!
//! The connector persists permanently for the tenant and appears in their
//! connector directory for all future agents.

use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct CreateCustomConnectorTool;

#[async_trait]
impl Tool for CreateCustomConnectorTool {
    fn name(&self) -> &str {
        "create_custom_connector"
    }

    fn description(&self) -> &str {
        "Create a new custom connector for an API or service that isn't built-in. \
         The connector is saved permanently for your tenant and usable by all future agents. \
         Three creation paths: (1) known_saas — provide the product name and the system \
         fills in the API shape; (2) api_docs — provide the base URL and the raw API \
         documentation text or OpenAPI spec; (3) manual — provide the base URL and \
         describe each endpoint you need. After creation, the connector is immediately \
         available as a tool in this step."
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "name",
                "string",
                "Short identifier for this connector, e.g. 'acme_erp' or 'internal_wiki'. \
                 Used as the tool name. Lowercase letters, numbers, underscores only.",
            ),
            ParameterSchema::required(
                "category",
                "string",
                "Category this connector belongs to, e.g. 'erp', 'crm', 'internal'. \
                 Use an existing category if it fits, or create a new one.",
            ),
            ParameterSchema::required(
                "creation_path",
                "string",
                "How to create this connector: 'known_saas' | 'api_docs' | 'manual'.",
            ),
            ParameterSchema::optional(
                "product_name",
                "string",
                "For known_saas: the product name, e.g. 'Stripe', 'Zendesk', 'Shopify'.",
            ),
            ParameterSchema::optional(
                "base_url",
                "string",
                "Base URL for API calls, e.g. 'https://api.acme.com/v2'. \
                 Required for api_docs and manual paths.",
            ),
            ParameterSchema::optional(
                "auth_type",
                "string",
                "Authentication type: 'bearer' | 'api_key_header' | 'basic' | 'none'. \
                 Defaults to 'bearer'.",
            ),
            ParameterSchema::optional(
                "auth_header_name",
                "string",
                "For api_key_header auth: the header name, e.g. 'X-API-Key'.",
            ),
            ParameterSchema::optional(
                "auth_credential_key",
                "string",
                "Name of the stored credential that holds the token or API key. \
                 The user will be prompted to add this credential if not already set.",
            ),
            ParameterSchema::optional(
                "api_docs",
                "string",
                "For api_docs path: raw API documentation text, OpenAPI JSON/YAML, \
                 or a URL to fetch documentation from.",
            ),
            ParameterSchema::optional(
                "endpoints",
                "array",
                "For manual path: array of endpoint objects. Each object: \
                 { method, path, description, params: [{name, location, type, description, required}] }",
            ),
            ParameterSchema::optional(
                "summary",
                "string",
                "One-line summary shown in the connector directory, \
                 e.g. 'Acme ERP: query orders, update inventory, manage customers'.",
            ),
        ]
    }

    /// Fallback — executor intercepts in production, parses docs, saves to DB.
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let name          = args["name"].as_str().unwrap_or("");
        let category      = args["category"].as_str().unwrap_or("");
        let creation_path = args["creation_path"].as_str().unwrap_or("manual");

        if name.is_empty() {
            return Ok(ToolResult::err("'name' is required"));
        }
        if category.is_empty() {
            return Ok(ToolResult::err("'category' is required"));
        }

        // Validate name format
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Ok(ToolResult::err(
                "connector name must contain only lowercase letters, numbers, and underscores",
            ));
        }

        Ok(ToolResult::ok(serde_json::json!({
            "status":        "pending",
            "name":          name,
            "category":      category,
            "creation_path": creation_path,
            "note":          "Executor should intercept this call, parse docs, save to DB, and inject live ToolSpec.",
        })))
    }
}
