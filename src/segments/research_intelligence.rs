//! Research & Intelligence segment plugin.
//! Covers: market research, competitive intel, M&A due diligence, scientific synthesis.
//! Note: ResearchAnalyst JobType already exists — this adds the plugin wrapper.

use crate::{
    connectors::notion::NotionConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::{
        registry::{SegmentPlugin, SegmentServices, SharedDeps},
        DomainProfile,
    },
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // Block publishing research without review
    rules.rules.push(PolicyRule {
        id: "research-publish-review".into(),
        name: "Research outputs require review before publishing".into(),
        tools: vec!["email".into(), "api_call".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::RequireApproval {
            message: "Research findings must be reviewed before external distribution".into(),
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "research_intelligence",
        name: "Research & Intelligence",
        domain: DomainProfile::research_intelligence(),
        connectors: vec![Arc::new(NotionConnector::new())],
        services: SegmentServices {
            policy: Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews: Some(deps.review_queue.clone()),
            evidence: Some(deps.evidence_packager.clone()),
            pii: Some(deps.pii_redactor.clone()),
            sla: None,
        },
        policy_rules: rules,
        sla_policies: vec![],
    }
}
