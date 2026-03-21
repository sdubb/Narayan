import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { Bell, CheckCircle2, AlertCircle, Loader2 } from 'lucide-react';
import { reviews } from '../../api';

export default function ReviewsTab({ onFlash }) {
  const [list, setList] = useState([]);
  const [loading, setLoading] = useState(true);
  const [resolving, setResolving] = useState(null);

  useEffect(() => {
    reviews.list().then(d => setList(d.reviews || [])).catch(() => {}).finally(() => setLoading(false));
  }, []);

  async function resolve(id, status) {
    setResolving(id);
    try {
      await reviews.resolve(id, status, `Resolved (${status}) from settings`);
      setList(l => l.map(r => r.id === id ? { ...r, status } : r));
      onFlash?.(`Review ${status}`);
    } catch (e) { onFlash?.(e.message); }
    finally { setResolving(null); }
  }

  async function resolveAll(status) {
    try {
      await reviews.resolveAll(status, 'Bulk resolved from settings');
      setList(l => l.map(r => r.status === 'pending' ? { ...r, status } : r));
      onFlash?.('All reviews resolved');
    } catch (e) { onFlash?.(e.message); }
  }

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  const pending = list.filter(r => r.status === 'pending');
  const resolved = list.filter(r => r.status !== 'pending');

  return (
    <div className="space-y-6">
      {pending.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-3">
            <p className="text-sm font-semibold text-tx-1">Pending ({pending.length})</p>
            <button onClick={() => resolveAll('approved')} className="btn-primary text-xs px-3 py-1.5">Approve all</button>
          </div>
          <div className="space-y-3">
            {pending.map(r => (
              <div key={r.id} className="card border-l-4 border-l-warn p-4">
                <p className="text-sm text-tx-1 mb-1">{r.summary || 'Pending review'}</p>
                {r.reason && <p className="text-xs text-tx-3 mb-3">{r.reason}</p>}
                <div className="flex items-center gap-2">
                  <button onClick={() => resolve(r.id, 'approved')} disabled={resolving === r.id}
                    className="rounded-lg border border-ok/30 bg-ok-soft px-3 py-1.5 text-xs font-medium text-ok hover:border-ok/50 transition-all disabled:opacity-50">
                    {resolving === r.id ? <Loader2 size={11} className="animate-spin" /> : <CheckCircle2 size={11} />} Approve
                  </button>
                  <button onClick={() => resolve(r.id, 'rejected')} disabled={resolving === r.id}
                    className="rounded-lg border border-err/30 bg-err-soft px-3 py-1.5 text-xs font-medium text-err hover:border-err/50 transition-all disabled:opacity-50">
                    <AlertCircle size={11} /> Reject
                  </button>
                  <span className="text-[10px] text-tx-4 ml-auto font-mono">{r.agent_id?.slice(0, 12)}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {resolved.length > 0 && (
        <div>
          <p className="text-sm font-semibold text-tx-1 mb-3">Resolved ({resolved.length})</p>
          <div className="space-y-1">
            {resolved.map(r => (
              <div key={r.id} className="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-bg-hover transition-colors">
                <span className={clsx('size-2 rounded-full', r.status === 'approved' ? 'bg-ok' : 'bg-err')} />
                <span className="text-xs text-tx-2 flex-1 truncate">{r.summary || r.id}</span>
                <span className="text-[10px] text-tx-4 capitalize">{r.status}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {list.length === 0 && (
        <div className="text-center py-8">
          <Bell size={24} className="text-tx-4 mx-auto mb-2" />
          <p className="text-sm text-tx-3">No reviews yet</p>
        </div>
      )}
    </div>
  );
}
