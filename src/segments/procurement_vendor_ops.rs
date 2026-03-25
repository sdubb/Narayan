//! Procurement & Vendor Ops segment plugin.
//! Covers: vendor intake, purchase approvals, contract routing, invoice matching, and renewals.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::{
        docusign::DocuSignConnector, notion::NotionConnector, quickbooks::QuickBooksConnector, stripe::StripeConnector,
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

    // Purchase orders above the threshold need approval.
    rules.rules.push(PolicyRule {
        id: "procurement-large-purchase".into(),
        name: "Large purchase requests require approval".into(),
        tools: vec!["api_call".into(), "http_request".into()],
        condition: PolicyCondition::ArgThreshold { field: "amount".into(), max: 5_000.0 },
        action: PolicyAction::RequireApproval {
            message: "Purchases above $5,000 require finance/procurement approval".into(),
        },
        enabled: true,
    });

    // Vendor banking and payment changes are high risk.
    rules.rules.push(PolicyRule {
        id: "procurement-vendor-payment-change".into(),
        name: "Vendor payment changes require review".into(),
        tools: vec!["api_call".into(), "http_request".into(), "file_write".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(bank|routing|account|beneficiary|wire|payment|remit)"#.into(),
        },
        action: PolicyAction::RequireApproval { message: "Vendor payment detail changes require human review".into() },
        enabled: true,
    });

    // Never leak banking or tax details into external outputs.
    rules.rules.push(PolicyRule {
        id: "procurement-pii-redact".into(),
        name: "Redact vendor financial identifiers".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec!["tax_id".into(), "bank_account".into(), "routing_number".into(), "payment_card".into()],
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "procurement_vendor_ops",
        name: "Procurement & Vendor Ops",
        domain: DomainProfile::procurement_vendor_ops(),
        connectors: vec![
            Arc::new(DocuSignConnector::new()),
            Arc::new(QuickBooksConnector::new()),
            Arc::new(StripeConnector::new()),
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
            id: "procurement-vendor-onboarding-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Vendor onboarding turnaround".into(),
            first_response_mins: 240,
            resolution_mins: 2880,
            priority: SlaPriority::High,
            escalation_rules: vec![
                EscalationRule {
                    trigger_pct: 80.0,
                    action: EscalationAction::Notify { message: "Vendor onboarding at 80% of SLA".into() },
                },
                EscalationRule {
                    trigger_pct: 100.0,
                    action: EscalationAction::EscalateToHuman { reason: "Vendor onboarding SLA breached".into() },
                },
            ],
        }],
    }
}
