import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import { Key, Plus, Trash2, Loader2, CheckCircle2, Eye, EyeOff, X, ExternalLink } from 'lucide-react';
import { credentials, providers as providersApi } from '../../api';

const FALLBACK_PROVIDERS = [
  { id: 'anthropic', label: 'Anthropic', models: ['claude-sonnet-4-20250514', 'claude-opus-4-20250514', 'claude-haiku-4-5-20251001'] },
  { id: 'openai', label: 'OpenAI', models: ['gpt-4o', 'gpt-4o-mini', 'o1', 'o3-mini'] },
  { id: 'groq', label: 'Groq', models: ['openai/gpt-oss-120b', 'llama-3.3-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'] },
  { id: 'gemini', label: 'Gemini', models: ['gemini-2.0-flash', 'gemini-2.0-pro', 'gemini-1.5-pro'] },
  { id: 'nvidia', label: 'NVIDIA', models: ['openai/gpt-oss-120b', 'nvidia/nemotron-3-super-120b-a12b', 'nvidia/nemotron-3-nano-30b-a3b', 'meta/llama-3.1-70b-instruct', 'meta/llama-3.1-8b-instruct', 'nvidia/llama-3.1-nemotron-70b-instruct'] },
  { id: 'openrouter', label: 'OpenRouter', models: [
    { id: 'openrouter/free', label: 'openrouter/free', badge: 'Free', hint: 'Best for quick tests; free-tier limits are rate-limited by OpenRouter.' },
    { id: 'openai/gpt-4o-mini', label: 'openai/gpt-4o-mini', badge: 'Low-cost', hint: 'Cheap, fast general-purpose model.' },
    { id: 'anthropic/claude-3.5-haiku', label: 'anthropic/claude-3.5-haiku', badge: 'Low-cost', hint: 'Fast Anthropic option on OpenRouter.' },
    { id: 'meta-llama/llama-3.3-70b-instruct', label: 'meta-llama/llama-3.3-70b-instruct', badge: 'Paid', hint: 'Higher quality, usage billed by provider.' },
    { id: 'qwen/qwen-2.5-72b-instruct', label: 'qwen/qwen-2.5-72b-instruct', badge: 'Paid', hint: 'Strong reasoning model, paid usage.' },
  ] },
  { id: 'ollama', label: 'Ollama', models: ['llama3.3', 'qwen2.5-coder', 'deepseek-r1'] },
  { id: 'compatible', label: 'Compatible', models: ['custom-model'] },
];

const PROVIDER_COLORS = {
  anthropic: 'bg-accent-soft text-accent', openai: 'bg-ok-soft text-ok',
  groq: 'bg-vio-soft text-vio', gemini: 'bg-info-soft text-info',
  nvidia: 'bg-ok-soft text-ok', openrouter: 'bg-warn-soft text-warn',
  ollama: 'bg-bg-active text-tx-2', compatible: 'bg-bg-active text-tx-3',
};

const PROVIDER_HELP = {
  anthropic: {
    title: 'Anthropic setup help',
    subtitle: 'Bring your own key from Anthropic Console.',
    links: [
      { label: 'Anthropic console', href: 'https://console.anthropic.com/settings/keys' },
      { label: 'Anthropic docs', href: 'https://docs.anthropic.com' },
    ],
    steps: [
      'Open the Anthropic console.',
      'Create an API key.',
      'Paste the key here and choose a model.',
    ],
  },
  openai: {
    title: 'OpenAI setup help',
    subtitle: 'Bring your own key from the OpenAI platform.',
    links: [
      { label: 'OpenAI API keys', href: 'https://platform.openai.com/api-keys' },
      { label: 'OpenAI docs', href: 'https://platform.openai.com/docs' },
    ],
    steps: [
      'Open the API keys page.',
      'Create or copy an API key.',
      'Paste the key here and choose a model.',
    ],
  },
  openrouter: {
    title: 'OpenRouter setup help',
    subtitle: 'Bring your own key. OpenRouter lets you switch between many models from one key.',
    links: [
      { label: 'OpenRouter dashboard', href: 'https://openrouter.ai/keys' },
      { label: 'Model catalog', href: 'https://openrouter.ai/models' },
    ],
    steps: [
      'Create an OpenRouter account.',
      'Generate an API key in the keys page.',
      'Paste the key here and choose a model.',
    ],
  },
  groq: {
    title: 'Groq setup help',
    subtitle: 'Bring your own key. Groq is best for low-latency hosted models.',
    links: [
      { label: 'Groq console', href: 'https://console.groq.com/keys' },
      { label: 'Groq docs', href: 'https://console.groq.com/docs' },
    ],
    steps: [
      'Sign in to Groq Console.',
      'Create an API key in the keys section.',
      'Paste the key here and pick a Groq model.',
    ],
  },
  gemini: {
    title: 'Gemini setup help',
    subtitle: 'Bring your own key from Google AI Studio or Vertex AI.',
    links: [
      { label: 'Google AI Studio', href: 'https://aistudio.google.com/app/apikey' },
      { label: 'Gemini docs', href: 'https://ai.google.dev/gemini-api/docs' },
    ],
    steps: [
      'Open Google AI Studio.',
      'Generate an API key.',
      'Paste the key here and choose a Gemini model.',
    ],
  },
  nvidia: {
    title: 'NVIDIA setup help',
    subtitle: 'Bring your own key from NVIDIA NIM / API portal.',
    links: [
      { label: 'NVIDIA API catalog', href: 'https://build.nvidia.com/explore/api' },
      { label: 'NVIDIA docs', href: 'https://docs.nvidia.com' },
    ],
    steps: [
      'Open the NVIDIA API portal.',
      'Generate or copy your key.',
      'Paste it here and choose a model.',
    ],
  },
  ollama: {
    title: 'Ollama setup help',
    subtitle: 'Local models usually do not need a hosted API key.',
    links: [
      { label: 'Ollama docs', href: 'https://ollama.com' },
    ],
    steps: [
      'Install and run Ollama locally.',
      'Choose a local model name.',
      'Leave API key empty if your backend does not require one.',
    ],
  },
  compatible: {
    title: 'Compatible endpoint help',
    subtitle: 'Use this for an OpenAI-compatible endpoint with a custom key.',
    links: [
      { label: 'OpenAI-compatible API docs', href: 'https://platform.openai.com/docs/api-reference' },
    ],
    steps: [
      'Enter the endpoint provider details.',
      'Paste the compatible API key.',
      'Pick the model name exposed by that endpoint.',
    ],
  },
};

const OPENROUTER_FREE_MODELS = new Set([
  'openrouter/free',
]);

const OPENROUTER_MODEL_HINTS = {
  'openrouter/free': 'Free tier, good for quick tests. Limits depend on OpenRouter availability.',
  'openai/gpt-4o-mini': 'Low-cost general model.',
  'anthropic/claude-3.5-haiku': 'Fast Anthropic option on OpenRouter.',
  'meta-llama/llama-3.3-70b-instruct': 'Paid, higher quality model.',
  'qwen/qwen-2.5-72b-instruct': 'Paid, strong reasoning model.',
};

function normalizeModels(providerId, models = []) {
  return models.map(m => {
    const model = typeof m === 'string' ? { id: m, label: m } : { ...m };
    if (providerId === 'openrouter') {
      const isFree = OPENROUTER_FREE_MODELS.has(model.id);
      model.badge = model.badge || (isFree ? 'Free' : OPENROUTER_MODEL_HINTS[model.id] ? 'Paid' : undefined);
      model.hint = model.hint || OPENROUTER_MODEL_HINTS[model.id] || (isFree ? 'Free models may be rate-limited by OpenRouter.' : '');
    }
    return model;
  });
}

function ModelBadge({ model }) {
  if (!model?.badge) return null;
  return (
    <span className={clsx(
      'rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide',
      model.badge === 'Free' ? 'bg-ok-soft text-ok border border-ok/20' : 'bg-warn-soft text-warn border border-warn/20'
    )}>
      {model.badge}
    </span>
  );
}

function modelMetaFor(providerId, modelId, liveModels = []) {
  if (providerId !== 'openrouter') return null;
  const live = openrouterModelDetails(modelId, liveModels);
  if (live) {
    return {
      badge: live.badge,
      hint: live.hint,
    };
  }
  const hint = OPENROUTER_MODEL_HINTS[modelId];
  return hint ? { badge: 'Paid', hint } : null;
}

function openrouterModelCompatible(model) {
  const id = String(model?.id || '').toLowerCase();
  if (!id) return false;
  if (['embed', 'embedding', 'rerank', 'moderation'].some(fragment => id.includes(fragment))) {
    return false;
  }
  return true;
}

function normalizeOpenRouterModel(model) {
  const id = String(model?.id || '').trim();
  const name = String(model?.name || id).trim();
  const prompt = String(model?.pricing?.prompt || '').trim();
  const completion = String(model?.pricing?.completion || '').trim();
  const supportedParameters = Array.isArray(model?.supported_parameters) ? model.supported_parameters : [];
  const capabilities = model?.capabilities || {
    tools: supportedParameters.some(p => ['tools', 'tool_choice'].includes(String(p))),
    structured_outputs: supportedParameters.some(p => ['structured_outputs', 'response_format'].includes(String(p))),
    text_only: model?.architecture?.input_modalities
      ? Array.isArray(model.architecture.input_modalities)
        ? model.architecture.input_modalities.every(m => String(m).toLowerCase() === 'text')
        : false
      : true,
  };
  const isFree = id === 'openrouter/free' || id.includes(':free') || (prompt === '0' || prompt === '0.0') && (completion === '0' || completion === '0.0');
  const badge = isFree ? 'Free' : 'Paid';
  const limit_hint = isFree
    ? 'Free models are rate-limited by OpenRouter and are best for testing.'
    : 'Usage is billed by the upstream provider through OpenRouter.';
  return {
    id,
    label: name,
    badge,
    hint: limit_hint,
    supported_parameters: supportedParameters,
    default_parameters: model?.default_parameters,
    per_request_limits: model?.per_request_limits,
    capabilities,
    context_length: model?.context_length,
    pricing: model?.pricing,
    architecture: model?.architecture,
    top_provider: model?.top_provider,
  };
}

function openrouterModelDetails(modelId, liveModels = []) {
  const id = String(modelId || '').trim();
  if (!id) return null;
  const live = liveModels.find(model =>
    model.id === id
    || model.canonical_slug === id
    || String(model.label || '').trim() === id
  );
  if (live) return live;
  const hint = OPENROUTER_MODEL_HINTS[id];
  if (!hint && id !== 'openrouter/free' && !id.includes(':free')) return null;
  return {
    id,
    label: id,
    badge: id === 'openrouter/free' || id.includes(':free') ? 'Free' : (hint ? 'Paid' : undefined),
    hint: hint || 'OpenRouter free tier; may be rate-limited.',
    capabilities: {
      tools: false,
      structured_outputs: false,
      text_only: true,
    },
  };
}

function openrouterFilterMatches(model, filter) {
  if (filter === 'all') return true;
  if (filter === 'free') return (model.badge || '').toLowerCase() === 'free';
  if (filter === 'paid') return (model.badge || '').toLowerCase() !== 'free';
  if (filter === 'tools') return !!model?.capabilities?.tools;
  if (filter === 'structured') return !!model?.capabilities?.structured_outputs;
  if (filter === 'text') return !!model?.capabilities?.text_only;
  return true;
}

export default function CredentialsTab({ onFlash, onProvidersChanged }) {
  const [list, setList] = useState([]);
  const [providerList, setProviderList] = useState(FALLBACK_PROVIDERS);
  const [liveOpenRouterModels, setLiveOpenRouterModels] = useState([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [form, setForm] = useState({ provider: 'anthropic', model: '', apiKey: '', label: '' });
  const [showKey, setShowKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(null);
  const [validationError, setValidationError] = useState('');
  const [openrouterFilter, setOpenrouterFilter] = useState('all');

  useEffect(() => {
    credentials.list().then(d => setList(d.credentials || [])).catch(() => {}).finally(() => setLoading(false));
    providersApi.list().then(d => { if (d.providers?.length) setProviderList(d.providers); }).catch(() => {});
    providersApi.openrouterModels()
      .then(d => {
        const models = (d?.models || []).map(normalizeOpenRouterModel);
        if (models.length) setLiveOpenRouterModels(models);
      })
      .catch(async () => {
        try {
          const resp = await fetch('https://openrouter.ai/api/v1/models');
          if (!resp.ok) return;
          const payload = await resp.json();
          const models = (payload?.data || payload?.models || payload || [])
            .filter(openrouterModelCompatible)
            .map(normalizeOpenRouterModel);
          if (models.length) setLiveOpenRouterModels(models);
        } catch {
          // Leave the curated fallback in place.
        }
      });
  }, []);

  const mergedProviderList = providerList.map(provider => (
    provider.id === 'openrouter' && liveOpenRouterModels.length
      ? { ...provider, models: liveOpenRouterModels }
      : provider
  ));

  const selectedProvider = mergedProviderList.find(p => p.id === form.provider) || mergedProviderList[0];
  const selectedModels = normalizeModels(selectedProvider?.id, selectedProvider?.models || []);
  const selectedModel = selectedModels.find(m => m.id === (form.model || selectedModels[0]?.id)) || selectedModels[0];
  const providerHelp = PROVIDER_HELP[form.provider];
  const filteredModels = form.provider === 'openrouter'
    ? selectedModels.filter(model => openrouterFilterMatches(model, openrouterFilter))
    : selectedModels;

  function selectedModelId(value) {
    if (!value) return '';
    return typeof value === 'string' ? value : value.id || '';
  }

  async function save() {
    setSaving(true);
    setValidationError('');
    try {
      const chosenModel = form.provider === 'openrouter'
        ? (form.model || selectedModelId(filteredModels[0]) || selectedModelId(selectedModel) || selectedModelId(selectedProvider?.models?.[0]))
        : (form.model || selectedModelId(selectedModel) || selectedModelId(selectedProvider?.models?.[0]));
      await credentials.validate(form.provider, form.apiKey, chosenModel);
      await credentials.set(form.provider, form.apiKey, chosenModel, form.label);
      const r = await credentials.list();
      setList(r.credentials || []);
      setAdding(false);
      setForm({ provider: 'anthropic', model: '', apiKey: '', label: '' });
      onFlash?.('Credential saved');
      onProvidersChanged?.();
    } catch (e) {
      const message = e.message || 'Validation failed';
      setValidationError(message);
      onFlash?.(message);
    }
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
              <div className="flex items-center gap-2 min-w-0">
                <p className="text-xs text-tx-3 truncate">{cred.model || cred.label || 'Active'}</p>
                {modelMetaFor(cred.provider, cred.model, liveOpenRouterModels)?.badge ? (
                  <span className={clsx(
                    'rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide border',
                    modelMetaFor(cred.provider, cred.model, liveOpenRouterModels)?.badge === 'Free'
                      ? 'bg-ok-soft text-ok border-ok/20'
                      : 'bg-warn-soft text-warn border-warn/20'
                  )}>
                    {modelMetaFor(cred.provider, cred.model, liveOpenRouterModels)?.badge}
                  </span>
                ) : null}
                {openrouterModelDetails(cred.model, liveOpenRouterModels)?.capabilities?.tools ? (
                  <span className="badge bg-info-soft text-info border border-info/20">Tools</span>
                ) : null}
                {openrouterModelDetails(cred.model, liveOpenRouterModels)?.capabilities?.structured_outputs ? (
                  <span className="badge bg-warn-soft text-warn border border-warn/20">Structured</span>
                ) : null}
                {openrouterModelDetails(cred.model, liveOpenRouterModels)?.capabilities?.text_only ? (
                  <span className="badge bg-ok-soft text-ok border border-ok/20">Text</span>
                ) : null}
              </div>
              {modelMetaFor(cred.provider, cred.model, liveOpenRouterModels)?.hint ? (
                <p className="mt-0.5 text-[11px] leading-4 text-tx-3">
                  {modelMetaFor(cred.provider, cred.model, liveOpenRouterModels)?.hint}
                </p>
              ) : null}
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
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_18rem]">
              <div className="space-y-3">
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="text-xs text-tx-3 mb-1 block">Provider</label>
                    <select value={form.provider} onChange={e => {
                      setValidationError('');
                      setOpenrouterFilter('all');
                      setForm(f => ({ ...f, provider: e.target.value, model: '' }));
                    }}
                      className="input-field text-sm">
                      {providerList.map(p => <option key={p.id} value={p.id}>{p.label}</option>)}
                    </select>
                  </div>
                  <div>
                    <label className="text-xs text-tx-3 mb-1 block">Model</label>
                    <div className="rounded-xl border border-border bg-bg-card px-1 py-1">
                      {form.provider === 'openrouter' && (
                        <div className="flex flex-wrap gap-2 px-2 pb-2">
                          {[
                            { id: 'all', label: 'All' },
                            { id: 'free', label: 'Free' },
                            { id: 'paid', label: 'Paid' },
                            { id: 'tools', label: 'Tools' },
                            { id: 'structured', label: 'Structured' },
                            { id: 'text', label: 'Text only' },
                          ].map(filter => (
                            <button
                              key={filter.id}
                              type="button"
                              onClick={() => setOpenrouterFilter(filter.id)}
                              className={clsx(
                                'rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide border transition-colors',
                                openrouterFilter === filter.id
                                  ? 'bg-accent-soft text-accent border-accent/20'
                                  : 'bg-bg-active text-tx-3 border-border hover:text-tx-1'
                              )}
                            >
                              {filter.label}
                            </button>
                          ))}
                        </div>
                      )}
                      <div className="max-h-72 overflow-y-auto pr-1 space-y-1">
                        {filteredModels.map(model => {
                          const active = (form.model || filteredModels[0]?.id) === model.id;
                          return (
                            <button
                              key={model.id}
                              type="button"
                              onClick={() => {
                                setValidationError('');
                                setForm(f => ({ ...f, model: model.id }));
                              }}
                              className={clsx(
                                'w-full rounded-lg px-3 py-2 text-left transition-colors',
                                active ? 'bg-accent-soft/30 border border-accent/20' : 'hover:bg-bg-active border border-transparent'
                              )}
                            >
                              <div className="flex items-center justify-between gap-2 flex-wrap">
                                <p className="text-xs font-medium text-tx-1 truncate">{model.label}</p>
                                <div className="flex items-center gap-1.5 flex-wrap justify-end">
                                  <ModelBadge model={model} />
                                  {model.capabilities?.tools ? <span className="badge bg-info-soft text-info border border-info/20">Tools</span> : null}
                                  {model.capabilities?.structured_outputs ? <span className="badge bg-warn-soft text-warn border border-warn/20">Structured</span> : null}
                                  {model.capabilities?.text_only ? <span className="badge bg-ok-soft text-ok border border-ok/20">Text</span> : <span className="badge bg-bg-active text-tx-3 border border-border">Multimodal</span>}
                                </div>
                              </div>
                              {model.hint ? <p className="mt-0.5 text-[11px] leading-4 text-tx-3">{model.hint}</p> : null}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  </div>
                </div>
                  <div>
                    <label className="text-xs text-tx-3 mb-1 block">API Key</label>
                    <div className="relative">
                    <input value={form.apiKey} onChange={e => {
                      setValidationError('');
                      setForm(f => ({ ...f, apiKey: e.target.value }));
                    }}
                      type={showKey ? 'text' : 'password'} placeholder="sk-..." className="input-field text-sm pr-10" />
                    <button onClick={() => setShowKey(s => !s)} className="absolute right-2 top-1/2 -translate-y-1/2 text-tx-4">
                      {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                  </div>
                  {validationError ? (
                    <p className="mt-1 text-[11px] leading-4 text-err">{validationError}</p>
                  ) : null}
                </div>
                <div>
                  <label className="text-xs text-tx-3 mb-1 block">Label (optional)</label>
                  <input value={form.label} onChange={e => setForm(f => ({ ...f, label: e.target.value }))}
                    placeholder="My production key" className="input-field text-sm" />
                </div>
                <button onClick={save} disabled={saving || !form.apiKey.trim()} className="btn-primary w-full flex items-center justify-center gap-2 disabled:opacity-50">
                  {saving ? <Loader2 size={14} className="animate-spin" /> : <Key size={14} />} Save credential
                </button>
              </div>

              <aside className="rounded-2xl border border-border bg-bg-active/50 p-4">
                <p className="text-sm font-semibold text-tx-1">
                  {providerHelp?.title || `${selectedProvider?.label || 'Provider'} setup help`}
                </p>
                <p className="mt-1 text-xs leading-5 text-tx-3">
                  {providerHelp?.subtitle || 'Bring your own key for this provider.'}
                </p>
                <div className="mt-4 space-y-3">
                  {(providerHelp?.steps || [
                    'Open the provider dashboard.',
                    'Create an API key.',
                    'Paste the key into Narayan.',
                  ]).map((step, index) => (
                    <div key={step} className="flex gap-3">
                      <span className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full bg-bg-card text-[10px] font-semibold text-tx-2 border border-border">
                        {index + 1}
                      </span>
                      <p className="text-xs leading-5 text-tx-2">{step}</p>
                    </div>
                  ))}
                </div>
                {providerHelp?.links?.length ? (
                  <div className="mt-4 space-y-2">
                    {providerHelp.links.map(link => (
                      <a key={link.href} href={link.href} target="_blank" rel="noreferrer"
                        className="flex items-center justify-between rounded-xl border border-border bg-bg-card px-3 py-2 text-xs text-tx-2 hover:border-accent/40 hover:text-tx-1 transition-colors">
                        <span>{link.label}</span>
                        <ExternalLink size={12} />
                      </a>
                    ))}
                  </div>
                ) : null}
              </aside>
            </div>
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
