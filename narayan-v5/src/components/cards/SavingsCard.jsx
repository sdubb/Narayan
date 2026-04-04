import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { TrendingUp, Clock, DollarSign, Zap, ChevronDown, ChevronUp } from 'lucide-react';
import { savings as savingsApi } from '../../api';

function safeNumber(value, fallback = 0) {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

function formatHours(value) {
  const hours = safeNumber(value);
  if (hours < 1) return `${Math.round(hours * 60)} min`;
  if (hours < 100) return `${hours.toFixed(1)} hrs`;
  return `${Math.round(hours)} hrs`;
}

function formatMoney(value) {
  const usd = safeNumber(value);
  if (usd >= 1_000_000) return `$${(usd / 1_000_000).toFixed(1)}M`;
  if (usd >= 1_000) return `$${(usd / 1_000).toFixed(1)}k`;
  return `$${usd.toFixed(0)}`;
}

function formatRoi(value) {
  const roi = safeNumber(value);
  if (roi <= 0) return '—';
  if (roi >= 1000) return `${Math.round(roi / 100) * 100}x`;
  if (roi >= 100) return `${Math.round(roi / 10) * 10}x`;
  return `${Math.round(roi)}x`;
}

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

function RoleRow({ role, maxHours }) {
  const hoursSaved = safeNumber(role?.human_hours_saved);
  const costSaved = safeNumber(role?.human_cost_saved_usd);
  const runs = safeNumber(role?.runs);
  const pct = maxHours > 0 ? (hoursSaved / maxHours) * 100 : 0;

  return (
    <div className="flex items-center gap-3 py-1.5">
      <p className="text-[12px] text-tx-2 truncate w-40 shrink-0">{role?.role_name || 'Role'}</p>
      <div className="flex-1 h-1.5 rounded-full bg-bg-active overflow-hidden">
        <motion.div
          className="h-full rounded-full bg-accent/60"
          initial={{ width: 0 }}
          animate={{ width: `${pct}%` }}
          transition={{ duration: 0.4, ease: 'easeOut' }}
        />
      </div>
      <p className="text-[11px] text-tx-3 w-16 text-right shrink-0">{formatHours(hoursSaved)}</p>
      <p className="text-[11px] text-tx-4 w-16 text-right shrink-0">{formatMoney(costSaved)} saved</p>
      <p className="text-[10px] text-tx-5 w-10 text-right shrink-0">
        {runs} run{runs !== 1 ? 's' : ''}
      </p>
    </div>
  );
}

export default function SavingsCard({ className = '' }) {
  const [data, setData] = useState(null);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    savingsApi.getSummary()
      .then(d => {
        if (!cancelled) {
          setData(d || null);
          setLoading(false);
        }
      })
      .catch(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, []);

  const totalRuns = safeNumber(data?.total_runs);
  const totalHumanHours = safeNumber(data?.total_human_hours);
  const totalHumanCostUsd = safeNumber(data?.total_human_cost_usd);
  const totalAiCostUsd = safeNumber(data?.total_ai_cost_usd);
  const roiMultiple = safeNumber(data?.roi_multiple);
  const byRole = Array.isArray(data?.by_role) ? data.by_role : [];

  if (loading || !data || totalRuns === 0) return null;

  const maxHours = Math.max(...byRole.map(role => safeNumber(role?.human_hours_saved)), 1);

  return (
    <motion.div
      className={`rounded-2xl border border-border bg-bg-card overflow-hidden ${className}`}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25 }}
    >
      <div className="px-5 py-4 border-b border-border flex items-center gap-2">
        <div className="size-6 rounded-lg bg-ok-soft border border-ok/20 flex items-center justify-center">
          <TrendingUp size={12} className="text-ok" />
        </div>
        <p className="text-[13px] font-semibold text-tx-1">Work saved by Narayan</p>
        <p className="ml-auto text-[11px] text-tx-4">
          {totalRuns} completed run{totalRuns !== 1 ? 's' : ''}
        </p>
      </div>

      <div className="px-5 py-4 grid grid-cols-2 gap-x-6 gap-y-4 sm:grid-cols-4">
        <Stat
          icon={Clock}
          label="Human hours saved"
          value={formatHours(totalHumanHours)}
          sub={`~ ${(totalHumanHours / 8).toFixed(1)} work days`}
          color="text-accent"
        />
        <Stat
          icon={DollarSign}
          label="Equivalent staff cost"
          value={formatMoney(totalHumanCostUsd)}
          sub="at market rates"
          color="text-ok"
          delay={0.05}
        />
        <Stat
          icon={Zap}
          label="ROI multiple"
          value={formatRoi(roiMultiple)}
          sub={`$${totalAiCostUsd.toFixed(2)} AI cost`}
          color="text-warn"
          delay={0.1}
        />
        <Stat
          icon={TrendingUp}
          label="Avg per run"
          value={formatHours(totalHumanHours / Math.max(totalRuns, 1))}
          sub={`${formatMoney(totalHumanCostUsd / Math.max(totalRuns, 1))} saved / run`}
          delay={0.15}
        />
      </div>

      <div className="px-5 pb-3">
        <div className="flex items-center gap-2 text-[11px] text-tx-4 mb-1.5">
          <span>AI cost</span>
          <div className="h-px flex-1 bg-border" />
          <span>Human equivalent</span>
        </div>
        <div className="flex h-2 rounded-full overflow-hidden gap-px">
          <div
            className="bg-tx-4/40 shrink-0"
            style={{
              width: roiMultiple > 0 ? `${Math.max(2, 100 / (roiMultiple + 1))}%` : '50%',
            }}
          />
          <div className="flex-1 bg-ok/50 rounded-r-full" />
        </div>
        <p className="text-[10px] text-tx-5 mt-1">
          Every $1 spent on AI replaced {formatRoi(roiMultiple)} in staff time
        </p>
      </div>

      {byRole.length > 0 && (
        <>
          <button
            type="button"
            onClick={() => setExpanded(e => !e)}
            className="w-full flex items-center gap-2 px-5 py-2.5 border-t border-border text-[11px] text-tx-4 hover:text-tx-2 hover:bg-bg-hover transition-colors"
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
                  {byRole.map(role => (
                    <RoleRow key={role.role_id || role.role_name} role={role} maxHours={maxHours} />
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
