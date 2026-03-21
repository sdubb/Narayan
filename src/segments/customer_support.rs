//! Customer Support segment plugin.
//! Covers: ticket handling, escalation, SLA enforcement, response drafting.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::zendesk::ZendeskConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Refunds over $100 require human approval
    rules.rules.push(PolicyRule {
        id: "support-refund-limit".into(),
        name: "Large refunds require approval".into(),
        tools: vec!["api_call".into()],
        condition: PolicyCondition::ArgThreshold { field: "amount".into(), max: 100.0 },
        action: PolicyAction::RequireApproval { message: "Refund over $100 requires human approval".into() },
        enabled: true,
    });

    // Redact PII before any external API call
    rules.rules.push(PolicyRule {
        id: "support-pii-redact".into(),
        name: "Redact PII in external calls".into(),
        tools: vec!["api_call".into(), "http_request".into(), "email".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact { fields: vec!["ssn".into(), "credit_card".into(), "password".into()] },
        enabled: true,
    });

    SegmentPlugin {
        id: "customer_support",
        name: "Customer Support",
        connectors: vec![Arc::new(ZendeskConnector::new())],
        services: SegmentServices {
            policy: Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews: Some(deps.review_queue.clone()),
            pii: Some(deps.pii_redactor.clone()),
            sla: None, // built from sla_policies below
            evidence: None,
        },
        policy_rules: rules,
        sla_policies: vec![
            SlaPolicy {
                id: "support-urgent".into(),
                tenant_id: tenant_id.into(),
                name: "Urgent ticket SLA".into(),
                first_response_mins: 15,
                resolution_mins: 120,
                priority: SlaPriority::Urgent,
                escalation_rules: vec![
                    EscalationRule {
                        trigger_pct: 80.0,
                        action: EscalationAction::Notify { message: "Urgent SLA at 80%".into() },
                    },
                    EscalationRule {
                        trigger_pct: 100.0,
                        action: EscalationAction::EscalateToHuman { reason: "Urgent SLA breached".into() },
                    },
                ],
            },
            SlaPolicy {
                id: "support-normal".into(),
                tenant_id: tenant_id.into(),
                name: "Normal ticket SLA".into(),
                first_response_mins: 60,
                resolution_mins: 480,
                priority: SlaPriority::Normal,
                escalation_rules: vec![EscalationRule {
                    trigger_pct: 80.0,
                    action: EscalationAction::Notify { message: "Normal SLA at 80%".into() },
                }],
            },
        ],
    }
}
