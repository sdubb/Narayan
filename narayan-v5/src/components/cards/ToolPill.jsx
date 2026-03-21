import { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, AlertCircle, ChevronDown } from 'lucide-react';

export default function ToolPill({ tool }) {
  const [expanded, setExpanded] = useState(false);
  const { name, args_preview, output_preview, success, error } = tool;

  return (
    <div className="inline-flex flex-col">
      <button
        onClick={() => setExpanded(e => !e)}
        className={clsx(
          'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium transition-all',
          'border hover:shadow-sm',
          success === true && 'border-ok/25 bg-ok-soft text-ok',
          success === false && 'border-err/25 bg-err-soft text-err',
          success == null && 'border-border bg-bg-active text-tx-3 animate-pulse',
        )}
      >
        <span className="font-mono">{name}</span>
        {success === true && <CheckCircle2 size={10} />}
        {success === false && <AlertCircle size={10} />}
        {success == null && (
          <motion.span
            className="size-1.5 rounded-full bg-accent"
            animate={{ opacity: [0.4, 1, 0.4] }}
            transition={{ duration: 1.2, repeat: Infinity }}
          />
        )}
        {(output_preview || error) && (
          <ChevronDown size={9} className={clsx('transition-transform', expanded && 'rotate-180')} />
        )}
      </button>

      <AnimatePresence>
        {expanded && (output_preview || error || args_preview) && (
          <motion.div
            className="mt-1.5 rounded-lg border border-border/60 bg-bg-active overflow-hidden"
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.15 }}
          >
            {args_preview && (
              <div className="px-2.5 py-2 border-b border-border/40">
                <span className="text-[10px] font-medium text-tx-4 uppercase tracking-wider">Args</span>
                <pre className="text-[11px] text-tx-2 font-mono mt-1 whitespace-pre-wrap break-all leading-relaxed">
                  {args_preview}
                </pre>
              </div>
            )}
            {(output_preview || error) && (
              <div className="px-2.5 py-2">
                <span className="text-[10px] font-medium text-tx-4 uppercase tracking-wider">
                  {error ? 'Error' : 'Output'}
                </span>
                <pre className={clsx(
                  'text-[11px] font-mono mt-1 whitespace-pre-wrap break-all leading-relaxed',
                  error ? 'text-err' : 'text-tx-2',
                )}>
                  {error || output_preview}
                </pre>
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
