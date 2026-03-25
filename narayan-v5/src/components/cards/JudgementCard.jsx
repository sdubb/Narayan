import { motion } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, Eye, RotateCcw, ShieldAlert } from 'lucide-react';

const TYPE_CONFIG = {
  continue: { color: 'ok', icon: CheckCircle2, label: 'Judgement: continue' },
  watch: { color: 'warn', icon: Eye, label: 'Judgement: watch' },
  revise: { color: 'vio', icon: RotateCcw, label: 'Judgement: revise' },
  escalate: { color: 'err', icon: ShieldAlert, label: 'Judgement: escalate' },
};

const COLOR_STYLES = {
  ok: 'border-ok/25 bg-ok-soft/40',
  err: 'border-err/25 bg-err-soft/40',
  warn: 'border-warn/25 bg-warn-soft/40',
  vio: 'border-vio/25 bg-vio-soft/40',
};

const TEXT_COLORS = {
  ok: 'text-ok',
  err: 'text-err',
  warn: 'text-warn',
  vio: 'text-vio',
};

function recommendationKey(value) {
  return String(value || 'watch').toLowerCase();
}

function scoreLabel(value) {
  if (value == null || Number.isNaN(Number(value))) return 'n/a';
  return `${Math.round(Number(value) * 100)}%`;
}

export default function JudgementCard({ event, compact = false }) {
  const key = recommendationKey(event.recommendation);
  const config = TYPE_CONFIG[key] || TYPE_CONFIG.watch;
  const Icon = config.icon;
  const colorStyle = COLOR_STYLES[config.color] || COLOR_STYLES.warn;
  const textColor = TEXT_COLORS[config.color] || TEXT_COLORS.warn;
  const score = Math.max(0, Math.min(Number(event.score ?? 0), 1));
  const confidence = Math.max(0, Math.min(Number(event.confidence ?? 0), 1));

  if (compact) {
    return (
      <div className={clsx('flex items-center gap-2 rounded-lg border px-3 py-2', colorStyle)}>
        <Icon size={12} className={clsx(textColor, 'shrink-0')} />
        <span className={clsx('text-xs font-medium', textColor)}>{config.label}</span>
        <span className="ml-auto text-[10px] font-mono text-tx-4">
          {scoreLabel(score)} - {scoreLabel(confidence)}
        </span>
      </div>
    );
  }

  return (
    <motion.div
      className={clsx('rounded-xl border overflow-hidden shadow-sm', colorStyle)}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.15 }}
    >
      <div className={clsx('flex items-center gap-2 px-3.5 py-2.5 border-b', colorStyle.split(' ')[0])}>
        <Icon size={12} className={clsx(textColor, 'shrink-0')} />
        <span className={clsx('text-xs font-bold tracking-wider uppercase', textColor)}>{config.label}</span>
      </div>
      <div className="px-3.5 py-2.5 space-y-2">
        {event.step_description && (
          <p className="text-xs text-tx-2">
            Step: <span className="font-medium text-tx-1">{event.step_description}</span>
          </p>
        )}
        {(event.profile || event.job_type) && (
          <p className="text-[11px] text-tx-4">
            {event.profile && <span className="font-medium text-tx-3">{event.profile}</span>}
            {event.profile && event.job_type && <span className="mx-1">·</span>}
            {event.job_type && <span className="font-mono">{event.job_type}</span>}
          </p>
        )}
        {event.summary && <p className="text-xs text-tx-3 leading-relaxed">{event.summary}</p>}
        <div className="grid grid-cols-2 gap-2">
          <div className="rounded-lg border border-border bg-bg-card px-2.5 py-2">
            <p className="text-[10px] uppercase tracking-wider text-tx-4 mb-1">Score</p>
            <div className="h-1.5 rounded-full bg-bg-active overflow-hidden">
              <div
                className={clsx('h-full rounded-full', textColor.replace('text-', 'bg-'))}
                style={{ width: `${score * 100}%` }}
              />
            </div>
            <p className="text-[10px] font-mono text-tx-4 mt-1">{scoreLabel(score)}</p>
          </div>
          <div className="rounded-lg border border-border bg-bg-card px-2.5 py-2">
            <p className="text-[10px] uppercase tracking-wider text-tx-4 mb-1">Confidence</p>
            <div className="h-1.5 rounded-full bg-bg-active overflow-hidden">
              <div
                className={clsx('h-full rounded-full', textColor.replace('text-', 'bg-'))}
                style={{ width: `${confidence * 100}%` }}
              />
            </div>
            <p className="text-[10px] font-mono text-tx-4 mt-1">{scoreLabel(confidence)}</p>
          </div>
        </div>
        {Array.isArray(event.reasons) && event.reasons.length > 0 && (
          <div className="space-y-1">
            {event.reasons.map((reason, index) => (
              <p key={`${reason}-${index}`} className="text-xs text-tx-3 leading-relaxed">
                - {reason}
              </p>
            ))}
          </div>
        )}
        {event.timestamp && <p className="text-[10px] text-tx-4 font-mono">{event.timestamp}</p>}
      </div>
    </motion.div>
  );
}
