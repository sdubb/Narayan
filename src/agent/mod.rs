pub mod agent_chat;
pub mod clarifier;
pub mod dag;
pub mod dag_engine;
pub mod definition;
pub mod evaluator;
pub mod executor;
pub mod r#loop;
pub mod manager;
pub mod orchestrator;
pub mod plan_mode;
pub mod planner;
pub mod preflight;
pub mod prompts;
pub mod reflector;
pub mod role_chat;
pub mod savings;
pub mod step_artifacts;
pub mod template_vars;
pub mod templates;
pub mod workflow_compiler;

#[allow(unused_imports)]
pub use agent_chat::{AgentChatManager, AgentChatMessage, AgentChatRequest};
#[allow(unused_imports)]
pub use clarifier::{ClarificationAnswers, LlmClarifier};
#[allow(unused_imports)]
pub use definition::{
    infer_failure_action, AgentDefinition, AgentDefinitionStatus, AgentRole, CompletionCheck, CompletionCriterion,
    ConnectorAuthType, ConnectorSource, EndpointDef, EndpointParam, ExecutionGuidelines, ExecutionLimits,
    ExecutionStrategy, FailureAction, FailureRule, GuidelineRule, MemoryScope, OutputDestination, OutputFormat,
    OutputSpec, ParamLocation, PlanModeAttachment, PlanModeAttachmentKind, PlanModeAttachmentUpload, PlanModeMessage,
    PlanModePhase, PlanModePreflightResult, PlanModeSandboxResult, PlanModeSession, PlanModeTestCheck,
    PlanModeTestConfidence, PlanModeTestResult, PlanModeTestStatus, PlanModeTestStepResult, RoleResponsibility,
    RoleStatus, RulePhase, TenantConnector, ToolPool, TriggerConfidence, TriggerDef, TriggerType,
    WorkforceEventPayload, WorkforceEventSubscription,
};
#[allow(unused_imports)]
pub use evaluator::LlmEvaluator;
#[allow(unused_imports)]
pub use executor::LlmExecutor;
#[allow(unused_imports)]
pub use manager::AgentManager;
#[allow(unused_imports)]
pub use plan_mode::PlanModeManager;
#[allow(unused_imports)]
pub use plan_mode::steps::{
    PlanModeRetryPolicy, PlanModeWorkflowDraft, PlanModeWorkflowResponsibility, PlanModeWorkflowStep,
    workflow_contract_prompt_fragment,
};
#[allow(unused_imports)]
pub use plan_mode::registry::{build_registry_candidate_context, build_registry_candidate_set};
#[allow(unused_imports)]
pub use preflight::LlmPreflight;
#[allow(unused_imports)]
pub use r#loop::AgentLoop;
#[allow(unused_imports)]
pub use reflector::LlmReflector;
#[allow(unused_imports)]
pub use role_chat::RoleChatManager;
#[allow(unused_imports)]
pub use templates::{all_templates, find_template, RoleTemplate};
#[allow(unused_imports)]
pub use workflow_compiler::{
    BindingRule, CompiledStep, CompiledWorkflow, CompilerCardRequest, CompilerError, CompilerResult, DataSignature,
    DataStrategy, DeterminismConfig, DslStepType, ExecutionConstraints, ExecutionMode, ExecutionPolicy,
    ExpressionFunctionSpec, IdempotencyClass, PrimitiveType, ResourceBinding, ResumeBehavior, SchedulerConfig,
    TypeSpec, VariantFallbackMode, VariantMatchRule, WorkflowCompiler, WorkflowVariant, WorkflowVariantOverrides,
    WorkflowVariantPolicy, WorkflowVariantSelection,
};

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
pub(crate) mod test_helpers;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod tests2;
