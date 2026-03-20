use super::recorder::StepRecord;

/// Replay a recorded agent execution, printing each step to stdout.
pub fn replay(steps: &[StepRecord]) {
    tracing::info!("replaying {} recorded steps", steps.len());
    for step in steps {
        tracing::info!(
            step  = step.step_index,
            action = %step.action,
            result = %crate::util::truncate(&step.result, 100),
            ts     = %step.timestamp,
            "replay"
        );
    }
}
