//! Customer Success & Renewals segment plugin.
//! Covers: account health, renewals, churn risk, QBR prep, and escalation follow-up.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::{
        freshdesk::FreshdeskConnector, hubspot::HubSpotConnector, intercom::IntercomConnector,
        salesforce::SalesforceConnector, zendesk::ZendeskConnector,
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

    // Renewals and discounting need human approval.
    rules.rules.push(PolicyRule {
        id: "cs-renewal-discount-review".into(),
        name: "Renewal discounts require approval".into(),
        tools: vec!["api_call".into(), "email".into()],
        condition: PolicyCondition::ArgThreshold { field: "discount_pct".into(), max: 20.0 },
        action: PolicyAction::RequireApproval {
            message: "Renewal discounts above 20% require account manager approval".into(),
        },
        enabled: true,
    });

    // High-churn or VIP accounts should be escalated for review.
    rules.rules.push(PolicyRule {
        id: "cs-at-risk-escalation".into(),
        name: "At-risk customer actions require review".into(),
        tools: vec!["api_call".into(), "email".into(), "http_request".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(churn|renewal|vip|at risk|escalat|escalation|save|retention)"#.into(),
        },
        action: PolicyAction::RequireApproval {
            message: "Customer success follow-ups on at-risk accounts require review".into(),
        },
        enabled: true,
    });

    // Customer data should never leak into public output.
    rules.rules.push(PolicyRule {
        id: "cs-pii-redact".into(),
        name: "Redact customer contact details".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec![
                "personal_email".into(),
                "personal_phone".into(),
                "home_address".into(),
                "payment_card".into(),
            ],
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "customer_success_renewals",
        name: "Customer Success & Renewals",
        domain: DomainProfile::customer_success_renewals(),
        connectors: vec![
            Arc::new(SalesforceConnector::new()),
            Arc::new(HubSpotConnector::new()),
            Arc::new(ZendeskConnector::new()),
            Arc::new(IntercomConnector::new()),
            Arc::new(FreshdeskConnector::new()),
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
            id: "cs-renewal-response-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Renewal response turnaround".into(),
            first_response_mins: 120,
            resolution_mins: 1440,
            priority: SlaPriority::High,
            escalation_rules: vec![
                EscalationRule {
                    trigger_pct: 80.0,
                    action: EscalationAction::Notify { message: "Renewal response at 80% of SLA".into() },
                },
                EscalationRule {
                    trigger_pct: 100.0,
                    action: EscalationAction::EscalateToHuman { reason: "Renewal response SLA breached".into() },
                },
            ],
        }],
    }
}
