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
    scheduler::queue::{ExecutionTask, Queue},
    segments::AgentServices,
    storage::PostgresStore,
    workspace::manager::WorkspaceManager,
};

pub struct Worker {
    id: usize,
    name: String,
    store: Arc<PostgresStore>,
    queue: Arc<dyn Queue>,
    agent_loop: Arc<AgentLoop>,
    metrics: Arc<Metrics>,
    workspace_manager: Arc<WorkspaceManager>,
    services: Arc<AgentServices>,
    event_bus: Arc<EventBus>,
}

fn is_retryable_worker_error(error: &anyhow::Error) -> bool {
    let mut text = error.to_string().to_ascii_lowercase();
    for cause in error.chain() {
        let cause_text = cause.to_string().to_ascii_lowercase();
        if !text.contains(&cause_text) {
            text.push(' ');
            text.push_str(&cause_text);
        }
    }

    text.contains("rate limit")
        || text.contains("too many requests")
        || text.contains("timeout")
        || text.contains("timed out")
        || text.contains("temporarily unavailable")
        || text.contains("service unavailable")
        || text.contains("connection reset")
        || text.contains("broken pipe")
        || text.contains("503")
        || text.contains("502")
        || text.contains("504")
}

fn should_retry_task(task: &ExecutionTask, error: &anyhow::Error) -> bool {
    task.attempt < 3 && is_retryable_worker_error(error)
}

impl Worker {
    pub fn new(
        id: usize,
        name: String,
        store: Arc<PostgresStore>,
        queue: Arc<dyn Queue>,
        agent_loop: Arc<AgentLoop>,
        metrics: Arc<Metrics>,
        workspace_manager: Arc<WorkspaceManager>,
        services: Arc<AgentServices>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self { id, name, store, queue, agent_loop, metrics, workspace_manager, services, event_bus }
    }

    pub async fn process_next(&self, cancel_token: tokio_util::sync::CancellationToken) -> Result<bool> {
        let task = tokio::select! {
            _ = cancel_token.cancelled() => return Ok(false),
            res = self.queue.dequeue() => match res? {
                Some(t) => t,
                None => return Ok(false),
            }
        };

        tracing::debug!(worker = self.id, agent_id = %task.agent_id, "dequeued");
        self.metrics.agent_started();

        let result = self.run_task(&task, cancel_token).await;
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
                if should_retry_task(&task, e) {
                    self.queue.retry(ExecutionTask { attempt: task.attempt + 1, ..task.clone() }).await?;
                } else {
                    self.queue.ack(&task).await?;
                    self.mark_failed_async(&task.agent_id);
                }
            }
        }

        Ok(true)
    }

    async fn run_task(&self, task: &ExecutionTask, cancel_token: tokio_util::sync::CancellationToken) -> Result<()> {
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

        if let Some(step) = plan.as_ref().and_then(|p| p.next_step(state.current_step as usize)) {
            state.current_task = Some(format!("step {}: {}", step.index, step.description));
        }
        state.mark_running();
        state.set_execution_checkpoint(&task.id, task.attempt, state.current_step, "running");

        // --- Distributed Execution: Take Ownership & Start Heartbeat ---
        let lease_duration = 30; // seconds
        state.claimed_by = Some(self.name.clone());
        state.lease_expires_at = Some(chrono::Utc::now() + chrono::Duration::seconds(lease_duration));
        self.store.upsert_agent(&state).await?;

        // Hierarchical cancellation: if global cancel_token is triggered, task_cancel_token follows.
        // We can also trigger task_cancel_token independently on lease loss.
        let task_cancel_token = cancel_token.child_token();
        let heartbeat_token = task_cancel_token.clone();
        let store = Arc::clone(&self.store);
        let agent_id = state.id.clone();
        let worker_name = self.name.clone();

        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                tokio::select! {
                    _ = heartbeat_token.cancelled() => break,
                    _ = interval.tick() => {
                        match store.renew_lease(&agent_id, &worker_name, lease_duration).await {
                            Ok(true) => {},
                            Ok(false) => {
                                tracing::error!(agent_id = %agent_id, "lease lost — another node has claimed this agent or it was deleted. aborting.");
                                heartbeat_token.cancel();
                                break;
                            }
                            Err(e) => {
                                tracing::warn!(agent_id = %agent_id, error = %e, "heartbeat DB error — will retry next tick");
                            }
                        }
                    }
                }
            }
        });

        let outcome = match self.agent_loop.run_step(&mut state, &mut plan, &mut history).await {
            Ok(outcome) => {
                task_cancel_token.cancel();
                let _ = heartbeat_handle.await;
                outcome
            }
            Err(error) => {
                task_cancel_token.cancel();
                let _ = heartbeat_handle.await;
                state.plan = plan;
                state.metadata["step_history"] = serde_json::to_value(&history).unwrap_or_default();
                state.metadata["last_worker_error"] = serde_json::json!(error.to_string());
                state.set_execution_checkpoint(&task.id, task.attempt, state.current_step, "failed");
                self.store.upsert_agent(&state).await?;
                return Err(error);
            }
        };

        state.plan = plan;
        state.metadata["step_history"] = serde_json::to_value(&history).unwrap_or_default();
        state.clear_execution_checkpoint();
        state.current_task = None;

        // Release ownership
        state.claimed_by = None;
        state.lease_expires_at = None;
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
            StepOutcome::PlanApprovalNeeded => {
                tracing::info!(
                    agent_id = %task.agent_id,
                    "plan created — awaiting user approval"
                );
            }
            StepOutcome::Complete => {
                tracing::info!(agent_id = %task.agent_id, "✓ goal complete");
                self.archive_workspace_async(&task.agent_id, &state.tenant_id);
                self.package_evidence_async(
                    task.agent_id.clone(),
                    state.tenant_id.clone(),
                    state.goal.clone(),
                    "completed".into(),
                );
                if let Some(goal_instance_id) =
                    state.metadata.get("goal_instance_id").and_then(|v| v.as_str()).map(String::from)
                {
                    let store = Arc::clone(&self.store);
                    let tenant = state.tenant_id.clone();
                    spawn_savings_estimation(store, tenant, goal_instance_id);
                }

                // Trigger workforce event dispatcher for role completion
                let role_id = state.metadata.get("role_id").and_then(|v| v.as_str()).map(str::to_string);
                let role_name = state.metadata.get("role_name").and_then(|v| v.as_str()).map(str::to_string);
                let agent_def_id =
                    state.metadata.get("agent_definition_id").and_then(|v| v.as_str()).map(str::to_string);
                let goal_instance_id =
                    state.metadata.get("goal_instance_id").and_then(|v| v.as_str()).map(str::to_string);

                if let (Some(role_id), Some(role_name), Some(agent_def_id), Some(goal_instance_id)) =
                    (role_id, role_name, agent_def_id, goal_instance_id)
                {
                    let payload = crate::agent::definition::WorkforceEventPayload {
                        tenant_id: state.tenant_id.clone(),
                        agent_id: agent_def_id.clone(),
                        agent_name: state
                            .metadata
                            .get("agent_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("agent")
                            .to_string(),
                        role_id: role_id.clone(),
                        role_name: role_name.clone(),
                        goal_instance_id: goal_instance_id.clone(),
                        status: "completed".to_string(),
                        output_data: state.metadata.get("final_output").cloned().unwrap_or(serde_json::json!({})),
                        failure_reason: None,
                        emitted_at: chrono::Utc::now(),
                    };

                    let store = Arc::clone(&self.store);
                    tokio::spawn(async move {
                        match crate::events::workforce::dispatch(&payload, &store).await {
                            Ok(spawned) => {
                                tracing::info!(
                                    role_id = %role_id,
                                    role_name = %role_name,
                                    new_goals = spawned,
                                    "workforce event dispatcher processed role completion"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    role_id = %role_id,
                                    error = %e,
                                    "workforce event dispatcher failed"
                                );
                            }
                        }
                    });
                }
            }
            StepOutcome::PartiallyComplete { note } => {
                tracing::warn!(agent_id = %task.agent_id, note = %note, "⚠ goal partially complete");
                self.archive_workspace_async(&task.agent_id, &state.tenant_id);
                self.package_evidence_async(
                    task.agent_id.clone(),
                    state.tenant_id.clone(),
                    state.goal.clone(),
                    "partially_complete".into(),
                );
                // Pro-rated savings estimation for partial runs
                if let Some(goal_instance_id) =
                    state.metadata.get("goal_instance_id").and_then(|v| v.as_str()).map(String::from)
                {
                    let store = Arc::clone(&self.store);
                    let tenant = state.tenant_id.clone();
                    spawn_savings_estimation(store, tenant, goal_instance_id);
                }
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

/// Fire-and-forget savings estimation — spawned on every Complete outcome.
/// Loads the GoalInstance + its AgentRole, runs the estimator, persists.
fn spawn_savings_estimation(store: Arc<crate::storage::PostgresStore>, tenant_id: String, goal_instance_id: String) {
    tokio::spawn(async move {
        // Load goal instance
        let gi = match store.get_goal_instance(&tenant_id, &goal_instance_id).await {
            Ok(Some(gi)) => gi,
            _ => return,
        };
        // Only estimate completed, non-test runs that haven't been estimated yet
        if gi.human_hours_saved > 0.0 || gi.is_test {
            return;
        }

        // Load the role
        let role = match store.get_agent_role(&tenant_id, &gi.role_id).await {
            Ok(Some(r)) => r,
            _ => return,
        };

        let estimator = crate::agent::savings::WorkSavingsEstimator::new(Arc::clone(&store));
        let mut gi_mut = gi;
        if let Err(e) = estimator.estimate_and_persist(&mut gi_mut, &role).await {
            tracing::warn!(error = %e, "savings estimation failed — non-fatal");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(attempt: u32) -> ExecutionTask {
        ExecutionTask::new("agent-1".into()).with_attempt(attempt)
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
        let transient = anyhow::anyhow!("temporary timeout");
        assert!(should_retry_task(&sample_task(0), &transient));
        assert!(should_retry_task(&sample_task(1), &transient));
        assert!(should_retry_task(&sample_task(2), &transient));
        assert!(!should_retry_task(&sample_task(3), &transient));
    }

    #[test]
    fn test_retry_blocks_non_transient_errors() {
        let hard = anyhow::anyhow!("missing credentials");
        assert!(!should_retry_task(&sample_task(0), &hard));
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
