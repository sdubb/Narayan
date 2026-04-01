use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use tokio::process::Command;

use crate::tools::{ParameterSchema, Tool, ToolResult};

fn optional_string(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key).and_then(|value| value.as_str()).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn require_explicit_user_request(args: &serde_json::Value) -> Result<(), String> {
    if args.get("explicit_user_request").and_then(|value| value.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err("worktree tools are explicit-use only; set explicit_user_request=true only when the user specifically asked for worktree isolation".into())
    }
}

fn sanitize_label(raw: &str) -> String {
    let collapsed = raw
        .trim()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '-' })
        .collect::<String>();
    let mut out = String::with_capacity(collapsed.len());
    let mut last_dash = false;
    for ch in collapsed.chars() {
        if ch == '-' {
            if !last_dash {
                out.push(ch);
            }
            last_dash = true;
        } else {
            out.push(ch);
            last_dash = false;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() { "isolated".into() } else { trimmed[..trimmed.len().min(48)].to_string() }
}

fn resolve_path(path: &str, workspace_path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() { candidate.to_path_buf() } else { Path::new(workspace_path).join(candidate) }
}

fn canonicalish(path: &Path) -> anyhow::Result<PathBuf> {
    if path.exists() {
        Ok(path.canonicalize()?)
    } else if let Some(parent) = path.parent() {
        Ok(parent.canonicalize()?.join(path.file_name().unwrap_or_default()))
    } else {
        Ok(path.to_path_buf())
    }
}

fn path_within(base: &Path, candidate: &Path) -> anyhow::Result<bool> {
    Ok(canonicalish(candidate)?.starts_with(canonicalish(base)?))
}

async fn run_git(repo_path: &Path, args: &[&str]) -> anyhow::Result<ToolResult> {
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new("git").current_dir(repo_path).args(args).output(),
    )
    .await;

    match output {
        Ok(Ok(result)) => {
            let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
            let payload =
                serde_json::json!({"stdout": stdout, "stderr": stderr, "exit_code": result.status.code().unwrap_or(-1)});
            if result.status.success() {
                Ok(ToolResult::ok(payload))
            } else {
                Ok(ToolResult { success: false, output: payload, error: Some(stderr.trim().to_string()) })
            }
        }
        Ok(Err(error)) => Ok(ToolResult::err(format!("git spawn failed: {}", error))),
        Err(_) => Ok(ToolResult::err("git command timed out after 120s")),
    }
}

pub struct EnterWorktreeTool;

#[async_trait]
impl Tool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "enter_worktree"
    }

    fn description(&self) -> &str {
        "Create an isolated git worktree for explicit user-requested worktree flows. Do not use as a generic branch shortcut."
    }

    fn category(&self) -> &'static str {
        "code"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("explicit_user_request", "boolean", "Must be true only when the user explicitly requested a worktree."),
            ParameterSchema::optional("repo_path", "string", "Workspace-relative or absolute git repository path. Defaults to the workspace root."),
            ParameterSchema::optional("branch_name", "string", "Optional branch name for the isolated worktree."),
            ParameterSchema::optional("name", "string", "Short label for the worktree path."),
            ParameterSchema::required("workspace_path", "string", "Injected automatically."),
            ParameterSchema::required("agent_id", "string", "Injected automatically."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if let Err(message) = require_explicit_user_request(&args) {
            return Ok(ToolResult::err(message));
        }
        let workspace_path = optional_string(&args, "workspace_path").ok_or_else(|| anyhow::anyhow!("workspace_path is required"))?;
        let agent_id = optional_string(&args, "agent_id").ok_or_else(|| anyhow::anyhow!("agent_id is required"))?;
        let workspace_root = PathBuf::from(&workspace_path);
        if !workspace_root.exists() {
            return Ok(ToolResult::err("worktree tools require a local workspace path"));
        }

        let repo_path = resolve_path(optional_string(&args, "repo_path").as_deref().unwrap_or("."), &workspace_path);
        if !path_within(&workspace_root, &repo_path)? {
            return Ok(ToolResult::err("repo_path must stay inside the current workspace"));
        }
        let repo_root = repo_path.canonicalize().map_err(|error| anyhow::anyhow!("resolve repo path: {}", error))?;
        let verify_repo = run_git(&repo_root, &["rev-parse", "--show-toplevel"]).await?;
        if !verify_repo.success {
            return Ok(ToolResult::err("enter_worktree requires a git repository inside the current workspace"));
        }

        let branch_name = optional_string(&args, "branch_name").unwrap_or_else(|| {
            format!("narayan-{}-{}", sanitize_label(&agent_id), sanitize_label(optional_string(&args, "name").as_deref().unwrap_or("isolated")))
        });
        let worktree_label = sanitize_label(optional_string(&args, "name").as_deref().unwrap_or(&branch_name));
        let worktree_root = workspace_root
            .parent()
            .unwrap_or(&workspace_root)
            .join(".narayan_worktrees")
            .join(sanitize_label(&agent_id));
        tokio::fs::create_dir_all(&worktree_root).await?;
        let worktree_path = worktree_root.join(worktree_label);
        if worktree_path.exists() {
            return Ok(ToolResult::err(format!(
                "worktree path '{}' already exists",
                worktree_path.display()
            )));
        }

        let worktree_path_string = worktree_path.display().to_string();
        let result = run_git(&repo_root, &["worktree", "add", "-b", &branch_name, &worktree_path_string]).await?;
        if !result.success {
            return Ok(result);
        }

        Ok(ToolResult::ok(serde_json::json!({
            "entered": true,
            "repo_path": repo_root.display().to_string(),
            "worktree_path": worktree_path_string,
            "branch_name": branch_name,
            "explicit_use_only": true,
        })))
    }
}

pub struct ExitWorktreeTool;

#[async_trait]
impl Tool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "exit_worktree"
    }

    fn description(&self) -> &str {
        "Remove an explicitly created isolated git worktree after use."
    }

    fn category(&self) -> &'static str {
        "code"
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("explicit_user_request", "boolean", "Must be true only when the user explicitly requested worktree isolation."),
            ParameterSchema::required("worktree_path", "string", "Worktree path returned by enter_worktree."),
            ParameterSchema::required("workspace_path", "string", "Injected automatically."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if let Err(message) = require_explicit_user_request(&args) {
            return Ok(ToolResult::err(message));
        }
        let workspace_path = optional_string(&args, "workspace_path").ok_or_else(|| anyhow::anyhow!("workspace_path is required"))?;
        let workspace_root = PathBuf::from(&workspace_path);
        let worktree_path = resolve_path(
            optional_string(&args, "worktree_path").ok_or_else(|| anyhow::anyhow!("worktree_path is required"))?.as_str(),
            &workspace_path,
        );
        if !path_within(workspace_root.parent().unwrap_or(&workspace_root), &worktree_path)? {
            return Ok(ToolResult::err("worktree_path must stay inside the workspace's managed worktree area"));
        }
        if !worktree_path.exists() {
            return Ok(ToolResult::err(format!("worktree '{}' does not exist", worktree_path.display())));
        }

        let git_common = run_git(&worktree_path, &["rev-parse", "--git-common-dir"]).await?;
        if !git_common.success {
            return Ok(ToolResult::err("exit_worktree requires a valid git worktree path"));
        }
        let common_dir_raw =
            git_common.output.get("stdout").and_then(|value| value.as_str()).unwrap_or_default().trim().to_string();
        let common_dir = {
            let candidate = PathBuf::from(&common_dir_raw);
            if candidate.is_absolute() { candidate } else { worktree_path.join(candidate) }
        };
        let common_dir = common_dir.canonicalize().unwrap_or(common_dir);
        let repo_root = common_dir.parent().ok_or_else(|| anyhow::anyhow!("unable to resolve main repository root"))?;
        let worktree_path_string = worktree_path.display().to_string();
        let result = run_git(repo_root, &["worktree", "remove", "--force", &worktree_path_string]).await?;
        if !result.success {
            return Ok(result);
        }

        Ok(ToolResult::ok(serde_json::json!({
            "removed": true,
            "worktree_path": worktree_path_string,
            "explicit_use_only": true,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_requires_explicit_user_request() {
        let error = require_explicit_user_request(&serde_json::json!({})).expect_err("missing flag should fail");
        assert!(error.contains("explicit-use only"));
    }

    #[test]
    fn sanitize_label_collapses_noise() {
        assert_eq!(sanitize_label(" Feature / Branch "), "feature-branch");
    }
}
