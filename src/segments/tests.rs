//! Integration tests for all segment plugins.
//!
//! Tests verify that each plugin:
//!   - Constructs without panicking
//!   - Has sane SLA values (first_response < resolution)
//!   - Registers at least one policy rule where expected
//!   - Has a connector for the appropriate segment
//!   - Services are correctly enabled/disabled per segment design

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        compliance::{CitationTracker, EvidencePackager, PiiRedactor, ReviewQueue, SlaTracker},
        policy::PolicyEngine,
        segments::{
            compliance_ops, customer_support, data_analytics, engineering,
            finance_accounting, hr_people_ops, it_ops_itsm, legal_contract,
            marketing_growth, research_intelligence, sales_revops,
            registry::{SegmentPlugin, SegmentRegistry, SharedDeps},
        },
    };

    // ── Test helpers ──────────────────────────────────────────────────────

    fn fake_deps() -> SharedDeps {
        use sqlx::postgres::PgPoolOptions;
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://narayan:narayan@localhost/narayan")
            .expect("lazy pool");
        SharedDeps {
            policy_engine:     Arc::new(PolicyEngine::new()),
            citation_tracker:  Arc::new(CitationTracker::new(pool.clone())),
            review_queue:      Arc::new(ReviewQueue::new(pool.clone())),
            evidence_packager: Arc::new(EvidencePackager::new(
                Arc::new(CitationTracker::new(pool.clone())),
                Arc::new(crate::audit::AuditLog::new(pool)),
            )),
            pii_redactor: Arc::new(PiiRedactor::new()),
        }
    }

    fn all_plugins(deps: &SharedDeps) -> Vec<SegmentPlugin> {
        vec![
            engineering::plugin(deps, "t1"),
            customer_support::plugin(deps, "t1"),
            compliance_ops::plugin(deps, "t1"),
            sales_revops::plugin(deps, "t1"),
            finance_accounting::plugin(deps, "t1"),
            hr_people_ops::plugin(deps, "t1"),
            legal_contract::plugin(deps, "t1"),
            it_ops_itsm::plugin(deps, "t1"),
            research_intelligence::plugin(deps, "t1"),
            data_analytics::plugin(deps, "t1"),
            marketing_growth::plugin(deps, "t1"),
        ]
    }

    // ── Plugin construction ───────────────────────────────────────────────

    #[test]
    fn test_all_plugins_construct_without_panic() {
        let deps = fake_deps();
        let plugins = all_plugins(&deps);
        assert_eq!(plugins.len(), 11, "expected 11 segment plugins");
    }

    #[test]
    fn test_all_plugin_ids_are_unique() {
        let deps = fake_deps();
        let ids: Vec<&str> = all_plugins(&deps).iter().map(|p| p.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "duplicate plugin IDs detected");
    }

    #[test]
    fn test_all_plugin_names_are_non_empty() {
        let deps = fake_deps();
        for plugin in all_plugins(&deps) {
            assert!(!plugin.name.is_empty(), "plugin '{}' has empty name", plugin.id);
        }
    }

    // ── SLA policy sanity ─────────────────────────────────────────────────

    #[test]
    fn test_sla_policies_have_sane_deadlines() {
        let deps = fake_deps();
        for plugin in all_plugins(&deps) {
            for sla in &plugin.sla_policies {
                assert!(
                    sla.first_response_mins < sla.resolution_mins,
                    "plugin '{}' SLA '{}': first_response ({}) must be < resolution ({})",
                    plugin.id, sla.name, sla.first_response_mins, sla.resolution_mins
                );
                assert!(
                    sla.first_response_mins > 0,
                    "plugin '{}' SLA '{}': first_response must be > 0",
                    plugin.id, sla.name
                );
                assert!(
                    !sla.escalation_rules.is_empty(),
                    "plugin '{}' SLA '{}': must have at least one escalation rule",
                    plugin.id, sla.name
                );
            }
        }
    }

    #[test]
    fn test_sla_escalation_trigger_percentages_are_valid() {
        let deps = fake_deps();
        for plugin in all_plugins(&deps) {
            for sla in &plugin.sla_policies {
                for rule in &sla.escalation_rules {
                    assert!(
                        rule.trigger_pct > 0.0 && rule.trigger_pct <= 100.0,
                        "plugin '{}' SLA '{}': trigger_pct {} is not in (0, 100]",
                        plugin.id, sla.name, rule.trigger_pct
                    );
                }
            }
        }
    }

    // ── Policy rules ──────────────────────────────────────────────────────

    #[test]
    fn test_high_risk_segments_have_policy_rules() {
        let deps = fake_deps();
        let must_have_rules = ["compliance_ops", "legal_contract", "finance_accounting", "hr_people_ops"];
        for plugin in all_plugins(&deps) {
            if must_have_rules.contains(&plugin.id) {
                assert!(
                    !plugin.policy_rules.rules.is_empty(),
                    "high-risk segment '{}' must have policy rules",
                    plugin.id
                );
            }
        }
    }

    #[test]
    fn test_all_policy_rules_have_non_empty_ids() {
        let deps = fake_deps();
        for plugin in all_plugins(&deps) {
            for rule in &plugin.policy_rules.rules {
                assert!(
                    !rule.id.is_empty(),
                    "plugin '{}': policy rule has empty id",
                    plugin.id
                );
                assert!(
                    !rule.name.is_empty(),
                    "plugin '{}': rule '{}' has empty name",
                    plugin.id, rule.id
                );
            }
        }
    }

    #[test]
    fn test_compliance_segment_requires_review_for_all_outputs() {
        let deps    = fake_deps();
        let plugin  = compliance_ops::plugin(&deps, "t1");
        let has_universal_review = plugin.policy_rules.rules.iter().any(|r| {
            matches!(&r.action, crate::policy::rules::PolicyAction::RequireApproval { .. })
        });
        assert!(has_universal_review, "compliance segment must require approval on outputs");
    }

    #[test]
    fn test_legal_segment_blocks_signing() {
        let deps   = fake_deps();
        let plugin = legal_contract::plugin(&deps, "t1");
        let blocks_signing = plugin.policy_rules.rules.iter().any(|r| {
            matches!(&r.action, crate::policy::rules::PolicyAction::Block { .. })
                && r.id.contains("signing")
        });
        assert!(blocks_signing, "legal segment must block agent contract signing");
    }

    #[test]
    fn test_finance_segment_blocks_truncate() {
        let deps   = fake_deps();
        let plugin = finance_accounting::plugin(&deps, "t1");
        let blocks_truncate = plugin.policy_rules.rules.iter().any(|r| {
            matches!(&r.action, crate::policy::rules::PolicyAction::Block { .. })
                && r.id.contains("delete")
        });
        assert!(blocks_truncate, "finance segment must block destructive SQL");
    }

    #[test]
    fn test_hr_segment_requires_review_for_hiring_actions() {
        let deps   = fake_deps();
        let plugin = hr_people_ops::plugin(&deps, "t1");
        let has_hire_review = plugin.policy_rules.rules.iter().any(|r| {
            matches!(&r.action, crate::policy::rules::PolicyAction::RequireApproval { .. })
                && r.id.contains("hiring")
        });
        assert!(has_hire_review, "HR segment must require approval for hiring actions");
    }

    // ── Services activation ───────────────────────────────────────────────

    #[test]
    fn test_policy_engine_active_in_all_segments() {
        let deps = fake_deps();
        for plugin in all_plugins(&deps) {
            assert!(
                plugin.services.policy.is_some(),
                "segment '{}': policy must always be active",
                plugin.id
            );
        }
    }

    #[test]
    fn test_pii_redactor_active_in_customer_facing_segments() {
        let deps = fake_deps();
        let must_have_pii = [
            "customer_support", "compliance_ops", "sales_revops",
            "finance_accounting", "hr_people_ops", "legal_contract",
            "marketing_growth",
        ];
        for plugin in all_plugins(&deps) {
            if must_have_pii.contains(&plugin.id) {
                assert!(
                    plugin.services.pii.is_some(),
                    "segment '{}' must have PII redaction active",
                    plugin.id
                );
            }
        }
    }

    #[test]
    fn test_evidence_packager_active_in_audit_grade_segments() {
        let deps = fake_deps();
        let must_have_evidence = [
            "compliance_ops", "finance_accounting", "legal_contract", "it_ops_itsm",
        ];
        for plugin in all_plugins(&deps) {
            if must_have_evidence.contains(&plugin.id) {
                assert!(
                    plugin.services.evidence.is_some(),
                    "segment '{}' must have evidence packaging active",
                    plugin.id
                );
            }
        }
    }

    #[test]
    fn test_engineering_segment_does_not_activate_evidence_packager() {
        let deps   = fake_deps();
        let plugin = engineering::plugin(&deps, "t1");
        assert!(
            plugin.services.evidence.is_none(),
            "engineering segment should not activate evidence packaging — unnecessary overhead"
        );
    }

    #[test]
    fn test_review_queue_active_in_all_segments() {
        let deps = fake_deps();
        for plugin in all_plugins(&deps) {
            assert!(
                plugin.services.reviews.is_some(),
                "segment '{}': review queue must always be active",
                plugin.id
            );
        }
    }

    // ── Connector registration ────────────────────────────────────────────

    #[test]
    fn test_all_segment_connectors_report_correct_type() {
        let deps = fake_deps();
        for plugin in all_plugins(&deps) {
            for conn in &plugin.connectors {
                let t = conn.connector_type();
                assert!(!t.is_empty(), "segment '{}': connector has empty type", plugin.id);
            }
        }
    }

    #[test]
    fn test_segment_registry_builder_merges_all_connectors() {
        let deps = fake_deps();
        let registry = SegmentRegistry::builder()
            .add(engineering::plugin(&deps, "t1"))
            .add(customer_support::plugin(&deps, "t1"))
            .add(sales_revops::plugin(&deps, "t1"))
            .add(it_ops_itsm::plugin(&deps, "t1"))
            .build();

        // engineering → github, customer_support → zendesk,
        // sales_revops → salesforce, it_ops_itsm → servicenow + pagerduty
        let connectors = registry.connector_registry.list();
        assert!(connectors.contains(&"github"),      "github connector must be registered");
        assert!(connectors.contains(&"zendesk"),     "zendesk connector must be registered");
        assert!(connectors.contains(&"salesforce"),  "salesforce connector must be registered");
        assert!(connectors.contains(&"servicenow"),  "servicenow connector must be registered");
        assert!(connectors.contains(&"pagerduty"),   "pagerduty connector must be registered");
    }

    #[test]
    fn test_segment_registry_merges_sla_policies() {
        let deps = fake_deps();
        let registry = SegmentRegistry::builder()
            .add(customer_support::plugin(&deps, "t1"))
            .add(it_ops_itsm::plugin(&deps, "t1"))
            .build();

        // customer_support has 2 SLA tiers, it_ops_itsm has 2 SLA tiers → 4 total
        assert!(
            registry.sla_tracker.is_some(),
            "merged registry must have an SLA tracker when any segment contributes policies"
        );
    }

    #[test]
    fn test_segment_registry_with_no_sla_policies_has_none_tracker() {
        let deps = fake_deps();
        // research_intelligence and marketing_growth have no SLA policies
        let registry = SegmentRegistry::builder()
            .add(research_intelligence::plugin(&deps, "t1"))
            .add(marketing_growth::plugin(&deps, "t1"))
            .build();

        assert!(
            registry.sla_tracker.is_none(),
            "registry with no SLA-bearing segments must not build an SlaTracker"
        );
    }

    #[test]
    fn test_agent_services_none_has_all_fields_none() {
        let svc = crate::segments::registry::AgentServices::none();
        assert!(svc.policy.is_none());
        assert!(svc.citations.is_none());
        assert!(svc.sla.is_none());
        assert!(svc.reviews.is_none());
        assert!(svc.evidence.is_none());
        assert!(svc.pii.is_none());
    }

    #[test]
    fn test_merged_services_take_union_across_segments() {
        let deps = fake_deps();
        // engineering activates policy + reviews only (no evidence)
        // compliance_ops activates all services
        let registry = SegmentRegistry::builder()
            .add(engineering::plugin(&deps, "t1"))
            .add(compliance_ops::plugin(&deps, "t1"))
            .build();

        let svc = registry.agent_services();
        // Union: evidence is on because compliance_ops has it, even though engineering doesn't
        assert!(svc.evidence.is_some(), "union of services must include evidence from compliance_ops");
        assert!(svc.policy.is_some(),   "policy must be on (both segments activate it)");
    }

    // ── Connector inbound goal generation ────────────────────────────────

    #[tokio::test]
    async fn test_github_pr_opened_generates_goal() {
        use crate::connectors::{ConnectorConfig, ConnectorEvent, framework::Connector};
        use crate::connectors::github::GitHubConnector;

        let conn  = GitHubConnector::new();
        let event = ConnectorEvent {
            connector_type: "github".into(),
            event_type:     "pull_request".into(),
            payload:        serde_json::json!({
                "action": "opened",
                "pull_request": {
                    "title": "Add login flow",
                    "html_url": "https://github.com/acme/api/pull/42",
                    "body": "Implements JWT login",
                }
            }),
            tenant_id:   "t1".into(),
            external_id: Some("42".into()),
        };
        let config = ConnectorConfig {
            id: "c1".into(), tenant_id: "t1".into(),
            connector_type: "github".into(),
            credentials: serde_json::json!({}),
            settings:    serde_json::json!({}),
            enabled:     true,
        };

        let goal = conn.handle_inbound(&event, &config).await.unwrap();
        assert!(goal.is_some(), "PR opened must produce a goal");
        let g = goal.unwrap();
        assert!(g.contains("Add login flow"), "goal must mention PR title");
        assert!(g.contains("review"), "goal must mention review");
    }

    #[tokio::test]
    async fn test_zendesk_ticket_created_generates_goal() {
        use crate::connectors::{ConnectorConfig, ConnectorEvent, framework::Connector};
        use crate::connectors::zendesk::ZendeskConnector;

        let conn  = ZendeskConnector::new();
        let event = ConnectorEvent {
            connector_type: "zendesk".into(),
            event_type:     "ticket_created".into(),
            payload:        serde_json::json!({
                "id": "12345",
                "subject": "Login page is broken",
                "description": "Users cannot log in since yesterday",
                "priority": "urgent",
            }),
            tenant_id:   "t1".into(),
            external_id: Some("12345".into()),
        };
        let config = ConnectorConfig {
            id: "c1".into(), tenant_id: "t1".into(),
            connector_type: "zendesk".into(),
            credentials: serde_json::json!({}),
            settings:    serde_json::json!({}),
            enabled:     true,
        };

        let goal = conn.handle_inbound(&event, &config).await.unwrap();
        assert!(goal.is_some());
        let g = goal.unwrap();
        assert!(g.contains("12345"),        "goal must reference ticket id");
        assert!(g.contains("urgent"),       "goal must include priority");
        assert!(g.contains("Login page"),   "goal must include subject");
    }

    #[tokio::test]
    async fn test_pagerduty_incident_triggered_generates_goal() {
        use crate::connectors::{ConnectorConfig, ConnectorEvent, framework::Connector};
        use crate::connectors::pagerduty::PagerDutyConnector;

        let conn  = PagerDutyConnector::new();
        let event = ConnectorEvent {
            connector_type: "pagerduty".into(),
            event_type:     "incident.triggered".into(),
            payload:        serde_json::json!({
                "id": "P123",
                "title": "Database CPU at 100%",
                "urgency": "high",
                "service": { "summary": "production-db" },
            }),
            tenant_id:   "t1".into(),
            external_id: Some("P123".into()),
        };
        let config = ConnectorConfig {
            id: "c1".into(), tenant_id: "t1".into(),
            connector_type: "pagerduty".into(),
            credentials: serde_json::json!({}),
            settings:    serde_json::json!({}),
            enabled:     true,
        };

        let goal = conn.handle_inbound(&event, &config).await.unwrap();
        assert!(goal.is_some());
        let g = goal.unwrap();
        assert!(g.contains("P123"),              "goal must reference incident id");
        assert!(g.contains("production-db"),     "goal must reference service");
        assert!(g.contains("Database CPU"),      "goal must include incident title");
    }

    #[tokio::test]
    async fn test_docusign_envelope_completed_generates_obligation_goal() {
        use crate::connectors::{ConnectorConfig, ConnectorEvent, framework::Connector};
        use crate::connectors::docusign::DocuSignConnector;

        let conn  = DocuSignConnector::new();
        let event = ConnectorEvent {
            connector_type: "docusign".into(),
            event_type:     "envelope_completed".into(),
            payload:        serde_json::json!({
                "envelopeId": "ENV-001",
                "emailSubject": "Master Services Agreement - Acme Corp",
            }),
            tenant_id:   "t1".into(),
            external_id: Some("ENV-001".into()),
        };
        let config = ConnectorConfig {
            id: "c1".into(), tenant_id: "t1".into(),
            connector_type: "docusign".into(),
            credentials: serde_json::json!({}),
            settings:    serde_json::json!({}),
            enabled:     true,
        };

        let goal = conn.handle_inbound(&event, &config).await.unwrap();
        assert!(goal.is_some());
        let g = goal.unwrap();
        assert!(g.contains("ENV-001"),           "goal must reference envelope id");
        assert!(g.contains("obligations"),       "completed envelope must trigger obligation extraction");
    }

    #[tokio::test]
    async fn test_dbt_cloud_job_errored_generates_triage_goal() {
        use crate::connectors::{ConnectorConfig, ConnectorEvent, framework::Connector};
        use crate::connectors::dbt_cloud::DbtCloudConnector;

        let conn  = DbtCloudConnector::new();
        let event = ConnectorEvent {
            connector_type: "dbt_cloud".into(),
            event_type:     "job.run.errored".into(),
            payload:        serde_json::json!({
                "job_name": "daily_refresh",
                "run_id":   "9001",
                "status_message": "Database error: relation does not exist",
            }),
            tenant_id:   "t1".into(),
            external_id: Some("9001".into()),
        };
        let config = ConnectorConfig {
            id: "c1".into(), tenant_id: "t1".into(),
            connector_type: "dbt_cloud".into(),
            credentials: serde_json::json!({}),
            settings:    serde_json::json!({}),
            enabled:     true,
        };

        let goal = conn.handle_inbound(&event, &config).await.unwrap();
        assert!(goal.is_some());
        let g = goal.unwrap();
        assert!(g.contains("daily_refresh"), "goal must reference job name");
        assert!(g.contains("9001"),          "goal must reference run id");
        assert!(g.contains("triage") || g.contains("findings"), "goal must be a triage task");
    }

    #[tokio::test]
    async fn test_unknown_event_type_returns_none() {
        use crate::connectors::{ConnectorConfig, ConnectorEvent, framework::Connector};
        use crate::connectors::github::GitHubConnector;

        let conn  = GitHubConnector::new();
        let event = ConnectorEvent {
            connector_type: "github".into(),
            event_type:     "repository.renamed".into(),
            payload:        serde_json::json!({}),
            tenant_id:      "t1".into(),
            external_id:    None,
        };
        let config = ConnectorConfig {
            id: "c1".into(), tenant_id: "t1".into(),
            connector_type: "github".into(),
            credentials: serde_json::json!({}),
            settings:    serde_json::json!({}),
            enabled:     true,
        };
        let goal = conn.handle_inbound(&event, &config).await.unwrap();
        assert!(goal.is_none(), "unrecognised event types must return None (no goal created)");
    }
}
