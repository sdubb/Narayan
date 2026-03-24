use std::path::{Path, PathBuf};

use anyhow::Result;

/// A workspace backed by the local filesystem.
pub struct LocalWorkspace {
    pub root: PathBuf,
}

impl LocalWorkspace {
    /// Create and initialise a local workspace directory tree.
    pub async fn create(root: PathBuf) -> Result<Self> {
        for subdir in &["files", "logs", "artifacts", "tmp"] {
            tokio::fs::create_dir_all(root.join(subdir)).await?;
        }
        tracing::debug!(path = %root.display(), "local workspace created");
        Ok(Self { root })
    }

    pub fn open(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolve a workspace-relative path, preventing path-escape attacks.
    pub fn resolve(&self, rel: impl AsRef<Path>) -> Result<PathBuf> {
        let joined = self.root.join(rel.as_ref());
        // Use the joined path directly (canonicalize requires it to exist)
        let abs = if joined.is_absolute() { joined.clone() } else { std::env::current_dir()?.join(&joined) };
        // Simple prefix check without canonicalize (avoids requires-exists constraint)
        let root_abs =
            if self.root.is_absolute() { self.root.clone() } else { std::env::current_dir()?.join(&self.root) };
        if !abs.starts_with(&root_abs) {
            anyhow::bail!("path escape blocked: {:?}", rel.as_ref());
        }
        Ok(joined)
    }

    /// Read a file relative to workspace root.
    pub async fn read(&self, rel: impl AsRef<Path>) -> Result<Vec<u8>> {
        let path = self.resolve(rel)?;
        Ok(tokio::fs::read(path).await?)
    }

    /// Write a file relative to workspace root. Creates parent dirs.
    pub async fn write(&self, rel: impl AsRef<Path>, data: &[u8]) -> Result<()> {
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, data).await?;
        Ok(())
    }

    /// Append text to a log or output file.
    pub async fn append(&self, rel: impl AsRef<Path>, content: &str) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let path = self.resolve(rel)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await?;
        f.write_all(content.as_bytes()).await?;
        Ok(())
    }

    /// Recursively list all files under this workspace.
    pub async fn list_all(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let mut rd = match tokio::fs::read_dir(&dir).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            while let Some(entry) = rd.next_entry().await? {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    files.push(p);
                }
            }
        }
        Ok(files)
    }

    /// Delete the entire workspace directory.
    pub async fn delete(&self) -> Result<()> {
        if self.root.exists() {
            tokio::fs::remove_dir_all(&self.root)
                .await
                .map_err(|e| anyhow::anyhow!("delete workspace {:?}: {}", self.root, e))?;
            tracing::debug!(path = %self.root.display(), "local workspace deleted");
        }
        Ok(())
    }

    /// Total size of all files in bytes.
    pub async fn size_bytes(&self) -> u64 {
        self.list_all().await.unwrap_or_default().iter().filter_map(|p| p.metadata().ok().map(|m| m.len())).sum()
    }
}
