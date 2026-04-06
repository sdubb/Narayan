//! Plan mode — compiler-style agent configuration conversation.
//!
//! The module tree follows the architecture:
//!   - `intent`     – IntentExtractor, intent analysis, trigger parsing
//!   - `registry`   – ConnectorResolver, capability packets
//!   - `clarify`    – ClarificationEngine, step queues, boundary/subsystem setup steps
//!   - `repair`     – Compiler repair loop, compact repair context
//!   - `boundary`   – Boundary handshake detection, collection, injection
//!   - `subsystems` – Agent subsystem policies, binding, review summaries
//!   - `review`     – WorkflowContract (replaces hints), review checklist, role defaults
//!   - `orchestrator` – Thin coordinator: PlanModeManager routes phases to modules

#[path = "orchestrator.rs"]
mod orchestrator;

pub mod boundary;
pub mod clarify;
pub mod discovery;
pub mod intent;
pub mod registry;
pub mod repair;
pub mod review;
pub mod steps;
pub mod subsystems;

// Re-export the public API from orchestrator (PlanModeManager, IntentExtractor, etc.)
pub use orchestrator::*;

// Re-export key types from submodules for convenience
pub use boundary::{BoundaryNeed, BoundaryScope, BoundarySetupResult};
pub use registry::CapabilityPacket;
pub use review::{WorkflowContract, CompilerValidationState, ApprovalStatus, GovernanceCheck, ReviewChecklistItem};
pub use steps::{
    ClarificationStep, StepField, PlanModeRetryPolicy, PlanModeWorkflowDraft,
    PlanModeWorkflowResponsibility, PlanModeWorkflowStep,
    generate_steps, parse_and_apply, default_completion_criteria,
    workflow_contract_prompt_fragment, intent_extractor_system_prompt,
};
pub use subsystems::SubsystemPolicy;
