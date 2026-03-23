import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { TrendingUp, Clock, DollarSign, Zap, ChevronDown, ChevronUp } from 'lucide-react';
import { savings as savingsApi } from '../../api';

// ── Number formatters ──────────────────────────────────────────────────────
function formatHours(h) {
  if (h < 1)   return `${Math.round(h * 60)} min`;
  if (h < 100) return `${h.toFixed(1)} hrs`;
  return `${Math.round(h)} hrs`;
}

function formatMoney(usd) {
  if (usd >= 1_000_000) return `$${(usd / 1_000_000).toFixed(1)}M`;
  if (usd >= 1_000)     return `$${(usd / 1_000).toFixed(1)}k`;
  return `$${usd.toFixed(0)}`;
}

function formatRoi(x) {
  if (x <= 0)    return '—';
  if (x >= 1000) return `${Math.round(x / 100) * 100}×`;
  if (x >= 100)  return `${Math.round(x / 10) * 10}×`;
  return `${Math.round(x)}×`;
}

// ── Stat tile ──────────────────────────────────────────────────────────────
function Stat({ icon: Icon, label, value, sub, color = 'text-tx-1', delay = 0 }) {
  return (
    <motion.div
      className="flex items-start gap-3"
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay, duration: 0.2 }}
    >
      <div className="size-9 rounded-xl bg-bg flex items-center justify-center shrink-0 border border-border">
        <Icon size={16} className={color} />
      </div>
      <div>
        <p className={`text-xl font-bold tracking-tight ${color}`}>{value}</p>
        <p className="text-[11px] text-tx-4 leading-tight">{label}</p>
        {sub && <p className="text-[10px] text-tx-5 mt-0.5">{sub}</p>}
      </div>
    </motion.div>
  );
}

// ── Role breakdown row ─────────────────────────────────────────────────────
function RoleRow({ role, maxHours }) {
  const pct = maxHours > 0 ? (role.human_hours_saved / maxHours) * 100 : 0;
  return (
    <div className="flex items-center gap-3 py-1.5">
      <p className="text-[12px] text-tx-2 truncate w-40 shrink-0">{role.role_name}</p>
      <div className="flex-1 h-1.5 rounded-full bg-bg-active overflow-hidden">
        <motion.div
          className="h-full rounded-full bg-accent/60"
          initial={{ width: 0 }}
          animate={{ width: `${pct}%` }}
          transition={{ duration: 0.4, ease: 'easeOut' }}
        />
      </div>
      <p className="text-[11px] text-tx-3 w-16 text-right shrink-0">
        {formatHours(role.human_hours_saved)}
      </p>
      <p className="text-[11px] text-tx-4 w-16 text-right shrink-0">
        {formatMoney(role.human_cost_saved_usd)} saved
      </p>
      <p className="text-[10px] text-tx-5 w-10 text-right shrink-0">
        {role.runs} run{role.runs !== 1 ? 's' : ''}
      </p>
    </div>
  );
}

// ── Main component ─────────────────────────────────────────────────────────
export default function SavingsCard({ className = '' }) {
  const [data,     setData]     = useState(null);
  const [loading,  setLoading]  = useState(true);
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    savingsApi.getSummary()
      .then(d => { if (!cancelled) { setData(d); setLoading(false); } })
      .catch(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, []);

  // Don't render if no data or nothing saved yet
  if (loading || !data || data.total_runs === 0) return null;

  const maxHours = Math.max(...(data.by_role?.map(r => r.human_hours_saved) ?? [1]), 1);

  return (
    <motion.div
      className={`rounded-2xl border border-border bg-bg-card overflow-hidden ${className}`}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25 }}
    >
      {/* Header bar */}
      <div className="px-5 py-4 border-b border-border flex items-center gap-2">
        <div className="size-6 rounded-lg bg-ok-soft border border-ok/20 flex items-center justify-center">
          <TrendingUp size={12} className="text-ok" />
        </div>
        <p className="text-[13px] font-semibold text-tx-1">Work saved by Narayan</p>
        <p className="ml-auto text-[11px] text-tx-4">{data.total_runs} completed run{data.total_runs !== 1 ? 's' : ''}</p>
      </div>

      {/* Main stats */}
      <div className="px-5 py-4 grid grid-cols-2 gap-x-6 gap-y-4 sm:grid-cols-4">
        <Stat
          icon={Clock}
          label="Human hours saved"
          value={formatHours(data.total_human_hours)}
          sub={`≈ ${(data.total_human_hours / 8).toFixed(1)} work days`}
          color="text-accent"
          delay={0}
        />
        <Stat
          icon={DollarSign}
          label="Equivalent staff cost"
          value={formatMoney(data.total_human_cost_usd)}
          sub="at market rates"
          color="text-ok"
          delay={0.05}
        />
        <Stat
          icon={Zap}
          label="ROI multiple"
          value={formatRoi(data.roi_multiple)}
          sub={`$${data.total_ai_cost_usd.toFixed(2)} AI cost`}
          color="text-warn"
          delay={0.1}
        />
        <Stat
          icon={TrendingUp}
          label="Avg per run"
          value={formatHours(data.total_human_hours / data.total_runs)}
          sub={`${formatMoney(data.total_human_cost_usd / data.total_runs)} saved / run`}
          delay={0.15}
        />
      </div>

      {/* ROI bar */}
      <div className="px-5 pb-3">
        <div className="flex items-center gap-2 text-[11px] text-tx-4 mb-1.5">
          <span>AI cost</span>
          <div className="h-px flex-1 bg-border" />
          <span>Human equivalent</span>
        </div>
        <div className="flex h-2 rounded-full overflow-hidden gap-px">
          {/* AI cost portion — tiny sliver */}
          <div
            className="bg-tx-4/40 shrink-0"
            style={{
              width: data.roi_multiple > 0
                ? `${Math.max(2, 100 / (data.roi_multiple + 1))}%`
                : '50%'
            }}
          />
          <div className="flex-1 bg-ok/50 rounded-r-full" />
        </div>
        <p className="text-[10px] text-tx-5 mt-1">
          Every $1 spent on AI replaced {formatRoi(data.roi_multiple)} in staff time
        </p>
      </div>

      {/* Role breakdown toggle */}
      {data.by_role?.length > 0 && (
        <>
          <button
            onClick={() => setExpanded(e => !e)}
            className="w-full flex items-center gap-2 px-5 py-2.5 border-t border-border
                       text-[11px] text-tx-4 hover:text-tx-2 hover:bg-bg-hover transition-colors"
          >
            {expanded ? <ChevronUp size={11} /> : <ChevronDown size={11} />}
            {expanded ? 'Hide' : 'Show'} breakdown by role
          </button>

          <AnimatePresence>
            {expanded && (
              <motion.div
                initial={{ height: 0, opacity: 0 }}
                animate={{ height: 'auto', opacity: 1 }}
                exit={{ height: 0, opacity: 0 }}
                transition={{ duration: 0.2 }}
                className="overflow-hidden"
              >
                <div className="px-5 py-3 border-t border-border space-y-0.5">
                  <div className="flex items-center gap-3 pb-1 text-[10px] text-tx-5 uppercase tracking-wider">
                    <span className="w-40 shrink-0">Role</span>
                    <span className="flex-1" />
                    <span className="w-16 text-right shrink-0">Hours</span>
                    <span className="w-16 text-right shrink-0">Saved</span>
                    <span className="w-10 text-right shrink-0">Runs</span>
                  </div>
                  {data.by_role.map(role => (
                    <RoleRow key={role.role_id} role={role} maxHours={maxHours} />
                  ))}
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </>
      )}
    </motion.div>
  );
}
