//! Agent subsystem binding engine.
//!
//! Subsystems are the seven first-class capabilities every agent definition must
//! explicitly configure: memory, knowledge, swarm, scheduler, skills, storage,
//! and workspace. This module provides:
//!
//! - The canonical list of subsystem names (`AGENT_SUBSYSTEMS`).
//! - Policy structs that capture the *intended posture* for each subsystem.
//! - Role-category-aware defaults so plan mode can pre-populate sensible policies.
//! - An `apply_subsystem_policy` function that stamps policy decisions onto a
//!   mutable `AgentRole`, wiring memory scope, execution guidelines, and
//!   constraints into the role definition.
//! - A human-readable review summary for the plan-mode review step.

use serde::{Deserialize, Serialize};

use crate::agent::definition::{AgentRole, GuidelineRule, MemoryScope, RoleCategory};

// ── Re-export & helpers ──────────────────────────────────────────────────────

pub use super::intent::AGENT_SUBSYSTEMS;

/// Returns the canonical subsystem name list.
pub fn subsystem_names() -> &'static [&'static str] {
    AGENT_SUBSYSTEMS
}

/// Formats the setup prompt string shown to the LLM during plan-mode
/// configuration so it knows which subsystems need explicit decisions.
pub fn subsystem_setup_prompt() -> String {
    format!(
        "Configure the agent subsystems explicitly: {}.",
        AGENT_SUBSYSTEMS.join(", ")
    )
}

// ── Policy enums ─────────────────────────────────────────────────────────────

/// Memory subsystem posture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPolicy {
    /// No memory access at all.
    Disabled,
    /// Memory lives only for the duration of a single goal instance.
    SessionOnly,
    /// Memory persists across goal instances (agent-level by default).
    Persistent,
    /// Memory is scoped to a named partition (e.g. per-customer, per-project).
    Scoped(String),
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self::SessionOnly
    }
}

/// Knowledge-base subsystem posture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgePolicy {
    /// No access to the knowledge base.
    Disabled,
    /// Can read/query the knowledge base but never write.
    ReadOnly,
    /// Can both read and write (e.g. ingest new documents).
    ReadWrite,
}

impl Default for KnowledgePolicy {
    fn default() -> Self {
        Self::ReadOnly
    }
}

/// Swarm (multi-agent delegation) subsystem posture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwarmPolicy {
    /// No swarm capabilities.
    Disabled,
    /// Can delegate sub-tasks to other agents but cannot spawn peers.
    DelegateOnly,
    /// Full swarm: can spawn, delegate, and coordinate peer agents.
    FullSwarm,
}

impl Default for SwarmPolicy {
    fn default() -> Self {
        Self::Disabled
    }
}

/// Scheduler subsystem posture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerPolicy {
    /// No scheduler access.
    Disabled,
    /// Role runs on demand (manual or trigger-based) -- no self-scheduling.
    OnDemand,
    /// Role can schedule itself on a cron expression.
    Scheduled(String),
    /// Role activates in response to external events / webhooks.
    Triggered,
}

impl Default for SchedulerPolicy {
    fn default() -> Self {
        Self::OnDemand
    }
}

/// Skills subsystem posture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillsPolicy {
    /// Only built-in platform skills are available.
    BuiltinOnly,
    /// Built-in plus tenant-defined custom skills.
    WithCustom,
    /// All skills, including dynamically discovered ones.
    Unrestricted,
}

impl Default for SkillsPolicy {
    fn default() -> Self {
        Self::BuiltinOnly
    }
}

/// Storage subsystem posture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoragePolicy {
    /// No storage access.
    Disabled,
    /// Can only read/write within the agent's own workspace directory.
    WorkspaceOnly,
    /// Can access tenant-scoped shared storage (e.g. shared drive, S3 prefix).
    TenantScoped,
    /// Unrestricted storage access (admin-level).
    Unrestricted,
}

impl Default for StoragePolicy {
    fn default() -> Self {
        Self::WorkspaceOnly
    }
}

/// Workspace subsystem posture (file-system-level access within the sandbox).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePolicy {
    /// Can read workspace files but never create or modify them.
    ReadOnly,
    /// Full read/write access to workspace files.
    ReadWrite,
    /// Runs in an isolated scratch directory -- cannot see the main workspace.
    Isolated,
}

impl Default for WorkspacePolicy {
    fn default() -> Self {
        Self::ReadOnly
    }
}

// ── Composite policy ─────────────────────────────────────────────────────────

/// Complete subsystem policy for a single role, covering all seven subsystems.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubsystemPolicy {
    pub memory: MemoryPolicy,
    pub knowledge: KnowledgePolicy,
    pub swarm: SwarmPolicy,
    pub scheduler: SchedulerPolicy,
    pub skills: SkillsPolicy,
    pub storage: StoragePolicy,
    pub workspace: WorkspacePolicy,
}

// ── Constructors ─────────────────────────────────────────────────────────────

/// Returns a `SubsystemPolicy` where every subsystem uses its `Default` value.
pub fn default_subsystem_policy() -> SubsystemPolicy {
    SubsystemPolicy::default()
}

/// Returns a sensible `SubsystemPolicy` pre-populated from a `RoleCategory`.
///
/// Plan mode calls this to give the user a reasonable starting point; the user
/// can then override individual policies before the role is finalised.
pub fn subsystem_policy_from_role_category(category: &RoleCategory) -> SubsystemPolicy {
    match category {
        RoleCategory::SoftwareEngineer => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadWrite,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::WithCustom,
            storage: StoragePolicy::WorkspaceOnly,
            workspace: WorkspacePolicy::ReadWrite,
        },

        RoleCategory::ResearchAnalyst => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadWrite,
            swarm: SwarmPolicy::DelegateOnly,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::WithCustom,
            storage: StoragePolicy::WorkspaceOnly,
            workspace: WorkspacePolicy::ReadOnly,
        },

        RoleCategory::CustomerSupport => SubsystemPolicy {
            memory: MemoryPolicy::SessionOnly,
            knowledge: KnowledgePolicy::ReadOnly,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::BuiltinOnly,
            storage: StoragePolicy::TenantScoped,
            workspace: WorkspacePolicy::ReadOnly,
        },

        RoleCategory::DevOps => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadWrite,
            swarm: SwarmPolicy::DelegateOnly,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::WithCustom,
            storage: StoragePolicy::WorkspaceOnly,
            workspace: WorkspacePolicy::ReadWrite,
        },

        RoleCategory::Marketing => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadWrite,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::WithCustom,
            storage: StoragePolicy::TenantScoped,
            workspace: WorkspacePolicy::ReadOnly,
        },

        RoleCategory::DataExtraction => SubsystemPolicy {
            memory: MemoryPolicy::SessionOnly,
            knowledge: KnowledgePolicy::ReadOnly,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::BuiltinOnly,
            storage: StoragePolicy::WorkspaceOnly,
            workspace: WorkspacePolicy::ReadWrite,
        },

        RoleCategory::SalesRevOps => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadWrite,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::WithCustom,
            storage: StoragePolicy::TenantScoped,
            workspace: WorkspacePolicy::ReadOnly,
        },

        RoleCategory::FinanceAccounting => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadOnly,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::BuiltinOnly,
            storage: StoragePolicy::TenantScoped,
            workspace: WorkspacePolicy::ReadOnly,
        },

        RoleCategory::HRPeopleOps => SubsystemPolicy {
            memory: MemoryPolicy::SessionOnly,
            knowledge: KnowledgePolicy::ReadOnly,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::BuiltinOnly,
            storage: StoragePolicy::TenantScoped,
            workspace: WorkspacePolicy::ReadOnly,
        },

        RoleCategory::LegalContract => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadOnly,
            swarm: SwarmPolicy::Disabled,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::BuiltinOnly,
            storage: StoragePolicy::TenantScoped,
            workspace: WorkspacePolicy::ReadOnly,
        },

        RoleCategory::ITOpsITSM => SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadWrite,
            swarm: SwarmPolicy::DelegateOnly,
            scheduler: SchedulerPolicy::OnDemand,
            skills: SkillsPolicy::WithCustom,
            storage: StoragePolicy::WorkspaceOnly,
            workspace: WorkspacePolicy::ReadWrite,
        },

        // General and any future variants fall back to all-defaults.
        _ => SubsystemPolicy::default(),
    }
}

// ── Apply policy to role ─────────────────────────────────────────────────────

/// Stamps the subsystem policy onto an `AgentRole`.
///
/// This is the single point where policy decisions become concrete role config:
///   - `memory_scope` is derived from the memory policy.
///   - Execution guidelines are appended for swarm, scheduler, and skills.
///   - Hard constraints are added for storage and workspace boundaries.
///
/// The function is idempotent: calling it twice with the same policy produces
/// the same result (it clears policy-generated guidelines before re-adding).
pub fn apply_subsystem_policy(policy: &SubsystemPolicy, role: &mut AgentRole) {
    // ── 1. Memory scope ──────────────────────────────────────────────────
    role.memory_scope = match &policy.memory {
        MemoryPolicy::Disabled => MemoryScope::Role, // most isolated scope available
        MemoryPolicy::SessionOnly => MemoryScope::Role,
        MemoryPolicy::Persistent => MemoryScope::Agent,
        MemoryPolicy::Scoped(_) => MemoryScope::Global,
    };

    // ── 2. Clear previously injected policy guidelines ───────────────────
    // We tag policy-generated rules with a tool_scope prefix of "[subsystem-policy]"
    // so we can strip them on re-apply without touching user-authored rules.
    let policy_tag = "[subsystem-policy]";
    role.execution_guidelines.rules.retain(|r| {
        r.tool_scope
            .as_deref()
            .map_or(true, |scope| !scope.starts_with(policy_tag))
    });
    role.execution_guidelines.priorities.retain(|p| !p.starts_with(policy_tag));

    // ── 3. Swarm guidelines ──────────────────────────────────────────────
    match &policy.swarm {
        SwarmPolicy::Disabled => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Do not delegate tasks to other agents. Execute all work within this role.".into(),
                tool_scope: Some(format!("{policy_tag}/swarm")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        SwarmPolicy::DelegateOnly => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "You may delegate sub-tasks to other agents when the task falls outside your expertise, \
                       but do not spawn or coordinate peer agents. Always await delegate results before proceeding."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/swarm")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        SwarmPolicy::FullSwarm => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Full swarm capabilities enabled. You may spawn, delegate to, and coordinate peer agents. \
                       Prefer delegation for independent sub-tasks to improve throughput."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/swarm")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
    }

    // ── 4. Scheduler guidelines ──────────────────────────────────────────
    match &policy.scheduler {
        SchedulerPolicy::Disabled => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Scheduler access is disabled. Do not attempt to schedule future runs.".into(),
                tool_scope: Some(format!("{policy_tag}/scheduler")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        SchedulerPolicy::OnDemand => {
            // On-demand is the neutral default; no extra guideline needed beyond
            // a priority marker so the review summary knows the decision was made.
            role.execution_guidelines
                .priorities
                .push(format!("{policy_tag} Scheduler: on-demand only"));
        }
        SchedulerPolicy::Scheduled(cron_expr) => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: format!(
                    "This role runs on schedule: `{cron_expr}`. Do not self-schedule beyond the declared cadence."
                ),
                tool_scope: Some(format!("{policy_tag}/scheduler")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        SchedulerPolicy::Triggered => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "This role activates on external triggers (webhooks / events). \
                       Do not poll or self-schedule; wait for the trigger payload."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/scheduler")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
    }

    // ── 5. Skills guidelines ─────────────────────────────────────────────
    match &policy.skills {
        SkillsPolicy::BuiltinOnly => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Only built-in platform skills are available. Do not attempt to invoke custom or dynamically-discovered skills.".into(),
                tool_scope: Some(format!("{policy_tag}/skills")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        SkillsPolicy::WithCustom => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Built-in and tenant-defined custom skills are available. \
                       Prefer built-in skills when both can accomplish the task."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/skills")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        SkillsPolicy::Unrestricted => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "All skills are available, including dynamically discovered ones. \
                       Validate skill output schema before relying on unfamiliar skills."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/skills")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
    }

    // ── 6. Storage constraints ───────────────────────────────────────────
    match &policy.storage {
        StoragePolicy::Disabled => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Storage access is disabled. Do not read from or write to any persistent storage.".into(),
                tool_scope: Some(format!("{policy_tag}/storage")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        StoragePolicy::WorkspaceOnly => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Storage is limited to the agent workspace directory. \
                       Do not access files outside the workspace boundary."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/storage")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        StoragePolicy::TenantScoped => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Storage access is tenant-scoped. You may read/write shared tenant storage \
                       (e.g. shared drives, S3 prefixes) but never access other tenants' data."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/storage")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        StoragePolicy::Unrestricted => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Unrestricted storage access is granted. Exercise caution -- verify paths before \
                       writing and never overwrite critical system files."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/storage")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
    }

    // ── 7. Workspace constraints ─────────────────────────────────────────
    match &policy.workspace {
        WorkspacePolicy::ReadOnly => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Workspace is read-only. You may inspect files but must not create, modify, or delete them.".into(),
                tool_scope: Some(format!("{policy_tag}/workspace")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        WorkspacePolicy::ReadWrite => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Workspace is read-write. You may create, modify, and delete files within the workspace.".into(),
                tool_scope: Some(format!("{policy_tag}/workspace")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        WorkspacePolicy::Isolated => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Workspace is isolated. You operate in a private scratch directory and cannot see \
                       or modify the main workspace. Write all outputs to the scratch directory."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/workspace")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
    }

    // ── 8. Knowledge guideline (informational) ───────────────────────────
    match &policy.knowledge {
        KnowledgePolicy::Disabled => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Knowledge base access is disabled. Do not query the knowledge base.".into(),
                tool_scope: Some(format!("{policy_tag}/knowledge")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        KnowledgePolicy::ReadOnly => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Knowledge base is read-only. You may query existing documents but must not ingest or update content.".into(),
                tool_scope: Some(format!("{policy_tag}/knowledge")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        KnowledgePolicy::ReadWrite => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Knowledge base is read-write. You may query, ingest, and update documents as needed.".into(),
                tool_scope: Some(format!("{policy_tag}/knowledge")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
    }

    // ── 9. Memory guideline (informational, beyond the scope mapping) ────
    match &policy.memory {
        MemoryPolicy::Disabled => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Memory is disabled. Do not attempt to read or write to the memory store.".into(),
                tool_scope: Some(format!("{policy_tag}/memory")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        MemoryPolicy::SessionOnly => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Memory is session-scoped. Stored data will not persist beyond the current goal instance.".into(),
                tool_scope: Some(format!("{policy_tag}/memory")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        MemoryPolicy::Persistent => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: "Memory is persistent and shared across goal instances at the agent level. \
                       Use it for learned preferences, accumulated context, and cross-run state."
                    .into(),
                tool_scope: Some(format!("{policy_tag}/memory")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
        MemoryPolicy::Scoped(partition) => {
            role.execution_guidelines.rules.push(GuidelineRule {
                text: format!(
                    "Memory is scoped to partition `{partition}`. Data is isolated to this partition \
                     and shared globally within it."
                ),
                tool_scope: Some(format!("{policy_tag}/memory")),
                phase: crate::agent::definition::RulePhase::Always,
            });
        }
    }
}

// ── Review summary ───────────────────────────────────────────────────────────

/// Produces a human-readable summary of the subsystem policy, suitable for the
/// plan-mode review step.
///
/// Example output:
/// ```text
/// Subsystem Policy Summary
/// ========================
///   memory:    Persistent
///   knowledge: ReadWrite
///   swarm:     Disabled
///   scheduler: OnDemand
///   skills:    WithCustom
///   storage:   WorkspaceOnly
///   workspace: ReadWrite
/// ```
pub fn subsystem_review_summary(policy: &SubsystemPolicy) -> String {
    let memory_str = match &policy.memory {
        MemoryPolicy::Disabled => "Disabled".to_string(),
        MemoryPolicy::SessionOnly => "SessionOnly".to_string(),
        MemoryPolicy::Persistent => "Persistent".to_string(),
        MemoryPolicy::Scoped(partition) => format!("Scoped({partition})"),
    };

    let knowledge_str = match &policy.knowledge {
        KnowledgePolicy::Disabled => "Disabled",
        KnowledgePolicy::ReadOnly => "ReadOnly",
        KnowledgePolicy::ReadWrite => "ReadWrite",
    };

    let swarm_str = match &policy.swarm {
        SwarmPolicy::Disabled => "Disabled",
        SwarmPolicy::DelegateOnly => "DelegateOnly",
        SwarmPolicy::FullSwarm => "FullSwarm",
    };

    let scheduler_str = match &policy.scheduler {
        SchedulerPolicy::Disabled => "Disabled".to_string(),
        SchedulerPolicy::OnDemand => "OnDemand".to_string(),
        SchedulerPolicy::Scheduled(cron) => format!("Scheduled({cron})"),
        SchedulerPolicy::Triggered => "Triggered".to_string(),
    };

    let skills_str = match &policy.skills {
        SkillsPolicy::BuiltinOnly => "BuiltinOnly",
        SkillsPolicy::WithCustom => "WithCustom",
        SkillsPolicy::Unrestricted => "Unrestricted",
    };

    let storage_str = match &policy.storage {
        StoragePolicy::Disabled => "Disabled",
        StoragePolicy::WorkspaceOnly => "WorkspaceOnly",
        StoragePolicy::TenantScoped => "TenantScoped",
        StoragePolicy::Unrestricted => "Unrestricted",
    };

    let workspace_str = match &policy.workspace {
        WorkspacePolicy::ReadOnly => "ReadOnly",
        WorkspacePolicy::ReadWrite => "ReadWrite",
        WorkspacePolicy::Isolated => "Isolated",
    };

    format!(
        "Subsystem Policy Summary\n\
         ========================\n\
         {blank:2}memory:    {memory}\n\
         {blank:2}knowledge: {knowledge}\n\
         {blank:2}swarm:     {swarm}\n\
         {blank:2}scheduler: {scheduler}\n\
         {blank:2}skills:    {skills}\n\
         {blank:2}storage:   {storage}\n\
         {blank:2}workspace: {workspace}",
        blank = "",
        memory = memory_str,
        knowledge = knowledge_str,
        swarm = swarm_str,
        scheduler = scheduler_str,
        skills = skills_str,
        storage = storage_str,
        workspace = workspace_str,
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::RoleCategory;

    #[test]
    fn test_subsystem_names_matches_constant() {
        let names = subsystem_names();
        assert_eq!(names, AGENT_SUBSYSTEMS);
        assert_eq!(names.len(), 7);
        assert!(names.contains(&"memory"));
        assert!(names.contains(&"workspace"));
    }

    #[test]
    fn test_subsystem_setup_prompt_contains_all_subsystems() {
        let prompt = subsystem_setup_prompt();
        for name in AGENT_SUBSYSTEMS {
            assert!(prompt.contains(name), "prompt missing subsystem: {name}");
        }
    }

    #[test]
    fn test_default_subsystem_policy() {
        let policy = default_subsystem_policy();
        assert_eq!(policy.memory, MemoryPolicy::SessionOnly);
        assert_eq!(policy.knowledge, KnowledgePolicy::ReadOnly);
        assert_eq!(policy.swarm, SwarmPolicy::Disabled);
        assert_eq!(policy.scheduler, SchedulerPolicy::OnDemand);
        assert_eq!(policy.skills, SkillsPolicy::BuiltinOnly);
        assert_eq!(policy.storage, StoragePolicy::WorkspaceOnly);
        assert_eq!(policy.workspace, WorkspacePolicy::ReadOnly);
    }

    #[test]
    fn test_software_engineer_policy() {
        let policy = subsystem_policy_from_role_category(&RoleCategory::SoftwareEngineer);
        assert_eq!(policy.memory, MemoryPolicy::Persistent);
        assert_eq!(policy.knowledge, KnowledgePolicy::ReadWrite);
        assert_eq!(policy.workspace, WorkspacePolicy::ReadWrite);
        assert_eq!(policy.skills, SkillsPolicy::WithCustom);
    }

    #[test]
    fn test_research_analyst_policy() {
        let policy = subsystem_policy_from_role_category(&RoleCategory::ResearchAnalyst);
        assert_eq!(policy.memory, MemoryPolicy::Persistent);
        assert_eq!(policy.knowledge, KnowledgePolicy::ReadWrite);
        assert_eq!(policy.swarm, SwarmPolicy::DelegateOnly);
        assert_eq!(policy.skills, SkillsPolicy::WithCustom);
    }

    #[test]
    fn test_customer_support_policy() {
        let policy = subsystem_policy_from_role_category(&RoleCategory::CustomerSupport);
        assert_eq!(policy.memory, MemoryPolicy::SessionOnly);
        assert_eq!(policy.knowledge, KnowledgePolicy::ReadOnly);
        assert_eq!(policy.storage, StoragePolicy::TenantScoped);
    }

    #[test]
    fn test_general_uses_defaults() {
        let policy = subsystem_policy_from_role_category(&RoleCategory::General);
        let default = default_subsystem_policy();
        assert_eq!(policy.memory, default.memory);
        assert_eq!(policy.knowledge, default.knowledge);
        assert_eq!(policy.swarm, default.swarm);
        assert_eq!(policy.scheduler, default.scheduler);
        assert_eq!(policy.skills, default.skills);
        assert_eq!(policy.storage, default.storage);
        assert_eq!(policy.workspace, default.workspace);
    }

    #[test]
    fn test_review_summary_format() {
        let policy = SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            knowledge: KnowledgePolicy::ReadWrite,
            swarm: SwarmPolicy::DelegateOnly,
            scheduler: SchedulerPolicy::Scheduled("0 9 * * 1-5".into()),
            skills: SkillsPolicy::WithCustom,
            storage: StoragePolicy::TenantScoped,
            workspace: WorkspacePolicy::ReadWrite,
        };
        let summary = subsystem_review_summary(&policy);
        assert!(summary.contains("Persistent"));
        assert!(summary.contains("ReadWrite"));
        assert!(summary.contains("DelegateOnly"));
        assert!(summary.contains("Scheduled(0 9 * * 1-5)"));
        assert!(summary.contains("WithCustom"));
        assert!(summary.contains("TenantScoped"));
        // workspace ReadWrite appears
        assert!(summary.contains("workspace: ReadWrite"));
    }

    #[test]
    fn test_review_summary_scoped_memory() {
        let policy = SubsystemPolicy {
            memory: MemoryPolicy::Scoped("customer-123".into()),
            ..SubsystemPolicy::default()
        };
        let summary = subsystem_review_summary(&policy);
        assert!(summary.contains("Scoped(customer-123)"));
    }

    #[test]
    fn test_apply_policy_sets_memory_scope() {
        use crate::agent::definition::MemoryScope;

        let mut role = make_test_role();
        let policy = SubsystemPolicy {
            memory: MemoryPolicy::Persistent,
            ..SubsystemPolicy::default()
        };
        apply_subsystem_policy(&policy, &mut role);
        assert_eq!(role.memory_scope, MemoryScope::Agent);

        let policy_disabled = SubsystemPolicy {
            memory: MemoryPolicy::Disabled,
            ..SubsystemPolicy::default()
        };
        apply_subsystem_policy(&policy_disabled, &mut role);
        assert_eq!(role.memory_scope, MemoryScope::Role);

        let policy_scoped = SubsystemPolicy {
            memory: MemoryPolicy::Scoped("partition-x".into()),
            ..SubsystemPolicy::default()
        };
        apply_subsystem_policy(&policy_scoped, &mut role);
        assert_eq!(role.memory_scope, MemoryScope::Global);
    }

    #[test]
    fn test_apply_policy_is_idempotent() {
        let mut role = make_test_role();
        let policy = subsystem_policy_from_role_category(&RoleCategory::SoftwareEngineer);

        apply_subsystem_policy(&policy, &mut role);
        let count_after_first = role.execution_guidelines.rules.len();

        apply_subsystem_policy(&policy, &mut role);
        let count_after_second = role.execution_guidelines.rules.len();

        assert_eq!(count_after_first, count_after_second, "apply should be idempotent");
    }

    #[test]
    fn test_apply_policy_preserves_user_rules() {
        let mut role = make_test_role();
        role.execution_guidelines.rules.push(GuidelineRule::always("Always log before writing to CRM"));

        let policy = subsystem_policy_from_role_category(&RoleCategory::SalesRevOps);
        apply_subsystem_policy(&policy, &mut role);

        // User rule should still be there
        assert!(role.execution_guidelines.rules.iter().any(|r| r.text.contains("Always log before writing")));
    }

    // Helper: build a minimal AgentRole for testing.
    fn make_test_role() -> AgentRole {
        use chrono::Utc;
        AgentRole {
            id: "test-role".into(),
            agent_id: "test-agent".into(),
            tenant_id: "test-tenant".into(),
            version: 1,
            status: crate::agent::definition::RoleStatus::Draft,
            name: "Test Role".into(),
            trigger: Default::default(),
            purpose: "Testing subsystem policies".into(),
            role_category: RoleCategory::General,
            execution_guidelines: Default::default(),
            connectors: vec![],
            tools: vec![],
            output_spec: Default::default(),
            memory_scope: Default::default(),
            execution_limits: Default::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}
