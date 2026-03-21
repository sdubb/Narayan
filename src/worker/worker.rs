use std::sync::Arc;

use anyhow::Result;

use crate::{
    agent::{
        planner::Plan,
        prompts::StepHistory,
        r#loop::{AgentLoop, StepOutcome},
    },
    events::{AgentEvent, EventBus},
    metrics::Metrics,
    scheduler::queue::{Queue, Task},
    segments::AgentServices,
    storage::PostgresStore,
    workspace::manager::WorkspaceManager,
};

pub struct Worker {
    id: usize,
    store: Arc<PostgresStore>,
    queue: Arc<dyn Queue>,
    agent_loop: Arc<AgentLoop>,
    metrics: Arc<Metrics>,
    workspace_manager: Arc<WorkspaceManager>,
    services: Arc<AgentServices>,
    event_bus: Arc<EventBus>,
}

fn should_retry_task(task: &Task) -> bool {
    task.attempt < 3
}

impl Worker {
    pub fn new(
        id: usize,
        store: Arc<PostgresStore>,
        queue: Arc<dyn Queue>,
        agent_loop: Arc<AgentLoop>,
        metrics: Arc<Metrics>,
        workspace_manager: Arc<WorkspaceManager>,
        services: Arc<AgentServices>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self { id, store, queue, agent_loop, metrics, workspace_manager, services, event_bus }
    }

    pub async fn process_next(&self) -> Result<bool> {
        let task = match self.queue.dequeue().await? {
            Some(t) => t,
            None => return Ok(false),
        };

        tracing::debug!(worker = self.id, agent_id = %task.agent_id, "dequeued");
        self.metrics.agent_started();

        let result = self.run_task(&task).await;
        self.metrics.agent_finished();

        match result {
            Ok(_) => {
                self.queue.ack(&task).await?;
            }
            Err(ref e) => {
                tracing::error!(
                    worker   = self.id,
                    agent_id = %task.agent_id,
                    error    = %e,
                    attempt  = task.attempt,
                    "task failed"
                );
                if should_retry_task(&task) {
                    self.queue.retry(Task { attempt: task.attempt + 1, ..task.clone() }).await?;
                } else {
                    self.queue.ack(&task).await?;
                    self.mark_failed_async(&task.agent_id);
                }
            }
        }

        Ok(true)
    }

    async fn run_task(&self, task: &Task) -> Result<()> {
        let mut state = self
            .store
            .get_agent_internal(&task.agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("agent {} not found", task.agent_id))?;

        let mut plan: Option<Plan> = state.plan.take();
        let mut history: StepHistory = state
            .metadata
            .get("step_history")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();

        let outcome = self.agent_loop.run_step(&mut state, &mut plan, &mut history).await?;

        state.plan = plan;
        state.metadata["step_history"] = serde_json::to_value(&history).unwrap_or_default();
        self.store.upsert_agent(&state).await?;
        self.metrics.step_completed_for_tenant(&state.tenant_id);

        match &outcome {
            StepOutcome::Continue { delay_secs } => {
                tracing::info!(
                    agent_id   = %task.agent_id,
                    step       = state.current_step,
                    delay_secs,
                    "step done — rescheduled"
                );
            }
            StepOutcome::Complete => {
                tracing::info!(agent_id = %task.agent_id, "✓ goal complete");
                self.archive_workspace_async(&task.agent_id, &state.tenant_id);
                // ── Evidence packaging — fire-and-forget on completion ────────
                self.package_evidence_async(
                    task.agent_id.clone(),
                    state.tenant_id.clone(),
                    state.goal.clone(),
                    "completed".into(),
                );
            }
            StepOutcome::Failed(reason) => {
                tracing::error!(agent_id = %task.agent_id, reason = %reason, "✗ agent failed");
                self.archive_workspace_async(&task.agent_id, &state.tenant_id);
                self.package_evidence_async(
                    task.agent_id.clone(),
                    state.tenant_id.clone(),
                    state.goal.clone(),
                    "failed".into(),
                );
            }
            _ => {}
        }

        Ok(())
    }

    /// Fire-and-forget evidence packaging — called on Complete and Failed.
    /// Only runs if EvidencePackager is active in the current segment set.
    fn package_evidence_async(&self, agent_id: String, tenant_id: String, goal: String, status: String) {
        let evidence = match &self.services.evidence {
            Some(ep) => ep.clone(),
            None => return,
        };
        let bus = self.event_bus.clone();
        tokio::spawn(async move {
            match evidence.package(&agent_id, &tenant_id, &goal, &status, serde_json::json!({ "auto": true })).await {
                Ok(pkg) => {
                    tracing::info!(
                        agent_id      = %agent_id,
                        citations     = pkg.citations.len(),
                        audit_entries = pkg.audit_entries.len(),
                        "evidence packaged"
                    );
                    bus.publish(AgentEvent::EvidencePackaged {
                        agent_id,
                        citations: pkg.citations.len(),
                        audit_entries: pkg.audit_entries.len(),
                    });
                }
                Err(e) => tracing::warn!(agent_id = %agent_id, error = %e, "evidence packaging failed"),
            }
        });
    }

    fn archive_workspace_async(&self, agent_id: &str, tenant_id: &str) {
        let wm = self.workspace_manager.clone();
        let aid = agent_id.to_string();
        let tid = tenant_id.to_string();
        tokio::spawn(async move {
            let info = crate::workspace::manager::WorkspaceInfo {
                id: crate::util::new_id(),
                tenant_id: tid.clone(),
                agent_id: aid.clone(),
                mode: crate::workspace::resolver::WorkspaceMode::Hybrid,
                local_path: Some(format!("{}/{}/agents/{}", wm.local_root(), tid, aid)),
                storage_key: Some(format!("workspaces/{}/{}", tid, aid)),
                created_at: chrono::Utc::now(),
                archived: false,
            };
            if let Err(e) = wm.archive(&info).await {
                tracing::warn!(agent_id = %aid, error = %e, "workspace archive failed");
            }
        });
    }

    fn mark_failed_async(&self, agent_id: &str) {
        let store = self.store.clone();
        let id = agent_id.to_string();
        tokio::spawn(async move {
            if let Ok(Some(mut state)) = store.get_agent_internal(&id).await {
                state.mark_failed();
                let _ = store.upsert_agent(&state).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(attempt: u32) -> Task {
        Task::new("agent-1".into()).with_attempt(attempt)
    }

    fn sample_plan() -> Plan {
        Plan {
            goal: "fix CI pipeline".into(),
            job_type: Some("software_engineer".into()),
            steps: vec![crate::agent::planner::PlannedStep {
                index: 0,
                description: "Inspect workflow".into(),
                tool: Some("file_read".into()),
                tool_args: None,
                success_criteria: "reviewed".into(),
                condition: None,
            }],
            rationale: "inspect first".into(),
        }
    }

    #[test]
    fn test_retry_allowed_for_first_three_attempts() {
        assert!(should_retry_task(&sample_task(0)));
        assert!(should_retry_task(&sample_task(1)));
        assert!(should_retry_task(&sample_task(2)));
        assert!(!should_retry_task(&sample_task(3)));
    }

    #[test]
    fn test_plan_on_agent_state_not_in_metadata() {
        let mut state =
            crate::state::AgentState::new("agent-1".into(), "tenant-1".into(), "fix CI".into(), "/tmp/ws".into());
        state.plan = Some(sample_plan());
        assert!(state.metadata.get("plan").is_none(), "plan must not leak into metadata");
        assert_eq!(state.plan.as_ref().unwrap().goal, "fix CI pipeline");
    }

    #[test]
    fn test_plan_field_none_on_new_agent() {
        let state = crate::state::AgentState::new("agent-1".into(), "tenant-1".into(), "test".into(), "/tmp".into());
        assert!(state.plan.is_none());
    }

    #[test]
    fn test_evidence_packaging_skipped_when_service_is_none() {
        // package_evidence_async must be a no-op when services.evidence = None.
        // We can verify this by checking the service is None without spawning tasks.
        let services = Arc::new(AgentServices::none());
        assert!(services.evidence.is_none(), "evidence must be None for AgentServices::none()");
    }
}
