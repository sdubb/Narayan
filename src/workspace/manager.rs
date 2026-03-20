use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    config::WorkspaceConfig,
    storage::PostgresStore,
    workspace::{
        local::LocalWorkspace,
        remote::{ObjectStorage, ObjectStorageConfig, RemoteWorkspace, S3CompatibleStorage},
        resolver::{select_mode, WorkspaceMode},
    },
};

// ── WorkspaceInfo — persisted in DB ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub tenant_id: String,
    pub agent_id: String,
    pub mode: WorkspaceMode,
    pub local_path: Option<String>,
    pub storage_key: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub archived: bool,
}

impl WorkspaceInfo {
    pub fn effective_path(&self) -> String {
        self.local_path
            .clone()
            .or_else(|| self.storage_key.clone())
            .unwrap_or_else(|| format!("unknown/{}", self.agent_id))
    }
}

// ── WorkspaceHandle — live handle for agent use ────────────────────────────

pub struct WorkspaceHandle {
    pub info: WorkspaceInfo,
    pub local: Option<LocalWorkspace>,
    pub remote: Option<RemoteWorkspace>,
}

impl WorkspaceHandle {
    /// Resolve a workspace-relative path to an absolute local path.
    /// For remote-only mode, downloads the file to a temp location first.
    pub async fn resolve_local(&self, rel: &str) -> Result<PathBuf> {
        match &self.local {
            Some(lw) => lw.resolve(rel),
            None => {
                // Remote mode: cache to tmp
                let tmp = std::env::temp_dir().join(format!("narayan_{}", self.info.agent_id));
                tokio::fs::create_dir_all(&tmp).await?;
                let local_path = tmp.join(rel.trim_start_matches('/'));
                if !local_path.exists() {
                    if let Some(rw) = &self.remote {
                        let data = rw.read(rel).await?;
                        if let Some(p) = local_path.parent() {
                            tokio::fs::create_dir_all(p).await?;
                        }
                        tokio::fs::write(&local_path, data).await?;
                    }
                }
                Ok(local_path)
            }
        }
    }

    /// Write a file — to local if available, remote otherwise.
    pub async fn write(&self, rel: &str, data: Vec<u8>) -> Result<()> {
        if let Some(lw) = &self.local {
            lw.write(rel, &data).await?;
        }
        if let Some(rw) = &self.remote {
            rw.write(rel, data).await?;
        }
        Ok(())
    }

    pub fn local_path_str(&self) -> String {
        self.info.effective_path()
    }
}

// ── WorkspaceManager ──────────────────────────────────────────────────────

pub struct WorkspaceManager {
    cfg: WorkspaceConfig,
    store: Arc<PostgresStore>,
    storage: Option<Arc<dyn ObjectStorage>>,
}

impl WorkspaceManager {
    pub fn new(cfg: WorkspaceConfig, store: Arc<PostgresStore>) -> Result<Self> {
        let storage: Option<Arc<dyn ObjectStorage>> = if let (Some(bucket), Some(endpoint), Some(ak), Some(sk)) =
            (cfg.s3_bucket.as_ref(), cfg.s3_endpoint.as_ref(), cfg.s3_access_key.as_ref(), cfg.s3_secret_key.as_ref())
        {
            let s3_cfg = ObjectStorageConfig {
                bucket: bucket.clone(),
                endpoint: endpoint.clone(),
                region: cfg.s3_region.clone().unwrap_or_else(|| "auto".into()),
                access_key: ak.clone(),
                secret_key: sk.clone(),
            };
            match S3CompatibleStorage::new(s3_cfg) {
                Ok(s) => {
                    tracing::info!(endpoint = %endpoint, bucket = %bucket, "object storage configured");
                    Some(Arc::new(s))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to init object storage — remote mode unavailable");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self { cfg, store, storage })
    }

    /// Create a workspace for an agent.
    /// Selects Local or Remote based on disk usage and config.
    pub async fn create(&self, tenant_id: &str, agent_id: &str) -> Result<WorkspaceHandle> {
        let local_root = PathBuf::from(&self.cfg.local_root);
        let config_mode = WorkspaceMode::from_str(&self.cfg.mode);
        let effective = select_mode(&config_mode, &local_root, self.cfg.disk_threshold_pct);

        tracing::debug!(
            tenant_id = %tenant_id,
            agent_id  = %agent_id,
            mode      = ?effective,
            "creating workspace"
        );

        let local_path = format!("{}/{}/agents/{}", self.cfg.local_root, tenant_id, agent_id);
        let storage_key = format!("workspaces/{}/{}", tenant_id, agent_id);

        let (local, remote, mode_used, lp, sk) = match &effective {
            WorkspaceMode::Local => {
                let lw = LocalWorkspace::create(PathBuf::from(&local_path)).await?;
                (Some(lw), None, WorkspaceMode::Local, Some(local_path.clone()), None)
            }
            WorkspaceMode::Remote => match &self.storage {
                Some(s) => {
                    let rw = RemoteWorkspace::new(storage_key.clone(), s.clone());
                    (None, Some(rw), WorkspaceMode::Remote, None, Some(storage_key.clone()))
                }
                None => {
                    tracing::warn!(
                        agent_id = %agent_id,
                        "remote mode requested but no object storage configured — falling back to local"
                    );
                    let lw = LocalWorkspace::create(PathBuf::from(&local_path)).await?;
                    (Some(lw), None, WorkspaceMode::Local, Some(local_path.clone()), None)
                }
            },
            WorkspaceMode::Hybrid => {
                // Always create local; also mirror to remote if available
                let lw = LocalWorkspace::create(PathBuf::from(&local_path)).await?;
                let rw = self.storage.as_ref().map(|s| RemoteWorkspace::new(storage_key.clone(), s.clone()));
                (
                    Some(lw),
                    rw,
                    WorkspaceMode::Hybrid,
                    Some(local_path.clone()),
                    self.storage.as_ref().map(|_| storage_key.clone()),
                )
            }
        };

        let info = WorkspaceInfo {
            id: crate::util::new_id(),
            tenant_id: tenant_id.to_string(),
            agent_id: agent_id.to_string(),
            mode: mode_used,
            local_path: lp,
            storage_key: sk,
            created_at: Utc::now(),
            archived: false,
        };

        // Persist metadata
        self.store.upsert_workspace(&info).await?;

        Ok(WorkspaceHandle { info, local, remote })
    }

    /// Archive a completed workspace to object storage, then remove local copy.
    pub async fn archive(&self, info: &WorkspaceInfo) -> Result<()> {
        let Some(storage) = &self.storage else {
            tracing::debug!(agent_id = %info.agent_id, "no object storage — skipping archive");
            return Ok(());
        };

        let Some(ref local_path) = info.local_path else {
            return Ok(());
        };
        let lw = LocalWorkspace::open(PathBuf::from(local_path));

        let storage_prefix =
            info.storage_key.clone().unwrap_or_else(|| format!("workspaces/{}/{}", info.tenant_id, info.agent_id));

        let files = lw.list_all().await.unwrap_or_default();
        let file_count = files.len();

        for path in files {
            let rel = path.strip_prefix(local_path).unwrap_or(&path);
            let key = format!("{}/{}", storage_prefix, rel.to_string_lossy().replace('\\', "/"));
            match tokio::fs::read(&path).await {
                Ok(data) => {
                    let mime = mime_guess::from_path(&path).first_or_octet_stream().to_string();
                    if let Err(e) = storage.put(&key, data, &mime).await {
                        tracing::warn!(key = %key, error = %e, "failed to archive file");
                    }
                }
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "read failed during archive"),
            }
        }

        tracing::info!(
            agent_id   = %info.agent_id,
            files      = file_count,
            prefix     = %storage_prefix,
            "workspace archived to object storage"
        );

        // Delete local workspace
        if let Err(e) = lw.delete().await {
            tracing::warn!(error = %e, "failed to delete local workspace after archive");
        }

        // Mark archived in DB
        self.store.mark_workspace_archived(&info.id).await?;
        Ok(())
    }

    /// Background cleanup: delete local workspaces older than `hours` and archive them first.
    pub async fn cleanup_old(&self, older_than_hours: u64) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::hours(older_than_hours as i64);
        let old = self.store.list_workspaces_older_than(cutoff).await?;
        let count = old.len();

        for ws in old {
            if !ws.archived {
                if let Err(e) = self.archive(&ws).await {
                    tracing::error!(agent_id = %ws.agent_id, error = %e, "cleanup archive failed");
                }
            } else if let Some(ref lp) = ws.local_path {
                let lw = LocalWorkspace::open(PathBuf::from(lp));
                if let Err(e) = lw.delete().await {
                    tracing::warn!(agent_id = %ws.agent_id, error = %e, "cleanup delete failed");
                }
            }
        }

        if count > 0 {
            tracing::info!(cleaned = count, "workspace cleanup complete");
        }
        Ok(count)
    }

    /// Check if object storage is available.
    pub fn has_remote_storage(&self) -> bool {
        self.storage.is_some()
    }

    /// Local workspace root path string (for workers constructing paths).
    pub fn local_root(&self) -> &str {
        &self.cfg.local_root
    }
}
