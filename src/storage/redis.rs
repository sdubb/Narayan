use anyhow::Result;
use redis::AsyncCommands;

use crate::scheduler::queue::Task;

pub struct RedisQueue {
    client: redis::Client,
    queue_key: String,
    processing_key: String,
}

impl RedisQueue {
    pub fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        Ok(Self { client, queue_key: "narayan:queue".into(), processing_key: "narayan:processing".into() })
    }

    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection> {
        Ok(self.client.get_multiplexed_async_connection().await?)
    }

    /// Push a task onto the right end of the queue list.
    pub async fn enqueue(&self, task: &Task) -> Result<()> {
        let mut conn = self.conn().await?;
        let payload = serde_json::to_string(task)?;
        conn.rpush::<_, _, ()>(&self.queue_key, payload).await?;
        Ok(())
    }

    /// Pop one task from the queue (blocking with timeout).
    pub async fn dequeue(&self, timeout_secs: f64) -> Result<Option<Task>> {
        let mut conn = self.conn().await?;
        // BLPOP returns the source key and payload.
        let result: Option<[String; 2]> = conn.blpop(&self.queue_key, timeout_secs).await?;

        match result {
            Some([_, payload]) => {
                // Track in processing set
                conn.rpush::<_, _, ()>(&self.processing_key, &payload).await?;
                Ok(Some(serde_json::from_str(&payload)?))
            }
            None => Ok(None),
        }
    }

    /// Acknowledge task completion – remove from processing set.
    pub async fn ack(&self, task: &Task) -> Result<()> {
        let mut conn = self.conn().await?;
        let payload = serde_json::to_string(task)?;
        conn.lrem::<_, _, ()>(&self.processing_key, 1, payload).await?;
        Ok(())
    }

    /// Re-queue a failed task.
    pub async fn retry(&self, task: &Task) -> Result<()> {
        let mut conn = self.conn().await?;
        // Remove from processing
        let payload = serde_json::to_string(task)?;
        conn.lrem::<_, _, ()>(&self.processing_key, 1, &payload).await?;
        // Push back to front of queue for priority retry
        conn.lpush::<_, _, ()>(&self.queue_key, payload).await?;
        Ok(())
    }

    /// Current depth of the ready queue.
    pub async fn queue_depth(&self) -> Result<usize> {
        let mut conn = self.conn().await?;
        let len: usize = conn.llen(&self.queue_key).await?;
        Ok(len)
    }
}
