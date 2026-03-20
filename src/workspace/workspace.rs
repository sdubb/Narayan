use std::path::PathBuf;

use anyhow::Result;

use crate::workspace::{environment::WorkspaceEnvironment, filesystem::WorkspaceFs, terminal::WorkspaceTerminal};

/// Unified workspace handle for a single agent.
pub struct Workspace {
    pub agent_id: String,
    pub fs: WorkspaceFs,
    pub terminal: WorkspaceTerminal,
    pub env: WorkspaceEnvironment,
}

impl Workspace {
    /// Create a new workspace rooted at `base_dir/agents/<agent_id>`.
    pub async fn create(base_dir: impl Into<PathBuf>, agent_id: String) -> Result<Self> {
        let root: PathBuf = base_dir.into().join("agents").join(&agent_id);
        tokio::fs::create_dir_all(&root).await?;

        let fs = WorkspaceFs::new(root.clone());
        fs.ensure_dirs().await?;

        let terminal = WorkspaceTerminal::new(root.clone());
        let env = WorkspaceEnvironment::new();

        Ok(Self { agent_id, fs, terminal, env })
    }

    /// Open an existing workspace.
    pub fn open(base_dir: impl Into<PathBuf>, agent_id: String) -> Self {
        let root: PathBuf = base_dir.into().join("agents").join(&agent_id);
        Self {
            fs: WorkspaceFs::new(root.clone()),
            terminal: WorkspaceTerminal::new(root),
            env: WorkspaceEnvironment::new(),
            agent_id,
        }
    }

    pub fn root_path(&self) -> &std::path::Path {
        self.fs.root()
    }
}
