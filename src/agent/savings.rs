//! Work savings estimator — calculates human hours and cost saved per agent run.
//!
//! Called once when a GoalInstance transitions to Completed. Writes
//! `human_hours_saved` and `human_cost_saved_usd` back to the instance.
//!
//! ## How the estimate is calculated
//!
//! 1. `category` (from the AgentRole) maps to a market hourly rate for the
//!    equivalent human job (e.g. sales_revops → $48/hr).
//!
//! 2. `completion_criteria` on the role carries the collection size hint
//!    (e.g. "50 Salesforce leads"). The actions list tells us the work type.
//!
//! 3. `minutes_per_item` is looked up from action keywords. Conservative
//!    numbers — defensible, not inflated.
//!
//! 4. `human_hours = item_count × minutes_per_item / 60`
//!    `human_cost = human_hours × hourly_rate`
//!    `roi_multiple = human_cost / ai_cost` (shown in the UI)

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::{
    agent::definition::{AgentRole, CompletionCheck},
    state::{GoalInstance, GoalInstanceStatus},
    storage::PostgresStore,
};

// ── Hourly rates ──────────────────────────────────────────────────────────
// US market mid-level rates. Updated conservatively — these are used in
// customer-facing ROI numbers so we never want to overstate.

pub fn hourly_rate_for_category(category: &str) -> f64 {
    match category {
        "customer_support"    => 28.0,
        "sales_revops"        => 48.0,
        "finance_accounting"  => 58.0,
        "devops" | "it_ops_itsm" => 90.0,
        "hr_people_ops"       => 42.0,
        "legal_contract"      => 180.0,
        "research_analyst"    => 52.0,
        "software_engineer"   => 105.0,
        "marketing"           => 45.0,
        _                     => 35.0,   // general knowledge worker
    }
}

// ── Minutes per item by work type ────────────────────────────────────────
// "item" = one unit of the primary thing the role processes.
// Conservative: based on real benchmarks where available, estimates elsewhere.

fn minutes_per_item(actions: &[&str]) -> f64 {
    // Check action keywords in priority order
    let joined = actions.join(" ").to_lowercase();

    if joined.contains("contract") || joined.contains("legal") || joined.contains("nda") {
        return 45.0;   // contract review: ~45 min/contract
    }
    if joined.contains("research") && joined.contains("report") {
        return 90.0;   // research report: ~90 min
    }
    if joined.contains("code review") || joined.contains("pr review") {
        return 15.0;   // code review pass: ~15 min/PR
    }
    if joined.contains("invoice") || joined.contains("reconcil") {
        return 12.0;   // accounting item: ~12 min
    }
    if joined.contains("candidate") || joined.contains("screen") || joined.contains("recruit") {
        return 20.0;   // candidate screening: ~20 min
    }
    if joined.contains("ticket") || joined.contains("support") || joined.contains("respond") {
        return 6.0;    // support ticket response: ~6 min
    }
    if joined.contains("enrich") || joined.contains("prospect") || joined.contains("lead") {
        return 8.0;    // lead enrichment: ~8 min
    }
    if joined.contains("report") || joined.contains("summarise") || joined.contains("summarize") {
        return 25.0;   // report generation: ~25 min
    }
    if joined.contains("deploy") || joined.contains("monitor") || joined.contains("incident") {
        return 18.0;   // DevOps task: ~18 min
    }

    5.0  // default: general record/item processing
}

// ── Estimator ─────────────────────────────────────────────────────────────

pub struct WorkSavingsEstimator {
    store: Arc<PostgresStore>,
}

impl WorkSavingsEstimator {
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self { store }
    }

    /// Calculate and persist savings for a completed or partially complete GoalInstance.
    /// Idempotent — safe to call multiple times.
    pub async fn estimate_and_persist(
        &self,
        gi:   &mut GoalInstance,
        role: &AgentRole,
    ) -> Result<()> {
        use crate::state::GoalInstanceStatus;

        // Only estimate on completed or partially-complete runs
        let is_estimable = matches!(
            gi.status,
            GoalInstanceStatus::Completed | GoalInstanceStatus::PartiallyComplete
        );
        if !is_estimable { return Ok(()); }

        // Already estimated
        if gi.human_hours_saved > 0.0 { return Ok(()); }

        // ── Quality gate: check result is non-empty ───────────────────────
        // A run that completed but produced no output (0 items, empty workspace)
        // should not receive full savings credit.
        let quality_factor = self.quality_factor(gi);
        if quality_factor == 0.0 {
            tracing::info!(
                goal_instance_id = %gi.id,
                "savings skipped — run produced no output"
            );
            return Ok(());
        }

        let (raw_hours, raw_cost) = self.estimate(gi, role);

        // Pro-rate for partial completion
        let (hours, cost_usd) = if gi.status == GoalInstanceStatus::PartiallyComplete {
            let partial_fraction = self.partial_completion_fraction(gi, role);
            (round2(raw_hours * partial_fraction * quality_factor),
             round2(raw_cost  * partial_fraction * quality_factor))
        } else {
            (round2(raw_hours * quality_factor), round2(raw_cost * quality_factor))
        };

        gi.human_hours_saved    = hours;
        gi.human_cost_saved_usd = cost_usd;
        gi.updated_at           = Utc::now();

        self.store.update_goal_instance_savings(
            &gi.tenant_id, &gi.id, hours, cost_usd,
        ).await?;

        tracing::info!(
            goal_instance_id = %gi.id,
            human_hours      = %format!("{:.2}", hours),
            human_cost_usd   = %format!("{:.2}", cost_usd),
            ai_cost_usd      = %format!("{:.4}", gi.cost_usd),
            roi_multiple     = %if gi.cost_usd > 0.0 { format!("{:.0}x", cost_usd / gi.cost_usd) } else { "∞".into() },
            quality_factor   = %format!("{:.2}", quality_factor),
            "savings estimated"
        );

        Ok(())
    }

    /// 0.0 = no output at all (skip estimation), 1.0 = full output present.
    fn quality_factor(&self, gi: &GoalInstance) -> f64 {
        // Check gi.result for evidence of real output
        if let Some(result) = &gi.result {
            // Explicit count fields
            for key in &["processed", "count", "total", "items", "rows"] {
                if let Some(n) = result[key].as_u64() {
                    if n > 0 { return 1.0; }
                }
            }
            // Non-null, non-empty result object with any keys
            if result.is_object() && result.as_object().map(|m| !m.is_empty()).unwrap_or(false) {
                return 1.0;
            }
            if result.is_array() && result.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                return 1.0;
            }
            // Result exists but appears empty — give half credit (something ran)
            return 0.5;
        }
        // No result at all — no savings credit
        0.0
    }

    /// For partially complete runs, estimate what fraction of the work was done.
    fn partial_completion_fraction(&self, gi: &GoalInstance, role: &AgentRole) -> f64 {
        // Try to read items_processed from result
        let processed = gi.result.as_ref()
            .and_then(|r| r.get("processed").or_else(|| r.get("count")))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let expected = self.extract_item_count(gi, role);

        if expected > 0 && processed > 0 {
            (processed as f64 / expected as f64).min(1.0)
        } else {
            0.5 // conservative: give 50% credit if we can't measure
        }
    }

    /// Pure estimate — does not write to DB.
    pub fn estimate(&self, gi: &GoalInstance, role: &AgentRole) -> (f64, f64) {
        let category = role.purpose_category();
        let rate     = hourly_rate_for_category(&category);

        // Extract item count from completion criteria
        let item_count = self.extract_item_count(gi, role);

        // Get action keywords from role guidelines
        let action_keywords: Vec<&str> = role.execution_guidelines.rules.iter()
            .map(|r| r.text.as_str())
            .collect();
        let mins = minutes_per_item(&action_keywords);

        let human_hours = (item_count as f64 * mins) / 60.0;
        let human_cost  = human_hours * rate;

        // Floor: even a single run saves at least the setup time
        let min_hours = 0.1;  // 6 minutes minimum
        let hours = human_hours.max(min_hours);
        let cost  = human_cost.max(min_hours * rate);

        (round2(hours), round2(cost))
    }

    fn extract_item_count(&self, gi: &GoalInstance, role: &AgentRole) -> u64 {
        // 1. Check GoalInstance result for item counts
        if let Some(result) = &gi.result {
            for key in &["processed", "count", "total", "items", "records"] {
                if let Some(n) = result[key].as_u64() { return n.max(1); }
            }
        }

        // 2. Check completion_criteria for AllItemsProcessed hint
        for criterion in &role.execution_guidelines.completion_criteria {
            if let CompletionCheck::AllItemsProcessed { collection_hint } = &criterion.check {
                // Try to parse a number out of the collection hint
                // e.g. "50 Salesforce leads" → 50
                if let Some(n) = extract_number_from_str(collection_hint) {
                    return n;
                }
            }
            if let CompletionCheck::CountMatches { source, .. } = &criterion.check {
                if let Some(n) = extract_number_from_str(source) { return n; }
            }
        }

        // 3. Fallback: treat the whole run as one unit of work
        1
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn extract_number_from_str(s: &str) -> Option<u64> {
    s.split_whitespace()
        .find_map(|w| w.parse::<u64>().ok())
        .filter(|&n| n > 0)
}

// ── Tenant aggregate ──────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct TenantSavingsSummary {
    pub total_runs:             u64,
    pub total_human_hours:      f64,
    pub total_human_cost_usd:   f64,
    pub total_ai_cost_usd:      f64,
    pub roi_multiple:           f64,
    /// Breakdown by agent role
    pub by_role: Vec<RoleSavings>,
}

#[derive(Debug, serde::Serialize)]
pub struct RoleSavings {
    pub role_id:              String,
    pub role_name:            String,
    pub runs:                 u64,
    pub human_hours_saved:    f64,
    pub human_cost_saved_usd: f64,
    pub ai_cost_usd:          f64,
}

impl TenantSavingsSummary {
    pub fn roi_multiple(human: f64, ai: f64) -> f64 {
        if ai <= 0.0 { return 0.0; }
        round2(human / ai)
    }
}

// ── AgentRole helper ──────────────────────────────────────────────────────

// Extend AgentRole with a category accessor for savings estimation.
// Category is stored as a guideline rule added during intent extraction.
trait RoleCategoryAccessor {
    fn purpose_category(&self) -> String;
}

impl RoleCategoryAccessor for AgentRole {
    fn purpose_category(&self) -> String {
        // Try to find category from guidelines (stored during intent extraction)
        // or infer from purpose keywords
        let purpose = self.purpose.to_lowercase();
        if purpose.contains("lead") || purpose.contains("crm") || purpose.contains("salesforce") { return "sales_revops".into(); }
        if purpose.contains("ticket") || purpose.contains("support") || purpose.contains("customer") { return "customer_support".into(); }
        if purpose.contains("invoice") || purpose.contains("reconcil") || purpose.contains("accounting") { return "finance_accounting".into(); }
        if purpose.contains("deploy") || purpose.contains("infra") || purpose.contains("incident") { return "devops".into(); }
        if purpose.contains("contract") || purpose.contains("legal") || purpose.contains("nda") { return "legal_contract".into(); }
        if purpose.contains("candidate") || purpose.contains("hiring") || purpose.contains("onboard") { return "hr_people_ops".into(); }
        if purpose.contains("research") || purpose.contains("analys") { return "research_analyst".into(); }
        if purpose.contains("code") || purpose.contains("pull request") || purpose.contains("deploy") { return "software_engineer".into(); }
        "general".into()
    }
}
