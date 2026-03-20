use std::path::Path;

/// Workspace storage mode.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMode {
    /// Use local disk only.
    Local,
    /// Use object storage only.
    Remote,
    /// Prefer local; fall back to remote when disk is above threshold.
    Hybrid,
}

impl WorkspaceMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "remote" => Self::Remote,
            "local" => Self::Local,
            _ => Self::Hybrid,
        }
    }
}

/// Returns the current disk usage percentage for the filesystem containing `path`.
/// Returns 0.0 on error so we default to local when we can't measure.
pub fn disk_usage_pct(path: &Path) -> f64 {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let path_c = CString::new(path.to_string_lossy().as_bytes()).ok();
        if let Some(p) = path_c {
            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            if unsafe { libc::statvfs(p.as_ptr(), &mut stat) } == 0 && stat.f_blocks > 0 {
                let used = stat.f_blocks - stat.f_bfree;
                return (used as f64 / stat.f_blocks as f64) * 100.0;
            }
        }
        0.0
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

/// Choose the effective workspace mode given config and current disk state.
pub fn select_mode(configured: &WorkspaceMode, local_root: &Path, threshold_pct: u8) -> WorkspaceMode {
    match configured {
        WorkspaceMode::Local => WorkspaceMode::Local,
        WorkspaceMode::Remote => WorkspaceMode::Remote,
        WorkspaceMode::Hybrid => {
            let usage = disk_usage_pct(local_root);
            if usage > threshold_pct as f64 {
                tracing::warn!(
                    disk_usage_pct = usage,
                    threshold = threshold_pct,
                    "disk above threshold — switching workspace to Remote"
                );
                WorkspaceMode::Remote
            } else {
                WorkspaceMode::Local
            }
        }
    }
}
