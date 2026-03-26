use std::sync::Arc;

use anyhow::Result;
use std::path::PathBuf;

use crate::{
    agent::prompts::JobType,
    compliance::sla::SlaPriority,
    gateway::LlmGateway,
    segments::AgentServices,
    state::{AgentState, GoalState},
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
        let agent = self
            .store
            .get_agent_definition(tenant_id, agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent definition not found: {}", agent_id))?;

        // Parse pending_roles from memory_ref
        let pending_roles = Self::extract_pending_roles(&agent.memory_ref)?;
        if pending_roles.is_empty() {
            anyhow::bail!("No pending roles found in agent: {}", agent_id);
        }

        // Extract the first pending role
        let next_role_resp = pending_roles.first().cloned().ok_or_else(|| {
            anyhow::anyhow!("Pending roles array is empty")
        })?;

        // Create new session with the next role pre-filled
        let mut session = crate::agent::definition::PlanModeSession {
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
                purpose: String::new(),
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
            intent_cache: None,
            pending_steps: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        // Load existing roles for the agent to populate cross-role context
        let existing_roles = self.store.list_roles_for_agent(tenant_id, agent_id).await.unwrap_or_default();
        tracing::info!(
            agent_id = %agent_id,
            next_role_name = %session.draft_role.as_ref().map(|r| r.name.clone()).unwrap_or_default(),
            existing_role_count = existing_roles.len(),
            "resuming plan mode for next role in multi-role agent"
        );

        Ok(session)
    }

    /// Parse pending_roles from memory_ref format: "agent:xxx|pending_roles:[...]"
    fn extract_pending_roles(memory_ref: &str) -> Result<Vec<serde_json::Value>> {
        if let Some(pos) = memory_ref.find("|pending_roles:") {
            let json_str = &memory_ref[pos + 15..]; // skip "|pending_roles:"
            let cleaned = json_str.trim_end_matches('`');
            match serde_json::from_str::<Vec<serde_json::Value>>(cleaned) {
                Ok(roles) => Ok(roles),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse pending_roles JSON");
                    Ok(vec![])
                }
            }
        } else {
            Ok(vec![])
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
            build_goal_and_agent(goal_id, agent_id, tenant_id.clone(), description.clone(), workspace_path);

        agent_state.conversation_id = conversation_id;

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

        self.store.upsert_agent(&agent_state).await?;
        self.store.upsert_goal(&goal).await?;

        tracing::info!(goal_id = %goal.id, agent_id = %agent_state.id, "goal created");
        Ok((goal, agent_state))
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
}
