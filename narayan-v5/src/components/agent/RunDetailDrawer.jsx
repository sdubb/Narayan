import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  X, CheckCircle2, XCircle, AlertCircle, Clock,
  DollarSign, Zap, ChevronRight, Loader2, Info,
} from 'lucide-react';
import { goalInstances as goalInstancesApi } from '../../api';

// ── Helpers ────────────────────────────────────────────────────────────────
function fmt(usd)  { return usd >= 0.001 ? `$${usd.toFixed(4)}` : '<$0.001'; }
function fmth(hrs) {
  if (!hrs) return null;
  return hrs < 1 ? `${Math.round(hrs * 60)} min` : `${hrs.toFixed(1)} hrs`;
}
function timeAgo(iso) {
  if (!iso) return '';
  const diff = (Date.now() - new Date(iso)) / 1000;
  if (diff < 60)   return `${Math.round(diff)}s ago`;
  if (diff < 3600) return `${Math.round(diff / 60)}m ago`;
  if (diff < 86400)return `${Math.round(diff / 3600)}h ago`;
  return new Date(iso).toLocaleDateString();
}

const STATUS_STYLES = {
  completed:          { bg: 'bg-ok-soft',   border: 'border-ok/20',   text: 'text-ok',   icon: CheckCircle2 },
  partially_complete: { bg: 'bg-warn-soft', border: 'border-warn/20', text: 'text-warn', icon: AlertCircle  },
  failed:             { bg: 'bg-err-soft',  border: 'border-err/20',  text: 'text-err',  icon: XCircle      },
  running:            { bg: 'bg-info-soft', border: 'border-info/20', text: 'text-info', icon: Loader2      },
};

// ── Criterion row ──────────────────────────────────────────────────────────
function CriterionRow({ criterion, index }) {
  const [open, setOpen] = useState(false);
  const Icon = criterion.satisfied ? CheckCircle2 : XCircle;
  const color = criterion.satisfied ? 'text-ok' : 'text-err';
  const bg    = criterion.satisfied ? 'bg-ok-soft' : 'bg-err-soft';
  const border= criterion.satisfied ? 'border-ok/20' : 'border-err/20';

  return (
    <motion.div
      className={`rounded-xl border ${border} ${bg} overflow-hidden`}
      initial={{ opacity: 0, y: 4 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: index * 0.06 }}
    >
      <button
        onClick={() => setOpen(o => !o)}
        className="w-full flex items-center gap-3 px-4 py-3 text-left"
      >
        <Icon size={15} className={`shrink-0 ${color}`} />
        <div className="flex-1 min-w-0">
          <p className="text-[13px] font-medium text-tx-1 truncate">{criterion.description}</p>
          <p className="text-[11px] text-tx-4 mt-0.5">{criterion.check_type?.replace(/_/g, ' ')}</p>
        </div>
        <span className={`text-[11px] font-semibold px-2 py-0.5 rounded-full ${bg} ${color} border ${border}`}>
          {criterion.satisfied ? 'PASS' : 'FAIL'}
        </span>
        <ChevronRight
          size={12}
          className={`text-tx-4 shrink-0 transition-transform ${open ? 'rotate-90' : ''}`}
        />
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ height: 0 }} animate={{ height: 'auto' }} exit={{ height: 0 }}
            className="overflow-hidden border-t border-inherit"
          >
            <p className="px-4 py-3 text-[12px] text-tx-2 leading-relaxed font-mono whitespace-pre-wrap">
              {criterion.detail}
            </p>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}

// ── Step output row ────────────────────────────────────────────────────────
function StepOutputRow({ output, index }) {
  return (
    <motion.div
      className="flex items-center gap-3 px-3 py-2.5 rounded-lg border border-border bg-bg"
      initial={{ opacity: 0 }} animate={{ opacity: 1 }}
      transition={{ delay: 0.1 + index * 0.04 }}
    >
      <span className="size-5 rounded-full bg-bg-card border border-border flex items-center justify-center text-[10px] text-tx-4 font-mono shrink-0">
        {output.step ?? index + 1}
      </span>
      <div className="flex-1 min-w-0">
        {output.processed > 0 && (
          <span className="text-[12px] text-tx-2">
            {output.processed} item{output.processed !== 1 ? 's' : ''} processed
          </span>
        )}
        {output.connectors?.length > 0 && (
          <span className="text-[11px] text-tx-4 ml-2">
            via {output.connectors.join(', ')}
          </span>
        )}
      </div>
      <span className={`text-[10px] font-semibold ${output.success ? 'text-ok' : 'text-err'}`}>
        {output.success ? '✓' : '✗'}
      </span>
    </motion.div>
  );
}

// ── Main component ─────────────────────────────────────────────────────────
export default function RunDetailDrawer({ instanceId, onClose }) {
  const [data,    setData]    = useState(null);
  const [loading, setLoading] = useState(true);
  const [error,   setError]   = useState('');

  useEffect(() => {
    let cancelled = false;
    goalInstancesApi.getDetail(instanceId)
      .then(d  => { if (!cancelled) { setData(d); setLoading(false); } })
      .catch(e => { if (!cancelled) { setError(e.message); setLoading(false); } });
    return () => { cancelled = true; };
  }, [instanceId]);

  const status  = data?.status ?? 'running';
  const sStyle  = STATUS_STYLES[status] ?? STATUS_STYLES.running;
  const SIcon   = sStyle.icon;

  const criteriaChecks = data?.result?.criteria_checks ?? [];
  const stepOutputs = Array.isArray(data?.result?.step_outputs)
    ? data.result.step_outputs
    : (data?.result?.step_outputs ? [data.result.step_outputs] : []);

  const passCount = criteriaChecks.filter(c => c.satisfied).length;
  const totalCriteria = criteriaChecks.length;

  return (
    <motion.div
      className="fixed inset-0 z-50 flex justify-end"
      initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
    >
      <div className="absolute inset-0 bg-tx-1/20 backdrop-blur-[2px]" onClick={onClose} />

      <motion.div
        className="relative w-full max-w-lg h-full flex flex-col bg-bg border-l border-border shadow-xl overflow-y-auto"
        initial={{ x: '100%' }} animate={{ x: 0 }} exit={{ x: '100%' }}
        transition={{ type: 'spring', damping: 30, stiffness: 300 }}
      >
        {/* Header */}
        <div className="sticky top-0 z-10 flex items-center gap-3 px-5 py-4 border-b border-border bg-bg-card shrink-0">
          <div className={`size-8 rounded-lg flex items-center justify-center ${sStyle.bg} border ${sStyle.border}`}>
            <SIcon size={14} className={sStyle.text} />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-[13px] font-semibold text-tx-1 capitalize">{status.replace(/_/g, ' ')}</p>
            <p className="text-[11px] text-tx-4">{timeAgo(data?.created_at)}</p>
          </div>
          <button onClick={onClose} className="p-1.5 rounded-lg text-tx-4 hover:text-tx-1 hover:bg-bg-hover transition-all">
            <X size={15} />
          </button>
        </div>

        {loading ? (
          <div className="flex-1 flex items-center justify-center">
            <Loader2 size={20} className="text-tx-4 animate-spin" />
          </div>
        ) : error ? (
          <div className="p-6 text-sm text-err">{error}</div>
        ) : (
          <div className="flex-1 p-5 space-y-6">

            {/* Stats row */}
            <div className="grid grid-cols-3 gap-3">
              {[
                { icon: DollarSign, label: 'AI cost',    value: data?.cost_usd ? fmt(data.cost_usd) : '—' },
                { icon: Clock,      label: 'Human hours saved', value: fmth(data?.human_hours_saved) ?? '—' },
                { icon: Zap,        label: 'ROI',
                  value: data?.human_cost_saved_usd && data?.cost_usd && data.cost_usd > 0
                    ? `${Math.round(data.human_cost_saved_usd / data.cost_usd)}×`
                    : '—' },
              ].map(({ icon: Icon, label, value }) => (
                <div key={label} className="bg-bg-card rounded-xl border border-border px-3 py-3 text-center">
                  <Icon size={13} className="text-tx-4 mx-auto mb-1" />
                  <p className="text-[15px] font-bold text-tx-1">{value}</p>
                  <p className="text-[10px] text-tx-5 mt-0.5">{label}</p>
                </div>
              ))}
            </div>

            {/* Failure note */}
            {data?.failure_reason && (
              <div className="rounded-xl border border-warn/20 bg-warn-soft px-4 py-3 flex gap-2.5">
                <AlertCircle size={14} className="text-warn shrink-0 mt-0.5" />
                <p className="text-[12px] text-tx-2 leading-relaxed">{data.failure_reason}</p>
              </div>
            )}

            {/* Completion criteria */}
            {criteriaChecks.length > 0 && (
              <section>
                <div className="flex items-center gap-2 mb-3">
                  <p className="text-[12px] font-semibold text-tx-3 uppercase tracking-wider">
                    Completion criteria
                  </p>
                  <span className={`text-[11px] font-bold px-2 py-0.5 rounded-full ${
                    passCount === totalCriteria ? 'bg-ok-soft text-ok' : 'bg-warn-soft text-warn'
                  }`}>
                    {passCount}/{totalCriteria} passed
                  </span>
                </div>
                <div className="space-y-2">
                  {criteriaChecks.map((c, i) => (
                    <CriterionRow key={i} criterion={c} index={i} />
                  ))}
                </div>
              </section>
            )}

            {/* Step outputs */}
            {stepOutputs.length > 0 && (
              <section>
                <p className="text-[12px] font-semibold text-tx-3 uppercase tracking-wider mb-3">
                  Step outputs
                </p>
                <div className="space-y-1.5">
                  {stepOutputs.map((o, i) => (
                    <StepOutputRow key={i} output={o} index={i} />
                  ))}
                </div>
              </section>
            )}

            {/* No criteria yet */}
            {criteriaChecks.length === 0 && stepOutputs.length === 0 && (
              <div className="flex flex-col items-center gap-2 py-8 text-center">
                <Info size={18} className="text-tx-4" />
                <p className="text-[13px] text-tx-3">No criteria data for this run</p>
                <p className="text-[11px] text-tx-4 max-w-xs">
                  Completion criteria are checked when a run finishes. Configure them via plan mode.
                </p>
              </div>
            )}
          </div>
        )}
      </motion.div>
    </motion.div>
  );
}
