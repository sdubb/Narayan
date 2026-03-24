use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;

/// A single memory entry.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Core memory store trait supporting key-value recall.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn store(&self, agent_id: &str, key: &str, value: &str) -> Result<()>;
    async fn recall(&self, agent_id: &str, key: &str) -> Result<Option<String>>;
    async fn forget(&self, agent_id: &str, key: &str) -> Result<()>;
    async fn list_keys(&self, agent_id: &str) -> Result<Vec<String>>;
}

// ── Redis-backed store (production default) ────────────────────────────────

/// Redis-backed memory store.
///
/// Keys are stored as Redis hashes:
///   HSET  narayan:mem:{agent_id}  {key}  {value}
///   HGET  narayan:mem:{agent_id}  {key}
///   HDEL  narayan:mem:{agent_id}  {key}
///   HKEYS narayan:mem:{agent_id}
///
/// This replaces the in-process RwLock<HashMap> which lost all memory on
/// every restart.  Redis survives restarts, is visible across multiple
/// Narayan instances, and has no single-writer bottleneck.
pub struct RedisMemoryStore {
    client: redis::Client,
    /// Optional TTL in seconds applied to the whole hash on every write.
    /// None means keys live forever.
    ttl_secs: Option<u64>,
}

impl RedisMemoryStore {
    pub fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self { client, ttl_secs: None })
    }

    /// Set a TTL so agent memory is auto-expired after N seconds of inactivity.
    pub fn with_ttl(mut self, secs: u64) -> Self {
        self.ttl_secs = Some(secs);
        self
    }

    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection> {
        Ok(self.client.get_multiplexed_async_connection().await?)
    }

    fn hash_key(agent_id: &str) -> String {
        format!("narayan:mem:{agent_id}")
    }
}

#[async_trait]
impl MemoryStore for RedisMemoryStore {
    async fn store(&self, agent_id: &str, key: &str, value: &str) -> Result<()> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        let hk = Self::hash_key(agent_id);
        conn.hset::<_, _, _, ()>(&hk, key, value).await?;
        if let Some(ttl) = self.ttl_secs {
            conn.expire::<_, ()>(&hk, ttl as i64).await?;
        }
        Ok(())
    }

    async fn recall(&self, agent_id: &str, key: &str) -> Result<Option<String>> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        Ok(conn.hget(Self::hash_key(agent_id), key).await?)
    }

    async fn forget(&self, agent_id: &str, key: &str) -> Result<()> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        conn.hdel::<_, _, ()>(Self::hash_key(agent_id), key).await?;
        Ok(())
    }

    async fn list_keys(&self, agent_id: &str) -> Result<Vec<String>> {
        use redis::AsyncCommands;
        let mut conn = self.conn().await?;
        Ok(conn.hkeys(Self::hash_key(agent_id)).await?)
    }
}

// ── In-memory store (development / testing only) ───────────────────────────

/// In-memory store — for unit tests and local dev runs without Redis.
///
/// WARNING: All memory is lost on process restart.  Never use in production.
/// In production, use RedisMemoryStore.
pub struct InMemoryStore {
    data: Arc<RwLock<HashMap<String, HashMap<String, MemoryEntry>>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self { data: Arc::new(RwLock::new(HashMap::new())) }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MemoryStore for InMemoryStore {
    async fn store(&self, agent_id: &str, key: &str, value: &str) -> Result<()> {
        let mut data = self.data.write().await;
        let agent_store = data.entry(agent_id.to_string()).or_default();
        agent_store.insert(
            key.to_string(),
            MemoryEntry { key: key.to_string(), value: value.to_string(), created_at: chrono::Utc::now() },
        );
        Ok(())
    }

    async fn recall(&self, agent_id: &str, key: &str) -> Result<Option<String>> {
        let data = self.data.read().await;
        Ok(data.get(agent_id).and_then(|m| m.get(key)).map(|e| e.value.clone()))
    }

    async fn forget(&self, agent_id: &str, key: &str) -> Result<()> {
        let mut data = self.data.write().await;
        if let Some(m) = data.get_mut(agent_id) {
            m.remove(key);
        }
        Ok(())
    }

    async fn list_keys(&self, agent_id: &str) -> Result<Vec<String>> {
        let data = self.data.read().await;
        Ok(data.get(agent_id).map(|m| m.keys().cloned().collect()).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_store_and_recall() {
        let store = InMemoryStore::new();
        store.store("agent-1", "repo", "acme/api").await.unwrap();
        assert_eq!(store.recall("agent-1", "repo").await.unwrap(), Some("acme/api".into()));
    }

    #[tokio::test]
    async fn test_in_memory_forget() {
        let store = InMemoryStore::new();
        store.store("agent-1", "key", "val").await.unwrap();
        store.forget("agent-1", "key").await.unwrap();
        assert_eq!(store.recall("agent-1", "key").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_in_memory_list_keys() {
        let store = InMemoryStore::new();
        store.store("agent-1", "a", "1").await.unwrap();
        store.store("agent-1", "b", "2").await.unwrap();
        let mut keys = store.list_keys("agent-1").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn test_in_memory_recall_missing_returns_none() {
        let store = InMemoryStore::new();
        assert_eq!(store.recall("agent-1", "missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_in_memory_agents_are_isolated() {
        let store = InMemoryStore::new();
        store.store("agent-1", "key", "val-a1").await.unwrap();
        store.store("agent-2", "key", "val-a2").await.unwrap();
        assert_eq!(store.recall("agent-1", "key").await.unwrap(), Some("val-a1".into()));
        assert_eq!(store.recall("agent-2", "key").await.unwrap(), Some("val-a2".into()));
    }
}
