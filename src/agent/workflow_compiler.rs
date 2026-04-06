use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::{
    agent::definition::{AgentRole, RetryPolicy},
    gateway::{LlmBudgetTier, LlmExecutionIntent, LlmGenerationConfig, LlmRole},
    tools::{toolregistry as semantic_registry, ToolRegistry},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveType {
    Number,
    String,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeSpec {
    Primitive { primitive: PrimitiveType },
    Array { items: Box<TypeSpec> },
    Object { fields: BTreeMap<String, TypeSpec> },
}

impl TypeSpec {
    pub fn number() -> Self {
        Self::Primitive { primitive: PrimitiveType::Number }
    }

    pub fn string() -> Self {
        Self::Primitive { primitive: PrimitiveType::String }
    }

    pub fn boolean() -> Self {
        Self::Primitive { primitive: PrimitiveType::Boolean }
    }

    pub fn array(items: TypeSpec) -> Self {
        Self::Array { items: Box::new(items) }
    }

    pub fn object(fields: impl Into<BTreeMap<String, TypeSpec>>) -> Self {
        Self::Object { fields: fields.into() }
    }

    pub fn to_json_schema(&self) -> serde_json::Value {
        match self {
            TypeSpec::Primitive { primitive } => serde_json::json!({
                "type": match primitive {
                    PrimitiveType::Number => "number",
                    PrimitiveType::String => "string",
                    PrimitiveType::Boolean => "boolean",
                }
            }),
            TypeSpec::Array { items } => serde_json::json!({
                "type": "array",
                "items": items.to_json_schema(),
            }),
            TypeSpec::Object { fields } => {
                let mut props = serde_json::Map::new();
                for (key, value) in fields {
                    props.insert(key.clone(), value.to_json_schema());
                }
                serde_json::json!({
                    "type": "object",
                    "properties": props,
                    "additionalProperties": false,
                    "required": fields.keys().cloned().collect::<Vec<_>>(),
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpressionFunctionSpec {
    #[serde(default)]
    pub input: Vec<TypeSpec>,
    pub output: TypeSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedExpression {
    #[serde(rename = "type")]
    pub type_spec: TypeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(default)]
    pub args: Vec<TypedExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Box<TypedExpression>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Box<TypedExpression>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl TypedExpression {
    pub fn boolean_value(value: bool) -> Self {
        Self {
            type_spec: TypeSpec::boolean(),
            op: None,
            function: None,
            args: Vec::new(),
            left: None,
            right: None,
            value: Some(serde_json::json!(value)),
            path: None,
        }
    }

    pub fn number_value(value: impl Into<serde_json::Value>) -> Self {
        Self {
            type_spec: TypeSpec::number(),
            op: None,
            function: None,
            args: Vec::new(),
            left: None,
            right: None,
            value: Some(value.into()),
            path: None,
        }
    }

    pub fn path(path: impl Into<String>, type_spec: TypeSpec) -> Self {
        Self {
            type_spec,
            op: None,
            function: None,
            args: Vec::new(),
            left: None,
            right: None,
            value: None,
            path: Some(path.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    Sequential,
    Parallel,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Sequential
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DslStepType {
    FetchRecords,
    Filter,
    Compute,
    Aggregate,
    DetectAnomaly,
    LlmWorker,
    Branch,
    Notify,
    StoreResult,
    /// Cross-company or cross-team ACP boundary step.
    /// Validated at compile time against a signed handshake contract.
    AcpBoundary,
}

pub const LLM_WORKER_TOOL_NAME: &str = "llm_worker";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftDslStep {
    pub id: String,
    #[serde(rename = "type")]
    pub dsl_type: DslStepType,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_hint: Option<serde_json::Value>,
    #[serde(default)]
    pub input_mapping: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_back_to: Option<String>,
    #[serde(default)]
    pub retry_policy: RetryPolicy,
    // ── Boundary fields (only set for AcpBoundary steps) ─────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake_version: Option<u32>,
    /// "requester" | "responder"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_role: Option<String>,
    /// "cross_enterprise" | "cross_team"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyClass {
    Pure,
    SafeRepeat,
    SideEffect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeBehavior {
    Reuse,
    Recompute,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Transient,
    Data,
    Structural,
    Policy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecompileMode {
    Fork,
    InPlace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffectPolicy {
    pub pure: String,
    pub safe_repeat: String,
    pub side_effect: String,
}

impl Default for SideEffectPolicy {
    fn default() -> Self {
        Self {
            pure: "recompute".into(),
            safe_repeat: "reuse_or_recompute".into(),
            side_effect: "block_or_confirm".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecompilePolicy {
    #[serde(default = "default_recompile_mode")]
    pub mode: RecompileMode,
    #[serde(default)]
    pub trigger_on: Vec<FailureKind>,
    #[serde(default)]
    pub ignore_on: Vec<FailureKind>,
    #[serde(default = "default_max_recompile_count")]
    pub max_recompile_count: u32,
    #[serde(default)]
    pub preserve_state: Vec<String>,
    #[serde(default)]
    pub side_effect_policy: SideEffectPolicy,
    #[serde(default = "default_registry_version_policy")]
    pub registry_version_policy: String,
    #[serde(default)]
    pub approval_required_for: Vec<String>,
    #[serde(default)]
    pub emit_diff: bool,
}

fn default_recompile_mode() -> RecompileMode {
    RecompileMode::Fork
}

fn default_max_recompile_count() -> u32 {
    3
}

fn default_registry_version_policy() -> String {
    "pin_compile_time".into()
}

impl Default for RecompilePolicy {
    fn default() -> Self {
        Self {
            mode: default_recompile_mode(),
            trigger_on: vec![FailureKind::Structural, FailureKind::Policy],
            ignore_on: vec![FailureKind::Transient, FailureKind::Data],
            max_recompile_count: default_max_recompile_count(),
            preserve_state: vec![
                "completed_step_outputs".into(),
                "failed_step_input_snapshot".into(),
                "resource_bindings".into(),
                "execution_snapshot".into(),
                "user_confirmations".into(),
            ],
            side_effect_policy: SideEffectPolicy::default(),
            registry_version_policy: default_registry_version_policy(),
            approval_required_for: vec!["permissions_change".into(), "side_effect_step_change".into()],
            emit_diff: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub on_retry: ResumeBehavior,
    pub on_resume: ResumeBehavior,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self { on_retry: ResumeBehavior::Recompute, on_resume: ResumeBehavior::Reuse }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataStrategy {
    pub mode: String,
    #[serde(default)]
    pub page_size: Option<u32>,
}

impl Default for DataStrategy {
    fn default() -> Self {
        Self { mode: "single".into(), page_size: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionConstraints {
    #[serde(default)]
    pub max_rows: Option<u32>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub cost_budget: Option<u64>,
}

impl Default for ExecutionConstraints {
    fn default() -> Self {
        Self { max_rows: None, timeout_ms: None, cost_budget: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterminismConfig {
    #[serde(default = "default_time_mode")]
    pub time: String,
    #[serde(default = "default_randomness_mode")]
    pub randomness: String,
    #[serde(default = "default_external_calls_mode")]
    pub external_calls: String,
}

fn default_time_mode() -> String {
    "frozen".into()
}

fn default_randomness_mode() -> String {
    "seeded".into()
}

fn default_external_calls_mode() -> String {
    "recorded".into()
}

impl Default for DeterminismConfig {
    fn default() -> Self {
        Self {
            time: default_time_mode(),
            randomness: default_randomness_mode(),
            external_calls: default_external_calls_mode(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default = "default_scheduler_strategy")]
    pub strategy: String,
    #[serde(default = "default_scheduler_lock_check")]
    pub lock_check: String,
    #[serde(default)]
    pub max_concurrency: u32,
}

fn default_scheduler_strategy() -> String {
    "topological".into()
}

fn default_scheduler_lock_check() -> String {
    "before_execution".into()
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { strategy: default_scheduler_strategy(), lock_check: default_scheduler_lock_check(), max_concurrency: 5 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBinding {
    pub id: String,
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub schema: BTreeMap<String, TypeSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DataSignature {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_rate_bps: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VariantMatchRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count_min: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count_max: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_rate_bps_min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_rate_bps_max: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkflowVariantOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_constraints: Option<ExecutionConstraints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_strategy: Option<DataStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<SchedulerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowVariant {
    pub id: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub match_rule: VariantMatchRule,
    #[serde(default)]
    pub overrides: WorkflowVariantOverrides,
}

impl WorkflowVariant {
    fn specificity_score(&self) -> usize {
        (self.match_rule.schema_hash.is_some() as usize)
            + (self.match_rule.connector_id.is_some() as usize)
            + (self.match_rule.row_count_min.is_some() as usize)
            + (self.match_rule.row_count_max.is_some() as usize)
            + (self.match_rule.missing_rate_bps_min.is_some() as usize)
            + (self.match_rule.missing_rate_bps_max.is_some() as usize)
            + self.match_rule.tags.len()
    }

    fn matches(&self, signature: &DataSignature) -> bool {
        if let Some(expected) = &self.match_rule.schema_hash {
            if signature.schema_hash.as_deref() != Some(expected.as_str()) {
                return false;
            }
        }
        if let Some(expected) = &self.match_rule.connector_id {
            if signature.connector_id.as_deref() != Some(expected.as_str()) {
                return false;
            }
        }
        if let Some(min) = self.match_rule.row_count_min {
            if signature.row_count.is_some_and(|count| count < min) {
                return false;
            }
        }
        if let Some(max) = self.match_rule.row_count_max {
            if signature.row_count.is_some_and(|count| count > max) {
                return false;
            }
        }
        if let Some(min) = self.match_rule.missing_rate_bps_min {
            if signature.missing_rate_bps.is_some_and(|count| count < min) {
                return false;
            }
        }
        if let Some(max) = self.match_rule.missing_rate_bps_max {
            if signature.missing_rate_bps.is_some_and(|count| count > max) {
                return false;
            }
        }
        if !self.match_rule.tags.is_empty()
            && !self.match_rule.tags.iter().all(|tag| signature.tags.iter().any(|candidate| candidate == tag))
        {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VariantFallbackMode {
    #[default]
    UseDefaultVariant,
    Recompile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowVariantPolicy {
    pub default_variant_id: String,
    #[serde(default)]
    pub fallback: VariantFallbackMode,
    #[serde(default)]
    pub variants: Vec<WorkflowVariant>,
}

impl WorkflowVariantPolicy {
    fn default_variant<'a>(&'a self) -> Option<&'a WorkflowVariant> {
        self.variants.iter().find(|variant| variant.id == self.default_variant_id)
    }

    pub fn select<'a>(
        &'a self,
        signature: &DataSignature,
        base_execution: &ExecutionMode,
        base_constraints: &ExecutionConstraints,
        base_data_strategy: &DataStrategy,
        base_scheduler: &SchedulerConfig,
    ) -> Option<WorkflowVariantSelection> {
        let mut best: Option<&WorkflowVariant> = None;
        for variant in &self.variants {
            if !variant.matches(signature) {
                continue;
            }
            match best {
                None => best = Some(variant),
                Some(current) => {
                    let current_rank = (current.priority, current.specificity_score());
                    let candidate_rank = (variant.priority, variant.specificity_score());
                    if candidate_rank > current_rank {
                        best = Some(variant);
                    }
                }
            }
        }

        let selected = if let Some(best) = best {
            best
        } else if matches!(self.fallback, VariantFallbackMode::UseDefaultVariant) {
            self.default_variant()?
        } else {
            return None;
        };

        Some(WorkflowVariantSelection::from_variant(
            signature,
            selected,
            base_execution,
            base_constraints,
            base_data_strategy,
            base_scheduler,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowVariantSelection {
    pub signature: DataSignature,
    pub variant_id: String,
    pub description: String,
    pub priority: u32,
    pub execution: ExecutionMode,
    pub execution_constraints: ExecutionConstraints,
    pub data_strategy: DataStrategy,
    pub scheduler: SchedulerConfig,
}

impl WorkflowVariantSelection {
    fn from_variant(
        signature: &DataSignature,
        variant: &WorkflowVariant,
        base_execution: &ExecutionMode,
        base_constraints: &ExecutionConstraints,
        base_data_strategy: &DataStrategy,
        base_scheduler: &SchedulerConfig,
    ) -> Self {
        Self {
            signature: signature.clone(),
            variant_id: variant.id.clone(),
            description: variant.description.clone(),
            priority: variant.priority,
            execution: variant.overrides.execution.clone().unwrap_or_else(|| base_execution.clone()),
            execution_constraints: variant
                .overrides
                .execution_constraints
                .clone()
                .unwrap_or_else(|| base_constraints.clone()),
            data_strategy: variant.overrides.data_strategy.clone().unwrap_or_else(|| base_data_strategy.clone()),
            scheduler: variant.overrides.scheduler.clone().unwrap_or_else(|| base_scheduler.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingRule {
    pub dsl_type: DslStepType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub tool: String,
    pub operation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSnapshot {
    pub workflow_hash: String,
    #[serde(default)]
    pub input_state: serde_json::Value,
    #[serde(default)]
    pub tool_versions: BTreeMap<String, String>,
    #[serde(default)]
    pub compiler_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledStep {
    pub id: String,
    pub dsl_type: DslStepType,
    pub tool: Option<String>,
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_back_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_role: Option<LlmRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_intent: Option<LlmExecutionIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_generation: Option<LlmGenerationConfig>,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub input_mapping: BTreeMap<String, String>,
    #[serde(default)]
    pub output_mapping: BTreeMap<String, String>,
    pub output_schema: serde_json::Value,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub retry_policy: RetryPolicy,
    #[serde(default)]
    pub execution_policy: ExecutionPolicy,
    #[serde(default)]
    pub idempotency: IdempotencyClass,
    #[serde(default)]
    pub locks: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<crate::agent::planner::StepCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    // ── Boundary contract (only for AcpBoundary steps) ────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handshake_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_endpoint: Option<String>,
}

impl Default for IdempotencyClass {
    fn default() -> Self {
        Self::Pure
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledWorkflow {
    pub workflow_id: String,
    pub version: String,
    #[serde(default)]
    pub workflow_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_workflow_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recompile_reason: Option<String>,
    pub dsl_version: String,
    pub binding_version: String,
    pub runtime_version: String,
    #[serde(default)]
    pub tool_registry_version: String,
    pub entry_step: String,
    #[serde(default)]
    pub execution: ExecutionMode,
    #[serde(default)]
    pub steps: Vec<CompiledStep>,
    #[serde(default)]
    pub state_schema: serde_json::Value,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceBinding>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub tool_capabilities: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub binding_rules: Vec<BindingRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_policy: Option<WorkflowVariantPolicy>,
    #[serde(default)]
    pub execution_constraints: ExecutionConstraints,
    #[serde(default)]
    pub data_strategy: DataStrategy,
    #[serde(default)]
    pub determinism: DeterminismConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub recompile_policy: RecompilePolicy,
    #[serde(default)]
    pub expression_functions: BTreeMap<String, ExpressionFunctionSpec>,
    #[serde(default)]
    pub permissions: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_snapshot: Option<ExecutionSnapshot>,
}

#[derive(Debug, Clone)]
pub struct CompilerCardRequest {
    pub card_type: String,
    pub required_fields: Vec<String>,
    pub binding_target: String,
    pub resume_token: String,
}

#[derive(Debug, Clone)]
pub enum CompilerResult {
    Ready(CompiledWorkflow),
    NeedsCard(CompilerCardRequest),
}

pub(crate) fn legacy_args_template_from_compiled_step(step: &CompiledStep) -> serde_json::Value {
    if step.input_mapping.is_empty() {
        return step.args.clone();
    }

    fn translate(value: &serde_json::Value, input_mapping: &BTreeMap<String, String>) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => {
                if let Some(alias) = s.strip_prefix("$input.") {
                    if let Some(source) = input_mapping.get(alias) {
                        return serde_json::Value::String(legacy_template_for_source_path(source));
                    }
                }
                serde_json::Value::String(s.clone())
            }
            serde_json::Value::Object(map) => {
                let translated: serde_json::Map<String, serde_json::Value> =
                    map.iter().map(|(k, v)| (k.clone(), translate(v, input_mapping))).collect();
                serde_json::Value::Object(translated)
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(|value| translate(value, input_mapping)).collect())
            }
            other => other.clone(),
        }
    }

    translate(&step.args, &step.input_mapping)
}

fn legacy_template_for_source_path(source: &str) -> String {
    let Some((step_ref, tail)) = source.split_once('.') else {
        return source.to_string();
    };
    let Some(step_num) = step_ref.strip_prefix("step_") else {
        return source.to_string();
    };
    let Ok(index) = step_num.parse::<usize>() else {
        return source.to_string();
    };
    let legacy_index = index.saturating_sub(1);
    format!("{{{{$.deps.step-{legacy_index}.output.{tail}}}}}")
}

#[derive(Debug, thiserror::Error)]
pub enum CompilerError {
    #[error("{0}")]
    Message(String),
}

pub struct WorkflowCompiler;

impl WorkflowCompiler {
    pub fn compile(
        role: &AgentRole,
        intent: &serde_json::Value,
        tools: &ToolRegistry,
    ) -> Result<CompilerResult, CompilerError> {
        let draft_steps = workflow_draft_steps(intent);
        if draft_steps.is_empty() {
            return Err(CompilerError::Message("workflow_dsl is empty; compiler needs at least one typed step".into()));
        }

        let mut resources = collect_resources(role, intent);
        let mut compiled_steps = Vec::new();
        let mut binding_rules = Vec::new();
        let mut tool_capabilities = BTreeMap::new();
        let mut expression_functions = BTreeMap::new();
        expression_functions.insert(
            "len".into(),
            ExpressionFunctionSpec { input: vec![TypeSpec::array(TypeSpec::string())], output: TypeSpec::number() },
        );
        expression_functions.insert(
            "count".into(),
            ExpressionFunctionSpec { input: vec![TypeSpec::array(TypeSpec::string())], output: TypeSpec::number() },
        );

        let workflow_id = format!("wf_{}", uuid::Uuid::new_v4());
        let mut previous_step_id: Option<String> = None;
        let mut previous_output_key: Option<String> = None;

        if needs_database(&draft_steps) && !resources.values().any(|resource| resource.resource_type == "database") {
            return Ok(CompilerResult::NeedsCard(CompilerCardRequest {
                card_type: "database".into(),
                required_fields: vec!["host".into(), "port".into(), "db_name".into()],
                binding_target: "database".into(),
                resume_token: "bind_database".into(),
            }));
        }
        if needs_api(&draft_steps) && !resources.values().any(|resource| resource.resource_type == "api") {
            return Ok(CompilerResult::NeedsCard(CompilerCardRequest {
                card_type: "api_auth".into(),
                required_fields: vec!["base_url".into(), "api_key".into()],
                binding_target: "api".into(),
                resume_token: "bind_api".into(),
            }));
        }
        if needs_mcp(&draft_steps) && !resources.values().any(|resource| resource.resource_type == "mcp_server") {
            return Ok(CompilerResult::NeedsCard(CompilerCardRequest {
                card_type: "mcp".into(),
                required_fields: vec!["server_url".into()],
                binding_target: "mcp".into(),
                resume_token: "bind_mcp".into(),
            }));
        }
        if needs_acp(&draft_steps) && !resources.values().any(|resource| resource.resource_type == "acp_peer") {
            return Ok(CompilerResult::NeedsCard(CompilerCardRequest {
                card_type: "acp".into(),
                required_fields: vec!["server_url".into(), "target_agent".into()],
                binding_target: "acp".into(),
                resume_token: "bind_acp".into(),
            }));
        }

        for (index, draft_step) in draft_steps.iter().enumerate() {
            let dsl_type = draft_step.dsl_type.clone();
            let compiled = compile_step(
                index,
                draft_step,
                &resources,
                tools,
                role,
                previous_step_id.as_deref(),
                previous_output_key.as_deref(),
            )?;

            if let Some(tool) = compiled.tool.clone() {
                if let Some(tool_name) = tool.split(':').next() {
                    if let Some(tool_ref) = tools.get(tool_name) {
                        tool_capabilities.insert(
                            tool_ref.name().to_string(),
                            tool_ref.parameters_schema().into_iter().map(|p| p.name).collect(),
                        );
                    }
                }
            }

            binding_rules.push(BindingRule {
                dsl_type: dsl_type.clone(),
                source: compiled.resource.clone(),
                tool: compiled.tool.clone().unwrap_or_default(),
                operation: compiled.operation.clone().unwrap_or_default(),
            });

            if let Some(output_key) = compiled.output_mapping.keys().next().cloned() {
                previous_output_key = Some(output_key);
            }
            previous_step_id = Some(compiled.id.clone());
            compiled_steps.push(compiled);
        }

        let entry_step = compiled_steps.first().map(|step| step.id.clone()).unwrap_or_else(|| "step_1".into());
        let workflow_hash = workflow_hash(role, intent, &compiled_steps);
        let state_schema = build_state_schema(&compiled_steps);
        let permissions = resources
            .iter()
            .map(|(resource_id, resource)| (resource_id.clone(), resource.permissions.clone()))
            .collect::<BTreeMap<_, _>>();
        let base_execution = if compiled_steps.len() > 1 { ExecutionMode::Parallel } else { ExecutionMode::Sequential };
        let base_execution_constraints =
            ExecutionConstraints { max_rows: Some(10_000), timeout_ms: Some(5_000), cost_budget: Some(100) };
        let base_data_strategy = DataStrategy { mode: "paginate".into(), page_size: Some(1_000) };
        let base_scheduler = SchedulerConfig::default();
        let parent_workflow_version = role
            .execution_guidelines
            .compiled_workflow
            .as_ref()
            .map(|workflow| workflow.workflow_version.clone())
            .filter(|value| !value.trim().is_empty());
        let recompile_reason = intent
            .get("recompile_reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let variant_policy = build_variant_policy(&compiled_steps);
        let compiled_workflow = CompiledWorkflow {
            workflow_id,
            version: "v2".into(),
            workflow_version: "v2".into(),
            parent_workflow_version,
            recompile_reason,
            dsl_version: "v1".into(),
            binding_version: "v1".into(),
            runtime_version: "v1".into(),
            tool_registry_version: tool_registry_version(tools),
            entry_step,
            execution: base_execution.clone(),
            steps: compiled_steps,
            state_schema,
            resources: std::mem::take(&mut resources),
            metadata: serde_json::json!({
                "goal": role.purpose,
                "category": role.role_category.as_str(),
            }),
            tool_capabilities,
            binding_rules,
            variant_policy,
            execution_constraints: base_execution_constraints,
            data_strategy: base_data_strategy,
            determinism: DeterminismConfig::default(),
            scheduler: base_scheduler,
            recompile_policy: RecompilePolicy::default(),
            expression_functions,
            permissions,
            execution_snapshot: Some(ExecutionSnapshot {
                workflow_hash,
                input_state: intent.clone(),
                tool_versions: BTreeMap::new(),
                compiler_version: env!("CARGO_PKG_VERSION").to_string(),
            }),
        };

        validate_compiled_workflow(&compiled_workflow)?;

        Ok(CompilerResult::Ready(compiled_workflow))
    }
}

fn typed_draft_steps(intent: &serde_json::Value) -> Vec<DraftDslStep> {
    let Some(items) = intent.get("workflow_dsl").and_then(|value| value.as_array()) else {
        return Vec::new();
    };

    items
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let object = value.as_object()?;
            let description = object
                .get("description")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)?;
            let dsl_type = object
                .get("type")
                .and_then(|value| value.as_str())
                .and_then(parse_dsl_step_type)
                .unwrap_or_else(|| infer_dsl_type(&description));
            let id = object
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("step_{}", index + 1));
            let resource_hint = object
                .get("resource_hint")
                .or_else(|| object.get("resource"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let tool_hint = object
                .get("tool_hint")
                .or_else(|| object.get("tool"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let tool = object
                .get("tool")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let tool_operation = object
                .get("tool_operation")
                .or_else(|| object.get("operation"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let resource_id = object
                .get("resource_id")
                .or_else(|| object.get("resource"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let resource_type = object
                .get("resource_type")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let args_hint = object.get("args_hint").cloned().or_else(|| object.get("args").cloned());
            let input_mapping = object
                .get("input_mapping")
                .and_then(|value| value.as_object())
                .map(|map| {
                    map.iter()
                        .filter_map(|(key, value)| value.as_str().map(|source| (key.clone(), source.trim().to_string())))
                        .filter(|(_, value)| !value.is_empty())
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            let output_schema = object.get("output_schema").cloned();
            let read_only = object.get("read_only").and_then(|value| value.as_bool());
            let success_criteria = object
                .get("success_criteria")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str().map(|s| s.trim().to_string()))
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let depends_on = object
                .get("depends_on")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str().map(|s| s.trim().to_string()))
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let next_steps = object
                .get("next_steps")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str().map(|s| s.trim().to_string()))
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let branch_condition = object
                .get("branch_condition")
                .or_else(|| object.get("condition"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let repeat_until = object
                .get("repeat_until")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let fallback_step = object
                .get("fallback_step")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let loop_back_to = object
                .get("loop_back_to")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let retry_policy = object
                .get("retry_policy")
                .cloned()
                .and_then(|value| serde_json::from_value::<RetryPolicy>(value).ok())
                .unwrap_or_default();

            Some(DraftDslStep {
                id,
                dsl_type,
                description,
                resource_hint,
                tool_hint,
                tool,
                tool_operation,
                resource_id,
                resource_type,
                args_hint,
                input_mapping,
                output_schema,
                read_only,
                success_criteria,
                depends_on,
                next_steps,
                branch_condition,
                repeat_until,
                fallback_step,
                loop_back_to,
                retry_policy,
                handshake_id: None,
                handshake_version: None,
                boundary_role: None,
                boundary_scope: None,
                peer_endpoint: None,
            })
        })
        .collect()
}

fn workflow_draft_steps(intent: &serde_json::Value) -> Vec<DraftDslStep> {
    typed_draft_steps(intent)
}

fn parse_dsl_step_type(value: &str) -> Option<DslStepType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fetch_records" => Some(DslStepType::FetchRecords),
        "filter" => Some(DslStepType::Filter),
        "compute" => Some(DslStepType::Compute),
        "aggregate" => Some(DslStepType::Aggregate),
        "detect_anomaly" => Some(DslStepType::DetectAnomaly),
        "llm_worker" => Some(DslStepType::LlmWorker),
        "branch" => Some(DslStepType::Branch),
        "notify" => Some(DslStepType::Notify),
        "store_result" => Some(DslStepType::StoreResult),
        "acp_boundary" => Some(DslStepType::AcpBoundary),
        _ => None,
    }
}

fn build_variant_policy(compiled_steps: &[CompiledStep]) -> Option<WorkflowVariantPolicy> {
    if compiled_steps.is_empty() {
        return None;
    }

    Some(WorkflowVariantPolicy {
        default_variant_id: "base".into(),
        fallback: VariantFallbackMode::Recompile,
        variants: vec![
            WorkflowVariant {
                id: "high_volume".into(),
                priority: 100,
                description: "High-volume execution profile for large datasets".into(),
                match_rule: VariantMatchRule { row_count_min: Some(10_001), ..VariantMatchRule::default() },
                overrides: WorkflowVariantOverrides {
                    execution: Some(ExecutionMode::Parallel),
                    execution_constraints: Some(ExecutionConstraints {
                        max_rows: Some(50_000),
                        timeout_ms: Some(15_000),
                        cost_budget: Some(500),
                    }),
                    data_strategy: Some(DataStrategy { mode: "paginate".into(), page_size: Some(250) }),
                    scheduler: Some(SchedulerConfig {
                        strategy: default_scheduler_strategy(),
                        lock_check: default_scheduler_lock_check(),
                        max_concurrency: 3,
                    }),
                },
            },
            WorkflowVariant {
                id: "sparse_data".into(),
                priority: 90,
                description: "Sparse or incomplete data profile".into(),
                match_rule: VariantMatchRule { missing_rate_bps_min: Some(4_000), ..VariantMatchRule::default() },
                overrides: WorkflowVariantOverrides {
                    execution: Some(ExecutionMode::Sequential),
                    execution_constraints: Some(ExecutionConstraints {
                        max_rows: Some(5_000),
                        timeout_ms: Some(5_000),
                        cost_budget: Some(100),
                    }),
                    data_strategy: Some(DataStrategy { mode: "paginate".into(), page_size: Some(500) }),
                    scheduler: Some(SchedulerConfig {
                        strategy: default_scheduler_strategy(),
                        lock_check: default_scheduler_lock_check(),
                        max_concurrency: 2,
                    }),
                },
            },
            WorkflowVariant {
                id: "base".into(),
                priority: 0,
                description: "Default execution profile".into(),
                match_rule: VariantMatchRule {
                    row_count_max: Some(10_000),
                    missing_rate_bps_max: Some(2_500),
                    ..VariantMatchRule::default()
                },
                overrides: WorkflowVariantOverrides {
                    execution: Some(if compiled_steps.len() > 1 {
                        ExecutionMode::Parallel
                    } else {
                        ExecutionMode::Sequential
                    }),
                    execution_constraints: Some(ExecutionConstraints {
                        max_rows: Some(10_000),
                        timeout_ms: Some(5_000),
                        cost_budget: Some(100),
                    }),
                    data_strategy: Some(DataStrategy { mode: "paginate".into(), page_size: Some(1_000) }),
                    scheduler: Some(SchedulerConfig::default()),
                },
            },
        ],
    })
}

pub fn data_signature_from_value(value: &serde_json::Value) -> DataSignature {
    let schema_shape = shape_signature(value);
    let schema_hash = if schema_shape.trim().is_empty() {
        None
    } else {
        Some(format!("{:x}", sha2::Sha256::digest(schema_shape.as_bytes())))
    };

    DataSignature {
        schema_hash,
        connector_id: extract_string_field(
            value,
            &["connector_id", "connection", "resource_id", "resource", "db", "source"],
        ),
        row_count: extract_row_count(value),
        missing_rate_bps: extract_missing_rate_bps(value),
        tags: extract_tags(value),
    }
}

fn shape_signature(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(_) => "bool".into(),
        serde_json::Value::Number(_) => "number".into(),
        serde_json::Value::String(_) => "string".into(),
        serde_json::Value::Array(items) => {
            let mut item_shapes = items.iter().map(shape_signature).collect::<Vec<_>>();
            item_shapes.sort();
            item_shapes.dedup();
            format!("array[{}]", item_shapes.join(","))
        }
        serde_json::Value::Object(map) => {
            let mut parts =
                map.iter().map(|(key, value)| format!("{key}:{}", shape_signature(value))).collect::<Vec<_>>();
            parts.sort();
            format!("object{{{}}}", parts.join(","))
        }
    }
}

fn extract_string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(text) =
            object.get(*key).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        {
            return Some(text);
        }
    }
    for nested in object.values() {
        if let Some(text) = extract_string_field(nested, keys) {
            return Some(text);
        }
    }
    None
}

fn extract_row_count(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Array(items) => Some(items.len() as u64),
        serde_json::Value::Object(map) => {
            if let Some(count) = map.get("row_count").and_then(|v| v.as_u64()) {
                return Some(count);
            }
            if let Some(rows) = map.get("rows").and_then(|v| v.as_array()) {
                return Some(rows.len() as u64);
            }
            if let Some(records) = map.get("records").and_then(|v| v.as_array()) {
                return Some(records.len() as u64);
            }
            for nested in map.values() {
                if let Some(count) = extract_row_count(nested) {
                    return Some(count);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_missing_rate_bps(value: &serde_json::Value) -> Option<u32> {
    if let Some(number) = value
        .as_object()
        .and_then(|map| map.get("missing_rate"))
        .and_then(|v| v.as_f64())
        .or_else(|| value.as_object().and_then(|map| map.get("null_rate")).and_then(|v| v.as_f64()))
    {
        return Some(percent_to_bps(number));
    }

    if let serde_json::Value::Array(items) = value {
        let mut total = 0u64;
        let mut missing = 0u64;
        for item in items.iter().filter_map(|item| item.as_object()) {
            for nested_value in item.values() {
                total = total.saturating_add(1);
                if nested_value.is_null()
                    || nested_value.as_str().map(|s| s.trim().is_empty()).unwrap_or(false)
                    || nested_value.as_array().map(|items| items.is_empty()).unwrap_or(false)
                    || nested_value.as_object().map(|map| map.is_empty()).unwrap_or(false)
                {
                    missing = missing.saturating_add(1);
                }
            }
        }
        if total > 0 {
            let ratio = (missing as f64) / (total as f64);
            return Some(percent_to_bps(ratio * 100.0));
        }
    }

    None
}

fn percent_to_bps(percent: f64) -> u32 {
    let percent = if percent.is_finite() && percent <= 1.0 { percent * 100.0 } else { percent };
    let clamped = if percent.is_nan() { 0.0 } else { percent.clamp(0.0, 100.0) };
    (clamped * 100.0).round() as u32
}

fn extract_tags(value: &serde_json::Value) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(array) = value.as_object().and_then(|map| map.get("tags")).and_then(|v| v.as_array()) {
        for tag in
            array.iter().filter_map(|value| value.as_str()).map(|value| value.trim()).filter(|value| !value.is_empty())
        {
            tags.push(tag.to_string());
        }
    }

    if let Some(source) = value
        .as_object()
        .and_then(|map| map.get("source"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        tags.push(source.to_string());
    }

    tags.sort();
    tags.dedup();
    tags
}

fn needs_database(steps: &[DraftDslStep]) -> bool {
    steps.iter().any(|step| {
        let lower = step_signal(step);
        matches!(step.resource_type.as_deref(), Some("database"))
            || matches!(step.tool.as_deref(), Some("external_db" | "sql_query" | "vector_search" | "vector_store" | "vector_delete"))
            || matches!(step.tool_hint.as_deref(), Some("external_db" | "sql_query" | "vector_search" | "vector_store" | "vector_delete"))
            || lower.contains("database")
            || lower.contains("db")
            || lower.contains("sql")
            || lower.contains("table")
            || lower.contains("row")
            || lower.contains("record")
    })
}

fn needs_api(steps: &[DraftDslStep]) -> bool {
    steps.iter().any(|step| {
        let lower = step_signal(step);
        matches!(step.resource_type.as_deref(), Some("api" | "http_endpoint"))
            || matches!(step.tool.as_deref(), Some("http_request" | "api_call" | "external_api"))
            || matches!(step.tool_hint.as_deref(), Some("http_request" | "api_call" | "external_api"))
            || lower.contains("api")
            || lower.contains("http")
            || lower.contains("endpoint")
            || lower.contains("rest")
    })
}

fn needs_mcp(steps: &[DraftDslStep]) -> bool {
    steps.iter().any(|step| {
        let lower = step_signal(step);
        matches!(step.resource_type.as_deref(), Some("mcp" | "mcp_server"))
            || matches!(step.tool.as_deref(), Some("mcp_session" | "request_more_connectors"))
            || matches!(step.tool_hint.as_deref(), Some("mcp_session" | "request_more_connectors"))
            || lower.contains("mcp")
            || lower.contains("model context protocol")
            || lower.contains("json-rpc")
    })
}

fn needs_acp(steps: &[DraftDslStep]) -> bool {
    steps.iter().any(|step| {
        let lower = step_signal(step);
        matches!(step.resource_type.as_deref(), Some("acp" | "acp_peer" | "acp_server"))
            || matches!(step.tool.as_deref(), Some("acp_session"))
            || matches!(step.tool_hint.as_deref(), Some("acp_session"))
            || lower.contains("acp")
            || lower.contains("agent-to-agent")
            || lower.contains("internal agent")
            || lower.contains("peer")
    })
}

fn step_signal(step: &DraftDslStep) -> String {
    let mut parts = vec![step.description.clone()];
    if let Some(resource) = &step.resource_hint {
        parts.push(resource.clone());
    }
    if let Some(tool) = &step.tool_hint {
        parts.push(tool.clone());
    }
    if let Some(tool) = &step.tool {
        parts.push(tool.clone());
    }
    if let Some(operation) = &step.tool_operation {
        parts.push(operation.clone());
    }
    if let Some(resource) = &step.resource_id {
        parts.push(resource.clone());
    }
    if let Some(resource_type) = &step.resource_type {
        parts.push(resource_type.clone());
    }
    if let Some(args) = &step.args_hint {
        parts.push(args.to_string());
    }
    parts.join(" ").to_lowercase()
}

fn collect_resources(role: &AgentRole, intent: &serde_json::Value) -> BTreeMap<String, ResourceBinding> {
    let mut resources = BTreeMap::new();

    resources.insert(
        "workspace".into(),
        ResourceBinding {
            id: "workspace".into(),
            resource_type: "filesystem".into(),
            connector: None,
            permissions: vec!["read_only".into()],
            schema: BTreeMap::new(),
        },
    );

    for connector in &role.connectors {
        let resource_id = connector.clone();
        let resource_type = if connector.contains("db") {
            "database".to_string()
        } else if connector.contains("api") {
            "http_endpoint".to_string()
        } else if connector.contains("mcp") {
            "mcp_server".to_string()
        } else if connector.contains("acp") || connector.contains("agent") {
            "acp_peer".to_string()
        } else {
            "connector".to_string()
        };

        resources.insert(
            resource_id.clone(),
            ResourceBinding {
                id: resource_id,
                resource_type,
                connector: Some(connector.clone()),
                permissions: vec!["read_only".into()],
                schema: BTreeMap::new(),
            },
        );
    }

    if let Some(db) =
        intent.get("uses_external_db").and_then(|value| value.as_str()).filter(|value| !value.trim().is_empty())
    {
        resources.entry(db.to_string()).or_insert(ResourceBinding {
            id: db.to_string(),
            resource_type: "database".into(),
            connector: Some(db.to_string()),
            permissions: vec!["read_only".into()],
            schema: BTreeMap::new(),
        });
    }

    if let Some(api) =
        intent.get("uses_external_api").and_then(|value| value.as_str()).filter(|value| !value.trim().is_empty())
    {
        resources.entry(api.to_string()).or_insert(ResourceBinding {
            id: api.to_string(),
            resource_type: "http_endpoint".into(),
            connector: Some(api.to_string()),
            permissions: vec!["read_only".into()],
            schema: BTreeMap::new(),
        });
    }

    if let Some(acp) =
        intent.get("uses_acp_peer").and_then(|value| value.as_str()).filter(|value| !value.trim().is_empty())
    {
        resources.entry(acp.to_string()).or_insert(ResourceBinding {
            id: acp.to_string(),
            resource_type: "acp_peer".into(),
            connector: Some(acp.to_string()),
            permissions: vec!["read_only".into()],
            schema: BTreeMap::new(),
        });
    }

    resources
}

fn resolve_resource_binding(hint: &str, resources: &BTreeMap<String, ResourceBinding>, resource_type: &str) -> Option<String> {
    let normalized = hint.trim().to_lowercase();
    if normalized.is_empty() {
        return None;
    }

    if let Some(resource) = resources.get(hint).filter(|resource| resource.resource_type == resource_type) {
        return Some(resource.id.clone());
    }

    resources
        .values()
        .find(|resource| {
            resource.resource_type == resource_type
                && (resource.id.eq_ignore_ascii_case(hint)
                    || resource.connector.as_deref().map(|value| value.eq_ignore_ascii_case(hint)).unwrap_or(false))
        })
        .map(|resource| resource.id.clone())
}

fn infer_dsl_type(hint: &str) -> DslStepType {
    let lower = hint.to_lowercase();
    if lower.contains("branch") || lower.contains("if ") || lower.contains("otherwise") || lower.contains("when ") {
        return DslStepType::Branch;
    }
    if lower.contains("anomal") || lower.contains("outlier") || lower.contains("detect") {
        return DslStepType::DetectAnomaly;
    }
    if lower.contains("aggregate") || lower.contains("group") || lower.contains("sum") || lower.contains("count") {
        return DslStepType::Aggregate;
    }
    if lower.contains("notify") || lower.contains("alert") || lower.contains("send ") {
        return DslStepType::Notify;
    }
    if lower.contains("store") || lower.contains("save") || lower.contains("write") {
        return DslStepType::StoreResult;
    }
    if lower.contains("filter") || lower.contains("where ") || lower.contains("only ") {
        return DslStepType::Filter;
    }
    if lower.contains("compute") || lower.contains("calculate") || lower.contains("derive") {
        return DslStepType::Compute;
    }
    if lower.contains("summarize")
        || lower.contains("summary")
        || lower.contains("draft")
        || lower.contains("compose")
        || lower.contains("rewrite")
        || lower.contains("explain")
        || lower.contains("respond")
        || lower.contains("answer")
        || lower.contains("classify")
        || lower.contains("extract intent")
        || lower.contains("intent extraction")
        || lower.contains("reason")
        || lower.contains("generate text")
    {
        return DslStepType::LlmWorker;
    }
    DslStepType::FetchRecords
}

pub(crate) fn infer_llm_role(hint: &str) -> LlmRole {
    let lower = hint.to_lowercase();
    if lower.contains("extract") || lower.contains("parse") || lower.contains("intent") {
        LlmRole::Extractor
    } else if lower.contains("route") || lower.contains("router") || lower.contains("decide next") {
        LlmRole::Router
    } else if lower.contains("critic") || lower.contains("review") || lower.contains("score") {
        LlmRole::Critic
    } else if lower.contains("validate") || lower.contains("schema") || lower.contains("check output") {
        LlmRole::Validator
    } else if lower.contains("recover") || lower.contains("repair") || lower.contains("fix") {
        LlmRole::Recovery
    } else if lower.contains("failure") || lower.contains("error classify") || lower.contains("classify failure") {
        LlmRole::FailureClassifier
    } else {
        LlmRole::Drafter
    }
}

fn registry_step_type(dsl_type: &DslStepType) -> semantic_registry::DslStepType {
    match dsl_type {
        DslStepType::FetchRecords => semantic_registry::DslStepType::FetchRecords,
        DslStepType::Filter => semantic_registry::DslStepType::Filter,
        DslStepType::Compute => semantic_registry::DslStepType::Compute,
        DslStepType::Aggregate => semantic_registry::DslStepType::Aggregate,
        DslStepType::DetectAnomaly => semantic_registry::DslStepType::DetectAnomaly,
        DslStepType::LlmWorker => semantic_registry::DslStepType::Compute,
        DslStepType::Branch => semantic_registry::DslStepType::Branch,
        DslStepType::Notify => semantic_registry::DslStepType::Notify,
        DslStepType::StoreResult => semantic_registry::DslStepType::StoreResult,
        DslStepType::AcpBoundary => semantic_registry::DslStepType::Notify,
    }
}

fn registry_resource_kind(resource_type: Option<&str>) -> Option<semantic_registry::ResourceKind> {
    match resource_type.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) if value == "database" => Some(semantic_registry::ResourceKind::Database),
        Some(value) if value == "api" || value == "http_endpoint" => Some(semantic_registry::ResourceKind::HttpEndpoint),
        Some(value) if value == "connector" => Some(semantic_registry::ResourceKind::Connector),
        Some(value) if value == "acp" || value == "acp_peer" || value == "acp_server" => {
            Some(semantic_registry::ResourceKind::AcpPeer)
        }
        Some(value) if value == "filesystem" => Some(semantic_registry::ResourceKind::FileSystem),
        Some(value) if value == "api_key" => Some(semantic_registry::ResourceKind::ApiKey),
        Some(value) if value == "ssh_host" => Some(semantic_registry::ResourceKind::SshHost),
        Some(value) if value == "docker_daemon" => Some(semantic_registry::ResourceKind::DockerDaemon),
        Some(value) if value == "kube_cluster" => Some(semantic_registry::ResourceKind::KubeCluster),
        Some(value) if value == "mcp" || value == "mcp_server" => Some(semantic_registry::ResourceKind::McpServer),
        _ => None,
    }
}

fn semantic_resource_context(resources: &BTreeMap<String, ResourceBinding>) -> semantic_registry::ResourceContext {
    let bindings = resources
        .iter()
        .filter_map(|(id, resource)| {
            registry_resource_kind(Some(resource.resource_type.as_str())).map(|kind| (id.clone(), kind))
        })
        .collect::<HashMap<_, _>>();
    semantic_registry::ResourceContext { bindings }
}

fn default_operation_for_step(step: &DraftDslStep) -> Option<String> {
    if let Some(operation) = step.tool_operation.as_ref() {
        return Some(operation.clone());
    }
    if let Some(tool) = step.tool.as_deref().or(step.tool_hint.as_deref()) {
        let lower = tool.to_ascii_lowercase();
        let op = match lower.as_str() {
            "web_search_tool" => "search",
            "web_fetch" => "fetch",
            "browser_open" => "open",
            "browser_interact" => "extract",
            "browser_network" => "monitor",
            "http_request" => "get",
            "screenshot" => "screenshot",
            "external_db" => "query",
            "sql_query" => "select",
            "vector_search" => "search",
            "vector_store" => "store",
            "vector_delete" => "delete",
            "data_engine" => "transform_records",
            "data_extractor" => "extract",
            "spreadsheet_read" => "read",
            "spreadsheet_write" => "write",
            "pdf_read" => "read",
            "pdf_create" => "create",
            "image_process" => "resize",
            "email" => "send",
            "notification" => "send",
            "pushover" => "send",
            "send_message" => "send",
            "mcp_session" => "call_tool",
            "search_mcp_registry" => "search",
            "acp_session" => "send_message",
            "file_read" => "read",
            "file_write" => "write",
            "file_edit" => "replace",
            "glob_search" => "search",
            "content_search" => "search",
            "compress" => "compress",
            "memory_store" => "store",
            "memory_recall" => "recall",
            "memory_forget" => "forget",
            "memory_consolidate" => "consolidate",
            "code_run" => "run",
            "shell" => "exec",
            "git_operations" => "status",
            "diff" => "diff",
            "patch" => "apply",
            "docker" => "status",
            "kubernetes" => "get",
            "ssh_exec" => "exec",
            "crypto_tool" => "hash",
            _ => return None,
        };
        return Some(op.to_string());
    }

    Some(match step.dsl_type {
        DslStepType::FetchRecords => "fetch",
        DslStepType::Filter => "filter",
        DslStepType::Compute => "compute",
        DslStepType::Aggregate => "aggregate",
        DslStepType::DetectAnomaly => "detect_anomaly",
        DslStepType::LlmWorker => "reason",
        DslStepType::Notify => "send",
        DslStepType::StoreResult => "write",
        DslStepType::Branch => return None,
        DslStepType::AcpBoundary => return None,
    }
    .to_string())
}

fn step_constraints(step: &DraftDslStep) -> semantic_registry::StepConstraints {
    semantic_registry::StepConstraints {
        read_only: step.read_only.unwrap_or(false),
        requires_approval: false,
    }
}

fn explicit_branch_condition(step: &DraftDslStep) -> Option<String> {
    step.branch_condition.clone().or_else(|| step.repeat_until.clone())
}

fn bind_semantic_step(
    draft_step: &DraftDslStep,
    resources: &BTreeMap<String, ResourceBinding>,
) -> Result<semantic_registry::BoundTool, CompilerError> {
    let step_type = registry_step_type(&draft_step.dsl_type);
    let operation = default_operation_for_step(draft_step);
    let tool = draft_step.tool.clone().or_else(|| draft_step.tool_hint.clone());
    let resource_id = draft_step.resource_id.clone().or_else(|| {
        draft_step.resource_hint.as_ref().and_then(|hint| {
            match draft_step.resource_type.as_deref() {
                Some("database") => resolve_resource_binding(hint, resources, "database"),
                Some("api") | Some("http_endpoint") => resolve_resource_binding(hint, resources, "http_endpoint"),
                Some("mcp") | Some("mcp_server") => resolve_resource_binding(hint, resources, "mcp_server"),
                Some("acp") | Some("acp_peer") | Some("acp_server") => {
                    resolve_resource_binding(hint, resources, "acp_peer")
                }
                Some("connector") => resolve_resource_binding(hint, resources, "connector"),
                Some("filesystem") => resolve_resource_binding(hint, resources, "filesystem"),
                _ => None,
            }
        })
    });
    let semantic_step = semantic_registry::DslStep {
        id: draft_step.id.clone(),
        step_type,
        tool,
        operation,
        resource_id,
        resource_type: registry_resource_kind(draft_step.resource_type.as_deref()),
        constraints: step_constraints(draft_step),
    };
    let semantic_resources = semantic_resource_context(resources);

    semantic_registry::bind_step(&semantic_step, &semantic_resources)
        .map_err(|error| CompilerError::Message(format!("binding failed for step '{}': {error:?}", draft_step.id)))
}

pub(crate) fn infer_llm_execution_intent(role: &LlmRole, hint: &str) -> LlmExecutionIntent {
    let lower = hint.to_lowercase();
    match role {
        LlmRole::Extractor | LlmRole::Validator | LlmRole::FailureClassifier => LlmExecutionIntent::Strict,
        LlmRole::Router | LlmRole::Critic | LlmRole::Recovery => LlmExecutionIntent::Balanced,
        LlmRole::Drafter => {
            if lower.contains("draft")
                || lower.contains("compose")
                || lower.contains("write")
                || lower.contains("summarize")
            {
                LlmExecutionIntent::Creative
            } else {
                LlmExecutionIntent::Balanced
            }
        }
    }
}

pub(crate) fn infer_llm_budget_tier(role: &LlmRole, hint: &str) -> LlmBudgetTier {
    let lower = hint.to_lowercase();
    if ["every minute", "every hour", "hourly", "minutely", "recurring", "poll", "watch", "monitor", "heartbeat"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return LlmBudgetTier::Lean;
    }

    match role {
        LlmRole::Extractor | LlmRole::Validator | LlmRole::FailureClassifier => LlmBudgetTier::Lean,
        LlmRole::Router | LlmRole::Critic | LlmRole::Recovery => LlmBudgetTier::Standard,
        LlmRole::Drafter => {
            if lower.contains("draft")
                || lower.contains("compose")
                || lower.contains("summarize")
                || lower.contains("write")
            {
                LlmBudgetTier::High
            } else {
                LlmBudgetTier::Standard
            }
        }
    }
}

pub(crate) fn llm_generation_for_hint(hint: &str, role: &LlmRole) -> LlmGenerationConfig {
    let execution_intent = infer_llm_execution_intent(role, hint);
    let budget_tier = infer_llm_budget_tier(role, hint);
    let mut config = LlmGenerationConfig::new(role.clone(), execution_intent, budget_tier);
    config.cost_budget_usd = Some(match budget_tier {
        LlmBudgetTier::Lean => 0.001,
        LlmBudgetTier::Standard => 0.01,
        LlmBudgetTier::High => 0.05,
    });
    if matches!(budget_tier, LlmBudgetTier::Lean) {
        config.max_tokens = config.max_tokens.min(128);
        config.temperature = 0.0;
    }
    config.with_json_schema_response(format!("llm_{:?}_output", role).to_lowercase(), llm_output_schema(role))
}

pub(crate) fn llm_output_schema(role: &LlmRole) -> serde_json::Value {
    match role {
        LlmRole::Extractor => serde_json::json!({
            "type": "object",
            "properties": {
                "intent": {"type": "string"},
                "entities": {"type": "array", "items": {"type": "string"}},
                "confidence": {"type": "number"},
            },
            "required": ["intent", "confidence"]
        }),
        LlmRole::Router => serde_json::json!({
            "type": "object",
            "properties": {
                "next_step": {"type": "string"},
                "reason": {"type": "string"},
                "confidence": {"type": "number"},
            },
            "required": ["next_step", "reason"]
        }),
        LlmRole::Drafter => serde_json::json!({
            "type": "object",
            "properties": {
                "draft_text": {"type": "string"},
                "style": {"type": "string"},
                "confidence": {"type": "number"},
            },
            "required": ["draft_text"]
        }),
        LlmRole::Critic => serde_json::json!({
            "type": "object",
            "properties": {
                "issues": {"type": "array", "items": {"type": "string"}},
                "score": {"type": "number"},
                "should_retry": {"type": "boolean"},
            },
            "required": ["issues", "score", "should_retry"]
        }),
        LlmRole::Validator => serde_json::json!({
            "type": "object",
            "properties": {
                "is_valid": {"type": "boolean"},
                "errors": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["is_valid", "errors"]
        }),
        LlmRole::Recovery => serde_json::json!({
            "type": "object",
            "properties": {
                "fixed_output": {"type": "object"},
                "strategy_used": {"type": "string"},
            },
            "required": ["fixed_output", "strategy_used"]
        }),
        LlmRole::FailureClassifier => serde_json::json!({
            "type": "object",
            "properties": {
                "failure_type": {
                    "type": "string",
                    "enum": ["retryable", "repairable", "replan_required", "fatal"]
                },
                "reason": {"type": "string"},
            },
            "required": ["failure_type", "reason"]
        }),
    }
}

pub(crate) fn llm_output_mapping(role: &LlmRole, step_id: &str) -> BTreeMap<String, String> {
    match role {
        LlmRole::Extractor => BTreeMap::from([
            ("intent".into(), format!("{}.intent", step_id)),
            ("entities".into(), format!("{}.entities", step_id)),
            ("confidence".into(), format!("{}.confidence", step_id)),
        ]),
        LlmRole::Router => BTreeMap::from([
            ("next_step".into(), format!("{}.next_step", step_id)),
            ("reason".into(), format!("{}.reason", step_id)),
            ("confidence".into(), format!("{}.confidence", step_id)),
        ]),
        LlmRole::Drafter => BTreeMap::from([
            ("draft_text".into(), format!("{}.draft_text", step_id)),
            ("style".into(), format!("{}.style", step_id)),
            ("confidence".into(), format!("{}.confidence", step_id)),
        ]),
        LlmRole::Critic => BTreeMap::from([
            ("issues".into(), format!("{}.issues", step_id)),
            ("score".into(), format!("{}.score", step_id)),
            ("should_retry".into(), format!("{}.should_retry", step_id)),
        ]),
        LlmRole::Validator => BTreeMap::from([
            ("is_valid".into(), format!("{}.is_valid", step_id)),
            ("errors".into(), format!("{}.errors", step_id)),
        ]),
        LlmRole::Recovery => BTreeMap::from([
            ("fixed_output".into(), format!("{}.fixed_output", step_id)),
            ("strategy_used".into(), format!("{}.strategy_used", step_id)),
        ]),
        LlmRole::FailureClassifier => BTreeMap::from([
            ("failure_type".into(), format!("{}.failure_type", step_id)),
            ("reason".into(), format!("{}.reason", step_id)),
        ]),
    }
}

fn compile_step(
    index: usize,
    draft_step: &DraftDslStep,
    resources: &BTreeMap<String, ResourceBinding>,
    _tools: &ToolRegistry,
    role: &AgentRole,
    previous_step_id: Option<&str>,
    previous_output_key: Option<&str>,
) -> Result<CompiledStep, CompilerError> {
    let hint = draft_step.description.trim();
    let dsl_type = &draft_step.dsl_type;
    let step_id = if draft_step.id.trim().is_empty() { format!("step_{}", index + 1) } else { draft_step.id.clone() };
    let lower = hint.to_lowercase();
    let dependency = previous_step_id.map(str::to_string).into_iter().collect::<Vec<_>>();

    match dsl_type {
        DslStepType::AcpBoundary => {
            let boundary_role = draft_step.boundary_role.clone().unwrap_or_else(|| "requester".into());
            let operation = if boundary_role == "responder" { "receive" } else { "send" }.to_string();
            let output_schema = draft_step.output_schema.clone().unwrap_or_else(|| serde_json::json!({"type": "object"}));

            Ok(CompiledStep {
                id: step_id.clone(),
                dsl_type: dsl_type.clone(),
                tool: Some("acp_boundary".into()),
                operation: Some(operation),
                branch_condition: explicit_branch_condition(draft_step),
                repeat_until: draft_step.repeat_until.clone(),
                fallback_step: draft_step.fallback_step.clone(),
                loop_back_to: draft_step.loop_back_to.clone(),
                llm_role: None,
                execution_intent: None,
                llm_generation: None,
                args: draft_step.args_hint.clone().unwrap_or_else(|| serde_json::json!({})),
                input_mapping: if draft_step.input_mapping.is_empty() {
                    BTreeMap::new()
                } else {
                    draft_step.input_mapping.clone()
                },
                output_mapping: BTreeMap::new(),
                output_schema,
                success_criteria: if draft_step.success_criteria.is_empty() {
                    vec!["boundary handoff completed".into()]
                } else {
                    draft_step.success_criteria.clone()
                },
                retry_policy: draft_step.retry_policy.clone(),
                execution_policy: ExecutionPolicy::default(),
                idempotency: IdempotencyClass::SideEffect,
                locks: Vec::new(),
                depends_on: if draft_step.depends_on.is_empty() { dependency } else { draft_step.depends_on.clone() },
                next_steps: draft_step.next_steps.clone(),
                condition: None,
                resource: draft_step.resource_id.clone().or_else(|| draft_step.resource_hint.clone()),
                handshake_id: draft_step.handshake_id.clone(),
                handshake_version: draft_step.handshake_version,
                boundary_role: draft_step.boundary_role.clone(),
                boundary_scope: draft_step.boundary_scope.clone(),
                peer_endpoint: draft_step.peer_endpoint.clone(),
            })
        }
        DslStepType::FetchRecords => {
            let bound = bind_semantic_step(draft_step, resources)?;
            let tool = bound.tool_name.to_string();
            let operation = bound.operation;
            let resource = draft_step.resource_id.clone().or_else(|| {
                draft_step
                    .resource_hint
                    .as_ref()
                    .and_then(|hint| resolve_resource_binding(hint, resources, "database"))
            });
            let mut args = draft_step.args_hint.clone().unwrap_or_else(|| match tool.as_str() {
                "external_db" => serde_json::json!({
                    "db": resource.clone().unwrap_or_else(|| "database".into()),
                    "operation": operation,
                    "sql": infer_sql_query(&lower),
                    "max_rows": role.execution_limits.max_steps.min(1000),
                }),
                "sql_query" => serde_json::json!({
                    "query": infer_sql_query(&lower),
                    "connection_key": resource.clone().unwrap_or_else(|| "database".into()),
                    "max_rows": 500,
                    "timeout_secs": 30,
                }),
                "web_search_tool" => serde_json::json!({
                    "query": hint,
                    "limit": 10,
                }),
                "web_fetch" | "browser_open" => serde_json::json!({
                    "url": resource.clone().unwrap_or_else(|| hint.to_string()),
                }),
                "browser_interact" => serde_json::json!({
                    "url": resource.clone().unwrap_or_else(|| hint.to_string()),
                    "action": operation.clone(),
                }),
                "data_extractor" => serde_json::json!({
                    "source": hint,
                }),
                "pdf_read" => serde_json::json!({
                    "path": resource.clone().unwrap_or_else(|| hint.to_string()),
                }),
                "file_read" => serde_json::json!({
                    "path": resource.clone().unwrap_or_else(|| hint.to_string()),
                }),
                "glob_search" | "content_search" => serde_json::json!({
                    "pattern": hint,
                }),
                "vector_search" | "memory_recall" => serde_json::json!({
                    "query": hint,
                }),
                "http_request" => serde_json::json!({
                    "method": operation.clone(),
                    "url": hint,
                }),
                _ => serde_json::json!({
                    "query": hint,
                }),
            });
            if let Some(object) = args.as_object_mut() {
                if let Some(resource_id) = resource.as_ref() {
                    object.entry("resource").or_insert_with(|| serde_json::Value::String(resource_id.clone()));
                }
            }
            let output_schema = draft_step.output_schema.clone().unwrap_or_else(|| {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "records": {"type": "array", "items": {"type": "object"}},
                        "meta": {"type": "object"},
                    }
                })
            });

            Ok(CompiledStep {
                id: step_id.clone(),
                dsl_type: dsl_type.clone(),
                tool: Some(tool),
                operation: Some(operation),
                branch_condition: explicit_branch_condition(draft_step),
                repeat_until: draft_step.repeat_until.clone(),
                fallback_step: draft_step.fallback_step.clone(),
                loop_back_to: draft_step.loop_back_to.clone(),
                llm_role: None,
                execution_intent: None,
                llm_generation: None,
                args,
                input_mapping: if draft_step.input_mapping.is_empty() {
                    BTreeMap::new()
                } else {
                    draft_step.input_mapping.clone()
                },
                output_mapping: BTreeMap::from([("records".into(), format!("{}.records", step_id))]),
                output_schema,
                success_criteria: if draft_step.success_criteria.is_empty() {
                    vec!["records returned".into()]
                } else {
                    draft_step.success_criteria.clone()
                },
                retry_policy: draft_step.retry_policy.clone(),
                execution_policy: ExecutionPolicy::default(),
                idempotency: IdempotencyClass::SafeRepeat,
                locks: resource.clone().into_iter().collect(),
                depends_on: if draft_step.depends_on.is_empty() { dependency } else { draft_step.depends_on.clone() },
                next_steps: draft_step.next_steps.clone(),
                condition: draft_step
                    .branch_condition
                    .as_ref()
                    .map(|_| expression_from_hint(hint, &step_id, "records")),
                resource,
                handshake_id: draft_step.handshake_id.clone(),
                handshake_version: draft_step.handshake_version,
                boundary_role: draft_step.boundary_role.clone(),
                boundary_scope: draft_step.boundary_scope.clone(),
                peer_endpoint: draft_step.peer_endpoint.clone(),
            })
        }
        DslStepType::Filter | DslStepType::Compute | DslStepType::Aggregate | DslStepType::DetectAnomaly => {
            let bound = bind_semantic_step(draft_step, resources)?;
            let tool = bound.tool_name.to_string();
            let operation = bound.operation;
            let source_ref = previous_step_id.unwrap_or("step_1").to_string();
            let source_output_key = previous_output_key.unwrap_or("records");
            let input_mapping = if draft_step.input_mapping.is_empty() {
                BTreeMap::from([("records".into(), format!("{source_ref}.{source_output_key}"))])
            } else {
                draft_step.input_mapping.clone()
            };
            let condition_spec = pipeline_condition_from_hint(hint);
            let (pipeline_op, condition, output_key) = match dsl_type {
                DslStepType::Filter => ("filter", Some(condition_spec.clone()), "records"),
                DslStepType::Compute => ("compute", None, "records"),
                DslStepType::Aggregate => ("aggregate", None, "records"),
                DslStepType::DetectAnomaly => ("detect_anomaly", Some(condition_spec), "anomalies"),
                _ => ("filter", None, "records"),
            };
            let output_schema = draft_step.output_schema.clone().unwrap_or_else(|| {
                let mut properties = serde_json::Map::new();
                properties.insert(
                    output_key.to_string(),
                    serde_json::json!({"type": "array", "items": {"type": "object"}}),
                );
                properties.insert("meta".into(), serde_json::json!({"type": "object"}));
                serde_json::json!({
                    "type": "object",
                    "properties": properties,
                })
            });

            Ok(CompiledStep {
                id: step_id.clone(),
                dsl_type: dsl_type.clone(),
                tool: Some(tool),
                operation: Some(operation),
                branch_condition: explicit_branch_condition(draft_step),
                repeat_until: draft_step.repeat_until.clone(),
                fallback_step: draft_step.fallback_step.clone(),
                loop_back_to: draft_step.loop_back_to.clone(),
                llm_role: None,
                execution_intent: None,
                llm_generation: None,
                args: serde_json::json!({
                    "records": "$input.records",
                    "pipeline": [{
                        "op": pipeline_op,
                        "condition": condition.clone().unwrap_or_else(|| serde_json::json!({"field": "id", "exists": true})),
                    }],
                }),
                input_mapping,
                output_mapping: BTreeMap::from([(output_key.into(), format!("{}.{}", step_id, output_key))]),
                output_schema,
                success_criteria: if draft_step.success_criteria.is_empty() {
                    vec!["pipeline completed".into()]
                } else {
                    draft_step.success_criteria.clone()
                },
                retry_policy: draft_step.retry_policy.clone(),
                execution_policy: ExecutionPolicy::default(),
                idempotency: IdempotencyClass::Pure,
                locks: Vec::new(),
                depends_on: if draft_step.depends_on.is_empty() { dependency } else { draft_step.depends_on.clone() },
                next_steps: draft_step.next_steps.clone(),
                condition: match dsl_type {
                    DslStepType::Filter => Some(expression_from_hint(hint, &source_ref, source_output_key)),
                    DslStepType::DetectAnomaly => Some(expression_from_hint(hint, &source_ref, source_output_key)),
                    _ => None,
                },
                resource: None,
                handshake_id: draft_step.handshake_id.clone(),
                handshake_version: draft_step.handshake_version,
                boundary_role: draft_step.boundary_role.clone(),
                boundary_scope: draft_step.boundary_scope.clone(),
                peer_endpoint: draft_step.peer_endpoint.clone(),
            })
        }
        DslStepType::LlmWorker => {
            let instruction = hint.trim().to_string();
            let llm_role = infer_llm_role(hint);
            let execution_intent = infer_llm_execution_intent(&llm_role, hint);
            let llm_generation = llm_generation_for_hint(hint, &llm_role);
            let mut input_mapping = BTreeMap::new();
            if let (Some(prev_step), Some(prev_key)) = (previous_step_id, previous_output_key) {
                input_mapping.insert("context".into(), format!("{}.{}", prev_step, prev_key));
            }

            let mut args = serde_json::json!({
                "instruction": instruction,
                "temperature": llm_generation.temperature,
                "max_tokens": llm_generation.max_tokens,
                "execution_intent": llm_generation.execution_intent,
                "budget_tier": llm_generation.budget_tier,
                "response_format": "json",
            });
            if !input_mapping.is_empty() {
                if let Some(map) = args.as_object_mut() {
                    map.insert("context".into(), serde_json::Value::String("$input.context".into()));
                }
            }

            Ok(CompiledStep {
                id: step_id.clone(),
                dsl_type: dsl_type.clone(),
                tool: Some(LLM_WORKER_TOOL_NAME.into()),
                operation: Some("reason".into()),
                branch_condition: explicit_branch_condition(draft_step),
                repeat_until: draft_step.repeat_until.clone(),
                fallback_step: draft_step.fallback_step.clone(),
                loop_back_to: draft_step.loop_back_to.clone(),
                args: draft_step.args_hint.clone().unwrap_or(args),
                input_mapping,
                output_mapping: llm_output_mapping(&llm_role, &step_id),
                output_schema: llm_output_schema(&llm_role),
                success_criteria: if draft_step.success_criteria.is_empty() {
                    vec!["llm reasoning completed".into()]
                } else {
                    draft_step.success_criteria.clone()
                },
                retry_policy: draft_step.retry_policy.clone(),
                execution_policy: ExecutionPolicy::default(),
                idempotency: IdempotencyClass::SafeRepeat,
                locks: Vec::new(),
                depends_on: if draft_step.depends_on.is_empty() { dependency } else { draft_step.depends_on.clone() },
                next_steps: draft_step.next_steps.clone(),
                condition: None,
                resource: None,
                llm_role: Some(llm_role),
                execution_intent: Some(execution_intent),
                llm_generation: Some(llm_generation),
                handshake_id: draft_step.handshake_id.clone(),
                handshake_version: draft_step.handshake_version,
                boundary_role: draft_step.boundary_role.clone(),
                boundary_scope: draft_step.boundary_scope.clone(),
                peer_endpoint: draft_step.peer_endpoint.clone(),
            })
        }
        DslStepType::Branch => {
            let source_ref = previous_step_id.unwrap_or("step_1").to_string();
            let source_output_key = previous_output_key.unwrap_or("records");
            Ok(CompiledStep {
                id: step_id.clone(),
                dsl_type: dsl_type.clone(),
                tool: None,
                operation: None,
                branch_condition: explicit_branch_condition(draft_step),
                repeat_until: draft_step.repeat_until.clone(),
                fallback_step: draft_step.fallback_step.clone(),
                loop_back_to: draft_step.loop_back_to.clone(),
                llm_role: None,
                execution_intent: None,
                llm_generation: None,
                args: serde_json::json!({}),
                input_mapping: BTreeMap::new(),
                output_mapping: BTreeMap::new(),
                output_schema: serde_json::json!({"type": "object"}),
                success_criteria: if draft_step.success_criteria.is_empty() {
                    vec!["branch evaluated".into()]
                } else {
                    draft_step.success_criteria.clone()
                },
                retry_policy: draft_step.retry_policy.clone(),
                execution_policy: ExecutionPolicy { on_retry: ResumeBehavior::Block, on_resume: ResumeBehavior::Block },
                idempotency: IdempotencyClass::Pure,
                locks: Vec::new(),
                depends_on: if draft_step.depends_on.is_empty() { dependency } else { draft_step.depends_on.clone() },
                next_steps: draft_step.next_steps.clone(),
                condition: Some(expression_from_hint(hint, &source_ref, source_output_key)),
                resource: None,
                handshake_id: draft_step.handshake_id.clone(),
                handshake_version: draft_step.handshake_version,
                boundary_role: draft_step.boundary_role.clone(),
                boundary_scope: draft_step.boundary_scope.clone(),
                peer_endpoint: draft_step.peer_endpoint.clone(),
            })
        }
        DslStepType::Notify => {
            let bound = bind_semantic_step(draft_step, resources)?;
            let tool = bound.tool_name.to_string();
            let operation = bound.operation;

            Ok(CompiledStep {
                id: step_id.clone(),
                dsl_type: dsl_type.clone(),
                tool: Some(tool),
                operation: Some(operation),
                branch_condition: explicit_branch_condition(draft_step),
                repeat_until: draft_step.repeat_until.clone(),
                fallback_step: draft_step.fallback_step.clone(),
                loop_back_to: draft_step.loop_back_to.clone(),
                llm_role: None,
                execution_intent: None,
                llm_generation: None,
                args: draft_step.args_hint.clone().unwrap_or_else(|| serde_json::json!({
                    "message": hint,
                })),
                input_mapping: if draft_step.input_mapping.is_empty() {
                    BTreeMap::new()
                } else {
                    draft_step.input_mapping.clone()
                },
                output_mapping: BTreeMap::from([("notification".into(), format!("{}.notification", step_id))]),
                output_schema: draft_step.output_schema.clone().unwrap_or_else(|| serde_json::json!({
                    "type": "object",
                    "properties": {"status": {"type": "string"}}
                })),
                success_criteria: if draft_step.success_criteria.is_empty() {
                    vec!["notification sent".into()]
                } else {
                    draft_step.success_criteria.clone()
                },
                retry_policy: draft_step.retry_policy.clone(),
                execution_policy: ExecutionPolicy { on_retry: ResumeBehavior::Block, on_resume: ResumeBehavior::Block },
                idempotency: IdempotencyClass::SideEffect,
                locks: Vec::new(),
                depends_on: if draft_step.depends_on.is_empty() { dependency } else { draft_step.depends_on.clone() },
                next_steps: draft_step.next_steps.clone(),
                condition: None,
                resource: None,
                handshake_id: draft_step.handshake_id.clone(),
                handshake_version: draft_step.handshake_version,
                boundary_role: draft_step.boundary_role.clone(),
                boundary_scope: draft_step.boundary_scope.clone(),
                peer_endpoint: draft_step.peer_endpoint.clone(),
            })
        }
        DslStepType::StoreResult => {
            let bound = bind_semantic_step(draft_step, resources)?;
            let tool = bound.tool_name.to_string();
            let operation = bound.operation;

            Ok(CompiledStep {
                id: step_id.clone(),
                dsl_type: dsl_type.clone(),
                tool: Some(tool),
                operation: Some(operation),
                branch_condition: explicit_branch_condition(draft_step),
                repeat_until: draft_step.repeat_until.clone(),
                fallback_step: draft_step.fallback_step.clone(),
                loop_back_to: draft_step.loop_back_to.clone(),
                llm_role: None,
                execution_intent: None,
                llm_generation: None,
                args: draft_step.args_hint.clone().unwrap_or_else(|| serde_json::json!({
                    "content": "$input.content",
                    "path": "workspace/results.json",
                })),
                input_mapping: if draft_step.input_mapping.is_empty() {
                    BTreeMap::from([(
                        "content".into(),
                        format!("{}.{}", previous_step_id.unwrap_or("step_1"), previous_output_key.unwrap_or("records")),
                    )])
                } else {
                    draft_step.input_mapping.clone()
                },
                output_mapping: BTreeMap::from([("path".into(), format!("{}.path", step_id))]),
                output_schema: draft_step.output_schema.clone().unwrap_or_else(|| serde_json::json!({"type": "object"})),
                success_criteria: if draft_step.success_criteria.is_empty() {
                    vec!["result stored".into()]
                } else {
                    draft_step.success_criteria.clone()
                },
                retry_policy: draft_step.retry_policy.clone(),
                execution_policy: ExecutionPolicy { on_retry: ResumeBehavior::Block, on_resume: ResumeBehavior::Block },
                idempotency: IdempotencyClass::SideEffect,
                locks: Vec::new(),
                depends_on: if draft_step.depends_on.is_empty() { dependency } else { draft_step.depends_on.clone() },
                next_steps: draft_step.next_steps.clone(),
                condition: None,
                resource: None,
                handshake_id: draft_step.handshake_id.clone(),
                handshake_version: draft_step.handshake_version,
                boundary_role: draft_step.boundary_role.clone(),
                boundary_scope: draft_step.boundary_scope.clone(),
                peer_endpoint: draft_step.peer_endpoint.clone(),
            })
        }
    }
}

fn infer_sql_query(hint: &str) -> String {
    if hint.contains("count") {
        "SELECT COUNT(*) AS count FROM users".into()
    } else if hint.contains("users") {
        "SELECT * FROM users".into()
    } else {
        "SELECT * FROM records LIMIT 100".into()
    }
}

fn pipeline_condition_from_hint(hint: &str) -> serde_json::Value {
    let number = Regex::new(r"(?i)\b(?:>=|gt|greater than or equal to)\s*(\d+)\b|\b(?:>|greater than)\s*(\d+)\b")
        .ok()
        .and_then(|re| re.captures(hint))
        .and_then(|caps| caps.get(1).or_else(|| caps.get(2)))
        .and_then(|value| value.as_str().parse::<i64>().ok())
        .unwrap_or(5);

    let field = Regex::new(r"(?i)\b([a-z_][a-z0-9_]*)\s*(?:>|gt|greater than)\s*\d+\b")
        .ok()
        .and_then(|re| re.captures(hint))
        .and_then(|caps| caps.get(1))
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| "failed_logins".into());

    serde_json::json!({
        "field": field,
        "gt": number,
    })
}

fn expression_from_hint(
    _hint: &str,
    source_ref: &str,
    source_output_key: &str,
) -> crate::agent::planner::StepCondition {
    let records_path = TypedExpression::path(
        format!("{source_ref}.{source_output_key}"),
        TypeSpec::array(TypeSpec::object(BTreeMap::<String, TypeSpec>::new())),
    );
    let len_expr = TypedExpression {
        type_spec: TypeSpec::number(),
        op: None,
        function: Some("len".into()),
        args: vec![records_path],
        left: None,
        right: None,
        value: None,
        path: None,
    };
    crate::agent::planner::StepCondition::Expression(TypedExpression {
        type_spec: TypeSpec::boolean(),
        op: Some("gt".into()),
        function: None,
        args: Vec::new(),
        left: Some(Box::new(len_expr)),
        right: Some(Box::new(TypedExpression::number_value(serde_json::json!(0)))),
        value: None,
        path: None,
    })
}

fn workflow_hash(role: &AgentRole, intent: &serde_json::Value, steps: &[CompiledStep]) -> String {
    let payload = serde_json::json!({
        "role": role.name,
        "purpose": role.purpose,
        "intent": intent,
        "steps": steps,
    });
    let digest = sha2::Sha256::digest(serde_json::to_vec(&payload).unwrap_or_default());
    format!("wf_{}", hex::encode(digest))
}

fn build_state_schema(steps: &[CompiledStep]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let required = steps.iter().map(|step| step.id.clone()).collect::<Vec<_>>();
    for step in steps {
        properties.insert(step.id.clone(), step.output_schema.clone());
    }

    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn tool_registry_version(tools: &ToolRegistry) -> String {
    let mut names = tools.list().into_iter().map(|name| name.to_string()).collect::<Vec<_>>();
    names.sort_unstable();
    let payload = serde_json::json!({
        "tools": names,
    });
    let digest = sha2::Sha256::digest(serde_json::to_vec(&payload).unwrap_or_default());
    format!("registry_{}", hex::encode(digest))
}

fn validate_compiled_workflow(workflow: &CompiledWorkflow) -> Result<(), CompilerError> {
    if workflow.workflow_id.trim().is_empty() {
        return Err(CompilerError::Message("workflow_id cannot be empty".into()));
    }
    if workflow.workflow_version.trim().is_empty() {
        return Err(CompilerError::Message("workflow_version cannot be empty".into()));
    }
    if workflow.version.trim().is_empty() {
        return Err(CompilerError::Message("version cannot be empty".into()));
    }
    if workflow.version != workflow.workflow_version {
        return Err(CompilerError::Message("version and workflow_version must match".into()));
    }
    if workflow.dsl_version.trim().is_empty()
        || workflow.binding_version.trim().is_empty()
        || workflow.runtime_version.trim().is_empty()
    {
        return Err(CompilerError::Message("workflow version fields cannot be empty".into()));
    }
    if workflow.tool_registry_version.trim().is_empty() {
        return Err(CompilerError::Message("tool_registry_version cannot be empty".into()));
    }
    if workflow.steps.is_empty() {
        return Err(CompilerError::Message("compiled workflow must contain at least one step".into()));
    }
    if workflow.recompile_policy.max_recompile_count == 0 {
        return Err(CompilerError::Message("recompile_policy.max_recompile_count must be greater than zero".into()));
    }
    if !matches!(workflow.recompile_policy.mode, RecompileMode::Fork | RecompileMode::InPlace) {
        return Err(CompilerError::Message("recompile_policy.mode is invalid".into()));
    }

    if let Some(policy) = &workflow.variant_policy {
        if policy.default_variant_id.trim().is_empty() {
            return Err(CompilerError::Message("variant_policy.default_variant_id cannot be empty".into()));
        }
        if policy.variants.is_empty() {
            return Err(CompilerError::Message("variant_policy must define at least one variant".into()));
        }
        let mut variant_ids = BTreeMap::<String, ()>::new();
        for variant in &policy.variants {
            if variant.id.trim().is_empty() {
                return Err(CompilerError::Message("variant_policy contains an empty variant id".into()));
            }
            if variant_ids.insert(variant.id.clone(), ()).is_some() {
                return Err(CompilerError::Message(format!("duplicate workflow variant id '{}'", variant.id)));
            }
        }
        if !variant_ids.contains_key(&policy.default_variant_id) {
            return Err(CompilerError::Message(format!(
                "variant_policy.default_variant_id '{}' does not match any variant",
                policy.default_variant_id
            )));
        }
    }

    let mut step_ids = BTreeMap::<String, ()>::new();
    for step in &workflow.steps {
        if step.id.trim().is_empty() {
            return Err(CompilerError::Message("compiled step id cannot be empty".into()));
        }
        if step_ids.insert(step.id.clone(), ()).is_some() {
            return Err(CompilerError::Message(format!("duplicate compiled step id '{}'", step.id)));
        }
        validate_compiled_step(step, workflow)?;
    }

    if !step_ids.contains_key(&workflow.entry_step) {
        return Err(CompilerError::Message(format!(
            "entry_step '{}' does not match any compiled step",
            workflow.entry_step
        )));
    }

    let Some(state_schema_object) = workflow.state_schema.as_object() else {
        return Err(CompilerError::Message(format!(
            "state_schema must be a JSON object schema, got {}",
            workflow.state_schema
        )));
    };

    let Some(required) = state_schema_object.get("required").and_then(|value| value.as_array()) else {
        return Err(CompilerError::Message("state_schema must declare required step buckets".into()));
    };
    let required_ids = required.iter().filter_map(|value| value.as_str()).collect::<Vec<_>>();
    for step in &workflow.steps {
        if !required_ids.contains(&step.id.as_str()) {
            return Err(CompilerError::Message(format!(
                "state_schema is missing required entry for step '{}'",
                step.id
            )));
        }
    }

    for step in &workflow.steps {
        for dep in &step.depends_on {
            if !step_ids.contains_key(dep) {
                return Err(CompilerError::Message(format!("step '{}' depends on unknown step '{}'", step.id, dep)));
            }
        }
        for next in &step.next_steps {
            if !step_ids.contains_key(next) {
                return Err(CompilerError::Message(format!(
                    "step '{}' references unknown next step '{}'",
                    step.id, next
                )));
            }
        }
        if let Some(fallback_step) = &step.fallback_step {
            if !step_ids.contains_key(fallback_step) {
                return Err(CompilerError::Message(format!(
                    "step '{}' references unknown fallback step '{}'",
                    step.id, fallback_step
                )));
            }
        }
        if let Some(loop_back_to) = &step.loop_back_to {
            if !step_ids.contains_key(loop_back_to) {
                return Err(CompilerError::Message(format!(
                    "step '{}' references unknown loop_back_to step '{}'",
                    step.id, loop_back_to
                )));
            }
        }
    }

    Ok(())
}

fn validate_compiled_step(step: &CompiledStep, workflow: &CompiledWorkflow) -> Result<(), CompilerError> {
    if step.dsl_type != DslStepType::Branch {
        if step.tool.as_ref().map(|value| value.trim().is_empty()).unwrap_or(true) {
            return Err(CompilerError::Message(format!("step '{}' must bind to a tool", step.id)));
        }
        if step.operation.as_ref().map(|value| value.trim().is_empty()).unwrap_or(true) {
            return Err(CompilerError::Message(format!("step '{}' must bind to an operation", step.id)));
        }
    }

    if step.tool.as_deref() == Some(LLM_WORKER_TOOL_NAME) {
        if step.llm_role.is_none() {
            return Err(CompilerError::Message(format!("llm_worker step '{}' must define llm_role", step.id)));
        }
        if step.execution_intent.is_none() {
            return Err(CompilerError::Message(format!("llm_worker step '{}' must define execution_intent", step.id)));
        }
        if step.llm_generation.is_none() {
            return Err(CompilerError::Message(format!("llm_worker step '{}' must define llm_generation", step.id)));
        }
        if step.operation.as_deref() != Some("reason") {
            return Err(CompilerError::Message(format!("llm_worker step '{}' must use operation 'reason'", step.id)));
        }
    }

    if contains_placeholder_value(&step.args) {
        return Err(CompilerError::Message(format!("step '{}' contains unresolved placeholder values", step.id)));
    }

    for (input_name, source_path) in &step.input_mapping {
        if input_name.trim().is_empty() || source_path.trim().is_empty() {
            return Err(CompilerError::Message(format!("step '{}' contains empty input mapping", step.id)));
        }
        let source_lower = source_path.to_ascii_lowercase();
        if source_path.contains("{{") || source_lower.contains("tbd") || source_lower.contains("placeholder") {
            return Err(CompilerError::Message(format!("step '{}' contains unresolved input mapping", step.id)));
        }
    }

    for (output_name, target_path) in &step.output_mapping {
        if output_name.trim().is_empty() || target_path.trim().is_empty() {
            return Err(CompilerError::Message(format!("step '{}' contains empty output mapping", step.id)));
        }
        let target_lower = target_path.to_ascii_lowercase();
        if target_path.contains("{{") || target_lower.contains("tbd") || target_lower.contains("placeholder") {
            return Err(CompilerError::Message(format!("step '{}' contains unresolved output mapping", step.id)));
        }
    }

    if !matches!(step.dsl_type, DslStepType::Branch) && step.output_mapping.is_empty() {
        return Err(CompilerError::Message(format!("step '{}' must define output_mapping", step.id)));
    }

    if let Some(condition) = &step.condition {
        validate_typed_expression(condition, &workflow.expression_functions)?;
    }

    if let Some(resource_id) = &step.resource {
        if !workflow.resources.contains_key(resource_id) {
            return Err(CompilerError::Message(format!(
                "step '{}' references unknown resource '{}'",
                step.id, resource_id
            )));
        }
    }

    Ok(())
}

fn validate_typed_expression(
    expr: &crate::agent::planner::StepCondition,
    registry: &BTreeMap<String, ExpressionFunctionSpec>,
) -> Result<(), CompilerError> {
    match expr {
        crate::agent::planner::StepCondition::Deterministic(_) => Ok(()),
        crate::agent::planner::StepCondition::Expression(expr) => validate_typed_expression_node(expr, registry),
    }
}

fn validate_typed_expression_node(
    expr: &TypedExpression,
    registry: &BTreeMap<String, ExpressionFunctionSpec>,
) -> Result<(), CompilerError> {
    if let Some(function) = &expr.function {
        let spec = registry.get(function).ok_or_else(|| {
            CompilerError::Message(format!("typed expression uses unsupported function '{}'", function))
        })?;
        if spec.input.len() != expr.args.len() {
            return Err(CompilerError::Message(format!(
                "typed expression function '{}' expects {} args, got {}",
                function,
                spec.input.len(),
                expr.args.len()
            )));
        }
        for arg in &expr.args {
            validate_typed_expression_node(arg, registry)?;
        }
        if matches!(function.as_str(), "len" | "count") {
            if !type_spec_compatible(&TypeSpec::number(), &expr.type_spec) {
                return Err(CompilerError::Message(format!(
                    "typed expression function '{}' must return a numeric type",
                    function
                )));
            }
        } else if !type_spec_compatible(&spec.output, &expr.type_spec) {
            return Err(CompilerError::Message(format!(
                "typed expression function '{}' output type does not match declared type",
                function
            )));
        }
    }

    if let Some(op) = expr.op.as_deref() {
        let left = expr
            .left
            .as_deref()
            .ok_or_else(|| CompilerError::Message(format!("typed operator '{}' requires a left operand", op)))?;
        validate_typed_expression_node(left, registry)?;

        match op {
            "gt" | "gte" | "lt" | "lte" => {
                let right = expr.right.as_deref().ok_or_else(|| {
                    CompilerError::Message(format!("typed operator '{}' requires a right operand", op))
                })?;
                validate_typed_expression_node(right, registry)?;
                if !type_spec_compatible(&TypeSpec::number(), &left.type_spec)
                    || !type_spec_compatible(&TypeSpec::number(), &right.type_spec)
                    || !type_spec_compatible(&TypeSpec::boolean(), &expr.type_spec)
                {
                    return Err(CompilerError::Message(format!(
                        "typed operator '{}' requires numeric operands and boolean output",
                        op
                    )));
                }
            }
            "eq" | "neq" => {
                let right = expr.right.as_deref().ok_or_else(|| {
                    CompilerError::Message(format!("typed operator '{}' requires a right operand", op))
                })?;
                validate_typed_expression_node(right, registry)?;
                if !type_spec_compatible(&left.type_spec, &right.type_spec)
                    || !type_spec_compatible(&TypeSpec::boolean(), &expr.type_spec)
                {
                    return Err(CompilerError::Message(format!(
                        "typed operator '{}' requires matching operands and boolean output",
                        op
                    )));
                }
            }
            "and" | "or" => {
                let right = expr.right.as_deref().ok_or_else(|| {
                    CompilerError::Message(format!("typed operator '{}' requires a right operand", op))
                })?;
                validate_typed_expression_node(right, registry)?;
                if !type_spec_compatible(&TypeSpec::boolean(), &left.type_spec)
                    || !type_spec_compatible(&TypeSpec::boolean(), &right.type_spec)
                    || !type_spec_compatible(&TypeSpec::boolean(), &expr.type_spec)
                {
                    return Err(CompilerError::Message(format!(
                        "typed operator '{}' requires boolean operands and boolean output",
                        op
                    )));
                }
            }
            "not" => {
                if !type_spec_compatible(&TypeSpec::boolean(), &left.type_spec)
                    || !type_spec_compatible(&TypeSpec::boolean(), &expr.type_spec)
                {
                    return Err(CompilerError::Message(
                        "typed operator 'not' requires boolean operand and output".into(),
                    ));
                }
            }
            other => {
                return Err(CompilerError::Message(format!("unsupported typed expression operator '{}'", other)));
            }
        }
    }

    if let Some(path) = &expr.path {
        if path.trim().is_empty() {
            return Err(CompilerError::Message("typed expression path cannot be empty".into()));
        }
    }

    if let Some(value) = &expr.value {
        if !typed_value_matches_type(value, &expr.type_spec) {
            return Err(CompilerError::Message("typed expression value does not match declared type".into()));
        }
    }

    if expr.op.is_none() && expr.function.is_none() && expr.path.is_none() && expr.value.is_none() {
        return Err(CompilerError::Message("typed expression must define a value, path, function, or operator".into()));
    }

    Ok(())
}

fn type_spec_compatible(expected: &TypeSpec, actual: &TypeSpec) -> bool {
    match (expected, actual) {
        (
            TypeSpec::Primitive { primitive: expected_primitive },
            TypeSpec::Primitive { primitive: actual_primitive },
        ) => expected_primitive == actual_primitive,
        (TypeSpec::Array { items: expected_items }, TypeSpec::Array { items: actual_items }) => {
            if matches!(&**expected_items, TypeSpec::Object { fields } if fields.is_empty()) {
                true
            } else {
                type_spec_compatible(expected_items, actual_items)
            }
        }
        (TypeSpec::Object { fields: expected_fields }, TypeSpec::Object { fields: actual_fields }) => {
            if expected_fields.is_empty() {
                true
            } else {
                expected_fields.iter().all(|(key, expected)| {
                    actual_fields.get(key).map(|actual| type_spec_compatible(expected, actual)).unwrap_or(false)
                })
            }
        }
        _ => false,
    }
}

fn typed_value_matches_type(value: &serde_json::Value, type_spec: &TypeSpec) -> bool {
    match (value, type_spec) {
        (serde_json::Value::Bool(_), TypeSpec::Primitive { primitive: PrimitiveType::Boolean }) => true,
        (serde_json::Value::Number(_), TypeSpec::Primitive { primitive: PrimitiveType::Number }) => true,
        (serde_json::Value::String(_), TypeSpec::Primitive { primitive: PrimitiveType::String }) => true,
        (serde_json::Value::Array(_), TypeSpec::Array { .. }) => true,
        (serde_json::Value::Object(_), TypeSpec::Object { .. }) => true,
        (serde_json::Value::Null, _) => true,
        _ => false,
    }
}

fn contains_placeholder_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            lower.contains("tbd") || lower.contains("placeholder") || text.contains("{{")
        }
        serde_json::Value::Array(items) => items.iter().any(contains_placeholder_value),
        serde_json::Value::Object(map) => map.values().any(contains_placeholder_value),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_signature_detects_row_count() {
        let signature = data_signature_from_value(&serde_json::json!({
            "rows": [{ "id": 1 }, { "id": 2 }],
            "source": "postgres"
        }));

        assert_eq!(signature.row_count, Some(2));
        assert_eq!(signature.connector_id.as_deref(), Some("postgres"));
        assert!(signature.schema_hash.is_some());
    }

    #[test]
    fn test_variant_policy_selects_high_volume_variant() {
        let policy = WorkflowVariantPolicy {
            default_variant_id: "base".into(),
            fallback: VariantFallbackMode::UseDefaultVariant,
            variants: vec![
                WorkflowVariant {
                    id: "high_volume".into(),
                    priority: 100,
                    description: String::new(),
                    match_rule: VariantMatchRule { row_count_min: Some(10_001), ..VariantMatchRule::default() },
                    overrides: WorkflowVariantOverrides {
                        data_strategy: Some(DataStrategy { mode: "paginate".into(), page_size: Some(250) }),
                        ..WorkflowVariantOverrides::default()
                    },
                },
                WorkflowVariant {
                    id: "base".into(),
                    priority: 0,
                    description: String::new(),
                    match_rule: VariantMatchRule { row_count_max: Some(10_000), ..VariantMatchRule::default() },
                    overrides: WorkflowVariantOverrides::default(),
                },
            ],
        };

        let signature = DataSignature { row_count: Some(20_000), ..DataSignature::default() };
        let selection = policy
            .select(
                &signature,
                &ExecutionMode::Sequential,
                &ExecutionConstraints::default(),
                &DataStrategy::default(),
                &SchedulerConfig::default(),
            )
            .expect("high volume variant should match");

        assert_eq!(selection.variant_id, "high_volume");
        assert_eq!(selection.data_strategy.page_size, Some(250));
    }

    #[test]
    fn test_variant_policy_falls_back_to_default_variant() {
        let policy = WorkflowVariantPolicy {
            default_variant_id: "base".into(),
            fallback: VariantFallbackMode::UseDefaultVariant,
            variants: vec![WorkflowVariant {
                id: "base".into(),
                priority: 0,
                description: String::new(),
                match_rule: VariantMatchRule { row_count_max: Some(10_000), ..VariantMatchRule::default() },
                overrides: WorkflowVariantOverrides::default(),
            }],
        };

        let signature = DataSignature::default();
        let selection = policy
            .select(
                &signature,
                &ExecutionMode::Sequential,
                &ExecutionConstraints::default(),
                &DataStrategy::default(),
                &SchedulerConfig::default(),
            )
            .expect("default variant should match");

        assert_eq!(selection.variant_id, "base");
    }
}
