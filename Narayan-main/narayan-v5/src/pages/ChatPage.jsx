import { useState, useEffect, useCallback, useRef } from 'react';
import { reviews as reviewsApi, swarm, agentDefs as agentDefsApi } from '../api';
import { Sidebar } from '../components/layout';
import PlanModeChat from '../components/agent/PlanModeChat';
import AgentPage from './AgentPage';
import SavingsCard from '../components/cards/SavingsCard';
import { Cpu, Zap } from 'lucide-react';
import { motion } from 'framer-motion';

// ── Empty state ────────────────────────────────────────────────────────────
function EmptyState({ onNew }) {
  return (
    <div className="flex-1 flex flex-col items-center justify-center text-center px-8">
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        className="flex flex-col items-center"
      >
        <div className="size-14 rounded-2xl bg-accent-soft border border-accent/20 flex items-center justify-center mb-5">
          <Zap size={24} className="text-accent" />
        </div>
        <p className="font-serif text-2xl text-tx-1 mb-2">Build your first agent</p>
        <p className="text-[13px] text-tx-3 max-w-xs leading-relaxed mb-6">
          Agents automate your workflows. Each agent can have multiple roles — 
          scheduled, triggered, or on-demand.
        </p>
        <button onClick={onNew} className="btn-primary flex items-center gap-2">
          <Zap size={14} />
          Create an agent
        </button>
      </motion.div>
    </div>
  );
}

// ── Main ───────────────────────────────────────────────────────────────────
export default function ChatPage({ onNavigate }) {
  const [agents,          setAgents]          = useState([]);
  const [selectedAgentId, setSelectedAgentId] = useState(null);
  const [loading,         setLoading]         = useState(true);
  const [pendingReviews,  setPendingReviews]  = useState([]);

  // Plan mode state
  // 'new'       → creating a brand new agent (can't dismiss)
  // 'add_role'  → adding role to existing agent (can dismiss)
  // null        → not in plan mode
  const [planModeFor, setPlanModeFor] = useState(null); // null | 'new' | 'add_role'

  const pollRef = useRef(null);

  // ── Load agents ──────────────────────────────────────────────────────
  const loadAgents = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      // GET /agent-definitions now embeds roles in each agent — one request, no N+1
      const res = await agentDefsApi.list();
      const withRoles = res.agents || [];
      setAgents(withRoles);
      if (!selectedAgentId && withRoles.length > 0) {
        setSelectedAgentId(withRoles[0].id);
      }
    } catch {}
    finally { if (!silent) setLoading(false); }
  }, [selectedAgentId]);

  useEffect(() => {
    loadAgents();
    const poll = () => {
      reviewsApi.list()
        .then(r => setPendingReviews((r.reviews || []).filter(rv => rv.status === 'pending')))
        .catch(() => {});
      swarm.status().catch(() => {});
    };
    poll();
    pollRef.current = setInterval(() => { loadAgents(true); poll(); }, 15000);
    return () => clearInterval(pollRef.current);
  }, []);

  // ── New agent ────────────────────────────────────────────────────────
  function handleNewAgent() {
    setPlanModeFor('new');
  }

  // Called when plan mode completes (either new agent or add role)
  function handlePlanModeComplete({ agentId, roleId }) {
    setPlanModeFor(null);
    setSelectedAgentId(agentId);
    loadAgents(true);
  }

  // Called when "Add Role" is triggered from AgentPage — we open plan mode
  // with existingAgentId so it can be dismissed
  // (AgentPage handles its own PlanModeChat, so nothing needed here)

  // ── Logout ───────────────────────────────────────────────────────────
  function handleNavigate(dest) {
    onNavigate(dest); // App.jsx handles logout, settings, etc.
  }

  const selectedAgent = agents.find(a => a.id === selectedAgentId) || null;

  return (
    <div className="flex h-screen bg-bg overflow-hidden">

      {/* ── Sidebar ─────────────────────────────────────────────── */}
      <Sidebar
        agents={agents}
        selectedAgentId={selectedAgentId}
        onSelectAgent={id => setSelectedAgentId(id)}
        onNewAgent={handleNewAgent}
        onNavigate={handleNavigate}
        pendingReviews={pendingReviews}
        loading={loading}
      />

      {/* ── Main area ────────────────────────────────────────────── */}
      <main className="flex flex-col flex-1 min-w-0">

        {/* Topbar */}
        <div className="flex items-center justify-between px-6 py-3 border-b border-border bg-bg-card/80 backdrop-blur shrink-0">
          {selectedAgent ? (
            <div className="flex items-center gap-2 min-w-0">
              <Cpu size={14} className="text-accent shrink-0" />
              <p className="text-sm font-semibold text-tx-1 truncate">{selectedAgent.name}</p>
              <span className="text-[10px] text-tx-4 shrink-0">
                {(selectedAgent.roles || []).length} role{(selectedAgent.roles || []).length !== 1 ? 's' : ''}
              </span>
            </div>
          ) : (
            <p className="text-sm text-tx-3">Agents</p>
          )}
        </div>

        {/* Content */}
        <div className="flex-1 flex flex-col min-h-0 overflow-y-auto">
          {loading ? null : selectedAgentId ? (
            <AgentPage
              agentId={selectedAgentId}
              onBack={() => setSelectedAgentId(null)}
            />
          ) : (
            <div className="flex-1 flex flex-col gap-4 p-6 max-w-2xl mx-auto w-full">
              <SavingsCard />
              <EmptyState onNew={handleNewAgent} />
            </div>
          )}
        </div>
      </main>

      {/* ── Plan mode overlay (new agent only — can't dismiss) ─── */}
      {planModeFor === 'new' && (
        <PlanModeChat
          agentName="New Agent"
          existingAgentId={null}
          onComplete={handlePlanModeComplete}
          onCancel={null} // null = no X button, can't exit
        />
      )}
    </div>
  );
}
