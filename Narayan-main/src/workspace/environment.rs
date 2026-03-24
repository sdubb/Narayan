use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Runtime environment variables and credentials available inside a workspace.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceEnvironment {
    pub vars: HashMap<String, String>,
    pub credentials: HashMap<String, String>,
}

impl WorkspaceEnvironment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn set_credential(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.credentials.insert(key.into(), value.into());
    }

    pub fn get_credential(&self, key: &str) -> Option<&str> {
        self.credentials.get(key).map(String::as_str)
    }

    /// Merge another environment into this one (other takes precedence).
    pub fn merge(&mut self, other: WorkspaceEnvironment) {
        self.vars.extend(other.vars);
        self.credentials.extend(other.credentials);
    }

    /// Persist the environment to a JSON file.
    pub async fn save(&self, path: &std::path::Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    /// Load the environment from a JSON file.
    pub async fn load(path: &std::path::Path) -> Result<Self> {
        let content = tokio::fs::read_to_string(path).await?;
        Ok(serde_json::from_str(&content)?)
    }
}
