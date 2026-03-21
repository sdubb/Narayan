//! Marketing & Growth segment plugin.
//! Covers: SEO audits, competitor monitoring, content research, campaign reporting.

use crate::{
    connectors::hubspot::HubSpotConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::registry::{SegmentPlugin, SegmentServices, SharedDeps},
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Never publish content directly — always save for human review
    rules.rules.push(PolicyRule {
        id: "marketing-publish-review".into(),
        name: "Content publishing requires review".into(),
        tools: vec!["api_call".into(), "http_request".into()],
        condition: PolicyCondition::ArgsMatch { pattern: r#"(publish|post|tweet|send_campaign|schedule_post)"#.into() },
        action: PolicyAction::RequireApproval {
            message: "Marketing content must be reviewed before publishing".into(),
        },
        enabled: true,
    });

    // Redact any personal contact data from marketing lists
    rules.rules.push(PolicyRule {
        id: "marketing-pii-redact".into(),
        name: "Redact personal contact data".into(),
        tools: vec![],
        condition: PolicyCondition::Always,
        action: PolicyAction::Redact {
            fields: vec!["personal_email".into(), "personal_phone".into(), "home_address".into()],
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "marketing_growth",
        name: "Marketing & Growth",
        connectors: vec![Arc::new(HubSpotConnector::new())],
        services: SegmentServices {
            policy: Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews: Some(deps.review_queue.clone()),
            pii: Some(deps.pii_redactor.clone()),
            evidence: None,
            sla: None,
        },
        policy_rules: rules,
        sla_policies: vec![],
    }
}
