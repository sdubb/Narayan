import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { motion, AnimatePresence } from 'framer-motion';
import {
  Plug, Loader2, CheckCircle2, ExternalLink, Trash2, X,
  Plus, Database, Globe, Server, ChevronRight, AlertCircle,
  Key, Eye, EyeOff, TestTube2, Zap,
} from 'lucide-react';
import { connectors, connections } from '../../api';

// ── Built-in connector catalogue ────────────────────────────────────────────
const BUILTIN = [
  // OAuth
  { type: 'github',      label: 'GitHub',       auth: 'oauth',  cat: 'devtools',   color: 'bg-tx-1 text-bg-card' },
  { type: 'slack',       label: 'Slack',        auth: 'oauth',  cat: 'comms',      color: 'bg-vio-soft text-vio' },
  { type: 'gmail',       label: 'Gmail',        auth: 'oauth',  cat: 'comms',      color: 'bg-err-soft text-err' },
  { type: 'outlook',     label: 'Outlook',      auth: 'oauth',  cat: 'comms',      color: 'bg-info-soft text-info' },
  { type: 'salesforce',  label: 'Salesforce',   auth: 'oauth',  cat: 'crm',        color: 'bg-info-soft text-info' },
  { type: 'hubspot',     label: 'HubSpot',      auth: 'oauth',  cat: 'crm',        color: 'bg-accent-soft text-accent' },
  { type: 'jira',        label: 'Jira',         auth: 'oauth',  cat: 'pm',         color: 'bg-info-soft text-info' },
  { type: 'notion',      label: 'Notion',       auth: 'oauth',  cat: 'pm',         color: 'bg-tx-1 text-bg-card' },
  { type: 'linear',      label: 'Linear',       auth: 'apikey', cat: 'pm',         color: 'bg-accent-soft text-accent' },
  { type: 'monday',      label: 'monday.com',   auth: 'apikey', cat: 'pm',         color: 'bg-info-soft text-info' },
  { type: 'quickbooks',  label: 'QuickBooks',   auth: 'oauth',  cat: 'finance',    color: 'bg-ok-soft text-ok' },
  { type: 'docusign',    label: 'DocuSign',     auth: 'oauth',  cat: 'legal',      color: 'bg-info-soft text-info' },
  { type: 'stripe',      label: 'Stripe',       auth: 'oauth',  cat: 'finance',    color: 'bg-vio-soft text-vio' },
  { type: 'intercom',    label: 'Intercom',     auth: 'oauth',  cat: 'support',    color: 'bg-info-soft text-info' },
  // API key
  { type: 'zendesk',     label: 'Zendesk',      auth: 'apikey', cat: 'support',    color: 'bg-ok-soft text-ok',
    fields: [{ key: 'subdomain', label: 'Subdomain', placeholder: 'acme (for acme.zendesk.com)' }] },
  { type: 'servicenow',  label: 'ServiceNow',   auth: 'apikey', cat: 'itsm',       color: 'bg-ok-soft text-ok',
    fields: [{ key: 'instance_url', label: 'Instance URL', placeholder: 'https://acme.service-now.com' }] },
  { type: 'pagerduty',   label: 'PagerDuty',    auth: 'apikey', cat: 'itsm',       color: 'bg-err-soft text-err' },
  { type: 'freshdesk',   label: 'Freshdesk',    auth: 'apikey', cat: 'support',    color: 'bg-ok-soft text-ok',
    fields: [{ key: 'domain', label: 'Domain', placeholder: 'acme (for acme.freshdesk.com)' }] },
  { type: 'greenhouse',  label: 'Greenhouse',   auth: 'apikey', cat: 'hr',         color: 'bg-ok-soft text-ok' },
  { type: 'dbt_cloud',   label: 'dbt Cloud',    auth: 'apikey', cat: 'data',       color: 'bg-accent-soft text-accent',
    fields: [{ key: 'account_id', label: 'Account ID', placeholder: '12345' }] },
  // Webhook
  { type: 'stripe_webhook', label: 'Stripe Webhooks', auth: 'webhook', cat: 'finance', color: 'bg-vio-soft text-vio' },
];

const CATS = [
  { id: 'all', label: 'All' },
  { id: 'crm', label: 'CRM' },
  { id: 'support', label: 'Support' },
  { id: 'comms', label: 'Comms' },
  { id: 'pm', label: 'Project Mgmt' },
  { id: 'devtools', label: 'Dev Tools' },
  { id: 'finance', label: 'Finance' },
  { id: 'itsm', label: 'ITSM' },
  { id: 'hr', label: 'HR' },
  { id: 'data', label: 'Data' },
  { id: 'legal', label: 'Legal' },
];

// ── Inline API key form ─────────────────────────────────────────────────────
function ApiKeyForm({ conn, onSave, onCancel }) {
  const [key, setKey]         = useState('');
  const [show, setShow]       = useState(false);
  const [settings, setSettings] = useState({});
  const [saving, setSaving]   = useState(false);
  const [err, setErr]         = useState('');

  async function handleSave() {
    if (!key.trim()) return;
    setSaving(true); setErr('');
    try {
      await connectors.installApiKey(conn.type, key.trim(), settings);
      onSave();
    } catch (e) { setErr(e.message); setSaving(false); }
  }

  return (
    <motion.div
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: 'auto' }}
      exit={{ opacity: 0, height: 0 }}
      className="space-y-2 overflow-hidden"
    >
      {(conn.fields || []).map(f => (
        <input
          key={f.key}
          value={settings[f.key] || ''}
          onChange={e => setSettings(s => ({ ...s, [f.key]: e.target.value }))}
          placeholder={f.placeholder}
          className="input-field text-xs w-full"
        />
      ))}
      <div className="relative">
        <input
          value={key}
          onChange={e => setKey(e.target.value)}
          onKeyDown={e => e.key === 'Enter' && handleSave()}
          placeholder="API key or token"
          type={show ? 'text' : 'password'}
          className="input-field text-xs w-full pr-8"
          autoFocus
        />
        <button
          onClick={() => setShow(s => !s)}
          className="absolute right-2 top-1/2 -translate-y-1/2 text-tx-4 hover:text-tx-2"
        >{show ? <EyeOff size={12} /> : <Eye size={12} />}</button>
      </div>
      {err && <p className="text-[11px] text-err">{err}</p>}
      <div className="flex gap-2">
        <button
          onClick={handleSave}
          disabled={!key.trim() || saving}
          className="btn-primary text-xs flex-1 disabled:opacity-50 flex items-center justify-center gap-1"
        >
          {saving ? <Loader2 size={11} className="animate-spin" /> : <Key size={11} />}
          Save
        </button>
        <button onClick={onCancel} className="btn-secondary text-xs px-2"><X size={12} /></button>
      </div>
    </motion.div>
  );
}

// ── Single built-in connector card ──────────────────────────────────────────
function ConnectorCard({ conn, installed, onInstalled, onUninstall }) {
  const [showForm, setShowForm] = useState(false);
  const [installing, setInstalling] = useState(false);

  async function handleWebhookInstall() {
    setInstalling(true);
    try {
      const r = await connectors.installWebhook(conn.type);
      onInstalled({ webhook_url: r.webhook_url, webhook_secret: r.webhook_secret });
    } catch (e) { setInstalling(false); }
  }

  return (
    <div className={clsx('card p-3.5 transition-all', installed && 'ring-2 ring-ok/25')}>
      <div className="flex items-center gap-2.5 mb-3">
        <span className={clsx('size-8 rounded-lg flex items-center justify-center text-xs font-bold shrink-0', conn.color)}>
          {conn.label.charAt(0)}
        </span>
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-tx-1 leading-tight">{conn.label}</p>
          <p className="text-[10px] text-tx-4">
            {conn.auth === 'oauth' ? 'OAuth' : conn.auth === 'webhook' ? 'Webhook' : 'API Key'}
          </p>
        </div>
        {installed && (
          <button onClick={() => onUninstall(conn.type)} className="p-1 rounded text-tx-4 hover:text-err transition-colors shrink-0">
            <Trash2 size={12} />
          </button>
        )}
      </div>

      {installed ? (
        <span className="badge bg-ok-soft text-ok border border-ok/20 w-full justify-center text-[10px]">
          <CheckCircle2 size={10} /> Connected
        </span>
      ) : conn.auth === 'oauth' ? (
        <a href={connectors.oauthStartUrl(conn.type)}
          className="btn-primary w-full text-xs text-center flex items-center justify-center gap-1.5">
          Connect <ExternalLink size={10} />
        </a>
      ) : conn.auth === 'webhook' ? (
        <button onClick={handleWebhookInstall} disabled={installing}
          className="btn-secondary w-full text-xs flex items-center justify-center gap-1.5">
          {installing ? <Loader2 size={11} className="animate-spin" /> : <Plug size={11} />}
          Generate webhook URL
        </button>
      ) : (
        <AnimatePresence mode="wait">
          {showForm ? (
            <ApiKeyForm
              key="form"
              conn={conn}
              onSave={() => { setShowForm(false); onInstalled(); }}
              onCancel={() => setShowForm(false)}
            />
          ) : (
            <motion.button
              key="btn"
              initial={{ opacity: 0 }} animate={{ opacity: 1 }}
              onClick={() => setShowForm(true)}
              className="btn-secondary w-full text-xs"
            >
              Add API key
            </motion.button>
          )}
        </AnimatePresence>
      )}
    </div>
  );
}

// ── Custom MCP server form ──────────────────────────────────────────────────
function McpForm({ onDone, onCancel }) {
  const [form, setForm]     = useState({ name: '', server_url: '', token: '', summary: '' });
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState(null);
  const [saving, setSaving] = useState(false);
  const [err, setErr]       = useState('');

  function set(k, v) { setForm(f => ({ ...f, [k]: v })); setTestResult(null); }

  async function test() {
    setTesting(true); setErr(''); setTestResult(null);
    try {
      const r = await connections.testMcp(form.server_url, form.token || undefined);
      setTestResult(r);
    } catch (e) { setErr(e.message); }
    finally { setTesting(false); }
  }

  async function save() {
    if (!form.name || !form.server_url) { setErr('Name and server URL are required'); return; }
    setSaving(true); setErr('');
    try {
      await connections.registerMcp(form.name, form.server_url, form.token || undefined, form.summary || undefined);
      onDone();
    } catch (e) { setErr(e.message); setSaving(false); }
  }

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1">Name *</label>
          <input value={form.name} onChange={e => set('name', e.target.value)}
            placeholder="my_platform" className="input-field text-xs w-full" />
        </div>
        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1">MCP Server URL *</label>
          <input value={form.server_url} onChange={e => set('server_url', e.target.value)}
            placeholder="https://api.myco.com/mcp/sse" className="input-field text-xs w-full" />
        </div>
      </div>
      <div>
        <label className="text-[11px] font-medium text-tx-3 block mb-1">Bearer token (optional)</label>
        <input value={form.token} onChange={e => set('token', e.target.value)}
          type="password" placeholder="sk-..." className="input-field text-xs w-full" />
      </div>
      <div>
        <label className="text-[11px] font-medium text-tx-3 block mb-1">Summary (optional)</label>
        <input value={form.summary} onChange={e => set('summary', e.target.value)}
          placeholder="My platform: query orders, update inventory..." className="input-field text-xs w-full" />
      </div>

      {testResult && (
        <div className={clsx('rounded-lg px-3 py-2.5 text-xs', testResult.reachable ? 'bg-ok-soft text-ok' : 'bg-err-soft text-err')}>
          {testResult.reachable
            ? `✓ Connected — ${testResult.tool_count} tool${testResult.tool_count !== 1 ? 's' : ''} available`
            : `✗ Unreachable — check the URL and token`}
          {testResult.tools?.length > 0 && (
            <div className="mt-1.5 text-tx-3 text-[11px]">
              Tools: {testResult.tools.slice(0, 5).map(t => t.name || t).join(', ')}
              {testResult.tools.length > 5 && ` +${testResult.tools.length - 5} more`}
            </div>
          )}
        </div>
      )}

      {err && <p className="text-xs text-err">{err}</p>}

      <div className="flex gap-2">
        <button onClick={test} disabled={!form.server_url || testing}
          className="btn-secondary text-xs flex items-center gap-1.5 disabled:opacity-50">
          {testing ? <Loader2 size={11} className="animate-spin" /> : <TestTube2 size={11} />}
          Test connection
        </button>
        <button onClick={save} disabled={saving || !form.name || !form.server_url}
          className="btn-primary text-xs flex-1 disabled:opacity-50 flex items-center justify-center gap-1.5">
          {saving ? <Loader2 size={11} className="animate-spin" /> : <Plus size={11} />}
          Register
        </button>
        <button onClick={onCancel} className="btn-secondary text-xs px-2"><X size={12} /></button>
      </div>
    </div>
  );
}

// ── Custom REST API form ────────────────────────────────────────────────────
function ApiForm({ onDone, onCancel }) {
  const [form, setForm] = useState({ name: '', base_url: '', token: '', auth_type: 'bearer', summary: '', test_path: '/' });
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState(null);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState('');

  function set(k, v) { setForm(f => ({ ...f, [k]: v })); setTestResult(null); }

  async function test() {
    setTesting(true); setErr(''); setTestResult(null);
    try {
      const r = await connections.testApi(form.base_url, form.token || undefined, form.auth_type, undefined, form.test_path);
      setTestResult(r);
    } catch (e) { setErr(e.message); }
    finally { setTesting(false); }
  }

  async function save() {
    if (!form.name || !form.base_url) { setErr('Name and base URL required'); return; }
    setSaving(true); setErr('');
    try {
      await connections.registerApi({ name: form.name, base_url: form.base_url, token: form.token || undefined, auth_type: form.auth_type, summary: form.summary || undefined });
      onDone();
    } catch (e) { setErr(e.message); setSaving(false); }
  }

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1">Name *</label>
          <input value={form.name} onChange={e => set('name', e.target.value)}
            placeholder="acme_backend" className="input-field text-xs w-full" />
        </div>
        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1">Base URL *</label>
          <input value={form.base_url} onChange={e => set('base_url', e.target.value)}
            placeholder="https://api.acme.com/v2" className="input-field text-xs w-full" />
        </div>
      </div>
      <div className="grid grid-cols-3 gap-3">
        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1">Auth type</label>
          <select value={form.auth_type} onChange={e => set('auth_type', e.target.value)}
            className="input-field text-xs w-full">
            <option value="bearer">Bearer token</option>
            <option value="api_key_header">API key header</option>
            <option value="basic">Basic auth</option>
            <option value="none">None</option>
          </select>
        </div>
        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1">
            {form.auth_type === 'bearer' ? 'Bearer token' : form.auth_type === 'basic' ? 'Username:password' : 'API key'}
          </label>
          <input value={form.token} onChange={e => set('token', e.target.value)}
            type="password" placeholder="..." className="input-field text-xs w-full" />
        </div>
        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1">Test path</label>
          <input value={form.test_path} onChange={e => set('test_path', e.target.value)}
            placeholder="/health" className="input-field text-xs w-full" />
        </div>
      </div>
      <div>
        <label className="text-[11px] font-medium text-tx-3 block mb-1">Summary (optional)</label>
        <input value={form.summary} onChange={e => set('summary', e.target.value)}
          placeholder="Acme backend: orders, inventory, customers..." className="input-field text-xs w-full" />
      </div>

      {testResult && (
        <div className={clsx('rounded-lg px-3 py-2 text-xs', testResult.reachable ? 'bg-ok-soft text-ok' : 'bg-warn-soft text-warn')}>
          {testResult.reachable ? `✓ Reachable (HTTP ${testResult.status})` : `HTTP ${testResult.status} — check credentials`}
        </div>
      )}

      {err && <p className="text-xs text-err">{err}</p>}

      <div className="flex gap-2">
        <button onClick={test} disabled={!form.base_url || testing}
          className="btn-secondary text-xs flex items-center gap-1.5 disabled:opacity-50">
          {testing ? <Loader2 size={11} className="animate-spin" /> : <TestTube2 size={11} />}
          Test
        </button>
        <button onClick={save} disabled={saving || !form.name || !form.base_url}
          className="btn-primary text-xs flex-1 disabled:opacity-50 flex items-center justify-center gap-1.5">
          {saving ? <Loader2 size={11} className="animate-spin" /> : <Plus size={11} />}
          Register API
        </button>
        <button onClick={onCancel} className="btn-secondary text-xs px-2"><X size={12} /></button>
      </div>
    </div>
  );
}

// ── External database form ──────────────────────────────────────────────────
function DbForm({ onDone, onCancel }) {
  const [form, setForm] = useState({ name: '', connection_string: '', allow_writes: false });
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState(null);
  const [saving, setSaving] = useState(false);
  const [show, setShow] = useState(false);
  const [err, setErr] = useState('');

  function set(k, v) { setForm(f => ({ ...f, [k]: v })); setTestResult(null); }

  async function test() {
    setTesting(true); setErr(''); setTestResult(null);
    try {
      const r = await connections.testDb(form.connection_string);
      setTestResult(r);
    } catch (e) { setErr(e.message); }
    finally { setTesting(false); }
  }

  async function save() {
    if (!form.name || !form.connection_string) { setErr('Name and connection string required'); return; }
    setSaving(true); setErr('');
    try {
      await connections.registerDb(form.name, form.connection_string, form.allow_writes);
      onDone();
    } catch (e) { setErr(e.message); setSaving(false); }
  }

  return (
    <div className="space-y-3">
      <div>
        <label className="text-[11px] font-medium text-tx-3 block mb-1">Database name *</label>
        <input value={form.name} onChange={e => set('name', e.target.value)}
          placeholder="acme_prod (used to call this DB in agents)" className="input-field text-xs w-full" />
      </div>
      <div>
        <label className="text-[11px] font-medium text-tx-3 block mb-1">Connection string *</label>
        <div className="relative">
          <input value={form.connection_string} onChange={e => set('connection_string', e.target.value)}
            type={show ? 'text' : 'password'}
            placeholder="postgresql://user:password@host:5432/dbname"
            className="input-field text-xs w-full pr-8" />
          <button onClick={() => setShow(s => !s)}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-tx-4 hover:text-tx-2">
            {show ? <EyeOff size={12} /> : <Eye size={12} />}
          </button>
        </div>
        <p className="text-[11px] text-tx-4 mt-1">Stored encrypted. Supports postgresql:// and mysql://</p>
      </div>
      <label className="flex items-center gap-2 cursor-pointer">
        <input type="checkbox" checked={form.allow_writes} onChange={e => set('allow_writes', e.target.checked)}
          className="rounded" />
        <span className="text-xs text-tx-2">Allow write operations (INSERT, UPDATE, DELETE)</span>
      </label>

      {testResult && (
        <div className={clsx('rounded-lg px-3 py-2.5 text-xs', testResult.connected ? 'bg-ok-soft text-ok' : 'bg-err-soft text-err')}>
          {testResult.connected
            ? `✓ Connected — ${testResult.table_count} table${testResult.table_count !== 1 ? 's' : ''} in public schema`
            : `✗ ${testResult.error || 'Connection failed'}`}
        </div>
      )}

      {err && <p className="text-xs text-err">{err}</p>}

      <div className="flex gap-2">
        <button onClick={test} disabled={!form.connection_string || testing}
          className="btn-secondary text-xs flex items-center gap-1.5 disabled:opacity-50">
          {testing ? <Loader2 size={11} className="animate-spin" /> : <TestTube2 size={11} />}
          Test connection
        </button>
        <button onClick={save} disabled={saving || !form.name || !form.connection_string}
          className="btn-primary text-xs flex-1 disabled:opacity-50 flex items-center justify-center gap-1.5">
          {saving ? <Loader2 size={11} className="animate-spin" /> : <Plus size={11} />}
          Register database
        </button>
        <button onClick={onCancel} className="btn-secondary text-xs px-2"><X size={12} /></button>
      </div>
    </div>
  );
}

// ── Custom connection card (MCP / API / DB) ─────────────────────────────────
function CustomCard({ conn, onRemove }) {
  const icon = conn.category?.includes('database')
    ? <Database size={14} className="text-tx-3" />
    : conn.category?.includes('mcp')
    ? <Server size={14} className="text-tx-3" />
    : <Globe size={14} className="text-tx-3" />;

  const typeLabel = conn.category?.includes('database') ? 'Database'
    : conn.category?.includes('mcp') ? 'MCP server' : 'REST API';

  return (
    <div className="card p-3.5 ring-2 ring-ok/25">
      <div className="flex items-start gap-2.5">
        <div className="size-8 rounded-lg bg-bg-active flex items-center justify-center shrink-0">{icon}</div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <p className="text-sm font-medium text-tx-1 truncate">{conn.name}</p>
            <span className="badge bg-ok-soft text-ok border border-ok/20 text-[10px]">
              <CheckCircle2 size={9} /> Connected
            </span>
          </div>
          <p className="text-[10px] text-tx-4">{typeLabel}</p>
          {conn.summary && <p className="text-[11px] text-tx-3 mt-0.5 leading-relaxed">{conn.summary}</p>}
        </div>
        <button onClick={() => onRemove(conn.name)} className="p-1 rounded text-tx-4 hover:text-err transition-colors shrink-0">
          <Trash2 size={12} />
        </button>
      </div>
    </div>
  );
}

// ── Main ConnectorsTab ──────────────────────────────────────────────────────
export default function ConnectorsTab({ onFlash }) {
  const [installed, setInstalled]       = useState([]);
  const [custom, setCustom]             = useState([]);
  const [loading, setLoading]           = useState(true);
  const [cat, setCat]                   = useState('all');
  const [adding, setAdding]             = useState(null); // null | 'mcp' | 'api' | 'db'
  const [webhookInfo, setWebhookInfo]   = useState(null); // shown after webhook install

  async function reload() {
    const [builtinRes, customRes] = await Promise.all([
      connectors.list().catch(() => ({ connectors: [] })),
      connections.list().catch(() => ({ connectors: [] })),
    ]);
    setInstalled(builtinRes.connectors || []);
    setCustom(customRes.connectors || []);
    setLoading(false);
  }

  useEffect(() => { reload(); }, []);

  const installedTypes = new Set(installed.map(c => c.type || c.connector_type));
  const filtered = cat === 'all' ? BUILTIN : BUILTIN.filter(c => c.cat === cat);

  async function uninstall(type) {
    try {
      await connectors.uninstall(type);
      setInstalled(l => l.filter(c => (c.type || c.connector_type) !== type));
      onFlash?.(`${type} disconnected`);
    } catch (e) { onFlash?.(e.message); }
  }

  async function removeCustom(name) {
    try {
      await connections.remove(name);
      setCustom(l => l.filter(c => c.name !== name));
      onFlash?.(`${name} removed`);
    } catch (e) { onFlash?.(e.message); }
  }

  if (loading) return (
    <div className="flex justify-center py-16">
      <Loader2 size={20} className="text-tx-4 animate-spin" />
    </div>
  );

  return (
    <div className="space-y-8">

      {/* ── Webhook info modal ─────────────────────────────────────────── */}
      <AnimatePresence>
        {webhookInfo && (
          <motion.div
            initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
            className="rounded-xl border border-ok/25 bg-ok-soft/30 p-4 space-y-2"
          >
            <div className="flex items-center justify-between">
              <p className="text-sm font-semibold text-ok">Webhook URL generated</p>
              <button onClick={() => setWebhookInfo(null)} className="text-tx-4 hover:text-tx-2"><X size={14} /></button>
            </div>
            <p className="text-[11px] text-tx-3">Paste these into the external system's webhook settings:</p>
            <div className="space-y-1.5">
              <div className="rounded-lg bg-bg px-3 py-2 font-mono text-[11px] text-tx-2 flex items-center justify-between gap-2">
                <span className="truncate">{webhookInfo.webhook_url}</span>
                <button onClick={() => navigator.clipboard?.writeText(webhookInfo.webhook_url)} className="text-tx-4 hover:text-accent text-[10px] shrink-0">Copy</button>
              </div>
              <div className="rounded-lg bg-bg px-3 py-2 font-mono text-[11px] text-tx-2 flex items-center justify-between gap-2">
                <span className="truncate">{webhookInfo.webhook_secret}</span>
                <button onClick={() => navigator.clipboard?.writeText(webhookInfo.webhook_secret)} className="text-tx-4 hover:text-accent text-[10px] shrink-0">Copy secret</button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Built-in connectors ─────────────────────────────────────────── */}
      <section>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-semibold text-tx-1">Built-in connectors</h3>
          <div className="flex items-center gap-1 overflow-x-auto">
            {CATS.map(c => (
              <button key={c.id} onClick={() => setCat(c.id)}
                className={clsx('px-2.5 py-1 rounded-lg text-[11px] font-medium whitespace-nowrap transition-colors',
                  cat === c.id ? 'bg-accent text-white' : 'text-tx-3 hover:text-tx-1 hover:bg-bg-active')}>
                {c.label}
              </button>
            ))}
          </div>
        </div>
        <div className="grid grid-cols-3 gap-3">
          {filtered.map(conn => (
            <ConnectorCard
              key={conn.type}
              conn={conn}
              installed={installedTypes.has(conn.type)}
              onInstalled={(info) => { reload(); info?.webhook_url && setWebhookInfo(info); onFlash?.(`${conn.label} connected`); }}
              onUninstall={uninstall}
            />
          ))}
        </div>
      </section>

      {/* ── Your custom connections ─────────────────────────────────────── */}
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h3 className="text-sm font-semibold text-tx-1">Your connections</h3>
            <p className="text-[11px] text-tx-4 mt-0.5">MCP servers, REST APIs, and databases you've connected</p>
          </div>
          <div className="flex items-center gap-2">
            {[
              { id: 'mcp', icon: <Server size={12} />, label: 'MCP server' },
              { id: 'api', icon: <Globe size={12} />, label: 'REST API' },
              { id: 'db',  icon: <Database size={12} />, label: 'Database' },
            ].map(btn => (
              <button key={btn.id}
                onClick={() => setAdding(adding === btn.id ? null : btn.id)}
                className={clsx('flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors',
                  adding === btn.id
                    ? 'bg-accent text-white'
                    : 'btn-secondary')}>
                {btn.icon}
                {btn.label}
              </button>
            ))}
          </div>
        </div>

        {/* Add form */}
        <AnimatePresence>
          {adding && (
            <motion.div
              initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }} exit={{ opacity: 0, height: 0 }}
              className="overflow-hidden mb-4"
            >
              <div className="card p-4">
                <div className="flex items-center gap-2 mb-4">
                  {adding === 'mcp' && <><Server size={14} className="text-accent" /><p className="text-sm font-semibold text-tx-1">Connect an MCP server</p></>}
                  {adding === 'api' && <><Globe size={14} className="text-accent" /><p className="text-sm font-semibold text-tx-1">Connect a REST API</p></>}
                  {adding === 'db'  && <><Database size={14} className="text-accent" /><p className="text-sm font-semibold text-tx-1">Connect a database</p></>}
                </div>
                {adding === 'mcp' && <McpForm onDone={() => { setAdding(null); reload(); onFlash?.('MCP server registered'); }} onCancel={() => setAdding(null)} />}
                {adding === 'api' && <ApiForm onDone={() => { setAdding(null); reload(); onFlash?.('REST API registered'); }} onCancel={() => setAdding(null)} />}
                {adding === 'db'  && <DbForm  onDone={() => { setAdding(null); reload(); onFlash?.('Database connected'); }} onCancel={() => setAdding(null)} />}
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Custom connection cards */}
        {custom.length === 0 && !adding ? (
          <div className="rounded-xl border border-dashed border-border-md p-8 text-center">
            <div className="size-10 rounded-xl bg-bg-active flex items-center justify-center mx-auto mb-3">
              <Zap size={18} className="text-tx-4" />
            </div>
            <p className="text-sm font-medium text-tx-1 mb-1">No custom connections yet</p>
            <p className="text-xs text-tx-4 max-w-xs mx-auto leading-relaxed">
              Connect your own backend API, MCP server, or Postgres database.
              Agents can then query and write to your systems directly.
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-3 gap-3">
            {custom.map(conn => (
              <CustomCard key={conn.name} conn={conn} onRemove={removeCustom} />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
