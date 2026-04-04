use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string};
pub struct ToolOutputTool;
#[async_trait]
impl Tool for ToolOutputTool {
    fn name(&self) -> &str {
        "tool_output"
    }
    fn description(&self) -> &str {
        "Format and summarize the output of a previous tool call into a human-readable report."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("tool_name", "string", "Name of the tool whose output to format."),
            ParameterSchema::required("output", "object", "The raw tool output JSON to format."),
            ParameterSchema::optional(
                "format",
                "string",
                "Output format: 'summary'|'table'|'json' (default: 'summary').",
            ),
        ]
    }


    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["formatted", "tool"],
            "properties": {
                "formatted": schema_string(),
                "tool": schema_string(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let tool_name = args["tool_name"].as_str().unwrap_or("unknown");
        let output = &args["output"];
        let format = args["format"].as_str().unwrap_or("summary");
        let formatted = match format {
            "json" => serde_json::to_string_pretty(output).unwrap_or_default(),
            "table" => json_to_table(output),
            _ => format!("Tool '{}' result: {}", tool_name, summarize(output)),
        };
        Ok(ToolResult::ok(serde_json::json!({"formatted": formatted, "tool": tool_name})))
    }
}
fn summarize(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(m) => m
            .iter()
            .take(5)
            .map(|(k, v)| format!("{}: {}", k, v.to_string().chars().take(80).collect::<String>()))
            .collect::<Vec<_>>()
            .join(", "),
        other => crate::util::truncate(&other.to_string(), 200).to_string(),
    }
}
fn json_to_table(v: &serde_json::Value) -> String {
    if let Some(arr) = v.as_array() {
        arr.iter()
            .enumerate()
            .map(|(i, item)| format!("[{}] {}", i + 1, summarize(item)))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        summarize(v)
    }
}
