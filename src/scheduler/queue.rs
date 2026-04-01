use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A single unit of work pushed onto the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTask {
    pub id: String,
    pub agent_id: String,
    pub attempt: u32,
    pub enqueued_at: chrono::DateTime<chrono::Utc>,
}

impl ExecutionTask {
    pub fn new(agent_id: String) -> Self {
        Self { id: uuid::Uuid::new_v4().to_string(), agent_id, attempt: 0, enqueued_at: chrono::Utc::now() }
    }

    pub fn with_attempt(mut self, n: u32) -> Self {
        self.attempt = n;
        self
    }
}

/// Queue backend abstraction.
#[async_trait]
pub trait Queue: Send + Sync {
    async fn enqueue(&self, task: ExecutionTask) -> Result<()>;
    async fn dequeue(&self) -> Result<Option<ExecutionTask>>;
    async fn ack(&self, task: &ExecutionTask) -> Result<()>;
    async fn retry(&self, task: ExecutionTask) -> Result<()>;
    /// Current number of tasks waiting in the queue.
    async fn depth(&self) -> Result<usize>;
}

// ── In-memory queue (for development / testing) ────────────────────────────

use std::{collections::VecDeque, sync::Arc};

use tokio::sync::Mutex;

pub struct InMemoryQueue {
    inner: Arc<Mutex<VecDeque<ExecutionTask>>>,
}

impl InMemoryQueue {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(VecDeque::new())) }
    }
}

impl Default for InMemoryQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Queue for InMemoryQueue {
    async fn enqueue(&self, task: ExecutionTask) -> Result<()> {
        self.inner.lock().await.push_back(task);
        Ok(())
    }

    async fn dequeue(&self) -> Result<Option<ExecutionTask>> {
        Ok(self.inner.lock().await.pop_front())
    }

    async fn ack(&self, _task: &ExecutionTask) -> Result<()> {
        Ok(())
    }

    async fn retry(&self, task: ExecutionTask) -> Result<()> {
        let mut q = self.inner.lock().await;
        q.push_front(task);
        Ok(())
    }

    async fn depth(&self) -> Result<usize> {
        Ok(self.inner.lock().await.len())
    }
}

// ── Redis-backed queue wrapper ─────────────────────────────────────────────

pub struct RedisBackedQueue {
    inner: crate::storage::RedisQueue,
}

impl RedisBackedQueue {
    pub fn new(redis_url: &str) -> Result<Self> {
        Ok(Self { inner: crate::storage::RedisQueue::new(redis_url)? })
    }
}

#[async_trait]
impl Queue for RedisBackedQueue {
    async fn enqueue(&self, task: ExecutionTask) -> Result<()> {
        self.inner.enqueue(&task).await
    }

    async fn dequeue(&self) -> Result<Option<ExecutionTask>> {
        self.inner.dequeue(1.0).await
    }

    async fn ack(&self, task: &ExecutionTask) -> Result<()> {
        self.inner.ack(task).await
    }

    async fn retry(&self, task: ExecutionTask) -> Result<()> {
        self.inner.retry(&task).await
    }

    async fn depth(&self) -> Result<usize> {
        self.inner.queue_depth().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_queue_depth_tracks_enqueue_dequeue() {
        let q = InMemoryQueue::new();
        assert_eq!(q.depth().await.unwrap(), 0);

        q.enqueue(ExecutionTask::new("a1".into())).await.unwrap();
        q.enqueue(ExecutionTask::new("a2".into())).await.unwrap();
        assert_eq!(q.depth().await.unwrap(), 2);

        let _ = q.dequeue().await.unwrap();
        assert_eq!(q.depth().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_in_memory_queue_retry_pushes_to_front() {
        let q = InMemoryQueue::new();
        q.enqueue(ExecutionTask::new("a1".into())).await.unwrap();
        q.enqueue(ExecutionTask::new("a2".into())).await.unwrap();

        let retry = ExecutionTask::new("retry-me".into()).with_attempt(1);
        q.retry(retry).await.unwrap();

        let next = q.dequeue().await.unwrap().unwrap();
        assert_eq!(next.agent_id, "retry-me", "retried task must be at front of queue");
    }

    #[tokio::test]
    async fn test_in_memory_queue_fifo_ordering() {
        let q = InMemoryQueue::new();
        q.enqueue(ExecutionTask::new("first".into())).await.unwrap();
        q.enqueue(ExecutionTask::new("second".into())).await.unwrap();
        q.enqueue(ExecutionTask::new("third".into())).await.unwrap();

        assert_eq!(q.dequeue().await.unwrap().unwrap().agent_id, "first");
        assert_eq!(q.dequeue().await.unwrap().unwrap().agent_id, "second");
        assert_eq!(q.dequeue().await.unwrap().unwrap().agent_id, "third");
        assert!(q.dequeue().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_in_memory_queue_ack_is_noop() {
        let q = InMemoryQueue::new();
        let task = ExecutionTask::new("a1".into());
        q.enqueue(task.clone()).await.unwrap();
        let dequeued = q.dequeue().await.unwrap().unwrap();
        // ack should not panic or error
        q.ack(&dequeued).await.unwrap();
        assert_eq!(q.depth().await.unwrap(), 0);
    }

    #[test]
    fn test_task_new_sets_attempt_zero() {
        let task = ExecutionTask::new("agent-1".into());
        assert_eq!(task.attempt, 0);
        assert_eq!(task.agent_id, "agent-1");
    }

    #[test]
    fn test_task_with_attempt() {
        let task = ExecutionTask::new("agent-1".into()).with_attempt(3);
        assert_eq!(task.attempt, 3);
    }
}
