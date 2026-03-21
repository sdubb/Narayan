import { motion } from 'framer-motion';
import clsx from 'clsx';

const statusConfig = {
  pending:          { dot: 'bg-tx-4',       label: 'Pending' },
  preflight:        { dot: 'bg-info',       label: 'Preflight' },
  clarifying:       { dot: 'bg-warn',       label: 'Clarifying' },
  planning:         { dot: 'bg-info',       label: 'Planning' },
  waiting_approval: { dot: 'bg-warn',       label: 'Awaiting Approval' },
  running:          { dot: 'bg-accent-glow', label: 'Running' },
  waiting:          { dot: 'bg-vio',        label: 'Waiting' },
  delegating:       { dot: 'bg-vio',        label: 'Delegating' },
  paused:           { dot: 'bg-tx-3',       label: 'Paused' },
  completed:        { dot: 'bg-ok',         label: 'Completed' },
  failed:           { dot: 'bg-err',        label: 'Failed' },
};

export default function StatusBadge({ status, size = 'sm', showLabel = true, className }) {
  const config = statusConfig[status] || statusConfig.pending;
  const isActive = ['running', 'preflight', 'planning', 'delegating'].includes(status);

  return (
    <span className={clsx('inline-flex items-center gap-1.5', className)}>
      <span className="relative flex">
        <span className={clsx(
          'rounded-full',
          config.dot,
          size === 'xs' && 'w-1.5 h-1.5',
          size === 'sm' && 'w-2 h-2',
          size === 'md' && 'w-2.5 h-2.5',
        )} />
        {isActive && (
          <motion.span
            className={clsx('absolute inset-0 rounded-full', config.dot)}
            animate={{ scale: [1, 1.8, 1], opacity: [0.6, 0, 0.6] }}
            transition={{ duration: 1.4, repeat: Infinity, ease: 'easeInOut' }}
          />
        )}
      </span>
      {showLabel && (
        <span className={clsx(
          'font-medium text-tx-3',
          size === 'xs' && 'text-[0.625rem]',
          size === 'sm' && 'text-xs',
          size === 'md' && 'text-sm',
        )}>
          {config.label}
        </span>
      )}
    </span>
  );
}
