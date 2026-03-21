//! HR & People Ops segment plugin.
//! Covers: candidate screening, onboarding, policy Q&A, performance data.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::greenhouse::GreenhouseConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Every hiring/screening decision requires human review — no exceptions
    rules.rules.push(PolicyRule {
        id: "hr-hiring-review".into(),
        name: "Hiring decisions require human review".into(),
        tools: vec!["email".into(), "api_call".into()],
        condition: PolicyCondition::ArgsMatch { pattern: r#"(hire|reject|offer|screen|shortlist)"#.into() },
        action: PolicyAction::RequireApproval {
            message: "Hiring actions must be reviewed by an HR manager before execution".into(),
        },
        enabled: true,
    });

    // Always redact sensitive HR PII
    rules.rules.push(PolicyRule {
        id: "hr-pii-redact".into(),
        name: "Redact HR PII in all outputs".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec!["ssn".into(), "dob".into(), "salary".into(), "medical".into(), "background_check".into()],
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "hr_people_ops",
        name: "HR & People Ops",
        connectors: vec![Arc::new(GreenhouseConnector::new())],
        services: SegmentServices {
            policy: Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews: Some(deps.review_queue.clone()),
            pii: Some(deps.pii_redactor.clone()),
            evidence: None,
            sla: None,
        },
        policy_rules: rules,
        sla_policies: vec![SlaPolicy {
            id: "hr-candidate-response-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Candidate response SLA".into(),
            first_response_mins: 1440, // 24h
            resolution_mins: 4320,     // 72h
            priority: SlaPriority::Normal,
            escalation_rules: vec![EscalationRule {
                trigger_pct: 80.0,
                action: EscalationAction::Notify { message: "Candidate response at 80% of SLA".into() },
            }],
        }],
    }
}
