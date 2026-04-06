import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import {
  Plus, Play, Pause, ChevronRight, Loader2, Clock,
  Zap, AlertCircle, CheckCircle2, XCircle, RotateCcw,
  Webhook, Calendar, Hand, GitBranch, Cpu, DollarSign,
  MessageSquare,
  Activity,
} from 'lucide-react';
import { agentDefs as agentDefsApi } from '../api';
import PlanModeChat from '../components/agent/PlanModeChat';
import AgentChatDrawer from '../components/agent/AgentChatDrawer';
import RoleChatDrawer from '../components/agent/RoleChatDrawer';
import AgentTimeline from '../components/agent/AgentTimeline';
import SwarmCanvas from '../components/agent/SwarmCanvas';
import RunDetailDrawer from '../components/agent/RunDetailDrawer';
import SavingsCard from '../components/cards/SavingsCard';
import AgentMessagesTab from '../components/agent/AgentMessagesTab';
import AgentTasksTab from '../components/agent/AgentTasksTab';
import AgentMemoryTab from '../components/agent/AgentMemoryTab';

// ── Helpers ────────────────────────────────────────────────────────────────
function timeAgo(iso) {
  if (!iso) return '—';
  const d = Date.now() - new Date(iso).getTime();
  const h = Math.floor(d / 3600000), m = Math.floor((d % 3600000) / 60000);
  if (h > 24) return `${Math.floor(h / 24)}d ago`;
  if (h > 0)  return `${h}h ago`;
  if (m > 0)  return `${m}m ago`;
  return 'just now';
}

function safeNumber(value, fallback = 0) {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

function TriggerIcon({ type }) {
  const icons = {
    webhook:         <Webhook size={12} />,
    schedule:        <Calendar size={12} />,
    manual:          <Hand size={12} />,
    user_message:    <Zap size={12} />,
    workforce_event: <GitBranch size={12} />,
  };
  return icons[type] || <Zap size={12} />;
}

function TriggerLabel({ trigger }) {
  if (!trigger) return <span className="text-tx-4">—</span>;
  const map = {
    webhook:         trigger.event_filter ? `On ${trigger.event_filter}` : 'Webhook',
    schedule:        trigger.cron || 'Scheduled',
    manual:          'Manual',
    user_message:    'On message',
    workforce_event: 'After another role',
  };
  return <span>{map[trigger.trigger_type] || trigger.trigger_type}</span>;
}

function StatusDot({ status }) {
  const cfg = {
    pending:   'bg-tx-4',
    running:   'bg-ok animate-pulse',
    completed: 'bg-ok',
    failed:    'bg-err',
    cancelled: 'bg-tx-4',
  }[status] || 'bg-tx-4';
  return <span className={clsx('size-2 rounded-full inline-block shrink-0', cfg)} />;
}

function pickTimelineRun(goalInstances) {
  if (!Array.isArray(goalInstances) || goalInstances.length === 0) return null;
  return goalInstances.find((gi) => gi.status === 'running' || gi.status === 'pending') || goalInstances[0];
}

function RoleStatusBadge({ status }) {
  const cfg = {
    draft:    { cls: 'bg-bg-active text-tx-3 border-border',          label: 'Draft' },
    testing:  { cls: 'bg-info-soft text-info border-info/20',         label: 'Testing' },
    active:   { cls: 'bg-ok-soft text-ok border-ok/20',               label: 'Active' },
    paused:   { cls: 'bg-warn-soft text-warn border-warn/20',         label: 'Paused' },
    archived: { cls: 'bg-bg-active text-tx-4 border-border',          label: 'Archived' },
  }[status] || { cls: 'bg-bg-active text-tx-3 border-border', label: status };
  return (
    <span className={clsx('inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold border', cfg.cls)}>
      {cfg.label}
    </span>
  );
}

// ── Role card ──────────────────────────────────────────────────────────────
function RoleCard({ role, agentId, onTrigger, onRefresh, onChat, onRunClick }) {
  const [expanded,   setExpanded]   = useState(false);
  const [instances,  setInstances]  = useState([]);
  const [loadingInst, setLoadingInst] = useState(false);
  const [triggering, setTriggering] = useState(false);

  async function loadInstances() {
    if (instances.length > 0) { setExpanded(p => !p); return; }
    setExpanded(true);
    setLoadingInst(true);
    try {
      const res = await agentDefsApi.listRoleInstances(agentId, role.id, 10);
      setInstances(res.goal_instances || []);
    } catch {}
    finally { setLoadingInst(false); }
  }

  async function handleTrigger(e) {
    e.stopPropagation();
    setTriggering(true);
    try {
      await agentDefsApi.triggerRole(agentId, role.id);
      onTrigger?.();
      // Reload instances after a short delay
      setTimeout(() => {
        agentDefsApi.listRoleInstances(agentId, role.id, 10)
          .then(r => setInstances(r.goal_instances || []));
        onRefresh?.();
      }, 1200);
    } catch {}
    finally { setTriggering(false); }
  }

  const canManualTrigger = ['manual', 'user_message'].includes(role.trigger?.trigger_type);

  return (
    <div className="rounded-xl border border-border bg-bg-card overflow-hidden">
      {/* Role header */}
      <button
        onClick={loadInstances}
        className="w-full flex items-center gap-3 px-4 py-3.5 hover:bg-bg-hover transition-colors text-left"
      >
        <ChevronRight
          size={14}
          className={clsx('text-tx-4 shrink-0 transition-transform', expanded && 'rotate-90')}
        />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-0.5">
            <p className="text-sm font-semibold text-tx-1 truncate">{role.name}</p>
            <RoleStatusBadge status={role.status} />
          </div>
          <p className="text-[11px] text-tx-3 truncate">{role.purpose || 'No description'}</p>
        </div>

        {/* Trigger chip */}
        <div className="flex items-center gap-1.5 text-[11px] text-tx-3 shrink-0 bg-bg px-2 py-1 rounded-lg border border-border">
          <TriggerIcon type={role.trigger?.trigger_type} />
          <TriggerLabel trigger={role.trigger} />
        </div>

        {/* Connectors */}
        {role.connectors?.length > 0 && (
          <div className="flex items-center gap-1 shrink-0">
            {role.connectors.slice(0, 3).map(c => (
              <span key={c} className="text-[10px] bg-accent-soft text-accent border border-accent/20 px-1.5 py-0.5 rounded">
                {c}
              </span>
            ))}
            {role.connectors.length > 3 && (
              <span className="text-[10px] text-tx-4">+{role.connectors.length - 3}</span>
            )}
          </div>
        )}

        {/* Manual trigger button */}
        {canManualTrigger && (
          <button
            onClick={handleTrigger}
            disabled={triggering}
            className="shrink-0 flex items-center gap-1 px-2.5 py-1.5 text-[11px] font-medium rounded-lg
                       bg-ok-soft text-ok border border-ok/20 hover:bg-ok/10 disabled:opacity-50 transition-colors"
          >
            {triggering ? <Loader2 size={10} className="animate-spin" /> : <Play size={10} />}
            Run
          </button>
        )}

        {/* Chat button */}
        <button
          onClick={e => { e.stopPropagation(); onChat(); }}
          className="shrink-0 flex items-center gap-1 px-2.5 py-1.5 text-[11px] font-medium rounded-lg
                     bg-bg text-tx-3 border border-border hover:text-accent hover:border-accent/40
                     hover:bg-accent-soft/20 transition-colors"
        >
          <MessageSquare size={10} />
          Chat
        </button>
      </button>

      {/* Expanded: recent runs */}
      <AnimatePresence>
        {expanded && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="overflow-hidden border-t border-border/50"
          >
            <div className="px-4 py-3">
              <p className="text-[11px] font-semibold text-tx-4 uppercase tracking-wider mb-2">Recent runs</p>
              {loadingInst ? (
                <div className="flex items-center gap-2 py-2">
                  <Loader2 size={12} className="animate-spin text-tx-4" />
                  <span className="text-xs text-tx-4">Loading…</span>
                </div>
              ) : instances.length === 0 ? (
                <p className="text-xs text-tx-4 py-2">No runs yet.{canManualTrigger ? ' Click Run to start one.' : ''}</p>
              ) : (
                <div className="space-y-1.5">
                  {instances.map(inst => (
                    <button
                      key={inst.id}
                      onClick={() => onRunClick(inst.id)}
                      className="w-full flex items-center gap-2.5 text-xs py-1.5 px-2 rounded-lg
                                 hover:bg-bg-hover transition-colors text-left group"
                    >
                      <StatusDot status={inst.status} />
                      <span className="text-tx-2 capitalize">{inst.status.replace(/_/g, ' ')}</span>
                      {safeNumber(inst.cost_usd) > 0 && (
                        <span className="flex items-center gap-0.5 text-tx-4">
                          <DollarSign size={9} />
                          {safeNumber(inst.cost_usd).toFixed(4)}
                        </span>
                      )}
                      {safeNumber(inst.human_hours_saved) > 0 && (
                        <span className="text-ok text-[10px]">
                          +{safeNumber(inst.human_hours_saved) < 1
                            ? `${Math.round(safeNumber(inst.human_hours_saved) * 60)}m`
                            : `${safeNumber(inst.human_hours_saved).toFixed(1)}h`} saved
                        </span>
                      )}
                      <span className="ml-auto text-tx-4">{timeAgo(inst.created_at)}</span>
                      <ChevronRight size={10} className="text-tx-5 opacity-0 group-hover:opacity-100 transition-opacity" />
                    </button>
                  ))}
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

// ── Main AgentPage ─────────────────────────────────────────────────────────
export default function AgentPage({ agentId, onBack, onNavigateSettings = null }) {
  const [agent,       setAgent]      = useState(null);
  const [roles,       setRoles]      = useState([]);
  const [goalInstances, setGoalInstances] = useState([]);
  const [loading,     setLoading]    = useState(true);
  const [error,       setError]      = useState('');
  const [showAddRole, setShowAddRole] = useState(false);
  const [showAgentChat, setShowAgentChat] = useState(false);
  const [chatRole,    setChatRole]   = useState(null); // { id, name } | null
  const [selectedRun, setSelectedRun] = useState(null); // goal_instance id | null
  const [activeTab,   setActiveTab]  = useState('timeline');

  const load = useCallback(async () => {
    if (!agentId) return;
    setLoading(true);
    try {
      const [agentRes, rolesRes, runsRes] = await Promise.all([
        agentDefsApi.get(agentId),
        agentDefsApi.listRoles(agentId),
        agentDefsApi.listGoalInstances(agentId, 25),
      ]);
      setAgent(agentRes);
      setRoles(rolesRes.roles || []);
      setGoalInstances(runsRes.goal_instances || []);
    } catch (e) {
      setError(e.message);
    } finally {
      setLoading(false);
    }
  }, [agentId]);

  useEffect(() => { load(); }, [load]);

  function handleRoleAdded({ agentId: _aid, roleId }) {
    setShowAddRole(false);
    load(); // refresh to show new role
  }

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 size={20} className="text-tx-4 animate-spin" />
      </div>
    );
  }

  if (error || !agent) {
    return (
      <div className="flex-1 flex items-center justify-center flex-col gap-3">
        <AlertCircle size={20} className="text-err" />
        <p className="text-sm text-tx-3">{error || 'Agent not found'}</p>
        <button onClick={onBack} className="btn-secondary text-sm">Back</button>
      </div>
    );
  }

  const activeRoles    = roles.filter(r => r.status === 'active');
  const inactiveRoles  = roles.filter(r => r.status !== 'active');
  const pendingRoles   = roles.filter(r => ['draft', 'testing'].includes(r.status));
  const activeRun = pickTimelineRun(goalInstances);
  const runtimeAgentId = activeRun?.agent_state_id || activeRun?.agent_id || null;
  const runtimeStatus = activeRun?.status || agent.status || 'draft';
  const runtimeAgent = runtimeAgentId ? {
    ...agent,
    id: runtimeAgentId,
    goal: activeRun?.input_data?.description || agent.goal || agent.name,
    status: runtimeStatus,
  } : null;
  const liveRoleCount = activeRoles.length;
  const totalRuns = goalInstances.length;
  const nextActionLabel = runtimeAgentId
    ? 'Review the live timeline'
    : liveRoleCount > 0
      ? 'Run an active role to create a live trace'
      : 'Add the first role to define behavior';

  return (
    <>
      <div className="flex-1 flex flex-col min-h-0 overflow-y-auto">

        {/* ── Agent header ────────────────────────────────────── */}
        <div className="px-6 py-5 border-b border-border bg-bg-card/80 backdrop-blur shrink-0">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <div className="flex items-center gap-2 mb-1">
                <Cpu size={16} className="text-accent shrink-0" />
                <h1 className="text-base font-semibold text-tx-1 truncate">{agent.name}</h1>
                <span className={clsx(
                  'px-2 py-0.5 rounded text-[10px] font-semibold border',
                  agent.status === 'active'
                    ? 'bg-ok-soft text-ok border-ok/20'
                    : 'bg-bg-active text-tx-3 border-border',
                )}>
                  {agent.status || 'draft'}
                </span>
                {pendingRoles.length > 0 && (
                  <span className="px-2 py-0.5 rounded text-[10px] font-semibold border bg-info-soft text-info border-info/20">
                    {pendingRoles.length} pending
                  </span>
                )}
              </div>
              {agent.persona && (
                <p className="text-xs text-tx-3 leading-relaxed max-w-lg">{agent.persona}</p>
              )}
              {/* Allowed connectors */}
              {agent.connectors?.length > 0 && (
                <div className="flex items-center gap-1.5 mt-2 flex-wrap">
                  <span className="text-[10px] text-tx-4">Can use:</span>
                  {agent.connectors.map(c => (
                    <span key={c} className="text-[10px] bg-accent-soft text-accent border border-accent/20 px-1.5 py-0.5 rounded">
                      {c}
                    </span>
                  ))}
                </div>
              )}
              <div className="mt-4 grid gap-2 sm:grid-cols-4">
                {[
                  { label: 'State', value: agent.status || 'draft', hint: runtimeAgentId ? 'Live run attached' : 'Waiting to run' },
                  { label: 'Roles', value: String(roles.length), hint: liveRoleCount > 0 ? `${liveRoleCount} active` : 'Add the first role' },
                  { label: 'Runs', value: String(totalRuns), hint: totalRuns > 0 ? 'Recent history' : 'No runs yet' },
                  { label: 'Next', value: runtimeAgentId ? 'Timeline' : 'Setup', hint: nextActionLabel },
                ].map(card => (
                  <div key={card.label} className="rounded-2xl border border-border bg-bg-card px-3 py-2.5">
                    <p className="text-[10px] uppercase tracking-[0.22em] text-tx-4">{card.label}</p>
                    <p className="mt-1 text-sm font-semibold text-tx-1 capitalize">{card.value}</p>
                    <p className="mt-1 text-[11px] leading-5 text-tx-3">{card.hint}</p>
                  </div>
                ))}
              </div>
            </div>

            {/* Add Role button */}
            <button
              onClick={() => setShowAddRole(true)}
              className="btn-primary shrink-0 flex items-center gap-2"
            >
              <Plus size={14} />
              Add role
            </button>
            <button
              onClick={() => setShowAgentChat(true)}
              className="btn-secondary shrink-0 flex items-center gap-2"
            >
              <MessageSquare size={14} />
              Open chat
            </button>
          </div>

          {/* Constraints */}
          {agent.constraints?.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-1.5">
              {agent.constraints.map((c, i) => (
                <span key={i} className="inline-flex items-center gap-1 text-[11px] bg-warn-soft text-warn border border-warn/20 px-2 py-0.5 rounded">
                  {c}
                </span>
              ))}
            </div>
          )}
        </div>

        {/* ── Tabs Header ──────────────────────────────────────── */}
        <div className="px-6 py-0 border-b border-border bg-bg flex items-center gap-6">
          <button
            onClick={() => setActiveTab('timeline')}
            className={clsx('py-3 text-sm font-medium border-b-2 transition-colors', activeTab === 'timeline' ? 'border-accent text-accent' : 'border-transparent text-tx-3 hover:text-tx-2')}
          >
            Timeline
          </button>
          <button
            onClick={() => setActiveTab('swarm')}
            className={clsx('py-3 text-sm font-medium border-b-2 transition-colors', activeTab === 'swarm' ? 'border-accent text-accent' : 'border-transparent text-tx-3 hover:text-tx-2')}
          >
            Swarm
          </button>
          <button
            onClick={() => setActiveTab('roles')}
            className={clsx('py-3 text-sm font-medium border-b-2 transition-colors', activeTab === 'roles' ? 'border-accent text-accent' : 'border-transparent text-tx-3 hover:text-tx-2')}
          >
            Roles
          </button>
          <button
            onClick={() => setActiveTab('messages')}
            className={clsx('py-3 text-sm font-medium border-b-2 transition-colors', activeTab === 'messages' ? 'border-accent text-accent' : 'border-transparent text-tx-3 hover:text-tx-2')}
          >
            Messages
          </button>
          <button
            onClick={() => setActiveTab('tasks')}
            className={clsx('py-3 text-sm font-medium border-b-2 transition-colors', activeTab === 'tasks' ? 'border-accent text-accent' : 'border-transparent text-tx-3 hover:text-tx-2')}
          >
            Tasks
          </button>
          <button
            onClick={() => setActiveTab('memory')}
            className={clsx('py-3 text-sm font-medium border-b-2 transition-colors', activeTab === 'memory' ? 'border-accent text-accent' : 'border-transparent text-tx-3 hover:text-tx-2')}
          >
            Memory
          </button>
        </div>

        {/* ── Tab Content ──────────────────────────────────────── */}
        <div className="px-6 py-5 space-y-5">

          {activeTab === 'timeline' && (
            <div className="rounded-2xl border border-border bg-bg-card p-4">
              {runtimeAgentId ? (
                <AgentTimeline
                  agentId={runtimeAgentId}
                  initialStatus={runtimeStatus}
                  onNavigateSettings={onNavigateSettings}
                />
              ) : (
                <div className="flex flex-col items-center justify-center py-16 text-center">
                  <div className="size-12 rounded-2xl bg-bg-active flex items-center justify-center mb-4">
                    <Activity size={20} className="text-tx-4" />
                  </div>
                  <p className="text-sm font-medium text-tx-1 mb-1">No live run yet</p>
                  <p className="text-xs text-tx-3 max-w-xs leading-relaxed">
                    Run an active role to create a live runtime trace. The timeline will attach to it automatically.
                  </p>
                </div>
              )}
            </div>
          )}
          {activeTab === 'swarm' && (
            <div className="h-[70vh] rounded-2xl border border-border bg-bg-card overflow-hidden">
              {runtimeAgent ? (
                <SwarmCanvas parentAgent={runtimeAgent} onBack={() => setActiveTab('timeline')} />
              ) : (
                <div className="flex h-full items-center justify-center p-6 text-center">
                  <div>
                    <p className="text-sm font-medium text-tx-1 mb-1">No live swarm to inspect</p>
                    <p className="text-xs text-tx-3 max-w-sm leading-relaxed">
                      Once a role is running, this view will show the runtime agent and any children it creates.
                    </p>
                  </div>
                </div>
              )}
            </div>
          )}
          {activeTab === 'messages' && <AgentMessagesTab agentId={runtimeAgentId} />}
          {activeTab === 'tasks' && <AgentTasksTab agentId={runtimeAgentId} />}
          {activeTab === 'memory' && <AgentMemoryTab agentId={runtimeAgentId} />}

          {activeTab === 'roles' && roles.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-16 text-center">
              <div className="size-12 rounded-2xl bg-accent-soft border border-accent/20 flex items-center justify-center mb-4">
                <Zap size={20} className="text-accent" />
              </div>
              <p className="text-sm font-medium text-tx-1 mb-1">No roles yet</p>
              <p className="text-xs text-tx-3 max-w-xs leading-relaxed mb-4">
                Roles define what this agent does, when it runs, and where it sends results.
              </p>
              <button onClick={() => setShowAddRole(true)} className="btn-primary flex items-center gap-2">
                <Plus size={14} />
                Add first role
              </button>
            </div>
          ) : (
            <>
              <SavingsCard className="mb-2" />
              {pendingRoles.length > 0 && (
                <section>
              <p className="section-label mb-3 text-info">Pending review ({pendingRoles.length})</p>
              <p className="mb-3 text-xs leading-6 text-tx-3">
                These roles are drafted but not live yet. Review them before the agent can run.
              </p>
              <div className="space-y-3">
                {pendingRoles.map(role => (
                  <RoleCard
                    key={role.id}
                        role={role}
                        agentId={agentId}
                        onRefresh={load}
                        onChat={() => setChatRole({ id: role.id, name: role.name })}
                        onRunClick={id => setSelectedRun(id)}
                      />
                    ))}
                  </div>
                </section>
              )}
              {activeRoles.length > 0 && (
                <section>
                  <p className="section-label mb-3">Active roles</p>
                  <p className="mb-3 text-xs leading-6 text-tx-3">
                    Active roles can run now and contribute to the live timeline.
                  </p>
                  <div className="space-y-3">
                    {activeRoles.map(role => (
                      <RoleCard
                        key={role.id}
                        role={role}
                        agentId={agentId}
                        onRefresh={load}
                        onChat={() => setChatRole({ id: role.id, name: role.name })}
                        onRunClick={id => setSelectedRun(id)}
                      />
                    ))}
                  </div>
                </section>
              )}

              {inactiveRoles.filter(r => !['draft', 'testing'].includes(r.status)).length > 0 && (
                <section>
                  <p className="section-label mb-3">Draft / paused roles</p>
                  <p className="mb-3 text-xs leading-6 text-tx-3">
                    These roles exist in the agent, but they need attention before they can run.
                  </p>
                  <div className="space-y-3">
                    {inactiveRoles.filter(r => !['draft', 'testing'].includes(r.status)).map(role => (
                      <RoleCard
                        key={role.id}
                        role={role}
                        agentId={agentId}
                        onRefresh={load}
                        onChat={() => setChatRole({ id: role.id, name: role.name })}
                        onRunClick={id => setSelectedRun(id)}
                      />
                    ))}
                  </div>
                </section>
              )}
            </>
          )}
        </div>
      </div>

      {/* ── Add Role modal ───────────────────────────────────── */}
      {showAddRole && (
        <PlanModeChat
          agentName={agent.name}
          existingAgentId={agentId}
          onComplete={handleRoleAdded}
          onCancel={() => setShowAddRole(false)}
        />
      )}

      {/* ── Agent chat drawer ─────────────────────────────────── */}
      <AnimatePresence>
        {showAgentChat && (
          <AgentChatDrawer
            agentId={agentId}
            agentName={agent.name}
            agent={agent}
            roles={roles}
            onClose={() => setShowAgentChat(false)}
          />
        )}
      </AnimatePresence>

      {/* ── Role chat drawer ─────────────────────────────────── */}
      <AnimatePresence>
        {chatRole && (
          <RoleChatDrawer
            roleId={chatRole.id}
            agentId={agentId}
            roleName={chatRole.name}
            onClose={() => setChatRole(null)}
            onRoleChanged={() => { setChatRole(null); load(); }}
          />
        )}
      </AnimatePresence>

      {/* ── Run detail drawer ────────────────────────────────── */}
      <AnimatePresence>
        {selectedRun && (
          <RunDetailDrawer
            instanceId={selectedRun}
            onClose={() => setSelectedRun(null)}
          />
        )}
      </AnimatePresence>
    </>
  );
}
