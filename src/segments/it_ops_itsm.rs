//! IT Ops & ITSM segment plugin.
//! Covers: incident runbooks, change advisory, health checks, postmortems.

use std::sync::Arc;
use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::{pagerduty::PagerDutyConnector, servicenow::ServiceNowConnector},
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Any infra destructive action requires human approval
    rules.rules.push(PolicyRule {
        id: "itsm-destructive-approval".into(),
        name: "Destructive infra ops require approval".into(),
        tools: vec!["docker".into(), "kubernetes".into(), "shell".into(), "ssh_exec".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(delete|destroy|terminate|stop|kill|drop|reset)"#.into(),
        },
        action: PolicyAction::RequireApproval {
            message: "Destructive infrastructure operation requires on-call engineer approval".into(),
        },
        enabled: true,
    });

    // Block agents from modifying prod without change record
    rules.rules.push(PolicyRule {
        id: "itsm-change-record-required".into(),
        name: "Production changes require change record".into(),
        tools: vec!["shell".into(), "ssh_exec".into(), "kubernetes".into()],
        condition: PolicyCondition::ArgsMatch { pattern: r"prod".into() },
        action: PolicyAction::RequireApproval {
            message: "Changes to production require an approved change record in ServiceNow".into(),
        },
        enabled: true,
    });

    SegmentPlugin {
        id:   "it_ops_itsm",
        name: "IT Ops & ITSM",
        connectors: vec![
            Arc::new(ServiceNowConnector::new()),
            Arc::new(PagerDutyConnector::new()),
        ],
        services: SegmentServices {
            policy:    Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews:   Some(deps.review_queue.clone()),
            evidence:  Some(deps.evidence_packager.clone()),
            pii:       None,
            sla:       None,
        },
        policy_rules: rules,
        sla_policies: vec![
            SlaPolicy {
                id: "itsm-p1-sla".into(),
                tenant_id: tenant_id.into(),
                name: "P1 incident SLA".into(),
                first_response_mins: 5,
                resolution_mins: 60,
                priority: SlaPriority::Critical,
                escalation_rules: vec![
                    EscalationRule { trigger_pct: 50.0, action: EscalationAction::Notify { message: "P1: 50% SLA elapsed".into() } },
                    EscalationRule { trigger_pct: 80.0, action: EscalationAction::EscalateToHuman { reason: "P1 at 80% SLA — escalating".into() } },
                ],
            },
            SlaPolicy {
                id: "itsm-change-sla".into(),
                tenant_id: tenant_id.into(),
                name: "Change advisory SLA".into(),
                first_response_mins: 60,
                resolution_mins: 480,
                priority: SlaPriority::Normal,
                escalation_rules: vec![
                    EscalationRule { trigger_pct: 90.0, action: EscalationAction::Notify { message: "Change advisory at 90% of SLA".into() } },
                ],
            },
        ],
    }
}
