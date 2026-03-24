import { motion } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, AlertCircle } from 'lucide-react';

const PHASES = ['preflight', 'planning', 'execution', 'done'];
const PHASE_LABELS = { preflight: 'Preflight', planning: 'Planning', execution: 'Execution', done: 'Done' };

function getPhaseStatus(phase, groupedEvents) {
  if (!groupedEvents) return 'future';
  const { preflight, plan, steps, terminal } = groupedEvents;

  if (phase === 'preflight') {
    if (preflight?.failed) return 'failed';
    if (preflight?.passed) return 'completed';
    if (preflight?.started) return 'active';
    return 'future';
  }
  if (phase === 'planning') {
    if (!preflight?.passed) return 'future';
    // While the user is reviewing the plan, keep Planning as "active" (pulsing
    // amber) so it's clear we're still in the planning gate, not in execution.
    if (plan?.approvalNeeded || plan?.replanning) return 'active';
    if (plan?.stepCount > 0) return 'completed';
    if (preflight?.passed && !plan?.stepCount) return 'active';
    return 'future';
  }
  if (phase === 'execution') {
    if (!plan?.stepCount) return 'future';
    if (terminal?.type) return terminal.type === 'failed' ? 'failed' : 'completed';
    if (steps?.length > 0) return 'active';
    return 'future';
  }
  if (phase === 'done') {
    if (terminal?.type === 'complete') return 'completed';
    if (terminal?.type === 'failed') return 'failed';
    return 'future';
  }
  return 'future';
}

function getStepProgress(groupedEvents) {
  if (!groupedEvents?.steps) return '';
  const completed = groupedEvents.steps.filter(s => s.completed).length;
  const total = groupedEvents.plan?.stepCount || groupedEvents.steps.length;
  return `${completed}/${total}`;
}

export default function PhaseBar({ groupedEvents, onPhaseClick }) {
  return (
    <div className="sticky top-0 z-10 flex items-center gap-1 px-4 py-2.5 bg-bg-card/90 backdrop-blur border-b border-border">
      {PHASES.map((phase, i) => {
        const status = getPhaseStatus(phase, groupedEvents);
        const isExecution = phase === 'execution';
        const progress = isExecution ? getStepProgress(groupedEvents) : '';

        return (
          <button
            key={phase}
            onClick={() => onPhaseClick?.(i)}
            className="flex items-center gap-1.5 group"
          >
            {/* Connector line */}
            {i > 0 && (
              <div className={clsx(
                'w-6 h-px',
                status === 'future' ? 'bg-border' : 'bg-ok/40',
              )} />
            )}

            {/* Circle */}
            <span className="relative flex items-center justify-center">
              {status === 'completed' ? (
                <CheckCircle2 size={16} className="text-ok" />
              ) : status === 'failed' ? (
                <AlertCircle size={16} className="text-err" />
              ) : status === 'active' ? (
                <span className="relative">
                  <span className="size-4 rounded-full bg-accent flex items-center justify-center">
                    <span className="size-1.5 rounded-full bg-white" />
                  </span>
                  <motion.span
                    className="absolute inset-0 rounded-full bg-accent"
                    animate={{ scale: [1, 1.6, 1], opacity: [0.4, 0, 0.4] }}
                    transition={{ duration: 1.4, repeat: Infinity, ease: 'easeInOut' }}
                  />
                </span>
              ) : (
                <span className="size-4 rounded-full border-2 border-border" />
              )}
            </span>

            {/* Label */}
            <span className={clsx(
              'text-[11px] font-medium whitespace-nowrap',
              status === 'active' ? 'text-accent' :
              status === 'completed' ? 'text-ok' :
              status === 'failed' ? 'text-err' : 'text-tx-4',
            )}>
              {PHASE_LABELS[phase]}
              {progress && ` ${progress}`}
            </span>
          </button>
        );
      })}
    </div>
  );
}
