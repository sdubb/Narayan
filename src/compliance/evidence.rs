//! Evidence packaging — bundles agent work products for legal/compliance review.
//!
//! Collects: agent state, step history, tool call logs, citations, audit entries,
//! and redacted transcripts into a single exportable package.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    audit::{AuditLog, AuditQuery},
    compliance::citations::CitationTracker,
};

/// A complete evidence package for a single agent's work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePackage {
    pub agent_id: String,
    pub tenant_id: String,
    pub goal: String,
    pub status: String,
    pub citations: Vec<super::citations::Citation>,
    pub audit_entries: Vec<crate::audit::AuditEntry>,
    pub metadata: serde_json::Value,
    pub packaged_at: chrono::DateTime<chrono::Utc>,
}

pub struct EvidencePackager {
    citation_tracker: std::sync::Arc<CitationTracker>,
    audit_log: std::sync::Arc<AuditLog>,
}

impl EvidencePackager {
    pub fn new(
        citation_tracker: std::sync::Arc<CitationTracker>,
        audit_log: std::sync::Arc<AuditLog>,
    ) -> Self {
        Self { citation_tracker, audit_log }
    }

    /// Build an evidence package for a specific agent.
    pub async fn package(
        &self,
        agent_id: &str,
        tenant_id: &str,
        goal: &str,
        status: &str,
        metadata: serde_json::Value,
    ) -> Result<EvidencePackage> {
        let citations = self.citation_tracker.get_for_agent(agent_id).await?;

        let audit_entries = self
            .audit_log
            .query(&AuditQuery {
                tenant_id: Some(tenant_id.to_string()),
                agent_id: Some(agent_id.to_string()),
                ..Default::default()
            })
            .await?;

        Ok(EvidencePackage {
            agent_id: agent_id.to_string(),
            tenant_id: tenant_id.to_string(),
            goal: goal.to_string(),
            status: status.to_string(),
            citations,
            audit_entries,
            metadata,
            packaged_at: chrono::Utc::now(),
        })
    }
}
