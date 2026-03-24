use std::path::{Path, PathBuf};

use anyhow::Result;

/// File-system operations scoped to an agent workspace directory.
pub struct WorkspaceFs {
    root: PathBuf,
}

impl WorkspaceFs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a relative path inside the workspace (prevents escapes).
    pub fn resolve(&self, rel: impl AsRef<Path>) -> Result<PathBuf> {
        let joined = self.root.join(rel.as_ref());
        let canonical = joined.canonicalize().unwrap_or(joined.clone());
        if !canonical.starts_with(&self.root) {
            anyhow::bail!("path escape detected: {:?}", rel.as_ref());
        }
        Ok(joined)
    }

    pub async fn read_file(&self, rel: impl AsRef<Path>) -> Result<String> {
        let path = self.resolve(rel)?;
        Ok(tokio::fs::read_to_string(path).await?)
    }

    pub async fn write_file(&self, rel: impl AsRef<Path>, content: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    pub async fn append_file(&self, rel: impl AsRef<Path>, content: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await?;
        file.write_all(content.as_bytes()).await?;
        Ok(())
    }

    pub async fn list_dir(&self, rel: impl AsRef<Path>) -> Result<Vec<String>> {
        let path = self.resolve(rel)?;
        let mut entries = tokio::fs::read_dir(path).await?;
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        Ok(names)
    }

    pub async fn ensure_dirs(&self) -> Result<()> {
        for subdir in &["files", "logs", "artifacts"] {
            tokio::fs::create_dir_all(self.root.join(subdir)).await?;
        }
        Ok(())
    }
}
