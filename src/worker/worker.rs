use std::sync::Arc;

use anyhow::Result;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    agent::{
        definition::{AgentDefinition, PlanModeMessage, PlanModePhase, PlanModeSession, RoleStatus},
        planner::Plan,
        prompts::StepHistory,
        r#loop::{AgentLoop, StepOutcome},
        workflow_compiler::{data_signature_from_value, FailureKind, RecompilePolicy},
    },
    events::{AgentEvent, EventBus},
    metrics::Metrics,
    scheduler::queue::{ExecutionTask, Queue},
    segments::AgentServices,
    state::GoalInstanceStatus,
    storage::PostgresStore,
    workspace::manager::WorkspaceManager,
};

pub struct Worker {
    id: usize,
    name: String,
    store: Arc<PostgresStore>,
    queue: Arc<dyn Queue>,
    agent_loop: Arc<AgentLoop>,
    dag_engine: Arc<crate::agent::dag_engine::DagEngine>,
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

fn classify_recompile_failure(reason: &str) -> Option<FailureKind> {
    let lower = reason.to_ascii_lowercase();

    if lower.contains("permission denied")
        || lower.contains("access denied")
        || lower.contains("policy")
        || lower.contains("forbidden")
        || lower.contains("plane guard")
        || lower.contains("exceeded safety limits")
    {
        return Some(FailureKind::Policy);
    }

    if lower.contains("invalid schema")
        || lower.contains("state_schema")
        || lower.contains("type mismatch")
        || lower.contains("typed expression")
        || lower.contains("missing required entry")
        || lower.contains("depends on unknown step")
        || lower.contains("unknown next step")
        || lower.contains("entry_step")
        || lower.contains("missing resource")
        || lower.contains("resource binding")
        || lower.contains("unsupported operation")
        || lower.contains("tool not found")
        || lower.contains("missing tool")
        || lower.contains("placeholder")
        || lower.contains("unresolved")
        || lower.contains("binding")
        || lower.contains("connector")
        || lower.contains("api key")
        || lower.contains("oauth")
        || lower.contains("credential")
    {
        return Some(FailureKind::Structural);
    }

    None
}

fn outcome_recompile_request(outcome: &StepOutcome, state: &crate::state::AgentState) -> Option<(FailureKind, String)> {
    let (kind, reason) = match outcome {
        StepOutcome::PermanentError { reason } => Some((FailureKind::Structural, reason.clone())),
        StepOutcome::PolicyViolation { reason } => Some((FailureKind::Policy, reason.clone())),
        StepOutcome::Failed(reason) => classify_recompile_failure(reason).map(|kind| (kind, reason.clone())),
        _ => None,
    }?;

    if should_request_recompile(state, &kind) {
        Some((kind, reason))
    } else {
        None
    }
}

fn should_request_recompile(state: &crate::state::AgentState, kind: &FailureKind) -> bool {
    let policy = state
        .metadata
        .get("recompile_policy")
        .cloned()
        .and_then(|value| serde_json::from_value::<RecompilePolicy>(value).ok());

    let Some(policy) = policy else {
        return matches!(kind, FailureKind::Structural | FailureKind::Policy);
    };

    if policy.ignore_on.contains(kind) {
        return false;
    }
    if !policy.trigger_on.is_empty() {
        return policy.trigger_on.contains(kind);
    }
    matches!(kind, FailureKind::Structural | FailureKind::Policy)
}

fn mark_recompile_requested(state: &mut crate::state::AgentState, kind: FailureKind, reason: String) {
    let mut recompile_count = state.metadata.get("recompile_count").and_then(|value| value.as_u64()).unwrap_or(0);
    recompile_count = recompile_count.saturating_add(1);
    state.metadata["needs_recompile"] = serde_json::json!(true);
    state.metadata["recompile_count"] = serde_json::json!(recompile_count);
    state.metadata["recompile_failure_kind"] = serde_json::json!(kind);
    state.metadata["recompile_reason"] = serde_json::json!(reason);
    state.metadata["recompile_requested_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
    state.metadata["recompile_mode"] = serde_json::json!("fork");
    if let Some(version) = state.metadata.get("workflow_version").cloned() {
        state.metadata["failed_workflow_version"] = version;
    }
    if let Some(parent) = state.metadata.get("parent_workflow_version").cloned() {
        state.metadata["recompile_parent_workflow_version"] = parent;
    }
    if let Some(input_data) = state.metadata.get("input_data").cloned() {
        state.metadata["recompile_data_signature"] =
            serde_json::to_value(data_signature_from_value(&input_data)).unwrap_or_default();
    }
}

async fn load_original_plan_mode_session(
    store: &Arc<PostgresStore>,
    state: &crate::state::AgentState,
) -> Result<Option<PlanModeSession>> {
    if let Some(session_id) = state.metadata.get("plan_mode_session_id").and_then(|value| value.as_str()) {
        if let Some(session) = store.get_plan_mode_session(&state.tenant_id, session_id).await? {
            return Ok(Some(session));
        }
    }

    if let Some(goal_fingerprint) = state.metadata.get("plan_mode_goal_fingerprint").and_then(|value| value.as_str()) {
        if let Some(session) =
            store.get_latest_plan_mode_session_by_goal_fingerprint(&state.tenant_id, goal_fingerprint).await?
        {
            return Ok(Some(session));
        }
    }

    if let Some(goal_fingerprint) = state.metadata.get("recompile_goal_fingerprint").and_then(|value| value.as_str()) {
        if let Some(session) =
            store.get_latest_plan_mode_session_by_goal_fingerprint(&state.tenant_id, goal_fingerprint).await?
        {
            return Ok(Some(session));
        }
    }

    Ok(None)
}

async fn create_recompile_plan_mode_session(
    store: &Arc<PostgresStore>,
    event_bus: &Arc<EventBus>,
    state: &mut crate::state::AgentState,
    kind: &FailureKind,
    reason: &str,
) -> Result<Option<String>> {
    let Some(role_id) = state.metadata.get("role_id").and_then(|value| value.as_str()).map(str::to_string) else {
        return Ok(None);
    };

    let Some(mut role) = store.get_agent_role(&state.tenant_id, &role_id).await? else {
        return Ok(None);
    };

    let agent_definition = store
        .get_agent_definition(&state.tenant_id, &role.agent_id)
        .await?
        .unwrap_or_else(|| AgentDefinition::new(role.agent_id.clone(), state.tenant_id.clone(), role.name.clone()));
    let original_session = load_original_plan_mode_session(store, state).await?;

    let now = chrono::Utc::now();
    let draft_agent_id = Uuid::new_v4().to_string();
    let draft_role_id = Uuid::new_v4().to_string();
    let session_id = Uuid::new_v4().to_string();

    let source_agent =
        original_session.as_ref().map(|session| session.draft_agent.clone()).unwrap_or(agent_definition.clone());
    let AgentDefinition { name, persona, connectors, constraints, .. } = source_agent;

    let mut draft_agent = AgentDefinition::new(draft_agent_id.clone(), state.tenant_id.clone(), name);
    draft_agent.persona = persona;
    draft_agent.connectors = connectors;
    draft_agent.constraints = constraints;
    draft_agent.memory_ref = format!("agent:{}", &draft_agent.id[..8]);

    role.id = draft_role_id;
    role.agent_id = draft_agent.id.clone();
    role.tenant_id = state.tenant_id.clone();
    role.status = RoleStatus::Draft;
    role.version = role.version.saturating_add(1);
    role.updated_at = now;
    role.created_at = now;

    let input_data = state.metadata.get("input_data").cloned().unwrap_or_else(|| serde_json::json!({}));
    let data_signature = data_signature_from_value(&input_data);
    let workflow_version = state
        .metadata
        .get("workflow_version")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| role.version.to_string());
    let variant_id = state.metadata.get("workflow_variant_id").and_then(|value| value.as_str()).map(str::to_string);
    let workflow_execution_profile = state.metadata.get("workflow_execution_profile").cloned().unwrap_or_default();
    let source_goal_fingerprint = original_session
        .as_ref()
        .and_then(|session| session.goal_fingerprint.clone())
        .or_else(|| {
            state.metadata.get("plan_mode_goal_fingerprint").and_then(|value| value.as_str()).map(str::to_string)
        })
        .or_else(|| {
            state.metadata.get("recompile_goal_fingerprint").and_then(|value| value.as_str()).map(str::to_string)
        });
    let repaired_conversation =
        original_session.as_ref().map(|session| session.conversation.clone()).unwrap_or_default();
    let repaired_attachments = original_session.as_ref().map(|session| session.attachments.clone()).unwrap_or_default();
    let repaired_attachment_context =
        original_session.as_ref().map(|session| session.attachment_context.clone()).unwrap_or_default();
    let repaired_session_workspace = original_session.as_ref().and_then(|session| session.session_workspace.clone());
    let repaired_intent_cache =
        original_session.as_ref().and_then(|session| session.intent_cache.clone()).or_else(|| {
            Some(serde_json::json!({
                "goal": state.goal.clone(),
                "recompile_reason": reason,
                "recompile_failure_kind": format!("{:?}", kind),
                "failed_workflow_version": state.metadata.get("failed_workflow_version").cloned(),
                "parent_workflow_version": state.metadata.get("recompile_parent_workflow_version").cloned(),
                "workflow_variant_id": variant_id.clone(),
                "workflow_data_signature": data_signature.clone(),
                "workflow_execution_profile": workflow_execution_profile.clone(),
            }))
        });
    let repaired_pending_steps =
        original_session.as_ref().map(|session| session.pending_steps.clone()).unwrap_or_default();
    let fingerprint_payload = serde_json::json!({
        "goal": state.goal.clone(),
        "workflow_version": workflow_version,
        "reason": reason,
        "failure_kind": format!("{:?}", kind),
        "data_signature": data_signature.clone(),
        "variant_id": variant_id.clone(),
        "workflow_execution_profile": workflow_execution_profile.clone(),
    });
    let fingerprint =
        format!("pmg_{}", hex::encode(Sha256::digest(serde_json::to_vec(&fingerprint_payload).unwrap_or_default())));
    let goal_fingerprint = source_goal_fingerprint.unwrap_or_else(|| fingerprint.clone());
    let repair_version =
        original_session.as_ref().map(|session| session.repair_version.saturating_add(1)).unwrap_or_else(|| {
            state.metadata.get("recompile_count").and_then(|value| value.as_u64()).unwrap_or(1).max(1) as u32
        });
    let reused_from_session_id = original_session
        .as_ref()
        .map(|session| session.id.clone())
        .or_else(|| state.metadata.get("plan_mode_session_id").and_then(|value| value.as_str()).map(str::to_string));
    let repair_root_session_id = original_session
        .as_ref()
        .and_then(|session| session.repair_root_session_id.clone())
        .or_else(|| reused_from_session_id.clone())
        .or_else(|| Some(session_id.clone()));

    let session = PlanModeSession {
        id: session_id.clone(),
        tenant_id: state.tenant_id.clone(),
        draft_agent: draft_agent.clone(),
        draft_role: Some(role),
        conversation: {
            let mut conversation = repaired_conversation;
            conversation.push(PlanModeMessage {
                role: "assistant".into(),
                content: format!(
                    "Runtime marked this workflow for recompilation. Failure kind: {:?}. Reason: {}. Open this session to repair the compiled workflow.",
                    kind, reason
                ),
            });
            conversation
        },
        attachments: repaired_attachments,
        attachment_context: repaired_attachment_context,
        session_workspace: repaired_session_workspace,
        goal_fingerprint: Some(goal_fingerprint),
        repair_version,
        reused_from_session_id,
        repair_root_session_id,
        phase: PlanModePhase::Reviewing,
        intent_cache: repaired_intent_cache,
        pending_steps: repaired_pending_steps,
        created_at: now,
        updated_at: now,
    };

    state.metadata["recompile_plan_mode_session_id"] = serde_json::json!(session.id.clone());
    state.metadata["recompile_plan_mode_agent_id"] = serde_json::json!(draft_agent.id.clone());
    state.metadata["recompile_goal_fingerprint"] = serde_json::json!(fingerprint);
    state.metadata["recompile_reused_from_session_id"] = serde_json::json!(session.reused_from_session_id.clone());
    state.metadata["recompile_repair_root_session_id"] = serde_json::json!(session.repair_root_session_id.clone());
    state.metadata["recompile_phase"] = serde_json::json!("reviewing");

    store.upsert_agent_definition(&draft_agent).await?;
    store.upsert_plan_mode_session(&session).await?;

    event_bus.publish(AgentEvent::RecompileRequested {
        agent_id: state.id.clone(),
        reason: reason.to_string(),
        failure_kind: format!("{:?}", kind).to_ascii_lowercase(),
        failed_workflow_version: state
            .metadata
            .get("failed_workflow_version")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        parent_workflow_version: state
            .metadata
            .get("recompile_parent_workflow_version")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        data_signature: state.metadata.get("recompile_data_signature").cloned(),
        plan_mode_session_id: Some(session.id.clone()),
        variant_id,
    });

    Ok(Some(session.id))
}

impl Worker {
    pub fn new(
        id: usize,
        name: String,
        store: Arc<PostgresStore>,
        queue: Arc<dyn Queue>,
        agent_loop: Arc<AgentLoop>,
        dag_engine: Arc<crate::agent::dag_engine::DagEngine>,
        metrics: Arc<Metrics>,
        workspace_manager: Arc<WorkspaceManager>,
        services: Arc<AgentServices>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self { id, name, store, queue, agent_loop, dag_engine, metrics, workspace_manager, services, event_bus }
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
        self.persist_goal_instance_status(
            &state,
            GoalInstanceStatus::Running,
            None,
            None,
            state.current_task.clone(),
            plan.as_ref().map(|p| p.steps.len() as u32),
        )
        .await;

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

        let outcome = if state.workflow_id.is_some() {
            let dag_outcome = self.dag_engine.run_workflow(&state, task_cancel_token.clone()).await;
            task_cancel_token.cancel();
            let _ = heartbeat_handle.await;

            match dag_outcome {
                Ok(crate::agent::dag_engine::WorkflowOutcome::Completed) => {
                    state.workflow_id = None;
                    state.mark_completed();
                    StepOutcome::Complete
                }
                Ok(crate::agent::dag_engine::WorkflowOutcome::Failed(reason)) => {
                    state.workflow_id = None;
                    state.mark_failed();
                    state.metadata["last_worker_error"] = serde_json::json!(reason);
                    if let Some(kind) = classify_recompile_failure(&reason) {
                        mark_recompile_requested(&mut state, kind.clone(), reason.clone());
                        let _ = create_recompile_plan_mode_session(
                            &self.store,
                            &self.event_bus,
                            &mut state,
                            &kind,
                            &reason,
                        )
                        .await?;
                    }
                    StepOutcome::Failed(format!("DAG workflow failed: {}", reason))
                }
                Ok(crate::agent::dag_engine::WorkflowOutcome::Cancelled) => {
                    state.workflow_id = None;
                    state.mark_failed();
                    state.metadata["last_worker_error"] = serde_json::json!("DAG workflow cancelled");
                    StepOutcome::Failed("DAG workflow cancelled".into())
                }
                Err(error) => {
                    state.workflow_id = None;
                    state.mark_failed();
                    let reason = error.to_string();
                    state.metadata["last_worker_error"] = serde_json::json!(reason.clone());
                    if let Some(kind) = classify_recompile_failure(&reason) {
                        mark_recompile_requested(&mut state, kind.clone(), reason.clone());
                        let _ = create_recompile_plan_mode_session(
                            &self.store,
                            &self.event_bus,
                            &mut state,
                            &kind,
                            &reason,
                        )
                        .await?;
                    }
                    StepOutcome::Failed(format!("DAG engine error: {:#}", error))
                }
            }
        } else {
            match self.agent_loop.run_step(&mut state, &mut plan, &mut history).await {
                Ok(outcome) => {
                    task_cancel_token.cancel();
                    let _ = heartbeat_handle.await;
                    outcome
                }
                Err(error) => {
                    task_cancel_token.cancel();
                    let _ = heartbeat_handle.await;
                    state.plan = plan.clone();
                    state.metadata["step_history"] = serde_json::to_value(&history).unwrap_or_default();
                    let reason = error.to_string();
                    state.metadata["last_worker_error"] = serde_json::json!(reason.clone());
                    if let Some(kind) = classify_recompile_failure(&reason) {
                        mark_recompile_requested(&mut state, kind.clone(), reason.clone());
                        let _ = create_recompile_plan_mode_session(
                            &self.store,
                            &self.event_bus,
                            &mut state,
                            &kind,
                            &reason,
                        )
                        .await?;
                    }
                    state.mark_failed();
                    state.set_execution_checkpoint(&task.id, task.attempt, state.current_step, "failed");
                    self.store.upsert_agent(&state).await?;
                    self.persist_goal_instance_status(
                        &state,
                        GoalInstanceStatus::Failed,
                        Some(serde_json::json!({ "error": reason.clone() })),
                        Some(reason.clone()),
                        Some(reason),
                        plan.as_ref().map(|p| p.steps.len() as u32),
                    )
                    .await;
                    return Err(error);
                }
            }
        };

        if let Some((kind, reason)) = outcome_recompile_request(&outcome, &state) {
            mark_recompile_requested(&mut state, kind.clone(), reason.clone());
            let _ =
                create_recompile_plan_mode_session(&self.store, &self.event_bus, &mut state, &kind, &reason).await?;
        }

        state.plan = plan.clone();
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
                self.persist_goal_instance_status(
                    &state,
                    GoalInstanceStatus::Running,
                    None,
                    None,
                    Some(format!("rescheduled in {} seconds", delay_secs)),
                    plan.as_ref().map(|p| p.steps.len() as u32),
                )
                .await;
                tracing::info!(agent_id = %task.agent_id, step = state.current_step, delay_secs, "step done - rescheduled");
            }
            StepOutcome::PlanApprovalNeeded => {
                self.persist_goal_instance_status(
                    &state,
                    GoalInstanceStatus::Running,
                    None,
                    None,
                    Some("plan approval needed".into()),
                    plan.as_ref().map(|p| p.steps.len() as u32),
                )
                .await;
                tracing::info!(agent_id = %task.agent_id, "plan created - awaiting user approval");
            }
            StepOutcome::Complete => {
                let result = state.metadata.get("final_output").cloned().unwrap_or_else(|| {
                    serde_json::json!({
                        "final_answer": state.final_answer().unwrap_or(""),
                        "goal": state.goal,
                    })
                });
                self.persist_goal_instance_status(
                    &state,
                    GoalInstanceStatus::Completed,
                    Some(result),
                    None,
                    state.final_answer().map(str::to_string).or_else(|| Some("goal complete".into())),
                    plan.as_ref().map(|p| p.steps.len() as u32),
                )
                .await;
                tracing::info!(agent_id = %task.agent_id, "goal complete");
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
                                tracing::info!(role_id = %role_id, role_name = %role_name, new_goals = spawned, "workforce event dispatcher processed role completion");
                            }
                            Err(e) => {
                                tracing::error!(role_id = %role_id, error = %e, "workforce event dispatcher failed");
                            }
                        }
                    });
                }
            }
            StepOutcome::PartiallyComplete { note } => {
                let result = state
                    .metadata
                    .get("partial_completion_result")
                    .cloned()
                    .or_else(|| state.metadata.get("final_output").cloned())
                    .unwrap_or_else(|| serde_json::json!({}));
                self.persist_goal_instance_status(
                    &state,
                    GoalInstanceStatus::PartiallyComplete,
                    Some(result),
                    Some(note.clone()),
                    Some(note.clone()),
                    plan.as_ref().map(|p| p.steps.len() as u32),
                )
                .await;
                tracing::warn!(agent_id = %task.agent_id, note = %note, "goal partially complete");
                self.archive_workspace_async(&task.agent_id, &state.tenant_id);
                self.package_evidence_async(
                    task.agent_id.clone(),
                    state.tenant_id.clone(),
                    state.goal.clone(),
                    "partially_complete".into(),
                );
                if let Some(goal_instance_id) =
                    state.metadata.get("goal_instance_id").and_then(|v| v.as_str()).map(String::from)
                {
                    let store = Arc::clone(&self.store);
                    let tenant = state.tenant_id.clone();
                    spawn_savings_estimation(store, tenant, goal_instance_id);
                }
            }
            StepOutcome::Failed(reason) => {
                self.persist_goal_instance_status(
                    &state,
                    GoalInstanceStatus::Failed,
                    Some(serde_json::json!({ "error": reason })),
                    Some(reason.clone()),
                    Some(reason.clone()),
                    plan.as_ref().map(|p| p.steps.len() as u32),
                )
                .await;
                tracing::error!(agent_id = %task.agent_id, reason = %reason, "agent failed");
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

    async fn persist_goal_instance_status(
        &self,
        state: &crate::state::AgentState,
        status: GoalInstanceStatus,
        result: Option<serde_json::Value>,
        failure_reason: Option<String>,
        last_message: Option<String>,
        total_steps: Option<u32>,
    ) {
        let Some(goal_instance_id) = state.metadata.get("goal_instance_id").and_then(|value| value.as_str()) else {
            return;
        };

        let Ok(Some(mut goal_instance)) = self.store.get_goal_instance(&state.tenant_id, goal_instance_id).await else {
            return;
        };

        goal_instance.agent_state_id = Some(state.id.clone());
        goal_instance.current_step = state.current_step;
        if let Some(total_steps) = total_steps {
            goal_instance.total_steps = total_steps;
        }
        if let Some(last_message) = last_message {
            goal_instance.last_message = Some(last_message);
        }

        match status {
            GoalInstanceStatus::Running => {
                goal_instance.mark_running(state.id.clone());
                goal_instance.current_step = state.current_step;
                if let Some(total_steps) = total_steps {
                    goal_instance.total_steps = total_steps;
                }
                goal_instance.result = None;
                goal_instance.failure_reason = None;
            }
            GoalInstanceStatus::Completed => {
                goal_instance.mark_completed(result.unwrap_or_else(|| serde_json::json!({})));
            }
            GoalInstanceStatus::PartiallyComplete => {
                goal_instance.mark_partially_complete(
                    failure_reason.clone().unwrap_or_else(|| "partially complete".into()),
                    result.unwrap_or_else(|| serde_json::json!({})),
                );
            }
            GoalInstanceStatus::Failed => {
                goal_instance.mark_failed(failure_reason.clone().unwrap_or_else(|| "failed".into()));
            }
            GoalInstanceStatus::Cancelled => {
                goal_instance.status = GoalInstanceStatus::Cancelled;
                goal_instance.failure_reason = failure_reason;
                goal_instance.updated_at = chrono::Utc::now();
                goal_instance.completed_at = Some(chrono::Utc::now());
            }
            GoalInstanceStatus::Pending => {
                goal_instance.status = GoalInstanceStatus::Pending;
            }
        }

        if let Err(error) = self.store.upsert_goal_instance(&goal_instance).await {
            tracing::warn!(goal_instance_id = %goal_instance_id, error = %error, "failed to persist goal instance status");
        }
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
                let goal_instance_id =
                    state.metadata.get("goal_instance_id").and_then(|value| value.as_str()).map(str::to_string);
                state.mark_failed();
                let _ = store.upsert_agent(&state).await;

                if let Some(goal_instance_id) = goal_instance_id {
                    if let Ok(Some(mut goal_instance)) =
                        store.get_goal_instance(&state.tenant_id, &goal_instance_id).await
                    {
                        goal_instance.agent_state_id = Some(state.id.clone());
                        goal_instance.current_step = state.current_step;
                        goal_instance.total_steps = state
                            .plan
                            .as_ref()
                            .map(|plan| plan.steps.len() as u32)
                            .unwrap_or(goal_instance.total_steps);
                        goal_instance.mark_failed(
                            state
                                .metadata
                                .get("last_worker_error")
                                .and_then(|value| value.as_str())
                                .unwrap_or("worker failed")
                                .to_string(),
                        );
                        let _ = store.upsert_goal_instance(&goal_instance).await;
                    }
                }
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
                foreach: None,
                index: 0,
                description: "Inspect workflow".into(),
                tool: Some("file_read".into()),
                tool_args: None,
                success_criteria: "reviewed".into(),
                condition: None,
                depends_on: vec![],
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
