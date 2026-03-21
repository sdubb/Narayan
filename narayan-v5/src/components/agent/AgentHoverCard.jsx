import { useState, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';

function formatDuration(ms) {
  if (!ms) return '-';
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  if (m > 0) return `${m}m ${s % 60}s`;
  return `${s}s`;
}

export default function AgentHoverCard({ agent, children }) {
  const [show, setShow] = useState(false);
  const timeout = useRef(null);

  const onEnter = () => { timeout.current = setTimeout(() => setShow(true), 300); };
  const onLeave = () => { clearTimeout(timeout.current); setShow(false); };

  if (!agent) return children;

  const runtime = agent.started_at ? Date.now() - new Date(agent.started_at).getTime() : null;

  const rows = [
    ['Status', agent.status],
    agent.metadata?.job_type && ['Type', agent.metadata.job_type.replace(/_/g, ' ')],
    ['Step', agent.plan?.steps ? `${agent.current_step || 0} of ${agent.plan.steps.length}` : `${agent.current_step || 0}`],
    runtime && ['Runtime', formatDuration(runtime)],
    agent.parent_agent_id && ['Parent', agent.parent_agent_id.slice(0, 12)],
    agent.pending_children?.length > 0 && ['Children', agent.pending_children.length],
  ].filter(Boolean);

  return (
    <span className="relative inline-flex" onMouseEnter={onEnter} onMouseLeave={onLeave}>
      {children}
      <AnimatePresence>
        {show && (
          <motion.div
            className="absolute left-full top-0 ml-2 z-50 w-52 rounded-xl border border-border bg-bg-card shadow-md p-3 space-y-2"
            initial={{ opacity: 0, scale: 0.95, x: -4 }}
            animate={{ opacity: 1, scale: 1, x: 0 }}
            exit={{ opacity: 0, scale: 0.95 }}
            transition={{ duration: 0.12 }}
          >
            <p className="text-xs font-medium text-tx-1 truncate">{agent.goal}</p>
            <div className="space-y-1">
              {rows.map(([label, value]) => (
                <div key={label} className="flex items-center justify-between text-[11px]">
                  <span className="text-tx-3">{label}</span>
                  <span className="font-mono text-tx-2">{value}</span>
                </div>
              ))}
            </div>
            {agent.current_task && (
              <div className="pt-1.5 border-t border-border/60">
                <p className="text-[10px] text-tx-4">Current</p>
                <p className="text-xs text-tx-2 font-mono truncate">{agent.current_task}</p>
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </span>
  );
}
