import { useState } from 'react';
import { useEffect } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, Loader2, Shield, Sparkles, TestTube2, ArrowRight } from 'lucide-react';
import { readAcpPeerConfig, writeAcpPeerConfig } from '../settings/acpStorage';

export default function ACPConnectionCard({ onConnected, onOpenSettings }) {
  const saved = readAcpPeerConfig();
  const [name, setName] = useState(saved.name || '');
  const [peerUrl, setPeerUrl] = useState(saved.peer_url || '');
  const [token, setToken] = useState(saved.token || '');
  const [summary, setSummary] = useState(saved.summary || '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');

  useEffect(() => {
    const next = readAcpPeerConfig();
    setName(next.name || '');
    setPeerUrl(next.peer_url || '');
    setToken(next.token || '');
    setSummary(next.summary || '');
  }, []);

  const canContinue = name.trim() && peerUrl.trim() && !saving;

  async function handleContinue() {
    if (!canContinue) return;
    setSaving(true);
    setError('');
    setSuccess('');
    try {
      const savedName = name.trim();
      writeAcpPeerConfig({
        name: savedName,
        peer_url: peerUrl.trim(),
        token: token.trim(),
        summary: summary.trim(),
      });
      setSuccess(`Captured ACP peer ${savedName}. Returning to plan mode...`);
      onConnected?.({
        name: savedName,
        peer_url: peerUrl.trim(),
        token: token.trim(),
        summary: summary.trim(),
      });
    } catch (e) {
      setError(e.message || 'Failed to capture ACP setup');
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
        <Shield size={14} className="text-accent" />
        <span className="text-sm font-semibold text-accent">Connect your ACP peer inline</span>
      </div>

      <div className="px-4 py-4 space-y-4">
        <div className="flex items-start gap-2.5">
          <Sparkles size={14} className="mt-0.5 text-accent shrink-0" />
          <div className="space-y-1">
            <p className="text-sm text-tx-1 font-medium">Capture the ACP peer details and keep going</p>
            <p className="text-xs text-tx-3">
              Add the peer endpoint now. If you still need to register or configure it fully, open Settings after this step.
            </p>
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-tx-2">Peer name</label>
            <input
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder="customer_ops_peer"
              className="input-field"
              autoComplete="off"
            />
          </div>
          <div className="space-y-1.5">
            <label className="text-xs font-medium text-tx-2">Peer URL</label>
            <input
              value={peerUrl}
              onChange={e => setPeerUrl(e.target.value)}
              placeholder="https://peer.example.com/acp"
              className="input-field"
              autoComplete="off"
            />
          </div>
          <div className="space-y-1.5 sm:col-span-2">
            <label className="text-xs font-medium text-tx-2">Handshake token (optional)</label>
            <input
              type="password"
              value={token}
              onChange={e => setToken(e.target.value)}
              placeholder="token or secret"
              className="input-field"
              autoComplete="off"
            />
          </div>
          <div className="space-y-1.5 sm:col-span-2">
            <label className="text-xs font-medium text-tx-2">Summary (optional)</label>
            <textarea
              value={summary}
              onChange={e => setSummary(e.target.value)}
              placeholder="What this ACP peer can do..."
              className="input-field min-h-[88px] resize-none"
            />
          </div>
        </div>

        {error && <p className="text-xs text-err">{error}</p>}
        {success && (
          <div className="flex items-center gap-2 rounded-lg border border-ok/20 bg-ok-soft/30 px-3 py-2 text-xs text-ok">
            <CheckCircle2 size={12} />
            {success}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={handleContinue}
            disabled={!canContinue}
            className={clsx('btn-primary flex items-center gap-2 disabled:opacity-50', saving && 'cursor-wait')}
          >
            {saving ? <Loader2 size={12} className="animate-spin" /> : <TestTube2 size={12} />}
            Use this ACP peer
          </button>
          <button
            type="button"
            onClick={() => onOpenSettings?.({ backendKind: 'acp', name: name.trim(), peerUrl: peerUrl.trim(), token: token.trim(), summary: summary.trim() })}
            className="inline-flex items-center gap-1.5 rounded-full border border-border bg-bg px-3 py-1.5 text-xs font-medium text-tx-2 hover:border-accent/40 hover:text-accent transition-colors"
          >
            Open ACP settings <ArrowRight size={11} />
          </button>
        </div>
      </div>
    </motion.div>
  );
}
