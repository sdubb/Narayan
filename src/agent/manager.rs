use std::sync::Arc;

use anyhow::Result;

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
