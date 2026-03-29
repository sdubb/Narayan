//! Webhook delivery engine with HMAC-SHA256 signing and retries.

use std::sync::Arc;

use anyhow::Result;
use ring::hmac;

use crate::{
    events::{AgentEvent, EventBus},
    webhooks::config::WebhookStore,
};

const MAX_RETRIES: i32 = 3;
const RETRY_DELAYS_SECS: [u64; 3] = [1, 5, 30];

pub struct WebhookDispatcher {
    store: Arc<WebhookStore>,
    http: reqwest::Client,
}

impl WebhookDispatcher {
    pub fn new(store: Arc<WebhookStore>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("HTTP client for webhooks");
        Self { store, http }
    }

    /// Deliver an event to all matching webhooks for a tenant.
    pub async fn deliver(&self, tenant_id: &str, event: &AgentEvent) -> Result<()> {
        let event_type = event_type_name(event);
        let payload = serde_json::to_value(event)?;

        let hooks = self.store.get_matching(tenant_id, &event_type).await?;
        for hook in hooks {
            let payload = payload.clone();
            let store = self.store.clone();
            let http = self.http.clone();
            let event_type = event_type.clone();

            tokio::spawn(async move {
                if let Err(e) =
                    deliver_with_retries(&http, &store, &hook.id, &hook.url, &hook.secret, &event_type, &payload).await
                {
                    tracing::error!(
                        webhook_id = %hook.id,
                        url = %hook.url,
                        error = %e,
                        "webhook delivery failed after all retries"
                    );
                }
            });
        }
        Ok(())
    }

    /// Spawn a background task that forwards all events for an agent to webhooks.
    pub fn bridge_agent(self: &Arc<Self>, event_bus: Arc<EventBus>, agent_id: String, tenant_id: String) {
        let dispatcher = self.clone();
        tokio::spawn(async move {
            let mut rx = event_bus.subscribe(&agent_id);
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Err(e) = dispatcher.deliver(&tenant_id, &event).await {
                            tracing::error!(error = %e, "webhook dispatch error");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(agent_id = %agent_id, skipped = n, "webhook bridge lagged");
                    }
                }
            }
        });
    }
}

async fn deliver_with_retries(
    http: &reqwest::Client,
    store: &WebhookStore,
    webhook_id: &str,
    url: &str,
    secret: &str,
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let body = serde_json::to_string(payload)?;
    let signature = sign_payload(secret, &body);

    for attempt in 1..=MAX_RETRIES {
        let result = http
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-Narayan-Signature", &signature)
            .header("X-Narayan-Event", event_type)
            .body(body.clone())
            .send()
            .await;

        match result {
            Ok(resp) => {
                let status = resp.status().as_u16() as i32;
                let resp_body = resp.text().await.unwrap_or_default();
                let success = (200..300).contains(&(status as u16 as usize));

                store
                    .record_delivery(webhook_id, event_type, payload, Some(status), Some(&resp_body), attempt, success)
                    .await?;

                if success {
                    store.reset_failure(webhook_id).await?;
                    return Ok(());
                }

                tracing::warn!(webhook_id, url, status, attempt, "webhook delivery got non-2xx response");
            }
            Err(e) => {
                store
                    .record_delivery(webhook_id, event_type, payload, None, Some(&e.to_string()), attempt, false)
                    .await?;
                tracing::warn!(webhook_id, url, attempt, error = %e, "webhook delivery failed");
            }
        }

        // Wait before retrying
        if attempt < MAX_RETRIES {
            let delay = RETRY_DELAYS_SECS[(attempt - 1) as usize];
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
    }

    // All retries exhausted — increment failure counter
    store.increment_failure(webhook_id).await?;
    anyhow::bail!("webhook delivery failed after {} attempts to {}", MAX_RETRIES, url)
}

/// HMAC-SHA256 signature of the payload body.
fn sign_payload(secret: &str, body: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, body.as_bytes());
    hex::encode(tag.as_ref())
}

/// Extract a machine-readable event type name from an AgentEvent.
fn event_type_name(event: &AgentEvent) -> String {
    match event {
        AgentEvent::PreflightStarted { .. } => "preflight_started",
        AgentEvent::PreflightPassed { .. } => "preflight_passed",
        AgentEvent::PreflightFailed { .. } => "preflight_failed",
        AgentEvent::ClarificationNeeded { .. } => "clarification_needed",
        AgentEvent::ClarificationReceived { .. } => "clarification_received",
        AgentEvent::PlanningStarted { .. } => "planning_started",
        AgentEvent::PlanCreated { .. } => "plan_created",
        AgentEvent::StepStarted { .. } => "step_started",
        AgentEvent::ToolCalled { .. } => "tool_called",
        AgentEvent::ToolResult { .. } => "tool_result",
        AgentEvent::StepCompleted { .. } => "step_completed",
        AgentEvent::StepRetrying { .. } => "step_retrying",
        AgentEvent::PolicyDecision { .. } => "policy_decision",
        AgentEvent::PiiRedacted { .. } => "pii_redacted",
        AgentEvent::SlaCheck { .. } => "sla_check",
        AgentEvent::CitationRecorded { .. } => "citation_recorded",
        AgentEvent::EvidencePackaged { .. } => "evidence_packaged",
        AgentEvent::ReviewRequired { .. } => "review_required",
        AgentEvent::ConnectorTrigger { .. } => "connector_trigger",
        AgentEvent::ChildSpawned { .. } => "child_spawned",
        AgentEvent::ChildrenComplete { .. } => "children_complete",
        AgentEvent::GoalComplete { .. } => "goal_complete",
        AgentEvent::GoalFailed { .. } => "goal_failed",
        AgentEvent::LlmCostUpdate { .. } => "llm_cost_update",
        AgentEvent::ExecutionLimitWarning { .. } => "execution_limit_warning",
        AgentEvent::PlanApproved { .. } => "plan_approved",
        AgentEvent::PlanRejected { .. } => "plan_rejected",
        AgentEvent::PlanEdited { .. } => "plan_edited",
        AgentEvent::PlanApprovalNeeded { .. } => "plan_approval_needed",
        AgentEvent::JudgementSignal { .. } => "judgement_signal",
        AgentEvent::RoleCompleted { .. } => "role_completed",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_payload_is_deterministic() {
        let sig1 = sign_payload("secret", "hello");
        let sig2 = sign_payload("secret", "hello");
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_sign_payload_differs_with_different_secrets() {
        let sig1 = sign_payload("secret-a", "hello");
        let sig2 = sign_payload("secret-b", "hello");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_event_type_name_coverage() {
        let event = AgentEvent::GoalComplete { agent_id: "a".into(), summary: "done".into() };
        assert_eq!(event_type_name(&event), "goal_complete");
    }
}
