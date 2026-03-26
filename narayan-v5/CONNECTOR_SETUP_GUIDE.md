# Connector Setup Modal Integration Guide

## Overview

The `ConnectorSetupModal` component provides real-time connector verification in chat contexts. It's used to ensure users have actually connected required integrations before proceeding with agent creation or role updates.

## Features

- **Real-time verification** via `GET /connectors` polling
- **OAuth support** with popup window detection
- **API key support** with settings tab redirect
- **Progress tracking** showing % of connectors verified
- **Auto-close** on modal mode when all verified
- **Error handling** with retry mechanism
- **Reusable hook** for any context needing verification

## Integration Points

### 1. Plan Mode Chat (`PlanModeChat.jsx`)
**When:** User clicks "Create agent" after specifying required connectors  
**Flow:**
1. User completes plan mode form
2. Clicks "Create agent" button
3. `handleSave()` extracts `draft_role.connectors` from session
4. If connectors exist, `ConnectorSetupModal` appears instead of immediate save
5. User connects each service (OAuth or API key)
6. On verification complete, modal auto-closes and agent saves

**Code locations:**
- Import: Line 1-10
- State: Lines 442-449
- Save handler: Lines 622-661
- Callback: Lines 668-687
- Render: Lines 925-950

### 2. Role Chat Drawer (`RoleChatDrawer.jsx`)
**When:** Chat suggests adding a connector (`update_connectors` change)  
**Flow:**
1. User in role chat sees "Add Slack" suggestion
2. Clicks "Apply change" on the ChangeCard
3. `applyChange()` detects `update_connectors` type
4. Extracts new connectors, shows `ConnectorSetupModal`
5. User connects each service
6. On verification complete, modal closes and change applies

**Code locations:**
- Import: Line 1-10
- State: Lines 130-140
- Change handler: Lines 181-217
- Render: Lines 420+

### 3. Goal/Agent Chat (NOT YET IMPLEMENTED)
**Planned for:** When agent goal creation mentions required connectors

## Built-in Connectors (20 total)

### OAuth (12)
- github, slack, gmail, outlook, salesforce, hubspot, jira, notion, quickbooks, docusign, stripe, intercom

### API Key (8)
- linear, monday, zendesk, servicenow, pagerduty, freshdesk, greenhouse, dbt_cloud

## Component Props

```jsx
<ConnectorSetupModal
  requiredConnectors={['slack', 'salesforce']}  // Array of connector IDs
  onClose={() => setShowModal(false)}            // Close handler
  onVerified={(success) => handleVerified(success)}  // Verification callback
  mode="modal"  // 'modal' (full screen) or 'inline' (embedded)
/>
```

## Hook Usage

For lighter-weight integration without full modal:

```jsx
const { verified, missing, loading, verify } = useConnectorVerification(['slack', 'salesforce']);

if (missing.length > 0) {
  return <span>Missing: {missing.join(', ')}</span>;
}
```

## API Flow

1. **List installed:** `const installed = await connectors.list();`
   - Returns list of installed connector objects
   - Each has `id` or `type` field matching connector ID

2. **OAuth:** `const url = connectors.oauthStartUrl(connectorId);`
   - Returns OAuth start URL
   - Modal opens popup at this URL
   - Polls for window closure to detect completion

3. **API Key:** User navigates to `/settings?tab=connectors&setup=<id>`
   - Submits API key in settings
   - Modal re-checks `connectors.list()` after 2000ms

## Testing Checklist

- [ ] OAuth popup opens when clicking connector button
- [ ] Window close detection works (modal sees oauth complete)
- [ ] `connectors.list()` returns updated installed list after OAuth
- [ ] API key redirect to settings works
- [ ] Modal re-verification shows newly added connector as "confirmed"
- [ ] Progress bar updates to 100% when all verified
- [ ] Modal auto-closes on modal mode when 100% complete
- [ ] Modal shows error message on API failures
- [ ] Retry button re-verifies after error

## Common Issues

### Modal doesn't show
- Check: `requiredConnectors` array is populated
- Check: `showConnectorModal` state is true
- Check: Mode is 'modal' for full-screen display

### Verification fails
- Check: Backend `GET /connectors` endpoint is returning correct format
- Check: Connector IDs match (case-sensitive string equality)
- Check: User has auth token (401 error)

### OAuth popup doesn't detect completion
- Check: Window.open() popup permissions are enabled
- Check: Browser allows popup opening
- Check: Poll timeout (120 x 500ms = 60 seconds)

### API key connector not marked as verified
- Check: Settings page submission triggers API call
- Check: `connectors.list()` includes newly added connector
- Check: Re-verify delay (2000ms) is long enough for API to process

## Future Enhancements

- Custom MCP/API/DB connector support
- Bulk connector operations
- Connector dependency chains
- Fallback manual verification
- Connector health checks

