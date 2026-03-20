use std::sync::Arc;

use anyhow::Result;
use tokio::task::JoinSet;

use crate::{
    agent::AgentLoop,
    events::EventBus,
    metrics::Metrics,
    scheduler::queue::Queue,
    segments::AgentServices,
    storage::PostgresStore,
    worker::worker::Worker,
    workspace::manager::WorkspaceManager,
};

pub struct WorkerPool {
    pool_size:         usize,
    store:             Arc<PostgresStore>,
    queue:             Arc<dyn Queue>,
    agent_loop:        Arc<AgentLoop>,
    metrics:           Arc<Metrics>,
    workspace_manager: Arc<WorkspaceManager>,
    services:          Arc<AgentServices>,
    event_bus:         Arc<EventBus>,
}

fn idle_sleep_duration() -> tokio::time::Duration {
    tokio::time::Duration::from_millis(50)
}

fn error_sleep_duration() -> tokio::time::Duration {
    tokio::time::Duration::from_millis(200)
}

impl WorkerPool {
    pub fn new(
        pool_size:         usize,
        store:             Arc<PostgresStore>,
        queue:             Arc<dyn Queue>,
        agent_loop:        Arc<AgentLoop>,
        metrics:           Arc<Metrics>,
        workspace_manager: Arc<WorkspaceManager>,
        services:          Arc<AgentServices>,
        event_bus:         Arc<EventBus>,
    ) -> Self {
        Self { pool_size, store, queue, agent_loop, metrics, workspace_manager, services, event_bus }
    }

    pub async fn run(&self) -> Result<()> {
        tracing::info!(pool_size = self.pool_size, "worker pool starting");
        let mut set = JoinSet::new();

        for id in 0..self.pool_size {
            let worker = Worker::new(
                id,
                self.store.clone(),
                self.queue.clone(),
                self.agent_loop.clone(),
                self.metrics.clone(),
                self.workspace_manager.clone(),
                self.services.clone(),
                self.event_bus.clone(),
            );
            set.spawn(async move { worker_loop(worker).await });
        }

        while let Some(res) = set.join_next().await {
            if let Err(e) = res {
                tracing::error!(error = %e, "worker panicked");
            }
        }
        Ok(())
    }
}

async fn worker_loop(worker: Worker) {
    loop {
        match worker.process_next().await {
            Ok(true)  => {}
            Ok(false) => tokio::time::sleep(idle_sleep_duration()).await,
            Err(e)    => {
                tracing::error!(error = %e, "worker error");
                tokio::time::sleep(error_sleep_duration()).await;
            }
        }
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
