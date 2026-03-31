import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import { Key, Plus, Trash2, Loader2, CheckCircle2, Eye, EyeOff, X } from 'lucide-react';
import { credentials, providers as providersApi } from '../../api';

const FALLBACK_PROVIDERS = [
  { id: 'anthropic', label: 'Anthropic', models: ['claude-sonnet-4-20250514', 'claude-opus-4-20250514', 'claude-haiku-4-5-20251001'] },
  { id: 'openai', label: 'OpenAI', models: ['gpt-4o', 'gpt-4o-mini', 'o1', 'o3-mini'] },
  { id: 'groq', label: 'Groq', models: ['openai/gpt-oss-120b', 'llama-3.3-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'] },
  { id: 'gemini', label: 'Gemini', models: ['gemini-2.0-flash', 'gemini-2.0-pro', 'gemini-1.5-pro'] },
  { id: 'nvidia', label: 'NVIDIA', models: ['openai/gpt-oss-120b', 'nvidia/nemotron-3-super-120b-a12b', 'nvidia/nemotron-3-nano-30b-a3b', 'meta/llama-3.1-70b-instruct', 'meta/llama-3.1-8b-instruct', 'nvidia/llama-3.1-nemotron-70b-instruct'] },
  { id: 'openrouter', label: 'OpenRouter', models: ['openai/gpt-4o', 'anthropic/claude-3-5-sonnet', 'meta-llama/llama-3.3-70b-instruct'] },
  { id: 'ollama', label: 'Ollama', models: ['llama3.3', 'qwen2.5-coder', 'deepseek-r1'] },
  { id: 'compatible', label: 'Compatible', models: ['custom-model'] },
];

const PROVIDER_COLORS = {
  anthropic: 'bg-accent-soft text-accent', openai: 'bg-ok-soft text-ok',
  groq: 'bg-vio-soft text-vio', gemini: 'bg-info-soft text-info',
  nvidia: 'bg-ok-soft text-ok', openrouter: 'bg-warn-soft text-warn',
  ollama: 'bg-bg-active text-tx-2', compatible: 'bg-bg-active text-tx-3',
};

export default function CredentialsTab({ onFlash, onProvidersChanged }) {
  const [list, setList] = useState([]);
  const [providerList, setProviderList] = useState(FALLBACK_PROVIDERS);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [form, setForm] = useState({ provider: 'anthropic', model: '', apiKey: '', label: '' });
  const [showKey, setShowKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(null);

  useEffect(() => {
    credentials.list().then(d => setList(d.credentials || [])).catch(() => {}).finally(() => setLoading(false));
    providersApi.list().then(d => { if (d.providers?.length) setProviderList(d.providers); }).catch(() => {});
  }, []);

  const selectedProvider = providerList.find(p => p.id === form.provider) || providerList[0];

  async function save() {
    setSaving(true);
    try {
      await credentials.set(form.provider, form.apiKey, form.model || selectedProvider.models[0], form.label);
      const r = await credentials.list();
      setList(r.credentials || []);
      setAdding(false);
      setForm({ provider: 'anthropic', model: '', apiKey: '', label: '' });
      onFlash?.('Credential saved');
      onProvidersChanged?.();
    } catch (e) { onFlash?.(e.message); }
    finally { setSaving(false); }
  }

  async function remove(provider) {
    setDeleting(null);
    try {
      await credentials.delete(provider);
      setList(l => l.filter(c => c.provider !== provider));
      onFlash?.('Credential deleted');
      onProvidersChanged?.();
    } catch (e) { onFlash?.(e.message); }
  }

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  return (
    <div className="space-y-4">
      {list.length === 0 && (
        <div className="rounded-[1.5rem] border border-dashed border-accent/20 bg-accent-soft/30 px-5 py-4">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 flex size-10 items-center justify-center rounded-2xl border border-accent/20 bg-bg-card text-accent">
              <Key size={16} />
            </div>
            <div className="min-w-0">
              <p className="text-sm font-semibold text-tx-1">No AI providers yet</p>
              <p className="mt-1 text-sm leading-6 text-tx-2">
                Add one provider API key to unlock agent creation. You can still explore the rest of settings while you set it up.
              </p>
            </div>
          </div>
        </div>
      )}

      {/* Existing credentials */}
      <div className="grid gap-3">
        {list.map(cred => (
          <motion.div key={cred.provider} className="card p-4 flex items-center gap-3" layout>
            <span className={clsx('size-10 rounded-xl flex items-center justify-center text-sm font-bold', PROVIDER_COLORS[cred.provider] || 'bg-bg-active text-tx-3')}>
              {cred.provider.charAt(0).toUpperCase()}
            </span>
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium text-tx-1 capitalize">{cred.provider}</p>
              <p className="text-xs text-tx-3 truncate">{cred.model || cred.label || 'Active'}</p>
            </div>
            <span className="badge bg-ok-soft text-ok border border-ok/20"><CheckCircle2 size={10} /> Active</span>
            {deleting === cred.provider ? (
              <div className="flex items-center gap-2">
                <span className="text-xs text-err">Delete?</span>
                <button onClick={() => remove(cred.provider)} className="text-xs font-medium text-err hover:text-err/80">Yes</button>
                <button onClick={() => setDeleting(null)} className="text-xs text-tx-3">No</button>
              </div>
            ) : (
              <button onClick={() => setDeleting(cred.provider)} className="p-1.5 rounded-lg text-tx-4 hover:text-err hover:bg-err-soft transition-all">
                <Trash2 size={14} />
              </button>
            )}
          </motion.div>
        ))}
      </div>

      {/* Add new */}
      <AnimatePresence>
        {adding ? (
          <motion.div className="card p-5 space-y-4" initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
            <div className="flex items-center justify-between">
              <span className="text-sm font-semibold text-tx-1">Add credential</span>
              <button onClick={() => setAdding(false)} className="text-tx-4 hover:text-tx-2"><X size={14} /></button>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-xs text-tx-3 mb-1 block">Provider</label>
                <select value={form.provider} onChange={e => setForm(f => ({ ...f, provider: e.target.value, model: '' }))}
                  className="input-field text-sm">
                  {providerList.map(p => <option key={p.id} value={p.id}>{p.label}</option>)}
                </select>
              </div>
              <div>
                <label className="text-xs text-tx-3 mb-1 block">Model</label>
                <select value={form.model || selectedProvider.models[0]}
                  onChange={e => setForm(f => ({ ...f, model: e.target.value }))}
                  className="input-field text-sm">
                  {selectedProvider.models.map(m => <option key={m} value={m}>{m}</option>)}
                </select>
              </div>
            </div>
            <div>
              <label className="text-xs text-tx-3 mb-1 block">API Key</label>
              <div className="relative">
                <input value={form.apiKey} onChange={e => setForm(f => ({ ...f, apiKey: e.target.value }))}
                  type={showKey ? 'text' : 'password'} placeholder="sk-..." className="input-field text-sm pr-10" />
                <button onClick={() => setShowKey(s => !s)} className="absolute right-2 top-1/2 -translate-y-1/2 text-tx-4">
                  {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
            </div>
            <div>
              <label className="text-xs text-tx-3 mb-1 block">Label (optional)</label>
              <input value={form.label} onChange={e => setForm(f => ({ ...f, label: e.target.value }))}
                placeholder="My production key" className="input-field text-sm" />
            </div>
            <button onClick={save} disabled={saving || !form.apiKey.trim()} className="btn-primary w-full flex items-center justify-center gap-2 disabled:opacity-50">
              {saving ? <Loader2 size={14} className="animate-spin" /> : <Key size={14} />} Save credential
            </button>
          </motion.div>
        ) : (
          <button onClick={() => setAdding(true)}
            className="w-full rounded-xl border-2 border-dashed border-border hover:border-accent/40 bg-bg-card p-6 flex flex-col items-center gap-2 transition-colors group">
            <Plus size={20} className="text-tx-4 group-hover:text-accent transition-colors" />
            <span className="text-sm text-tx-3 group-hover:text-tx-1">Add credential</span>
          </button>
        )}
      </AnimatePresence>
    </div>
  );
}
