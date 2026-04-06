use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    Extractor,
    Router,
    Drafter,
    Critic,
    Validator,
    Recovery,
    FailureClassifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmExecutionIntent {
    Strict,
    Balanced,
    Creative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmBudgetTier {
    Lean,
    Standard,
    High,
}

impl LlmBudgetTier {
    pub fn default_max_tokens(&self) -> u32 {
        match self {
            Self::Lean => 128,
            Self::Standard => 512,
            Self::High => 2048,
        }
    }

    pub fn default_temperature(&self) -> f32 {
        match self {
            Self::Lean => 0.0,
            Self::Standard => 0.2,
            Self::High => 0.7,
        }
    }

    pub fn task_complexity_label(&self) -> &'static str {
        match self {
            Self::Lean => "simple",
            Self::Standard => "medium",
            Self::High => "complex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmGenerationConfig {
    pub role: LlmRole,
    pub execution_intent: LlmExecutionIntent,
    pub budget_tier: LlmBudgetTier,
    pub max_tokens: u32,
    pub temperature: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cadence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,
}

impl LlmGenerationConfig {
    pub fn new(role: LlmRole, execution_intent: LlmExecutionIntent, budget_tier: LlmBudgetTier) -> Self {
        Self {
            max_tokens: budget_tier.default_max_tokens(),
            temperature: budget_tier.default_temperature(),
            role,
            execution_intent,
            budget_tier,
            cost_budget_usd: None,
            cadence: None,
            response_format: None,
        }
    }

    pub fn with_limits(mut self, max_tokens: u32, temperature: f32) -> Self {
        self.max_tokens = max_tokens;
        self.temperature = temperature;
        self
    }

    pub fn with_response_format(mut self, response_format: serde_json::Value) -> Self {
        self.response_format = Some(response_format);
        self
    }

    pub fn with_json_object_response(self) -> Self {
        self.with_response_format(serde_json::json!({
            "type": "json_object"
        }))
    }

    pub fn with_json_schema_response(
        mut self,
        name: impl Into<String>,
        schema: serde_json::Value,
    ) -> Self {
        self.response_format = Some(serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": name.into(),
                "strict": true,
                "schema": schema,
            }
        }));
        self
    }
}
