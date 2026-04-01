//! Sales & RevOps segment plugin.
//! Covers: prospect research, CRM enrichment, outreach, pipeline intelligence.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::{
        salesforce::SalesforceConnector,
        shipstation::ShipStationConnector,
        shopify::ShopifyConnector,
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

    // Never send outreach without review
    rules.rules.push(PolicyRule {
        id: "sales-outreach-review".into(),
        name: "Outreach emails require approval".into(),
        tools: vec!["email".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::RequireApproval {
            message: "Sales outreach must be reviewed by rep before sending".into(),
        },
        enabled: true,
    });

    // Redact any PII from enrichment writes
    rules.rules.push(PolicyRule {
        id: "sales-pii-redact".into(),
        name: "Redact sensitive PII in CRM enrichment".into(),
        tools: vec!["api_call".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact { fields: vec!["ssn".into(), "credit_card".into(), "personal_phone".into()] },
        enabled: true,
    });

    SegmentPlugin {
        id: "sales_revops",
        name: "Sales & RevOps",
        domain: DomainProfile::sales_revops(),
        connectors: vec![
            Arc::new(SalesforceConnector::new()),
            Arc::new(ShopifyConnector::new()),
            Arc::new(ShipStationConnector::new()),
        ],
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
            id: "sales-renewal-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Renewal outreach SLA".into(),
            first_response_mins: 60,
            resolution_mins: 480,
            priority: SlaPriority::High,
            escalation_rules: vec![EscalationRule {
                trigger_pct: 80.0,
                action: EscalationAction::Notify { message: "Renewal outreach prep at 80% of SLA".into() },
            }],
        }],
    }
}
