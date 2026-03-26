# Narayan Connector Connection Flow - Analysis Report

## Overview
This document maps the current connector connection flow in Narayan, from chat-based connector mentions through installation and validation.

---

## 1. Frontend Chat Components

### PlanModeChat.jsx
**File:** [narayan-v5/src/components/agent/PlanModeChat.jsx](narayan-v5/src/components/agent/PlanModeChat.jsx)

**Purpose:** Main conversational interface for building workflows in "Plan Mode"

**Key Phases:**
- `capturing_intent` → User describes what they want
- `resolving_connectors` → LLM detects required connectors (transparent to UI)
- `capturing_trigger` → Trigger configuration
- `capturing_output` → Output definition
- `capturing_constraints` → Rules/constraints
- `reviewing` → Final review phase
- `complete` → Workflow saved

**Connector Detection:**
- **Not explicitly handled in chat** — The LLM backend detects connector needs during planning
- Does NOT parse user text like "connect to Salesforce"
- Uses LLM to identify tools/connectors required in the plan

**Test/Validation:**
- Has `testing` state and calls `planModeApi.test(sessionId)` to run deterministic workflow tests
- Shows `TestResultPanel` with preflight checks + sandbox step results
- Can detect missing credentials during preflight phase

**Related Code:**
```javascript
const [testing, setTesting] = useState(false);
const [testResult, setTestResult] = useState(null);

// Run test before save
async function runTest() {
  setTesting(true);
  try {
    const res = await planModeApi.test(sessionId);
    setTestResult(res); // Shows pass/partial/fail
  } finally {
    setTesting(false);
  }
}
```

---

### RoleChatDrawer.jsx
**File:** [narayan-v5/src/components/agent/RoleChatDrawer.jsx](narayan-v5/src/components/agent/RoleChatDrawer.jsx)

**Purpose:** Chat for modifying existing roles (roles within agent definitions)

**Connector Handling:**
- Does NOT explicitly handle connector setup in chat
- Receives `pending_change` objects from backend with type `update_connectors`
- Shows `ChangeCard` with proposal confirmation UI
- User clicks "Apply change" to confirm

**Related Code:**
```javascript
function ChangeCard({ change, onConfirm, onDismiss, applying }) {
  const typeLabels = {
    // ...
    update_connectors: 'Update connectors',
    // ...
  };
  // Shows title and description, then Apply/Dismiss buttons
}

// Apply change endpoint
async function applyChange() {
  if (!pendingChange) return;
  setApplying(true);
  try {
    await roleChatApi.apply(roleId, sessionId, pendingChange);
    // Change applied
    onRoleChanged?.();
  } catch (e) {
    setError(e.message);
  }
}
```

---

### ConnectorsTab.jsx
**File:** [narayan-v5/src/components/settings/ConnectorsTab.jsx](narayan-v5/src/components/settings/ConnectorsTab.jsx)

**Purpose:** Settings UI for installing/managing connectors

**Connector Installation Flows:**

#### OAuth Connectors
```javascript
<a href={connectors.oauthStartUrl(conn.type)}
  className="btn-primary w-full text-xs">
  Connect <ExternalLink size={10} />
</a>
```
- Redirects to `GET /auth/oauth/{provider}/start`
- User grants permission
- Redirected to callback → token stored
- Returns `<CheckCircle2 /> Connected` badge

#### API Key Connectors
```javascript
function ApiKeyForm({ conn, onSave, onCancel }) {
  const [key, setKey] = useState('');
  const [settings, setSettings] = useState({});
  
  async function handleSave() {
    if (!key.trim()) return;
    setSaving(true);
    try {
      await connectors.installApiKey(conn.type, key.trim(), settings);
      onSave();
    } catch (e) {
      setErr(e.message);
      setSaving(false);
    }
  }
  // Renders form fields, password input with show/hide toggle
}
```
- Shows inline form with API key input
- Optional settings fields (subdomain, instance URL, etc.)
- Calls `POST /connectors/{type}/install` with credentials

#### Webhook Connectors
```javascript
async function handleWebhookInstall() {
  setInstalling(true);
  try {
    const r = await connectors.installWebhook(conn.type);
    onInstalled({ 
      webhook_url: r.webhook_url, 
      webhook_secret: r.webhook_secret 
    });
  } catch (e) {
    setInstalling(false);
  }
}
```
- Calls `POST /connectors/{type}/webhook-install`
- Returns webhook URL + secret to paste into external system

---

### PlanApprovalCard.jsx
**File:** [narayan-v5/src/components/cards/PlanApprovalCard.jsx](narayan-v5/src/components/cards/PlanApprovalCard.jsx#L80-L170)

**Purpose:** Detects missing connector credentials during plan approval

**CredentialGap Detection:**
```javascript
function CredentialGap({ name, onResolved, onWrongTool, onNavigateSettings }) {
  const [mode, setMode] = useState('choice'); // 'choice' | 'connect' | 'wrong'
  const [apiKey, setApiKey] = useState('');
  
  async function handleSaveKey() {
    if (!apiKey.trim()) return;
    setSaving(true);
    try {
      try {
        // Try connector install first
        await connectorsApi.installApiKey(name, apiKey.trim());
      } catch {
        // Fallback to generic credential storage
        await credentialsApi.set(name, apiKey.trim(), '', label(name));
      }
      onResolved(name);
    } catch (e) {
      setSaveErr(e.message);
      setSaving(false);
    }
  }
}
```

**Three Modes:**
1. **choice** - "Do you use X?" with Yes/No buttons
2. **connect** - Inline API key entry
3. **wrong** - "What do you use instead?" (alternative tool suggestion)

**Limitation:** 
- ⚠️ **NO validation after credential entry** — just saves the credential
- Assumes credentials are valid; user only finds out during execution

---

### ClarificationCard.jsx
**File:** [narayan-v5/src/components/cards/ClarificationCard.jsx](narayan-v5/src/components/cards/ClarificationCard.jsx#L75-L90)

**Purpose:** Handles user clarification questions during agent execution

**Connector-Related:**
```javascript
{q.connectorType && <span className="badge">
  <Plug size={9} /> {q.connectorType}
</span>}

{q.connectorType && onNavigateSettings && (
  <button onClick={onNavigateSettings}>
    {q.actionLabel || `Connect ${q.connectorType} in Settings`}
    <ArrowRight size={10} />
  </button>
)}
```
- Clarification questions can have `connectorType` property
- Shows badge with connector name
- Links to Settings for connector setup

---

## 2. Backend API Routes for Connectors

### Installation Routes
**File:** [src/connectors/oauth.rs](src/connectors/oauth.rs#L479-L560)

#### POST /connectors/:type/install
```rust
pub async fn install_connector(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let api_key = body["api_key"].as_str().or_else(|| body["token"].as_str())?;
    let settings = body.get("settings").cloned().unwrap_or_default();
    
    match state.connector_installs.upsert_api_key(
      &tenant.tenant_id, 
      &connector_type, 
      &api_key, 
      settings
    ).await {
        Ok(id) => {
            StatusCode::CREATED,
            Json(serde_json::json!({
                "installed": true,
                "id": id,
                "connector": connector_type,
            }))
        }
        Err(e) => StatusCode::INTERNAL_SERVER_ERROR
    }
}
```

**Request:**
```json
{
  "api_key": "sk_live_xxx",
  "settings": {
    "subdomain": "acme",  // optional, connector-specific
    "instance_url": "https://acme.service-now.com"
  }
}
```

**Response:**
```json
{
  "installed": true,
  "id": "nar_xxx",
  "connector": "salesforce"
}
```

#### POST /connectors/:type/webhook-install
```rust
pub async fn install_webhook_connector(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let base = std::env::var("NARAYAN_BASE_URL")?;
    let webhook_url = format!("{}/connectors/{}/webhook", base, connector_type);
    let webhook_secret = body["webhook_secret"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("nar_whsec_{}", new_id()));
    
    match state.connector_installs.upsert_webhook_only(
      &tenant.tenant_id,
      &connector_type,
      &webhook_secret,
      settings
    ).await {
        Ok((id, secret)) => {
            StatusCode::CREATED,
            Json(serde_json::json!({
                "installed": true,
                "webhook_url": webhook_url,
                "webhook_secret": secret,
            }))
        }
    }
}
```

**Response:**
```json
{
  "installed": true,
  "webhook_url": "https://api.narayan.dev/connectors/slack/webhook",
  "webhook_secret": "nar_whsec_abc123"
}
```

#### GET /connectors
```rust
pub async fn list_connectors(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
) -> impl IntoResponse {
    match state.connector_installs.list_for_tenant(&tenant.tenant_id).await {
        Ok(installs) => {
            Json(serde_json::json!({
                "connectors": installs,
                "count": installs.len()
            }))
        }
    }
}
```

**Response Format:**
```json
{
  "connectors": [
    {
      "id": "nar_xxx",
      "connector_type": "salesforce",
      "auth_type": "oauth",
      "connected": true,
      "settings": {"org_id": "00Dxx000000IZ3"},
      "created_at": "2026-03-26T10:00:00Z"
    }
  ],
  "count": 1
}
```

#### DELETE /connectors/:type
Uninstalls a connector (marks as disabled in DB)

---

### Inbound Webhook Route

#### POST /connectors/:type/webhook
**File:** [src/api/routes.rs](src/api/routes.rs#L1629-L1720)

```rust
pub async fn connector_inbound(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    // 1. Load connector from registry
    let connector = state.connector_registry.get(&connector_type)?;
    
    // 2. Load credentials from connector_installs store
    let (credentials, settings) = state.connector_installs
        .get(&tenant.tenant_id, &connector_type).await?;
    
    // 3. Build ConnectorConfig
    let config = ConnectorConfig {
        credentials,
        settings,
        // ...
    };
    
    // 4. Call connector::handle_inbound()
    let goal_str = connector.handle_inbound(&event, &config).await?;
    
    // 5. Create agent goal or match to AgentRole triggers
    // ...
}
```

---

## 3. Connector Framework & Validation

### Connector Trait
**File:** [src/connectors/framework.rs](src/connectors/framework.rs#L1-80)

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn connector_type(&self) -> &str;
    
    async fn handle_inbound(
        &self, 
        event: &ConnectorEvent, 
        config: &ConnectorConfig
    ) -> Result<Option<String>>;
    
    async fn deliver_output(
        &self,
        config: &ConnectorConfig,
        external_id: &str,
        output: &str,
        metadata: &serde_json::Value,
    ) -> Result<()>;
    
    /// ⚠️ VALIDATION METHOD (not currently called after install!)
    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()>;
}
```

### Per-Connector validate_config Examples
- [GitHub](src/connectors/github.rs#L130): `async fn validate_config()`
- [Zendesk](src/connectors/zendesk.rs#L89): `async fn validate_config()`
- [Salesforce](src/connectors/salesforce.rs#L133): `async fn validate_config()`
- [ServiceNow](src/connectors/servicenow.rs#L90): `async fn validate_config()`
- [HubSpot](src/connectors/hubspot.rs#L98): `async fn validate_config()`
- [QuickBooks](src/connectors/quickbooks.rs#L118): `async fn validate_config()`
- [PagerDuty](src/connectors/pagerduty.rs#L122): `async fn validate_config()`
- [Intercom](src/connectors/intercom.rs#L141): `async fn validate_config()`

**Purpose:** Each connector can verify that its configuration (API key, URL, auth token) is valid against the actual external service.

---

## 4. Test/Validation Endpoints (Already Exist!)

### POST /connections/mcp/test
**File:** [src/api/routes.rs](src/api/routes.rs#L2692-L2730)

```rust
pub async fn test_mcp_connection(
    State(_state): State<AppState>,
    _tenant: AuthenticatedTenant,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let server_url = body["server_url"].as_str()?;
    let token = body["token"].as_str();
    
    // Make a tools/list call to MCP server
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    
    let mut req = client.post(server_url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 1,
        }));
    
    if let Some(tok) = token {
        req = req.bearer_auth(tok);
    }
    
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            Json(serde_json::json!({
                "reachable": status < 400,
                "status": status,
                "tools": /* parsed tools array */,
                "tool_count": /* count */,
            }))
        }
        Err(e) => Json(serde_json::json!({
            "reachable": false,
            "error": e.to_string(),
        }))
    }
}
```

### POST /connections/api/test
**File:** [src/api/routes.rs](src/api/routes.rs#L2788-L2830)

```rust
pub async fn test_api_connection(
    State(_state): State<AppState>,
    _tenant: AuthenticatedTenant,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let base_url = body["base_url"].as_str()?;
    let token = body["token"].as_str();
    let auth_type = body["auth_type"].as_str().unwrap_or("bearer");
    let test_path = body["test_path"].as_str().unwrap_or("/");
    
    let full_url = format!("{}{}", base_url.trim_end_matches('/'), test_path);
    
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    
    let mut req = client.get(&full_url);
    if let Some(tok) = token {
        req = match auth_type {
            "api_key_header" => {
                let header = body["auth_header_name"].as_str().unwrap_or("X-API-Key");
                req.header(header, tok)
            }
            "basic" => req.basic_auth(tok, Option::<&str>::None),
            _ => req.bearer_auth(tok),
        };
    }
    
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            Json(serde_json::json!({
                "reachable": status < 500,
                "status": status,
                "sample": /* response body */,
            }))
        }
        Err(e) => Json(serde_json::json!({
            "reachable": false,
            "error": e.to_string(),
        }))
    }
}
```

### POST /connections/db/test
**File:** [src/api/routes.rs](src/api/routes.rs#L2901+)

Tests database connections (PostgreSQL, MySQL, etc.)

---

## 5. Connector Installation Store

**File:** [src/connectors/installs.rs](src/connectors/installs.rs#L1-300)

### ConnectorInstall (DB Model)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConnectorInstall {
    pub id: String,
    pub tenant_id: String,
    pub connector_type: String,
    pub auth_type: String,              // "oauth" | "api_key" | "webhook_only"
    pub token_enc: Option<String>,      // AES-256-GCM encrypted
    pub refresh_enc: Option<String>,    // OAuth refresh token (encrypted)
    pub token_expires_at: Option<DateTime<Utc>>,
    pub settings: serde_json::Value,    // JSON: subdomain, instance_url, etc.
    pub webhook_secret_enc: Option<String>, // encrypted webhook secret
    pub enabled: bool,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### Key Store Methods
```rust
impl ConnectorInstallStore {
    pub async fn upsert_api_key(
        &self,
        tenant_id: &str,
        connector_type: &str,
        api_key: &str,
        settings: serde_json::Value,
    ) -> Result<String> { /* encrypts and stores */ }
    
    pub async fn upsert_oauth_token(
        &self,
        tenant_id: &str,
        connector_type: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
        settings: serde_json::Value,
    ) -> Result<String> { /* encrypts and stores */ }
    
    pub async fn upsert_webhook_only(
        &self,
        tenant_id: &str,
        connector_type: &str,
        webhook_secret: &str,
        settings: serde_json::Value,
    ) -> Result<(String, String)> { /* stores secret */ }
    
    pub async fn list_for_tenant(&self, tenant_id: &str) 
        -> Result<Vec<ConnectorInstallView>> { /* lists all */ }
    
    pub fn decrypt_token(&self, install: &ConnectorInstall) 
        -> Option<String> { /* decrypts token */ }
    
    pub async fn delete(&self, tenant_id: &str, connector_type: &str) 
        -> Result<bool> { /* marks enabled=false */ }
}
```

---

## 6. Frontend API Client

**File:** [narayan-v5/src/api/index.js](narayan-v5/src/api/index.js#L145-L165)

```javascript
export const connectors = {
  // List all installed connectors
  list: () => req('GET', '/connectors'),
  
  // Install API key connector
  installApiKey: (type, api_key, settings = {}) =>
    req('POST', `/connectors/${type}/install`, { api_key, settings }),
  
  // Install webhook connector
  installWebhook: (type, settings = {}) =>
    req('POST', `/connectors/${type}/webhook-install`, { settings }),
  
  // OAuth flow start URL
  oauthStartUrl: (provider) => {
    const token = getToken();
    const base = import.meta.env.VITE_API_URL || '/api';
    return `${base}/auth/oauth/${provider}/start?token=${encodeURIComponent(token || '')}`;
  },
  
  // Fire test webhook
  testWebhook: (type, payload) => 
    req('POST', `/connectors/${type}/webhook`, payload),
  
  // Uninstall connector
  uninstall: (type) => req('DELETE', `/connectors/${type}`),
};

// Custom connections (MCP, REST API, Database)
export const connections = {
  testMcp: (server_url, token) => 
    req('POST', '/connections/mcp/test', { server_url, token }),
  
  testApi: (base_url, token, auth_type, auth_header_name, test_path) =>
    req('POST', '/connections/api/test', { 
      base_url, token, auth_type, auth_header_name, test_path 
    }),
  
  testDb: (connection_string) => 
    req('POST', '/connections/db/test', { connection_string }),
};
```

---

## 7. Current Connector Connection Flow (Visual Map)

```
┌─── User in PlanMode Chat ────────────────────────────────────────────┐
│                                                                        │
│  1. User describes goal: "Monitor Salesforce opportunities"          │
│                          ↓                                            │
│  2. LLM Backend detects: Salesforce connector needed                 │
│     (phase: "resolving_connectors")                                  │
│                          ↓                                            │
│  3. Backend plans steps with Salesforce API calls                    │
│     (phase: "reviewing")                                             │
│                          ↓                                            │
│  4. Frontend shows Plan Approval UI                                  │
│                          ↓                                            │
│  5. Server-side validation in approve_plan():                        │
│     a) Get installed connectors via ConnectorInstallStore            │
│     b) Scan plan tools vs. installed credentials                     │
│     c) If missing: return 400 {"error":"missing_credentials"}       │
│                          ↓                                            │
└──────► 6. Frontend shows CredentialGap Card ────────────────────────┘
         │
         ├─ User option 1: "Yes, connect it"
         │  └─→ Show inline API key form
         │      └─→ POST /connectors/salesforce/install
         │          └─→ ✓ Stored in connector_installs table
         │
         ├─ User option 2: "No, we use something else"
         │  └─→ Show alternative tool selector
         │
         └─ User action: If added = CredentialGap resolves
            └─→ User re-clicks "Approve Plan"
                └─→ Plan validation now passes (connector in list)
                    └─→ ✓ Agent execution can proceed
                        ⚠️ NO VALIDATION TEST at this point!
                            Agent will fail at runtime if creds are invalid
```

---

## 8. Current Flow Issues & Gaps

### ⚠️ Problem 1: No Active Validation After Installation
- **Location:** `POST /connectors/:type/install` endpoint
- **Issue:** Just stores credentials without verifying they work
- **Current:** Server assumes credentials are valid if decryption succeeds
- **Result:** Users don't know if credentials are bad until agent execution fails

### ⚠️ Problem 2: validate_config() Methods Exist But Never Called
- **Location:** Each connector implements `async fn validate_config()` in traits
- **Issue:** These methods are defined but never invoked anywhere
- **Where defined:**
  - `src/connectors/github.rs#L130`
  - `src/connectors/salesforce.rs#L133`
  - `src/connectors/zendesk.rs#L89`
  - etc.
- **Why not called:** Unclear design choice; possibly planned for future

### ⚠️ Problem 3: Test Endpoints Exist But Only for Custom Connections
- **Test endpoints available:**
  - `POST /connections/mcp/test` - for MCP servers ✓
  - `POST /connections/api/test` - for REST APIs ✓
  - `POST /connections/db/test` - for databases ✓
- **Missing:** No test endpoint for built-in connectors (Salesforce, Zendesk, etc.)

### ⚠️ Problem 4: No Feedback to User After Credential Entry
- **Location:** `PlanApprovalCard.jsx` / `CredentialGap` component
- **Issue:** User enters credentials → saves → assumes success
- **No indication:** Is the credential valid? Is the service reachable?
- **Result:** User might not realize failed credential until much later

---

## 9. Recommended Solutions

### Option A: Add POST /connectors/:type/test Endpoint
```rust
pub async fn test_connector(
    State(state): State<AppState>,
    tenant: AuthenticatedTenant,
    Path(connector_type): Path<String>,
) -> impl IntoResponse {
    // 1. Load installed connector config
    let install = state.connector_installs
        .get(&tenant.tenant_id, &connector_type).await?;
    
    // 2. Get connector from registry
    let connector = state.connector_registry.get(&connector_type)?;
    
    // 3. Call connector.validate_config()
    match connector.validate_config(&config).await {
        Ok(()) => Json(serde_json::json!({
            "valid": true,
            "connector": connector_type,
        })),
        Err(e) => Json(serde_json::json!({
            "valid": false,
            "error": e.to_string(),
        })),
    }
}
```

**Register route:**
```rust
.route("/connectors/:type/test", post(test_connector))
```

### Option B: Auto-Validate on Install
```rust
pub async fn install_connector(...) -> impl IntoResponse {
    // ... existing code ...
    
    // AFTER storing credentials:
    let connector = state.connector_registry.get(&connector_type)?;
    match connector.validate_config(&config).await {
        Ok(()) => {
            // ✓ Return 201 CREATED with valid: true
            (StatusCode::CREATED, Json(serde_json::json!({
                "installed": true,
                "valid": true,
                "id": id,
            })))
        }
        Err(e) => {
            // Delete the installation
            state.connector_installs.delete(...).await;
            // ✗ Return 400 with error
            err(StatusCode::BAD_REQUEST, format!(
                "Connector installation failed validation: {}", 
                e
            ))
        }
    }
}
```

### Option C: Return Validation Status in Test API
```rust
// POST /plan-mode/sessions/:id/test
// Include connector validation in preflight checks:

let preflight_checks = vec![
    // ... existing checks ...
    CheckResult {
        label: format!("Validate {} connector", "salesforce"),
        success: validate_connector_result.is_ok(),
        detail: validate_connector_result.err(),
    }
];
```

---

## 10. Key File Summary

| File | Purpose | Connector Flow Role |
|------|---------|-------------------|
| [narayan-v5/src/components/agent/PlanModeChat.jsx](narayan-v5/src/components/agent/PlanModeChat.jsx) | Main chat interface | Detects connector needs (LLM-driven) |
| [narayan-v5/src/components/agent/RoleChatDrawer.jsx](narayan-v5/src/components/agent/RoleChatDrawer.jsx) | Role chat | Shows connector change proposals |
| [narayan-v5/src/components/settings/ConnectorsTab.jsx](narayan-v5/src/components/settings/ConnectorsTab.jsx) | Connector management UI | OAuth, API key, webhook install forms |
| [narayan-v5/src/components/cards/PlanApprovalCard.jsx](narayan-v5/src/components/cards/PlanApprovalCard.jsx) | Plan approval UI | Detects + prompts for missing connectors |
| [narayan-v5/src/api/index.js](narayan-v5/src/api/index.js) | Frontend API client | Calls connector endpoints |
| [src/connectors/oauth.rs](src/connectors/oauth.rs) | Connector installation routes | install_connector, install_webhook_connector, list_connectors, uninstall |
| [src/connectors/framework.rs](src/connectors/framework.rs) | Connector trait | validate_config() method (not used) |
| [src/connectors/{github,salesforce,zendesk,etc}.rs](src/connectors/) | Individual connectors | Each implements validate_config() |
| [src/connectors/installs.rs](src/connectors/installs.rs) | Credential store | Encrypts/decrypts/stores connector configs |
| [src/api/routes.rs](src/api/routes.rs) | Main API routes | POST /connectors/:type/install, connector_inbound, test endpoints |

---

## 11. Conclusion

**Current State:**
- Connectors are installed through chat-driven plan approval
- Credentials are encrypted and stored
- Validation code exists but isn't invoked
- No feedback to user about credential validity
- User finds out credential is bad only when agent runs

**Recommended Next Steps:**
1. Add `POST /connectors/:type/test` endpoint to validate installed connectors
2. Call validation from `install_connector()` to fail-fast on bad credentials
3. Call validation in `approve_plan()` preflight checks
4. Expose validation status in frontend (success/fail badge)
5. Consider webhook test payload capability for inbound connectors
