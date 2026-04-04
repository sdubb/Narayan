import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { ArrowLeft, ChevronDown, GitBranch, CheckCircle2, AlertCircle } from 'lucide-react';
import { agents as agentsApi } from '../../api';

const STATUS_STYLES = {
  pending:    { border: 'border-border',    dot: 'bg-tx-4',   line: 'stroke-border' },
  running:    { border: 'border-accent',    dot: 'bg-accent', line: 'stroke-accent' },
  completed:  { border: 'border-ok',        dot: 'bg-ok',     line: 'stroke-ok' },
  failed:     { border: 'border-err',       dot: 'bg-err',    line: 'stroke-err' },
  delegating: { border: 'border-vio',       dot: 'bg-vio',    line: 'stroke-vio' },
};

function ChildNode({ child, selected, onClick, index, total }) {
  const styles = STATUS_STYLES[child.status] || STATUS_STYLES.pending;
  const isActive = ['running', 'delegating'].includes(child.status);

  return (
    <motion.button
      onClick={onClick}
      className={clsx(
        'rounded-xl border-2 bg-bg-card p-3 w-40 text-left transition-shadow',
        styles.border,
        selected && 'shadow-md ring-2 ring-accent/20',
      )}
      initial={{ scale: 0, opacity: 0 }}
      animate={{ scale: 1, opacity: 1 }}
      transition={{ delay: 0.1 + index * 0.1, type: 'spring', stiffness: 300 }}
    >
      <p className="text-xs font-medium text-tx-1 truncate mb-1">{child.goal}</p>
      <div className="flex items-center gap-1.5">
        <span className={clsx('size-2 rounded-full', styles.dot, isActive && 'animate-pulse-dot')} />
        <span className="text-[10px] text-tx-3 capitalize">{child.status}</span>
        <span className="text-[10px] font-mono text-tx-4 ml-auto">
          {child.current_step || 0}/{child.step_count || '?'}
        </span>
      </div>
    </motion.button>
  );
}

export default function SwarmCanvas({ parentAgent, onBack }) {
  const [children, setChildren] = useState([]);
  const [selectedChildId, setSelectedChildId] = useState(null);
  const [childDetail, setChildDetail] = useState(null);
  const hasParentAgent = !!parentAgent?.id;

  useEffect(() => {
    if (!hasParentAgent) return;
    let cancelled = false;
    const refresh = async () => {
      try {
        const data = await agentsApi.children(parentAgent.id);
        if (!cancelled) {
          setChildren(data.children || []);
          if (!selectedChildId && data.children?.length > 0) setSelectedChildId(data.children[0].id);
        }
      } catch {}
    };
    refresh();
    const iv = setInterval(refresh, 3000);
    return () => { cancelled = true; clearInterval(iv); };
  }, [hasParentAgent, parentAgent?.id]);

  useEffect(() => {
    if (!hasParentAgent || !selectedChildId) return;
    agentsApi.get(selectedChildId).then(setChildDetail).catch(() => {});
  }, [hasParentAgent, selectedChildId]);

  if (!hasParentAgent) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-center">
        <div>
          <p className="text-sm font-medium text-tx-1 mb-1">No live swarm to inspect</p>
          <p className="text-xs text-tx-3 max-w-sm leading-relaxed">
            Once a role is running, this view will show the current runtime agent and its children.
          </p>
        </div>
      </div>
    );
  }

  const allComplete = children.length > 0 && children.every(c => c.status === 'completed' || c.status === 'failed');

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-border">
        <button onClick={onBack} className="flex items-center gap-1.5 text-xs text-tx-3 hover:text-tx-1 transition-colors">
          <ArrowLeft size={14} /> Back to timeline
        </button>
        <span className="text-xs font-medium text-tx-2 flex-1">
          Swarm: {parentAgent?.goal?.slice(0, 40) || 'Agent'}
        </span>
        {allComplete && (
          <span className="badge bg-ok-soft text-ok border border-ok/20">
            <CheckCircle2 size={10} /> All complete
          </span>
        )}
      </div>

      {/* Canvas */}
      <div className="flex-1 overflow-auto p-6">
        {/* Parent node */}
        <div className="flex justify-center mb-8">
          <div className={clsx('rounded-xl border-2 bg-bg-card p-4 w-48 text-center', STATUS_STYLES[parentAgent?.status]?.border || 'border-border')}>
            <GitBranch size={16} className="text-vio mx-auto mb-1" />
            <p className="text-xs font-semibold text-tx-1 truncate">{parentAgent?.goal?.slice(0, 30) || 'Parent'}</p>
            <p className="text-[10px] text-tx-3 mt-0.5 capitalize">{parentAgent?.status} · delegating</p>
          </div>
        </div>

        {/* SVG lines */}
        <svg className="w-full h-8 -mt-4 mb-2" preserveAspectRatio="none">
          <line x1="50%" y1="0" x2="50%" y2="100%" className="stroke-border" strokeWidth="1" strokeDasharray="4" />
        </svg>

        {/* Children nodes */}
        <div className="flex flex-wrap justify-center gap-4">
          {children.map((child, i) => (
            <ChildNode
              key={child.id}
              child={child}
              index={i}
              total={children.length}
              selected={child.id === selectedChildId}
              onClick={() => setSelectedChildId(child.id)}
            />
          ))}
        </div>
      </div>

      {/* Child detail feed */}
      {selectedChildId && childDetail && (
        <div className="border-t border-border max-h-64 overflow-y-auto px-4 py-3">
          <div className="flex items-center gap-2 mb-2">
            <span className="text-xs font-medium text-tx-2">
              {childDetail.goal?.slice(0, 40) || 'Child agent'}
            </span>
            <select
              value={selectedChildId}
              onChange={e => setSelectedChildId(e.target.value)}
              className="ml-auto text-[11px] text-tx-3 border border-border rounded px-1.5 py-0.5 bg-bg-card"
            >
              {children.map(c => (
                <option key={c.id} value={c.id}>{c.goal?.slice(0, 25) || c.id.slice(0, 12)}</option>
              ))}
            </select>
          </div>
          <div className="text-xs text-tx-3">
            Status: <span className="capitalize font-medium text-tx-2">{childDetail.status}</span>
            {' · '}Step {childDetail.current_step || 0}
            {childDetail.final_answer && (
              <p className="mt-2 text-tx-2 leading-relaxed">{childDetail.final_answer}</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
