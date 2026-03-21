import { useState } from 'react';
import {
  Key, Network, BarChart3, BookOpen, Bell, Link2, Plug, Shield, DollarSign,
  ChevronLeft, CheckCircle2, AlertCircle,
} from 'lucide-react';
import clsx from 'clsx';
import {
  CredentialsTab, RoutingTab, UsageTab, SkillsTab, ReviewsTab,
  CitationsTab, ConnectorsTab, AutoApprovalsTab, BillingTab,
} from '../components/settings';

const TABS = [
  { id: 'credentials',   label: 'Credentials',   icon: Key },
  { id: 'routing',       label: 'Routing',        icon: Network },
  { id: 'usage',         label: 'Usage',          icon: BarChart3 },
  { id: 'skills',        label: 'Skills',         icon: BookOpen },
  { id: 'reviews',       label: 'Reviews',        icon: Bell },
  { id: 'citations',     label: 'Citations',      icon: Link2 },
  { id: 'connectors',    label: 'Connectors',     icon: Plug },
  { id: 'autoapprovals', label: 'Auto-approvals', icon: Shield },
  { id: 'billing',       label: 'Billing',        icon: DollarSign },
];

const TAB_COMPONENTS = {
  credentials:   CredentialsTab,
  routing:       RoutingTab,
  usage:         UsageTab,
  skills:        SkillsTab,
  reviews:       ReviewsTab,
  citations:     CitationsTab,
  connectors:    ConnectorsTab,
  autoapprovals: AutoApprovalsTab,
  billing:       BillingTab,
};

export default function SettingsPage({ onBack }) {
  const [tab, setTab]     = useState('credentials');
  const [error, setError] = useState('');
  const [ok, setOk]       = useState('');

  function flash(m) { setOk(m); setTimeout(() => setOk(''), 3000); }

  const ActiveTab = TAB_COMPONENTS[tab];

  return (
    <div className="min-h-screen bg-bg">
      {/* Header */}
      <div className="border-b border-border bg-bg-card sticky top-0 z-10">
        <div className="max-w-2xl mx-auto px-6 py-4 flex items-center gap-4">
          <button onClick={onBack} className="flex items-center gap-1.5 text-sm text-tx-3 hover:text-tx-1 transition-colors">
            <ChevronLeft size={15} /> Back
          </button>
          <p className="font-serif text-xl text-tx-1 flex-1">Settings</p>
        </div>
        <div className="max-w-2xl mx-auto px-6 flex gap-0 overflow-x-auto">
          {TABS.map(t => {
            const Icon = t.icon;
            return (
              <button key={t.id} onClick={() => { setError(''); setTab(t.id); }}
                className={clsx('flex items-center gap-1.5 px-4 py-2.5 text-sm font-medium border-b-2 transition-all whitespace-nowrap',
                  tab === t.id ? 'border-accent text-accent' : 'border-transparent text-tx-3 hover:text-tx-1')}>
                <Icon size={14} />{t.label}
              </button>
            );
          })}
        </div>
      </div>

      <div className="max-w-2xl mx-auto px-6 py-8 space-y-6">
        {error && (
          <div className="flex items-start gap-2 rounded-xl bg-err-soft border border-err/20 px-4 py-3 text-sm text-err animate-fade">
            <AlertCircle size={14} className="mt-0.5 shrink-0" />{error}
          </div>
        )}
        {ok && (
          <div className="flex items-center gap-2 rounded-xl bg-ok-soft border border-ok/20 px-4 py-3 text-sm text-ok animate-fade">
            <CheckCircle2 size={14} />{ok}
          </div>
        )}

        {ActiveTab && <ActiveTab onFlash={flash} setError={setError} flash={flash} />}
      </div>
    </div>
  );
}
