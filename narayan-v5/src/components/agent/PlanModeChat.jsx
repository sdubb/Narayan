import { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import {
  Loader2, Send, CheckCircle2, Sparkles, X, ArrowRight,
  Bot, User, AlertCircle, Search, Zap, Paperclip, FileText, Trash2,
} from 'lucide-react';
import { planMode as planModeApi } from '../../api';
import { ConnectorSetupModal, useConnectorVerification } from '../connectors/ConnectorSetupModal';

// Phase labels shown in the progress strip
const PHASE_LABELS = {
  capturing_intent:      'Understanding your goal',
  resolving_connectors:  'Identifying integrations',
  capturing_trigger:     'Setting the trigger',
  capturing_output:      'Defining output',
  capturing_constraints: 'Adding rules',
  reviewing:             'Review',
  complete:              'Done',
};

const PHASE_ORDER = [
  'capturing_intent',
  'resolving_connectors',
  'capturing_trigger',
  'capturing_output',
  'capturing_constraints',
  'reviewing',
  'complete',
];

// Persona labels and ordering
const PERSONA_ORDER = ['teams', 'founders', 'personal'];
const PERSONA_LABELS = {
  teams: 'Team Workflows',
  founders: 'Founder Tools',
  personal: 'Personal Assistants',
};

const MAX_ATTACHMENT_BYTES = 15 * 1024 * 1024;

function formatBytes(bytes) {
  if (!bytes) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

function readFileAsBase64(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result;
      if (typeof result !== 'string') {
        reject(new Error(`Failed to read ${file.name}`));
        return;
      }
      const comma = result.indexOf(',');
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.onerror = () => reject(new Error(`Failed to read ${file.name}`));
    reader.readAsDataURL(file);
  });
}

function attachmentPrompt(attachments) {
  const names = attachments.map(a => a.name).join(', ');
  return `Please analyze the attached file${attachments.length === 1 ? '' : 's'}${names ? `: ${names}` : ''}.`;
}

function testStatusTone(status) {
  if (status === 'pass') return 'bg-ok-soft text-ok border-ok/20';
  if (status === 'partial') return 'bg-amber-500/10 text-amber-600 border-amber-500/20';
  return 'bg-err-soft text-err border-err/20';
}

function confidenceTone(confidence) {
  if (confidence === 'high') return 'text-ok';
  if (confidence === 'partial') return 'text-amber-600';
  return 'text-err';
}

function TestResultPanel({ result, onRevise, revising = false }) {
  if (!result) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-bg px-3 py-3 text-xs text-tx-3">
        Run the deterministic test to validate the workflow outline before saving.
      </div>
    );
  }

  const preflightChecks = result.preflight?.checks || [];
  const sandboxSteps = result.sandbox?.steps || [];
  const statusLabel = String(result.status || 'partial').replace('_', ' ');
  const confidenceLabel = String(result.confidence || 'partial').replace('_', ' ');

  return (
    <div className="rounded-xl border border-border bg-bg px-3 py-3 text-xs text-tx-2 space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <span className={clsx('inline-flex items-center rounded-full border px-2 py-0.5 font-medium capitalize', testStatusTone(result.status))}>
            {statusLabel}
          </span>
          <span className={clsx('font-medium capitalize', confidenceTone(result.confidence))}>
            {confidenceLabel} confidence
          </span>
        </div>
      </div>

      {result.summary ? <p className="text-[11px] leading-relaxed text-tx-3 whitespace-pre-wrap">{result.summary}</p> : null}

      {preflightChecks.length > 0 && (
        <div className="space-y-1">
          <p className="text-[10px] font-semibold uppercase tracking-wide text-tx-4">Preflight</p>
          <div className="space-y-1">
            {preflightChecks.slice(0, 4).map((check, idx) => (
              <div key={`${check.label}-${idx}`} className="flex items-start gap-2">
                <span className={clsx('mt-1 size-1.5 rounded-full shrink-0', check.success ? 'bg-ok' : 'bg-err')} />
                <div className="min-w-0">
                  <p className="text-[11px] text-tx-2">{check.label}</p>
                  {check.detail ? <p className="text-[10px] text-tx-4 whitespace-pre-wrap">{check.detail}</p> : null}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {sandboxSteps.length > 0 && (
        <div className="space-y-1">
          <p className="text-[10px] font-semibold uppercase tracking-wide text-tx-4">Sandbox</p>
          <div className="space-y-1">
            {sandboxSteps.slice(0, 4).map((step, idx) => (
              <div key={`${step.step}-${idx}`} className="flex items-start gap-2">
                <span className={clsx('mt-1 size-1.5 rounded-full shrink-0', step.success && !step.blocked ? 'bg-ok' : step.blocked ? 'bg-amber-500' : 'bg-err')} />
                <div className="min-w-0">
                  <p className="text-[11px] text-tx-2">
                    Step {step.step + 1}: {step.description}
                  </p>
                  {step.error ? (
                    <p className="text-[10px] text-err whitespace-pre-wrap">{step.error}</p>
                  ) : step.output ? (
                    <p className="text-[10px] text-tx-4 whitespace-pre-wrap break-words">{JSON.stringify(step.output)}</p>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {result.status !== 'pass' && (
        <div className="flex items-center justify-between gap-2 pt-1">
          <p className="text-[10px] text-tx-4">
            {result.status === 'partial'
              ? 'Partial results can usually be repaired and retested.'
              : 'This draft should be revised before saving.'}
          </p>
          <button
            type="button"
            onClick={onRevise}
            disabled={!onRevise || revising}
            className="btn-secondary flex items-center gap-2 disabled:opacity-50"
          >
            {revising ? <Loader2 size={13} className="animate-spin" /> : <Sparkles size={13} />}
            {revising ? 'Revising…' : 'Revise plan'}
          </button>
        </div>
      )}
    </div>
  );
}

// ── Phase progress strip ───────────────────────────────────────────────────
function PhaseStrip({ phase }) {
  const current = PHASE_ORDER.indexOf(phase);
  // Skip resolving_connectors in the visual strip (it's transparent to the user)
  const visible = PHASE_ORDER.filter(p => p !== 'resolving_connectors');
  const visibleIdx = visible.indexOf(phase) === -1
    ? visible.indexOf('capturing_intent')
    : visible.indexOf(phase);

  return (
    <div className="flex items-center gap-1.5 px-1">
      {visible.map((p, i) => {
        const done    = i < visibleIdx;
        const active  = i === visibleIdx;
        const future  = i > visibleIdx;
        return (
          <div key={p} className="flex items-center gap-1.5">
            <div className={clsx(
              'h-1 rounded-full transition-all duration-500',
              done   ? 'bg-ok w-6'               : '',
              active ? 'bg-accent w-8'            : '',
              future ? 'bg-border w-4 opacity-50' : '',
            )} />
            {active && (
              <span className="text-[10px] font-medium text-accent whitespace-nowrap">
                {PHASE_LABELS[p]}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

// ── Message bubble ─────────────────────────────────────────────────────────
function Bubble({ role, content, isNew, attachments = [] }) {
  const isUser = role === 'user';
  return (
    <motion.div
      className={clsx('flex gap-3', isUser ? 'flex-row-reverse' : 'flex-row')}
      initial={isNew ? { opacity: 0, y: 8 } : false}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18 }}
    >
      {/* Avatar */}
      <div className={clsx(
        'size-7 rounded-full flex items-center justify-center shrink-0 mt-0.5',
        isUser ? 'bg-tx-1' : 'bg-accent-soft border border-accent/20',
      )}>
        {isUser
          ? <User size={13} className="text-bg-card" />
          : <Bot size={13} className="text-accent" />}
      </div>

      {/* Text */}
      <div className={clsx(
        'max-w-lg rounded-2xl px-4 py-3 text-[13px] leading-relaxed whitespace-pre-wrap',
        isUser
          ? 'bg-tx-1 text-bg-card rounded-tr-sm'
          : 'bg-bg-card border border-border text-tx-1 rounded-tl-sm',
      )}>
        {content}
        {attachments.length > 0 && (
          <div className="mt-3 flex flex-wrap gap-2">
            {attachments.map((file, idx) => (
              <div
                key={`${file.name}-${idx}`}
                className={clsx(
                  'inline-flex items-center gap-1.5 rounded-lg border px-2 py-1 text-[11px]',
                  isUser
                    ? 'border-white/15 bg-white/10 text-bg-card/90'
                    : 'border-border bg-bg text-tx-2',
                )}
              >
                <FileText size={11} className={isUser ? 'text-white/90' : 'text-accent'} />
                <span className="truncate max-w-[10rem]">{file.name}</span>
                {file.size ? <span className="opacity-70">{formatBytes(file.size)}</span> : null}
              </div>
            ))}
          </div>
        )}
      </div>
    </motion.div>
  );
}

// ── Typing indicator ───────────────────────────────────────────────────────
function TypingDots() {
  return (
    <motion.div
      className="flex gap-3"
      initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
    >
      <div className="size-7 rounded-full flex items-center justify-center shrink-0 bg-accent-soft border border-accent/20">
        <Bot size={13} className="text-accent" />
      </div>
      <div className="bg-bg-card border border-border rounded-2xl rounded-tl-sm px-4 py-3 flex items-center gap-1">
        {[0, 1, 2].map(i => (
          <span
            key={i}
            className="size-1.5 rounded-full bg-tx-4 inline-block animate-pulse"
            style={{ animationDelay: `${i * 0.18}s` }}
          />
        ))}
      </div>
    </motion.div>
  );
}

// ── Template Picker ────────────────────────────────────────────────────────
function TemplatePicker({ templates, onSelect, onSkip, loading }) {
  const [search, setSearch] = useState('');
  const [selectedPersona, setSelectedPersona] = useState(null);

  // Group templates by persona
  const grouped = templates.reduce((acc, t) => {
    const p = t.persona || 'personal';
    if (!acc[p]) acc[p] = [];
    acc[p].push(t);
    return acc;
  }, {});

  // Filter by search
  const filtered = templates.filter(t => {
    const q = search.toLowerCase();
    if (!q) return true;
    return t.name.toLowerCase().includes(q) ||
           t.description.toLowerCase().includes(q) ||
           t.category.toLowerCase().includes(q);
  });

  // Filter by persona
  const displayTemplates = selectedPersona
    ? filtered.filter(t => t.persona === selectedPersona)
    : filtered;

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Search */}
      <div className="px-5 py-4 border-b border-border bg-bg-card shrink-0">
        <div className="relative">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-tx-4" />
          <input
            type="text"
            placeholder="Search templates..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            className="w-full bg-bg border border-border rounded-lg pl-9 pr-3 py-2 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-accent focus:ring-1 focus:ring-accent/20"
          />
        </div>

        {/* Persona filter chips */}
        <div className="flex items-center gap-2 mt-3">
          <button
            onClick={() => setSelectedPersona(null)}
            className={clsx(
              'px-2.5 py-1 text-[11px] font-medium rounded-full border transition-all',
              !selectedPersona
                ? 'bg-accent text-white border-accent'
                : 'bg-bg text-tx-3 border-border hover:border-tx-4'
            )}
          >
            All
          </button>
          {PERSONA_ORDER.map(p => (
            <button
              key={p}
              onClick={() => setSelectedPersona(p)}
              className={clsx(
                'px-2.5 py-1 text-[11px] font-medium rounded-full border transition-all',
                selectedPersona === p
                  ? 'bg-accent text-white border-accent'
                  : 'bg-bg text-tx-3 border-border hover:border-tx-4'
              )}
            >
              {PERSONA_LABELS[p] || p}
            </button>
          ))}
        </div>
      </div>

      {/* Template grid */}
      <div className="flex-1 overflow-y-auto px-5 py-4">
        {loading ? (
          <div className="flex items-center justify-center h-32">
            <Loader2 size={20} className="text-tx-4 animate-spin" />
          </div>
        ) : displayTemplates.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-32 text-center">
            <p className="text-sm text-tx-3">No templates found</p>
            <p className="text-xs text-tx-4 mt-1">Try a different search term</p>
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-3">
            {displayTemplates.map(t => (
              <button
                key={t.id}
                onClick={() => onSelect(t)}
                className="group text-left p-4 rounded-xl border border-border bg-bg-card hover:border-accent/40 hover:bg-accent-soft/20 transition-all"
              >
                <div className="flex items-start gap-3">
                  <span className="text-2xl shrink-0">{t.emoji || '⚡'}</span>
                  <div className="min-w-0">
                    <p className="text-sm font-semibold text-tx-1 truncate group-hover:text-accent transition-colors">
                      {t.name}
                    </p>
                    <p className="text-[11px] text-tx-3 mt-1 line-clamp-2 leading-relaxed">
                      {t.description}
                    </p>
                    {t.required_connectors?.length > 0 && (
                      <div className="flex items-center gap-1 mt-2 flex-wrap">
                        {t.required_connectors.slice(0, 3).map(c => (
                          <span key={c} className="text-[9px] bg-accent-soft text-accent border border-accent/20 px-1.5 py-0.5 rounded">
                            {c}
                          </span>
                        ))}
                        {t.required_connectors.length > 3 && (
                          <span className="text-[9px] text-tx-4">+{t.required_connectors.length - 3}</span>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Skip button */}
      <div className="shrink-0 border-t border-border bg-bg-card px-5 py-3">
        <button
          onClick={onSkip}
          className="w-full flex items-center justify-center gap-2 text-sm text-tx-3 hover:text-tx-1 transition-colors"
        >
          <Zap size={14} />
          Start from scratch instead
        </button>
      </div>
    </div>
  );
}

// ── Main component ─────────────────────────────────────────────────────────
// Props:
//   agentName   — name for a new agent (blank if adding role to existing)
//   existingAgentId — set when adding a role to an existing agent
//   onComplete  — called with { agentId, roleId } when the role is saved
//   onCancel    — called if user dismisses (only allowed if existingAgentId is set — new agents must complete)
export default function PlanModeChat({ agentName = 'New Agent', existingAgentId = null, onComplete, onCancel }) {
  const isAddingRole  = !!existingAgentId;
  const canCancel     = isAddingRole; // can only exit if adding a role; new agents must complete plan mode

  const [step,        setStep]        = useState('picker'); // 'picker' | 'chat'
  const [templates,   setTemplates]   = useState([]);
  const [templatesLoading, setTemplatesLoading] = useState(true);
  const [selectedTemplate, setSelectedTemplate] = useState(null);

  const [messages,   setMessages]   = useState([]);
  const [input,      setInput]      = useState('');
  const [phase,      setPhase]      = useState('capturing_intent');
  const [sessionId,  setSessionId]  = useState(null);
  const [loading,    setLoading]    = useState(false);
  const [sending,    setSending]    = useState(false);
  const [complete,   setComplete]   = useState(false);
  const [saving,     setSaving]     = useState(false);
  const [testing,    setTesting]    = useState(false);
  const [revising,   setRevising]   = useState(false);
  const [testResult, setTestResult]  = useState(null);
  const [error,      setError]      = useState('');
  const [pendingAttachments, setPendingAttachments] = useState([]);
  const [attachmentsBusy, setAttachmentsBusy] = useState(false);
  const [activeAgentId, setActiveAgentId] = useState(existingAgentId);
  
  const [showConnectorModal, setShowConnectorModal] = useState(false);
  const [requiredConnectors, setRequiredConnectors] = useState([]);
  const [connectorVerified, setConnectorVerified] = useState(false);
  const connectorVerification = useConnectorVerification(requiredConnectors);
  const bottomRef = useRef(null);
  const inputRef  = useRef(null);
  const fileInputRef = useRef(null);

  // ── Load templates ──────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    async function loadTemplates() {
      try {
        const res = await planModeApi.listTemplates();
        if (!cancelled) {
          setTemplates(res.templates || []);
        }
      } catch (e) {
        // Non-fatal - user can still start from scratch
        console.error('Failed to load templates:', e);
      } finally {
        if (!cancelled) setTemplatesLoading(false);
      }
    }
    loadTemplates();
    return () => { cancelled = true; };
  }, []);

  // ── Start session (from template or scratch) ────────────────────────────
  const startSession = useCallback(async (template = null, agentIdOverride = null) => {
    setLoading(true);
    setError('');
    try {
      const res = await planModeApi.start(agentName, agentIdOverride ?? activeAgentId, template?.id || null);
      setSessionId(res.session_id);
      setActiveAgentId(res.agent_id || agentIdOverride || activeAgentId);
      setMessages([{ role: 'assistant', content: res.message || 'What should this agent do?', isNew: true }]);
      setPhase(res.phase || 'capturing_intent');
      setTestResult(null);
      setSelectedTemplate(template);
      setStep('chat');
    } catch (e) {
      setError(e.message || 'Failed to start session');
    } finally {
      setLoading(false);
    }
  }, [agentName, activeAgentId]);

  const handleAttachmentPick = useCallback(async (event) => {
    const files = Array.from(event.target.files || []);
    event.target.value = '';
    if (files.length === 0) return;

    setError('');
    setAttachmentsBusy(true);
    try {
      const uploads = [];
      for (const file of files) {
        if (file.size > MAX_ATTACHMENT_BYTES) {
          throw new Error(`"${file.name}" is larger than ${formatBytes(MAX_ATTACHMENT_BYTES)}.`);
        }

        const contentBase64 = await readFileAsBase64(file);
        uploads.push({
          id: `${file.name}-${file.size}-${file.lastModified}-${Math.random().toString(36).slice(2)}`,
          name: file.name,
          mime_type: file.type || null,
          size: file.size,
          content_base64: contentBase64,
        });
      }

      setPendingAttachments(prev => [...prev, ...uploads]);
    } catch (e) {
      setError(e.message || 'Failed to read attachment');
    } finally {
      setAttachmentsBusy(false);
    }
  }, []);

  const removeAttachment = useCallback((id) => {
    setPendingAttachments(prev => prev.filter(file => file.id !== id));
  }, []);

  // Auto-scroll on new messages
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, sending]);

  // Focus input when ready
  useEffect(() => {
    if (!loading && !complete && step === 'chat') inputRef.current?.focus();
  }, [loading, complete, step]);

  // ── Send a turn ────────────────────────────────────────────────────────
  const sendMessage = useCallback(async (text) => {
    const trimmed = text.trim();
    const attachments = pendingAttachments.map(({ name, mime_type, content_base64, size }) => ({
      name,
      mime_type,
      content_base64,
      size,
    }));
    const userMsg = trimmed || (attachments.length ? attachmentPrompt(attachments) : '');
    if (!userMsg || sending || !sessionId || attachmentsBusy) return;
    setInput('');
    setError('');
    setSending(true);

    // Append user bubble immediately
    setMessages(prev => [...prev, {
      role: 'user',
      content: userMsg,
      isNew: true,
      attachments: pendingAttachments.map(({ id, name, size }) => ({ id, name, size })),
    }]);

    try {
      const res = await planModeApi.turn(sessionId, userMsg, attachments);
      const newPhase = res.phase || phase;
      setPhase(newPhase);
      setMessages(prev => [...prev, { role: 'assistant', content: res.reply, isNew: true }]);
      setPendingAttachments([]);
      setTestResult(null);

      if (res.complete || newPhase === 'complete') {
        setComplete(true);
      }
    } catch (e) {
      setError(e.message || 'Something went wrong. Try again.');
      // Remove user bubble on error so they can retry
      setMessages(prev => prev.slice(0, -1));
    } finally {
      setSending(false);
    }
  }, [sessionId, sending, phase, pendingAttachments, attachmentsBusy]);

  // ── Save and deploy ────────────────────────────────────────────────────
  const runTest = useCallback(async () => {
    if (!sessionId || testing) return;
    setTesting(true);
    setError('');
    try {
      const res = await planModeApi.test(sessionId);
      setTestResult(res);
    } catch (e) {
      setError(e.message || 'Failed to run test');
    } finally {
      setTesting(false);
    }
  }, [sessionId, testing]);

  const handleRevise = useCallback(async () => {
    if (!sessionId || revising || !testResult || testResult.status === 'pass') return;
    setRevising(true);
    setError('');
    setMessages(prev => [...prev, {
      role: 'user',
      content: 'Please revise the draft using the latest test result and keep the workflow deterministic.',
      isNew: true,
    }]);
    try {
      const res = await planModeApi.revise(sessionId, testResult);
      const newPhase = res.phase || phase;
      setPhase(newPhase);
      setMessages(prev => [...prev, { role: 'assistant', content: res.reply, isNew: true }]);
      setTestResult(null);
      setComplete(newPhase === 'complete');
    } catch (e) {
      setError(e.message || 'Failed to revise plan');
      setMessages(prev => prev.slice(0, -1));
    } finally {
      setRevising(false);
    }
  }, [sessionId, revising, testResult, phase]);

  const continueOrComplete = useCallback(async (res) => {
    if (res?.has_more_roles) {
      setRequiredConnectors([]);
      setConnectorVerified(false);
      setShowConnectorModal(false);
      setTestResult(null);
      setComplete(false);
      setActiveAgentId(res.agent_id);
      await startSession(null, res.agent_id);
      return;
    }

    onComplete?.({ agentId: res.agent_id, roleId: res.role_id });
  }, [startSession, onComplete]);

  const handleSave = useCallback(async () => {
    if (!sessionId) return;
    const status = testResult?.status;
    if (!status || status !== 'pass') {
      const label = status ? `The test result is "${status}".` : 'This plan has not been tested yet.';
      const proceed = window.confirm(`${label}\n\nSave anyway?`);
      if (!proceed) return;
    }
    
    // Extract required connectors from draft
    if (phase === 'complete') {
      try {
        const session = await planModeApi.get(sessionId);
        const draftConnectors = session?.draft_role?.connectors || [];
        
        if (draftConnectors.length > 0) {
          setRequiredConnectors(draftConnectors);
          setShowConnectorModal(true);
          setConnectorVerified(false);
          return; // Don't save yet
        }
      } catch (e) {
        console.warn('Failed to extract connectors:', e);
      }
    }
    
    // Skip connector check if already verified or no connectors
    if (requiredConnectors.length > 0 && !connectorVerified) {
      return;
    }
    
    setSaving(true); setError('');
    try {
      const res = await planModeApi.save(sessionId);
      await continueOrComplete(res);
    } catch (e) {
      setError(e.message || 'Failed to save agent');
    } finally {
      setSaving(false);
    }
  }, [sessionId, testResult, phase, requiredConnectors, connectorVerified, continueOrComplete]);

  // ── Keyboard submit ────────────────────────────────────────────────────
  function onKeyDown(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      if (input.trim() || pendingAttachments.length > 0) {
        e.preventDefault();
        sendMessage(input);
      }
    }
  }

  // Handle connector verification callback
  const handleConnectorsVerified = (verified) => {
    if (verified) {
      setConnectorVerified(true);
      setShowConnectorModal(false);
      // Auto-save after verification
      setTimeout(() => {
        setSaving(true); setError('');
        planModeApi.save(sessionId)
          .then(res => continueOrComplete(res))
          .catch(e => {
            setError(e.message || 'Failed to save agent');
          })
          .finally(() => setSaving(false));
      }, 300);
    }
  };

  // ─────────────────────────────────────────────────────────────────────
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-tx-1/40 backdrop-blur-sm">
      <motion.div
        className="relative w-full max-w-3xl mx-4 h-[85vh] flex flex-col rounded-2xl border border-border bg-bg shadow-md overflow-hidden"
        initial={{ opacity: 0, scale: 0.97, y: 16 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.22, ease: [0.25, 0.1, 0.25, 1] }}
      >
        {/* ── Header ───────────────────────────────────────────── */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-border bg-bg-card shrink-0">
          <div className="flex items-center gap-2.5">
            <div className="size-8 rounded-lg bg-accent-soft border border-accent/20 flex items-center justify-center">
              <Sparkles size={15} className="text-accent" />
            </div>
            <div>
              <p className="text-sm font-semibold text-tx-1">
                {step === 'picker'
                  ? (isAddingRole ? 'Choose a role template' : 'Choose a template')
                  : (isAddingRole ? 'Add a new role' : 'Configure new agent')}
              </p>
              <p className="text-[11px] text-tx-4">
                {step === 'picker'
                  ? 'Select a template to get started quickly'
                  : (isAddingRole
                      ? 'Describe what this role should do'
                      : 'Describe what this agent does — we\'ll set everything up')}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            {step === 'chat' && <PhaseStrip phase={phase} />}
            {canCancel && (
              <button
                onClick={onCancel}
                className="p-1.5 rounded-lg text-tx-4 hover:text-tx-1 hover:bg-bg-hover transition-all"
                title="Cancel"
              >
                <X size={15} />
              </button>
            )}
            {!canCancel && step === 'chat' && (
              <span className="text-[11px] text-tx-4 italic">Complete setup to continue</span>
            )}
          </div>
        </div>

        {/* ── Content ─────────────────────────────────────────────────── */}
        {step === 'picker' ? (
          <TemplatePicker
            templates={templates}
            loading={templatesLoading}
            onSelect={template => startSession(template)}
            onSkip={() => startSession(null)}
          />
        ) : (
          <>
            {/* ── Message area ─────────────────────────────────────── */}
            <div className="flex-1 overflow-y-auto px-5 py-5 space-y-4">
              {loading ? (
                <div className="flex items-center justify-center h-full">
                  <Loader2 size={20} className="text-tx-4 animate-spin" />
                </div>
              ) : (
                <>
                  {selectedTemplate && (
                    <div className="flex items-center gap-2 px-3 py-2 bg-accent-soft/30 border border-accent/20 rounded-lg text-xs text-accent">
                      <span>{selectedTemplate.emoji}</span>
                      <span>Using template: <strong>{selectedTemplate.name}</strong></span>
                    </div>
                  )}
                  {messages.map((msg, i) => (
                    <Bubble
                      key={i}
                      role={msg.role}
                      content={msg.content}
                      isNew={msg.isNew}
                    />
                  ))}
                  {sending && <TypingDots />}
                </>
              )}
              <div ref={bottomRef} />
            </div>

            {/* ── Error ────────────────────────────────────────────── */}
            <AnimatePresence>
              {error && (
                <motion.div
                  className="mx-4 mb-2 flex items-center gap-2 rounded-lg bg-err-soft border border-err/20 px-3 py-2 text-xs text-err"
                  initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
                >
                  <AlertCircle size={12} />
                  {error}
                  <button onClick={() => setError('')} className="ml-auto"><X size={11} /></button>
                </motion.div>
              )}
            </AnimatePresence>

            {/* ── Input / Save footer ───────────────────────────────── */}
            <div className="shrink-0 border-t border-border bg-bg-card px-4 py-3">
              {complete ? (
                // Phase complete — show save button
                <div className="space-y-3">
                  <div className="flex items-center gap-3">
                    <div className="flex-1 flex items-center gap-2 text-sm text-ok">
                      <CheckCircle2 size={15} />
                      <span>Plan confirmed — ready to test and save</span>
                    </div>
                    <button
                      onClick={runTest}
                      disabled={testing || saving}
                      className="btn-secondary flex items-center gap-2 disabled:opacity-50"
                    >
                      {testing
                        ? <Loader2 size={14} className="animate-spin" />
                        : <Zap size={14} />}
                      {testing ? 'Testing…' : testResult ? 'Re-run test' : 'Run test'}
                    </button>
                    <button
                      onClick={handleSave}
                      disabled={saving || testing}
                      className="btn-primary flex items-center gap-2 disabled:opacity-50"
                    >
                      {saving
                        ? <Loader2 size={14} className="animate-spin" />
                        : <ArrowRight size={14} />}
                      {saving ? 'Saving…' : isAddingRole ? 'Add role' : 'Create agent'}
                    </button>
                  </div>
                  <TestResultPanel result={testResult} onRevise={handleRevise} revising={revising} />
                </div>
              ) : (
                // Chat input
                <div className="rounded-xl border border-border bg-bg px-3.5 py-2.5 focus-within:border-border-md focus-within:ring-2 focus-within:ring-accent/10 transition-all">
                  {pendingAttachments.length > 0 && (
                    <div className="mb-2 flex flex-wrap gap-2">
                      {pendingAttachments.map(file => (
                        <div
                          key={file.id}
                          className="inline-flex items-center gap-2 rounded-lg border border-border bg-bg-card px-2.5 py-1.5 text-[11px] text-tx-2"
                        >
                          <FileText size={11} className="text-accent shrink-0" />
                          <div className="min-w-0">
                            <p className="max-w-[12rem] truncate">{file.name}</p>
                            <p className="text-[10px] text-tx-4">{formatBytes(file.size)}</p>
                          </div>
                          <button
                            type="button"
                            onClick={() => removeAttachment(file.id)}
                            className="p-1 rounded-md text-tx-4 hover:text-err hover:bg-err-soft transition-colors"
                            title={`Remove ${file.name}`}
                          >
                            <Trash2 size={11} />
                          </button>
                        </div>
                      ))}
                    </div>
                  )}

                  <div className="flex items-end gap-2.5">
                    <textarea
                      ref={inputRef}
                      value={input}
                      onChange={e => setInput(e.target.value)}
                      onKeyDown={onKeyDown}
                      placeholder={loading ? 'Starting…' : attachmentsBusy ? 'Reading files…' : pendingAttachments.length > 0 ? 'Add a note or press Enter to send files' : 'Reply…'}
                      disabled={loading || sending || attachmentsBusy}
                      rows={1}
                      className="flex-1 bg-transparent text-[13px] text-tx-1 placeholder-tx-4 outline-none resize-none leading-relaxed max-h-28 disabled:opacity-50"
                      onInput={e => {
                        e.target.style.height = 'auto';
                        e.target.style.height = Math.min(e.target.scrollHeight, 112) + 'px';
                      }}
                    />
                    <button
                      type="button"
                      onClick={() => fileInputRef.current?.click()}
                      disabled={loading || sending || attachmentsBusy}
                      className={clsx(
                        'p-1.5 rounded-lg transition-all shrink-0 border',
                        loading || sending || attachmentsBusy
                          ? 'bg-bg-active text-tx-4 border-border cursor-not-allowed'
                          : 'bg-bg-card text-tx-3 border-border hover:text-accent hover:border-accent/40 hover:bg-accent-soft/20',
                      )}
                      title="Attach files"
                    >
                      {attachmentsBusy ? <Loader2 size={14} className="animate-spin" /> : <Paperclip size={14} />}
                    </button>
                    <button
                      type="button"
                      onClick={() => sendMessage(input)}
                      disabled={loading || sending || attachmentsBusy || (!input.trim() && pendingAttachments.length === 0)}
                      className={clsx(
                        'p-1.5 rounded-lg transition-all shrink-0',
                        !loading && !sending && !attachmentsBusy && (input.trim() || pendingAttachments.length > 0)
                          ? 'bg-tx-1 text-bg-card hover:bg-tx-2'
                          : 'bg-bg-active text-tx-4 cursor-not-allowed',
                      )}
                    >
                      {sending ? <Loader2 size={14} className="animate-spin" /> : <Send size={14} />}
                    </button>
                  </div>
                  <input
                    ref={fileInputRef}
                    type="file"
                    className="hidden"
                    multiple
                    accept=".pdf,.xls,.xlsx,.csv,.txt,.md,.json,.html,.htm,.xml,.yaml,.yml,.log,.rst,.toml,.doc,.docx"
                    onChange={handleAttachmentPick}
                  />
                </div>
              )}
              {!complete && (
                <p className="text-[11px] text-tx-4 mt-1.5 text-center">
                  Enter to send | Shift+Enter for new line | Attach PDFs, spreadsheets, CSVs, and docs
                </p>
              )}
            </div>
          </>
        )}
      </motion.div>

      {/* Connector Setup Modal */}
      <AnimatePresence>
        {showConnectorModal && (
          <ConnectorSetupModal
            requiredConnectors={requiredConnectors}
            onVerified={handleConnectorsVerified}
            onClose={() => {
              setShowConnectorModal(false);
              setError('Connectors not verified. You can add them in Settings later.');
            }}
            mode="modal"
          />
        )}
      </AnimatePresence>
    </div>
  );
}
