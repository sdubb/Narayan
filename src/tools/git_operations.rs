use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use crate::tools::{ParameterSchema, Tool, ToolResult, schema_string, schema_integer};

pub struct GitOperationsTool;

#[async_trait]
impl Tool for GitOperationsTool {
    fn name(&self) -> &str {
        "git_operations"
    }
    fn description(&self) -> &str {
        "Perform Git operations: clone, status, add, commit, push, pull, branch, diff, log. \
         Runs standard git commands inside the workspace."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "operation",
                "string",
                "Git operation: clone|status|add|commit|push|pull|branch|checkout|diff|log|init",
            ),
            ParameterSchema::optional("repo_url", "string", "Repository URL (for clone)."),
            ParameterSchema::optional("path", "string", "Local repo path (default: current dir)."),
            ParameterSchema::optional("message", "string", "Commit message (for commit)."),
            ParameterSchema::optional("branch", "string", "Branch name (for checkout/branch)."),
            ParameterSchema::optional("remote", "string", "Remote name (default: origin)."),
            ParameterSchema::optional("args", "string", "Extra args passed directly to git."),
        ]
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "required": ["stdout", "stderr", "exit_code"],
            "properties": {
                "stdout": schema_string(),
                "stderr": schema_string(),
                "exit_code": schema_integer(),
            },
            "additionalProperties": true,
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let op = match args["operation"].as_str() {
            Some(o) => o,
            None => return Ok(ToolResult::err("'operation' is required")),
        };
        let path = args["path"].as_str().unwrap_or(".");
        let remote = args["remote"].as_str().unwrap_or("origin");
        let branch = args["branch"].as_str().unwrap_or("main");
        let message = args["message"].as_str().unwrap_or("");
        let extra = args["args"].as_str().unwrap_or("");

        let git_cmd: String = match op {
            "clone" => {
                let url = match args["repo_url"].as_str() {
                    Some(u) => u,
                    None => return Ok(ToolResult::err("'repo_url' is required for clone")),
                };
                format!("git clone {url} {path} {extra}")
            }
            "status" => format!("git -C {path} status {extra}"),
            "add" => format!("git -C {path} add {extra} ."),
            "commit" => {
                if message.is_empty() {
                    return Ok(ToolResult::err("'message' is required for commit"));
                }
                format!("git -C {path} commit -m {msg} {extra}", msg = shell_quote(message))
            }
            "push" => format!("git -C {path} push {remote} {branch} {extra}"),
            "pull" => format!("git -C {path} pull {remote} {branch} {extra}"),
            "branch" => format!("git -C {path} branch {extra}"),
            "checkout" => format!("git -C {path} checkout {branch} {extra}"),
            "diff" => format!("git -C {path} diff {extra}"),
            "log" => format!("git -C {path} log --oneline -20 {extra}"),
            "init" => format!("git init {path} {extra}"),
            other => return Ok(ToolResult::err(format!("Unknown git operation: '{other}'"))),
        };

        run_git(&git_cmd).await
    }
}

async fn run_git(cmd: &str) -> anyhow::Result<ToolResult> {
    let out = tokio::time::timeout(Duration::from_secs(120), Command::new("sh").arg("-c").arg(cmd).output()).await;

    match out {
        Ok(Ok(o)) => {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
            let ok = o.status.success();
            let payload =
                serde_json::json!({"stdout": stdout, "stderr": stderr, "exit_code": o.status.code().unwrap_or(-1)});
            if ok {
                Ok(ToolResult::ok(payload))
            } else {
                Ok(ToolResult { success: false, output: payload, error: Some(stderr.trim().to_string()) })
            }
        }
        Ok(Err(e)) => Ok(ToolResult::err(format!("git spawn failed: {e}"))),
        Err(_) => Ok(ToolResult::err("git command timed out after 120s")),
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
