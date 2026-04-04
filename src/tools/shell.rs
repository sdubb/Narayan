use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use crate::tools::{ParameterSchema, Tool, ToolResult};

const DEFAULT_TIMEOUT: u64 = 60;
const MAX_TIMEOUT: u64 = 600;
const MAX_OUTPUT: usize = 1_048_576;
const SAFE_ENV: &[&str] = &["HOME", "LANG", "LC_ALL", "LC_CTYPE", "PATH", "SHELL", "TERM", "TMPDIR", "USER"];

pub struct ShellTool {
    pub workspace: Option<String>,
}
impl ShellTool {
    pub fn new() -> Self {
        Self { workspace: None }
    }
}
impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> &str {
        "Execute a shell command in the agent workspace with an isolated environment."
    }
    fn input_contract(&self) -> Option<String> {
        Some(
            "{ command, cwd?, timeout_secs? }. command is required. cwd is relative to the workspace root. timeout_secs defaults to 60 and is capped at 600.".into(),
        )
    }
    fn output_contract(&self) -> Option<String> {
        Some("{ stdout, stderr, exit_code }. Non-zero exits return success=false with stderr in error.".into())
    }
    fn when_to_use(&self) -> Option<String> {
        Some(
            "Use for small workspace-local shell actions, repo inspection, and build/test commands that need a shell."
                .into(),
        )
    }
    fn when_not_to_use(&self) -> Option<String> {
        Some("Do not use for destructive system-wide commands, long-running daemons, or tasks better expressed as structured code or data transforms.".into())
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("command", "string", "Shell command to execute."),
            ParameterSchema::optional("cwd", "string", "Working directory relative to workspace root."),
            ParameterSchema::optional("timeout_secs", "integer", "Timeout seconds (max 600)."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let cmd = match args["command"].as_str().filter(|s| !s.trim().is_empty()) {
            Some(c) => c.to_string(),
            None => return Ok(ToolResult::err("'command' is required")),
        };
        if is_blocked(&cmd) {
            return Ok(ToolResult::err(format!(
                "Command blocked by safety policy: '{}'",
                cmd.split_whitespace().next().unwrap_or("?")
            )));
        }
        let timeout = args["timeout_secs"].as_u64().unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
        let cwd = resolve_cwd(self.workspace.as_deref(), args["cwd"].as_str());
        let mut c = Command::new("sh");
        c.arg("-c").arg(&cmd).current_dir(&cwd).env_clear();
        for v in SAFE_ENV {
            if let Ok(val) = std::env::var(v) {
                c.env(v, val);
            }
        }
        match tokio::time::timeout(Duration::from_secs(timeout), c.output()).await {
            Ok(Ok(out)) => {
                let mut stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                cap(&mut stdout, MAX_OUTPUT, "stdout");
                cap(&mut stderr, MAX_OUTPUT, "stderr");
                let code = out.status.code().unwrap_or(-1);
                let ok = out.status.success();
                let payload = serde_json::json!({"stdout": stdout, "stderr": stderr, "exit_code": code});
                if ok {
                    Ok(ToolResult::ok(payload))
                } else {
                    Ok(ToolResult { success: false, output: payload, error: Some(stderr.trim().to_string()) })
                }
            }
            Ok(Err(e)) => Ok(ToolResult::err(format!("spawn failed: {e}"))),
            Err(_) => Ok(ToolResult::err(format!("timed out after {timeout}s"))),
        }
    }
}

fn resolve_cwd(workspace: Option<&str>, rel: Option<&str>) -> std::path::PathBuf {
    let base = workspace.map(std::path::PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    if let Some(r) = rel {
        base.join(r)
    } else {
        base
    }
}

fn is_blocked(cmd: &str) -> bool {
    let l = cmd.to_lowercase();
    ["rm -rf /", "rm -rf /*", "mkfs", "dd if=/dev/zero of=/dev", ":(){ :|:& };:", "chmod -r 777 /"]
        .iter()
        .any(|p| l.contains(p))
}

fn cap(s: &mut String, max: usize, label: &str) {
    if s.len() > max {
        let b = (0..=max).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
        s.truncate(b);
        s.push_str(&format!("\n[{label} truncated at 1 MiB]"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;

    #[test]
    fn test_resolve_cwd_uses_workspace_and_relative_directory() {
        let cwd = resolve_cwd(Some("/tmp/workspace"), Some("src"));
        assert!(cwd.ends_with("workspace/src"));
    }

    #[test]
    fn test_is_blocked_catches_destructive_commands() {
        assert!(is_blocked("rm -rf /"));
        assert!(is_blocked("echo test && chmod -R 777 /"));
        assert!(!is_blocked("printf 'safe'"));
    }

    #[test]
    fn test_cap_preserves_char_boundaries_and_marks_truncation() {
        let mut text = "hello🙂world".to_string();
        cap(&mut text, 7, "stdout");
        assert!(text.starts_with("hello"));
        assert!(text.contains("[stdout truncated at 1 MiB]"));
    }

    #[tokio::test]
    async fn test_execute_requires_non_empty_command() {
        let tool = ShellTool::new();
        let result = tool.execute(serde_json::json!({ "command": "" })).await.expect("tool should return result");

        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("'command' is required"));
    }

    #[tokio::test]
    async fn test_execute_blocks_unsafe_commands() {
        let tool = ShellTool::new();
        let result =
            tool.execute(serde_json::json!({ "command": "rm -rf /" })).await.expect("tool should return result");

        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or_default().contains("Command blocked by safety policy"));
    }

    #[tokio::test]
    async fn test_execute_runs_command_and_captures_stdout() {
        let tool = ShellTool::new();
        let result = tool
            .execute(serde_json::json!({ "command": "printf 'narayan-shell-test'" }))
            .await
            .expect("tool should execute");

        assert!(result.success);
        assert_eq!(result.output["stdout"], "narayan-shell-test");
        assert_eq!(result.output["exit_code"], 0);
    }

    #[tokio::test]
    async fn test_execute_reports_non_zero_exit_code_and_stderr() {
        let tool = ShellTool::new();
        let result = tool
            .execute(serde_json::json!({ "command": "printf 'boom' 1>&2; exit 7" }))
            .await
            .expect("tool should execute");

        assert!(!result.success);
        assert_eq!(result.output["exit_code"], 7);
        assert_eq!(result.output["stderr"], "boom");
        assert_eq!(result.error.as_deref(), Some("boom"));
    }
}
