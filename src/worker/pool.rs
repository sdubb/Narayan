use std::sync::Arc;

use anyhow::Result;
use tokio::task::JoinSet;

use crate::{
    agent::AgentLoop, events::EventBus, metrics::Metrics, scheduler::queue::Queue, segments::AgentServices,
    storage::PostgresStore, worker::worker::Worker, workspace::manager::WorkspaceManager,
};

pub struct WorkerPool {
    pool_size: usize,
    name: String,
    store: Arc<PostgresStore>,
    queue: Arc<dyn Queue>,
    agent_loop: Arc<AgentLoop>,
    metrics: Arc<Metrics>,
    workspace_manager: Arc<WorkspaceManager>,
    services: Arc<AgentServices>,
    event_bus: Arc<EventBus>,
}

fn idle_sleep_duration() -> tokio::time::Duration {
    tokio::time::Duration::from_millis(50)
}

fn error_sleep_duration() -> tokio::time::Duration {
    tokio::time::Duration::from_millis(200)
}

impl WorkerPool {
    pub fn new(
        pool_size: usize,
        name: String,
        store: Arc<PostgresStore>,
        queue: Arc<dyn Queue>,
        agent_loop: Arc<AgentLoop>,
        metrics: Arc<Metrics>,
        workspace_manager: Arc<WorkspaceManager>,
        services: Arc<AgentServices>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self { pool_size, name, store, queue, agent_loop, metrics, workspace_manager, services, event_bus }
    }

    pub async fn run(&self, cancel_token: tokio_util::sync::CancellationToken) -> Result<()> {
        tracing::info!(pool_size = self.pool_size, "worker pool starting");
        let mut set = JoinSet::new();

        for id in 0..self.pool_size {
            let worker_name = format!("{}-{}", self.name, id);
            let worker = Worker::new(
                id,
                worker_name.clone(),
                self.store.clone(),
                self.queue.clone(),
                self.agent_loop.clone(),
                self.metrics.clone(),
                self.workspace_manager.clone(),
                self.services.clone(),
                self.event_bus.clone(),
            );
            let token = cancel_token.clone();
            set.spawn(async move {
                loop {
                    if token.is_cancelled() { break; }
                    match worker.process_next(token.clone()).await {
                        Ok(true) => {}
                        Ok(false) => {
                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        }
                        Err(e) => {
                            tracing::error!(worker = %worker_name, error = %e, "worker failed");
                            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        }
                    }
                }
            });
        }

        while let Some(res) = set.join_next().await {
            if let Err(e) = res {
                tracing::error!(error = %e, "worker panicked");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sleep_durations() {
        assert_eq!(idle_sleep_duration(), tokio::time::Duration::from_millis(50));
        assert_eq!(error_sleep_duration(), tokio::time::Duration::from_millis(200));
        assert!(error_sleep_duration() > idle_sleep_duration());
    }
}
