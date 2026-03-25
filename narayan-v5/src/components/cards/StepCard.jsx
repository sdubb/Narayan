import { useState } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, AlertCircle, RotateCcw, ChevronDown, Zap } from 'lucide-react';
import ToolPill from './ToolPill';
import PolicyCard from './PolicyCard';
import CitationCard from './CitationCard';
import JudgementCard from './JudgementCard';

export default function StepCard({ step }) {
  const [collapsed, setCollapsed] = useState(false);
  const {
    index, description, tools = [], policy = [], citations = [],
    piiEvents = [], slaEvents = [], reviews = [], judgements = [],
    completed, summary, retrying, retryDelay, retryReason,
  } = step;

  const isActive = !completed && !retrying;

  return (
    <motion.div
      className={clsx(
        'rounded-xl border bg-bg-card shadow-sm overflow-hidden',
        'border-l-4',
        completed ? 'border-l-ok' : retrying ? 'border-l-warn' : 'border-l-accent',
      )}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.25, 0.1, 0.25, 1] }}
    >
      {/* Header */}
      <button
        onClick={() => setCollapsed(c => !c)}
        className="w-full flex items-center gap-3 px-4 py-3 hover:bg-bg-hover/50 transition-colors"
      >
        <span className="flex items-center justify-center size-6 rounded-lg bg-bg-active text-xs font-mono font-semibold text-accent shrink-0">
          {index}
        </span>
        <span className="text-sm font-medium text-tx-1 flex-1 text-left truncate">{description}</span>
        <div className="flex items-center gap-2 shrink-0">
          {isActive && (
            <motion.span
              className="size-2 rounded-full bg-accent"
              animate={{ opacity: [0.4, 1, 0.4], scale: [0.95, 1.05, 0.95] }}
              transition={{ duration: 1.4, repeat: Infinity, ease: 'easeInOut' }}
            />
          )}
          {completed && <CheckCircle2 size={14} className="text-ok" />}
          {retrying && <RotateCcw size={14} className="text-warn" />}
          <ChevronDown size={14} className={clsx('text-tx-4 transition-transform', collapsed && '-rotate-90')} />
        </div>
      </button>

      {/* Body */}
      {!collapsed && (
        <div className="px-4 pb-3 space-y-2">
          {/* Tool pills */}
          {tools.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {tools.map((t, i) => (
                <ToolPill key={`${t.name}-${i}`} tool={t} />
              ))}
            </div>
          )}

          {/* Policy decisions */}
          {policy.map((p, i) => (
            <PolicyCard key={`policy-${i}`} event={{ ...p, type: 'policy_decision' }} compact />
          ))}

          {/* PII events */}
          {piiEvents.map((p, i) => (
            <PolicyCard key={`pii-${i}`} event={{ ...p, type: 'pii_redacted' }} compact />
          ))}

          {/* SLA events */}
          {slaEvents.map((s, i) => (
            <PolicyCard key={`sla-${i}`} event={{ ...s, type: 'sla_check' }} compact />
          ))}

          {/* Citations */}
          {citations.map((c, i) => (
            <CitationCard key={`cite-${i}`} event={{ ...c, type: 'citation_recorded' }} compact />
          ))}

          {/* Judgements */}
          {judgements.map((j, i) => (
            <JudgementCard key={`judgement-${i}`} event={{ ...j, type: 'judgement_signal' }} compact />
          ))}

          {/* Retry info */}
          {retrying && (
            <div className="flex items-center gap-2 rounded-lg bg-warn-soft border border-warn/20 px-3 py-2">
              <RotateCcw size={12} className="text-warn shrink-0" />
              <span className="text-xs text-warn">
                Retrying in {retryDelay || 10}s — {retryReason || 'Transient failure'}
              </span>
            </div>
          )}

          {/* Completion summary */}
          {completed && summary && (
            <div className="flex items-start gap-2 rounded-lg bg-ok-soft/50 border border-ok/15 px-3 py-2">
              <CheckCircle2 size={12} className="text-ok shrink-0 mt-0.5" />
              <p className="text-xs text-tx-2 leading-relaxed">{summary}</p>
            </div>
          )}
        </div>
      )}
    </motion.div>
  );
}
