import { useState } from 'react';
import clsx from 'clsx';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronRight, XCircle, Loader2 } from 'lucide-react';
import Sparkline from './Sparkline';
import { agents as agentsApi } from '../../api';

const STATUS = {
  pending:    { dot: 'bg-tx-4',       label: 'Pending' },
  preflight:  { dot: 'bg-info',       label: 'Preflight' },
  clarifying: { dot: 'bg-warn',       label: 'Clarifying' },
  running:    { dot: 'bg-ok',         label: 'Running' },
  waiting:    { dot: 'bg-info',       label: 'Scheduled' },
  delegating: { dot: 'bg-vio',        label: 'Delegating' },
  paused:     { dot: 'bg-warn',       label: 'Paused' },
  completed:  { dot: 'bg-ok',         label: 'Done' },
  failed:     { dot: 'bg-err',        label: 'Failed' },
};

const ACTIVE_STATUSES = new Set(['pending', 'running', 'waiting', 'clarifying', 'delegating', 'preflight', 'paused']);
const isActive = s => ACTIVE_STATUSES.has(s);

function timeAgo(iso) {
  if (!iso) return '';
  const d = Date.now() - new Date(iso).getTime();
  const h = Math.floor(d / 3600000), m = Math.floor((d % 3600000) / 60000);
  if (h > 0) return `${h}h ago`;
  if (m > 0) return `${m}m ago`;
  return 'just now';
}

function AgentSubItem({ agent, onCancelled }) {
  const [cancelling, setCancelling] = useState(false);
  const st = agent.status || 'pending';
  const cfg = STATUS[st] || STATUS.pending;
  const canCancel = isActive(st);

  async function handleCancel(e) {
    e.stopPropagation();
    setCancelling(true);
    try {
      await agentsApi.cancel(agent.id);
      onCancelled?.(agent.id);
    } catch {
      // silently fail
    } finally {
      setCancelling(false);
    }
  }

  return (
    <motion.div
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: 'auto' }}
      exit={{ opacity: 0, height: 0 }}
      className="flex items-center gap-2 pl-5 pr-2 py-1.5 text-[10px] group"
    >
      <span className={clsx('size-1.5 rounded-full shrink-0', cfg.dot, isActive(st) && 'animate-pulse-dot')} />
      <span className="text-tx-3 truncate flex-1" title={agent.goal}>
        {(agent.goal || 'Agent').substring(0, 40)}
      </span>
      <span className="text-tx-4 shrink-0">{cfg.label}</span>
      {canCancel && (
        <button
          onClick={handleCancel}
          disabled={cancelling}
          className="opacity-0 group-hover:opacity-100 p-0.5 rounded text-err/60 hover:text-err hover:bg-err-soft transition-all shrink-0"
          title="Cancel agent"
        >
          {cancelling ? <Loader2 size={11} className="animate-spin" /> : <XCircle size={11} />}
        </button>
      )}
    </motion.div>
  );
}

export default function AgentListItem({ conversation, selected, latestStatus, onClick, agents: convAgents, onAgentCancelled }) {
  const [expanded, setExpanded] = useState(false);
  const cfg = STATUS[latestStatus] || STATUS.pending;
  const title = conversation.title || 'New conversation';
  const sparkColor = latestStatus === 'failed' ? '#ef4444' : latestStatus === 'completed' ? '#22c55e' : '#f59e0b';
  const agentCount = convAgents?.length || conversation.agent_count || 0;
  const hasAgents = agentCount > 0;
  const activeCount = convAgents?.filter(a => isActive(a.status))?.length || 0;

  return (
    <div>
      <motion.button
        onClick={onClick}
        className={clsx(
          'w-full text-left rounded-lg transition-all',
          selected ? 'bg-bg-active border-l-2 border-l-accent pl-2.5 pr-3 py-2.5' : 'hover:bg-bg-hover px-3 py-2.5',
        )}
        layout
        layoutId={conversation.id}
      >
        <div className="flex items-center gap-2 mb-0.5">
          {hasAgents && (
            <button
              onClick={e => { e.stopPropagation(); setExpanded(p => !p); }}
              className="p-0.5 rounded text-tx-4 hover:text-tx-2 transition-all shrink-0"
            >
              <ChevronRight size={11} className={clsx('transition-transform', expanded && 'rotate-90')} />
            </button>
          )}
          <span className={clsx('size-1.5 rounded-full shrink-0', cfg.dot, isActive(latestStatus) && 'animate-pulse-dot')} />
          <p className="text-xs font-medium text-tx-1 truncate flex-1">{title}</p>
        </div>
        <div className="flex items-center gap-2 text-[10px] text-tx-4 pl-3.5">
          <span>{cfg.label}</span>
          {activeCount > 0 && <span className="text-warn font-mono">{activeCount} active</span>}
          <Sparkline data={[]} color={sparkColor} width={36} height={12} />
          <span className="ml-auto">{timeAgo(conversation.updated_at)}</span>
        </div>
      </motion.button>

      {/* Expandable agent list */}
      <AnimatePresence>
        {expanded && convAgents && convAgents.length > 0 && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="overflow-hidden"
          >
            {convAgents.map(agent => (
              <AgentSubItem
                key={agent.id}
                agent={agent}
                onCancelled={onAgentCancelled}
              />
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
