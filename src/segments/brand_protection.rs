//! Brand Protection & Monitoring segment plugin.
//! Covers: website defacement detection, content change monitoring, competitor intelligence,
//! reputation management, social media monitoring, and trademark protection.

use crate::{
    connectors::brand_monitoring::BrandMonitoringConnector,
    policy::rules::{PolicyAction, PolicyCondition, PolicyRule, PolicyRuleSet},
    segments::{
        registry::{SegmentPlugin, SegmentServices, SharedDeps},
        DomainProfile,
    },
};
use std::sync::Arc;

pub fn plugin(deps: &SharedDeps, tenant_id: &str) -> SegmentPlugin {
    let mut rules = PolicyRuleSet::new(tenant_id.into());

    // All brand monitoring alerts must be escalated to human review
    rules.rules.push(PolicyRule {
        id: "brand-monitor-escalate".into(),
        name: "Brand protection alerts require approval before action".into(),
        tools: vec!["notification".into(), "email".into(), "api_call".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(defacement|trademark|violation|emergency|escalate|incident)"#.into(),
        },
        action: PolicyAction::RequireApproval {
            message: "Brand protection issues must be reviewed by the legal/ops team before escalation".into(),
        },
        enabled: true,
    });

    // Block any direct modifications to website content without review
    rules.rules.push(PolicyRule {
        id: "brand-modify-website-review".into(),
        name: "Website modifications require legal review".into(),
        tools: vec!["file_write".into(), "api_call".into(), "http_request".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(PUT|PATCH|POST|DELETE|upload|modify|replace|deploy|publish|update).*website"#.into(),
        },
        action: PolicyAction::RequireApproval {
            message: "Any website modifications must be approved by authorized personnel".into(),
        },
        enabled: true,
    });

    // Block automated takedown notices without review
    rules.rules.push(PolicyRule {
        id: "brand-protect-legal-hold".into(),
        name: "Legal actions require human authorization".into(),
        tools: vec!["email".into(), "api_call".into()],
        condition: PolicyCondition::ArgsMatch {
            pattern: r#"(DMCA|takedown|cease.*desist|violation|copyright|trademark.*claim)"#.into(),
        },
        action: PolicyAction::RequireApproval {
            message: "Legal actions (takedowns, DMCA notices) require attorney review".into(),
        },
        enabled: true,
    });

    // Log all competitor intelligence gathering
    rules.rules.push(PolicyRule {
        id: "brand-monitor-research".into(),
        name: "Competitor research requires documentation".into(),
        tools: vec!["web_fetch".into(), "web_search_tool".into(), "screenshot".into()],
        condition: PolicyCondition::Always,
        action: PolicyAction::RequireApproval {
            message: "All competitive intelligence must be documented with sources for audit trail".into(),
        },
        enabled: true,
    });

    SegmentPlugin {
        id: "brand_protection",
        name: "Brand Protection & Monitoring",
        domain: DomainProfile::brand_protection(),
        connectors: vec![Arc::new(BrandMonitoringConnector::new())],
        services: SegmentServices {
            policy: Some(deps.policy_engine.clone()),
            citations: Some(deps.citation_tracker.clone()),
            reviews: Some(deps.review_queue.clone()),
            evidence: Some(deps.evidence_packager.clone()),
            pii: None,
            sla: None,
        },
        policy_rules: rules,
        sla_policies: vec![],
    }
}
