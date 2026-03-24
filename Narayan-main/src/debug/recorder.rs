use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub step_index: usize,
    pub action: String,
    pub result: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentRecorder {
    pub steps: Vec<StepRecord>,
}

impl AgentRecorder {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn record(&mut self, step: usize, action: String, result: String) {
        self.steps.push(StepRecord { step_index: step, action, result, timestamp: chrono::Utc::now().to_rfc3339() });
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}
