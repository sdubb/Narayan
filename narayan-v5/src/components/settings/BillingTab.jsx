import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { CreditCard, Loader2, CheckCircle2, ExternalLink, Zap } from 'lucide-react';
import { billing } from '../../api';

const PLANS = [
  { id: 'free', name: 'Free', price: '$0', steps: '1,000', agents: '3' },
  { id: 'go', name: 'Go', price: '$15/mo', steps: '20,000', agents: '20' },
  { id: 'pro', name: 'Pro', price: '$79/mo', steps: '150,000', agents: '200' },
  { id: 'enterprise', name: 'Enterprise', price: 'Custom', steps: 'Unlimited', agents: 'Unlimited' },
];

export default function BillingTab({ onFlash }) {
  const [sub, setSub] = useState(null);
  const [invoices, setInvoices] = useState([]);
  const [credits, setCredits] = useState(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([
      billing.subscription().catch(() => null),
      billing.invoices().catch(() => ({ invoices: [] })),
      billing.credits().catch(() => null),
    ]).then(([s, inv, c]) => { setSub(s); setInvoices(inv.invoices || []); setCredits(c); })
      .finally(() => setLoading(false));
  }, []);

  async function checkout(plan) {
    try {
      const result = await billing.checkout(plan);
      if (result.redirect_url) window.location.href = result.redirect_url;
    } catch (e) { onFlash?.(e.message); }
  }

  async function cancel() {
    try {
      await billing.cancelSubscription();
      onFlash?.('Subscription cancelled');
      const s = await billing.subscription();
      setSub(s);
    } catch (e) { onFlash?.(e.message); }
  }

  async function buyCredits() {
    try {
      const result = await billing.purchaseCredits();
      if (result.redirect_url) window.location.href = result.redirect_url;
    } catch (e) { onFlash?.(e.message); }
  }

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  const currentPlan = sub?.plan || 'free';

  return (
    <div className="space-y-6">
      {/* Current plan hero */}
      {sub && (
        <div className="card p-5 border-l-4 border-l-accent">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-lg font-semibold text-tx-1 capitalize">{currentPlan} Plan</p>
              {sub.next_billing_date && <p className="text-xs text-tx-3 mt-0.5">Renews {new Date(sub.next_billing_date).toLocaleDateString()}</p>}
            </div>
            {currentPlan !== 'free' && (
              <button onClick={cancel} className="btn-ghost text-xs text-err">Cancel</button>
            )}
          </div>
        </div>
      )}

      {/* Plan comparison */}
      <div className="grid grid-cols-4 gap-3">
        {PLANS.map(plan => {
          const isCurrent = plan.id === currentPlan;
          return (
            <div key={plan.id} className={clsx('card p-4 text-center transition-all', isCurrent && 'ring-2 ring-accent/30 border-accent/30')}>
              {isCurrent && <span className="badge bg-accent-soft text-accent border border-accent/20 mb-2 mx-auto">Current</span>}
              <p className="text-base font-semibold text-tx-1">{plan.name}</p>
              <p className="text-lg font-bold text-accent mt-1">{plan.price}</p>
              <div className="mt-3 space-y-1 text-xs text-tx-3">
                <p>{plan.steps} steps/mo</p>
                <p>{plan.agents} agents</p>
              </div>
              {!isCurrent && plan.id !== 'enterprise' && (
                <button onClick={() => checkout(plan.id)}
                  className={clsx('mt-3 w-full text-xs rounded-lg px-3 py-2 font-medium transition-all',
                    PLANS.indexOf(plan) > PLANS.findIndex(p => p.id === currentPlan)
                      ? 'btn-primary' : 'btn-secondary')}>
                  {PLANS.indexOf(plan) > PLANS.findIndex(p => p.id === currentPlan) ? 'Upgrade' : 'Switch'}
                </button>
              )}
              {plan.id === 'enterprise' && !isCurrent && (
                <button className="mt-3 w-full btn-secondary text-xs">Contact sales</button>
              )}
            </div>
          );
        })}
      </div>

      {/* Credits */}
      <div className="card p-5">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-semibold text-tx-1">Step Credits</p>
            <p className="text-xs text-tx-3 mt-0.5">{credits?.remaining ?? 0} extra steps available</p>
          </div>
          <button onClick={buyCredits} className="btn-primary text-xs flex items-center gap-1.5">
            <Zap size={12} /> Buy 5,000 steps — $8
          </button>
        </div>
      </div>

      {/* Invoices */}
      {invoices.length > 0 && (
        <div>
          <p className="text-sm font-semibold text-tx-1 mb-3">Invoices</p>
          <div className="card overflow-hidden divide-y divide-border/60">
            {invoices.map((inv, i) => (
              <div key={i} className="flex items-center gap-3 px-4 py-3 hover:bg-bg-hover transition-colors">
                <CreditCard size={14} className="text-tx-3 shrink-0" />
                <span className="text-xs text-tx-1 flex-1">{inv.description || `Invoice #${i + 1}`}</span>
                <span className="text-xs font-mono text-tx-3">${(inv.amount || 0).toFixed(2)}</span>
                <span className={clsx('text-[10px]', inv.status === 'paid' ? 'text-ok' : 'text-warn')}>{inv.status}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
