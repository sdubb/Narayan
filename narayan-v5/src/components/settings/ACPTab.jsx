import { useEffect, useState } from 'react';
import clsx from 'clsx';
import { CheckCircle2, Loader2, Shield, Sparkles, Save, ArrowRight } from 'lucide-react';
import { clearAcpPeerConfig, readAcpPeerConfig, writeAcpPeerConfig } from './acpStorage';

export default function ACPTab({ onFlash, flash }) {
  const [form, setForm] = useState(readAcpPeerConfig);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState('');

  useEffect(() => {
    setForm(readAcpPeerConfig());
  }, []);

  function setField(key, value) {
    setForm(prev => ({ ...prev, [key]: value }));
    setStatus('');
  }

  async function handleSave() {
    if (!form.name.trim() || !form.peer_url.trim()) {
      setStatus('Name and peer URL are required.');
      return;
    }

    setSaving(true);
    setStatus('');
    try {
      const payload = {
        name: form.name.trim(),
        peer_url: form.peer_url.trim(),
        token: form.token.trim(),
        summary: form.summary.trim(),
      };
      writeAcpPeerConfig(payload);
      const message = `Saved ACP peer ${payload.name}.`;
      setStatus(message);
      onFlash?.(message);
      flash?.(message);
    } finally {
      setSaving(false);
    }
  }

  function clearSaved() {
    clearAcpPeerConfig();
    setForm({ name: '', peer_url: '', token: '', summary: '' });
    const message = 'Cleared saved ACP peer.';
    setStatus(message);
    onFlash?.(message);
    flash?.(message);
  }

  const hasSaved = Boolean(form.name.trim() && form.peer_url.trim());

  return (
    <div className="space-y-6">
      <div className="rounded-[1.5rem] border border-accent/20 bg-accent-soft/40 px-5 py-4">
        <div className="flex items-start gap-3">
          <div className="mt-0.5 flex size-10 items-center justify-center rounded-2xl border border-accent/20 bg-bg-card text-accent">
            <Shield size={16} />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-sm font-semibold text-tx-1">ACP peer settings</p>
            <p className="mt-1 text-sm leading-6 text-tx-2">
              Save the peer details that plan mode will reference when a workflow needs ACP coordination.
              This is a local settings surface until a dedicated ACP backend API is available.
            </p>
          </div>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-[1.2fr_0.8fr]">
        <div className="space-y-4 rounded-[1.5rem] border border-border bg-bg-card p-5">
          <div className="flex items-center gap-2 text-accent">
            <Sparkles size={14} />
            <p className="text-xs font-semibold uppercase tracking-[0.22em]">ACP configuration</p>
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-tx-2">Peer name</label>
              <input
                value={form.name}
                onChange={e => setField('name', e.target.value)}
                placeholder="customer_ops_peer"
                className="input-field"
              />
            </div>
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-tx-2">Peer URL</label>
              <input
                value={form.peer_url}
                onChange={e => setField('peer_url', e.target.value)}
                placeholder="https://peer.example.com/acp"
                className="input-field"
              />
            </div>
            <div className="space-y-1.5 sm:col-span-2">
              <label className="text-xs font-medium text-tx-2">Handshake token</label>
              <input
                type="password"
                value={form.token}
                onChange={e => setField('token', e.target.value)}
                placeholder="token or secret"
                className="input-field"
              />
            </div>
            <div className="space-y-1.5 sm:col-span-2">
              <label className="text-xs font-medium text-tx-2">Summary</label>
              <textarea
                value={form.summary}
                onChange={e => setField('summary', e.target.value)}
                placeholder="What this ACP peer can do..."
                className="input-field min-h-[110px] resize-none"
              />
            </div>
          </div>

          {status && (
            <div className={clsx(
              'flex items-center gap-2 rounded-lg border px-3 py-2 text-xs',
              hasSaved ? 'border-ok/20 bg-ok-soft/30 text-ok' : 'border-warn/20 bg-warn-soft/30 text-warn',
            )}>
              {hasSaved ? <CheckCircle2 size={12} /> : <Shield size={12} />}
              {status}
            </div>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={handleSave}
              disabled={saving}
              className="btn-primary flex items-center gap-2 disabled:opacity-50"
            >
              {saving ? <Loader2 size={12} className="animate-spin" /> : <Save size={12} />}
              Save ACP settings
            </button>
            <button
              type="button"
              onClick={clearSaved}
              className="inline-flex items-center gap-1.5 rounded-full border border-border bg-bg px-3 py-1.5 text-xs font-medium text-tx-2 hover:border-err/40 hover:text-err transition-colors"
            >
              Clear saved peer
            </button>
          </div>
        </div>

        <div className="space-y-4 rounded-[1.5rem] border border-border bg-bg-card p-5">
          <div className="flex items-center gap-2 text-tx-1">
            <ArrowRight size={14} className="text-accent" />
            <p className="text-sm font-semibold">How plan mode uses this</p>
          </div>
          <div className="space-y-3 text-sm leading-6 text-tx-2">
            <p>When a plan asks for ACP, the chat UI opens this settings surface or the inline ACP card.</p>
            <p>Saving here keeps the peer details available for your next plan-mode session.</p>
            <p>The backend ACP integration can be wired later without changing the UX flow.</p>
          </div>
        </div>
      </div>
    </div>
  );
}
