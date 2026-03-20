use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Pricing for a single model (USD per million tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

impl ModelPricing {
    pub fn cost(&self, input_tokens: u32, output_tokens: u32) -> f64 {
        (input_tokens as f64 / 1_000_000.0) * self.input_per_million
            + (output_tokens as f64 / 1_000_000.0) * self.output_per_million
    }
}

/// Running usage totals for one agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentUsage {
    pub agent_id: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
    pub total_requests: u64,
}

/// Running usage totals for one tenant (aggregated across all agents).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TenantUsage {
    pub tenant_id: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
    pub total_requests: u64,
    /// UTC timestamp of when the current billing period started.
    pub period_start: Option<chrono::DateTime<chrono::Utc>>,
}

/// Result of a spend-limit check.
#[derive(Debug, Clone)]
pub enum SpendCheck {
    /// Under limit — proceed.
    Ok,
    /// Over limit — block the request.
    Exceeded { limit_usd: f64, current_usd: f64 },
    /// Within 20% of limit — warn but allow.
    Warning { limit_usd: f64, current_usd: f64, pct_used: f64 },
}

/// Tracks token usage and cost across all agents and tenants.
pub struct CostTracker {
    agent_usage: Arc<RwLock<HashMap<String, AgentUsage>>>,
    tenant_usage: Arc<RwLock<HashMap<String, TenantUsage>>>,
    pricing: HashMap<String, ModelPricing>,
}

impl CostTracker {
    pub fn new(pricing: HashMap<String, ModelPricing>) -> Self {
        Self {
            agent_usage: Arc::new(RwLock::new(HashMap::new())),
            tenant_usage: Arc::new(RwLock::new(HashMap::new())),
            pricing,
        }
    }

    /// Default pricing table for well-known models.
    pub fn default_pricing() -> HashMap<String, ModelPricing> {
        let mut m = HashMap::new();
        // ── Anthropic Claude 4.6 (current as of 2026) ─────────────────────
        m.insert("claude-sonnet-4-6".into(), ModelPricing { input_per_million: 3.0, output_per_million: 15.0 });
        m.insert("claude-opus-4-6".into(), ModelPricing { input_per_million: 15.0, output_per_million: 75.0 });
        m.insert("claude-haiku-4-5".into(), ModelPricing { input_per_million: 0.25, output_per_million: 1.25 });
        // ── Legacy model strings (kept for backwards compatibility) ────────
        m.insert("claude-sonnet-4-20250514".into(), ModelPricing { input_per_million: 3.0, output_per_million: 15.0 });
        m.insert(
            "claude-haiku-4-5-20251001".into(),
            ModelPricing { input_per_million: 0.25, output_per_million: 1.25 },
        );
        m.insert("claude-opus-4-5".into(), ModelPricing { input_per_million: 15.0, output_per_million: 75.0 });
        // ── OpenAI ─────────────────────────────────────────────────────────
        m.insert("gpt-4o".into(), ModelPricing { input_per_million: 2.5, output_per_million: 10.0 });
        m.insert("gpt-4o-mini".into(), ModelPricing { input_per_million: 0.15, output_per_million: 0.60 });
        m.insert("gpt-4-turbo".into(), ModelPricing { input_per_million: 10.0, output_per_million: 30.0 });
        m.insert("o1".into(), ModelPricing { input_per_million: 15.0, output_per_million: 60.0 });
        m.insert("o3-mini".into(), ModelPricing { input_per_million: 1.10, output_per_million: 4.40 });
        // ── Google Gemini ──────────────────────────────────────────────────
        m.insert("gemini-2.0-flash".into(), ModelPricing { input_per_million: 0.10, output_per_million: 0.40 });
        m.insert("gemini-2.0-pro".into(), ModelPricing { input_per_million: 1.25, output_per_million: 5.00 });
        m.insert("gemini-1.5-pro".into(), ModelPricing { input_per_million: 1.25, output_per_million: 5.00 });
        m
    }

    /// Record a completed LLM call — updates both agent and tenant usage.
    pub async fn record(
        &self,
        tenant_id: &str,
        agent_id: &str,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) {
        let cost = self.pricing.get(model).map(|p| p.cost(input_tokens, output_tokens)).unwrap_or(0.0);

        // Update per-agent usage
        {
            let mut usage = self.agent_usage.write().await;
            let entry = usage
                .entry(agent_id.to_string())
                .or_insert_with(|| AgentUsage { agent_id: agent_id.to_string(), ..Default::default() });
            entry.total_input_tokens += input_tokens as u64;
            entry.total_output_tokens += output_tokens as u64;
            entry.total_cost_usd += cost;
            entry.total_requests += 1;
        }

        // Update per-tenant usage
        {
            let mut usage = self.tenant_usage.write().await;
            let entry = usage.entry(tenant_id.to_string()).or_insert_with(|| TenantUsage {
                tenant_id: tenant_id.to_string(),
                period_start: Some(chrono::Utc::now()),
                ..Default::default()
            });
            entry.total_input_tokens += input_tokens as u64;
            entry.total_output_tokens += output_tokens as u64;
            entry.total_cost_usd += cost;
            entry.total_requests += 1;
        }

        tracing::debug!(
            tenant_id, agent_id, model, input_tokens, output_tokens, cost_usd = cost,
            "LLM call recorded"
        );
    }

    /// Check whether a tenant has exceeded their spend limit.
    /// `limit_usd` of 0.0 means unlimited (Enterprise plan).
    pub async fn check_spend_limit(&self, tenant_id: &str, limit_usd: f64) -> SpendCheck {
        // Unlimited plan
        if limit_usd <= 0.0 {
            return SpendCheck::Ok;
        }

        let current = self.get_tenant_usage(tenant_id).await.map(|u| u.total_cost_usd).unwrap_or(0.0);

        if current >= limit_usd {
            return SpendCheck::Exceeded { limit_usd, current_usd: current };
        }

        let pct = current / limit_usd;
        if pct >= 0.8 {
            return SpendCheck::Warning { limit_usd, current_usd: current, pct_used: pct * 100.0 };
        }

        SpendCheck::Ok
    }

    pub async fn get_usage(&self, agent_id: &str) -> Option<AgentUsage> {
        self.agent_usage.read().await.get(agent_id).cloned()
    }

    pub async fn get_tenant_usage(&self, tenant_id: &str) -> Option<TenantUsage> {
        self.tenant_usage.read().await.get(tenant_id).cloned()
    }

    pub async fn all_usage(&self) -> Vec<AgentUsage> {
        self.agent_usage.read().await.values().cloned().collect()
    }

    pub async fn total_cost_usd(&self) -> f64 {
        self.agent_usage.read().await.values().map(|u| u.total_cost_usd).sum()
    }

    /// Reset a tenant's usage for a new billing period.
    pub async fn reset_tenant_period(&self, tenant_id: &str) {
        let mut usage = self.tenant_usage.write().await;
        if let Some(entry) = usage.get_mut(tenant_id) {
            entry.total_input_tokens = 0;
            entry.total_output_tokens = 0;
            entry.total_cost_usd = 0.0;
            entry.total_requests = 0;
            entry.period_start = Some(chrono::Utc::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pricing_calc() {
        let pricing = ModelPricing { input_per_million: 3.0, output_per_million: 15.0 };
        let cost = pricing.cost(1_000_000, 1_000_000);
        assert!((cost - 18.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_record_usage() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        tracker.record("tenant-1", "agent1", "claude-sonnet-4-6", 1000, 500).await;
        let usage = tracker.get_usage("agent1").await.unwrap();
        assert_eq!(usage.total_input_tokens, 1000);
        assert_eq!(usage.total_output_tokens, 500);
        assert_eq!(usage.total_requests, 1);
        assert!(usage.total_cost_usd > 0.0);
    }

    #[tokio::test]
    async fn test_unknown_model_zero_cost() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        tracker.record("tenant-1", "agent1", "unknown-model-xyz", 1000, 500).await;
        let usage = tracker.get_usage("agent1").await.unwrap();
        assert!((usage.total_cost_usd - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_multi_agent_tracking() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        tracker.record("tenant-1", "agent1", "claude-sonnet-4-6", 100, 50).await;
        tracker.record("tenant-1", "agent2", "gpt-4o", 200, 100).await;
        let all = tracker.all_usage().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_tenant_usage_aggregation() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        tracker.record("tenant-1", "agent1", "claude-sonnet-4-6", 1000, 500).await;
        tracker.record("tenant-1", "agent2", "claude-sonnet-4-6", 2000, 1000).await;
        let tenant = tracker.get_tenant_usage("tenant-1").await.unwrap();
        assert_eq!(tenant.total_input_tokens, 3000);
        assert_eq!(tenant.total_output_tokens, 1500);
        assert_eq!(tenant.total_requests, 2);
    }

    #[tokio::test]
    async fn test_spend_limit_exceeded() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        // Record enough to exceed a $5 limit: 1M input tokens of claude-sonnet = $3, 1M output = $15
        tracker.record("tenant-1", "agent1", "claude-sonnet-4-6", 1_000_000, 1_000_000).await;
        match tracker.check_spend_limit("tenant-1", 5.0).await {
            SpendCheck::Exceeded { .. } => {} // expected
            other => panic!("expected Exceeded, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_spend_limit_unlimited() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        tracker.record("tenant-1", "agent1", "claude-sonnet-4-6", 1_000_000, 1_000_000).await;
        match tracker.check_spend_limit("tenant-1", 0.0).await {
            SpendCheck::Ok => {} // 0 = unlimited
            other => panic!("expected Ok (unlimited), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_spend_limit_warning() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        // $3 input cost on a $3.50 limit = 85.7%
        tracker.record("tenant-1", "agent1", "claude-sonnet-4-6", 1_000_000, 0).await;
        match tracker.check_spend_limit("tenant-1", 3.50).await {
            SpendCheck::Warning { .. } => {} // expected
            other => panic!("expected Warning, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_reset_tenant_period() {
        let tracker = CostTracker::new(CostTracker::default_pricing());
        tracker.record("tenant-1", "agent1", "claude-sonnet-4-6", 1000, 500).await;
        tracker.reset_tenant_period("tenant-1").await;
        let usage = tracker.get_tenant_usage("tenant-1").await.unwrap();
        assert_eq!(usage.total_cost_usd, 0.0);
        assert_eq!(usage.total_requests, 0);
    }
}
CostTracker::default_pricing());
        t.record("tenant-1", "agent1", "claude-sonnet-4-6", 1000, 500).await;
        let u = t.get_usage("agent1").await.unwrap();
        assert_eq!(u.total_input_tokens, 1000);
        assert_eq!(u.total_requests, 1);
        assert!(u.total_cost_usd > 0.0);
    }

    #[tokio::test]
    async fn test_unknown_model_zero_cost() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("tenant-1", "agent1", "unknown-xyz", 1000, 500).await;
        let u = t.get_usage("agent1").await.unwrap();
        assert!((u.total_cost_usd - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_tenant_aggregation() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1000, 500).await;
        t.record("t1", "a2", "claude-sonnet-4-6", 2000, 1000).await;
        let u = t.get_tenant_usage("t1").await.unwrap();
        assert_eq!(u.total_input_tokens, 3000);
        assert_eq!(u.total_requests, 2);
    }

    #[tokio::test]
    async fn test_spend_limit_exceeded() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1_000_000, 1_000_000).await;
        assert!(matches!(t.check_spend_limit("t1", 5.0).await, SpendCheck::Exceeded { .. }));
    }

    #[tokio::test]
    async fn test_spend_limit_unlimited() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1_000_000, 1_000_000).await;
        assert!(matches!(t.check_spend_limit("t1", 0.0).await, SpendCheck::Ok));
    }

    #[tokio::test]
    async fn test_spend_limit_warning() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1_000_000, 0).await;
        assert!(matches!(t.check_spend_limit("t1", 3.50).await, SpendCheck::Warning { .. }));
    }

    #[tokio::test]
    async fn test_reset_period() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1000, 500).await;
        t.reset_tenant_period("t1").await;
        let u = t.get_tenant_usage("t1").await.unwrap();
        assert_eq!(u.total_cost_usd, 0.0);
    }
}
CostTracker::default_pricing());
        t.record("tenant-1", "agent1", "claude-sonnet-4-6", 1000, 500).await;
        let u = t.get_usage("agent1").await.unwrap();
        assert_eq!(u.total_requests, 1);
        assert!(u.total_cost_usd > 0.0);
    }

    #[tokio::test]
    async fn test_unknown_model_zero_cost() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "unknown-model", 1000, 500).await;
        let u = t.get_usage("a1").await.unwrap();
        assert!((u.total_cost_usd).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_tenant_usage_aggregation() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("tenant-1", "agent1", "claude-sonnet-4-6", 1000, 500).await;
        t.record("tenant-1", "agent2", "claude-sonnet-4-6", 2000, 1000).await;
        let u = t.get_tenant_usage("tenant-1").await.unwrap();
        assert_eq!(u.total_input_tokens, 3000);
        assert_eq!(u.total_requests, 2);
    }

    #[tokio::test]
    async fn test_spend_limit_exceeded() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1_000_000, 1_000_000).await;
        assert!(matches!(t.check_spend_limit("t1", 5.0).await, SpendCheck::Exceeded { .. }));
    }

    #[tokio::test]
    async fn test_spend_limit_unlimited() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1_000_000, 1_000_000).await;
        assert!(matches!(t.check_spend_limit("t1", 0.0).await, SpendCheck::Ok));
    }

    #[tokio::test]
    async fn test_spend_limit_warning() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1_000_000, 0).await;
        assert!(matches!(t.check_spend_limit("t1", 3.50).await, SpendCheck::Warning { .. }));
    }

    #[tokio::test]
    async fn test_reset_tenant_period() {
        let t = CostTracker::new(CostTracker::default_pricing());
        t.record("t1", "a1", "claude-sonnet-4-6", 1000, 500).await;
        t.reset_tenant_period("t1").await;
        let u = t.get_tenant_usage("t1").await.unwrap();
        assert_eq!(u.total_cost_usd, 0.0);
        assert_eq!(u.total_requests, 0);
    }
}
