use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::RwLock;

use crate::{metrics::Metrics, providers::ChatResponse};

struct CacheEntry {
    response: ChatResponse,
    inserted_at: Instant,
    ttl: Duration,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }
}

/// Thread-safe TTL response cache keyed by a hash of the request.
/// Tracks cache hit/miss metrics for observability.
pub struct ResponseCache {
    inner: Arc<RwLock<HashMap<String, CacheEntry>>>,
    default_ttl: Duration,
    max_entries: usize,
    metrics: Option<Arc<Metrics>>,  // Optional metrics tracking
}

impl ResponseCache {
    pub fn new(default_ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: Duration::from_secs(default_ttl_secs),
            max_entries,
            metrics: None,
        }
    }

    /// Create a cache with metrics tracking enabled.
    pub fn with_metrics(default_ttl_secs: u64, max_entries: usize, metrics: Arc<Metrics>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: Duration::from_secs(default_ttl_secs),
            max_entries,
            metrics: Some(metrics),
        }
    }

    /// Look up a cached response. Returns `None` if missing or expired.
    /// Records cache hit/miss metrics.
    pub async fn get(&self, key: &str) -> Option<ChatResponse> {
        let cache = self.inner.read().await;
        let result = cache.get(key).and_then(|entry| if entry.is_expired() { None } else { Some(entry.response.clone()) });
        
        // Track metrics
        if let Some(ref metrics) = self.metrics {
            if result.is_some() {
                metrics.response_cache_hit();
            } else {
                metrics.response_cache_miss();
            }
        }
        
        result
    }

    /// Store a response. Evicts expired entries and enforces `max_entries`.
    pub async fn set(&self, key: String, response: ChatResponse) {
        let mut cache = self.inner.write().await;

        // Evict expired entries first
        cache.retain(|_, v| !v.is_expired());

        // If still at capacity, evict oldest entry
        if cache.len() >= self.max_entries {
            if let Some(oldest_key) = cache.iter().min_by_key(|(_, v)| v.inserted_at).map(|(k, _)| k.clone()) {
                cache.remove(&oldest_key);
            }
        }

        cache.insert(key, CacheEntry { response, inserted_at: Instant::now(), ttl: self.default_ttl });
    }

    /// Invalidate all entries for a given agent.
    pub async fn invalidate_prefix(&self, prefix: &str) {
        let mut cache = self.inner.write().await;
        cache.retain(|k, _| !k.starts_with(prefix));
    }

    pub async fn size(&self) -> usize {
        self.inner.read().await.len()
    }
}

/// Build a cache key by hashing message contents + tools.
/// Uses a simple deterministic string hash — no external crypto deps needed.
pub fn make_cache_key(agent_id: &str, messages_hash: &str) -> String {
    format!("{}:{}", agent_id, messages_hash)
}

/// Simple djb2-style hash of a string for cache keying.
pub fn hash_str(input: &str) -> String {
    let mut hash: u64 = 5381;
    for byte in input.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    format!("{:016x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_response(text: &str) -> crate::providers::ChatResponse {
        crate::providers::ChatResponse {
            content: Some(text.to_string()),
            tool_calls: vec![],
            input_tokens: 10,
            output_tokens: 20,
        }
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache = ResponseCache::new(60, 100);
        assert!(cache.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_hit() {
        let cache = ResponseCache::new(60, 100);
        let resp = mock_response("hello");
        cache.set("key1".to_string(), resp.clone()).await;
        let result = cache.get("key1").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, Some("hello".to_string()));
    }

    #[tokio::test]
    async fn test_cache_ttl_expiry() {
        let cache = ResponseCache::new(1, 100);
        cache.set("key1".to_string(), mock_response("expire me")).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert!(cache.get("key1").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_eviction() {
        let cache = ResponseCache::new(60, 2);
        cache.set("k1".to_string(), mock_response("a")).await;
        cache.set("k2".to_string(), mock_response("b")).await;
        cache.set("k3".to_string(), mock_response("c")).await;
        assert_eq!(cache.size().await, 2);
    }

    #[tokio::test]
    async fn test_invalidate_prefix() {
        let cache = ResponseCache::new(60, 100);
        cache.set("agent1:abc".to_string(), mock_response("a1")).await;
        cache.set("agent2:def".to_string(), mock_response("a2")).await;
        cache.invalidate_prefix("agent1").await;
        assert!(cache.get("agent1:abc").await.is_none());
        assert!(cache.get("agent2:def").await.is_some());
    }

    #[tokio::test]
    async fn test_hash_determinism() {
        assert_eq!(hash_str("hello"), hash_str("hello"));
    }

    #[tokio::test]
    async fn test_make_cache_key() {
        assert_eq!(make_cache_key("a1", "h1"), "a1:h1");
    }
}
