//! Bridges EventBus → AuditLog.
//!
//! Subscribes to agent events and writes them as immutable audit entries.
//! Runs as a background task — one per tracked agent.

use std::sync::Arc;

use crate::{
    audit::log::{AuditAction, AuditLog},
    events::{AgentEvent, EventBus},
    metrics::Metrics,
};

/// Spawn a background task that forwards agent events to the audit log.
/// Call this when a new agent starts executing.
/// Metrics are optional — if provided, lag events are tracked for observability.
pub fn bridge_agent_events(
    event_bus: Arc<EventBus>,
    audit_log: Arc<AuditLog>,
    agent_id: String,
    tenant_id: String,
    metrics: Option<Arc<Metrics>>,
) {
    tokio::spawn(async move {
        let mut rx = event_bus.subscribe(&agent_id);
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let (action, detail) = event_to_audit(&event);
                    let aid = Some(event.agent_id());
                    if let Err(e) = audit_log.append(&tenant_id, aid, action, detail, None).await {
                        tracing::error!(error = %e, agent_id = %agent_id, "audit bridge write failed");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(agent_id = %agent_id, skipped = n, "audit bridge lagged");
                    // Track the lag event in metrics for alerting
                    if let Some(ref metrics) = metrics {
                        metrics.audit_bridge_lag(n as u64);
                    }
                }
            }
        }
    });
}

fn event_to_audit(event: &AgentEvent) -> (AuditAction, serde_json::Value) {
    match event {
        AgentEvent::StepStarted { step_index, description, .. } => {
            (AuditAction::StepStarted, serde_json::json!({ "step_index": step_index, "description": description }))
        }
        AgentEvent::StepCompleted { step_index, success, summary, .. } => (
            AuditAction::StepCompleted,
            serde_json::json!({ "step_index": step_index, "success": success, "summary": summary }),
        ),
        AgentEvent::ToolCalled { step_index, tool_name, args_preview, .. } => (
            AuditAction::ToolExecuted,
            serde_json::json!({ "step_index": step_index, "tool": tool_name, "args_preview": args_preview }),
        ),
        AgentEvent::ToolResult { step_index, tool_name, success, output_preview, .. } => (
            AuditAction::ToolExecuted,
            serde_json::json!({
                "step_index": step_index, "tool": tool_name,
                "success": success, "output_preview": output_preview
            }),
        ),
        AgentEvent::GoalComplete { summary, .. } => {
            (AuditAction::GoalCreated, serde_json::json!({ "outcome": "complete", "summary": summary }))
        }
        AgentEvent::GoalFailed { reason, .. } => {
            (AuditAction::GoalCreated, serde_json::json!({ "outcome": "failed", "reason": reason }))
        }
        // All other events logged as custom
        other => (AuditAction::Custom, serde_json::to_value(other).unwrap_or_default()),
    }
}
