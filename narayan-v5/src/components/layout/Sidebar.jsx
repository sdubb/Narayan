import { useState, useEffect, useCallback } from 'react';
import clsx from 'clsx';
import { Settings, LogOut, Bell, Plus, GitBranch, Loader2 } from 'lucide-react';
import AgentListItem from '../sidebar/AgentListItem';
import SkillAutocomplete from '../sidebar/SkillAutocomplete';
import { conversations as conversationsApi } from '../../api';

function dateGroup(iso) {
  const d = new Date(iso);
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today); yesterday.setDate(yesterday.getDate() - 1);
  if (d >= today) return 'Today';
  if (d >= yesterday) return 'Yesterday';
  return d.toLocaleDateString('en', { month: 'short', day: 'numeric' });
}

export default function Sidebar({
  conversations, selectedId, onSelect, onNewConversation,
  onNavigate, pendingReviews = [], swarmDepth, convLatestStatus = {},
  loading, skills = [], onRefresh,
}) {
  const [goalInput, setGoalInput] = useState('');
  const [convAgents, setConvAgents] = useState({}); // { convId: [agent, ...] }

  // Fetch agents for all conversations to show in expandable list
  const fetchConvAgents = useCallback(async () => {
    if (!conversations?.length) return;
    const results = {};
    // Only fetch for recent conversations (limit to 10 to avoid spamming)
    const recent = conversations.slice(0, 10);
    await Promise.all(
      recent.map(conv =>
        conversationsApi.get(conv.id)
          .then(data => { results[conv.id] = data.agents || []; })
          .catch(() => { results[conv.id] = []; })
      )
    );
    setConvAgents(prev => ({ ...prev, ...results }));
  }, [conversations]);

  useEffect(() => {
    fetchConvAgents();
    // Refresh every 8 seconds to catch status changes
    const iv = setInterval(fetchConvAgents, 8000);
    return () => clearInterval(iv);
  }, [fetchConvAgents]);

  function handleAgentCancelled(agentId) {
    // Optimistically remove from active status and refresh
    setConvAgents(prev => {
      const updated = { ...prev };
      for (const convId of Object.keys(updated)) {
        updated[convId] = updated[convId].map(a =>
          a.id === agentId ? { ...a, status: 'failed' } : a
        );
      }
      return updated;
    });
    // Trigger a refresh from parent after a short delay
    setTimeout(() => {
      fetchConvAgents();
      onRefresh?.();
    }, 500);
  }

  const grouped = {};
  (conversations || []).forEach(conv => {
    const g = dateGroup(conv.updated_at || conv.created_at);
    if (!grouped[g]) grouped[g] = [];
    grouped[g].push(conv);
  });

  return (
    <aside className="w-64 flex flex-col border-r border-border bg-bg-card shrink-0 h-screen">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-4 border-b border-border">
        <p className="font-serif text-xl text-tx-1">Narayan</p>
        <div className="flex items-center gap-0.5">
          {pendingReviews.length > 0 && (
            <button onClick={() => onNavigate('settings')}
              className="relative p-1.5 rounded-lg text-warn hover:bg-warn-soft transition-all" title={`${pendingReviews.length} pending`}>
              <Bell size={15} />
              <span className="absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] rounded-full bg-warn text-bg-card text-[9px] font-bold flex items-center justify-center px-0.5">
                {pendingReviews.length}
              </span>
            </button>
          )}
          <button onClick={() => onNavigate('settings')} className="p-1.5 rounded-lg text-tx-3 hover:text-tx-1 hover:bg-bg-hover transition-all" title="Settings">
            <Settings size={15} />
          </button>
          <button onClick={() => onNavigate('logout')} className="p-1.5 rounded-lg text-tx-3 hover:text-err hover:bg-err-soft transition-all" title="Sign out">
            <LogOut size={15} />
          </button>
        </div>
      </div>

      {/* Goal input with skill autocomplete */}
      <div className="px-3 pt-3 pb-1">
        <div className="relative">
          <input
            value={goalInput}
            onChange={e => setGoalInput(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter' && goalInput.trim()) { onNewConversation(goalInput.trim()); setGoalInput(''); } }}
            placeholder="New goal..."
            className="input-field text-xs pr-8"
          />
          <button
            onClick={() => { if (goalInput.trim()) { onNewConversation(goalInput.trim()); setGoalInput(''); } }}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-tx-4 hover:text-accent transition-colors"
          >
            <Plus size={14} />
          </button>
          <SkillAutocomplete value={goalInput} skills={skills} onSelect={s => { setGoalInput(s); }} />
        </div>
      </div>

      {/* Conversation list */}
      <div className="flex-1 overflow-y-auto px-2 py-1 space-y-0.5">
        {loading ? (
          <div className="flex items-center justify-center py-8">
            <Loader2 size={16} className="text-tx-4 animate-spin" />
          </div>
        ) : Object.keys(grouped).length === 0 ? (
          <div className="px-3 py-8 text-center">
            <p className="text-xs text-tx-3">No conversations yet.</p>
            <p className="text-[11px] text-tx-4 mt-1">Type a goal above to start.</p>
          </div>
        ) : (
          Object.entries(grouped).map(([label, convs]) => (
            <div key={label}>
              <p className="section-label px-2 pt-3 pb-1">{label}</p>
              {convs.map(conv => (
                <AgentListItem
                  key={conv.id}
                  conversation={conv}
                  selected={conv.id === selectedId}
                  latestStatus={convLatestStatus[conv.id] || 'completed'}
                  onClick={() => onSelect(conv.id)}
                  agents={convAgents[conv.id] || []}
                  onAgentCancelled={handleAgentCancelled}
                />
              ))}
            </div>
          ))
        )}
      </div>

      {/* Footer */}
      <div className="p-2 border-t border-border space-y-1">
        <button onClick={() => { onSelect(null); }}
          className="w-full flex items-center gap-2 rounded-lg px-3 py-2 text-xs text-tx-3 hover:text-tx-1 hover:bg-bg-hover transition-all">
          <Plus size={13} /> New conversation
        </button>
        {swarmDepth != null && swarmDepth > 0 && (
          <div className="flex items-center gap-2 rounded-lg px-3 py-1.5 text-[11px] text-tx-4">
            <GitBranch size={11} className="text-vio shrink-0" />
            <span className="text-vio font-mono">{swarmDepth}</span>
            <span>sub-agent{swarmDepth !== 1 ? 's' : ''} queued</span>
          </div>
        )}
      </div>
    </aside>
  );
}
