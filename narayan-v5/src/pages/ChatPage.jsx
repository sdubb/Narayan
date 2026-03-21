import { useState, useEffect, useRef, useCallback } from 'react';
import {
  Send, Paperclip, X, Loader2, Bot,
  CheckCircle2, AlertCircle, AlertTriangle,
} from 'lucide-react';
import { agents, conversations as conversationsApi, reviews as reviewsApi, swarm } from '../api';
import clsx from 'clsx';
import { Sidebar } from '../components/layout';
import { AgentTimeline } from '../components/agent';
import { WorkspacePane } from '../components/agent';

// ── Status config ─────────────────────────────────────────────────────────
const STATUS = {
  pending:    { dot: 'bg-tx-4',  label: 'Pending',    spin: false },
  preflight:  { dot: 'bg-info',  label: 'Preflight',  spin: false },
  clarifying: { dot: 'bg-warn',  label: 'Clarifying', spin: false },
  running:    { dot: 'bg-ok',    label: 'Running',     spin: true  },
  waiting:    { dot: 'bg-info',  label: 'Scheduled',  spin: false },
  delegating: { dot: 'bg-vio',   label: 'Delegating', spin: true  },
  paused:     { dot: 'bg-warn',  label: 'Paused',     spin: false },
  completed:  { dot: 'bg-ok',    label: 'Done',       spin: false },
  failed:     { dot: 'bg-err',   label: 'Failed',     spin: false },
};

const TERMINAL = new Set(['completed', 'failed']);

function timeAgo(iso) {
  const d = Date.now() - new Date(iso).getTime();
  const h = Math.floor(d / 3600000), m = Math.floor((d % 3600000) / 60000);
  if (h > 0) return `${h}h ago`;
  if (m > 0) return `${m}m ago`;
  return 'just now';
}

// ── Image chip ────────────────────────────────────────────────────────────
function ImageChip({ file, onRemove }) {
  const [url] = useState(() => URL.createObjectURL(file));
  return (
    <div className="relative group size-12 rounded-lg overflow-hidden border border-border shrink-0">
      <img src={url} alt={file.name} className="size-full object-cover" />
      <button onClick={onRemove}
        className="absolute inset-0 bg-tx-1/60 opacity-0 group-hover:opacity-100 flex items-center justify-center transition-opacity">
        <X size={13} className="text-white" />
      </button>
    </div>
  );
}

// ── Conversation Thread ───────────────────────────────────────────────────
// Shows all agents in a conversation as message pairs
function ConversationThread({ convId, agentStatuses, terminalEvents, onStatusChange, onTerminal, onNavigateSettings }) {
  const [convAgents, setConvAgents] = useState([]);
  const bottomRef = useRef(null);

  useEffect(() => {
    if (!convId) return;
    let cancelled = false;
    const refresh = () => {
      conversationsApi.get(convId).then(data => {
        if (!cancelled) setConvAgents(data.agents || []);
      }).catch(() => {});
    };
    refresh();
    const iv = setInterval(refresh, 3000);
    return () => { cancelled = true; clearInterval(iv); };
  }, [convId]);

  useEffect(() => { bottomRef.current?.scrollIntoView({ behavior: 'smooth' }); }, [convAgents]);

  if (!convAgents.length) return (
    <div className="flex flex-col items-center justify-center h-full text-center px-8">
      <p className="font-serif text-2xl text-tx-1 mb-2">What should your agent do?</p>
      <p className="text-[13px] text-tx-3 max-w-xs leading-relaxed">
        Send a message to start the conversation.
      </p>
    </div>
  );

  return (
    <div className="px-6 py-4 space-y-6">
      {convAgents.map((agent, idx) => {
        const status = agentStatuses[agent.id] || agent.status;
        const isTerminal = TERMINAL.has(status);
        const terminalEvent = terminalEvents[agent.id];
        const isLast = idx === convAgents.length - 1;
        const cfg = STATUS[status] || STATUS.pending;
        return (
          <div key={agent.id} className="animate-in">
            {/* User message bubble */}
            <div className="flex justify-end mb-3">
              <div className="max-w-lg rounded-2xl rounded-br-md bg-tx-1 text-bg-card px-4 py-3">
                <p className="text-[13px] leading-relaxed whitespace-pre-wrap">{agent.goal}</p>
                <p className="text-[10px] opacity-60 mt-1">{timeAgo(agent.created_at)}</p>
              </div>
            </div>

            {/* Agent response */}
            <div className="flex justify-start">
              <div className="max-w-2xl w-full">
                <div className="flex items-center gap-2 mb-1.5">
                  <Bot size={14} className="text-accent shrink-0" />
                  <span className="text-[11px] font-semibold text-tx-3 uppercase tracking-wide">Narayan</span>
                  <span className="inline-flex items-center gap-1 text-[10px] text-tx-4">
                    <span className={clsx('size-1.5 rounded-full', cfg.dot, cfg.spin && 'animate-pulse')} />
                    {cfg.label}
                  </span>
                </div>

                {isTerminal ? (
                  <div className="rounded-2xl rounded-bl-md border border-border bg-bg-card overflow-hidden px-4 py-2">
                    <AgentTimeline
                      agentId={agent.id}
                      initialStatus={status}
                      onStatusChange={s => onStatusChange(agent.id, s)}
                      onTerminal={ev => onTerminal(agent.id, ev)}
                      onNavigateSettings={onNavigateSettings}
                    />
                  </div>
                ) : isLast ? (
                  <div className="rounded-2xl rounded-bl-md border border-border bg-bg-card overflow-hidden px-4 py-2">
                    <AgentTimeline
                      agentId={agent.id}
                      initialStatus={status}
                      onStatusChange={s => onStatusChange(agent.id, s)}
                      onTerminal={ev => onTerminal(agent.id, ev)}
                      onNavigateSettings={onNavigateSettings}
                    />
                  </div>
                ) : (
                  <div className="rounded-2xl rounded-bl-md border border-border bg-bg-card p-4">
                    <p className="text-[13px] text-tx-3">{agent.final_answer || 'Processing...'}</p>
                  </div>
                )}
              </div>
            </div>
          </div>
        );
      })}
      <div ref={bottomRef} />
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── MAIN CHAT PAGE ──────────────────────────────────────
// ═══════════════════════════════════════════════════════════
export default function ChatPage({ onNavigate }) {
  const [convList, setConvList]                   = useState([]);
  const [selectedConvId, setSelectedConvId]       = useState(null);
  const [input, setInput]                         = useState('');
  const [images, setImages]                       = useState([]);
  const [sending, setSending]                     = useState(false);
  const [loading, setLoading]                     = useState(true);
  const [error, setError]                         = useState('');
  const [agentStatuses, setAgentStatuses]         = useState({});
  const [terminalEvents, setTerminalEvents]       = useState({});
  const [pendingReviews, setPendingReviews]       = useState([]);
  const [swarmDepth, setSwarmDepth]               = useState(null);
  const [convLatestStatus, setConvLatestStatus]   = useState({});
  const [showWorkspace, setShowWorkspace]         = useState(false);
  const fileRef     = useRef(null);
  const textareaRef = useRef(null);
  const pollRef     = useRef(null);

  // ── Data loading ──────────────────────────────────────────
  const loadConversations = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      const r = await conversationsApi.list();
      setConvList(r.conversations || []);
    } catch (e) { if (!silent) setError(e.message); }
    finally { if (!silent) setLoading(false); }
  }, []);

  useEffect(() => {
    loadConversations();
    const poll = () => {
      reviewsApi.list().then(r => setPendingReviews((r.reviews || []).filter(rv => rv.status === 'pending'))).catch(() => {});
      swarm.status().then(s => setSwarmDepth(s.queue_depth ?? null)).catch(() => {});
    };
    poll();
    pollRef.current = setInterval(() => { loadConversations(true); poll(); }, 12000);
    return () => clearInterval(pollRef.current);
  }, []);

  // Auto-select first conversation
  useEffect(() => {
    if (!selectedConvId && convList.length > 0) setSelectedConvId(convList[0].id);
  }, [convList]);

  // ── Callbacks ─────────────────────────────────────────────
  function onStatusChange(agentId, status) {
    setAgentStatuses(p => ({ ...p, [agentId]: status }));
  }
  function onTerminal(agentId, ev) {
    setTerminalEvents(p => ({ ...p, [agentId]: ev }));
    onStatusChange(agentId, ev.type === 'goal_complete' ? 'completed' : 'failed');
  }

  // ── Goal submit ───────────────────────────────────────────
  async function send() {
    if (!input.trim()) return;
    setSending(true); setError('');
    try {
      const imgs = await Promise.all(images.map(f => new Promise(res => {
        const r = new FileReader(); r.onload = () => res({ name: f.name, data: r.result }); r.readAsDataURL(f);
      })));
      const res = await agents.createGoal(input.trim(), imgs, selectedConvId);
      setInput(''); setImages([]);
      if (textareaRef.current) textareaRef.current.style.height = 'auto';
      if (!selectedConvId) setSelectedConvId(res.conversation_id);
      await loadConversations(true);
    } catch (e) {
      if (e.message.startsWith('PAYMENT_REQUIRED:')) {
        setError('PLAN_LIMIT');
      } else {
        setError(e.message);
      }
    } finally { setSending(false); }
  }

  const selectedConv = selectedConvId ? convList.find(c => c.id === selectedConvId) : null;

  // Get latest active agent ID for workspace pane
  const latestAgentId = selectedConv
    ? (convList.find(c => c.id === selectedConvId)?.latest_agent_id || null)
    : null;

  return (
    <div className="flex h-screen bg-bg overflow-hidden">

      {/* ── Sidebar ──────────────────────────────────────────── */}
      <Sidebar
        conversations={convList}
        selectedId={selectedConvId}
        onSelect={setSelectedConvId}
        onNewConversation={(goal) => {
          setSelectedConvId(null);
          setInput(goal || '');
          textareaRef.current?.focus();
        }}
        onNavigate={onNavigate}
        pendingReviews={pendingReviews}
        swarmDepth={swarmDepth}
        convLatestStatus={convLatestStatus}
        loading={loading}
      />

      {/* ── Main area ────────────────────────────────────────── */}
      <main className="flex flex-col flex-1 min-w-0">

        {/* Header */}
        {selectedConv ? (
          <div className="flex items-center justify-between px-6 py-3 border-b border-border bg-bg-card/80 backdrop-blur shrink-0">
            <div className="min-w-0">
              <p className="text-[13px] font-medium text-tx-1 truncate max-w-lg">{selectedConv.title || 'Conversation'}</p>
              <div className="flex items-center gap-2 mt-0.5 text-[11px] text-tx-4">
                <span>{selectedConv.agent_count || 0} message{(selectedConv.agent_count || 0) !== 1 ? 's' : ''}</span>
                <span>&middot;</span>
                <span>{timeAgo(selectedConv.updated_at)}</span>
              </div>
            </div>
          </div>
        ) : (
          <div className="px-6 py-3 border-b border-border bg-bg-card/80 shrink-0">
            <p className="text-[13px] text-tx-3">New conversation</p>
          </div>
        )}

        {/* Content */}
        <div className="flex-1 overflow-y-auto">
          {selectedConvId ? (
            <ConversationThread
              convId={selectedConvId}
              agentStatuses={agentStatuses}
              terminalEvents={terminalEvents}
              onStatusChange={onStatusChange}
              onTerminal={onTerminal}
              onNavigateSettings={() => onNavigate('settings')}
            />
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-center px-8">
              <p className="font-serif text-2xl text-tx-1 mb-2">What should your agent do?</p>
              <p className="text-[13px] text-tx-3 max-w-xs leading-relaxed">
                Describe a goal. Your agent will plan, execute, and report back &mdash; no human steps needed.
              </p>
            </div>
          )}
        </div>

        {/* Input bar */}
        <div className="border-t border-border bg-bg-card px-4 py-4 shrink-0">
          {error === 'PLAN_LIMIT' ? (
            <div className="flex items-center gap-3 rounded-xl bg-warn-soft border border-warn/25 px-4 py-3 mb-3 animate-fade">
              <AlertTriangle size={14} className="text-warn shrink-0" />
              <div className="flex-1 min-w-0">
                <p className="text-[13px] font-medium text-warn">Step limit reached</p>
                <p className="text-[12px] text-warn/80">Upgrade your plan or buy a credit top-up to keep running agents.</p>
              </div>
              <button onClick={() => onNavigate('settings')}
                className="shrink-0 rounded-lg bg-warn px-3 py-1.5 text-[12px] font-semibold text-bg-card hover:bg-warn/90 transition-all">
                Upgrade
              </button>
              <button onClick={() => setError('')} className="text-warn/60 hover:text-warn transition-colors shrink-0">
                <X size={13} />
              </button>
            </div>
          ) : error ? (
            <div className="flex items-center gap-2 rounded-lg bg-err-soft border border-err/20 px-3 py-2 mb-3 text-[13px] text-err animate-fade">
              <AlertCircle size={13} />{error}
              <button onClick={() => setError('')} className="ml-auto"><X size={13} /></button>
            </div>
          ) : null}

          {images.length > 0 && (
            <div className="flex items-center gap-2 mb-3">
              {images.map((f, i) => (
                <ImageChip key={i} file={f} onRemove={() => setImages(p => p.filter((_, j) => j !== i))} />
              ))}
            </div>
          )}

          <div className="flex items-end gap-2.5 rounded-xl border border-border bg-bg px-4 py-3 focus-within:border-border-md focus-within:ring-2 focus-within:ring-accent/10 transition-all">
            <button onClick={() => fileRef.current?.click()}
              className="p-1 rounded-md text-tx-4 hover:text-tx-2 hover:bg-bg-active transition-all shrink-0 mb-0.5" title="Attach image">
              <Paperclip size={16} />
            </button>
            <input ref={fileRef} type="file" accept="image/*" multiple className="hidden"
              onChange={e => setImages(p => [...p, ...Array.from(e.target.files)].slice(0, 5))} />
            <textarea ref={textareaRef} value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); } }}
              placeholder="Send a message..."
              rows={1}
              className="flex-1 bg-transparent text-[13px] text-tx-1 placeholder-tx-4 outline-none resize-none leading-relaxed max-h-32"
              style={{ overflow: input.split('\n').length > 4 ? 'auto' : 'hidden' }}
              onInput={e => { e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 128) + 'px'; }} />
            <button onClick={send} disabled={sending || !input.trim()}
              className={clsx('p-2 rounded-lg transition-all shrink-0 mb-0.5',
                input.trim() && !sending ? 'bg-tx-1 text-bg-card hover:bg-tx-2 active:scale-95' : 'bg-bg-active text-tx-4 cursor-not-allowed')}>
              {sending ? <Loader2 size={15} className="animate-spin" /> : <Send size={15} />}
            </button>
          </div>
          <p className="text-[11px] text-tx-4 mt-2 text-center">
            {selectedConvId ? 'Follow-up messages continue this conversation' : 'Start a new conversation'} &middot; Shift+Enter for newline
          </p>
        </div>
      </main>

      {/* ── Workspace pane (optional right panel) ─────────── */}
      {showWorkspace && latestAgentId && (
        <WorkspacePane agentId={latestAgentId} onClose={() => setShowWorkspace(false)} />
      )}
    </div>
  );
}
