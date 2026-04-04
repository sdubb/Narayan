use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use dashmap::DashMap;

pub struct Metrics {
    pub steps_total: AtomicU64,
    pub steps_last_window: AtomicU64,
    pub agents_running: AtomicU64,
    pub goals_total: AtomicU64,
    pub llm_calls_total: AtomicU64,
    pub llm_cache_hits: AtomicU64,
    pub response_cache_hits: AtomicU64,   // LLM response cache hits
    pub response_cache_misses: AtomicU64, // LLM response cache misses
    pub audit_bridge_lags: AtomicU64,     // Number of times audit bridge lagged
    pub audit_events_dropped: AtomicU64,  // Total audit events lost to lag
    pub input_tokens_total: AtomicU64,
    pub output_tokens_total: AtomicU64,
    pub started_at: Instant,
    /// Per-tenant step counters — used for plan enforcement.
    /// Reset at the start of each billing month.
    tenant_steps: DashMap<String, AtomicU64>,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            steps_total: AtomicU64::new(0),
            steps_last_window: AtomicU64::new(0),
            agents_running: AtomicU64::new(0),
            goals_total: AtomicU64::new(0),
            llm_calls_total: AtomicU64::new(0),
            llm_cache_hits: AtomicU64::new(0),
            response_cache_hits: AtomicU64::new(0),
            response_cache_misses: AtomicU64::new(0),
            audit_bridge_lags: AtomicU64::new(0),
            audit_events_dropped: AtomicU64::new(0),
            input_tokens_total: AtomicU64::new(0),
            output_tokens_total: AtomicU64::new(0),
            started_at: Instant::now(),
            tenant_steps: DashMap::new(),
        }
    }

    pub fn step_completed(&self) {
        self.steps_total.fetch_add(1, Ordering::Relaxed);
        self.steps_last_window.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a step for a specific tenant — used for plan enforcement.
    pub fn step_completed_for_tenant(&self, tenant_id: &str) {
        self.step_completed();
        self.tenant_steps
            .entry(tenant_id.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn steps_this_month(&self, tenant_id: &str) -> u64 {
        self.tenant_steps.get(tenant_id).map(|c| c.load(Ordering::Relaxed)).unwrap_or(0)
    }

    pub fn reset_monthly_steps(&self) {
        self.tenant_steps.clear();
    }

    /// Load per-tenant step counts from the costs DB table for the current calendar month.
    /// Called on startup so restarts don't reset plan enforcement counters.
    pub async fn load_steps_from_db(&self, pool: &sqlx::PgPool) {
        use sqlx::Row;
        let rows = sqlx::query(
            r#"SELECT tenant_id, COUNT(*) AS steps
                 FROM costs
                WHERE period_start >= date_trunc('month', NOW())
                GROUP BY tenant_id"#,
        )
        .fetch_all(pool)
        .await;

        match rows {
            Ok(rows) => {
                for row in rows {
                    let tenant_id: String = row.get("tenant_id");
                    let steps: i64 = row.get("steps");
                    self.tenant_steps
                        .entry(tenant_id)
                        .or_insert_with(|| AtomicU64::new(0))
                        .fetch_add(steps as u64, Ordering::Relaxed);
                }
                tracing::info!("loaded per-tenant step counts from DB");
            }
            Err(e) => tracing::warn!(error = %e, "failed to load step counts from DB — starting at 0"),
        }
    }

    pub fn agent_started(&self) {
        self.agents_running.fetch_add(1, Ordering::Relaxed);
    }

    pub fn agent_finished(&self) {
        self.agents_running.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn goal_created(&self) {
        self.goals_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn llm_call(&self, input_tokens: u32, output_tokens: u32, cache_hit: bool) {
        self.llm_calls_total.fetch_add(1, Ordering::Relaxed);
        self.input_tokens_total.fetch_add(input_tokens as u64, Ordering::Relaxed);
        self.output_tokens_total.fetch_add(output_tokens as u64, Ordering::Relaxed);
        if cache_hit {
            self.llm_cache_hits.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record an LLM response cache hit.
    pub fn response_cache_hit(&self) {
        self.response_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an LLM response cache miss.
    pub fn response_cache_miss(&self) {
        self.response_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Get cache hit ratio (0.0 to 1.0) for observability.
    pub fn cache_hit_ratio(&self) -> f64 {
        let hits = self.response_cache_hits.load(Ordering::Relaxed) as f64;
        let misses = self.response_cache_misses.load(Ordering::Relaxed) as f64;
        let total = hits + misses;
        if total == 0.0 {
            0.0
        } else {
            hits / total
        }
    }

    /// Record an audit bridge lag event — called when events are dropped.
    pub fn audit_bridge_lag(&self, events_dropped: u64) {
        self.audit_bridge_lags.fetch_add(1, Ordering::Relaxed);
        self.audit_events_dropped.fetch_add(events_dropped, Ordering::Relaxed);
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            steps_total: self.steps_total.load(Ordering::Relaxed),
            agents_running: self.agents_running.load(Ordering::Relaxed),
            goals_total: self.goals_total.load(Ordering::Relaxed),
            llm_calls_total: self.llm_calls_total.load(Ordering::Relaxed),
            llm_cache_hits: self.llm_cache_hits.load(Ordering::Relaxed),
            input_tokens_total: self.input_tokens_total.load(Ordering::Relaxed),
            output_tokens_total: self.output_tokens_total.load(Ordering::Relaxed),
            uptime_secs: self.uptime_secs(),
        }
    }

    /// Background task that resets the per-second window counter every second.
    pub async fn run_window_reset(metrics: Arc<Metrics>) {
        let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(1));
        loop {
            ticker.tick().await;
            let window = metrics.steps_last_window.swap(0, Ordering::Relaxed);
            tracing::debug!(steps_per_sec = window, "metrics window");
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// A point-in-time snapshot of all metrics (serializable).
#[derive(Debug, serde::Serialize)]
pub struct MetricsSnapshot {
    pub steps_total: u64,
    pub agents_running: u64,
    pub goals_total: u64,
    pub llm_calls_total: u64,
    pub llm_cache_hits: u64,
    pub input_tokens_total: u64,
    pub output_tokens_total: u64,
    pub uptime_secs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_snapshot_reflects_counter_updates() {
        let metrics = Metrics::new();
        metrics.step_completed();
        metrics.step_completed();
        metrics.agent_started();
        metrics.goal_created();
        metrics.llm_call(120, 45, true);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.steps_total, 2);
        assert_eq!(snapshot.agents_running, 1);
        assert_eq!(snapshot.goals_total, 1);
        assert_eq!(snapshot.llm_calls_total, 1);
        assert_eq!(snapshot.llm_cache_hits, 1);
        assert_eq!(snapshot.input_tokens_total, 120);
        assert_eq!(snapshot.output_tokens_total, 45);
    }

    #[test]
    fn test_agent_finished_decrements_running_count() {
        let metrics = Metrics::new();
        metrics.agent_started();
        metrics.agent_started();
        metrics.agent_finished();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.agents_running, 1);
    }

    #[test]
    fn test_llm_call_without_cache_hit_leaves_cache_counter_unchanged() {
        let metrics = Metrics::new();
        metrics.llm_call(10, 20, false);
        metrics.llm_call(5, 7, false);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.llm_calls_total, 2);
        assert_eq!(snapshot.llm_cache_hits, 0);
        assert_eq!(snapshot.input_tokens_total, 15);
        assert_eq!(snapshot.output_tokens_total, 27);
    }
}
