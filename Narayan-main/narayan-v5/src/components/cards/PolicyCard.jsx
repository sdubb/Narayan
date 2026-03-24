import { motion } from 'framer-motion';
import clsx from 'clsx';
import { Shield, Lock, CheckCircle2, Eye, Clock, AlertTriangle, Bell } from 'lucide-react';

const TYPE_CONFIG = {
  policy_decision: (ev) => {
    const isBlock = ev.decision === 'block';
    const isApproval = ev.decision === 'require_approval';
    return {
      color: isBlock ? 'err' : isApproval ? 'warn' : 'ok',
      icon: isBlock ? Lock : isApproval ? Shield : CheckCircle2,
      label: isBlock ? 'Policy: blocked' : isApproval ? 'Awaiting approval' : 'Policy: allow',
    };
  },
  pii_redacted: (ev) => ({
    color: ev.fields_redacted?.length > 0 ? 'warn' : 'ok',
    icon: Eye,
    label: ev.fields_redacted?.length > 0 ? 'PII redacted' : 'PII scan — clean',
  }),
  sla_check: (ev) => {
    const pct = ev.pct_elapsed || 0;
    return {
      color: pct >= 100 ? 'err' : pct >= 80 ? 'warn' : 'ok',
      icon: Clock,
      label: pct >= 100 ? 'SLA breached' : pct >= 80 ? 'SLA warning' : 'SLA on track',
    };
  },
};

const COLOR_STYLES = {
  ok: 'border-ok/25 bg-ok-soft/40',
  err: 'border-err/25 bg-err-soft/40',
  warn: 'border-warn/25 bg-warn-soft/40',
  vio: 'border-vio/25 bg-vio-soft/40',
  info: 'border-info/25 bg-info-soft/40',
};

const TEXT_COLORS = { ok: 'text-ok', err: 'text-err', warn: 'text-warn', vio: 'text-vio', info: 'text-info' };

export default function PolicyCard({ event, compact = false }) {
  const type = event.type || 'policy_decision';
  const configFn = TYPE_CONFIG[type] || TYPE_CONFIG.policy_decision;
  const config = configFn(event);
  const Icon = config.icon;
  const colorStyle = COLOR_STYLES[config.color] || COLOR_STYLES.ok;
  const textColor = TEXT_COLORS[config.color] || TEXT_COLORS.ok;

  if (compact) {
    return (
      <div className={clsx('flex items-center gap-2 rounded-lg border px-3 py-2', colorStyle)}>
        <Icon size={12} className={clsx(textColor, 'shrink-0')} />
        <span className={clsx('text-xs font-medium', textColor)}>{config.label}</span>
        {event.tool && <span className="text-[10px] font-mono text-tx-4 ml-auto">{event.tool}</span>}
        {event.risk_level && <span className="text-[10px] text-tx-4">risk: {event.risk_level}</span>}
      </div>
    );
  }

  return (
    <motion.div
      className={clsx('rounded-xl border overflow-hidden shadow-sm', colorStyle)}
      initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.15 }}
    >
      <div className={clsx('flex items-center gap-2 px-3.5 py-2.5 border-b', colorStyle.split(' ')[0])}>
        <Icon size={12} className={clsx(textColor, 'shrink-0')} />
        <span className={clsx('text-xs font-bold tracking-wider uppercase', textColor)}>{config.label}</span>
      </div>
      <div className="px-3.5 py-2.5 space-y-1.5">
        {event.rule_id && <p className="text-xs text-tx-2">Rule: <span className="font-mono">{event.rule_id}</span></p>}
        {event.reason && <p className="text-xs text-tx-3">{event.reason}</p>}
        {event.message && <p className="text-xs text-tx-3">{event.message}</p>}
        {event.tool && <p className="text-xs text-tx-4">Tool: <span className="font-mono">{event.tool}</span> · Risk: {event.risk_level || 'medium'}</p>}
        {type === 'pii_redacted' && event.fields_redacted?.length > 0 && (
          <p className="text-xs text-tx-3">Fields: {event.fields_redacted.join(', ')}</p>
        )}
        {type === 'sla_check' && (
          <div className="flex items-center gap-2">
            <div className="flex-1 h-1.5 rounded-full bg-bg-active overflow-hidden">
              <div className={clsx('h-full rounded-full', textColor.replace('text-', 'bg-'))}
                style={{ width: `${Math.min(event.pct_elapsed || 0, 100)}%` }} />
            </div>
            <span className="text-[10px] font-mono text-tx-4">{(event.pct_elapsed || 0).toFixed(0)}%</span>
          </div>
        )}
        {event.action === 'escalate' && (
          <div className="flex items-center gap-1.5 text-xs text-err">
            <AlertTriangle size={11} /> Escalated to review
          </div>
        )}
        {event.decision === 'require_approval' && (
          <div className="flex items-center gap-1.5 text-xs text-warn">
            <Bell size={11} /> Submitted to review queue
          </div>
        )}
      </div>
    </motion.div>
  );
}
