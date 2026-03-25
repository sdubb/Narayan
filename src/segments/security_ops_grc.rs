//! Security Ops & GRC segment plugin.
//! Covers: access reviews, security evidence, incident response, risk tracking, and audit readiness.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::{
        github::GitHubConnector, notion::NotionConnector, pagerduty::PagerDutyConnector,
        servicenow::ServiceNowConnector,
    },
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::{
        registry::{SegmentPlugin, SegmentServices, SharedDeps},
        DomainProfile,
    },
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Security changes and privileged access always need review.
    rules.rules.push(PolicyRule {
        id: "security-privileged-change-review".into(),
        name: "Privileged security changes require approval".into(),
        tools: vec!["api_call".into(), "http_request".into(), "shell".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(access|permission|role|admin|privilege|secret|token|credential|key|exception|waiver)"#.into(),
        },
        action: PolicyAction::RequireApproval {
            message: "Security, identity, and privilege changes require human review".into(),
        },
        enabled: true,
    });

    // Keep secrets and identifiers out of visible outputs.
    rules.rules.push(PolicyRule {
        id: "security-sensitive-redact".into(),
        name: "Redact secrets and sensitive identifiers".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec!["api_key".into(), "secret".into(), "token".into(), "password".into(), "ssn".into()],
        },
        enabled: true,
    });

    // Security and GRC evidence should be captured for audits.
    rules.rules.push(PolicyRule {
        id: "security-audit-evidence".into(),
        name: "Security work should leave evidence".into(),
        tools: vec!["api_call".into(), "http_request".into(), "file_write".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::RequireApproval {
            message: "Security and GRC actions should be reviewed with evidence attached".into(),
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "security_ops_grc",
        name: "Security Ops & GRC",
        domain: DomainProfile::security_ops_grc(),
        connectors: vec![
            Arc::new(ServiceNowConnector::new()),
            Arc::new(PagerDutyConnector::new()),
            Arc::new(GitHubConnector::new()),
            Arc::new(NotionConnector::new()),
        ],
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
            id: "security-incidents-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Security incident triage".into(),
            first_response_mins: 15,
            resolution_mins: 240,
            priority: SlaPriority::Critical,
            escalation_rules: vec![
                EscalationRule {
                    trigger_pct: 50.0,
                    action: EscalationAction::Notify { message: "Security incident at 50% of SLA".into() },
                },
                EscalationRule {
                    trigger_pct: 90.0,
                    action: EscalationAction::EscalateToHuman {
                        reason: "Security incident approaching SLA breach".into(),
                    },
                },
            ],
        }],
    }
}
