// ── Narayan API Client ─────────────────────────────────────────────────────
const BASE = import.meta.env.VITE_API_URL || '/api';

function getToken() {
  return localStorage.getItem('narayan_token');
}

function emitUnauthenticated(expectedToken = null) {
  const activeToken = getToken();
  if (expectedToken === null) {
    if (activeToken) return;
  } else if (activeToken && activeToken !== expectedToken) {
    return;
  }
  window.dispatchEvent(new CustomEvent('narayan:unauthenticated', {
    detail: {
      token: expectedToken,
      at: Date.now(),
    },
  }));
}

async function req(method, path, body, isPublic = false) {
  const headers = { 'Content-Type': 'application/json' };
  let token = null;
  if (!isPublic) {
    token = getToken();
    if (!token) {
      emitUnauthenticated(null);
      throw new Error('Not authenticated');
    }
    headers['Authorization'] = `Bearer ${token}`;
  }

  const res = await fetch(`${BASE}${path}`, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });

  if (res.status === 401 && !isPublic) {
    throw new Error('Session expired. Please sign in again.');
  }

  if (!res.ok) {
    const err = await res.text().catch(() => 'Unknown error');
    // Surface plan limit errors clearly
    if (res.status === 402) throw new Error(`PAYMENT_REQUIRED:${err}`);
    throw new Error(err || `HTTP ${res.status}`);
  }
  const ct = res.headers.get('content-type') || '';
  return ct.includes('application/json') ? res.json() : res.text();
}

async function reqBlob(method, path, body, isPublic = false) {
  const headers = {};
  let token = null;
  if (!isPublic) {
    token = getToken();
    if (!token) {
      emitUnauthenticated(null);
      throw new Error('Not authenticated');
    }
    headers['Authorization'] = `Bearer ${token}`;
  }

  const res = await fetch(`${BASE}${path}`, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });

  if (res.status === 401 && !isPublic) {
    throw new Error('Session expired. Please sign in again.');
  }

  if (!res.ok) {
    const err = await res.text().catch(() => 'Unknown error');
    if (res.status === 402) throw new Error(`PAYMENT_REQUIRED:${err}`);
    throw new Error(err || `HTTP ${res.status}`);
  }

  return res.blob();
}

// ── Auth ───────────────────────────────────────────────────────────────────
export const auth = {
  register: (name, username, email, password) =>
    req('POST', '/auth/register', { name, username, email, password }, true),
  login: (identifier, password) =>
    req('POST', '/auth/login', { identifier, password }, true),
};

// ── Credentials ────────────────────────────────────────────────────────────
export const credentials = {
  set:    (provider, api_key, model, label) =>
    req('PUT', '/credentials', { provider, api_key, model, label }),
  list:   () => req('GET', '/credentials'),
  delete: (provider) => req('DELETE', `/credentials/${provider}`),
};

export const databaseConnections = {
  test: (connection_string) =>
    req('POST', '/connections/db/test', { connection_string }),
  register: (name, connection_string, allow_writes = false) =>
    req('POST', '/connections/db', { name, connection_string, allow_writes }),
};

export const providers = {
  list: () => req('GET', '/providers'),
};

// ── Routing ────────────────────────────────────────────────────────────────
export const routing = { update: (config) => req('PUT', '/routing', config) };

// ── Goals / Agents ─────────────────────────────────────────────────────────
export const agents = {
  createGoal: (description, images = [], conversationId = null) =>
    req('POST', '/goals', {
      description,
      images,
      ...(conversationId ? { conversation_id: conversationId } : {}),
    }),
  list:    () => req('GET', '/agents'),
  get:     (id) => req('GET', `/agents/${id}`),
  logs:    (id) => req('GET', `/agents/${id}/logs`),
  pause:   (id) => req('POST', `/agents/${id}/pause`),
  resume:  (id) => req('POST', `/agents/${id}/resume`),
  clarify: (id, answers, freeform) =>
    req('POST', `/agents/${id}/clarify`, { answers, freeform }),
  replay:  (id) => req('GET', `/agents/${id}/replay`),
  children: (id) => req('GET', `/agents/${id}/children`),
  cancel:  (id) => req('POST', `/agents/${id}/cancel`),
  approvePlan: (id, approved, feedback = '', editedSteps = null, revise = false) =>
    req('POST', `/agents/${id}/approve-plan`, {
      approved,
      revise,
      feedback: feedback || '',
      edited_steps: editedSteps || null,
    }),
  // Parse pending roles from memory_ref
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
  // Check if agent has more roles to configure
  hasMoreRolesToConfigure: (agent) => {
    return agents.getPendingRoles(agent).length > 0;
  },
  // Resume plan mode for next role
  resumePlanModeForNextRole: (agentId) =>
    req('POST', `/agents/${agentId}/plan-mode/resume`, {}),
};

// ── Agent Messaging ───────────────────────────────────────────────────────
export const agentMessages = {
  list:     (agentId, params = {}) => {
    const qs = new URLSearchParams(params).toString();
    return req('GET', `/agents/${agentId}/messages${qs ? `?${qs}` : ''}`);
  },
  get:      (agentId, messageId)   => req('GET', `/agents/${agentId}/messages/${messageId}`),
  ack:      (agentId, messageId)   => req('POST', `/agents/${agentId}/messages/${messageId}/ack`),
  continueChild: (agentId, childId, instruction) =>
    req('POST', `/agents/${agentId}/children/${childId}/continue`, { instruction }),
  listChildren: (agentId) => req('GET', `/agents/${agentId}/children`),
};

// ── Session Tasks ─────────────────────────────────────────────────────────
export const sessionTasks = {
  list:   (agentId)             => req('GET', `/agents/${agentId}/tasks`),
  get:    (agentId, taskId)     => req('GET', `/agents/${agentId}/tasks/${taskId}`),
  create: (agentId, body)       => req('POST', `/agents/${agentId}/tasks`, body),
  update: (agentId, taskId, body) => req('PUT', `/agents/${agentId}/tasks/${taskId}`, body),
  stop:   (agentId, taskId)     => req('POST', `/agents/${agentId}/tasks/${taskId}/stop`),
};

// ── Memory ────────────────────────────────────────────────────────────────
export const memory = {
  consolidate: (agentId) => req('POST', `/agents/${agentId}/memory/consolidate`),
  topics:      (agentId) => req('GET', `/agents/${agentId}/memory/topics`),
};

// ── Workspace ─────────────────────────────────────────────────────────────
export const workspace = {
  files: (agentId) => req('GET', `/agents/${agentId}/workspace/files`),
  tree:  (agentId) => req('GET', `/agents/${agentId}/workspace/tree`),
  file:  (agentId, path) => req('GET', `/agents/${agentId}/workspace/files/${encodeURIComponent(path)}`),
  download: (agentId, path) => reqBlob('GET', `/agents/${agentId}/workspace/files/${encodeURIComponent(path)}/download`),
  bundle: (agentId) => reqBlob('GET', `/agents/${agentId}/workspace/files.tar.zst`),
};

// ── Conversations ─────────────────────────────────────────────────────────
export const conversations = {
  list: () => req('GET', '/conversations'),
  get:  (id) => req('GET', `/conversations/${id}`),
};

// ── Reviews ────────────────────────────────────────────────────────────────
export const reviews = {
  list: () => req('GET', '/reviews'),
  resolve: (id, status, notes) =>
    req('POST', `/reviews/${id}/resolve`, {
      status: status === 'auto_approved' ? 'approved' : status, notes,
    }),
  resolveAll: (status, notes) =>
    req('POST', '/reviews/resolve-all', {
      status: status === 'auto_approved' ? 'approved' : status, notes,
    }),
};

// ── Citations ──────────────────────────────────────────────────────────────
export const citations = {
  listForAgent: (agentId) =>
    req('GET', `/agents/${agentId}/citations`).catch(() => ({ citations: [] })),
  all: () => req('GET', '/citations').catch(() => ({ citations: [] })),
};

// ── Auto-approvals ─────────────────────────────────────────────────────────
export const autoApprovals = {
  list:   () => req('GET', '/auto-approvals').catch(() => ({ rules: [] })),
  create: (rule_id, notes) => req('POST', '/auto-approvals', { rule_id, notes }),
  delete: (rule_id) => req('DELETE', `/auto-approvals/${encodeURIComponent(rule_id)}`),
};

// ── Connector installs (OAuth + API key + webhook) ─────────────────────────
export const connectors = {
  // List all installed connectors for this tenant
  list: () => req('GET', '/connectors'),
  // Install an API-key connector: { api_key, settings? }
  installApiKey: (type, api_key, settings = {}) =>
    req('POST', `/connectors/${type}/install`, { api_key, settings }),
  // Install a webhook-only connector (returns webhook_url + webhook_secret)
  installWebhook: (type, settings = {}) =>
    req('POST', `/connectors/${type}/webhook-install`, { settings }),
  // Validate that a connector's credentials actually work
  validate: (type) => req('POST', `/connectors/${type}/validate`),
  // OAuth start — redirects browser to provider consent page.
  // Token is appended as ?token= so the backend can validate without Authorization header.
  oauthStartUrl: (provider) => {
    const token = getToken();
    const base  = import.meta.env.VITE_API_URL || '/api';
    return `${base}/auth/oauth/${provider}/start?token=${encodeURIComponent(token || '')}`;
  },
  // Fire a test webhook payload to a connector
  testWebhook: (type, payload) => req('POST', `/connectors/${type}/webhook`, payload),
  // Uninstall a connector
  uninstall: (type) => req('DELETE', `/connectors/${type}`),
};

// ── Outbound webhooks ──────────────────────────────────────────────────────
export const webhooks = {
  list:   () => req('GET', '/webhooks'),
  create: (url, events, secret) => req('POST', '/webhooks', { url, events, secret }),
  delete: (id) => req('DELETE', `/webhooks/${id}`),
};

// ── Audit log ──────────────────────────────────────────────────────────────
export const audit = {
  query: (params = {}) => {
    const qs = new URLSearchParams(
      Object.fromEntries(Object.entries(params).filter(([, v]) => v != null))
    ).toString();
    return req('GET', `/audit${qs ? `?${qs}` : ''}`);
  },
};

// ── Metrics ────────────────────────────────────────────────────────────────
export const metrics = {
  get:   () => req('GET', '/metrics'),
  costs: () => req('GET', '/costs'),
};

// ── Billing ────────────────────────────────────────────────────────────────
export const billing = {
  // Get current subscription (plan, status, period)
  subscription: () => req('GET', '/billing/subscription'),
  // Create a checkout session → redirect user to returned redirect_url
  checkout: (plan, provider = 'paypal') =>
    req('POST', '/billing/checkout', { plan, provider }),
  cancelSubscription: () => req('POST', '/billing/subscription/cancel'),
  invoices: () => req('GET', '/billing/invoices'),
  // Credit top-ups
  credits: () => req('GET', '/billing/credits'),
  purchaseCredits: () => req('POST', '/billing/credits/purchase'),
};

// ── Skills ─────────────────────────────────────────────────────────────────
export const skills = {
  upload:   (name, description, steps, author) =>
    req('POST', '/skills/upload', { name, description, steps, author }),
  list:     () => req('GET', '/skills'),
  install:  (name) => req('POST', '/skills/install', { name }),
  registry: () => req('GET', '/skills/registry'),
};

// ── Savings / ROI ──────────────────────────────────────────────────────────
export const savings = {
  getSummary: () => req('GET', '/savings'),
};

// ── Goal instance detail (criteria checks, step outputs) ───────────────────
export const goalInstances = {
  getDetail: (id) => req('GET', `/goal-instances/${id}`),
};

// ── Role chat ─────────────────────────────────────────────────────────────
export const roleChat = {
  start:  (roleId)                     => req('POST', `/roles/${roleId}/chat`, {}),
  turn:   (roleId, sessionId, message) => req('POST', `/roles/${roleId}/chat/${sessionId}/turn`, { message }),
  apply:  (roleId, sessionId, change)  => req('POST', `/roles/${roleId}/chat/${sessionId}/apply`, { change }),
};

// ── Custom connections (MCP, REST API, Database) ──────────────────────────
export const connections = {
  // MCP server
  testMcp:       (server_url, token) => req('POST', '/connections/mcp/test', { server_url, token }),
  registerMcp:   (name, server_url, token, summary) => req('POST', '/connections/mcp', { name, server_url, token, summary }),
  // REST API
  testApi:       (base_url, token, auth_type, auth_header_name, test_path) =>
    req('POST', '/connections/api/test', { base_url, token, auth_type, auth_header_name, test_path }),
  registerApi:   (body) => req('POST', '/connections/api', body),
  // Database
  testDb:        (connection_string) => req('POST', '/connections/db/test', { connection_string }),
  registerDb:    (name, connection_string, allow_writes) =>
    req('POST', '/connections/db', { name, connection_string, allow_writes }),
  // List all custom connections
  list:          () => req('GET', '/tenant-connectors'),
  remove:        (name) => req('DELETE', `/tenant-connectors/${name}`),
};

// ── Agent Definitions (multi-role agents) ─────────────────────────────────
export const agentDefs = {
  list:   () => req('GET', '/agent-definitions'),
  get:    (id) => req('GET', `/agent-definitions/${id}`),
  update: (id, body) => req('PUT', `/agent-definitions/${id}`, body),
  delete: (id) => req('DELETE', `/agent-definitions/${id}`),
  // Roles
  listRoles:  (agentId) => req('GET', `/agent-definitions/${agentId}/roles`),
  createRole: (agentId, body) => req('POST', `/agent-definitions/${agentId}/roles`, body),
  updateRole: (agentId, roleId, body) => req('PUT', `/agent-definitions/${agentId}/roles/${roleId}`, body),
  deleteRole: (agentId, roleId) => req('DELETE', `/agent-definitions/${agentId}/roles/${roleId}`),
  triggerRole: (agentId, roleId, inputData = {}) =>
    req('POST', `/agent-definitions/${agentId}/roles/${roleId}/trigger`, { input_data: inputData }),
  // Goal instances
  listGoalInstances: (agentId, limit = 50) =>
    req('GET', `/agent-definitions/${agentId}/goal-instances?limit=${limit}`),
  listRoleInstances: (agentId, roleId, limit = 50) =>
    req('GET', `/agent-definitions/${agentId}/roles/${roleId}/goal-instances?limit=${limit}`),
  summary: (agentId) =>
    req('GET', `/agent-definitions/${agentId}/summary`),
  chat: (agentId, message, conversation = []) =>
    req('POST', `/agent-definitions/${agentId}/chat`, { message, conversation }),
  exportSummaryPdf: (agentId) =>
    reqBlob('GET', `/agent-definitions/${agentId}/summary.pdf`),
  // Custom connectors
  listConnectors:   () => req('GET', '/tenant-connectors'),
  deleteConnector:  (name) => req('DELETE', `/tenant-connectors/${name}`),
};

// ── Plan Mode ─────────────────────────────────────────────────────────────
export const planMode = {
  // Start a new plan mode session, optionally for an existing agent (to add a role)
  // If templateId is provided, skips intent capture and uses pre-built role
  start: (agentName, agentId = null, templateId = null, attachments = []) =>
    req('POST', '/plan-mode/sessions', {
      agent_name: agentName,
      ...(agentId ? { agent_id: agentId } : {}),
      ...(templateId ? { template_id: templateId } : {}),
      ...(attachments.length ? { attachments } : {}),
    }),
  // Send a turn in the conversation — session tracks phase and history server-side
  turn: (sessionId, message, attachments = []) =>
    req('POST', `/plan-mode/sessions/${sessionId}/turn`, {
      message,
      ...(attachments.length ? { attachments } : {}),
    }),
  // Run deterministic workflow validation before save
  test: (sessionId) =>
    req('POST', `/plan-mode/sessions/${sessionId}/test`, {}),
  // Feed a failing/partial test result back into plan mode for repair
  revise: (sessionId, testResult) =>
    req('POST', `/plan-mode/sessions/${sessionId}/revise`, { test_result: testResult }),
  // Save and deploy — creates AgentDefinition + AgentRole in DB
  // The draft_role is stored in the session, no need to send it from the frontend
  save: (sessionId) =>
    req('POST', `/plan-mode/sessions/${sessionId}/save`, {}),
  // List available templates
  listTemplates: () => req('GET', '/plan-mode/templates'),
};

// ── Swarm ──────────────────────────────────────────────────────────────────
export const swarm = { status: () => req('GET', '/swarm/status') };

// ── Health ─────────────────────────────────────────────────────────────────
export const health = { check: () => req('GET', '/health', null, true) };

// ── SSE Stream ─────────────────────────────────────────────────────────────
export function streamAgent(agentId, onEvent, onError) {
  const token = getToken();
  const url   = `${BASE}/agents/${agentId}/stream`;
  let active  = true;
  const ctrl  = new AbortController();

  (async () => {
    try {
      if (!token) {
        emitUnauthenticated(null);
        onError?.(new Error('Session expired. Please sign in again.'));
        return;
      }
      const res = await fetch(url, {
        headers: { Authorization: `Bearer ${token}` },
        signal:  ctrl.signal,
      });
      if (res.status === 401) {
        onError?.(new Error('Session expired. Please sign in again.'));
        return;
      }
      if (!res.ok) {
        onError?.(new Error(`Stream failed: HTTP ${res.status}`));
        return;
      }
      const reader  = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer    = '';
      while (active) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const parts = buffer.split('\n\n');
        buffer = parts.pop() ?? '';
        for (const part of parts) {
          for (const line of part.split('\n')) {
            if (line.startsWith('data: ')) {
              const data = line.slice(6).trim();
              if (data && data !== '[DONE]') {
                try { onEvent(JSON.parse(data)); } catch {}
              }
            }
          }
        }
      }
    } catch (err) {
      if (err.name !== 'AbortError') onError?.(err);
    }
  })();

  return { close: () => { active = false; ctrl.abort(); } };
}
