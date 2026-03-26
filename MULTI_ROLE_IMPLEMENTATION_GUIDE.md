# Multi-Role Agent Implementation Guide

**Goal:** Make plan mode goal splitting fully functional so job descriptions with multiple roles create all roles autonomously with proper sequencing.

---

## Architecture Overview

### Two-Phase Multi-Role System

```
PHASE 1: CONFIGURATION (Plan Mode)
┌─────────────────────────────────────────────────────────┐
│ User writes job: "Process leads, enrich, send emails"   │
│                                                           │
│ LLM detects 3 responsibilities:                          │
│  - Lead Processor (webhook trigger)                      │
│  - Lead Enricher (runs after Processor)                  │
│  - Email Notifier (manual trigger)                       │
│                                                           │
│ User chooses: "B - Split into separate roles"           │
│                                                           │
│ Backend stashes: memory_ref = "agent:xxx|pending_roles:[Enricher, Notifier]"
│ Backend saves: Role #1 (Processor)                      │
│                                                           │
│ Frontend detects pending_roles:                         │
│  Shows badge: "2 more roles to configure"               │
│  User clicks "Configure next role"                      │
│                                                           │
│ Plan mode re-opens with Role #2 (Enricher)              │
│  ...questions...                                         │
│  Saves: Role #2 (Enricher)                              │
│  memory_ref = "agent:xxx|pending_roles:[Notifier]"     │
│                                                           │
│ Repeat for Role #3 (Notifier)                           │
│  memory_ref = "agent:xxx" (empty)                       │
│                                                           │
│ FINAL STATE:                                             │
│  AgentDefinition: {id, name, connectors, ...}           │
│  AgentRole #1: {trigger: Webhook, ...}                  │
│  AgentRole #2: {trigger: WorkforceEvent, ...}           │
│  AgentRole #3: {trigger: Manual, ...}                   │
└─────────────────────────────────────────────────────────┘

PHASE 2: EXECUTION (Workforce Events)
┌─────────────────────────────────────────────────────────┐
│ Webhook fires: "lead_created" from Salesforce           │
│ Scheduler creates agent for Role #1                     │
│                                                           │
│ Role #1 executes:                                        │
│  - Reads lead data                                       │
│  - Validates required fields                            │
│  - COMPLETES → emits RoleCompleted event                │
│                                                           │
│ WorkforceEvent subscriber listening for Role #1 completion:
│  Fires and creates agent for Role #2                    │
│  Passes input_mapping: {lead_id: "$.output_data.id"}    │
│                                                           │
│ Role #2 executes:                                        │
│  - Fetches lead_id from parent output                    │
│  - Enriches data via APIs                               │
│  - COMPLETES → emits RoleCompleted event                │
│                                                           │
│ Role #3 is manual trigger registered but can fire       │
│  On user request via API                                │
│                                                           │
│ Result: Single user action → 3 roles executed sequentially
│         with full audit trail                           │
└─────────────────────────────────────────────────────────┘
```

---

## Implementation Steps

### STEP 1: Frontend - Parse `pending_roles` from Agent

**File:** `narayan-v5/src/api/index.js`  
**Current:** No pending_roles parsing

**Add:**
```javascript
export const agents = {
  // ... existing methods ...
  
  /// Parse agent.memory_ref for pending roles
  getPendingRoles: (agent) => {
    if (!agent?.memory_ref) return [];
    
    const match = agent.memory_ref.match(/\|pending_roles:(\[.*?\])/);
    if (!match) return [];
    
    try {
      return JSON.parse(match[1]);
    } catch {
      return [];
    }
  },
  
  /// Check if multi-role job is incomplete
  hasMoreRolesToConfigure: (agent) => {
    const pending = agents.getPendingRoles(agent);
    return pending.length > 0;
  }
};
```

**Why:** Allows UI to detect when agent has more roles waiting to be configured.

---

### STEP 2: Frontend - Show "Complete Setup" Badge

**File:** `narayan-v5/src/pages/AgentDetailPage.jsx` (or wherever agent is displayed)

**Add:**
```jsx
import { agents } from '@/api';

export function AgentDetailHeader({ agent }) {
  const pending = agents.getPendingRoles(agent);
  const hasMore = agents.hasMoreRolesToConfigure(agent);
  
  return (
    <div>
      <h1>{agent.name}</h1>
      
      {hasMore && (
        <div className="badge badge-warning">
          {pending.length} more role(s) to configure
        </div>
      )}
      
      {hasMore && (
        <button 
          onClick={() => openPlanModeForNextRole(agent.id, pending[0])}
          className="btn btn-primary"
        >
          Configure {pending[0].name}
        </button>
      )}
    </div>
  );
}
```

---

### STEP 3: Frontend - Re-open Plan Mode for Next Roles

**File:** `narayan-v5/src/pages/PlanModePage.jsx` (or planning component)

**Modify to accept `existingAgentId` query param:**
```jsx
import { useSearchParams } from 'react-router-dom';

export function PlanModePage() {
  const [searchParams] = useSearchParams();
  const existingAgentId = searchParams.get('agent_id');
  const preselectedRole = searchParams.get('role');
  
  // If editing existing agent: fetch agent definition
  if (existingAgentId) {
    const { data: agent } = useQuery(['agent', existingAgentId], 
      () => agents.getById(existingAgentId));
    
    // Skip to role selection, showing next pending role
    // OR skip to first question with role name pre-filled
  }
  
  return (
    <PlanModeChat 
      existingAgentId={existingAgentId}
      preselectedRoleName={preselectedRole}
    />
  );
}

// Link:
const openPlanModeForNextRole = (agentId, responsibility) => {
  window.location.href = `/plan-mode?agent_id=${agentId}&role=${responsibility.name}`;
};
```

---

### STEP 4: Backend - Modify PlanModeManager to Handle Existing Agent

**File:** `src/agent/plan_mode.rs`

**Modify `PlanModeManager::new()` and `start_session()`:**
```rust
pub struct PlanModeManager {
    // ... existing fields ...
}

impl PlanModeManager {
    /// Start plan mode for existing agent (multi-role)
    pub async fn start_session_for_existing_role(
        &self,
        agent_id: &str,
        tenant_id: &str,
        role_name_hint: Option<&str>,
    ) -> anyhow::Result<PlanModeSession> {
        // Load existing agent definition
        let agent = self.store.get_agent_definition(tenant_id, agent_id).await?;
        
        // Extract next pending role
        let next_role = self.extract_next_pending_role(&agent)?;
        
        // Create session with pre-filled data
        let mut session = PlanModeSession {
            draft_agent: agent.clone(),
            draft_role: AgentRole {
                id: new_id(),
                agent_id: agent.id.clone(),
                tenant_id: agent.tenant_id.clone(),
                name: next_role.name.clone(),
                // Start with responsibility pre-filled
                ..Default::default()
            },
            ..Default::default()
        };
        
        // Skip responsibility extraction, go straight to trigger question
        session.next_step = StepField::Trigger;
        
        Ok(session)
    }
    
    fn extract_next_pending_role(&self, agent: &AgentDefinition) -> anyhow::Result<RoleResponsibility> {
        let pending = Self::parse_pending_roles(&agent.memory_ref)?;
        pending.first()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No pending roles found"))
    }
    
    fn parse_pending_roles(memory_ref: &str) -> anyhow::Result<Vec<RoleResponsibility>> {
        if let Some(pos) = memory_ref.find("|pending_roles:") {
            let json_str = &memory_ref[pos + 15..]; // Skip "|pending_roles:"
            Ok(serde_json::from_str(json_str).unwrap_or_default())
        } else {
            Ok(vec![])
        }
    }
}
```

**API Endpoint:** Add to `src/api/routes.rs`
```rust
Router::new()
    .route("/agents/:id/plan-mode/continue", post(plan_mode_continue))
    
pub async fn plan_mode_continue(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let manager = state.plan_mode_manager.clone();
    match manager.start_session_for_existing_role(&agent_id, &tenant.tenant_id, None).await {
        Ok(session) => Json(session),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()),
    }
}
```

---

### STEP 5: Backend - Scheduler Checks `pending_children` Before Waking Parent

**File:** `src/scheduler/scheduler.rs`

**Modify wakeup logic:**
```rust
async fn should_wake_agent(
    &self,
    agent: &AgentState,
    store: &Arc<PostgresStore>,
) -> anyhow::Result<bool> {
    // If agent is Delegating, check if all children completed
    if agent.status == AgentStatus::Delegating {
        if agent.pending_children.is_empty() {
            return Ok(false); // No children, shouldn't be delegating
        }
        
        // Check all children
        let mut all_done = true;
        for child_id in &agent.pending_children {
            if let Ok(child) = store.get_agent(child_id).await {
                match child.status {
                    AgentStatus::Completed | AgentStatus::Failed => {
                        // Child done, continue checking
                    }
                    _ => {
                        // Still running
                        all_done = false;
                        break;
                    }
                }
            }
        }
        
        return Ok(all_done); // Wake parent only if ALL children done
    }
    
    // Normal wakeup conditions for non-delegating agents
    Ok(matches!(agent.status, 
        AgentStatus::Waiting | AgentStatus::Paused | AgentStatus::Clarifying
    ))
}
```

---

### STEP 6: Backend - Aggregate Child Results

**File:** `src/scheduler/scheduler.rs` or `src/worker/worker.rs`

**After child completes, aggregate into parent:**
```rust
async fn on_child_completed(
    &self,
    parent_id: &str,
    child_id: &str,
    child_result: &AgentState,
    store: &Arc<PostgresStore>,
) -> anyhow::Result<()> {
    let mut parent = store.get_agent(parent_id).await?;
    
    // Remove child from pending list
    parent.pending_children.retain(|id| id != child_id);
    
    // Collect child's final output
    let child_output = serde_json::json!({
        "child_id": child_id,
        "status": child_result.status.to_string(),
        "output": child_result.metadata.get("final_output"),
        "steps": child_result.step_count,
        "tokens_used": child_result.metadata.get("tokens_used"),
    });
    
    // Append to parent's metadata
    if let Some(ref mut children) = parent.metadata.get_mut("delegated_children") {
        if let Some(arr) = children.as_array_mut() {
            arr.push(child_output);
        }
    } else {
        parent.metadata.insert("delegated_children", serde_json::json!([child_output]));
    }
    
    parent.updated_at = Utc::now();
    store.upsert_agent(&parent).await?;
    
    // Emit event
    self.event_bus.publish(AgentEvent::ChildrenComplete {
        agent_id: parent_id.to_string(),
        child_ids: vec![child_id.to_string()],
    });
    
    Ok(())
}
```

---

### STEP 7: Backend - Emit `RoleCompleted` Event

**File:** `src/events/bus.rs`

**Add event type:**
```rust
pub enum AgentEvent {
    // ... existing ...
    
    RoleCompleted {
        agent_id: String,
        agent_definition_id: String,
        role_id: String,
        role_name: String,
        output_data: serde_json::Value,
    },
    
    // ... existing ...
}
```

**File:** `src/agent/loop.rs`

**Emit when role completes:**
```rust
// In AgentLoop::run_step(), when step statuses reach terminal:
if agent.step_index >= plan.steps.len() {
    // Agent completed this role
    
    if let Some(role_id) = agent.metadata.get("current_role_id").and_then(|v| v.as_str()) {
        self.event_bus.publish(AgentEvent::RoleCompleted {
            agent_id: agent.id.clone(),
            agent_definition_id: agent.definition_id.clone(),
            role_id: role_id.to_string(),
            role_name: agent.metadata.get("role_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            output_data: agent.metadata.get("final_output").cloned().unwrap_or(serde_json::json!({})),
        });
    }
}
```

---

### STEP 8: Backend - Implement WorkforceEvent Subscription

**File:** `src/events/workforce.rs` (new or modify existing)

**Track role completion subscriptions:**
```rust
pub struct WorkforceEventSubscriber {
    pub store: Arc<PostgresStore>,
    pub swarm: Arc<Swarm>,
}

impl WorkforceEventSubscriber {
    /// Register role to listen for another role's completion
    pub async fn subscribe_to_role_completion(
        &self,
        listening_role_id: &str,
        target_role_name: &str,
        input_mapping: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        let subscription = RoleSubscription {
            id: new_id(),
            listening_role_id: listening_role_id.to_string(),
            event_type: "role_completed".to_string(),
            filter: format!("role_name == '{}'", target_role_name),
            input_mapping,
            created_at: Utc::now(),
        };
        
        self.store.upsert_subscription(&subscription).await?;
        Ok(())
    }
    
    /// Called by event bus when RoleCompleted event fires
    pub async fn on_role_completed(
        &self,
        completed_role_name: &str,
        output_data: serde_json::Value,
    ) -> anyhow::Result<()> {
        // Find all subscriptions listening for this role
        let subscriptions = self.store.get_subscriptions_for_role(completed_role_name).await?;
        
        for sub in subscriptions {
            // Create new agent for listening role
            let role = self.store.get_agent_role(&sub.listening_role_id).await?;
            
            // Apply input mapping
            let mapped_input = apply_input_mapping(
                sub.input_mapping.as_ref(),
                &output_data,
            )?;
            
            // Create agent execution
            let mut agent = AgentState::new(
                new_id(),
                role.tenant_id.clone(),
                format!("Execute role: {}", role.name),
                workspace_path(),
            );
            agent.metadata.insert("role_id", serde_json::Value::String(role.id.clone()));
            agent.metadata.insert("role_name", serde_json::Value::String(role.name.clone()));
            agent.metadata.insert("input", mapped_input);
            
            self.store.upsert_agent(&agent).await?;
            self.swarm.push(agent.id.clone()).await?;
        }
        
        Ok(())
    }
}

fn apply_input_mapping(
    mapping: Option<&serde_json::Value>,
    output: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    if let Some(m) = mapping {
        // Mapping like: {"lead_id": "$.output_data.lead_id"}
        let mut result = serde_json::json!({});
        if let Some(obj) = m.as_object() {
            for (key, path_value) in obj {
                if let Some(path) = path_value.as_str() {
                    let value = extract_json_path(output, path)?;
                    result[key] = value;
                }
            }
        }
        Ok(result)
    } else {
        Ok(output.clone())
    }
}
```

---

### STEP 9: Wire Up Subscriptions on Role Save

**File:** `src/agent/plan_mode.rs`

**Modify `save()`:**
```rust
pub async fn save(&self, mut session: PlanModeSession) -> Result<(AgentDefinition, AgentRole)> {
    let mut agent = session.draft_agent.clone();
    let mut role = session.draft_role.clone();
    
    // ... existing save logic ...
    
    role.status = RoleStatus::Active;
    self.store.upsert_agent_role(&role).await?;
    
    // NEW: If this role has WorkforceEvent trigger, subscribe to parent completion
    if role.trigger.trigger_type == TriggerType::WorkforceEvent {
        if let Some(filter) = &role.trigger.workforce_event_filter {
            // Extract role name from filter: "role_name == 'Lead Enrichment'"
            if let Some(parent_name) = extract_role_name_from_filter(filter) {
                let subscriber = WorkforceEventSubscriber::new(self.store.clone(), self.swarm.clone());
                subscriber.subscribe_to_role_completion(
                    &role.id,
                    &parent_name,
                    role.trigger.input_mapping.clone(),
                ).await?;
            }
        }
    }
    
    // Update agent memory_ref to remove this role from pending
    if let Some(pending) = session.pending_roles.take() {
        if !pending.is_empty() {
            let remaining = pending[1..].to_vec();
            agent.memory_ref = if remaining.is_empty() {
                format!("agent:{}", agent.id)
            } else {
                format!("agent:{}|pending_roles:{}", agent.id, serde_json::to_string(&remaining)?)
            };
        }
    }
    
    agent.updated_at = Utc::now();
    self.store.upsert_agent_definition(&agent).await?;
    
    Ok((agent, role))
}
```

---

### STEP 10: End-to-End Test

**Test File:** `tests/integration/multi_role_agent.rs`

```rust
#[tokio::test]
async fn test_multi_role_agent_full_flow() {
    let (state, tenant_id) = setup_test().await;
    
    // Step 1: Start plan mode → suggests 3 roles
    let session1 = state.plan_mode_manager
        .start_new_session(tenant_id, "Process leads, enrich, send emails")
        .await
        .unwrap();
    
    // Step 2: User chooses to split
    let (agent1, role1) = state.plan_mode_manager
        .save_with_split(session1, true)
        .await
        .unwrap();
    
    // Verify role 1 saved
    assert_eq!(role1.name, "Lead Processor");
    assert_eq!(role1.trigger.trigger_type, TriggerType::Webhook);
    
    // Verify pending roles stashed
    let pending = agents.getPendingRoles(&agent1);
    assert_eq!(pending.len(), 2);
    
    // Step 3: Frontend detects pending, opens plan mode for role 2
    let session2 = state.plan_mode_manager
        .start_session_for_existing_role(&agent1.id, tenant_id, Some("Lead Enricher"))
        .await
        .unwrap();
    
    let (agent2, role2) = state.plan_mode_manager.save(session2).await.unwrap();
    assert_eq!(role2.name, "Lead Enricher");
    assert_eq!(role2.trigger.trigger_type, TriggerType::WorkforceEvent);
    
    // Verify subscription created
    let subs = state.store.get_subscriptions_for_role("Lead Processor").await.unwrap();
    assert_eq!(subs.len(), 1);
    
    // Step 4: Simulate webhook trigger → role 1 executes
    let (agent1_state, _) = state.agent_manager
        .create_goal_from_connector("webhook", "lead_created", lead_data.clone())
        .await
        .unwrap();
    
    // Step 5: Run agent
    let result = state.agent_loop.run_to_completion(&agent1_state).await.unwrap();
    assert_eq!(result.status, AgentStatus::Completed);
    
    // Emit RoleCompleted event
    state.event_bus.publish(AgentEvent::RoleCompleted {
        agent_id: agent1_state.id.clone(),
        agent_definition_id: agent1.id.clone(),
        role_id: role1.id.clone(),
        role_name: role1.name.clone(),
        output_data: result.metadata.get("final_output").cloned().unwrap_or_default(),
    });
    
    // Step 6: WorkforceEvent listener fires → role 2 created
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let agents_created = state.store.list_agents_for_definition(&agent1.id).await.unwrap();
    assert_eq!(agents_created.len(), 2); // Original + newly created for role 2
    
    // Verify role 2 got input from role 1
    let agent2_state = &agents_created[1];
    let lead_id = agent2_state.metadata.get("input")
        .and_then(|v| v.get("lead_id"))
        .and_then(|v| v.as_str());
    assert!(lead_id.is_some());
    
    println!("✅ Multi-role agent flow complete!");
}
```

---

## Verification Checklist

After implementing all steps:

- [ ] Plan mode suggests splitting into roles
- [ ] User chooses "B - split"
- [ ] Pending roles stashed in `memory_ref`
- [ ] Frontend detects `|pending_roles:` and shows badge
- [ ] "Configure next role" button works
- [ ] Plan mode reopens with existing agent ID
- [ ] Role #2 configurable without re-describing everything
- [ ] Role #2 saved to DB with WorkforceEvent trigger
- [ ] Webhook fires → Role #1 creates and executes
- [ ] Role #1 completes → RoleCompleted event emitted
- [ ] WorkforceEvent subscriber detects completion
- [ ] Role #2 automatically created with input from Role #1
- [ ] Role #2 executes with mapped input data
- [ ] Complete audit trail shows all 3 executions linked
- [ ] Each role has its own agent record with proper metadata

---

## Files Changed Summary

| File | Changes | Lines |
|------|---------|-------|
| `narayan-v5/src/api/index.js` | Add `getPendingRoles()`, `hasMoreRolesToConfigure()` | +20 |
| `narayan-v5/src/pages/AgentDetailPage.jsx` | Show badge + button for next role | +30 |
| `narayan-v5/src/pages/PlanModePage.jsx` | Accept `agent_id` query param | +40 |
| `src/agent/plan_mode.rs` | Add `start_session_for_existing_role()`, parse pending | +80 |
| `src/api/routes.rs` | Add `/agents/:id/plan-mode/continue` endpoint | +25 |
| `src/scheduler/scheduler.rs` | Check `pending_children` before waking | +50 |
| `src/worker/worker.rs` | Aggregate child results on completion | +60 |
| `src/events/bus.rs` | Add `RoleCompleted` event | +20 |
| `src/agent/loop.rs` | Emit `RoleCompleted` on agent completion | +30 |
| `src/events/workforce.rs` | Implement subscription + event handling | +150 |
| `src/agent/plan_mode.rs` (save) | Wire subscriptions for WorkforceEvent roles | +40 |
| **TOTAL** | | **~545 lines** |

---

## Priority Order

1. **Must have first:** Steps 1-4 (Frontend + Plan mode re-opening)
2. **Then:** Steps 5-7 (Scheduler + child result aggregation)
3. **Then:** Steps 8-9 (WorkforceEvent implementation)
4. **Finally:** Step 10 (Integration testing)

Once Steps 1-4 are done, users can configure multi-role agents. Steps 5-9 enable autonomous execution with proper sequencing.

