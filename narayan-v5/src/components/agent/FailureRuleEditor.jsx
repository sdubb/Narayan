import { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Plus, Trash2, ChevronDown, ChevronUp, Shield, AlertTriangle } from 'lucide-react';

// ── Action badge ───────────────────────────────────────────────────────────
const ACTION_STYLES = {
  skip_and_log:     { label: 'Skip & Log',  bg: 'bg-info-soft',  text: 'text-info',  border: 'border-info/20'  },
  skip_silently:    { label: 'Skip',        bg: 'bg-bg-active',  text: 'text-tx-3',  border: 'border-border'   },
  retry_once:       { label: 'Retry ×1',   bg: 'bg-warn-soft',  text: 'text-warn',  border: 'border-warn/20'  },
  escalate:         { label: 'Escalate',   bg: 'bg-vio-soft',   text: 'text-vio',   border: 'border-vio/20'   },
  abort:            { label: 'Abort Run',  bg: 'bg-err-soft',   text: 'text-err',   border: 'border-err/20'   },
};

function actionKey(rule) {
  if (!rule?.action) return 'skip_and_log';
  const a = rule.action;
  if (typeof a === 'string') return a;
  if (a.action) return a.action;          // { action: "skip_and_log", ... }
  if ('log_path' in a) return 'skip_and_log';
  if (a === 'SkipSilently' || a.SkipSilently !== undefined) return 'skip_silently';
  if (a === 'RetryOnce'    || a.RetryOnce    !== undefined) return 'retry_once';
  if (a === 'Abort'        || a.Abort        !== undefined) return 'abort';
  if (a.EscalateToHuman   !== undefined) return 'escalate';
  return 'skip_and_log';
}

function ActionBadge({ rule }) {
  const key   = actionKey(rule);
  const style = ACTION_STYLES[key] ?? ACTION_STYLES.skip_and_log;
  return (
    <span className={`text-[10px] font-semibold px-2 py-0.5 rounded-full border ${style.bg} ${style.text} ${style.border}`}>
      {style.label}
    </span>
  );
}

// ── Add rule form ──────────────────────────────────────────────────────────
function AddRuleForm({ onAdd, onClose }) {
  const [text,       setText]       = useState('');
  const [toolScope,  setToolScope]  = useState('');
  const [action,     setAction]     = useState('skip_and_log');
  const [channel,    setChannel]    = useState('');

  function submit() {
    if (!text.trim()) return;
    const rule = {
      text:       text.trim(),
      tool_scope: toolScope.trim() || null,
      action,
      notify_channel: action === 'escalate' ? (channel.trim() || null) : null,
    };
    onAdd(rule);
    onClose();
  }

  return (
    <motion.div
      className="rounded-xl border border-accent/30 bg-accent-soft/20 p-4 space-y-3"
      initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}
    >
      <p className="text-[12px] font-semibold text-tx-2">Add failure rule</p>

      {/* Description */}
      <div>
        <label className="text-[11px] text-tx-4 mb-1 block">When this happens</label>
        <textarea
          value={text}
          onChange={e => setText(e.target.value)}
          placeholder="e.g. 'If Salesforce query fails' or 'On missing email address'"
          rows={2}
          className="w-full text-[12px] bg-bg border border-border rounded-lg px-3 py-2
                     text-tx-1 placeholder-tx-4 outline-none resize-none
                     focus:border-accent/50 focus:ring-1 focus:ring-accent/10 transition-all"
        />
      </div>

      {/* Tool scope */}
      <div>
        <label className="text-[11px] text-tx-4 mb-1 block">Tool scope <span className="text-tx-5">(optional)</span></label>
        <input
          value={toolScope}
          onChange={e => setToolScope(e.target.value)}
          placeholder="e.g. salesforce, web_search"
          className="w-full text-[12px] bg-bg border border-border rounded-lg px-3 py-2
                     text-tx-1 placeholder-tx-4 outline-none
                     focus:border-accent/50 focus:ring-1 focus:ring-accent/10 transition-all"
        />
      </div>

      {/* Action */}
      <div>
        <label className="text-[11px] text-tx-4 mb-1 block">Then do</label>
        <select
          value={action}
          onChange={e => setAction(e.target.value)}
          className="w-full text-[12px] bg-bg border border-border rounded-lg px-3 py-2
                     text-tx-1 outline-none focus:border-accent/50 transition-all"
        >
          <option value="skip_and_log">Skip record and log to errors.txt</option>
          <option value="skip_silently">Skip silently (no log)</option>
          <option value="retry_once">Retry once</option>
          <option value="escalate">Escalate to human</option>
          <option value="abort">Abort the entire run</option>
        </select>
      </div>

      {/* Notify channel (only for escalate) */}
      <AnimatePresence>
        {action === 'escalate' && (
          <motion.div
            initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }}
          >
            <label className="text-[11px] text-tx-4 mb-1 block">Notify channel</label>
            <input
              value={channel}
              onChange={e => setChannel(e.target.value)}
              placeholder="#ops-alerts"
              className="w-full text-[12px] bg-bg border border-border rounded-lg px-3 py-2
                         text-tx-1 placeholder-tx-4 outline-none focus:border-accent/50 transition-all"
            />
          </motion.div>
        )}
      </AnimatePresence>

      <div className="flex gap-2 pt-1">
        <button onClick={submit} disabled={!text.trim()}
          className="btn-primary text-xs flex-1 disabled:opacity-50">
          Add rule
        </button>
        <button onClick={onClose} className="btn-secondary text-xs px-3">Cancel</button>
      </div>
    </motion.div>
  );
}

// ── Main component ─────────────────────────────────────────────────────────
export default function FailureRuleEditor({ rules = [], onAdd, onRemove, onApplyAll, className = '' }) {
  const [open,   setOpen]   = useState(false);
  const [adding, setAdding] = useState(false);

  const hasRules = rules.length > 0;

  return (
    <div className={`rounded-xl border border-border bg-bg-card overflow-hidden ${className}`}>
      {/* Header toggle */}
      <button
        onClick={() => setOpen(o => !o)}
        className="w-full flex items-center gap-3 px-4 py-3.5 hover:bg-bg-hover transition-colors"
      >
        <div className="size-7 rounded-lg bg-err-soft border border-err/20 flex items-center justify-center shrink-0">
          <Shield size={13} className="text-err" />
        </div>
        <div className="flex-1 text-left">
          <p className="text-[13px] font-semibold text-tx-1">Failure handling</p>
          <p className="text-[11px] text-tx-4">
            {hasRules ? `${rules.length} rule${rules.length !== 1 ? 's' : ''}` : 'No rules — add to control failure behaviour'}
          </p>
        </div>
        {open ? <ChevronUp size={13} className="text-tx-4" /> : <ChevronDown size={13} className="text-tx-4" />}
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ height: 0 }} animate={{ height: 'auto' }} exit={{ height: 0 }}
            className="overflow-hidden border-t border-border"
          >
            <div className="p-4 space-y-2">
              {/* Rule list */}
              {rules.length === 0 && !adding ? (
                <div className="flex flex-col items-center gap-2 py-4 text-center">
                  <AlertTriangle size={16} className="text-tx-5" />
                  <p className="text-[12px] text-tx-4">
                    No failure rules. Without rules, all failures abort the run.
                  </p>
                </div>
              ) : (
                <div className="space-y-1.5">
                  <AnimatePresence>
                    {rules.map((rule, i) => (
                      <motion.div
                        key={`${rule.text}-${i}`}
                        className="flex items-start gap-2.5 p-2.5 rounded-lg bg-bg border border-border group"
                        initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }}
                        exit={{ opacity: 0, x: 8 }}
                        transition={{ delay: i * 0.04 }}
                      >
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2 flex-wrap mb-1">
                            <ActionBadge rule={rule} />
                            {rule.tool_scope && (
                              <span className="text-[10px] bg-bg-active text-tx-3 px-1.5 py-0.5 rounded border border-border">
                                {rule.tool_scope}
                              </span>
                            )}
                          </div>
                          <p className="text-[12px] text-tx-2 leading-relaxed">{rule.text}</p>
                          {rule.action?.EscalateToHuman?.notify_channel && (
                            <p className="text-[11px] text-vio mt-1">
                              → notify {rule.action.EscalateToHuman.notify_channel}
                            </p>
                          )}
                        </div>
                        <button
                          onClick={() => onRemove(rule.text)}
                          className="p-1.5 rounded-lg text-tx-5 hover:text-err hover:bg-err-soft opacity-0 group-hover:opacity-100 transition-all shrink-0"
                        >
                          <Trash2 size={12} />
                        </button>
                      </motion.div>
                    ))}
                  </AnimatePresence>
                </div>
              )}

              {/* Add form */}
              <AnimatePresence>
                {adding && (
                  <AddRuleForm
                    onAdd={rule => { onAdd(rule); setAdding(false); }}
                    onClose={() => setAdding(false)}
                  />
                )}
              </AnimatePresence>

              {/* Footer actions */}
              {!adding && (
                <div className="flex gap-2 pt-1">
                  <button
                    onClick={() => setAdding(true)}
                    className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-medium
                               rounded-lg border border-dashed border-accent/40 text-accent
                               hover:bg-accent-soft/20 hover:border-accent transition-all"
                  >
                    <Plus size={11} />
                    Add rule
                  </button>
                  {rules.length > 0 && onApplyAll && (
                    <button
                      onClick={onApplyAll}
                      className="ml-auto text-[11px] text-tx-4 hover:text-accent transition-colors px-2"
                    >
                      Apply all changes
                    </button>
                  )}
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
