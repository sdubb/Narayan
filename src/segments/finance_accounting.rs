//! Finance & Accounting segment plugin.
//! Covers: invoice processing, reconciliation, expense categorisation, month-end close.

use std::sync::Arc;
use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::quickbooks::QuickBooksConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Any write to a financial system over $10k requires approval
    rules.rules.push(PolicyRule {
        id: "finance-large-transaction".into(),
        name: "Large financial transactions require approval".into(),
        tools: vec!["api_call".into()],
        condition: PolicyCondition::ArgThreshold { field: "amount".into(), max: 10_000.0 },
        action: PolicyAction::RequireApproval {
            message: "Transaction over $10,000 requires finance controller approval".into(),
        },
        enabled: true,
    });

    // Block any deletion from financial records
    rules.rules.push(PolicyRule {
        id: "finance-no-delete".into(),
        name: "Financial record deletion is blocked".into(),
        tools: vec!["sql_query".into()],
        condition: PolicyCondition::ArgsMatch { pattern: r"DELETE\s+FROM".into() },
        action: PolicyAction::Block {
            reason: "Financial records cannot be deleted — use void/reversal instead".into(),
        },
        enabled: true,
    });

    // Always redact PII in financial outputs
    rules.rules.push(PolicyRule {
        id: "finance-pii-redact".into(),
        name: "Redact PII in financial data".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec!["ssn".into(), "account_number".into(), "routing_number".into()],
        },
        enabled: true,
    });

    SegmentPlugin {
        id:   "finance_accounting",
        name: "Finance & Accounting",
        connectors: vec![Arc::new(QuickBooksConnector::new())],
        services: SegmentServices {
            policy:    Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews:   Some(deps.review_queue.clone()),
            evidence:  Some(deps.evidence_packager.clone()),
            pii:       Some(deps.pii_redactor.clone()),
            sla:       None,
        },
        policy_rules: rules,
        sla_policies: vec![
            SlaPolicy {
                id: "finance-close-sla".into(),
                tenant_id: tenant_id.into(),
                name: "Month-end close SLA".into(),
                first_response_mins: 120,
                resolution_mins: 2880, // 48h
                priority: SlaPriority::High,
                escalation_rules: vec![
                    EscalationRule {
                        trigger_pct: 80.0,
                        action: EscalationAction::Notify { message: "Month-end close at 80% of deadline".into() },
                    },
                    EscalationRule {
                        trigger_pct: 100.0,
                        action: EscalationAction::EscalateToHuman { reason: "Month-end close SLA breached".into() },
                    },
                ],
            },
        ],
    }
}
