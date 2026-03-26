import { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import {
  X, Send, Loader2, Bot, User, CheckCircle2,
  AlertCircle, ChevronRight, RotateCcw,
} from 'lucide-react';
import { roleChat as roleChatApi, agentDefs as agentDefsApi } from '../../api';
import { ConnectorSetupModal } from '../connectors/ConnectorSetupModal';
import FailureRuleEditor from './FailureRuleEditor';

// ── Message bubble ─────────────────────────────────────────────────────────
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
        {isUser
          ? <User size={11} className="text-bg-card" />
          : <Bot size={11} className="text-accent" />}
      </div>
      <div className={clsx(
        'max-w-sm rounded-2xl px-3.5 py-2.5 text-[13px] leading-relaxed whitespace-pre-wrap',
        isUser
          ? 'bg-tx-1 text-bg-card rounded-tr-sm'
          : 'bg-bg-card border border-border text-tx-1 rounded-tl-sm',
      )}>
        {content}
      </div>
    </motion.div>
  );
}

// ── Pending change confirmation card ──────────────────────────────────────
function ChangeCard({ change, onConfirm, onDismiss, applying }) {
  const typeLabels = {
    schedule:          'Change schedule',
    add_constraint:    'Add constraint',
    remove_constraint: 'Remove constraint',
    update_guidelines: 'Update guidelines',
    update_output:     'Update output',
    update_connectors: 'Update connectors',
    rename_role:       'Rename role',
    pause_role:        'Pause role',
    resume_role:       'Resume role',
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
      className="mx-4 mb-3 rounded-xl border-2 border-accent/30 bg-accent-soft/20 p-3.5"
    >
      <div className="flex items-start gap-2.5 mb-3">
        <div className="size-7 rounded-lg bg-accent flex items-center justify-center shrink-0">
          <ChevronRight size={13} className="text-white" />
        </div>
        <div className="flex-1 min-w-0">
          <p className="text-xs font-semibold text-accent">
            {typeLabels[change.change_type] || 'Proposed change'}
          </p>
          <p className="text-[12px] text-tx-2 mt-0.5 leading-relaxed">{change.description}</p>
        </div>
      </div>
      <div className="flex gap-2">
        <button
          onClick={onConfirm}
          disabled={applying}
          className="btn-primary text-xs flex-1 flex items-center justify-center gap-1.5 disabled:opacity-50"
        >
          {applying
            ? <Loader2 size={11} className="animate-spin" />
            : <CheckCircle2 size={11} />}
          Apply change
        </button>
        <button
          onClick={onDismiss}
          disabled={applying}
          className="btn-secondary text-xs px-3"
        >
          Dismiss
        </button>
      </div>
    </motion.div>
  );
}

// ── Typing indicator ───────────────────────────────────────────────────────
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

// ── Main drawer ────────────────────────────────────────────────────────────
export default function RoleChatDrawer({ roleId, agentId, roleName, onClose, onRoleChanged }) {
  const [messages,       setMessages]       = useState([]);
  const [input,          setInput]          = useState('');
  const [sessionId,      setSessionId]      = useState(null);
  const [loading,        setLoading]        = useState(true);
  const [sending,        setSending]        = useState(false);
  const [pendingChange,  setPendingChange]  = useState(null);
  const [applying,       setApplying]       = useState(false);
  const [error,          setError]          = useState('');
  // Failure rules from the role — loaded with the session greeting
  const [failureRules,   setFailureRules]   = useState([]);
  
  const [showConnectorModal, setShowConnectorModal] = useState(false);
  const [requiredConnectors, setRequiredConnectors] = useState([]);
  const [pendingChangeAfterConnectors, setPendingChangeAfterConnectors] = useState(null);
  
  const bottomRef = useRef(null);
  const inputRef  = useRef(null);

  // ── Start session ──────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    async function start() {
      try {
        const [res, roleRes] = await Promise.all([
          roleChatApi.start(roleId),
          // Load the role's current failure rules for the inline editor
          agentDefsApi.listRoles(agentId).catch(() => null),
        ]);
        if (cancelled) return;
        setSessionId(res.session_id);
        setMessages([{ role: 'assistant', content: res.message, isNew: false }]);
        // Extract failure_handling from the matching role
        if (roleRes?.roles) {
          const role = roleRes.roles.find(r => r.id === roleId);
          if (role?.execution_guidelines?.failure_handling) {
            setFailureRules(role.execution_guidelines.failure_handling);
          }
        }
      } catch (e) {
        if (!cancelled) setError(e.message || 'Failed to start');
      } finally {
        if (!cancelled) { setLoading(false); setTimeout(() => inputRef.current?.focus(), 50); }
      }
    }
    start();
    return () => { cancelled = true; };
  }, [roleId, agentId]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, sending, pendingChange]);

  // ── Send message ───────────────────────────────────────────────────────
  const send = useCallback(async (text) => {
    if (!text.trim() || sending || !sessionId) return;
    const msg = text.trim();
    setInput('');
    setError('');
    setSending(true);
    setMessages(prev => [...prev, { role: 'user', content: msg, isNew: true }]);

    try {
      const res = await roleChatApi.turn(roleId, sessionId, msg);
      setMessages(prev => [...prev, { role: 'assistant', content: res.reply, isNew: true }]);
      if (res.pending_change) {
        setPendingChange(res.pending_change);
      }
    } catch (e) {
      setError(e.message || 'Something went wrong');
      setMessages(prev => prev.slice(0, -1));
    } finally {
      setSending(false);
    }
  }, [roleId, sessionId, sending]);

  // ── Apply change ───────────────────────────────────────────────────────
  async function applyChange() {
    if (!pendingChange) return;
    
    // If change involves connectors, verify first
    if (pendingChange.change_type === 'update_connectors' && pendingChange.connectors_to_add?.length > 0) {
      setRequiredConnectors(pendingChange.connectors_to_add);
      setPendingChangeAfterConnectors(pendingChange);
      setShowConnectorModal(true);
      return; // Don't apply yet
    }
    
    // Otherwise apply directly
    await doApplyChange(pendingChange);
  }

  // Apply the change after connectors are verified
  async function doApplyChange(change) {
    if (!change) return;
    setApplying(true); setError('');
    try {
      await roleChatApi.apply(roleId, sessionId, change);
      const confirmMsg = `✓ Done — ${change.description}`;
      setMessages(prev => [...prev, { role: 'assistant', content: confirmMsg, isNew: true }]);
      setPendingChange(null);
      setPendingChangeAfterConnectors(null);
      onRoleChanged?.(); // refresh AgentPage
    } catch (e) {
      setError(e.message || 'Failed to apply change');
    } finally {
      setApplying(false);
    }
  }

  const handleConnectorsVerified = (verified) => {
    if (verified && pendingChangeAfterConnectors) {
      setShowConnectorModal(false);
      setTimeout(() => doApplyChange(pendingChangeAfterConnectors), 300);
    }
  };

  function onKeyDown(e) {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(input); }
  }

  // ── Quick actions ──────────────────────────────────────────────────────
  const quickActions = [
    'Why did the last run fail?',
    'What does this role do?',
    'Change the schedule',
    'Show recent runs',
  ];

  return (
    <motion.div
      className="fixed inset-0 z-50 flex justify-end"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
    >
      {/* Backdrop */}
      <div className="absolute inset-0 bg-tx-1/20 backdrop-blur-[2px]" onClick={onClose} />

      {/* Drawer */}
      <motion.div
        className="relative w-full max-w-md h-full flex flex-col bg-bg border-l border-border shadow-xl"
        initial={{ x: '100%' }}
        animate={{ x: 0 }}
        exit={{ x: '100%' }}
        transition={{ type: 'spring', damping: 30, stiffness: 300 }}
      >
        {/* Header */}
        <div className="flex items-center gap-3 px-4 py-4 border-b border-border bg-bg-card shrink-0">
          <div className="size-8 rounded-lg bg-accent-soft border border-accent/20 flex items-center justify-center">
            <Bot size={15} className="text-accent" />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-sm font-semibold text-tx-1 truncate">{roleName}</p>
            <p className="text-[11px] text-tx-4">Ask questions or request changes</p>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg text-tx-4 hover:text-tx-1 hover:bg-bg-hover transition-all"
          >
            <X size={15} />
          </button>
        </div>

        {/* Messages */}
        <div className="flex-1 overflow-y-auto px-4 py-4 space-y-3">
          {loading ? (
            <div className="flex items-center justify-center h-full">
              <Loader2 size={18} className="text-tx-4 animate-spin" />
            </div>
          ) : (
            <>
              {messages.map((msg, i) => (
                <Bubble key={i} role={msg.role} content={msg.content} isNew={msg.isNew} />
              ))}
              {sending && <TypingDots />}

              {/* Quick actions shown only at start */}
              {messages.length === 1 && !sending && (
                <div className="flex flex-wrap gap-1.5 pt-1">
                  {quickActions.map(q => (
                    <button
                      key={q}
                      onClick={() => send(q)}
                      className="px-2.5 py-1 rounded-full text-[11px] border border-border
                                 text-tx-3 hover:text-accent hover:border-accent/40
                                 hover:bg-accent-soft/20 transition-all"
                    >
                      {q}
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
          <div ref={bottomRef} />
        </div>

        {/* Pending change card */}
        <AnimatePresence>
          {pendingChange && (
            <ChangeCard
              change={pendingChange}
              onConfirm={applyChange}
              onDismiss={() => setPendingChange(null)}
              applying={applying}
            />
          )}
        </AnimatePresence>

        {/* Error */}
        <AnimatePresence>
          {error && (
            <motion.div
              initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
              className="mx-4 mb-2 flex items-center gap-2 rounded-lg bg-err-soft border border-err/20 px-3 py-2 text-xs text-err"
            >
              <AlertCircle size={11} />{error}
              <button onClick={() => setError('')} className="ml-auto"><X size={11} /></button>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Failure rule editor */}
        {!loading && (
          <FailureRuleEditor
            rules={failureRules}
            className="mx-4 mb-3 shrink-0"
            onAdd={async rule => {
              setError('');
              try {
                if (sessionId) {
                  await roleChatApi.apply(roleId, sessionId, {
                    change_type: 'add_failure_rule',
                    description: `Add rule: ${rule.text}`,
                    new_value:   rule,
                  });
                }
                setFailureRules(prev => [...prev, rule]);
                onRoleChanged?.();
              } catch (e) { setError(e.message || 'Failed to add rule'); }
            }}
            onRemove={async text => {
              setError('');
              try {
                if (sessionId) {
                  await roleChatApi.apply(roleId, sessionId, {
                    change_type: 'remove_failure_rule',
                    description: `Remove rule: ${text}`,
                    new_value:   { text },
                  });
                }
                setFailureRules(prev => prev.filter(r => r.text !== text));
                onRoleChanged?.();
              } catch (e) { setError(e.message || 'Failed to remove rule'); }
            }}
          />
        )}

        {/* Input */}
        <div className="shrink-0 border-t border-border bg-bg-card px-4 py-3">
          <div className="flex items-end gap-2 rounded-xl border border-border bg-bg
                          px-3.5 py-2.5 focus-within:border-border-md focus-within:ring-2
                          focus-within:ring-accent/10 transition-all">
            <textarea
              ref={inputRef}
              value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder={loading ? 'Starting…' : 'Ask anything about this role…'}
              disabled={loading || sending}
              rows={1}
              className="flex-1 bg-transparent text-[13px] text-tx-1 placeholder-tx-4
                         outline-none resize-none leading-relaxed max-h-24 disabled:opacity-50"
              onInput={e => {
                e.target.style.height = 'auto';
                e.target.style.height = Math.min(e.target.scrollHeight, 96) + 'px';
              }}
            />
            <button
              onClick={() => send(input)}
              disabled={loading || sending || !input.trim()}
              className={clsx(
                'p-1.5 rounded-lg transition-all shrink-0',
                input.trim() && !loading && !sending
                  ? 'bg-tx-1 text-bg-card hover:bg-tx-2'
                  : 'bg-bg-active text-tx-4 cursor-not-allowed',
              )}
            >
              {sending ? <Loader2 size={13} className="animate-spin" /> : <Send size={13} />}
            </button>
          </div>
          <p className="text-[11px] text-tx-4 mt-1.5 text-center">
            Enter to send · Shift+Enter for new line
          </p>
        </div>
      </motion.div>

      {/* Connector Setup Modal */}
      <AnimatePresence>
        {showConnectorModal && (
          <ConnectorSetupModal
            requiredConnectors={requiredConnectors}
            onVerified={handleConnectorsVerified}
            onClose={() => {
              setShowConnectorModal(false);
              setPendingChangeAfterConnectors(null);
              setError('Connectors not verified. You can add them in Settings later.');
            }}
            mode="modal"
          />
        )}
      </AnimatePresence>
    </motion.div>
  );
}
