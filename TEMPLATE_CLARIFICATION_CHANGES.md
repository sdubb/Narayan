# Template Clarification Fix - Implementation Summary

## What Was Fixed

**Problem:** Template fast-path in plan mode asked only hardcoded questions instead of using the full clarification pipeline from `generate_steps()`.

**Solution:** Templates now go through the *full* clarification pipeline but filter to only the steps explicitly listed in `template.ask_steps`.

## Files Modified

### `src/api/routes.rs`

**Change 1: Template initialization (lines ~3197-3286)**
- **Old approach:** Used `build_template_clarification_steps()` function with hardcoded question mapping
- **New approach:** Calls `crate::agent::plan_mode_steps::generate_steps()` to get all possible questions, then filters by `template.ask_steps`

**Before:**
```rust
let step_names: Vec<&str> = tmpl.ask_steps.iter().copied().collect();
let pending = build_template_clarification_steps(tmpl, &step_names);
session.pending_steps = pending.iter().filter_map(|s| serde_json::to_value(s).ok()).collect();
```

**After:**
```rust
let all_steps = crate::agent::plan_mode_steps::generate_steps(
    &intent,
    role.role_category.as_str(),
    &installed,
    &existing_role_names,
);

let template_ask_set: std::collections::HashSet<&str> = tmpl.ask_steps.iter().copied().collect();
let steps_to_ask: Vec<_> = all_steps
    .into_iter()
    .filter(|step| template_ask_set.contains(step.id.as_str()))
    .collect();

session.pending_steps = steps_to_ask.iter().filter_map(|s| serde_json::to_value(s).ok()).collect();
```

**Change 2: Removed deprecated function (lines ~3338-3415)**
- **Deleted:** `build_template_clarification_steps()` function
- **Reason:** No longer needed; the real pipeline generates semantic questions based on intent and category
- **Impact:** Eliminated 80+ lines of hardcoded question mappings

## How Templates Work Now

### Template Definition
```rust
pub struct RoleTemplate {
    name: &'static str,
    category: &'static str,
    ask_steps: &'static [&'static str],  // e.g. ["output_dest", "approval_threshold"]
    build_role: fn(...) -> AgentRole,
    // ...
}
```

### Execution Flow
```
1. User selects template
   ↓
2. Template pre-populates: connectors, role category, execution guidelines
   ↓
3. Call generate_steps() with the FULL intent
   ↓
4. Get all 10-15 possible clarification questions
   ↓
5. Filter: Keep only steps whose ID is in template.ask_steps
   ↓
6. Show filtered questions to user
```

### Key Insight
- Template's `ask_steps` acts as an **inclusion filter**, not a source of questions
- Questions come from the real `generate_steps()` pipeline
- Questions adapt to the intent and category automatically

## Example

**Template:** `investor_update`

```rust
ClarificationStep::new(
    "investor_email",
    "What email address(es) should receive the investor update draft?",
    StepField::OutputDestination,
)
```

**Old behavior:** Looked up "investor_email" in a hardcoded match expression
**New behavior:** Calls `generate_steps()` which includes this step, then filters for it

## Backward Compatibility

✅ All existing templates work unchanged
- They still specify the same `ask_steps`
- Questions are now semantic instead of hardcoded
- No changes to template definitions required

## Benefits

| Aspect | Before | After |
|--------|--------|-------|
| Question source | Hardcoded mapping | Real pipeline |
| New questions | Require code change | Automatic |
| Question quality | Static text | Semantic, context-aware |
| Extensibility | Low (hardcoded) | High (pipeline-driven) |
| Test coverage | Per-mapping | Per-category/intent |

---

**Implementation Date:** March 30, 2026  
**Status:** ✅ Complete  
**Risk Level:** Low (localized change, backward compatible)
