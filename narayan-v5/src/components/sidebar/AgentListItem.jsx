import clsx from 'clsx';
import { motion } from 'framer-motion';
import Sparkline from './Sparkline';

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

const isActive = s => ['running', 'preflight', 'delegating'].includes(s);

function timeAgo(iso) {
  if (!iso) return '';
  const d = Date.now() - new Date(iso).getTime();
  const h = Math.floor(d / 3600000), m = Math.floor((d % 3600000) / 60000);
  if (h > 0) return `${h}h ago`;
  if (m > 0) return `${m}m ago`;
  return 'just now';
}

export default function AgentListItem({ conversation, selected, latestStatus, onClick }) {
  const cfg = STATUS[latestStatus] || STATUS.pending;
  const title = conversation.title || 'New conversation';
  const sparkColor = latestStatus === 'failed' ? '#ef4444' : latestStatus === 'completed' ? '#22c55e' : '#f59e0b';

  return (
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
        <span className={clsx('size-1.5 rounded-full shrink-0', cfg.dot, isActive(latestStatus) && 'animate-pulse-dot')} />
        <p className="text-xs font-medium text-tx-1 truncate flex-1">{title}</p>
      </div>
      <div className="flex items-center gap-2 text-[10px] text-tx-4 pl-3.5">
        <span>{cfg.label}</span>
        <Sparkline data={[]} color={sparkColor} width={36} height={12} />
        <span className="ml-auto">{timeAgo(conversation.updated_at)}</span>
      </div>
    </motion.button>
  );
}
