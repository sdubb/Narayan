import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { ArrowRight, Save, Loader2 } from 'lucide-react';
import { routing, credentials } from '../../api';

const TIERS = [
  { id: 'simple', label: 'Simple', desc: 'Evaluator, preflight, clarifier' },
  { id: 'medium', label: 'Medium', desc: 'Reflector calls' },
  { id: 'complex', label: 'Complex', desc: 'Planner calls' },
  { id: 'fallback', label: 'Fallback', desc: 'If preferred fails' },
];

export default function RoutingTab({ onFlash }) {
  const [config, setConfig] = useState({ simple: '', medium: '', complex: '', fallback: '' });
  const [providers, setProviders] = useState([]);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    credentials.list().then(d => setProviders((d.credentials || []).map(c => c.provider))).catch(() => {});
  }, []);

  function update(tier, value) {
    setConfig(c => ({ ...c, [tier]: value }));
    setDirty(true);
  }

  async function save() {
    setSaving(true);
    try {
      await routing.update(config);
      setDirty(false);
      onFlash?.('Routing saved');
    } catch (e) { onFlash?.(e.message); }
    finally { setSaving(false); }
  }

  return (
    <div className="space-y-6">
      <p className="text-sm text-tx-2">Route different complexity tiers to different LLM providers. The agent selects the tier based on task complexity.</p>

      <div className="flex items-center gap-3 overflow-x-auto pb-2">
        {TIERS.map((tier, i) => (
          <div key={tier.id} className="flex items-center gap-3 shrink-0">
            {i > 0 && <ArrowRight size={14} className="text-tx-4 shrink-0" />}
            <div className="card p-4 w-44">
              <p className="text-sm font-semibold text-tx-1 mb-0.5">{tier.label}</p>
              <p className="text-[11px] text-tx-3 mb-3">{tier.desc}</p>
              <select
                value={config[tier.id]}
                onChange={e => update(tier.id, e.target.value)}
                className="input-field text-xs"
              >
                <option value="">Auto</option>
                {providers.map(p => <option key={p} value={p}>{p}</option>)}
              </select>
            </div>
          </div>
        ))}
      </div>

      <button onClick={save} disabled={saving || !dirty}
        className={clsx('btn-primary flex items-center gap-2 transition-all', !dirty && 'opacity-50 cursor-not-allowed')}>
        {saving ? <Loader2 size={14} className="animate-spin" /> : <Save size={14} />}
        Save routing
      </button>
    </div>
  );
}
