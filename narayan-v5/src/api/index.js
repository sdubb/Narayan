// ── Narayan API Client ─────────────────────────────────────────────────────
const BASE = import.meta.env.VITE_API_URL || '/api';

function getToken() {
  return localStorage.getItem('narayan_token');
}

async function req(method, path, body, isPublic = false) {
  const headers = { 'Content-Type': 'application/json' };
  if (!isPublic) {
    const token = getToken();
    if (!token) {
      window.dispatchEvent(new CustomEvent('narayan:unauthenticated'));
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
    window.dispatchEvent(new CustomEvent('narayan:unauthenticated'));
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

// ── Routing ────────────────────────────────────────────────────────────────
export const routing = { update: (config) => req('PUT', '/routing', config) };

// ── Goals / Agents ─────────────────────────────────────────────────────────
export const agents = {
  createGoal: (description, images = []) =>
    req('POST', '/goals', { description, images }),
  list:    () => req('GET', '/agents'),
  get:     (id) => req('GET', `/agents/${id}`),
  logs:    (id) => req('GET', `/agents/${id}/logs`),
  pause:   (id) => req('POST', `/agents/${id}/pause`),
  resume:  (id) => req('POST', `/agents/${id}/resume`),
  clarify: (id, answers, freeform) =>
    req('POST', `/agents/${id}/clarify`, { answers, freeform }),
  replay:  (id) => req('GET', `/agents/${id}/replay`),
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
      const res = await fetch(url, {
        headers: { Authorization: `Bearer ${token}` },
        signal:  ctrl.signal,
      });
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
