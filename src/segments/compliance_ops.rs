//! Compliance Ops segment plugin.
//! Covers: document pipelines, citation-first workflows, evidence packaging, regulatory review.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::servicenow::ServiceNowConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Every external delivery must be reviewed
    rules.rules.push(PolicyRule {
        id: "compliance-review-all-outputs".into(),
        name: "All agent outputs require review".into(),
        tools: vec!["email".into(), "api_call".into(), "http_request".into(), "file_write".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::RequireApproval {
            message: "Compliance agent output requires reviewer sign-off before delivery".into(),
        },
        enabled: true,
    });

    rules.rules.push(PolicyRule {
        id: "compliance-pii-redact-always".into(),
        name: "Always redact PII in compliance outputs".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec!["ssn".into(), "credit_card".into(), "dob".into(), "passport".into()],
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "compliance_ops",
        name: "Compliance Ops",
        connectors: vec![Arc::new(ServiceNowConnector::new())],
        services: SegmentServices {
            policy: Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews: Some(deps.review_queue.clone()),
            evidence: Some(deps.evidence_packager.clone()),
            pii: Some(deps.pii_redactor.clone()),
            sla: None,
        },
        policy_rules: rules,
        sla_policies: vec![SlaPolicy {
            id: "compliance-review-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Compliance review turnaround".into(),
            first_response_mins: 240,
            resolution_mins: 1440,
            priority: SlaPriority::High,
            escalation_rules: vec![
                EscalationRule {
                    trigger_pct: 75.0,
                    action: EscalationAction::Notify { message: "Compliance review at 75% of SLA".into() },
                },
                EscalationRule {
                    trigger_pct: 100.0,
                    action: EscalationAction::EscalateToHuman { reason: "Compliance review SLA breached".into() },
                },
            ],
        }],
    }
}
