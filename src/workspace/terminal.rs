use std::path::PathBuf;

use anyhow::Result;
use tokio::process::Command;

/// Executes shell commands inside a workspace directory.
pub struct WorkspaceTerminal {
    cwd: PathBuf,
}

#[derive(Debug)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl WorkspaceTerminal {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Run a shell command, capturing stdout and stderr.
    pub async fn run(&self, cmd: &str) -> Result<CommandOutput> {
        let output = Command::new("sh").arg("-c").arg(cmd).current_dir(&self.cwd).output().await?;

        Ok(CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}
