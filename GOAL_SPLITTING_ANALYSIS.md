# Goal Splitting & Multi-Role Spawning Analysis

## Executive Summary

Narayan implements **two distinct goal-splitting systems**:

1. **Plan Mode Goal Splitting** — LLM-driven configuration time role creation based on detected responsibilities
2. **Runtime Delegation** — Executor-time spawning of temporary child agents for parallel work

Both are **NOT currently integrated**. They operate independently.

---

## System 1: Plan Mode Goal Splitting (Configuration Time)

### Overview
During agent setup ("plan mode"), the LLM analyzes the job description and suggests splitting into multiple roles. Users choose "yes" or "no" to split. If yes, remaining roles are stashed and configured in subsequent sessions.

### Flow Diagram
```
User describes agent
    ↓
IntentExtractor (LLM) analyzes → outputs "responsibilities", "multi_role_suggested"
    ↓
User reviews suggestions
    ↓
USER: "B - split into separate roles"
    ↓
parse_and_apply() extracts remaining responsibilities
    ↓
Stash in AgentDefinition.memory_ref as "|pending_roles:[...]"
    ↓
save() persists ONE Agent + ONE Role to DB
    ↓
[Frontend detects memory_ref contains pending_roles]
    ↓
Open plan mode again for Role 2 (NOT in backend code)
```

### Implementation

#### 1. Intent Extraction — LLM analyzes job description
**File:** [src/agent/plan_mode.rs](src/agent/plan_mode.rs#L70-150)
**Method:** `IntentExtractor::extract_initial()`
**Lines:** 70-150

The LLM receives this schema and outputs `responsibilities` array:

```json
{
  "responsibilities": [
    {"name": "short role name", "actions": ["verbs"], "trigger_hint": "schedule|webhook|manual"}
  ],
  "multi_role_suggested": false,
  "multi_role_reason": "why split is recommended, or null"
}
```

**Exact code:**
```rust
let system = format!(
    r#"...
"responsibilities": [
  {{"name": "short role name", "actions": ["verbs"], "trigger_hint": "schedule|webhook|manual"}}
],
"multi_role_suggested": false,
"multi_role_reason": "why split is recommended, or null",
"clarifying_questions": []
}}...
- multi_role_suggested: true only if 2+ clearly distinct responsibilities with different triggers or outputs
- responsibilities: always list at least one entry"#,
    capability_section
);
```

#### 2. Role Split Decision — User chooses to split
**File:** [src/agent/plan_mode_steps.rs](src/agent/plan_mode_steps.rs#L230-350)
**Function:** `parse_and_apply()`
**Lines:** 315-350
**Step Field:** `StepField::RoleSplit`

The step parser asks: **"A - Configure as one role, B - Split into separate roles"**

```rust
StepField::RoleSplit => {
    let wants_split = lower.contains('b') && !lower.contains("best")
        || lower.contains("split")
        || lower.contains("separate")
        || lower.contains("two roles")
        || lower.contains("multiple");

    if wants_split {
        let responsibilities = intent["responsibilities"].as_array().cloned().unwrap_or_default();
        if responsibilities.len() > 1 {
            let mut remaining = responsibilities.clone();
            remaining.remove(0); // first is being configured now
            *pending_roles_sink = Some(remaining.clone());
            // Update role name to first responsibility
            if let Some(name) = responsibilities[0]["name"].as_str() {
                role.name = name.to_string();
            }
            return format!(
                "I'll configure {} roles. Starting with: **{}**.",
                responsibilities.len(),
                responsibilities[0]["name"].as_str().unwrap_or("Role 1")
            );
        }
    }
    "Keeping as one role.".into()
}
```

**Key points:**
- Checks if user answer contains keywords: `'b'`, `"split"`, `"separate"`, `"two roles"`, `"multiple"`
- Extracts `responsibilities` array from cached intent (from LLM extraction)
- Removes first responsibility (being configured in current session)
- Stores remaining responsibilities in `pending_roles_sink` (output parameter)

#### 3. Stashing Pending Roles — Store in agent metadata
**File:** [src/agent/plan_mode.rs](src/agent/plan_mode.rs#L1300-1330)
**Method:** `handle_clarifications()`
**Lines:** 1300-1330

After the user answers the role split step, pending roles are serialized:

```rust
// If user chose to split roles, stash pending responsibilities
if let Some(remaining) = pending_roles {
    if !session.draft_agent.memory_ref.contains("|pending_roles:") {
        let meta = session.draft_agent.memory_ref.clone();
        session.draft_agent.memory_ref =
            format!("{}|pending_roles:{}", meta, serde_json::to_string(&remaining).unwrap_or_default());
    }
}
```

**Example value:**
```
memory_ref = "agent:a1b2c3d4|pending_roles:[{\"name\":\"Enrich\",\"actions\":[\"lookup\"],\"trigger_hint\":\"webhook\"},{\"name\":\"Notify\",\"actions\":[\"send\"],\"trigger_hint\":\"manual\"}]"
```

#### 4. Save Agent + Role to Database
**File:** [src/agent/plan_mode.rs](src/agent/plan_mode.rs#L2450-2500)
**Method:** `PlanModeManager::save()`
**Lines:** 2450-2500

```rust
pub async fn save(&self, mut session: PlanModeSession) -> Result<(AgentDefinition, AgentRole)> {
    let mut agent = session.draft_agent.clone();
    agent.status = AgentDefinitionStatus::Active;
    agent.updated_at = Utc::now();

    self.store.upsert_agent_definition(&agent).await?;  // Saves memory_ref with |pending_roles:

    let role = match session.draft_role.take() {
        Some(mut r) => {
            r.status = RoleStatus::Active;
            r.updated_at = Utc::now();
            
            // Enrich workflow outline
            let intent = session.intent_cache.as_ref().cloned().unwrap_or_else(|| serde_json::json!({}));
            enrich_workflow_outline(&mut r, &intent);
            
            // Resolve depends_on hints to actual role IDs
            if let Some(hint) = r.trigger.depends_on_role_id.clone() {
                if let Some(name) = hint.strip_prefix("name:") {
                    let existing = self.store.list_roles_for_agent(&agent.tenant_id, &agent.id).await?;
                    if let Some(found) = existing.iter().find(|er| er.name == name) {
                        r.trigger.depends_on_role_id = Some(found.id.clone());
                    }
                }
            }

            self.store.upsert_agent_role(&r).await?;
            
            // Sync workforce event subscription if needed
            crate::events::workforce::sync_subscriptions_for_role(&r, &self.store).await?;
            r
        }
        None => anyhow::bail!("cannot save plan mode session with no role defined")
    };

    Ok((agent, role))
}
```

#### 5. Data Structures for Plan Mode Roles
**File:** [src/agent/definition.rs](src/agent/definition.rs#L50-150)

**AgentDefinition** stores the universe of allowed connectors + hard constraints:
```rust
pub struct AgentDefinition {
    pub id: String,
    pub tenant_id: String,
    pub name: String,                    // e.g. "Sales Ops Agent"
    pub persona: String,                  // System prompt
    pub connectors: Vec<String>,          // ALLOWED UNIVERSE (security boundary)
    pub constraints: Vec<String>,         // Hard rules for ALL roles
    pub memory_ref: String,               // "agent:xxx|pending_roles:[...]"
    pub status: AgentDefinitionStatus,    // Draft, Active, Paused, Archived
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**AgentRole** is the actual executable unit:
```rust
pub struct AgentRole {
    pub id: String,
    pub agent_id: String,
    pub tenant_id: String,
    pub version: u32,                     // Incremented on each save
    pub status: RoleStatus,               // Draft, Testing, Active, Paused, Archived
    pub name: String,                     // e.g. "Lead Enrichment"
    pub trigger: TriggerDef,              // WHEN this role fires
    pub purpose: String,                  // WHAT this role does
    pub role_category: RoleCategory,      // SalesRevOps, CustomerSupport, etc.
    pub execution_guidelines: ExecutionGuidelines,  // Rules + workflow outline
    pub connectors: Vec<String>,          // RELEVANT SUBSET for this role
    pub tools: Vec<String>,               // Explicit tool overrides
    pub output_spec: OutputSpec,          // WHERE output goes
    pub memory_scope: MemoryScope,        // Shared vs role-scoped memory
    pub execution_limits: ExecutionLimits,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**RoleResponsibility** (used in intent):
```rust
pub struct RoleResponsibility {
    pub name: String,           // "Lead Enrichment"
    pub actions: Vec<String>,   // ["lookup company", "fetch employees"]
    pub trigger_hint: String,   // "schedule", "webhook", "manual"
}
```

**TriggerDef** defines HOW each role is triggered:
```rust
pub struct TriggerDef {
    pub trigger_type: TriggerType,  // Webhook, Schedule, UserMessage, Manual, WorkforceEvent
    
    // Webhook fields
    pub source_connector: Option<String>,  // "salesforce"
    pub event_filter: Option<String>,      // "lead_created"
    
    // Schedule fields
    pub cron: Option<String>,      // "0 9 * * 5"
    pub timezone: Option<String>,  // "America/New_York"
    
    // WorkforceEvent fields (cross-agent chaining)
    pub workforce_event_filter: Option<String>,  // "role_name == 'Enrichment' AND status == 'completed'"
    pub input_mapping: Option<serde_json::Value>,  // {"lead_id": "$.output_data.lead_id"}
    pub depends_on_role_id: Option<String>,  // Name or ID of role that must complete first
    
    pub confidence: TriggerConfidence,  // High, Medium, Low
    pub allowed_users: Option<Vec<String>>,
    pub intent_keywords: Option<Vec<String>>,
}
```

### Multi-Role Session Architecture
**File:** [src/agent/plan_mode.rs](src/agent/plan_mode.rs#L1079-1085) (from ARCHITECTURE.md inline comments)
**Lines:** 1079-1085

```
BEFORE SAVE:
  AgentDefinition (DRAFT) + AgentRole #1 (DRAFT)
  
AFTER SAVE:
  AgentDefinition (ACTIVE) 
  │├── memory_ref = "agent:xxx|pending_roles:[role2, role3, ...]"
  │└── AgentRole #1 (ACTIVE)
  
[FRONTEND detects pending_roles in memory_ref]
  
REOPEN PLAN MODE for Role #2:
  Pass existingAgentId to PlanModeChat
  Pre-populate with responsibility name from pending_roles
  
AFTER ROLE #2 SAVE:
  AgentDefinition (ACTIVE)
  │├── memory_ref = "agent:xxx|pending_roles:[role3, ...]"
  │└── AgentRole #1 (ACTIVE)
  └── AgentRole #2 (ACTIVE)
  
REPEAT until pending_roles is empty
  
FINAL STATE:
  AgentDefinition (ACTIVE)
  │├── memory_ref = "agent:xxx" (no pending_roles)
  │├── AgentRole #1 (ACTIVE) - trigger: webhook
  │├── AgentRole #2 (ACTIVE) - trigger: WorkforceEvent after #1
  └── AgentRole #3 (ACTIVE) - trigger: schedule
```

---

## System 2: Runtime Delegation (Execution Time)

### Overview
During agent execution, an executor can call the `delegate` tool to spawn parallel sub-agents. These are temporary, not reusable. Parent agent pauses until all children complete.

### Implementation

#### 1. Delegate Tool Definition
**File:** [src/tools/delegate.rs](src/tools/delegate.rs#L1-100)
**Lines:** 1-100

```rust
pub struct DelegateTool {
    pub store: Arc<PostgresStore>,
    pub workspace_manager: Arc<WorkspaceManager>,
    pub swarm: Arc<Swarm>,
}

impl Tool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }
    fn description(&self) -> &str {
        "Spawn one or more parallel sub-agents to work on independent sub-tasks simultaneously. \
         The current agent pauses until all children complete, then resumes with their combined results."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required(
                "sub_goals",
                "array",
                "List of independent sub-goal strings to execute in parallel.",
            ),
            ParameterSchema::required("tenant_id", "string", "Tenant ID — injected automatically."),
            ParameterSchema::required(
                "parent_agent_id",
                "string",
                "Parent agent ID — injected automatically.",
            ),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let DelegateArgs { tenant_id, parent_id, sub_goals } = parse_delegate_args(&args)?;

        let mut child_ids = Vec::new();
        for sub_goal in &sub_goals {
            let child_id = crate::util::new_id();
            
            // Create workspace for child
            let handle = self.workspace_manager.create(&tenant_id, &child_id).await?;
            let workspace = handle.local_path_str();
            
            // Create child agent state
            let mut child = crate::state::AgentState::new(
                child_id.clone(),
                tenant_id.clone(),
                sub_goal.clone(),
                workspace
            );
            child.parent_agent_id = Some(parent_id.clone());
            
            // Persist to DB
            self.store.upsert_agent(&child).await?;
            
            tracing::info!(parent = %parent_id, child = %child_id, sub_goal = %sub_goal, "child agent spawned");
            
            // Enqueue via Swarm (thread-safe queue)
            if let Err(e) = self.swarm.push(child_id.clone()).await {
                tracing::warn!(child = %child_id, error = %e, "swarm enqueue failed");
            }
            
            child_ids.push(child_id);
        }
        
        Ok(ToolResult::ok(serde_json::json!({
            "child_agent_ids": child_ids,
            "message": format!("{} sub-agents spawned and scheduled", child_ids.len()),
        })))
    }
}
```

#### 2. Agent State Delegation Fields
**File:** [src/state/agent_state.rs](src/state/agent_state.rs#L1-100)
**Lines:** 57-70

```rust
pub struct AgentState {
    // ... other fields ...
    
    /// If this is a child agent, the parent's agent_id.
    pub parent_agent_id: Option<String>,
    
    /// Child agent IDs spawned by this agent (for delegation).
    pub pending_children: Vec<String>,
    
    // ... other fields ...
}

impl AgentState {
    pub fn mark_delegating(&mut self, child_ids: Vec<String>) {
        self.status = AgentStatus::Delegating;
        self.pending_children = child_ids;
        self.updated_at = Utc::now();
    }

    pub fn is_child(&self) -> bool {
        self.parent_agent_id.is_some()
    }
}
```

#### 3. Agent Status Enum
**File:** [src/state/agent_state.rs](src/state/agent_state.rs#L5-25)
**Lines:** 5-25

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Pending,       // Newly created — pre-flight not yet run
    Preflight,     // Pre-flight running
    Clarifying,    // Waiting for clarification answers
    Running,       // Actively executing a step
    Waiting,       // Step complete — waiting for scheduler
    Delegating,    // Waiting for child agents to complete
    Completed,     // Goal successfully completed
    Failed,        // Unrecoverable failure
    Paused,        // Manually paused by user
}
```

#### 4. Agent Loop Delegation Detection
**File:** [src/agent/loop.rs](src/agent/loop.rs#L610-640)
**Method:** `AgentLoop::run_step()` post-execution handling
**Lines:** 610-640

```rust
// ── 8. Delegation check ─────────────────────────────────────────────
for tool_result in &result.tool_results {
    if let Some(arr) = tool_result.output.get("child_agent_ids").and_then(|v| v.as_array()) {
        let child_ids: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        
        if !child_ids.is_empty() {
            // Emit events for each child spawned
            for cid in &child_ids {
                self.event_bus.publish(AgentEvent::ChildSpawned {
                    agent_id: state.id.clone(),
                    child_agent_id: cid.clone(),
                    sub_goal: step.description.clone(),
                });
            }
            
            // Advance step and mark as delegating
            state.advance_step();
            state.mark_delegating(child_ids.clone());
            
            // Return to worker pool — will be rescheduled when children complete
            return Ok(StepOutcome::Delegating { child_ids });
        }
    }
}
```

#### 5. Event Emission
**File:** [src/events/bus.rs](src/events/bus.rs#L171-180)
**Lines:** 171-180

```rust
pub enum AgentEvent {
    // ... other events ...
    
    // ── Delegation ─────────────────────────────────────────────────────
    ChildSpawned {
        agent_id: String,
        child_agent_id: String,
        sub_goal: String,
    },
    ChildrenComplete {
        agent_id: String,
        child_ids: Vec<String>,
    },
    
    // ... other events ...
}
```

---

## Data Structure Hierarchy

### Plan Mode Roles (Configuration)
```
PlanModeSession (transient, not saved)
  ├── draft_agent: AgentDefinition
  │   ├── id, tenant_id, name
  │   ├── connectors: ["salesforce", "slack", "web_fetch"]  ← allowed universe
  │   ├── constraints: ["never delete records"]
  │   └── memory_ref: "agent:xxx|pending_roles:[{...}, {...}]"  ← stashed roles
  │
  ├── draft_role: AgentRole
  │   ├── id, agent_id, tenant_id, version
  │   ├── name: "Lead Enrichment"
  │   ├── trigger: TriggerDef
  │   │   ├── trigger_type: Webhook
  │   │   ├── source_connector: "salesforce"
  │   │   ├── event_filter: "lead_created"
  │   │   └── confidence: High
  │   ├── purpose: "Enrich inbound leads and prepare CRM"
  │   ├── connectors: ["salesforce", "web_fetch"]  ← subset of agent connectors
  │   ├── execution_guidelines:
  │   │   ├── rules: ["Always lookup company"]
  │   │   ├── failure_handling: ["If lookup fails, skip"]
  │   │   ├── workflow_outline: [
  │   │   │   {description: "Fetch lead data", tool: "salesforce", ...},
  │   │   │   {description: "Look up company", tool: "web_fetch", ...},
  │   │   │   {description: "Update record", tool: "salesforce", ...}
  │   │   │ ]
  │   │   └── completion_criteria: ["Record updated"]
  │   ├── output_spec: {destination: "connector_record", format: "json"}
  │   └── status: Draft → Active (on save)
  │
  ├── intent_cache: serde_json::Value
  │   ├── responsibilities: [{name, actions, trigger_hint}, ...]
  │   ├── multi_role_suggested: true
  │   ├── multi_role_reason: "..."
  │   └── ...other extracted fields...
  │
  └── pending_steps: Vec<ClarificationStep>
      └── [queue of remaining questions to ask]

↓ save() ↓

DATABASE:
  agent_definitions
    └── {id, tenant_id, name, connectors, constraints, memory_ref: "agent:xxx|pending_roles:[...]", status}
  
  agent_roles
    └── {id, agent_id, tenant_id, version, name, trigger, purpose, connectors, tools, execution_guidelines, status}
```

### Runtime Delegation (Execution)
```
parent_state: AgentState
  ├── id: "agent-123"
  ├── goal: "Enrich 100 leads"
  ├── status: Delegating (after delegate tool called)
  ├── pending_children: ["child-1", "child-2", "child-3", "child-4", "child-5"]
  ├── parent_agent_id: None
  └── ...

    ↓ calls `delegate` tool ↓

child_states: Vec<AgentState> (NEW, not from roles)
  ├── {id: "child-1", goal: "Enrich leads 1-20", parent_agent_id: "agent-123", status: Pending}
  ├── {id: "child-2", goal: "Enrich leads 21-40", parent_agent_id: "agent-123", status: Pending}
  ├── {id: "child-3", goal: "Enrich leads 41-60", parent_agent_id: "agent-123", status: Pending}
  ├── {id: "child-4", goal: "Enrich leads 61-80", parent_agent_id: "agent-123", status: Pending}
  └── {id: "child-5", goal: "Enrich leads 81-100", parent_agent_id: "agent-123", status: Pending}

[All enqueued to swarm, executed in parallel by worker pool]

    ↓ on completion ↓

parent_state: AgentState
  ├── status: Waiting (rescheduled)
  ├── pending_children: [] (cleared)
  ├── metadata: {child_results: [...combine all outputs...]}
  └── [resumes next step]
```

---

## Trigger Types & Multi-Role Orchestration

### TriggerType Enum
**File:** [src/agent/definition.rs](src/agent/definition.rs#L800-860)

```rust
pub enum TriggerType {
    /// Manual on-demand activation
    Manual,
    
    /// Triggered by a user message
    UserMessage,
    
    /// Webhook from a connector (e.g., Salesforce lead created)
    Webhook,
    
    /// Scheduled execution (cron)
    Schedule,
    
    /// Cross-agent event-driven chaining
    WorkforceEvent,
}
```

### Cross-Agent Role Chaining Example

**Role 1: Lead Enrichment**
```rust
TriggerDef {
    trigger_type: Webhook,
    source_connector: Some("salesforce"),
    event_filter: Some("lead_created"),
    ...
}
```

**Role 2: Send Notification (runs after Role 1)**
```rust
TriggerDef {
    trigger_type: WorkforceEvent,
    workforce_event_filter: Some("role_name == 'Lead Enrichment' AND status == 'completed'"),
    input_mapping: Some({"lead_id": "$.output_data.lead_id"}),
    ...
}
```

---

## Integration Points & Gaps

### What's Implemented
✅ Plan mode extracts `responsibilities` from LLM  
✅ User can choose to split roles  
✅ Remaining roles stashed in `memory_ref`  
✅ Single-agent multi-role persistence to DB  
✅ Delegate tool spawns child agents at runtime  
✅ Child agents tracked in `pending_children`  
✅ Delegation events published to SSE  
✅ WorkforceEvent trigger type defined  

### What's NOT Found in Backend
❌ Frontend parsing of `|pending_roles:` from agent memory_ref  
❌ Frontend logic to re-open plan mode for Role #2  
❌ Scheduler logic to wait for all `pending_children` before waking parent  
❌ Result aggregation logic — how child outputs combine  
❌ WorkforceEvent bus implementation — how roles listen for completion  
❌ Cross-agent output mapping — how `input_mapping` is applied  

### Critical Gaps
1. **Multi-role sequencing**: No visible implementation of how Role 2 waits for Role 1 completion
2. **Parent-child result collection**: Delegation logic marks status `Delegating` but unclear how parent **resumes** with child results
3. **Workflow event subscription**: `sync_subscriptions_for_role()` called in save but implementation not traced
4. **Plan mode frontend**: How does UI detect `pending_roles` and trigger role #2 setup?

---

## Decision Tree: Split vs Keep Single

### Plan Mode Decision
```
User input: "I need an agent to..."
    ↓
IntentExtractor LLM pass 1 (quick)
    ↓
Build detailed capability context
    ↓
IntentExtractor LLM pass 2 (refined)
    ↓
Extract .responsibilities[] and .multi_role_suggested
    ↓
In CapturingClarifications phase:
  Generate steps including: StepField::RoleSplit
    ↓
  User sees:
    "I found 3 distinct responsibilities:
     A - Lead Enrichment (webhook)
     B - Output Formatting (schedule)
     C - Notification Sending (manual)
     
     A - Configure as one role
     B - Split into separate roles"
    ↓
  User: "B"
    ↓
  parse_and_apply() extracts responsibilities[1:]
    ↓
  Stash in memory_ref
    ↓
  Show responsibility names:
    "I'll configure 3 roles.
     Starting with: **Lead Enrichment**
     Then: Output Formatting, Notification Sending
     
     Current role trigger?..."
```

---

## Code Snippets by Functionality

### 1. Extract Responsibilities
**File:** src/agent/plan_mode.rs, Line 70-150  
**Responsibility Detection.**

Extracted from `intent["responsibilities"]` array returned by LLM.

### 2. Suggest Split
**File:** src/agent/plan_mode_steps.rs, Line 315-350  
**Role Split Decision.**

```rust
// Inside parse_and_apply() for StepField::RoleSplit
if wants_split && responsibilities.len() > 1 {
    *pending_roles_sink = Some(remaining);
}
```

### 3. Stash & Save
**File:** src/agent/plan_mode.rs, Line 1300-1330 + Line 2450-2500  
**Multi-Role Persistence.**

```rust
// Stash
session.draft_agent.memory_ref = format!("{}|pending_roles:{}", meta, serde_json::to_string(&remaining)?);

// Save
store.upsert_agent_definition(&agent).await?;
store.upsert_agent_role(&role).await?;
```

### 4. Spawn Children at Runtime
**File:** src/tools/delegate.rs, Line 50-90  
**Child Agent Creation.**

```rust
for sub_goal in &sub_goals {
    let child = AgentState::new(child_id, tenant_id, sub_goal, workspace);
    child.parent_agent_id = Some(parent_id);
    store.upsert_agent(&child).await?;
    swarm.push(child_id).await?;
}
```

### 5. Detect Delegation in Executor
**File:** src/agent/loop.rs, Line 610-640  
**Delegation Recognition.**

```rust
if let Some(arr) = tool_result.output.get("child_agent_ids") {
    let child_ids: Vec<String> = /* extract */;
    state.mark_delegating(child_ids.clone());
    return Ok(StepOutcome::Delegating { child_ids });
}
```

---

## Summary Table

| Aspect | Plan Mode | Delegation |
|--------|-----------|-----------|
| **When** | Configuration time | Runtime execution |
| **Trigger** | User input "split roles" | Executor calls `delegate` tool |
| **Scope** | Reusable roles (saved to DB) | Temporary sub-agents (not saved) |
| **Data** | ResponsibilityDef[] → AgentRole | sub_goals: String[] → AgentState |
| **State** | ActvieDefinitionStatus + RoleStatus | AgentStatus::Delegating |
| **Parent-Child** | Multiple roles in one agent | Parent pauses, children run parallel |
| **Orchestration** | Front-end detects pending_roles | Scheduler waits for pending_children |
| **Fully Implemented** | ~70% | ~50% |

---

## File Reference Quick Links

| File | Purpose | Key Lines |
|------|---------|-----------|
| [src/agent/plan_mode.rs](src/agent/plan_mode.rs) | Plan mode orchestration | 70-150 (extract), 1300-1330 (stash), 2450-2500 (save) |
| [src/agent/plan_mode_steps.rs](src/agent/plan_mode_steps.rs) | Step parsing & decision logic | 230-350 (role split) |
| [src/agent/definition.rs](src/agent/definition.rs) | Data models | 50-150 (AgentDefinition), 100-250 (AgentRole) |
| [src/agent/planner.rs](src/agent/planner.rs) | Plan structure & deterministic planning | 35-75 (Plan, PlannedStep) |
| [src/state/agent_state.rs](src/state/agent_state.rs) | Agent execution state | 5-200 (AgentState, delegation fields) |
| [src/agent/loop.rs](src/agent/loop.rs) | Execution orchestration & delegation check | 610-640 (delegation detection) |
| [src/tools/delegate.rs](src/tools/delegate.rs) | Runtime child agent spawning | 1-100 (DelegateTool) |
| [src/events/bus.rs](src/events/bus.rs) | SSE events | 171-180 (ChildSpawned, ChildrenComplete) |

