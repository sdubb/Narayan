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
  { id: 'credentials', label: 'Credentials', icon: Key },
  { id: 'routing', label: 'Routing', icon: Network },
  { id: 'usage', label: 'Usage', icon: BarChart3 },
  { id: 'skills', label: 'Skills', icon: BookOpen },
  { id: 'reviews', label: 'Reviews', icon: Bell },
  { id: 'citations', label: 'Citations', icon: Link2 },
  { id: 'connectors', label: 'Connectors', icon: Plug },
  { id: 'autoapprovals', label: 'Auto-approvals', icon: Shield },
  { id: 'billing', label: 'Billing', icon: DollarSign },
];

const TAB_COMPONENTS = {
  credentials: CredentialsTab,
  routing: RoutingTab,
  usage: UsageTab,
  skills: SkillsTab,
  reviews: ReviewsTab,
  citations: CitationsTab,
  connectors: ConnectorsTab,
  autoapprovals: AutoApprovalsTab,
  billing: BillingTab,
};

export default function SettingsPage({ onBack, canCreateAgents = true, onProvidersChanged }) {
  const [tab, setTab] = useState('credentials');
  const [error, setError] = useState('');
  const [ok, setOk] = useState('');

  function flash(message) {
    setOk(message);
    window.setTimeout(() => setOk(''), 3000);
  }

  const ActiveTab = TAB_COMPONENTS[tab];

  return (
    <div className="relative min-h-screen overflow-hidden bg-[radial-gradient(circle_at_top_left,_rgba(201,106,46,0.08),_transparent_24%),linear-gradient(180deg,_#f7f4ef_0%,_#f4efe8_100%)]">
      <div className="pointer-events-none absolute inset-0">
        <div className="absolute right-[-4rem] top-16 h-72 w-72 rounded-full bg-info/10 blur-3xl" />
      </div>

      <div className="sticky top-0 z-10 border-b border-border bg-bg-card/88 backdrop-blur">
        <div className="mx-auto flex max-w-5xl items-center gap-4 px-6 py-4">
          <button onClick={onBack} className="flex items-center gap-1.5 text-sm text-tx-3 transition-colors hover:text-tx-1">
            <ChevronLeft size={15} /> {canCreateAgents ? 'Back' : 'Go to workspace'}
          </button>
          <div className="min-w-0 flex-1">
            <p className="font-serif text-2xl text-tx-1">
              {canCreateAgents ? 'Settings' : 'Connect your first AI provider'}
            </p>
            <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">
              {canCreateAgents
                ? 'Credentials, routing, reviews, and billing'
                : 'Add one provider to unlock agent creation'}
            </p>
          </div>
        </div>
        <div className="mx-auto max-w-5xl overflow-x-auto px-4 pb-0">
          <div className="flex min-w-max gap-1 rounded-t-2xl border border-border border-b-0 bg-bg px-2 pt-2">
            {TABS.map(t => {
              const Icon = t.icon;
              return (
                <button
                  key={t.id}
                  onClick={() => {
                    setError('');
                    setTab(t.id);
                  }}
                  className={clsx(
                    'flex items-center gap-1.5 rounded-t-xl px-4 py-2.5 text-sm font-medium transition-all whitespace-nowrap',
                    tab === t.id
                      ? 'bg-bg-card text-tx-1 shadow-card'
                      : 'text-tx-3 hover:bg-bg-hover hover:text-tx-1',
                  )}
                >
                  <Icon size={14} />
                  {t.label}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      <div className="mx-auto max-w-5xl px-6 py-8">
        {!canCreateAgents && (
          <div className="mb-6 rounded-[1.5rem] border border-accent/20 bg-accent-soft/60 px-5 py-4 shadow-card">
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex size-10 items-center justify-center rounded-2xl border border-accent/20 bg-bg-card text-accent">
                <Key size={16} />
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-semibold text-tx-1">Add one AI provider to continue</p>
                <p className="mt-1 text-sm leading-6 text-tx-2">
                  Agent creation stays paused until you save a provider API key. The rest of the workspace remains open,
                  and we will take you back as soon as setup is done.
                </p>
              </div>
            </div>
          </div>
        )}
        <div className="space-y-4">
          {error && (
            <div className="flex items-start gap-2 rounded-2xl border border-err/20 bg-err-soft px-4 py-3 text-sm text-err">
              <AlertCircle size={14} className="mt-0.5 shrink-0" />
              {error}
            </div>
          )}
          {ok && (
            <div className="flex items-center gap-2 rounded-2xl border border-ok/20 bg-ok-soft px-4 py-3 text-sm text-ok">
              <CheckCircle2 size={14} />
              {ok}
            </div>
          )}
        </div>

        <div className="mt-6 overflow-hidden rounded-[2rem] border border-border bg-bg-card/90 shadow-[0_20px_45px_rgba(26,23,20,0.06)]">
          <div className="border-b border-border px-6 py-4">
            <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent">Workspace controls</p>
            <h1 className="mt-2 font-serif text-2xl text-tx-1">{TABS.find(t => t.id === tab)?.label}</h1>
          </div>
          <div className="p-6">
            {ActiveTab && <ActiveTab onFlash={flash} setError={setError} flash={flash} onProvidersChanged={onProvidersChanged} />}
          </div>
        </div>
      </div>
    </div>
  );
}
