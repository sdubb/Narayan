import { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import { Layers, Cpu, ChevronDown, CheckCircle2 } from 'lucide-react';

function formatLabel(value) {
  return String(value || '')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

export default function PlanCard({ event }) {
  const [open, setOpen] = useState(true);
  const steps = event.steps || [];
  const runtimePolicy = event.runtimePolicy || event.runtime_policy || '';
  const researchSummary = event.researchSummary || event.research_summary || '';

  return (
    <motion.div
      className="rounded-xl border-l-4 border-l-info border border-border bg-bg-card shadow-sm overflow-hidden"
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.25, 0.1, 0.25, 1] }}
    >
      <div className="px-4 py-3 border-b border-border/60 flex items-center gap-2">
        <Layers size={14} className="text-info shrink-0" />
        <span className="text-sm font-semibold text-tx-1">
          Plan — {event.step_count || steps.length} steps
        </span>
        {event.job_type && (
          <span className="badge bg-info-soft text-info border border-info/20">
            <Cpu size={10} />
            {event.job_type.replace(/_/g, ' ')}
          </span>
        )}
      </div>

      <div className="px-4 py-3">
        {event.rationale && (
          <p className="text-xs text-tx-2 leading-relaxed mb-3">{event.rationale}</p>
        )}

        {(runtimePolicy || researchSummary) && (
          <div className="mb-3 space-y-2 rounded-xl border border-border bg-bg px-3 py-2.5">
            {runtimePolicy && (
              <div>
                <p className="text-[10px] uppercase tracking-wide text-tx-4">Runtime policy</p>
                <p className="text-[11px] text-tx-2 leading-relaxed mt-1 whitespace-pre-wrap">{runtimePolicy}</p>
              </div>
            )}
            {researchSummary && (
              <div>
                <p className="text-[10px] uppercase tracking-wide text-tx-4">Research summary</p>
                <p className="text-[11px] text-tx-2 leading-relaxed mt-1 whitespace-pre-wrap">{researchSummary}</p>
              </div>
            )}
          </div>
        )}

        <button
          onClick={() => setOpen(o => !o)}
          className="flex items-center gap-1.5 text-xs text-info/80 hover:text-info transition-colors mb-2"
        >
          <ChevronDown size={12} className={clsx('transition-transform', !open && '-rotate-90')} />
          {open ? 'Hide steps' : 'Show steps'}
        </button>

        <AnimatePresence>
          {open && steps.length > 0 && (
            <motion.div
              className="space-y-2"
              initial="hidden"
              animate="visible"
              exit="hidden"
              variants={{ visible: { transition: { staggerChildren: 0.05 } }, hidden: {} }}
            >
              {steps.map((s, i) => (
                <motion.div
                  key={i}
                  className="rounded-lg border border-border/60 bg-bg px-3 py-2.5"
                  variants={{
                    hidden: { opacity: 0, y: 8 },
                    visible: { opacity: 1, y: 0 },
                  }}
                  transition={{ duration: 0.15 }}
                >
                  <div className="flex items-start gap-2.5">
                    <span className="font-mono text-xs text-accent/70 shrink-0 w-5 text-right mt-0.5">
                      {s.step_index ?? s.index ?? i}
                    </span>
                    <div className="flex-1 min-w-0">
                      <p className="text-xs text-tx-1 leading-relaxed">{s.description}</p>
                      {s.tool && (
                        <span className="inline-block mt-1 font-mono text-[10px] text-tx-4 bg-bg-active rounded px-1.5 py-0.5">
                          {s.tool}
                        </span>
                      )}
                      {(s.llm_role || s.execution_intent || s.budget_tier || s.llm_generation) && (
                        <div className="mt-1 flex flex-wrap gap-1">
                          {(s.llm_role || s.llm_generation?.role) && <span className="badge bg-vio-soft text-vio border border-vio/20">{formatLabel(s.llm_role || s.llm_generation?.role)}</span>}
                          {(s.execution_intent || s.llm_generation?.execution_intent) && <span className="badge bg-info-soft text-info border border-info/20">{formatLabel(s.execution_intent || s.llm_generation?.execution_intent)}</span>}
                          {(s.budget_tier || s.llm_generation?.budget_tier) && <span className="badge bg-accent-soft text-accent border border-accent/20">{formatLabel(s.budget_tier || s.llm_generation?.budget_tier)}</span>}
                        </div>
                      )}
                    </div>
                    {s.completed && <CheckCircle2 size={12} className="text-ok shrink-0 mt-0.5" />}
                  </div>
                  {s.success_criteria && (
                    <p className="text-[11px] text-tx-3 mt-1.5 pl-7">
                      <span className="text-tx-4">Done when:</span> {s.success_criteria}
                    </p>
                  )}
                </motion.div>
              ))}
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </motion.div>
  );
}
