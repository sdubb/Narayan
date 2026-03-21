import { useState } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { Bell, CheckCircle2, AlertCircle, RotateCcw, Zap, Loader2, ChevronDown } from 'lucide-react';
import { autoApprovals } from '../../api';

const RESOLUTION_LABELS = {
  approved: { label: 'Approved', color: 'ok' },
  auto_approved: { label: 'Auto-approved', color: 'ok' },
  changes_requested: { label: 'Changes requested', color: 'warn' },
  rejected: { label: 'Rejected', color: 'err' },
};

export default function ReviewCard({ event }) {
  const [resolving, setResolving] = useState(false);
  const [resolved, setResolved] = useState(false);
  const [resolution, setResolution] = useState(null);
  const [showNote, setShowNote] = useState(false);
  const [note, setNote] = useState('');
  const [err, setErr] = useState('');

  async function resolve(status, noteOverride) {
    setResolving(true); setErr('');
    try {
      const token = localStorage.getItem('narayan_token');
      const BASE = import.meta.env.VITE_API_URL || '/api';
      const finalNote = noteOverride ?? (note.trim() || `Resolved (${status}) from UI`);
      await fetch(`${BASE}/reviews/${event.review_id}/resolve`, {
        method: 'POST',
        headers: { 'Authorization': `Bearer ${token}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ status: status === 'auto_approved' ? 'approved' : status, notes: finalNote }),
      });
      setResolution(status);
      setResolved(true);
    } catch (e) { setErr(e.message); }
    finally { setResolving(false); }
  }

  if (resolved) {
    const r = RESOLUTION_LABELS[resolution] || { label: 'Resolved', color: 'ok' };
    const bgCls = r.color === 'ok' ? 'border-ok/25 bg-ok-soft' : r.color === 'warn' ? 'border-warn/25 bg-warn-soft' : 'border-err/25 bg-err-soft';
    const textCls = r.color === 'ok' ? 'text-ok' : r.color === 'warn' ? 'text-warn' : 'text-err';
    return (
      <motion.div className={clsx('rounded-xl border p-4 flex items-center gap-3', bgCls)}
        initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
        <CheckCircle2 size={16} className={clsx(textCls, 'shrink-0')} />
        <span className={clsx('text-sm font-medium', textCls)}>Review {r.label} — agent resuming</span>
      </motion.div>
    );
  }

  return (
    <motion.div
      className="rounded-xl border-l-4 border-l-warn border border-warn/25 bg-warn-soft/30 overflow-hidden shadow-sm"
      initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }}
    >
      <div className="flex items-center gap-2 px-4 py-3 border-b border-warn/15">
        <Bell size={14} className="text-warn" />
        <span className="text-sm font-semibold text-warn">Human review required</span>
      </div>
      <div className="px-4 py-3 space-y-3">
        <p className="text-sm text-tx-1">{event.summary || 'Review item created'}</p>
        {event.reason && <p className="text-xs text-tx-3">Rule: {event.reason}</p>}

        <button onClick={() => setShowNote(o => !o)}
          className="flex items-center gap-1.5 text-xs text-tx-3 hover:text-tx-2 transition-colors">
          <ChevronDown size={10} className={clsx('transition-transform', !showNote && '-rotate-90')} />
          {showNote ? 'Hide note' : 'Add note'}
        </button>
        {showNote && (
          <textarea value={note} onChange={e => setNote(e.target.value)} rows={2} placeholder="Instructions for the agent..."
            className="input-field text-xs resize-none" />
        )}

        {err && <p className="text-xs text-err">{err}</p>}

        <div className="grid grid-cols-2 gap-2">
          <button onClick={async () => {
            autoApprovals.create(event.rule_id || event.reason || 'unknown', `Auto-approved from chat`).catch(() => {});
            resolve('auto_approved', 'Auto-approved: rule saved');
          }} disabled={resolving}
            className="flex flex-col items-start gap-0.5 rounded-xl border border-ok/30 bg-ok-soft px-3 py-2.5 hover:border-ok/50 transition-all disabled:opacity-50 text-left">
            <div className="flex items-center gap-1.5">
              <Zap size={11} className="text-ok" />
              <span className="text-xs font-semibold text-ok">Auto-approve</span>
            </div>
            <span className="text-[10px] text-ok/70">Don't ask again for this rule</span>
          </button>

          <button onClick={() => resolve('approved')} disabled={resolving}
            className="flex flex-col items-start gap-0.5 rounded-xl border border-ok/30 bg-ok-soft px-3 py-2.5 hover:border-ok/50 transition-all disabled:opacity-50 text-left">
            <div className="flex items-center gap-1.5">
              {resolving ? <Loader2 size={11} className="text-ok animate-spin" /> : <CheckCircle2 size={11} className="text-ok" />}
              <span className="text-xs font-semibold text-ok">Approve</span>
            </div>
            <span className="text-[10px] text-ok/70">Proceed this time</span>
          </button>

          <button onClick={() => { if (!note.trim()) { setShowNote(true); return; } resolve('changes_requested'); }}
            disabled={resolving}
            className="flex flex-col items-start gap-0.5 rounded-xl border border-warn/30 bg-warn-soft px-3 py-2.5 hover:border-warn/50 transition-all disabled:opacity-50 text-left">
            <div className="flex items-center gap-1.5">
              <RotateCcw size={11} className="text-warn" />
              <span className="text-xs font-semibold text-warn">Request changes</span>
            </div>
            <span className="text-[10px] text-warn/70">Retry with your note</span>
          </button>

          <button onClick={() => resolve('rejected')} disabled={resolving}
            className="flex flex-col items-start gap-0.5 rounded-xl border border-err/30 bg-err-soft px-3 py-2.5 hover:border-err/50 transition-all disabled:opacity-50 text-left">
            <div className="flex items-center gap-1.5">
              <AlertCircle size={11} className="text-err" />
              <span className="text-xs font-semibold text-err">Reject</span>
            </div>
            <span className="text-[10px] text-err/70">Block this action</span>
          </button>
        </div>
      </div>
    </motion.div>
  );
}
