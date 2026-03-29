use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub scheduler: SchedulerConfig,
    pub worker: WorkerConfig,
    pub gateway: GatewayConfig,
    pub workspace: WorkspaceConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RedisConfig {
    pub url: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SchedulerConfig {
    pub poll_interval_ms: u64,
    pub max_batch_size: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkerConfig {
    pub pool_size: usize,
    pub node_name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GatewayConfig {
    pub cache_ttl_secs: u64,
    pub cache_max_entries: usize,
    pub requests_per_sec: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkspaceConfig {
    /// Local disk root for all workspaces: /var/narayan/workspaces
    pub local_root: String,
    /// Storage mode: "local" | "remote" | "hybrid"
    pub mode: String,
    /// Disk usage % above which Hybrid switches to Remote (default: 80)
    pub disk_threshold_pct: u8,
    /// Archive and clean local workspaces older than this many hours (default: 24)
    pub cleanup_after_hours: u64,
    // ── Object storage (optional — all None = local only) ────────────────
    pub s3_bucket: Option<String>,
    pub s3_endpoint: Option<String>,
    pub s3_region: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let _ = dotenv::dotenv();

        let cfg = config::Config::builder()
            // Server
            .set_default("server.host", "0.0.0.0")?
            .set_default("server.port", 8080)?
            // Database
            .set_default("database.url", "postgres://localhost/narayan")?
            .set_default("database.max_connections", 20)?
            // Redis
            .set_default("redis.url", "redis://localhost:6379")?
            .set_default("redis.enabled", true)?
            // Scheduler
            .set_default("scheduler.poll_interval_ms", 500u64)?
            .set_default("scheduler.max_batch_size", 256i64)?
            // Worker
            .set_default("worker.pool_size", 32i64)?
            .set_default("worker.node_name", "narayan-node")?
            // Gateway
            .set_default("gateway.cache_ttl_secs", 300i64)?
            .set_default("gateway.cache_max_entries", 10_000i64)?
            .set_default("gateway.requests_per_sec", 10.0f64)?
            // Workspace
            .set_default("workspace.local_root", "./workspace")?
            .set_default("workspace.mode", "hybrid")?
            .set_default("workspace.disk_threshold_pct", 80u64)?
            .set_default("workspace.cleanup_after_hours", 24u64)?
            .add_source(config::Environment::default().prefix("NARAYAN").separator("__"))
            .build()?;

        Ok(cfg.try_deserialize()?)
    }
}
