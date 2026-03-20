//! Segment registry — the plugin host.
//!
//! Holds all registered segment plugins and provides:
//!   - Connector registry (for inbound webhook routing)
//!   - Merged AgentServices (union of all active segments' services)
//!   - Per-segment SLA policy lookup
//!   - Per-segment tenant policy rules

use std::sync::Arc;

use crate::{
    compliance::{CitationTracker, EvidencePackager, PiiRedactor, ReviewQueue, SlaPolicy, SlaTracker},
    connectors::{Connector, ConnectorRegistry},
    policy::{PolicyEngine, PolicyRuleSet},
};

/// Everything a segment plugin needs access to at construction time.
/// All fields are pre-built shared instances from main.rs.
pub struct SharedDeps {
    pub policy_engine:    Arc<PolicyEngine>,
    pub citation_tracker: Arc<CitationTracker>,
    pub review_queue:     Arc<ReviewQueue>,
    pub evidence_packager: Arc<EvidencePackager>,
    pub pii_redactor:     Arc<PiiRedactor>,
}

/// Services activated by a segment — subset of SharedDeps.
/// None means the segment does not use that service.
pub struct SegmentServices {
    pub policy:    Option<Arc<PolicyEngine>>,
    pub citations: Option<Arc<CitationTracker>>,
    pub sla:       Option<Arc<SlaTracker>>,
    pub reviews:   Option<Arc<ReviewQueue>>,
    pub evidence:  Option<Arc<EvidencePackager>>,
    pub pii:       Option<Arc<PiiRedactor>>,
}

impl SegmentServices {
    pub fn none() -> Self {
        Self { policy: None, citations: None, sla: None,
               reviews: None, evidence: None, pii: None }
    }
}

/// A self-contained segment plugin.
pub struct SegmentPlugin {
    /// Unique identifier used in logs and config.
    pub id: &'static str,
    /// Human-readable name shown in admin UI.
    pub name: &'static str,
    /// Connectors this segment contributes to the inbound registry.
    pub connectors: Vec<Arc<dyn Connector>>,
    /// Services this segment activates.
    pub services: SegmentServices,
    /// Tenant policy rules added on top of platform defaults.
    pub policy_rules: PolicyRuleSet,
    /// SLA policies this segment registers.
    pub sla_policies: Vec<SlaPolicy>,
}

/// Builder for assembling multiple segment plugins.
pub struct SegmentRegistryBuilder {
    plugins: Vec<SegmentPlugin>,
}

impl SegmentRegistryBuilder {
    pub fn new() -> Self {
        Self { plugins: Vec::new() }
    }

    pub fn add(mut self, plugin: SegmentPlugin) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Build the registry. Merges all segments into:
    ///   - A single ConnectorRegistry
    ///   - A merged AgentServices (union of all active services)
    ///   - A merged SlaTracker with all registered policies
    pub fn build(self) -> SegmentRegistry {
        let mut connector_registry = ConnectorRegistry::new();
        let mut merged_services    = MergedServices::default();
        let mut all_sla_policies   = Vec::new();
        let mut all_policy_rules   = Vec::new();

        for plugin in self.plugins {
            // Register connectors
            for conn in plugin.connectors {
                connector_registry.register(conn);
            }

            // Merge services — if ANY active segment enables a service, it's on
            if let Some(p) = plugin.services.policy    { merged_services.policy    = Some(p); }
            if let Some(c) = plugin.services.citations  { merged_services.citations  = Some(c); }
            if let Some(r) = plugin.services.reviews    { merged_services.reviews    = Some(r); }
            if let Some(e) = plugin.services.evidence   { merged_services.evidence   = Some(e); }
            if let Some(p) = plugin.services.pii        { merged_services.pii        = Some(p); }

            // Collect SLA policies
            all_sla_policies.extend(plugin.sla_policies);

            // Collect policy rules
            all_policy_rules.extend(plugin.policy_rules.rules);
        }

        // Build merged SLA tracker from union of all policies
        let sla_tracker = if all_sla_policies.is_empty() {
            None
        } else {
            Some(Arc::new(SlaTracker::new(all_sla_policies)))
        };

        // Build merged policy ruleset
        let merged_ruleset = PolicyRuleSet {
            tenant_id: "merged".into(),
            rules: all_policy_rules,
        };

        SegmentRegistry {
            connector_registry,
            merged_services,
            sla_tracker,
            merged_ruleset,
        }
    }
}

impl Default for SegmentRegistryBuilder {
    fn default() -> Self { Self::new() }
}

/// Merged output of all registered segment plugins.
pub struct SegmentRegistry {
    pub connector_registry: ConnectorRegistry,
    pub sla_tracker:        Option<Arc<SlaTracker>>,
    pub merged_ruleset:     PolicyRuleSet,
    merged_services:        MergedServices,
}

impl SegmentRegistry {
    pub fn builder() -> SegmentRegistryBuilder {
        SegmentRegistryBuilder::new()
    }

    /// Materialise the merged AgentServices struct for injection into AgentLoop/Executor/Worker.
    pub fn agent_services(&self) -> AgentServices {
        AgentServices {
            policy:    self.merged_services.policy.clone(),
            citations: self.merged_services.citations.clone(),
            sla:       self.sla_tracker.clone(),
            reviews:   self.merged_services.reviews.clone(),
            evidence:  self.merged_services.evidence.clone(),
            pii:       self.merged_services.pii.clone(),
        }
    }
}

/// Intermediate merge accumulator (all Option so we can build up from multiple segments).
#[derive(Default)]
struct MergedServices {
    policy:    Option<Arc<PolicyEngine>>,
    citations: Option<Arc<CitationTracker>>,
    reviews:   Option<Arc<ReviewQueue>>,
    evidence:  Option<Arc<EvidencePackager>>,
    pii:       Option<Arc<PiiRedactor>>,
}

/// The final injectable services struct — passed into AgentLoop, LlmExecutor, Worker.
/// This replaces the previous separate wiring approach.
pub struct AgentServices {
    pub policy:    Option<Arc<PolicyEngine>>,
    pub citations: Option<Arc<CitationTracker>>,
    pub sla:       Option<Arc<SlaTracker>>,
    pub reviews:   Option<Arc<ReviewQueue>>,
    pub evidence:  Option<Arc<EvidencePackager>>,
    pub pii:       Option<Arc<PiiRedactor>>,
}

impl AgentServices {
    /// Zero services — for unit tests and minimal deployments.
    pub fn none() -> Self {
        Self { policy: None, citations: None, sla: None,
               reviews: None, evidence: None, pii: None }
    }
}
