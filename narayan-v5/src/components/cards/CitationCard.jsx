import { motion } from 'framer-motion';
import clsx from 'clsx';
import { Link2 } from 'lucide-react';

export default function CitationCard({ event, compact = false }) {
  const confidence = event.confidence ?? 0;
  const barColor = confidence >= 0.8 ? 'bg-ok' : confidence >= 0.5 ? 'bg-warn' : 'bg-err';
  const textColor = confidence >= 0.8 ? 'text-ok' : confidence >= 0.5 ? 'text-warn' : 'text-err';

  if (compact) {
    return (
      <div className="flex items-center gap-2 rounded-lg border border-vio/20 bg-vio-soft/30 px-3 py-2">
        <Link2 size={11} className="text-vio shrink-0" />
        <span className="text-xs text-tx-2 flex-1 truncate">{event.claim || 'Citation'}</span>
        <span className="text-[10px] font-mono text-tx-4">{event.source_type || 'tool_output'}</span>
        <div className="w-10 h-1.5 rounded-full bg-bg-active overflow-hidden">
          <div className={clsx('h-full rounded-full', barColor)} style={{ width: `${confidence * 100}%` }} />
        </div>
        <span className={clsx('text-[10px] font-mono', textColor)}>{Math.round(confidence * 100)}%</span>
      </div>
    );
  }

  return (
    <motion.div
      className="rounded-xl border border-vio/20 bg-vio-soft/20 overflow-hidden shadow-sm"
      initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.15 }}
    >
      <div className="flex items-center gap-2 px-3.5 py-2.5 border-b border-vio/15">
        <Link2 size={12} className="text-vio shrink-0" />
        <span className="text-xs font-bold tracking-wider uppercase text-vio">
          Citation — step {event.step_index ?? '?'}
        </span>
      </div>
      <div className="px-3.5 py-3 space-y-2">
        <p className="text-xs text-tx-1 leading-relaxed">{event.claim}</p>
        <div className="flex items-center gap-3">
          <span className="text-[10px] font-mono text-tx-4">{event.source_ref || 'Unknown source'}</span>
          <span className="text-[10px] text-tx-4 capitalize">{event.source_type || 'tool_output'}</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-[10px] text-tx-4">Confidence:</span>
          <div className="flex-1 h-2 rounded-full bg-bg-active overflow-hidden">
            <motion.div
              className={clsx('h-full rounded-full', barColor)}
              initial={{ width: 0 }}
              animate={{ width: `${confidence * 100}%` }}
              transition={{ duration: 0.4, ease: 'easeOut' }}
            />
          </div>
          <span className={clsx('text-xs font-mono font-medium', textColor)}>
            {Math.round(confidence * 100)}%
          </span>
        </div>
      </div>
    </motion.div>
  );
}
