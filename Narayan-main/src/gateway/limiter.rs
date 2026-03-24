use std::{collections::HashMap, sync::Arc, time::Instant};

use tokio::sync::Mutex;

/// Token bucket rate limiter for a single provider.
struct Bucket {
    tokens: f64,
    capacity: f64,
    refill_per_sec: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self { tokens: capacity, capacity, refill_per_sec, last_refill: Instant::now() }
    }

    /// Try to consume `cost` tokens. Returns true if granted.
    fn try_consume(&mut self, cost: f64) -> bool {
        self.refill();
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last_refill = Instant::now();
    }

    /// Seconds to wait until `cost` tokens are available.
    fn wait_secs(&mut self, cost: f64) -> f64 {
        self.refill();
        if self.tokens >= cost {
            0.0
        } else {
            (cost - self.tokens) / self.refill_per_sec
        }
    }
}

/// Per-provider rate limiter configuration.
#[derive(Debug, Clone)]
pub struct ProviderLimits {
    /// Maximum requests per second.
    pub requests_per_sec: f64,
    /// Maximum burst size (requests).
    pub burst: f64,
}

impl Default for ProviderLimits {
    fn default() -> Self {
        Self { requests_per_sec: 10.0, burst: 20.0 }
    }
}

/// Manages rate limits across all providers.
pub struct RateLimiter {
    buckets: HashMap<String, Arc<Mutex<Bucket>>>,
}

impl RateLimiter {
    pub fn new(limits: HashMap<String, ProviderLimits>) -> Self {
        let buckets = limits
            .into_iter()
            .map(|(name, limit)| (name, Arc::new(Mutex::new(Bucket::new(limit.burst, limit.requests_per_sec)))))
            .collect();
        Self { buckets }
    }

    pub fn with_defaults(provider_names: &[&str]) -> Self {
        let limits = provider_names.iter().map(|name| (name.to_string(), ProviderLimits::default())).collect();
        Self::new(limits)
    }

    /// Wait until a request slot is available for the given provider.
    /// Returns immediately if no limit is configured for that provider.
    pub async fn acquire(&self, provider: &str) {
        let bucket = match self.buckets.get(provider) {
            Some(b) => b.clone(),
            None => return,
        };

        loop {
            let wait = {
                let mut b = bucket.lock().await;
                if b.try_consume(1.0) {
                    return;
                }
                b.wait_secs(1.0)
            };

            tracing::debug!(provider, wait_secs = wait, "rate limited, waiting");
            tokio::time::sleep(tokio::time::Duration::from_secs_f64(wait)).await;
        }
    }

    /// Non-blocking check — returns false if rate limited.
    pub async fn try_acquire(&self, provider: &str) -> bool {
        match self.buckets.get(provider) {
            Some(bucket) => bucket.lock().await.try_consume(1.0),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_burst_allows_initial() {
        let mut limits = HashMap::new();
        limits.insert("test_provider".to_string(), ProviderLimits { requests_per_sec: 1.0, burst: 5.0 });
        let limiter = RateLimiter::new(limits);
        for _ in 0..5 {
            assert!(limiter.try_acquire("test_provider").await);
        }
    }

    #[tokio::test]
    async fn test_try_acquire_exhausted() {
        let mut limits = HashMap::new();
        limits.insert("test_provider".to_string(), ProviderLimits { requests_per_sec: 1.0, burst: 2.0 });
        let limiter = RateLimiter::new(limits);
        assert!(limiter.try_acquire("test_provider").await);
        assert!(limiter.try_acquire("test_provider").await);
        assert!(!limiter.try_acquire("test_provider").await);
    }

    #[tokio::test]
    async fn test_unknown_provider_passthrough() {
        let limiter = RateLimiter::new(HashMap::new());
        assert!(limiter.try_acquire("unknown_provider").await);
    }
}
