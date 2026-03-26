import { useState, useEffect, useRef, useCallback } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import {
  Bot, User, Loader2, X, Sparkles, ArrowRight, Download,
  MessageSquare, Layers3, Activity, Users, AlertTriangle, FileText, Image, Code, File,
} from 'lucide-react';
import { agentDefs as agentDefsApi, agents as agentsApi, workspace as workspaceApi } from '../../api';

function humanize(value) {
  if (value == null || value === '') return 'None';
  return String(value).replace(/_/g, ' ');
}

function titleCase(value) {
  return humanize(value)
    .split(' ')
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function timeAgo(iso) {
  if (!iso) return 'just now';
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  const hours = Math.floor(mins / 60);
  const days = Math.floor(hours / 24);
  if (days > 0) return `${days}d ago`;
  if (hours > 0) return `${hours}h ago`;
  if (mins > 0) return `${mins}m ago`;
  return 'just now';
}

function parsePendingRoles(agent) {
  if (!agent?.memory_ref) return [];
  const match = agent.memory_ref.match(/\|pending_roles:(\[.*?\])/);
  if (!match) return [];
  try {
    return JSON.parse(match[1]);
  } catch {
    return [];
  }
}

function runStatusTone(status) {
  const value = String(status || '').toLowerCase();
  if (value === 'completed') return 'bg-ok-soft text-ok border-ok/20';
  if (value === 'running' || value === 'pending') return 'bg-info-soft text-info border-info/20';
  if (value === 'failed' || value === 'partially_complete') return 'bg-err-soft text-err border-err/20';
  if (value === 'cancelled') return 'bg-bg-active text-tx-4 border-border';
  return 'bg-bg-active text-tx-3 border-border';
}

function roleStatusTone(status) {
  const value = String(status || '').toLowerCase();
  if (value === 'active') return 'bg-ok-soft text-ok border-ok/20';
  if (value === 'testing') return 'bg-info-soft text-info border-info/20';
  if (value === 'draft') return 'bg-bg-active text-tx-3 border-border';
  if (value === 'paused') return 'bg-warn-soft text-warn border-warn/20';
  return 'bg-bg-active text-tx-3 border-border';
}

function statTone(tone) {
  return {
    neutral: 'bg-bg-card border-border',
    accent: 'bg-accent-soft/20 border-accent/20',
    ok: 'bg-ok-soft/20 border-ok/20',
    warn: 'bg-warn-soft/20 border-warn/20',
  }[tone] || 'bg-bg-card border-border';
}

function formatSize(bytes) {
  if (!bytes) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function fileIcon(name) {
  const ext = name.split('.').pop()?.toLowerCase();
  if (ext === 'md' || ext === 'txt' || ext === 'pdf' || ext === 'doc' || ext === 'docx') return FileText;
  if (ext === 'csv' || ext === 'json' || ext === 'yaml' || ext === 'yml' || ext === 'xml') return Code;
  if (ext === 'png' || ext === 'jpg' || ext === 'jpeg' || ext === 'gif' || ext === 'svg') return Image;
  return File;
}

function fileKindLabel(name) {
  const ext = name.split('.').pop()?.toLowerCase();
  if (!ext) return 'file';
  if (['csv', 'xlsx', 'xls', 'ods'].includes(ext)) return 'spreadsheet';
  if (['pdf'].includes(ext)) return 'pdf';
  if (['doc', 'docx', 'md', 'txt', 'rtf'].includes(ext)) return 'document';
  if (['json', 'xml', 'yaml', 'yml'].includes(ext)) return 'data';
  if (['png', 'jpg', 'jpeg', 'gif', 'svg'].includes(ext)) return 'image';
  return ext;
}

function MetricCard({ label, value, note, tone = 'neutral' }) {
  return (
    <div className={clsx('rounded-xl border px-3 py-3', statTone(tone))}>
      <p className="text-[10px] uppercase tracking-wide text-tx-4">{label}</p>
      <div className="mt-1 flex items-end justify-between gap-2">
        <p className="text-base font-semibold text-tx-1 truncate">{value}</p>
      </div>
      {note ? <p className="mt-1 text-[11px] text-tx-3 leading-relaxed">{note}</p> : null}
    </div>
  );
}

function SectionCard({ title, subtitle, icon: Icon, action, children }) {
  return (
    <div className="rounded-2xl border border-border bg-bg-card overflow-hidden">
      <div className="flex items-start justify-between gap-3 px-3.5 py-3 border-b border-border/70">
        <div className="flex items-start gap-2.5 min-w-0">
          {Icon ? (
            <div className="size-7 rounded-lg bg-accent-soft border border-accent/20 flex items-center justify-center shrink-0">
              <Icon size={14} className="text-accent" />
            </div>
          ) : null}
          <div className="min-w-0">
            <p className="text-sm font-semibold text-tx-1">{title}</p>
            {subtitle ? <p className="text-[11px] text-tx-4 mt-0.5 leading-relaxed">{subtitle}</p> : null}
          </div>
        </div>
        {action}
      </div>
      <div className="p-3.5">
        {children}
      </div>
    </div>
  );
}

function Bubble({ role: msgRole, content, isNew }) {
  const isUser = msgRole === 'user';
  return (
    <motion.div
      className={clsx('flex gap-2.5', isUser ? 'flex-row-reverse' : 'flex-row')}
      initial={isNew ? { opacity: 0, y: 6 } : false}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.15 }}
    >
      <div className={clsx(
        'size-6 rounded-full flex items-center justify-center shrink-0 mt-0.5',
        isUser ? 'bg-tx-1' : 'bg-accent-soft border border-accent/20',
      )}>
        {isUser ? <User size={11} className="text-bg-card" /> : <Bot size={11} className="text-accent" />}
      </div>
      <div className={clsx(
        'max-w-md rounded-2xl px-3.5 py-2.5 text-[13px] leading-relaxed whitespace-pre-wrap',
        isUser ? 'bg-tx-1 text-bg-card rounded-tr-sm' : 'bg-bg-card border border-border text-tx-1 rounded-tl-sm',
      )}>
        {content}
      </div>
    </motion.div>
  );
}

function TypingDots() {
  return (
    <div className="flex gap-2.5">
      <div className="size-6 rounded-full bg-accent-soft border border-accent/20 flex items-center justify-center shrink-0">
        <Bot size={11} className="text-accent" />
      </div>
      <div className="bg-bg-card border border-border rounded-2xl rounded-tl-sm px-3.5 py-2.5 flex items-center gap-1">
        {[0, 1, 2].map(i => (
          <span
            key={i}
            className="size-1.5 rounded-full bg-tx-4 inline-block animate-pulse"
            style={{ animationDelay: `${i * 0.15}s` }}
          />
        ))}
      </div>
    </div>
  );
}

function normalizeTrigger(triggerSource) {
  if (!triggerSource) return 'unknown';
  switch (triggerSource.source) {
    case 'webhook':
      return `${triggerSource.connector || 'webhook'} / ${triggerSource.event_type || 'event'}`;
    case 'schedule':
      return triggerSource.cron || 'schedule';
    case 'user_message':
      return 'user message';
    case 'manual':
      return 'manual';
    case 'workforce_event':
      return `workforce event from ${triggerSource.source_role_name || 'another role'}`;
    default:
      return triggerSource.source || 'unknown';
  }
}

function runDetailLabel(run, roleName) {
  const parts = [
    roleName || run.role_id,
    titleCase(run.status),
    normalizeTrigger(run.trigger_source),
  ];
  return parts.filter(Boolean).join(' | ');
}

function flattenWorkspaceFiles(items, out = []) {
  for (const item of items || []) {
    const isDir = Boolean(item?.isDir ?? item?.is_dir);
    if (isDir) {
      flattenWorkspaceFiles(item.children || [], out);
    } else if (item?.path) {
      out.push(item);
    }
  }
  return out;
}

function prioritizeArtifact(a, b) {
  const score = (name) => {
    const ext = name.split('.').pop()?.toLowerCase();
    if (ext === 'pdf') return 0;
    if (['doc', 'docx', 'md', 'txt'].includes(ext)) return 1;
    if (['csv', 'xlsx', 'xls', 'ods'].includes(ext)) return 2;
    if (['json', 'xml', 'yaml', 'yml'].includes(ext)) return 3;
    return 4;
  };
  const diff = score(a.name) - score(b.name);
  if (diff !== 0) return diff;
  return String(b.modified || '').localeCompare(String(a.modified || ''));
}

export default function AgentChatDrawer({
  agentId,
  agentName,
  agent = null,
  roles: initialRoles = [],
  onClose,
}) {
  const [messages, setMessages] = useState([
    {
      role: 'assistant',
      content:
        `This is the control center for ${agentName}.\n\nI can summarize the agent, inspect its roles and recent runs, flag blockers, and compare it with other agents in the tenant.`,
      isNew: false,
    },
  ]);
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState('');
  const [contextLoading, setContextLoading] = useState(true);
  const [contextError, setContextError] = useState('');
  const [agentState, setAgentState] = useState(agent);
  const [roles, setRoles] = useState(initialRoles || []);
  const [runs, setRuns] = useState([]);
  const [peerAgents, setPeerAgents] = useState([]);
  const [workspaceFiles, setWorkspaceFiles] = useState([]);
  const bottomRef = useRef(null);
  const inputRef = useRef(null);

  useEffect(() => {
    setAgentState(agent || null);
  }, [agent]);

  useEffect(() => {
    setRoles(initialRoles || []);
  }, [initialRoles]);

  useEffect(() => {
    setMessages([{
      role: 'assistant',
      content:
        `This is the control center for ${agentName}.\n\nI can summarize the agent, inspect its roles and recent runs, flag blockers, and compare it with other agents in the tenant.`,
      isNew: false,
    }]);
    setInput('');
    setError('');
    setSending(false);
  }, [agentId, agentName]);

  useEffect(() => {
    let cancelled = false;

    async function loadContext() {
      if (!agentId) return;
      setContextLoading(true);
      setContextError('');
      try {
        const [runsRes, peersRes, rolesRes, agentRes] = await Promise.all([
          agentDefsApi.listGoalInstances(agentId, 8).catch(() => ({ goal_instances: [] })),
          agentDefsApi.list().catch(() => ({ agents: [] })),
          initialRoles?.length ? Promise.resolve(null) : agentDefsApi.listRoles(agentId).catch(() => null),
          agentState ? Promise.resolve(null) : agentDefsApi.get(agentId).catch(() => null),
        ]);

        if (cancelled) return;
        setRuns(runsRes?.goal_instances || []);
        setPeerAgents(peersRes?.agents || []);
        if (rolesRes?.roles) setRoles(rolesRes.roles);
        if (agentRes) setAgentState(agentRes);
      } catch (e) {
        if (!cancelled) setContextError(e.message || 'Failed to load agent context');
      } finally {
        if (!cancelled) setContextLoading(false);
      }
    }

    loadContext();
    return () => { cancelled = true; };
  }, [agentId, agentState, initialRoles]);

  useEffect(() => {
    let cancelled = false;

    async function loadWorkspaceFiles() {
      if (!agentId) return;
      try {
        const data = await workspaceApi.tree(agentId).catch(() => ({ files: [] }));
        if (cancelled) return;
        const tree = data?.files || data?.tree || data || [];
        setWorkspaceFiles(tree);
      } catch {
        if (!cancelled) setWorkspaceFiles([]);
      }
    }

    loadWorkspaceFiles();
    return () => { cancelled = true; };
  }, [agentId]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, sending]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const send = useCallback(async (text) => {
    const trimmed = text.trim();
    if (!trimmed || sending || !agentId) return;

    setInput('');
    setError('');
    setSending(true);
    setMessages(prev => [...prev, { role: 'user', content: trimmed, isNew: true }]);

    try {
      const conversation = messages.map(({ role, content }) => ({ role, content }));
      const res = await agentDefsApi.chat(agentId, trimmed, conversation);
      setMessages(prev => [...prev, { role: 'assistant', content: res.reply, isNew: true }]);
    } catch (e) {
      setError(e.message || 'Something went wrong');
      setMessages(prev => prev.slice(0, -1));
    } finally {
      setSending(false);
    }
  }, [agentId, messages, sending]);

  function onKeyDown(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send(input);
    }
  }

  const agentData = agentState || agent || {};
  const pendingRoles = parsePendingRoles(agentData);
  const activeRoles = roles.filter(role => role.status === 'active');
  const activeRuns = runs.filter(run => ['running', 'pending'].includes(String(run.status)));
  const failedRuns = runs.filter(run => ['failed', 'partially_complete'].includes(String(run.status)));
  const recentRuns = [...runs].sort((a, b) => new Date(b.created_at) - new Date(a.created_at));
  const peers = peerAgents.filter(peer => peer.id !== agentId).slice(0, 6);
  const artifacts = flattenWorkspaceFiles(workspaceFiles)
    .sort(prioritizeArtifact)
    .slice(0, 8);
  const roleNameById = roles.reduce((acc, role) => {
    acc[role.id] = role.name;
    return acc;
  }, {});
  const controlWarnings = [];
  if (pendingRoles.length > 0) controlWarnings.push(`${pendingRoles.length} pending role${pendingRoles.length === 1 ? '' : 's'} still need setup`);
  if (failedRuns.length > 0) controlWarnings.push(`${failedRuns.length} recent run${failedRuns.length === 1 ? '' : 's'} failed or only partially completed`);
  if (contextError) controlWarnings.push(contextError);

  const quickActions = [
    'Summarize this agent',
    'What roles does it have?',
    'Show recent runs and blockers',
    'Compare it with other agents',
  ];

  async function downloadBlob(blob, filename) {
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  async function downloadArtifact(path, name) {
    if (!agentId) return;
    try {
      const blob = await workspaceApi.download(agentId, path);
      await downloadBlob(blob, name || path.split('/').pop() || 'artifact');
    } catch {}
  }

  async function downloadSummaryPdf() {
    if (!agentId) return;
    try {
      const blob = await agentDefsApi.exportSummaryPdf(agentId);
      await downloadBlob(blob, `${(agentData.name || agentName).replace(/[^a-z0-9_-]+/gi, '-').replace(/-+/g, '-').replace(/^-|-$/g, '') || 'agent'}-summary.pdf`);
    } catch {}
  }

  async function downloadArtifactsBundle() {
    if (!agentId) return;
    try {
      const blob = await workspaceApi.bundle(agentId);
      const safeName = (agentData.name || agentName).replace(/[^a-z0-9_-]+/gi, '-').replace(/-+/g, '-').replace(/^-|-$/g, '') || 'agent';
      await downloadBlob(blob, `${safeName}-workspace-files.tar.zst`);
    } catch {}
  }

  return (
    <motion.div
      className="fixed inset-0 z-50 flex justify-end"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
    >
      <div className="absolute inset-0 bg-tx-1/20 backdrop-blur-[2px]" onClick={onClose} />
      <motion.div
        className="relative w-full max-w-lg h-full flex flex-col bg-bg border-l border-border shadow-xl"
        initial={{ x: '100%' }}
        animate={{ x: 0 }}
        exit={{ x: '100%' }}
        transition={{ type: 'spring', damping: 28, stiffness: 260 }}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-border bg-bg-card">
          <div className="flex items-center gap-2">
            <div className="size-8 rounded-lg bg-accent-soft border border-accent/20 flex items-center justify-center">
              <Sparkles size={15} className="text-accent" />
            </div>
            <div>
              <p className="text-sm font-semibold text-tx-1">Agent control center</p>
              <p className="text-[11px] text-tx-4 truncate">{agentData.name || agentName}</p>
            </div>
          </div>
          <button onClick={onClose} className="p-1.5 rounded-lg text-tx-4 hover:text-tx-1 hover:bg-bg-hover transition-colors">
            <X size={15} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          <SectionCard
            title="At a glance"
            subtitle="Status, workload, and workspace context."
            icon={Layers3}
          >
            {contextLoading ? (
              <div className="flex items-center justify-center py-4">
                <Loader2 size={18} className="text-tx-4 animate-spin" />
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-2.5">
                <MetricCard
                  label="State"
                  value={titleCase(agentData.status || 'draft')}
                  note={agentData.persona ? agentData.persona.slice(0, 90) : 'No persona defined yet.'}
                  tone={String(agentData.status || '').toLowerCase() === 'active' ? 'ok' : 'neutral'}
                />
                <MetricCard
                  label="Roles"
                  value={`${roles.length}`}
                  note={`${activeRoles.length} active, ${pendingRoles.length} pending`}
                  tone="accent"
                />
                <MetricCard
                  label="Runs"
                  value={`${recentRuns.length}`}
                  note={`${activeRuns.length} running or waiting, ${failedRuns.length} needing review`}
                  tone={failedRuns.length > 0 ? 'warn' : 'neutral'}
                />
                <MetricCard
                  label="Peers"
                  value={`${peers.length}`}
                  note={peers.length > 0 ? 'Other agents in this tenant' : 'No other agents found'}
                  tone="neutral"
                />
              </div>
            )}
            {controlWarnings.length > 0 && (
              <div className="mt-3 rounded-xl border border-warn/20 bg-warn-soft/20 px-3 py-2 text-[11px] text-warn space-y-1">
                {controlWarnings.map((warning, index) => (
                  <div key={`${warning}-${index}`} className="flex items-start gap-2">
                    <AlertTriangle size={11} className="mt-0.5 shrink-0" />
                    <span className="leading-relaxed">{warning}</span>
                  </div>
                ))}
              </div>
            )}
          </SectionCard>

          <SectionCard
            title="Agent snapshot"
            subtitle="Identity, connectors, constraints, and pending setup."
            icon={Sparkles}
          >
            <div className="space-y-3">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <p className="text-sm font-semibold text-tx-1 truncate">{agentData.name || agentName}</p>
                  <p className="text-[11px] text-tx-3 leading-relaxed mt-1">
                    {agentData.persona || 'No persona set for this agent yet.'}
                  </p>
                </div>
                <span className={clsx('shrink-0 inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold border', roleStatusTone(agentData.status))}>
                  {titleCase(agentData.status || 'draft')}
                </span>
              </div>

              {agentData.connectors?.length > 0 && (
                <div className="space-y-1">
                  <p className="text-[10px] uppercase tracking-wide text-tx-4">Connectors</p>
                  <div className="flex flex-wrap gap-1.5">
                    {agentData.connectors.map(connector => (
                      <span key={connector} className="text-[10px] bg-accent-soft text-accent border border-accent/20 px-1.5 py-0.5 rounded">
                        {connector}
                      </span>
                    ))}
                  </div>
                </div>
              )}

              {agentData.constraints?.length > 0 && (
                <div className="space-y-1">
                  <p className="text-[10px] uppercase tracking-wide text-tx-4">Constraints</p>
                  <div className="flex flex-wrap gap-1.5">
                    {agentData.constraints.map((constraint, index) => (
                      <span key={`${constraint}-${index}`} className="text-[10px] bg-warn-soft text-warn border border-warn/20 px-1.5 py-0.5 rounded">
                        {constraint}
                      </span>
                    ))}
                  </div>
                </div>
              )}

              {pendingRoles.length > 0 && (
                <div className="rounded-xl border border-info/20 bg-info-soft/20 px-3 py-2 text-[11px] text-info">
                  {pendingRoles.length} role{pendingRoles.length === 1 ? '' : 's'} remain in plan mode continuation.
                </div>
              )}
            </div>
          </SectionCard>

          <SectionCard
            title="Roles"
            subtitle="What this agent can do and how each role is triggered."
            icon={Layers3}
          >
            {roles.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border bg-bg px-3 py-4 text-xs text-tx-4">
                No roles are configured yet.
              </div>
            ) : (
              <div className="space-y-2">
                {roles.map(role => (
                  <div key={role.id} className="rounded-xl border border-border bg-bg px-3 py-2.5">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <p className="text-sm font-semibold text-tx-1 truncate">{role.name}</p>
                          <span className={clsx('inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold border', roleStatusTone(role.status))}>
                            {titleCase(role.status)}
                          </span>
                        </div>
                        <p className="text-[11px] text-tx-3 mt-1 leading-relaxed">
                          {role.purpose || 'No purpose written yet.'}
                        </p>
                      </div>
                      <span className="text-[10px] bg-bg-active text-tx-3 border border-border px-2 py-0.5 rounded whitespace-nowrap">
                        {titleCase(role.trigger?.trigger_type || 'manual')}
                      </span>
                    </div>
                    <div className="mt-2 flex flex-wrap gap-1.5">
                      {role.connectors?.length > 0 ? role.connectors.map(connector => (
                        <span key={connector} className="text-[10px] bg-accent-soft text-accent border border-accent/20 px-1.5 py-0.5 rounded">
                          {connector}
                        </span>
                      )) : (
                        <span className="text-[10px] text-tx-4">No connectors</span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </SectionCard>

          <SectionCard
            title="Recent runs"
            subtitle="Latest goal instances, errors, and throughput."
            icon={Activity}
          >
            {recentRuns.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border bg-bg px-3 py-4 text-xs text-tx-4">
                No runs yet.
              </div>
            ) : (
              <div className="space-y-2">
                {recentRuns.map(run => {
                  const roleName = roleNameById[run.role_id] || run.role_id;
                  return (
                    <div key={run.id} className="rounded-xl border border-border bg-bg px-3 py-2.5">
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <span className={clsx('inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold border capitalize', runStatusTone(run.status))}>
                              {titleCase(run.status)}
                            </span>
                            <p className="text-sm font-semibold text-tx-1 truncate">{roleName}</p>
                          </div>
                          <p className="text-[11px] text-tx-3 mt-1 leading-relaxed">
                            {runDetailLabel(run, roleName)}
                          </p>
                        </div>
                        <div className="text-right shrink-0">
                          <p className="text-[10px] text-tx-4">{timeAgo(run.created_at)}</p>
                          {run.cost_usd > 0 && (
                            <p className="text-[10px] text-tx-3 mt-1">${run.cost_usd.toFixed(4)}</p>
                          )}
                        </div>
                      </div>
                      {run.failure_reason && (
                        <div className="mt-2 rounded-lg border border-err/20 bg-err-soft/20 px-2.5 py-2 text-[11px] text-err leading-relaxed">
                          {run.failure_reason}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </SectionCard>

          <SectionCard
            title="Artifacts"
            subtitle="Downloads created by the workflow. Bundle export uses tar.zst for the smallest transfer."
            icon={Download}
            action={
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={downloadSummaryPdf}
                  className="text-[11px] px-2.5 py-1.5 rounded-full border border-accent/20 bg-accent-soft/20 text-accent hover:bg-accent-soft/30 transition-colors"
                >
                  Export summary PDF
                </button>
                <button
                  type="button"
                  onClick={downloadArtifactsBundle}
                  className="text-[11px] px-2.5 py-1.5 rounded-full border border-border bg-bg-card text-tx-3 hover:text-accent hover:border-accent/40 hover:bg-accent-soft/20 transition-colors"
                >
                  Download bundle
                </button>
              </div>
            }
          >
            {artifacts.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border bg-bg px-3 py-4 text-xs text-tx-4">
                No downloadable artifacts yet.
              </div>
            ) : (
              <div className="space-y-2">
                {artifacts.map(file => {
                  const Icon = fileIcon(file.name);
                  return (
                    <div key={file.path} className="rounded-xl border border-border bg-bg px-3 py-2.5">
                      <div className="flex items-start justify-between gap-3">
                        <div className="flex items-start gap-2.5 min-w-0">
                          <div className="size-8 rounded-lg bg-bg-card border border-border flex items-center justify-center shrink-0">
                            <Icon size={14} className="text-accent" />
                          </div>
                          <div className="min-w-0">
                            <div className="flex items-center gap-2 flex-wrap">
                              <p className="text-sm font-semibold text-tx-1 truncate">{file.name}</p>
                              <span className="text-[10px] px-1.5 py-0.5 rounded border border-border bg-bg-card text-tx-3 uppercase">
                                {fileKindLabel(file.name)}
                              </span>
                            </div>
                            <p className="text-[11px] text-tx-3 mt-1 leading-relaxed">
                              {formatSize(file.size)}{file.modified ? ` | ${timeAgo(file.modified)}` : ''}
                            </p>
                          </div>
                        </div>
                        <button
                          type="button"
                          onClick={() => downloadArtifact(file.path, file.name)}
                          className="shrink-0 inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border border-border bg-bg-card text-[11px] text-tx-3 hover:text-accent hover:border-accent/40 hover:bg-accent-soft/20 transition-colors"
                        >
                          <Download size={12} />
                          Download
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </SectionCard>

          <SectionCard
            title="Other agents"
            subtitle="Nearby agents in the same tenant."
            icon={Users}
          >
            {peers.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border bg-bg px-3 py-4 text-xs text-tx-4">
                No other agents available.
              </div>
            ) : (
              <div className="space-y-2">
                {peers.map(peer => (
                  <div key={peer.id} className="rounded-xl border border-border bg-bg px-3 py-2.5">
                    <div className="flex items-center justify-between gap-3">
                      <div className="min-w-0">
                        <p className="text-sm font-semibold text-tx-1 truncate">{peer.name}</p>
                        <p className="text-[11px] text-tx-3 mt-1">
                          {peer.persona ? peer.persona : 'No persona provided'}
                        </p>
                      </div>
                      <div className="text-right shrink-0">
                        <span className={clsx('inline-flex items-center px-2 py-0.5 rounded text-[10px] font-semibold border', roleStatusTone(peer.status))}>
                          {titleCase(peer.status || 'draft')}
                        </span>
                        <p className="text-[10px] text-tx-4 mt-1">
                          {Array.isArray(peer.roles) ? `${peer.roles.length} role${peer.roles.length === 1 ? '' : 's'}` : 'No role data'}
                        </p>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </SectionCard>

          <SectionCard
            title="Conversation"
            subtitle="Ask for summaries, comparisons, blockers, or next steps."
            icon={MessageSquare}
          >
            <div className="space-y-3">
              {messages.map((msg, index) => (
                <Bubble key={index} msgRole={msg.role} content={msg.content} isNew={msg.isNew} />
              ))}
              {sending && <TypingDots />}
              <div className="flex flex-wrap gap-2 pt-1">
                {quickActions.map(action => (
                  <button
                    key={action}
                    onClick={() => send(action)}
                    className="text-[11px] px-2.5 py-1.5 rounded-full border border-border bg-bg hover:bg-bg-hover text-tx-3 transition-colors"
                  >
                    {action}
                  </button>
                ))}
              </div>
              <div ref={bottomRef} />
            </div>
          </SectionCard>
        </div>

        {error && (
          <div className="px-4 py-2 text-xs text-err border-t border-border bg-err-soft/20">
            {error}
          </div>
        )}

        <div className="p-4 border-t border-border bg-bg-card space-y-3">
          <div className="flex items-end gap-2">
            <textarea
              ref={inputRef}
              value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="Ask about the agent, roles, goals, tasks, or blockers..."
              rows={2}
              className="flex-1 resize-none rounded-xl border border-border bg-bg px-3 py-2 text-sm text-tx-1 placeholder:text-tx-4 outline-none focus:border-accent/40"
            />
            <button
              onClick={() => send(input)}
              disabled={sending || !input.trim()}
              className="btn-primary px-3.5 py-2.5 disabled:opacity-50"
            >
              {sending ? <Loader2 size={14} className="animate-spin" /> : <ArrowRight size={14} />}
            </button>
          </div>
          <p className="text-[11px] text-tx-4 text-center">
            Enter to send | Shift+Enter for a new line
          </p>
        </div>
      </motion.div>
    </motion.div>
  );
}
