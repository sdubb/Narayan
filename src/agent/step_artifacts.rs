//! Per-step artifact output files.
//!
//! Instead of stuffing full step outputs into `state.metadata["step_outputs"]`
//! (a single JSONB array that can grow to 100KB+), each step writes its full
//! output to a dedicated file on disk. Only a compact pointer (~200 bytes)
//! goes into the metadata JSONB column.
//!
//! Layout:
//!   {workspace}/.narayan/artifacts/{agent_id}/step-{index}.json
//!
//! The full output remains accessible for template resolution and data flow,
//! while the metadata column stays lightweight.

use std::path::{Path, PathBuf};

/// Directory for all artifacts belonging to a specific agent.
pub fn agent_artifact_dir(workspace: &Path, agent_id: &str) -> PathBuf {
    workspace.join(".narayan").join("artifacts").join(agent_id)
}

/// Generates the artifact path for a specific step's output.
/// Format: `{workspace}/.narayan/artifacts/{agent_id}/step-{index}.json`
pub fn step_artifact_path(workspace: &Path, agent_id: &str, step_index: usize) -> PathBuf {
    agent_artifact_dir(workspace, agent_id).join(format!("step-{}.json", step_index))
}

/// Write step output to its artifact file. Creates directories if needed.
/// Returns the path written on success.
pub async fn write_step_artifact(
    workspace: &Path,
    agent_id: &str,
    step_index: usize,
    output: &serde_json::Value,
) -> Result<PathBuf, std::io::Error> {
    let dir = agent_artifact_dir(workspace, agent_id);
    tokio::fs::create_dir_all(&dir).await?;

    let path = step_artifact_path(workspace, agent_id, step_index);
    let bytes =
        serde_json::to_vec_pretty(output).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(&path, &bytes).await?;
    Ok(path)
}

/// Read a step's full output from its artifact file.
pub async fn read_step_artifact(
    workspace: &Path,
    agent_id: &str,
    step_index: usize,
) -> Result<serde_json::Value, anyhow::Error> {
    let path = step_artifact_path(workspace, agent_id, step_index);
    let bytes = tokio::fs::read(&path).await?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    Ok(value)
}

/// Build a compact pointer for storage in `metadata["step_outputs"]`.
/// Contains only summary-level information — the full output is on disk.
pub fn compact_step_pointer(
    step_index: usize,
    description: &str,
    success: bool,
    output_summary: &str,
    artifact_path: &Path,
    tools_called: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "step_index": step_index,
        "description": description,
        "success": success,
        "summary": crate::agent::prompts::truncate(output_summary, 200),
        "artifact_path": artifact_path.to_string_lossy(),
        "tools_called": tools_called,
    })
}

/// Cleanup artifacts for a completed or cancelled agent.
/// Best-effort — doesn't fail if directory doesn't exist.
pub async fn cleanup_agent_artifacts(workspace: &Path, agent_id: &str) {
    let dir = agent_artifact_dir(workspace, agent_id);
    if dir.exists() {
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_step_artifact_path_format() {
        let workspace = Path::new("/workspace");
        let path = step_artifact_path(workspace, "agent-123", 5);
        assert_eq!(path, PathBuf::from("/workspace/.narayan/artifacts/agent-123/step-5.json"));
    }

    #[test]
    fn test_step_artifact_path_index_zero() {
        let workspace = Path::new("/workspace");
        let path = step_artifact_path(workspace, "agent-abc", 0);
        assert!(path.to_string_lossy().contains("step-0.json"));
    }

    #[tokio::test]
    async fn test_write_and_read_artifact() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let agent_id = "test-agent";
        let step_index = 2;
        let output = serde_json::json!({
            "result": "enriched data",
            "records_processed": 42
        });

        // Write
        let path = write_step_artifact(workspace, agent_id, step_index, &output).await.expect("write should succeed");
        assert!(path.exists());

        // Read back
        let loaded = read_step_artifact(workspace, agent_id, step_index).await.expect("read should succeed");
        assert_eq!(loaded, output);
    }

    #[tokio::test]
    async fn test_read_missing_artifact() {
        let tmp = TempDir::new().unwrap();
        let result = read_step_artifact(tmp.path(), "no-agent", 99).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_compact_pointer_structure() {
        let pointer = compact_step_pointer(
            3,
            "Enrich lead data",
            true,
            "Successfully enriched 15 leads with company data from Apollo",
            Path::new("/workspace/.narayan/artifacts/a1/step-3.json"),
            &["web_search".to_string(), "data_engine".to_string()],
        );

        assert_eq!(pointer["step_index"], 3);
        assert_eq!(pointer["success"], true);
        assert!(pointer["summary"].as_str().unwrap().len() <= 200);
        assert_eq!(pointer["tools_called"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_cleanup_agent_artifacts() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let agent_id = "cleanup-test";

        // Create an artifact
        let output = serde_json::json!({"test": true});
        write_step_artifact(workspace, agent_id, 0, &output).await.unwrap();

        // Verify it exists
        let dir = agent_artifact_dir(workspace, agent_id);
        assert!(dir.exists());

        // Cleanup
        cleanup_agent_artifacts(workspace, agent_id).await;
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn test_cleanup_nonexistent_is_noop() {
        let tmp = TempDir::new().unwrap();
        // Should not panic
        cleanup_agent_artifacts(tmp.path(), "nonexistent-agent").await;
    }
}
