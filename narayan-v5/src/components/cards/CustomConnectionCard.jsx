import { useState } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, Globe, Loader2, Server, Shield, Sparkles, TestTube2 } from 'lucide-react';
import { connections } from '../../api';

const KINDS = {
  mcp: {
    title: 'Connect your MCP server inline',
    subtitle: 'Register a tenant MCP server, test that it responds, and continue without leaving chat.',
    icon: Server,
    testLabel: 'Test connection',
    saveLabel: 'Register MCP server',
    test: ({ server_url, token }) => connections.testMcp(server_url, token || undefined),
    save: ({ name, server_url, token, summary }) =>
      connections.registerMcp(name, server_url, token || undefined, summary || undefined),
    fields: [
      { key: 'name', label: 'Server name', placeholder: 'my_mcp_server', type: 'text' },
      { key: 'server_url', label: 'Server URL', placeholder: 'https://api.myco.com/mcp/sse', type: 'text' },
      { key: 'token', label: 'Bearer token (optional)', placeholder: 'sk-...', type: 'password' },
      { key: 'summary', label: 'Summary (optional)', placeholder: 'My server: search tickets, update records...', type: 'text' },
    ],
  },
  api: {
    title: 'Connect your REST API inline',
    subtitle: 'Register a tenant REST API, test it, and continue without leaving chat.',
    icon: Globe,
    testLabel: 'Test connection',
    saveLabel: 'Register REST API',
    test: ({ base_url, token, auth_type, test_path }) =>
      connections.testApi(base_url, token || undefined, auth_type, undefined, test_path),
    save: ({ name, base_url, token, auth_type, summary }) =>
      connections.registerApi({ name, base_url, token: token || undefined, auth_type, summary: summary || undefined }),
    fields: [
      { key: 'name', label: 'API name', placeholder: 'acme_backend', type: 'text' },
      { key: 'base_url', label: 'Base URL', placeholder: 'https://api.acme.com/v2', type: 'text' },
      {
        key: 'auth_type',
        label: 'Auth type',
        type: 'select',
        options: [
          { value: 'bearer', label: 'Bearer token' },
          { value: 'api_key_header', label: 'API key header' },
          { value: 'basic', label: 'Basic auth' },
          { value: 'none', label: 'None' },
        ],
      },
      { key: 'token', label: 'Secret / token', placeholder: '...', type: 'password' },
      { key: 'test_path', label: 'Test path', placeholder: '/health', type: 'text' },
      { key: 'summary', label: 'Summary (optional)', placeholder: 'Acme backend: orders, inventory, customers...', type: 'text' },
    ],
  },
};

function fieldLabel(kind, field, form) {
  if (kind !== 'api' || field.key !== 'token') return field.label;
  return form.auth_type === 'bearer'
    ? 'Bearer token'
    : form.auth_type === 'basic'
      ? 'Username:password'
      : 'API key';
}

export default function CustomConnectionCard({ kind, onConnected }) {
  const config = KINDS[kind];
  const Icon = config?.icon;
  const [form, setForm] = useState(
    kind === 'mcp'
      ? { name: '', server_url: '', token: '', summary: '' }
      : { name: '', base_url: '', token: '', auth_type: 'bearer', summary: '', test_path: '/' },
  );
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');
  const [testResult, setTestResult] = useState(null);

  if (!config) return null;

  const canTest =
    kind === 'mcp'
      ? Boolean(form.server_url.trim())
      : Boolean(form.base_url.trim());

  const canSave =
    kind === 'mcp'
      ? Boolean(form.name.trim() && form.server_url.trim())
      : Boolean(form.name.trim() && form.base_url.trim());

  function setField(key, value) {
    setForm(prev => ({ ...prev, [key]: value }));
    setTestResult(null);
  }

  async function handleTest() {
    if (!canTest) return;
    setTesting(true);
    setError('');
    setTestResult(null);
    try {
      const result = await config.test(form);
      setTestResult(result);
    } catch (e) {
      setError(e.message || 'Failed to test connection');
    } finally {
      setTesting(false);
    }
  }

  async function handleSave() {
    if (!canSave) return;
    setSaving(true);
    setError('');
    setSuccess('');
    try {
      const savedName = form.name.trim();
      await config.save(form);
      setSuccess(`Connected ${savedName}. Returning to plan mode...`);
      onConnected?.({ name: savedName, kind });
    } catch (e) {
      setError(e.message || 'Failed to save connection');
    } finally {
      setSaving(false);
    }
  }

  return (
    <motion.div
      className="rounded-xl border-l-4 border-l-accent border border-accent/20 bg-accent-soft/20 overflow-hidden"
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
    >
      <div className="flex items-center gap-2 px-4 py-3 border-b border-accent/15">
        <Icon size={14} className="text-accent" />
        <span className="text-sm font-semibold text-accent">{config.title}</span>
      </div>

      <div className="px-4 py-4 space-y-4">
        <div className="flex items-start gap-2.5">
          <Sparkles size={14} className="mt-0.5 text-accent shrink-0" />
          <div className="space-y-1">
            <p className="text-sm text-tx-1 font-medium">Add the connection here and keep going</p>
            <p className="text-xs text-tx-3">{config.subtitle}</p>
          </div>
        </div>

        <div className={clsx('grid gap-3', kind === 'mcp' ? 'grid-cols-1 sm:grid-cols-2' : 'grid-cols-1 sm:grid-cols-2')}>
          {config.fields.map(field => (
            <div key={field.key} className={field.key === 'summary' ? 'sm:col-span-2' : ''}>
              <label className="text-xs font-medium text-tx-2">{fieldLabel(kind, field, form)}</label>
              {field.type === 'select' ? (
                <select
                  value={form[field.key]}
                  onChange={e => setField(field.key, e.target.value)}
                  className="input-field mt-1"
                >
                  {field.options.map(option => (
                    <option key={option.value} value={option.value}>{option.label}</option>
                  ))}
                </select>
              ) : (
                <input
                  type={field.type}
                  value={form[field.key]}
                  onChange={e => setField(field.key, e.target.value)}
                  placeholder={field.placeholder}
                  className="input-field mt-1"
                  autoComplete="off"
                />
              )}
            </div>
          ))}
        </div>

        {error && <p className="text-xs text-err">{error}</p>}
        {success && (
          <div className="flex items-center gap-2 rounded-lg border border-ok/20 bg-ok-soft/30 px-3 py-2 text-xs text-ok">
            <CheckCircle2 size={12} />
            {success}
          </div>
        )}

        {testResult ? (
          <div className={clsx(
            'rounded-lg px-3 py-2.5 text-xs',
            testResult.reachable ? 'bg-ok-soft text-ok' : 'bg-err-soft text-err',
          )}>
            {kind === 'mcp'
              ? (testResult.reachable
                ? `✓ Connected — ${testResult.tool_count} tool${testResult.tool_count !== 1 ? 's' : ''} available`
                : '✗ Unreachable — check the URL and token')
              : (testResult.reachable
                ? `✓ Reachable (HTTP ${testResult.status})`
                : `HTTP ${testResult.status} — check credentials`)}
            {kind === 'mcp' && testResult.tools?.length > 0 && (
              <div className="mt-1.5 text-tx-3 text-[11px]">
                Tools: {testResult.tools.slice(0, 5).map(t => t.name || t).join(', ')}
                {testResult.tools.length > 5 && ` +${testResult.tools.length - 5} more`}
              </div>
            )}
          </div>
        ) : null}

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={handleTest}
            disabled={!canTest || testing}
            className="btn-secondary flex items-center gap-2 disabled:opacity-50"
          >
            {testing ? <Loader2 size={12} className="animate-spin" /> : <TestTube2 size={12} />}
            {config.testLabel}
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={!canSave || saving}
            className={clsx('btn-primary flex items-center gap-2 disabled:opacity-50', saving && 'cursor-wait')}
          >
            {saving ? <Loader2 size={12} className="animate-spin" /> : <Shield size={12} />}
            {config.saveLabel}
          </button>
        </div>
      </div>
    </motion.div>
  );
}
