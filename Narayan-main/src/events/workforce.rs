//! Workforce event dispatcher.
//!
//! When a GoalInstance completes or fails, it emits a WorkforceEventPayload.
//! The dispatcher matches that payload against all active WorkforceEventSubscriptions
//! for the tenant and creates new GoalInstances for matching subscriber roles.
//!
//! ## Flow
//!
//!   GoalInstance completes
//!       ↓
//!   loop.rs calls workforce::dispatch(event, store)
//!       ↓
//!   dispatcher loads all active subscriptions for tenant
//!       ↓
//!   for each subscription: evaluate event_filter
//!       ↓
//!   on match: apply input_mapping → create GoalInstance → upsert to DB
//!       ↓
//!   new GoalInstance picked up by scheduler on next poll
//!
//! The dispatcher is fire-and-forget — failures are logged but never
//! propagate back to the originating GoalInstance.  The upstream agent
//! is already complete by this point.
//!
//! ## Subscription sync
//!
//! WorkforceEventSubscriptions are created/updated/deleted automatically
//! whenever an AgentRole with TriggerType::WorkforceEvent is saved.
//! `sync_subscriptions_for_role` handles this.

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::{
    agent::definition::{WorkforceEventPayload, WorkforceEventSubscription},
    state::{GoalInstance, GoalInstanceStatus, TriggerSource},
    storage::PostgresStore,
};

/// Dispatch a workforce event — called after every GoalInstance completion or failure.
///
/// Loads all active subscriptions for the tenant, evaluates filters,
/// and creates new GoalInstances for matching subscribers.
/// Returns the number of new GoalInstances created.
pub async fn dispatch(
    event: &WorkforceEventPayload,
    store: &Arc<PostgresStore>,
) -> Result<usize> {
    let subscriptions = store
        .list_active_workforce_subscriptions(&event.tenant_id)
        .await
        .unwrap_or_default();

    if subscriptions.is_empty() {
        return Ok(0);
    }

    let mut spawned = 0;

    for sub in &subscriptions {
        // Skip subscriptions for the same role that just completed
        // (prevents trivial infinite loops).
        if sub.subscriber_role_id == event.role_id {
            continue;
        }

        if !event.matches_filter(&sub.event_filter) {
            continue;
        }

        // Load the subscriber role to get its current version and check it's active.
        let role = match store.get_agent_role(&event.tenant_id, &sub.subscriber_role_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                tracing::warn!(
                    subscription_id = %sub.id,
                    role_id         = %sub.subscriber_role_id,
                    "workforce subscription references non-existent role — skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to load subscriber role");
                continue;
            }
        };

        if !role.is_live() {
            tracing::debug!(
                role_id = %role.id,
                status  = ?role.status,
                "subscriber role not active — skipping"
            );
            continue;
        }

        // Map event payload fields to the new GoalInstance's input_data.
        let input_data = event.apply_mapping(&sub.input_mapping);

        let gi = GoalInstance::new(
            Uuid::new_v4().to_string(),
            event.tenant_id.clone(),
            sub.subscriber_agent_id.clone(),
            sub.subscriber_role_id.clone(),
            role.version,
            input_data,
            TriggerSource::WorkforceEvent {
                source_goal_instance_id: event.goal_instance_id.clone(),
                source_role_name:        event.role_name.clone(),
            },
            false, // is_test — inherit from role status in future
        );

        match store.upsert_goal_instance(&gi).await {
            Ok(_) => {
                tracing::info!(
                    tenant_id            = %event.tenant_id,
                    source_role          = %event.role_name,
                    source_goal_instance = %event.goal_instance_id,
                    subscriber_role      = %role.name,
                    new_goal_instance    = %gi.id,
                    "workforce event spawned new goal instance"
                );
                spawned += 1;
            }
            Err(e) => {
                tracing::error!(
                    error       = %e,
                    role_id     = %sub.subscriber_role_id,
                    "failed to create goal instance from workforce event"
                );
            }
        }
    }

    Ok(spawned)
}

/// Synchronise WorkforceEventSubscriptions for a role.
///
/// Called whenever an AgentRole is saved.
/// - If the role has TriggerType::WorkforceEvent and is active/testing:
///     upsert a subscription
/// - Otherwise: deactivate any existing subscription for this role
pub async fn sync_subscriptions_for_role(
    role: &crate::agent::definition::AgentRole,
    store: &Arc<PostgresStore>,
) -> Result<()> {
    use crate::agent::definition::TriggerType;

    let is_workforce_trigger = role.trigger.trigger_type == TriggerType::WorkforceEvent;
    let is_deployable = matches!(
        role.status,
        crate::agent::definition::RoleStatus::Active | crate::agent::definition::RoleStatus::Testing
    );

    if is_workforce_trigger && is_deployable {
        let filter = role
            .trigger
            .workforce_event_filter
            .clone()
            .unwrap_or_default();

        if filter.is_empty() {
            tracing::warn!(
                role_id = %role.id,
                "workforce_event role has no event_filter — subscription not created"
            );
            return Ok(());
        }

        let mapping = role
            .trigger
            .input_mapping
            .clone()
            .unwrap_or(serde_json::Value::Object(Default::default()));

        // Stable subscription ID derived from role ID so upsert is idempotent.
        let sub_id = format!("wfsub-{}", &role.id);

        let sub = WorkforceEventSubscription {
            id:                  sub_id,
            tenant_id:           role.tenant_id.clone(),
            subscriber_role_id:  role.id.clone(),
            subscriber_agent_id: role.agent_id.clone(),
            event_filter:        filter,
            input_mapping:       mapping,
            active:              true,
            created_at:          Utc::now(),
        };

        store.upsert_workforce_subscription(&sub).await?;

        tracing::info!(
            role_id = %role.id,
            filter  = %sub.event_filter,
            "workforce subscription synced"
        );
    } else {
        // Deactivate any existing subscription.
        let sub_id = format!("wfsub-{}", &role.id);
        let _ = store.deactivate_workforce_subscription(&role.tenant_id, &sub_id).await;
    }

    Ok(())
}

/// Build the workforce event payload from a completed/failed GoalInstance.
/// Requires the agent name and role name which are not stored on the instance.
pub async fn build_event(
    gi: &GoalInstance,
    store: &Arc<PostgresStore>,
) -> Option<WorkforceEventPayload> {
    // Load agent definition for agent_name
    let agent_def = store
        .get_agent_definition(&gi.tenant_id, &gi.agent_id)
        .await
        .ok()??;

    // Load role for role_name
    let role = store
        .get_agent_role(&gi.tenant_id, &gi.role_id)
        .await
        .ok()??;

    Some(gi.to_workforce_event(&agent_def.name, &role.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::WorkforceEventPayload;

    fn make_event(role_name: &str, status: &str) -> WorkforceEventPayload {
        WorkforceEventPayload {
            tenant_id:          "t-1".into(),
            agent_id:           "ag-1".into(),
            agent_name:         "Sales Agent".into(),
            role_id:            "role-src".into(),
            role_name:          role_name.into(),
            goal_instance_id:   "gi-src".into(),
            status:             status.into(),
            output_data:        serde_json::json!({ "lead_id": "L-999" }),
            failure_reason:     None,
            emitted_at:         Utc::now(),
        }
    }

    fn make_subscription(filter: &str) -> WorkforceEventSubscription {
        WorkforceEventSubscription {
            id:                  "sub-1".into(),
            tenant_id:           "t-1".into(),
            subscriber_role_id:  "role-sub".into(),
            subscriber_agent_id: "ag-2".into(),
            event_filter:        filter.into(),
            input_mapping:       serde_json::json!({ "lead_id": "$.output_data.lead_id" }),
            active:              true,
            created_at:          Utc::now(),
        }
    }

    #[test]
    fn test_filter_single_match() {
        let ev  = make_event("Lead Enrichment", "completed");
        let sub = make_subscription("role_name == 'Lead Enrichment'");
        assert!(ev.matches_filter(&sub.event_filter));
    }

    #[test]
    fn test_filter_no_match() {
        let ev  = make_event("Lead Enrichment", "completed");
        let sub = make_subscription("role_name == 'Weekly Report'");
        assert!(!ev.matches_filter(&sub.event_filter));
    }

    #[test]
    fn test_filter_and_both_match() {
        let ev  = make_event("Lead Enrichment", "completed");
        let sub = make_subscription("role_name == 'Lead Enrichment' AND status == 'completed'");
        assert!(ev.matches_filter(&sub.event_filter));
    }

    #[test]
    fn test_filter_and_partial_match() {
        let ev  = make_event("Lead Enrichment", "failed");
        let sub = make_subscription("role_name == 'Lead Enrichment' AND status == 'completed'");
        assert!(!ev.matches_filter(&sub.event_filter));
    }

    #[test]
    fn test_input_mapping_applied() {
        let ev  = make_event("Lead Enrichment", "completed");
        let sub = make_subscription("role_name == 'Lead Enrichment'");
        let mapped = ev.apply_mapping(&sub.input_mapping);
        assert_eq!(mapped["lead_id"], "L-999");
    }

    #[test]
    fn test_same_role_would_be_skipped() {
        // Subscription whose subscriber_role_id == source role_id should be skipped
        // to prevent trivial loops. This logic lives in dispatch(); here we just
        // verify the filter still matches so the guard is the only protection.
        let ev = make_event("Lead Enrichment", "completed");
        let mut sub = make_subscription("role_name == 'Lead Enrichment'");
        sub.subscriber_role_id = "role-src".into(); // same as event.role_id
        // Filter matches — the loop guard in dispatch() is what prevents creation
        assert!(ev.matches_filter(&sub.event_filter));
    }
}
