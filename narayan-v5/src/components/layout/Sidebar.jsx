import { useState } from 'react';
import clsx from 'clsx';
import { Settings, LogOut, Bell, Plus, Loader2, Cpu, Zap, ChevronRight } from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';

function timeAgo(iso) {
  if (!iso) return '';
  const d = Date.now() - new Date(iso).getTime();
  const h = Math.floor(d / 3600000), m = Math.floor((d % 3600000) / 60000);
  if (h > 24) return `${Math.floor(h / 24)}d ago`;
  if (h > 0)  return `${h}h ago`;
  if (m > 0)  return `${m}m ago`;
  return 'just now';
}

const ROLE_STATUS_DOT = {
  active:   'bg-ok',
  testing:  'bg-info',
  paused:   'bg-warn',
  draft:    'bg-tx-4',
  archived: 'bg-tx-4 opacity-40',
};

// ── Single agent item ──────────────────────────────────────────────────────
function AgentItem({ agent, selected, expanded, onToggleExpand, onClick }) {
  const roles = agent.roles || [];
  const activeCount = roles.filter(r => r.status === 'active').length;

  return (
    <div>
      <motion.div
        onClick={onClick}
        role="button"
        tabIndex={0}
        onKeyDown={e => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onClick?.(e);
          }
        }}
        layout
        layoutId={agent.id}
        className={clsx(
          'w-full text-left rounded-lg transition-all group cursor-pointer',
          selected
            ? 'bg-bg-active border-l-2 border-l-accent pl-2.5 pr-3 py-2.5'
            : 'hover:bg-bg-hover px-3 py-2.5',
        )}
      >
        <div className="flex items-center gap-2">
          {roles.length > 0 && (
            <button
              onClick={e => { e.stopPropagation(); onToggleExpand(); }}
              className="p-0.5 rounded text-tx-4 hover:text-tx-2 transition-all shrink-0"
            >
              <ChevronRight
                size={11}
                className={clsx('transition-transform', expanded && 'rotate-90')}
              />
            </button>
          )}
          <Cpu size={12} className={clsx('shrink-0', selected ? 'text-accent' : 'text-tx-4')} />
          <p className="text-xs font-medium text-tx-1 truncate flex-1">{agent.name}</p>
          <span className={clsx(
            'text-[9px] font-semibold uppercase shrink-0 px-1 py-0.5 rounded',
            agent.status === 'active'
              ? 'text-ok bg-ok-soft'
              : 'text-tx-4 bg-bg-active',
          )}>
            {agent.status || 'draft'}
          </span>
        </div>
        <div className="flex items-center gap-2 text-[10px] text-tx-4 mt-0.5 pl-5">
          {roles.length > 0
            ? <><span>{roles.length} role{roles.length !== 1 ? 's' : ''}</span>
                {activeCount > 0 && <span className="text-ok">{activeCount} active</span>}</>
            : <span className="italic">No roles yet</span>}
          <span className="ml-auto">{timeAgo(agent.updated_at)}</span>
        </div>
      </motion.div>

      {/* Expanded role list */}
      <AnimatePresence>
        {expanded && roles.length > 0 && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            className="overflow-hidden"
          >
            {roles.map(role => (
              <div
                key={role.id}
                className="flex items-center gap-2 pl-7 pr-3 py-1.5 text-[10px] text-tx-3"
              >
                <span className={clsx(
                  'size-1.5 rounded-full shrink-0',
                  ROLE_STATUS_DOT[role.status] || 'bg-tx-4',
                )} />
                <span className="truncate flex-1">{role.name}</span>
                <span className="text-tx-4 capitalize shrink-0">{role.status}</span>
              </div>
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

// ── Main Sidebar ───────────────────────────────────────────────────────────
export default function Sidebar({
  agents, selectedAgentId, onSelectAgent, onNewAgent,
  onNavigate, pendingReviews = [], loading, canCreateAgents = true,
}) {
  // Expanded set lives here so it survives parent re-renders
  const [expandedIds, setExpandedIds] = useState(() => new Set());

  function toggleExpand(id) {
    setExpandedIds(prev => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }

  return (
    <aside className="flex h-screen w-72 shrink-0 flex-col border-r border-border bg-bg-card/90 backdrop-blur">

      {/* Header */}
      <div className="flex items-start justify-between gap-3 border-b border-border px-4 py-4">
        <div>
          <p className="font-serif text-xl leading-none text-tx-1">Narayan</p>
          <p className="mt-1 text-[0.7rem] uppercase tracking-[0.24em] text-tx-4">Operational workspace</p>
        </div>
        <div className="flex items-center gap-0.5">
          {pendingReviews.length > 0 && (
            <button
              onClick={() => onNavigate('settings')}
              className="relative rounded-lg p-1.5 text-warn transition-all hover:bg-warn-soft"
              title={`${pendingReviews.length} pending`}
            >
              <Bell size={15} />
              <span className="absolute -right-0.5 -top-0.5 flex h-[14px] min-w-[14px] items-center justify-center rounded-full bg-warn px-0.5 text-[9px] font-bold text-bg-card">
                {pendingReviews.length}
              </span>
            </button>
          )}
          <button
            onClick={() => onNavigate('settings')}
            className="rounded-lg p-1.5 text-tx-3 transition-all hover:bg-bg-hover hover:text-tx-1"
            title="Settings"
          >
            <Settings size={15} />
          </button>
          <button
            onClick={() => onNavigate('logout')}
            className="rounded-lg p-1.5 text-tx-3 transition-all hover:bg-err-soft hover:text-err"
            title="Sign out"
          >
            <LogOut size={15} />
          </button>
        </div>
      </div>

      {/* New agent button */}
      <div className="px-3 pt-3 pb-2">
        <button
          onClick={() => {
            if (!canCreateAgents) {
              onNavigate('settings');
              return;
            }
            onNewAgent();
          }}
          className="flex w-full items-center gap-2 rounded-2xl border border-accent/20 bg-gradient-to-r from-accent to-accent-text px-4 py-3 text-left text-sm font-medium text-white shadow-[0_12px_30px_rgba(201,106,46,0.2)] transition-all hover:translate-y-[-1px] active:scale-[0.99]"
        >
          <Plus size={13} />
          {canCreateAgents ? 'New agent' : 'Add AI provider'}
        </button>
        {!canCreateAgents && (
          <p className="mt-2 px-1 text-[11px] leading-5 text-tx-4">
            Add one provider in Settings to unlock agent creation.
          </p>
        )}
      </div>

      {/* Agent list */}
      <div className="flex-1 overflow-y-auto px-2 py-1 space-y-0.5">
        {loading ? (
          <div className="flex items-center justify-center py-8">
            <Loader2 size={16} className="text-tx-4 animate-spin" />
          </div>
        ) : agents.length === 0 ? (
          <div className="px-3 py-8 text-center">
            <div className="mx-auto mb-3 flex size-10 items-center justify-center rounded-2xl border border-accent/20 bg-accent-soft">
              <Zap size={18} className="text-accent" />
            </div>
            <p className="text-xs font-medium text-tx-1 mb-1">No agents yet</p>
            <p className="text-[11px] text-tx-4 leading-relaxed">
              Create your first agent to automate a workflow.
            </p>
          </div>
        ) : (
          <>
            <p className="section-label px-2 pt-2 pb-1">Your agents</p>
            {agents.map(agent => (
              <AgentItem
                key={agent.id}
                agent={agent}
                selected={agent.id === selectedAgentId}
                expanded={expandedIds.has(agent.id)}
                onToggleExpand={() => toggleExpand(agent.id)}
                onClick={() => onSelectAgent(agent.id)}
              />
            ))}
          </>
        )}
      </div>

      <div className="border-t border-border px-4 py-4">
        <div className="rounded-2xl border border-border bg-bg-hover px-3 py-3">
          <p className="text-[0.7rem] uppercase tracking-[0.24em] text-tx-4">Workspace health</p>
          <div className="mt-2 flex items-center gap-2 text-sm text-tx-2">
            <span className="size-2 rounded-full bg-ok" />
            Connected and syncing
          </div>
        </div>
      </div>
    </aside>
  );
}
