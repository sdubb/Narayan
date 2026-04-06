import { useState, useEffect, useCallback, useRef } from 'react';
import { motion } from 'framer-motion';
import { Cpu, Zap, Sparkles } from 'lucide-react';
import { reviews as reviewsApi, swarm, agentDefs as agentDefsApi } from '../api';
import { Sidebar } from '../components/layout';
import PlanModeChat from '../components/agent/PlanModeChat';
import AgentPage from './AgentPage';
import SavingsCard from '../components/cards/SavingsCard';

function EmptyState({ onNew, canCreateAgents }) {
  const title = canCreateAgents ? 'Create your first agent' : 'Add an AI provider first';
  const body = canCreateAgents
    ? 'Describe the work, connect the tools, and ship with review built in. The workspace keeps the plan, trace, and follow-up attached.'
    : 'Connect one AI provider to unlock agent creation. We will keep you in the workspace and guide you back here once setup is complete.';
  const cta = canCreateAgents ? 'Create an agent' : 'Go to settings';

  return (
    <div className="flex flex-1 flex-col items-center justify-center px-8 text-center">
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        className="flex w-full max-w-2xl flex-col items-center rounded-[2rem] border border-border bg-bg-card/90 px-8 py-10 shadow-[0_24px_70px_rgba(26,23,20,0.08)] backdrop-blur"
      >
        <div className="mb-5 flex size-16 items-center justify-center rounded-[1.5rem] border border-accent/20 bg-accent-soft shadow-card">
          <Zap size={24} className="text-accent" />
        </div>
        <p className="mb-2 font-serif text-3xl text-tx-1">{title}</p>
        <p className="mb-6 max-w-xl text-sm leading-7 text-tx-3">{body}</p>
        <button onClick={onNew} className="btn-primary flex items-center gap-2 px-4 py-2.5">
          <Zap size={14} />
          {cta}
        </button>
        {!canCreateAgents && (
          <p className="mt-3 max-w-sm text-xs leading-6 text-tx-4">
            Once you add a provider, the compiler will be ready immediately.
          </p>
        )}
        <div className="mt-8 grid w-full max-w-md grid-cols-3 gap-3 text-left">
          {[
            ['Plan', 'Describe the workflow and the outcome'],
            ['Connect', 'Bring in the tools you already use'],
            ['Launch', 'Let the agent execute and log everything'],
          ].map(([title, text]) => (
            <div key={title} className="border-t border-border pt-3">
              <p className="text-sm font-medium text-tx-1">{title}</p>
              <p className="mt-1 text-xs leading-5 text-tx-3">{text}</p>
            </div>
          ))}
        </div>
      </motion.div>
    </div>
  );
}

export default function ChatPage({ onNavigate, canCreateAgents = true }) {
  const [agents, setAgents] = useState([]);
  const [selectedAgentId, setSelectedAgentId] = useState(null);
  const [loading, setLoading] = useState(true);
  const [pendingReviews, setPendingReviews] = useState([]);
  const [planModeFor, setPlanModeFor] = useState(null);
  const pollRef = useRef(null);

  const loadAgents = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      const res = await agentDefsApi.list();
      const withRoles = res.agents || [];
      setAgents(withRoles);
      if (!selectedAgentId && withRoles.length > 0) {
        setSelectedAgentId(withRoles[0].id);
      }
    } catch {}
    finally {
      if (!silent) setLoading(false);
    }
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
    pollRef.current = window.setInterval(() => {
      loadAgents(true);
      poll();
    }, 15000);
    return () => window.clearInterval(pollRef.current);
  }, []);

  function handleNewAgent() {
    if (!canCreateAgents) {
      onNavigate('settings');
      return;
    }
    setPlanModeFor('new');
  }

  function handlePlanModeComplete({ agentId }) {
    setPlanModeFor(null);
    setSelectedAgentId(agentId);
    loadAgents(true);
  }

  function handleNavigate(dest) {
    onNavigate(dest);
  }

  const selectedAgent = agents.find(a => a.id === selectedAgentId) || null;

  return (
    <div className="relative flex h-screen overflow-hidden bg-[radial-gradient(circle_at_top_left,_rgba(201,106,46,0.08),_transparent_24%),linear-gradient(180deg,_#f7f4ef_0%,_#f4efe8_100%)]">
        <Sidebar
          agents={agents}
          selectedAgentId={selectedAgentId}
          onSelectAgent={id => setSelectedAgentId(id)}
          onNewAgent={handleNewAgent}
          onNavigate={handleNavigate}
          pendingReviews={pendingReviews}
          loading={loading}
          canCreateAgents={canCreateAgents}
        />

      <main className="relative flex min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center justify-between border-b border-border bg-bg-card/85 px-6 py-3 backdrop-blur">
          {selectedAgent ? (
            <div className="flex min-w-0 items-center gap-2">
              <Cpu size={14} className="shrink-0 text-accent" />
              <p className="truncate text-sm font-semibold text-tx-1">{selectedAgent.name}</p>
              <span className="shrink-0 text-[10px] text-tx-4">
                {(selectedAgent.roles || []).length} role{(selectedAgent.roles || []).length !== 1 ? 's' : ''}
              </span>
              <span className="ml-2 rounded-full bg-ok-soft px-2 py-0.5 text-[10px] font-medium text-ok">
                Live
              </span>
            </div>
          ) : (
            <div>
              <p className="text-sm font-medium text-tx-1">Agents</p>
              <p className="text-[11px] text-tx-4">Build, review, and deploy operational workflows.</p>
            </div>
          )}
          <div className="hidden items-center gap-2 lg:flex">
            <div className="inline-flex items-center gap-2 rounded-full border border-border bg-bg px-3 py-1 text-xs text-tx-2">
              <Sparkles className="size-3.5 text-accent" />
              Working surface
            </div>
          </div>
        </div>

        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          {loading ? null : selectedAgentId ? (
            <AgentPage
              agentId={selectedAgentId}
              onBack={() => setSelectedAgentId(null)}
              onNavigateSettings={() => onNavigate('settings')}
            />
          ) : (
            <div className="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-5 px-6 py-8">
              <SavingsCard />
              <EmptyState onNew={handleNewAgent} canCreateAgents={canCreateAgents} />
            </div>
          )}
        </div>
      </main>

      {planModeFor === 'new' && (
        <div className="absolute inset-y-0 right-0 z-50 w-full max-w-[44rem]">
          <PlanModeChat
            agentName="New Agent"
            existingAgentId={null}
            onComplete={handlePlanModeComplete}
            onCancel={null}
            presentation="inline"
          />
        </div>
      )}
    </div>
  );
}
