//! Segment plugin system — self-contained, composable vertical modules.
//!
//! Each segment is a plugin that bundles together:
//!   - Which compliance/policy services to activate (AgentServices)
//!   - Which connectors to register (inbound triggers + outbound delivery)
//!   - Which SLA policy applies
//!   - Which tenant-specific policy rules apply
//!
//! Usage in main.rs:
//!   let registry = SegmentRegistry::builder()
//!       .add(segments::engineering::plugin())
//!       .add(segments::customer_support::plugin())
//!       .build(shared_deps);
//!
//! One tenant can activate multiple segments simultaneously.
//! An agent whose goal matches multiple segments uses the union of services
//! from all active segments — the most permissive policy wins per tool.

pub mod compliance_ops;
pub mod customer_success_renewals;
pub mod customer_support;
pub mod data_analytics;
pub mod domain;
pub mod engineering;
pub mod finance_accounting;
pub mod hr_people_ops;
pub mod it_ops_itsm;
pub mod legal_contract;
pub mod marketing_growth;
pub mod procurement_vendor_ops;
pub mod research_intelligence;
pub mod sales_revops;
pub mod security_ops_grc;

#[allow(unused_imports)]
pub use domain::{DomainProfile, JudgementTuning};
#[allow(unused_imports)]
pub use registry::{AgentServices, SegmentPlugin, SegmentRegistry, SegmentRegistryBuilder, SharedDeps};
mod registry;

#[cfg(test)]
mod tests;
