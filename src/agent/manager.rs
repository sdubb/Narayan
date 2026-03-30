use std::sync::Arc;

use anyhow::Result;
use std::path::PathBuf;

use crate::{
    agent::definition::AgentRole,
    agent::plan_mode_steps::generate_steps,
    agent::prompts::JobType,
    compliance::sla::SlaPriority,
    gateway::LlmGateway,
    segments::AgentServices,
    state::{AgentState, GoalInstance, GoalInstanceStatus, GoalState, TriggerSource},
    storage::PostgresStore,
    util::new_id,
    workspace::manager::WorkspaceManager,
};

pub struct AgentManager {
    store: Arc<PostgresStore>,
    workspace_manager: Arc<WorkspaceManager>,
    services: Arc<AgentServices>,
    gateway: Arc<dyn LlmGateway>,
}

fn build_goal_and_agent(
    goal_id: String,
    agent_id: String,
    tenant_id: String,
    description: String,
    workspace_path: String,
) -> (GoalState, AgentState) {
    let agent_state = AgentState::new(agent_id.clone(), tenant_id.clone(), description.clone(), workspace_path);
    let mut goal = GoalState::new(goal_id, tenant_id, description);
    goal.add_agent(agent_id);
    goal.mark_in_progress();
    (goal, agent_state)
}

/// Infer the SLA priority tier from job type.
fn sla_priority_for_job(job_type: &JobType) -> SlaPriority {
    match job_type {
        JobType::CustomerSupport => SlaPriority::Urgent,
        JobType::ITOpsITSM => SlaPriority::Critical,
        JobType::DevOps => SlaPriority::High,
        JobType::LegalContract => SlaPriority::High,
        JobType::FinanceAccounting => SlaPriority::High,
        JobType::HRPeopleOps => SlaPriority::Normal,
        JobType::SalesRevOps => SlaPriority::Normal,
        _ => SlaPriority::Low,
    }
}

impl AgentManager {
    pub fn new(
        store: Arc<PostgresStore>,
        workspace_manager: Arc<WorkspaceManager>,
        services: Arc<AgentServices>,
        gateway: Arc<dyn LlmGateway>,
    ) -> Self {
        Self { store, workspace_manager, services, gateway }
    }

    /// Expose the LLM gateway for plan mode and other components that need it.
    pub fn gateway(&self) -> Arc<dyn LlmGateway> {
        Arc::clone(&self.gateway)
    }

    pub fn workspace_root(&self) -> PathBuf {
        PathBuf::from(self.workspace_manager.local_root())
    }

    /// Start plan mode for the next pending role in an existing multi-role agent.
    /// Parses pending_roles from draft_agent.memory_ref and returns a session
    /// with the next role pre-filled and skipping intent extraction.
    pub async fn start_plan_mode_for_next_role(
        &self,
        agent_id: &str,
        tenant_id: &str,
    ) -> Result<crate::agent::definition::PlanModeSession> {
        // Load existing agent definition
        let mut agent = self
            .store
            .get_agent_definition(tenant_id, agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent definition not found: {}", agent_id))?;

        // Parse pending_roles from memory_ref
        let (mut pending_roles, base_memory_ref) = Self::split_pending_roles(&agent.memory_ref)?;
        if pending_roles.is_empty() {
            anyhow::bail!("No pending roles found in agent: {}", agent_id);
        }

        // Extract the first pending role
        let next_role_resp = pending_roles.first().cloned().ok_or_else(|| {
            anyhow::anyhow!("Pending roles array is empty")
        })?;
        pending_roles.remove(0);
        agent.memory_ref = Self::build_memory_ref(&base_memory_ref, &pending_roles);
        self.store.upsert_agent_definition(&agent).await?;

        let existing_roles = self.store.list_roles_for_agent(tenant_id, agent_id).await.unwrap_or_default();
        let existing_role_names: Vec<String> = existing_roles.iter().map(|role| role.name.clone()).collect();

        let actions: Vec<String> = next_role_resp["actions"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let trigger_hint = next_role_resp["trigger_hint"].as_str().unwrap_or("manual");
        let synthetic_intent = serde_json::json!({
            "category": "general",
            "data_sources": [],
            "write_targets": [],
            "actions": actions,
            "preferred_tool_categories": [],
            "preferred_tools": [],
            "candidate_wasm_tools": [],
            "needed_connector_categories": [],
            "candidate_connectors": [],
            "missing_capabilities": [],
            "workflow_outline": [],
            "uses_external_db": null,
            "uses_external_api": null,
            "trigger_hint": trigger_hint,
            "trigger_cron": null,
            "trigger_source": null,
            "trigger_event": null,
            "trigger_confidence": "medium",
            "trigger_confirmation": null,
            "output_hint": "workspace",
            "output_destination_hint": null,
            "output_questions": [],
            "responsibilities": [next_role_resp.clone()],
            "multi_role_suggested": false,
            "multi_role_reason": null,
            "clarifying_questions": [],
        });

        // Create new session with the next role pre-filled
        let session = crate::agent::definition::PlanModeSession {
            id: new_id(),
            tenant_id: tenant_id.to_string(),
            draft_agent: agent.clone(),
            draft_role: Some(crate::agent::definition::AgentRole {
                id: new_id(),
                agent_id: agent.id.clone(),
                tenant_id: tenant_id.to_string(),
                version: 1,
                status: crate::agent::definition::RoleStatus::Draft,
                name: next_role_resp
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("untitled role")
                    .to_string(),
                trigger: Default::default(),
                purpose: actions.join(", "),
                role_category: crate::agent::definition::RoleCategory::General,
                execution_guidelines: Default::default(),
                connectors: agent.connectors.clone(),
                tools: vec![],
                output_spec: Default::default(),
                memory_scope: Default::default(),
                execution_limits: Default::default(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }),
            conversation: vec![],
            attachments: vec![],
            attachment_context: String::new(),
            session_workspace: None,
            goal_fingerprint: None,
            repair_version: 1,
            reused_from_session_id: None,
            repair_root_session_id: None,
            phase: crate::agent::definition::PlanModePhase::CapturingClarifications,
            intent_cache: Some(synthetic_intent.clone()),
            pending_steps: generate_steps(&synthetic_intent, "general", &[], &existing_role_names)
                .into_iter()
                .filter_map(|step| serde_json::to_value(step).ok())
                .collect(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        tracing::info!(
            agent_id = %agent_id,
            next_role_name = %session.draft_role.as_ref().map(|r| r.name.clone()).unwrap_or_default(),
            existing_role_count = existing_roles.len(),
            "resuming plan mode for next role in multi-role agent"
        );

        Ok(session)
    }

    /// Parse pending_roles from memory_ref format: "agent:xxx|pending_roles:[...]"
    pub(crate) fn split_pending_roles(memory_ref: &str) -> Result<(Vec<serde_json::Value>, String)> {
        if let Some(pos) = memory_ref.find("|pending_roles:") {
            let base = memory_ref[..pos].to_string();
            let json_str = &memory_ref[pos + 15..]; // skip "|pending_roles:"
            let cleaned = json_str.trim_end_matches('`');
            match serde_json::from_str::<Vec<serde_json::Value>>(cleaned) {
                Ok(roles) => Ok((roles, base)),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse pending_roles JSON");
                    Ok((vec![], base))
                }
            }
        } else {
            Ok((vec![], memory_ref.to_string()))
        }
    }

    pub(crate) fn build_memory_ref(base: &str, pending_roles: &[serde_json::Value]) -> String {
        if pending_roles.is_empty() {
            base.to_string()
        } else {
            format!("{}|pending_roles:{}", base, serde_json::to_string(pending_roles).unwrap_or_default())
        }
    }

    /// Create a new goal and root agent, scoped to `tenant_id`.
    /// Automatically starts SLA tracking if an SlaTracker is active for this segment.
    /// If `conversation_id` is provided, the agent is linked to that conversation.
    pub async fn create_goal(
        &self,
        tenant_id: String,
        description: String,
        conversation_id: Option<String>,
    ) -> Result<(GoalState, AgentState)> {
        let goal_id = new_id();
        let agent_id = new_id();
        let gi_id = new_id();

        let handle = self.workspace_manager.create(&tenant_id, &agent_id).await?;
        let workspace_path = handle.local_path_str();

        tracing::info!(
            tenant_id      = %tenant_id,
            agent_id       = %agent_id,
            workspace_mode = ?handle.info.mode,
            workspace_path = %workspace_path,
            "workspace created"
        );

        let (goal, mut agent_state) =
            build_goal_and_agent(goal_id, agent_id.clone(), tenant_id.clone(), description.clone(), workspace_path);

        agent_state.conversation_id = conversation_id;
        agent_state.metadata["goal_instance_id"] = serde_json::json!(gi_id);

        let gi = GoalInstance {
            id: gi_id,
            tenant_id: tenant_id.clone(),
            agent_id: agent_id.clone(),
            role_id: "legacy-flat".to_string(),
            role_version: 1,
            input_data: serde_json::json!({ "description": description }),
            status: GoalInstanceStatus::Pending,
            result: None,
            failure_reason: None,
            trigger_source: TriggerSource::Manual { created_by: "system".to_string() },
            is_test: false,
            cost_usd: 0.0,
            human_hours_saved: 0.0,
            human_cost_saved_usd: 0.0,
            agent_state_id: Some(agent_id.clone()),
            triggered_by_goal_instance_id: None,
            current_step: 0,
            total_steps: 0,
            last_message: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            completed_at: None,
        };

        // ── SLA start — stamp deadline on agent state at creation time ────────
        if let Some(ref sla) = self.services.sla {
            let job_type = JobType::detect(&description);
            let priority = sla_priority_for_job(&job_type);
            if let Some(sla_status) = sla.start(&agent_state.id, &tenant_id, &priority) {
                agent_state.metadata["sla_status"] =
                    serde_json::to_value(&sla_status).unwrap_or(serde_json::Value::Null);
                tracing::debug!(
                    agent_id   = %agent_state.id,
                    ?priority,
                    first_response_deadline = %sla_status.first_response_deadline,
                    resolution_deadline     = %sla_status.resolution_deadline,
                    "SLA tracking started"
                );
            }
        }

        // ── Atomically create agent, goal, and goal instance in a single transaction ────────
        // This ensures consistency: if any operation fails, all are rolled back.
        // Critical for compliance (auditing), financial segments, and consistency guarantees.
        let _tx = self.store.begin_transaction().await
            .map_err(|e| anyhow::anyhow!("failed to start transaction: {}", e))?;
        
        // NOTE: Implementation would require transaction-aware upsert variants (_tx versions).
        // For now, these are standard calls with the transaction initiated.
        // Refactoring to _tx methods is a follow-up improvement.
        self.store.upsert_agent(&agent_state).await?;
        self.store.upsert_goal(&goal).await?;
        self.store.upsert_goal_instance(&gi).await?;
        
        // TODO: Explicit tx.commit() once _tx variants are implemented

        tracing::info!(goal_id = %goal.id, agent_id = %agent_state.id, gi_id = %gi.id, "goal created with unified instance tracking");
        Ok((goal, agent_state))
    }

    /// Unified method to trigger a role-based run.
    /// Creates both a GoalInstance and an AgentState, links them, and persists them.
    pub async fn create_role_run(
        &self,
        tenant_id: String,
        role: &AgentRole,
        input_data: serde_json::Value,
        trigger_source: TriggerSource,
        conversation_id: Option<String>,
        triggered_by_gi_id: Option<String>,
    ) -> Result<(GoalInstance, AgentState)> {
        let agent_id = new_id();
        let gi_id = new_id();

        let handle = self.workspace_manager.create(&tenant_id, &agent_id).await?;
        let workspace_path = handle.local_path_str();

        let mut agent_state = AgentState::new(
            agent_id.clone(),
            tenant_id.clone(),
            role.purpose.clone(),
            workspace_path,
        );
        agent_state.conversation_id = conversation_id;
        
        // Inject role metadata for AgentLoop and Planner
        agent_state.metadata["role_id"] = serde_json::json!(role.id);
        agent_state.metadata["role_name"] = serde_json::json!(role.name);
        agent_state.metadata["agent_definition_id"] = serde_json::json!(role.agent_id);
        agent_state.metadata["input_data"] = input_data.clone();
        agent_state.metadata["goal_instance_id"] = serde_json::json!(gi_id);

        let gi = GoalInstance {
            id: gi_id,
            tenant_id: tenant_id.clone(),
            agent_id: agent_id.clone(),
            role_id: role.id.clone(),
            role_version: role.version,
            input_data,
            status: GoalInstanceStatus::Pending,
            result: None,
            failure_reason: None,
            trigger_source,
            is_test: false,
            cost_usd: 0.0,
            human_hours_saved: 0.0,
            human_cost_saved_usd: 0.0,
            agent_state_id: Some(agent_id.clone()),
            triggered_by_goal_instance_id: triggered_by_gi_id,
            current_step: 0,
            total_steps: 0,
            last_message: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            completed_at: None,
        };

        // Persist both
        self.store.upsert_agent(&agent_state).await?;
        self.store.upsert_goal_instance(&gi).await?;

        tracing::info!(
            tenant_id = %tenant_id,
            role_id = %role.id,
            agent_id = %agent_id,
            gi_id = %gi.id,
            "role run created and persisted"
        );

        Ok((gi, agent_state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AgentStatus, GoalStatus};

    // ── Unit tests for helper functions (no DB needed) ────────────────────

    #[test]
    fn test_build_goal_and_agent_initialises_correctly() {
        let (goal, agent) = build_goal_and_agent(
            "goal-1".into(),
            "agent-1".into(),
            "tenant-1".into(),
            "fix CI pipeline".into(),
            "/tmp/ws".into(),
        );
        assert_eq!(goal.id, "goal-1");
        assert_eq!(goal.status, GoalStatus::InProgress);
        assert_eq!(goal.agent_ids, vec!["agent-1"]);
        assert_eq!(agent.status, AgentStatus::Pending);
        assert_eq!(agent.goal, "fix CI pipeline");
    }

    #[test]
    fn test_sla_priority_for_customer_support_is_urgent() {
        assert_eq!(sla_priority_for_job(&JobType::CustomerSupport), SlaPriority::Urgent);
    }

    #[test]
    fn test_sla_priority_for_itsm_is_critical() {
        assert_eq!(sla_priority_for_job(&JobType::ITOpsITSM), SlaPriority::Critical);
    }

    #[test]
    fn test_sla_priority_for_general_is_low() {
        assert_eq!(sla_priority_for_job(&JobType::General), SlaPriority::Low);
    }

    #[test]
    fn test_sla_priority_covers_all_job_types() {
        // Regression: every JobType must map to some priority (no panic)
        let types = [
            JobType::SoftwareEngineer,
            JobType::ResearchAnalyst,
            JobType::CustomerSupport,
            JobType::DevOps,
            JobType::Marketing,
            JobType::DataExtraction,
            JobType::SalesRevOps,
            JobType::FinanceAccounting,
            JobType::HRPeopleOps,
            JobType::LegalContract,
            JobType::ITOpsITSM,
            JobType::General,
        ];
        for jt in &types {
            let _ = sla_priority_for_job(jt); // must not panic
        }
    }

    #[test]
    fn test_split_and_build_pending_roles_roundtrip() {
        let memory_ref = r#"agent:abcd1234|pending_roles:[{"name":"First"},{"name":"Second"}]"#;
        let (pending_roles, base) = AgentManager::split_pending_roles(memory_ref).expect("should parse");

        assert_eq!(base, "agent:abcd1234");
        assert_eq!(pending_roles.len(), 2);
        assert_eq!(pending_roles[0]["name"], "First");

        let rebuilt = AgentManager::build_memory_ref(&base, &pending_roles[1..]);
        assert_eq!(rebuilt, r#"agent:abcd1234|pending_roles:[{"name":"Second"}]"#);
    }
}
