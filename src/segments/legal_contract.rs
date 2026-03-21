//! Legal & Contract Ops segment plugin.
//! Covers: contract review, clause extraction, redlining, due diligence.

use crate::{
    compliance::sla::{EscalationAction, EscalationRule, SlaPolicy, SlaPriority},
    connectors::docusign::DocuSignConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // All legal outputs require attorney review — hard block on direct delivery
    rules.rules.push(PolicyRule {
        id: "legal-attorney-review".into(),
        name: "Legal outputs require attorney review".into(),
        tools: vec!["email".into(), "api_call".into(), "http_request".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::RequireApproval {
            message: "Legal agent outputs require attorney sign-off before delivery".into(),
        },
        enabled: true,
    });

    // Block agents from executing contracts — review only
    rules.rules.push(PolicyRule {
        id: "legal-no-signing".into(),
        name: "Agents cannot execute contracts".into(),
        tools: vec!["api_call".into()],
        condition: PolicyCondition::ArgsMatch { pattern: r#"(sign|execute|countersign|envelope_send)"#.into() },
        action: PolicyAction::Block {
            reason: "Contract execution requires human authorisation — agents are review-only".into(),
        },
        enabled: true,
    });

    // Always redact PII from legal documents
    rules.rules.push(PolicyRule {
        id: "legal-pii-redact".into(),
        name: "Redact PII in legal document outputs".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec!["ssn".into(), "dob".into(), "passport_number".into(), "personal_address".into()],
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "legal_contract",
        name: "Legal & Contract Ops",
        connectors: vec![Arc::new(DocuSignConnector::new())],
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
            id: "legal-contract-review-sla".into(),
            tenant_id: tenant_id.into(),
            name: "Contract review turnaround".into(),
            first_response_mins: 120,
            resolution_mins: 2880,
            priority: SlaPriority::High,
            escalation_rules: vec![
                EscalationRule {
                    trigger_pct: 80.0,
                    action: EscalationAction::Notify { message: "Contract review at 80% of SLA".into() },
                },
                EscalationRule {
                    trigger_pct: 100.0,
                    action: EscalationAction::EscalateToHuman { reason: "Contract review SLA breached".into() },
                },
            ],
        }],
    }
}
