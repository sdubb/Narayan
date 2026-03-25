//! Data & Analytics Ops segment plugin.
//! Covers: pipeline monitoring, data quality checks, scheduled reports, schema migrations.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::dbt_cloud::DbtCloudConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::{
        registry::{SegmentPlugin, SegmentServices, SharedDeps},
        DomainProfile,
    },
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    rules.rules.push(PolicyRule {
        id: "data-schema-migration-approval".into(),
        name: "Schema migrations require approval".into(),
        tools: vec!["sql_query".into(), "shell".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r"(ALTER TABLE|DROP COLUMN|DROP TABLE|RENAME|MODIFY COLUMN)".into(),
        },
        action: PolicyAction::RequireApproval { message: "Schema migrations require data engineering approval".into() },
        enabled: true,
    });

    rules.rules.push(PolicyRule {
        id: "data-no-prod-truncate".into(),
        name: "Block TRUNCATE on production tables".into(),
        tools: vec!["sql_query".into()],
        condition: PolicyCondition::ArgsMatch { pattern: r"TRUNCATE".into() },
        action: PolicyAction::Block {
            reason: "TRUNCATE is irreversible — use DELETE with WHERE or soft-delete instead".into(),
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "data_analytics",
        name: "Data & Analytics Ops",
        domain: DomainProfile::data_analytics(),
        connectors: vec![Arc::new(DbtCloudConnector::new())],
        services: SegmentServices {
            policy: Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews: Some(deps.review_queue.clone()),
            evidence: None,
            pii: Some(deps.pii_redactor.clone()),
            sla: None,
        },
        policy_rules: rules,
        sla_policies: vec![SlaPolicy {
            id: "data-pipeline-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Pipeline failure SLA".into(),
            first_response_mins: 30,
            resolution_mins: 240,
            priority: SlaPriority::High,
            escalation_rules: vec![
                EscalationRule {
                    trigger_pct: 75.0,
                    action: EscalationAction::Notify { message: "Data pipeline failure at 75% of SLA".into() },
                },
                EscalationRule {
                    trigger_pct: 100.0,
                    action: EscalationAction::EscalateToHuman {
                        reason: "Data pipeline SLA breached — escalating to data engineering".into(),
                    },
                },
            ],
        }],
    }
}
