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

// Re-export the intent helpers still used directly by API routes.
pub use intent::{
    intent_needs_api_connection, intent_needs_database_connection, intent_needs_mcp_connection,
    intent_to_trigger,
};
pub use steps::parse_trigger_from_text;
