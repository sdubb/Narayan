use chrono::Utc;

use crate::state::AgentState;

pub struct CognitiveControlLoop {
    max_steps: usize,
    timeout_secs: i64,
}

impl CognitiveControlLoop {
    pub fn new(max_steps: usize, timeout_secs: u64) -> Self {
        Self { max_steps, timeout_secs: timeout_secs as i64 }
    }

    /// Returns false when the agent has exceeded either the step limit or the
    /// wall-clock timeout measured from `state.started_at`.
    ///
    /// Previously this used `Instant::now()` created inside `run_step()`,
    /// which reset on every wakeup — the timeout was never enforced across
    /// the full agent lifetime.  Now we measure from the persisted
    /// `started_at` timestamp so the check survives restarts.
    pub fn should_continue(&self, state: &AgentState) -> bool {
        if state.current_step as usize >= self.max_steps {
            tracing::warn!(
                agent_id   = %state.id,
                step       = state.current_step,
                max_steps  = self.max_steps,
                "cognitive control: step limit reached"
            );
            return false;
        }

        if let Some(started) = state.started_at {
            let elapsed = Utc::now().signed_duration_since(started).num_seconds();
            if elapsed >= self.timeout_secs {
                tracing::warn!(
                    agent_id      = %state.id,
                    elapsed_secs  = elapsed,
                    timeout_secs  = self.timeout_secs,
                    "cognitive control: wall-clock timeout reached"
                );
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AgentState;

    fn make_state() -> AgentState {
        AgentState::new("agent-1".into(), "tenant-1".into(), "test goal".into(), "/tmp/ws".into())
    }

    #[test]
    fn test_should_continue_within_limits() {
        let cl = CognitiveControlLoop::new(10, 300);
        let state = make_state();
        assert!(cl.should_continue(&state));
    }

    #[test]
    fn test_should_continue_exceeds_steps() {
        let cl = CognitiveControlLoop::new(10, 300);
        let mut state = make_state();
        state.current_step = 10;
        assert!(!cl.should_continue(&state));
    }

    #[test]
    fn test_should_continue_expired_started_at() {
        let cl = CognitiveControlLoop::new(50, 1); // 1-second timeout
        let mut state = make_state();
        state.started_at = Some(Utc::now() - chrono::Duration::seconds(10));
        assert!(!cl.should_continue(&state));
    }

    #[test]
    fn test_should_continue_no_started_at_skips_time_check() {
        let cl = CognitiveControlLoop::new(50, 1);
        let state = make_state(); // started_at = None
        assert!(cl.should_continue(&state));
    }
}
