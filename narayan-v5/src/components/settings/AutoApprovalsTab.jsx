import { useState, useEffect } from 'react';
import { Shield, Trash2, Plus, Loader2 } from 'lucide-react';
import { autoApprovals } from '../../api';

export default function AutoApprovalsTab({ onFlash }) {
  const [rules, setRules] = useState([]);
  const [loading, setLoading] = useState(true);
  const [newRule, setNewRule] = useState('');
  const [notes, setNotes] = useState('');

  useEffect(() => {
    autoApprovals.list().then(d => setRules(d.rules || [])).catch(() => {}).finally(() => setLoading(false));
  }, []);

  async function add() {
    if (!newRule.trim()) return;
    try {
      await autoApprovals.create(newRule.trim(), notes.trim() || undefined);
      const r = await autoApprovals.list();
      setRules(r.rules || []);
      setNewRule('');
      setNotes('');
      onFlash?.('Auto-approval rule added');
    } catch (e) { onFlash?.(e.message); }
  }

  async function remove(ruleId) {
    try {
      await autoApprovals.delete(ruleId);
      setRules(r => r.filter(rule => rule.rule_id !== ruleId));
      onFlash?.('Rule removed');
    } catch (e) { onFlash?.(e.message); }
  }

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  return (
    <div className="space-y-6">
      <p className="text-sm text-tx-2">Auto-approval rules let agents skip human review for specific operations. When a policy triggers review for a matching rule, it will be automatically approved.</p>

      {rules.length > 0 ? (
        <div className="space-y-2">
          {rules.map(r => (
            <div key={r.rule_id} className="card p-4 flex items-center gap-3">
              <Shield size={14} className="text-ok shrink-0" />
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-tx-1 font-mono">{r.rule_id}</p>
                {r.notes && <p className="text-xs text-tx-3 mt-0.5">{r.notes}</p>}
              </div>
              <button onClick={() => remove(r.rule_id)} className="p-1.5 rounded-lg text-tx-4 hover:text-err hover:bg-err-soft transition-all">
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      ) : (
        <div className="text-center py-8">
          <Shield size={24} className="text-tx-4 mx-auto mb-2" />
          <p className="text-sm text-tx-3">No auto-approval rules configured</p>
        </div>
      )}

      <div className="card p-4 space-y-3">
        <p className="text-sm font-semibold text-tx-1">Add rule</p>
        <input value={newRule} onChange={e => setNewRule(e.target.value)} placeholder="Rule ID (e.g., web_search_allowed)" className="input-field text-sm" />
        <input value={notes} onChange={e => setNotes(e.target.value)} placeholder="Notes (optional)" className="input-field text-sm" />
        <button onClick={add} disabled={!newRule.trim()} className="btn-primary flex items-center gap-2 disabled:opacity-50">
          <Plus size={14} /> Add auto-approval
        </button>
      </div>
    </div>
  );
}
