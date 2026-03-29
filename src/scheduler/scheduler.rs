use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::time::{interval, Duration};

use crate::{
    events::{AgentEvent, EventBus},
    scheduler::queue::{Queue, Task},
    state::AgentStatus,
    storage::PostgresStore,
    util::next_run_after,
};

#[async_trait]
pub trait Scheduler: Send + Sync {
    async fn run(&self) -> Result<()>;
}

/// Polls Postgres for two types of agents:
///
/// 1. Due agents (next_run <= NOW, status pending/waiting)
///    → claimed atomically with FOR UPDATE SKIP LOCKED
///    → pushed to worker queue
///
/// 2. Delegating agents whose all children are done
///    → woken up so they can continue their remaining steps
pub struct DbPollingScheduler {
    store: Arc<PostgresStore>,
    queue: Arc<dyn Queue>,
    event_bus: Arc<EventBus>,
    poll_interval_ms: u64,
    batch_size: usize,
}

fn merge_child_results_into_parent(parent: &mut crate::state::AgentState, child_states: &[crate::state::AgentState]) {
    let child_ids = parent.pending_children.clone();
    let mut child_summaries = Vec::new();

    for child in child_states {
        if !child_ids.iter().any(|id| id == &child.id) {
            continue;
        }

        let summary =
            child.metadata.get("last_reflection").and_then(|v| v.as_str()).unwrap_or("child completed").to_string();
        child_summaries.push(format!("Sub-agent {}: {}", &child.id[..child.id.len().min(8)], summary));

        if let Some(arr) = child.metadata.get("key_findings").and_then(|v| v.as_array()) {
            let mut parent_findings: Vec<serde_json::Value> =
                parent.metadata.get("key_findings").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            parent_findings.extend(arr.iter().cloned());
            if parent_findings.len() > 50 {
                parent_findings = parent_findings.split_off(parent_findings.len() - 50);
            }
            parent.metadata["key_findings"] = serde_json::Value::Array(parent_findings);
        }
    }

    if !child_summaries.is_empty() {
        parent.metadata["last_reflection"] =
            serde_json::Value::String(format!("Parallel sub-tasks complete:\n{}", child_summaries.join("\n")));
    }
}

impl DbPollingScheduler {
    pub fn new(
        store: Arc<PostgresStore>,
        queue: Arc<dyn Queue>,
        event_bus: Arc<EventBus>,
        poll_interval_ms: u64,
        batch_size: usize,
    ) -> Self {
        Self { store, queue, event_bus, poll_interval_ms, batch_size }
    }
}

#[async_trait]
impl Scheduler for DbPollingScheduler {
    async fn run(&self) -> Result<()> {
        let mut ticker = interval(Duration::from_millis(self.poll_interval_ms));
        tracing::info!(poll_ms = self.poll_interval_ms, batch_size = self.batch_size, "scheduler started");

        loop {
            ticker.tick().await;

            // ── 1. Claim and enqueue due agents ────────────────────────────
            // Scheduler claims with a temporary "scheduler" ID and a 60s lease.
            // When a worker picks it up, it will take definitive ownership.
            match self.store.claim_due_agents("scheduler", 60, self.batch_size as i64).await {
                Ok(agents) if !agents.is_empty() => {
                    tracing::debug!(count = agents.len(), "scheduler claimed agents");
                    for agent in agents {
                        if let Err(e) = self.queue.enqueue(Task::new(agent.id.clone())).await {
                            tracing::error!(agent_id = %agent.id, error = %e, "enqueue failed");
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "scheduler poll failed"),
            }

            // ── 2. Wake delegating agents whose children are done ──────────
            match self.store.resolve_delegating_agents(self.batch_size as i64).await {
                Ok(parents) if !parents.is_empty() => {
                    tracing::info!(count = parents.len(), "waking delegating agents");
                    for mut parent in parents {
                        let child_ids = parent.pending_children.clone();

                        self.event_bus.publish(AgentEvent::ChildrenComplete {
                            agent_id: parent.id.clone(),
                            child_ids: child_ids.clone(),
                        });

                        // Collect child artifacts into parent metadata
                        let mut child_states = Vec::new();
                        for child_id in &child_ids {
                            if let Ok(Some(child)) = self.store.get_agent_internal(child_id).await {
                                child_states.push(child);
                            }
                        }
                        merge_child_results_into_parent(&mut parent, &child_states);

                        // Clear pending children and re-schedule parent
                        parent.pending_children.clear();
                        parent.status = AgentStatus::Waiting;
                        parent.next_run = next_run_after(0);
                        parent.updated_at = chrono::Utc::now();

                        if let Err(e) = self.store.upsert_agent(&parent).await {
                            tracing::error!(agent_id = %parent.id, error = %e, "failed to wake parent");
                        } else {
                            if let Err(e) = self.queue.enqueue(Task::new(parent.id.clone())).await {
                                tracing::error!(agent_id = %parent.id, error = %e, "enqueue failed");
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "delegation resolver failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child_state(id: &str, summary: &str, findings: &[&str]) -> crate::state::AgentState {
        let mut child =
            crate::state::AgentState::new(id.into(), "tenant-1".into(), "sub-goal".into(), "/tmp/ws".into());
        child.metadata["last_reflection"] = serde_json::json!(summary);
        child.metadata["key_findings"] = serde_json::json!(findings);
        child
    }

    #[test]
    fn test_merge_child_results_into_parent_adds_summaries_and_findings() {
        let mut parent =
            crate::state::AgentState::new("parent-1".into(), "tenant-1".into(), "parent".into(), "/tmp/ws".into());
        parent.pending_children = vec!["child-1".into(), "child-2".into()];

        let children = vec![
            child_state("child-1", "done one", &["finding-a"]),
            child_state("child-2", "done two", &["finding-b", "finding-c"]),
        ];

        merge_child_results_into_parent(&mut parent, &children);

        let reflection = parent.metadata["last_reflection"].as_str().unwrap_or_default();
        assert!(reflection.contains("Parallel sub-tasks complete"));
        assert!(reflection.contains("done one"));
        assert!(reflection.contains("done two"));
        assert_eq!(parent.metadata["key_findings"].as_array().map(|a| a.len()), Some(3));
    }

    #[test]
    fn test_merge_child_results_caps_parent_findings_at_fifty() {
        let mut parent =
            crate::state::AgentState::new("parent-1".into(), "tenant-1".into(), "parent".into(), "/tmp/ws".into());
        parent.pending_children = vec!["child-1".into()];
        parent.metadata["key_findings"] =
            serde_json::Value::Array((0..49).map(|i| serde_json::json!(format!("old-{i}"))).collect());

        let child = child_state("child-1", "done", &["new-1", "new-2"]);
        merge_child_results_into_parent(&mut parent, &[child]);

        let findings = parent.metadata["key_findings"].as_array().expect("findings should be array");
        assert_eq!(findings.len(), 50);
        assert_eq!(findings.last(), Some(&serde_json::json!("new-2")));
    }
}
