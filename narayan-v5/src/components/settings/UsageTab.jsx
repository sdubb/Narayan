import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, Cell } from 'recharts';
import { Loader2, Activity, DollarSign, Zap, Clock } from 'lucide-react';
import { metrics as metricsApi } from '../../api';

const BAR_COLORS = ['#c96a2e', '#3b82f6', '#22c55e', '#8b5cf6', '#f59e0b', '#ef4444'];

function StatCard({ icon: Icon, label, value, sub }) {
  return (
    <div className="card p-4">
      <div className="flex items-center gap-2 mb-2">
        <Icon size={14} className="text-tx-3" />
        <span className="text-xs text-tx-3">{label}</span>
      </div>
      <p className="text-xl font-semibold text-tx-1 font-mono">{value}</p>
      {sub && <p className="text-[11px] text-tx-4 mt-0.5">{sub}</p>}
    </div>
  );
}

export default function UsageTab() {
  const [metricsData, setMetricsData] = useState(null);
  const [costsData, setCostsData] = useState(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([
      metricsApi.get().catch(() => null),
      metricsApi.costs().catch(() => null),
    ]).then(([m, c]) => { setMetricsData(m); setCostsData(c); }).finally(() => setLoading(false));
  }, []);

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  const pctUsed = costsData?.pct_used || 0;
  const barColor = pctUsed > 90 ? 'bg-err' : pctUsed > 70 ? 'bg-warn' : 'bg-ok';
  const usage = costsData?.usage || {};
  const providerData = Object.entries(usage).map(([name, data], i) => ({
    name, cost: data?.total_cost_usd || 0, color: BAR_COLORS[i % BAR_COLORS.length],
  })).filter(d => d.cost > 0);

  return (
    <div className="space-y-6">
      {/* Spend limit bar */}
      <div className="card p-5">
        <div className="flex items-center justify-between mb-2">
          <span className="text-sm font-medium text-tx-1">Spend this period</span>
          <span className="text-sm font-mono text-tx-2">
            ${(costsData?.total_cost_usd || 0).toFixed(2)} / ${(costsData?.limit_usd || 5).toFixed(0)}
          </span>
        </div>
        <div className="w-full h-3 rounded-full bg-bg-active overflow-hidden">
          <div className={clsx('h-full rounded-full transition-all', barColor)} style={{ width: `${Math.min(pctUsed, 100)}%` }} />
        </div>
        <p className="text-xs text-tx-4 mt-1">{pctUsed.toFixed(1)}% used</p>
      </div>

      {/* Cost by provider chart */}
      {providerData.length > 0 && (
        <div className="card p-5">
          <p className="text-sm font-medium text-tx-1 mb-4">Cost by provider</p>
          <ResponsiveContainer width="100%" height={200}>
            <BarChart data={providerData} layout="vertical">
              <XAxis type="number" tick={{ fontSize: 11, fill: '#8a8278' }} />
              <YAxis type="category" dataKey="name" tick={{ fontSize: 11, fill: '#4a4540' }} width={80} />
              <Tooltip formatter={(v) => `$${v.toFixed(3)}`} contentStyle={{ fontSize: 12, borderRadius: 8, border: '1px solid #e2dbd2' }} />
              <Bar dataKey="cost" radius={[0, 4, 4, 0]}>
                {providerData.map((d, i) => <Cell key={i} fill={d.color} />)}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </div>
      )}

      {/* Platform counters */}
      <div className="grid grid-cols-2 gap-3">
        <StatCard icon={Zap} label="Goals created" value={metricsData?.goals_created ?? 0} />
        <StatCard icon={Activity} label="Agents running" value={metricsData?.agents_running ?? 0} />
        <StatCard icon={DollarSign} label="Total cost" value={`$${(costsData?.total_cost_usd || 0).toFixed(2)}`} />
        <StatCard icon={Clock} label="Total requests" value={costsData?.total_requests ?? 0} />
      </div>

      {/* Token breakdown */}
      {costsData && (
        <div className="card p-5">
          <p className="text-sm font-medium text-tx-1 mb-3">Token usage</p>
          <div className="grid grid-cols-2 gap-4 text-xs">
            <div>
              <span className="text-tx-3">Input tokens</span>
              <p className="font-mono text-tx-1 text-lg">{(costsData.total_input_tokens || 0).toLocaleString()}</p>
            </div>
            <div>
              <span className="text-tx-3">Output tokens</span>
              <p className="font-mono text-tx-1 text-lg">{(costsData.total_output_tokens || 0).toLocaleString()}</p>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
