# Runtime Error Handling Improvements

## Overview

The Narayan runtime execution pipeline now provides granular error classification to enable smarter retry strategies and better failure visibility. Instead of collapsing all failures into a generic `Failed(String)` variant, `StepOutcome` now distinguishes between multiple error categories.

## StepOutcome Variants

### `Continue { delay_secs: i64 }`
Step executed successfully, move to next step.

### `TransientError { reason: String, retry_after_secs: u64 }`
**Characteristics:**
- Connection timeouts
- Service temporarily unavailable (503, 504)
- Connection reset by peer
- Temporary service disruptions

**Recovery Strategy:**
- Attempt retry WITH exponential backoff
- Evaluator LLM NOT called (deterministic recovery)
- Time to wait: Use `retry_after_secs` or exponential backoff (10s, 20s, 40s)
- Max retries: Configured by evaluator fast-path (typically 3 attempts)

**Examples:**
```
"step 2 aborted: connection timeout to stripe api"
"step 3 aborted: service unavailable (503)"
"step 4 aborted: connection reset by peer"
```

### `PermanentError { reason: String }`
**Characteristics:**
- Invalid/missing credentials
- Tool/connector not found
- Invalid schema or malformed request
- Authentication failures
- OAuth token expired

**Recovery Strategy:**
- Do NOT retry (LLM cannot fix configuration errors)
- Escalate to human immediately
- Requires user action: Add credentials, fix schema, renew tokens, etc.

**Examples:**
```
"step 1 aborted: stripe api key not found in credentials"
"step 5 aborted: invalid schema — missing required field 'amount'"
"step 3 aborted: authentication failed — oauth token expired"
```

### `PolicyViolation { reason: String }`
**Characteristics:**
- Permission denied
- Tool blocked by plane guard
- Policy engine rejection
- Access denied to resource
- Role/scope violations

**Recovery Strategy:**
- Escalate to admin/role reviewer
- Requires explicit approval or role permission update
- Cannot proceed until permissions are updated

**Examples:**
```
"step 2 aborted: Policy blocked tool 'delete_customer' for this role"
"step 4 aborted: Plane guard rejected — insufficient permissions"
"step 6 aborted: Access denied to PII fields"
```

### `RateLimited { retry_after_secs: u64, reason: String }`
**Characteristics:**
- HTTP 429 Too Many Requests
- Rate limit exceeded
- Quota exhausted temporarily

**Recovery Strategy:**
- Backoff for specified retry_after_secs
- Retry after delay
- Evaluator LLM NOT called (deterministic recovery)
- Consider queuing if high concurrency

**Examples:**
```
"step 3 aborted: rate limit 429 — retry after 60 seconds"
"step 5 aborted: api quota exceeded temporarily"
```

### `Failed(String)`
**When to use:** Generic failures that don't fit the above categories, or legacy code not yet migrated.

## Implementation Details

### Error Classification

The `classify_error()` helper function automatically categorizes errors based on message content:

```rust
fn classify_error(reason: &str) -> StepOutcome {
    // Checks for keywords: "policy", "permission", "access denied", etc.
    // Returns appropriate StepOutcome variant
}
```

**Classification Rules:**

| Keywords | Variant |
|----------|---------|
| policy, permission, access denied, plane guard, forbidden | `PolicyViolation` |
| rate limit, too many requests, 429 | `RateLimited` |
| credential, not found, invalid schema, authentication, oauth, api key | `PermanentError` |
| timeout, connection refused, service unavailable, 503, 504 | `TransientError` |
| (other) | `Failed` |

### Where Classification Happens

1. **FailureRule matches deterministically** (loop.rs line ~758):
   - FailureRule::Abort matched → `PermanentError`
   
2. **Evaluator aborts step** (loop.rs line ~1141):
   - All abort reasons passed through `classify_error()`
   - Returned as appropriate variant

3. **Provider credentials missing** (loop.rs line ~626):
   - Returns `PermanentError (credentials)`

## Usage Examples

### Frontend/API Layer

When receiving `StepOutcome`, handle each variant:

```rust
match step_outcome {
    StepOutcome::TransientError { reason, retry_after_secs } => {
        log::warn!("Transient error, will retry in {}s: {}", retry_after_secs, reason);
        // Show user a progress indicator
    }
    StepOutcome::PermanentError { reason } => {
        log::error!("Permanent error — user action required: {}", reason);
        // Show error modal with action button (add credentials, etc.)
    }
    StepOutcome::PolicyViolation { reason } => {
        log::error!("Access denied — escalating to admin: {}", reason);
        // Notify admin, show escalation UI
    }
    StepOutcome::RateLimited { retry_after_secs, reason } => {
        log::warn!("Rate limited, retrying in {}s", retry_after_secs);
        // Update UI with queue/wait status
    }
    _ => { /* handle other outcomes */ }
}
```

### Logging & Monitoring

Each variant provides context for observability:

```rust
// Transient failures are expected and should trigger automated retry
metrics.increment("runtime.error.transient");

// Permanent errors indicate configuration issues (actionable by user)
metrics.increment("runtime.error.permanent");

// Policy violations indicate permission issues (actionable by admin)
metrics.increment("runtime.error.policy_violation");

// Rate limits indicate scaling/quota issues (actionable by ops)
metrics.increment("runtime.error.rate_limited");
```

### Cost Optimization

**Evaluator LLM Calls Saved:**
- TransientError: Skip LLM, use exponential backoff
- RateLimited: Skip LLM, wait and retry
- PolicyViolation: Skip LLM, escalate immediately
- FailureRule deterministic abort: Skip LLM, return error classification

**Estimated Reduction:** 20-25% fewer evaluator LLM calls through finer error classification

## Migration Guide

### For Existing Code

Old pattern:
```rust
return Ok(StepOutcome::Failed(reason));
```

New pattern (when you know the error category):
```rust
if reason.contains("timeout") {
    return Ok(StepOutcome::TransientError { 
        reason, 
        retry_after_secs: 30 
    });
} else if reason.contains("credential") {
    return Ok(StepOutcome::PermanentError { reason });
} else {
    // Let classify_error() categorize it
    return Ok(classify_error(&reason));
}
```

### For Tests

Update test assertions to match new variants:

```rust
// Old
assert!(matches!(outcome, StepOutcome::Failed(_)));

// New
assert!(matches!(outcome, StepOutcome::PermanentError { .. }));
// or
assert!(matches!(outcome, StepOutcome::Failed(_) | StepOutcome::PermanentError { .. }));
```

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) - Overall runtime pipeline design
- [src/agent/loop.rs](src/agent/loop.rs) - Main execution loop with error handling
- [src/agent/evaluator.rs](src/agent/evaluator.rs) - Fast-paths that skip LLM calls
