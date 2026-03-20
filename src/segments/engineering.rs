//! Engineering Maintenance segment plugin.
//! Covers: code review, CI/CD, repo maintenance, incident response for SWEs.

use std::sync::Arc;
use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::github::GitHubConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Require human approval before any kubernetes destructive ops
    rules.rules.push(PolicyRule {
        id: "eng-k8s-delete-approval".into(),
        name: "Kubernetes delete requires approval".into(),
        tools: vec!["kubernetes".into()],
        condition: PolicyCondition::ArgsMatch { pattern: r#""action"\s*:\s*"delete""#.into() },
        action: PolicyAction::RequireApproval {
            message: "Kubernetes delete operations require engineer approval".into(),
        },
        enabled: true,
    });

    // Block direct prod DB writes without approval
    rules.rules.push(PolicyRule {
        id: "eng-prod-db-write".into(),
        name: "Production DB writes require approval".into(),
        tools: vec!["sql_query".into(), "shell".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(DROP|DELETE|TRUNCATE|INSERT|UPDATE).*prod"#.into(),
        },
        action: PolicyAction::RequireApproval {
            message: "Destructive SQL against production requires human approval".into(),
        },
        enabled: true,
    });

    SegmentPlugin {
        id:   "engineering",
        name: "Engineering Maintenance",
        connectors: vec![Arc::new(GitHubConnector::new())],
        services: SegmentServices {
            policy:    Some(deps.policy_engine.clone()),
            reviews:   Some(deps.review_queue.clone()),
            citations: None,
            sla:       None,
            evidence:  None,
            pii:       None,
        },
        policy_rules: rules,
        sla_policies: vec![
            SlaPolicy {
                id: "eng-incident-sla".into(),
                tenant_id: tenant_id.into(),
                name: "P1 incident response".into(),
                first_response_mins: 15,
                resolution_mins: 240,
                priority: SlaPriority::Critical,
                escalation_rules: vec![
                    EscalationRule {
                        trigger_pct: 50.0,
                        action: EscalationAction::Notify {
                            message: "P1 incident: 50% of SLA elapsed".into(),
                        },
                    },
                    EscalationRule {
                        trigger_pct: 90.0,
                        action: EscalationAction::EscalateToHuman {
                            reason: "P1 incident approaching SLA breach".into(),
                        },
                    },
                ],
            },
        ],
    }
}
