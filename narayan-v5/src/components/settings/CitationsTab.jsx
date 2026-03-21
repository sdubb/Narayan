import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { Link2, Loader2, ChevronDown, Search } from 'lucide-react';
import { citations } from '../../api';

export default function CitationsTab() {
  const [list, setList] = useState([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState(null);
  const [filter, setFilter] = useState('');

  useEffect(() => {
    citations.all().then(d => setList(d.citations || [])).catch(() => {}).finally(() => setLoading(false));
  }, []);

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  const filtered = filter ? list.filter(c =>
    c.claim?.toLowerCase().includes(filter.toLowerCase()) ||
    c.source_ref?.toLowerCase().includes(filter.toLowerCase()) ||
    c.agent_id?.includes(filter)
  ) : list;

  return (
    <div className="space-y-4">
      <div className="relative">
        <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-tx-4" />
        <input value={filter} onChange={e => setFilter(e.target.value)} placeholder="Filter citations..." className="input-field text-sm pl-9" />
      </div>

      {filtered.length === 0 ? (
        <div className="text-center py-8">
          <Link2 size={24} className="text-tx-4 mx-auto mb-2" />
          <p className="text-sm text-tx-3">No citations recorded</p>
        </div>
      ) : (
        <div className="card overflow-hidden divide-y divide-border/60">
          {/* Header */}
          <div className="grid grid-cols-12 gap-2 px-4 py-2.5 bg-bg text-[10px] font-semibold text-tx-4 uppercase tracking-wider">
            <span className="col-span-1">Step</span>
            <span className="col-span-4">Claim</span>
            <span className="col-span-2">Source</span>
            <span className="col-span-2">Type</span>
            <span className="col-span-2">Confidence</span>
            <span className="col-span-1" />
          </div>

          {filtered.map((c, i) => {
            const conf = c.confidence ?? 0;
            const barColor = conf >= 0.8 ? 'bg-ok' : conf >= 0.5 ? 'bg-warn' : 'bg-err';
            const textColor = conf >= 0.8 ? 'text-ok' : conf >= 0.5 ? 'text-warn' : 'text-err';
            return (
              <div key={i}>
                <button onClick={() => setExpanded(expanded === i ? null : i)}
                  className="w-full grid grid-cols-12 gap-2 px-4 py-3 text-xs hover:bg-bg-hover transition-colors items-center">
                  <span className="col-span-1 font-mono text-tx-4">{c.step_index ?? '-'}</span>
                  <span className="col-span-4 text-tx-1 text-left truncate">{c.claim}</span>
                  <span className="col-span-2 font-mono text-tx-3 truncate">{c.source_ref}</span>
                  <span className="col-span-2 text-tx-3 capitalize">{c.source_type}</span>
                  <span className="col-span-2 flex items-center gap-1">
                    <div className="w-12 h-1.5 rounded-full bg-bg-active overflow-hidden">
                      <div className={clsx('h-full rounded-full', barColor)} style={{ width: `${conf * 100}%` }} />
                    </div>
                    <span className={clsx('font-mono', textColor)}>{Math.round(conf * 100)}%</span>
                  </span>
                  <span className="col-span-1 flex justify-end">
                    <ChevronDown size={12} className={clsx('text-tx-4 transition-transform', expanded === i && 'rotate-180')} />
                  </span>
                </button>
                {expanded === i && (
                  <div className="px-4 pb-3 space-y-1 bg-bg-hover/50">
                    <p className="text-xs text-tx-2">{c.claim}</p>
                    {c.excerpt && <p className="text-[11px] text-tx-3 italic">"{c.excerpt}"</p>}
                    <div className="flex gap-4 text-[10px] text-tx-4">
                      <span>Agent: {c.agent_id?.slice(0, 12)}</span>
                      {c.created_at && <span>{new Date(c.created_at).toLocaleString()}</span>}
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
