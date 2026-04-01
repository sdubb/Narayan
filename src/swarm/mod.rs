//! Swarm coordination — replaces the global static Mutex with the existing
//! Redis-backed Queue so swarm tasks survive restarts and scale across
//! multiple Narayan instances without a single-process bottleneck.
//!
//! The old design:
//!   static SWARM_CELL: OnceLock<Mutex<SwarmScheduler>>
//!   → single global lock, in-memory only, lost on restart, single-instance
//!
//! The new design:
//!   Arc<dyn Queue> injected at startup (same Queue used by the worker pool)
//!   → no global lock, durable via Redis, works across instances

pub mod scheduler;

use std::sync::Arc;

use anyhow::Result;

use crate::scheduler::queue::{ExecutionTask, Queue};

/// Swarm coordinator — thin wrapper around the shared Queue.
/// Inject at startup from main.rs using the same Arc<dyn Queue> as WorkerPool.
pub struct Swarm {
    queue: Arc<dyn Queue>,
}

impl Swarm {
    pub fn new(queue: Arc<dyn Queue>) -> Self {
        Self { queue }
    }

    /// Enqueue a swarm task (agent) for execution.
    pub async fn push(&self, agent_id: String) -> Result<()> {
        self.queue.enqueue(ExecutionTask::new(agent_id)).await
    }

    /// Pop the next swarm task — used by swarm-aware workers.
    pub async fn next(&self) -> Result<Option<ExecutionTask>> {
        self.queue.dequeue().await
    }

    /// Current depth of the swarm queue — used by GET /swarm/status.
    pub async fn queue_depth(&self) -> Result<usize> {
        self.queue.depth().await
    }

    /// Always true — confirms the swarm uses the shared queue, not a static Mutex.
    pub fn is_queue_backed(&self) -> bool {
        true
    }
}
