use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};
pub struct AskUserTool;
#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }
    fn description(&self) -> &str {
        "Pause the agent and request input from the user. The agent enters 'clarifying' state. \
         Use this for plain questions, secret credentials, human approvals, or direct frontend card setup \
         like opening a database, connector, MCP, or API auth card before continuing. \
         Prefer this over inventing placeholders or retrying without new information."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::optional("question", "string", "Single question prompt to ask the user."),
            ParameterSchema::optional(
                "questions",
                "array",
                "Optional list of structured questions. Each item may include: id, type/question_type, prompt, placeholder, helper_text, options, multi_select, recommended, preview, required, secret, store_as_credential, connector_type, action_label, card_type, binding_target, resume_token.",
            ),
            ParameterSchema::optional("options", "array", "Optional list of answer options to present."),
            ParameterSchema::optional(
                "type",
                "string",
                "Question mode: clarification | approval | decision | mcq | multi_select | text | card_open | hybrid. Defaults to clarification.",
            ),
            ParameterSchema::optional(
                "multi_select",
                "boolean",
                "Whether multiple options may be selected.",
            ),
            ParameterSchema::optional(
                "recommended",
                "array",
                "Optional ordered list of recommended option values or labels.",
            ),
            ParameterSchema::optional(
                "preview",
                "object",
                "Optional preview payload for code/UI comparisons or examples.",
            ),
            ParameterSchema::optional(
                "required",
                "boolean",
                "Whether an answer is required before proceeding (default: true).",
            ),
            ParameterSchema::optional("placeholder", "string", "Optional input placeholder shown in the UI."),
            ParameterSchema::optional("helper_text", "string", "Short helper text explaining what to provide."),
            ParameterSchema::optional("secret", "boolean", "Whether the answer should be treated as a hidden secret."),
            ParameterSchema::optional(
                "store_as_credential",
                "string",
                "Credential key to store the answer under when secret=true or when the input should be reusable by later tools.",
            ),
            ParameterSchema::optional(
                "connector_type",
                "string",
                "Connector or account type the user should set up, e.g. 'gmail' or 'google'.",
            ),
            ParameterSchema::optional(
                "action_label",
                "string",
                "Optional call-to-action label for the frontend, e.g. 'Connect Gmail in Settings'.",
            ),
            ParameterSchema::optional(
                "card_type",
                "string",
                "Optional setup card type to open directly: database, connector, mcp, or api_auth.",
            ),
            ParameterSchema::optional(
                "required_fields",
                "array",
                "Required fields that the setup card should collect.",
            ),
            ParameterSchema::optional(
                "binding_target",
                "string",
                "The workflow binding target this card will resolve.",
            ),
            ParameterSchema::optional(
                "resume_token",
                "string",
                "Opaque token used to resume plan-mode compilation after card completion.",
            ),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let questions = if args.get("questions").and_then(|value| value.as_array()).is_some() {
            crate::agent::clarifier::parse_clarification_questions(&args["questions"])
        } else if let Some(question) = args["question"].as_str() {
            let options = args["options"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            vec![crate::agent::clarifier::ClarificationQuestion {
                id: args["store_as_credential"]
                    .as_str()
                    .or_else(|| args["connector_type"].as_str())
                    .map(str::to_string)
                    .unwrap_or_default(),
                question_type: args
                    .get("type")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .or_else(|| Some("clarification".into())),
                prompt: question.to_string(),
                placeholder: args["placeholder"].as_str().map(str::to_string),
                helper_text: args["helper_text"].as_str().map(str::to_string),
                options,
                multi_select: args["multi_select"].as_bool().unwrap_or(false),
                recommended: args["recommended"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect(),
                preview: args.get("preview").cloned(),
                required: args["required"].as_bool().unwrap_or(true),
                secret: args["secret"].as_bool().unwrap_or(false),
                store_as_credential: args["store_as_credential"].as_str().map(str::to_string),
                connector_type: args["connector_type"].as_str().map(str::to_string),
                action_label: args["action_label"].as_str().map(str::to_string),
                card_type: args["card_type"].as_str().map(str::to_string),
                required_fields: args["required_fields"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect(),
                binding_target: args["binding_target"].as_str().map(str::to_string),
                resume_token: args["resume_token"].as_str().map(str::to_string),
            }
            .normalized(0)]
        } else {
            return Ok(ToolResult::err("'question' or 'questions' required"));
        };

        if questions.is_empty() {
            return Ok(ToolResult::err("at least one question is required"));
        }

        // Store the pending question so the clarification API can surface it
        let pending = serde_json::json!({
            "questions": questions,
            "asked_at": chrono::Utc::now().to_rfc3339()
        });
        crate::tools::memory_store_internal::insert("ask_user:pending".into(), pending.to_string());
        Ok(ToolResult::ok(serde_json::json!({
            "status":    "awaiting_user_input",
            "questions": questions,
            "note":      "Agent is paused. User must respond via POST /agents/:id/clarify",
        })))
    }
}
