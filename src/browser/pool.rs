use std::sync::Arc;

use anyhow::Result;
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use tokio::sync::{Mutex, Semaphore};

#[derive(Debug, Clone)]
pub struct BrowserPoolConfig {
    pub size: usize,
    pub headless: bool,
    pub nav_timeout_ms: u64,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

impl Default for BrowserPoolConfig {
    fn default() -> Self {
        Self { size: 4, headless: true, nav_timeout_ms: 30_000, viewport_width: 1440, viewport_height: 900 }
    }
}

pub struct BrowserHandle {
    pub browser: Arc<Mutex<Browser>>,
    pub config: BrowserPoolConfig,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

pub struct BrowserPool {
    browsers: Vec<Arc<Mutex<Browser>>>,
    sem: Arc<Semaphore>,
    config: BrowserPoolConfig,
    next: std::sync::atomic::AtomicUsize,
}

impl BrowserPool {
    pub async fn new(config: BrowserPoolConfig) -> Result<Arc<Self>> {
        let mut browsers = Vec::with_capacity(config.size);
        for i in 0..config.size {
            let mut builder = BrowserConfig::builder()
                .window_size(config.viewport_width, config.viewport_height)
                .arg("--no-sandbox")
                .arg("--disable-setuid-sandbox")
                .arg("--disable-dev-shm-usage")
                .arg("--disable-gpu")
                .arg("--disable-background-timer-throttling");

            if !config.headless {
                builder = builder.with_head();
            }

            let cfg = builder.build().map_err(|e| anyhow::anyhow!("BrowserConfig: {}", e))?;

            let (browser, mut handler) =
                Browser::launch(cfg).await.map_err(|e| anyhow::anyhow!("Chromium launch #{}: {}", i, e))?;

            tokio::spawn(async move { while handler.next().await.is_some() {} });
            tracing::info!(instance = i, "Chromium instance ready");
            browsers.push(Arc::new(Mutex::new(browser)));
        }

        let sem = Arc::new(Semaphore::new(config.size));
        tracing::info!(pool_size = config.size, "browser pool ready");
        Ok(Arc::new(Self { browsers, sem, config, next: std::sync::atomic::AtomicUsize::new(0) }))
    }

    pub async fn acquire(&self, timeout_secs: u64) -> Result<BrowserHandle> {
        let permit =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), Arc::clone(&self.sem).acquire_owned())
                .await
                .map_err(|_| anyhow::anyhow!("pool timeout after {}s", timeout_secs))?
                .map_err(|e| anyhow::anyhow!("semaphore: {}", e))?;

        let idx = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.browsers.len();
        Ok(BrowserHandle { browser: Arc::clone(&self.browsers[idx]), config: self.config.clone(), _permit: permit })
    }

    pub fn size(&self) -> usize {
        self.browsers.len()
    }
}
