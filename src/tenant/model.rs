use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub username: String,
    pub name: String,
    pub email: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub status: TenantStatus,
    pub plan: TenantPlan,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    Active,
    Suspended,
    Deleted,
}

/// Billing tier. Everyone gets all connectors + full compliance stack.
/// The only differentiator is scale: steps/month and concurrent agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantPlan {
    Free,
    Go,
    Pro,
    Enterprise,
}

impl TenantPlan {
    /// Max agent steps this tenant can run per calendar month.
    /// Checked in AgentLoop before each step. u64::MAX = unlimited.
    pub fn max_steps_per_month(&self) -> u64 {
        match self {
            TenantPlan::Free => 1_000,
            TenantPlan::Go => 20_000,
            TenantPlan::Pro => 150_000,
            TenantPlan::Enterprise => u64::MAX,
        }
    }

    /// Max concurrent agents allowed.
    pub fn max_agents(&self) -> usize {
        match self {
            TenantPlan::Free => 3,
            TenantPlan::Go => 20,
            TenantPlan::Pro => 200,
            TenantPlan::Enterprise => usize::MAX,
        }
    }

    /// Max Narayan API req/s (not LLM req/s — tenant controls that via their own key).
    pub fn requests_per_sec(&self) -> f64 {
        match self {
            TenantPlan::Free => 2.0,
            TenantPlan::Go => 10.0,
            TenantPlan::Pro => 50.0,
            TenantPlan::Enterprise => 500.0,
        }
    }

    /// All plans get all connectors and full compliance.
    pub fn has_all_connectors(&self) -> bool {
        true
    }
    pub fn has_compliance_stack(&self) -> bool {
        true
    }

    /// spend_limit_usd is now INFORMATIONAL ONLY (tenant's own LLM spend, not Narayan's revenue).
    /// Kept for the dashboard display in GET /costs.
    pub fn spend_limit_usd(&self) -> f64 {
        match self {
            TenantPlan::Free => 50.0,
            TenantPlan::Go => 500.0,
            TenantPlan::Pro => 5_000.0,
            TenantPlan::Enterprise => 0.0,
        }
    }

    pub fn monthly_price_usd(&self) -> f64 {
        match self {
            TenantPlan::Free => 0.0,
            TenantPlan::Go => 15.0,
            TenantPlan::Pro => 79.0,
            TenantPlan::Enterprise => 0.0,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TenantPlan::Free => "free",
            TenantPlan::Go => "go",
            TenantPlan::Pro => "pro",
            TenantPlan::Enterprise => "enterprise",
        }
    }
}

impl std::fmt::Display for TenantPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TenantPlan {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "free" => Ok(TenantPlan::Free),
            "go" => Ok(TenantPlan::Go),
            "pro" => Ok(TenantPlan::Pro),
            "enterprise" => Ok(TenantPlan::Enterprise),
            other => Err(format!("unknown plan: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticatedTenant {
    pub tenant_id: String,
    pub plan: TenantPlan,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_roundtrip() {
        assert_eq!("go".parse::<TenantPlan>().unwrap(), TenantPlan::Go);
    }

    #[test]
    fn everyone_gets_connectors() {
        for plan in [TenantPlan::Free, TenantPlan::Go, TenantPlan::Pro] {
            assert!(plan.has_all_connectors());
            assert!(plan.has_compliance_stack());
        }
    }

    #[test]
    fn step_budgets() {
        assert_eq!(TenantPlan::Free.max_steps_per_month(), 1_000);
        assert_eq!(TenantPlan::Go.max_steps_per_month(), 20_000);
        assert_eq!(TenantPlan::Pro.max_steps_per_month(), 150_000);
    }
}
