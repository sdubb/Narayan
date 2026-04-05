//! Cron schedule ticker — bridges `TriggerType::Schedule` roles to agent creation.
//!
//! DB-backed, atomic, multi-node safe.  No in-memory scheduling state.
//!
//! Every 30 seconds the ticker:
//! 1. Claims roles whose `next_run_at` is NULL or in the past (`FOR UPDATE SKIP LOCKED`).
//! 2. Parses each role's cron expression and computes the *next* fire time.
//! 3. Creates a `GoalInstance` + `AgentState` via `AgentManager::create_goal`.
//! 4. Updates `next_run_at` so the role won't fire again until due.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use croner::Cron;

use crate::{
    agent::{definition::ExecutionStrategy, AgentManager, AgentRole},
    state::TriggerSource,
    storage::PostgresStore,
};

/// Batch size per tick — how many due roles to claim at once.
const DEFAULT_BATCH_SIZE: i64 = 50;

/// Interval between ticks.
const TICK_INTERVAL_SECS: u64 = 30;

pub struct ScheduleTicker {
    store: Arc<PostgresStore>,
    manager: Arc<AgentManager>,
}

impl ScheduleTicker {
    pub fn new(store: Arc<PostgresStore>, manager: Arc<AgentManager>) -> Self {
        Self { store, manager }
    }

    /// Run forever — tick every 30 seconds, claim and fire due scheduled roles.
    pub async fn run(&self) {
        tracing::info!("schedule ticker starting");
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(TICK_INTERVAL_SECS));
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                tracing::error!(error = %e, "schedule ticker cycle failed");
            }
        }
    }

    async fn tick(&self) -> Result<()> {
        let roles = self.store.claim_due_scheduled_roles(DEFAULT_BATCH_SIZE).await?;
        if roles.is_empty() {
            return Ok(());
        }
        tracing::info!(count = roles.len(), "schedule ticker claimed due roles");

        for role in &roles {
            if let Err(e) = self.process_role(role).await {
                tracing::error!(role_id = %role.id, error = %e, "schedule ticker: failed to process role");
                // Set next_run_at to now + 60s so we retry on the next cycle
                let retry_at = Utc::now() + chrono::Duration::seconds(60);
                if let Err(e2) = self.store.update_role_next_run_at(&role.id, retry_at).await {
                    tracing::error!(role_id = %role.id, error = %e2, "failed to set retry next_run_at");
                }
            }
        }
        Ok(())
    }

    async fn process_role(&self, role: &AgentRole) -> Result<()> {
        let role = self.store.get_agent_role(&role.tenant_id, &role.id).await?.unwrap_or_else(|| role.clone());

        if !role_ready_for_schedule(&role) {
            let retry_at = Utc::now() + chrono::Duration::seconds(60);
            self.store.update_role_next_run_at(&role.id, retry_at).await?;
            tracing::info!(
                role_id = %role.id,
                retry_at = %retry_at,
                "scheduled role is not ready yet; delaying until its compiled workflow is saved"
            );
            return Ok(());
        }

        let cron_str = role.trigger.cron.as_deref().unwrap_or("0 9 * * *");
        let now = Utc::now();

        // Compute next fire time
        let next_run_at = compute_next_run(cron_str, now, role.trigger.timezone.as_deref())?;

        // Create the run via AgentManager
        let (_gi, agent_state) = self
            .manager
            .create_role_run(
                role.tenant_id.clone(),
                &role,
                serde_json::json!({ "scheduled_at": now.to_rfc3339() }),
                TriggerSource::Schedule { cron: cron_str.to_string(), scheduled_at: now },
                None, // conversation_id
                None, // triggered_by_gi_id
            )
            .await?;

        // Set the real next_run_at
        self.store.update_role_next_run_at(&role.id, next_run_at).await?;

        tracing::info!(
            role_id      = %role.id,
            agent_id     = %agent_state.id,
            next_run_at  = %next_run_at,
            "scheduled role triggered"
        );
        Ok(())
    }
}

fn role_ready_for_schedule(role: &AgentRole) -> bool {
    matches!(role.execution_guidelines.execution_strategy, ExecutionStrategy::CoordinatorShell)
        || role.execution_guidelines.compiled_workflow.is_some()
}

/// Compute the next occurrence of a cron expression after `after`.
/// If `timezone` is provided, the cron is evaluated in that timezone.
fn compute_next_run(cron_str: &str, after: DateTime<Utc>, timezone: Option<&str>) -> Result<DateTime<Utc>> {
    let cron = Cron::new(cron_str).parse().map_err(|e| anyhow::anyhow!("invalid cron '{}': {}", cron_str, e))?;

    if let Some(tz_name) = timezone {
        // Parse timezone and compute in local time, then convert back to UTC
        if let Ok(tz) = tz_name.parse::<chrono_tz::Tz>() {
            let local_now = after.with_timezone(&tz);
            let next_local = cron
                .find_next_occurrence(&local_now, false)
                .map_err(|e| anyhow::anyhow!("no next occurrence for cron '{}': {}", cron_str, e))?;
            return Ok(next_local.with_timezone(&Utc));
        }
        tracing::warn!(tz = tz_name, "unknown timezone, falling back to UTC");
    }

    let next = cron
        .find_next_occurrence(&after, false)
        .map_err(|e| anyhow::anyhow!("no next occurrence for cron '{}': {}", cron_str, e))?;
    Ok(next.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::{AgentRole, ExecutionStrategy};
    use chrono::TimeZone;

    #[test]
    fn test_next_run_computed_correctly() {
        // Cron "0 9 * * *" (daily at 9am UTC), current time is 8am UTC
        let now = Utc.with_ymd_and_hms(2026, 3, 24, 8, 0, 0).unwrap();
        let next = compute_next_run("0 9 * * *", now, None).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 3, 24, 9, 0, 0).unwrap());
    }

    #[test]
    fn test_missed_cron_fires_next() {
        // Cron "0 9 * * *" (daily at 9am), current time is 10am (past today's fire)
        // Next occurrence should be tomorrow at 9am
        let now = Utc.with_ymd_and_hms(2026, 3, 24, 10, 0, 0).unwrap();
        let next = compute_next_run("0 9 * * *", now, None).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 3, 25, 9, 0, 0).unwrap());
    }

    #[test]
    fn test_every_minute_cron_next() {
        let now = Utc.with_ymd_and_hms(2026, 3, 24, 12, 30, 0).unwrap();
        let next = compute_next_run("* * * * *", now, None).unwrap();
        // Next minute
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 3, 24, 12, 31, 0).unwrap());
    }

    #[test]
    fn test_invalid_cron_returns_error() {
        let now = Utc::now();
        assert!(compute_next_run("not a cron", now, None).is_err());
    }

    #[test]
    fn test_role_ready_for_schedule_requires_compiled_workflow() {
        let mut role = AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary".into());
        role.execution_guidelines.execution_strategy = ExecutionStrategy::DeterministicWorkflow;

        assert!(!role_ready_for_schedule(&role));
    }

    #[test]
    fn test_role_ready_for_schedule_accepts_compiled_workflow() {
        let mut role = AgentRole::new("role-1".into(), "agent-1".into(), "tenant-1".into(), "Primary".into());
        role.execution_guidelines.execution_strategy = ExecutionStrategy::DeterministicWorkflow;
        role.execution_guidelines.compiled_workflow = Some(crate::agent::workflow_compiler::CompiledWorkflow {
            workflow_id: "wf_test".into(),
            version: "v2".into(),
            workflow_version: "v2".into(),
            parent_workflow_version: None,
            recompile_reason: None,
            dsl_version: "v1".into(),
            binding_version: "v1".into(),
            runtime_version: "v1".into(),
            tool_registry_version: "registry_test".into(),
            entry_step: "step_1".into(),
            execution: crate::agent::workflow_compiler::ExecutionMode::Sequential,
            steps: vec![],
            state_schema: serde_json::json!({}),
            resources: std::collections::BTreeMap::new(),
            metadata: serde_json::json!({}),
            tool_capabilities: std::collections::BTreeMap::new(),
            binding_rules: vec![],
            variant_policy: None,
            execution_constraints: crate::agent::workflow_compiler::ExecutionConstraints::default(),
            data_strategy: crate::agent::workflow_compiler::DataStrategy::default(),
            determinism: crate::agent::workflow_compiler::DeterminismConfig::default(),
            scheduler: crate::agent::workflow_compiler::SchedulerConfig::default(),
            recompile_policy: crate::agent::workflow_compiler::RecompilePolicy::default(),
            expression_functions: std::collections::BTreeMap::new(),
            permissions: std::collections::BTreeMap::new(),
            execution_snapshot: None,
        });

        assert!(role_ready_for_schedule(&role));
    }
}
