import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { Bot, Lock, Plug, Send, Loader2, CheckCircle2, ArrowRight } from 'lucide-react';
import { agents } from '../../api';

function normalizeQuestion(question, index) {
  if (typeof question === 'string') {
    return { id: `q_${index}`, prompt: question, placeholder: 'Your answer...', helperText: '', options: [], required: true, secret: false, connectorType: '', actionLabel: '' };
  }
  const options = Array.isArray(question?.options)
    ? question.options.map(o => typeof o === 'string' ? o : o?.label).filter(Boolean)
    : [];
  return {
    id: question?.id || `q_${index}`,
    prompt: question?.prompt || question?.question || `Question ${index + 1}`,
    placeholder: question?.placeholder || 'Your answer...',
    helperText: question?.helper_text || question?.helperText || '',
    options,
    required: question?.required !== false,
    secret: Boolean(question?.secret),
    connectorType: question?.connector_type || question?.connectorType || '',
    actionLabel: question?.action_label || question?.actionLabel || '',
  };
}

export default function ClarificationCard({ agentId, questions, onDone, onNavigateSettings }) {
  const normalized = questions.map(normalizeQuestion);
  const [answers, setAnswers] = useState(normalized.map(() => ''));
  const [loading, setLoading] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [err, setErr] = useState('');

  useEffect(() => {
    setAnswers(normalized.map(() => ''));
    setSubmitted(false);
    setErr('');
  }, [questions]);

  async function submit() {
    setLoading(true); setErr('');
    try {
      await agents.clarify(agentId, answers);
      setSubmitted(true);
      setTimeout(onDone, 1500);
    } catch (e) { setErr(e.message); }
    finally { setLoading(false); }
  }

  const hasMissing = normalized.some((q, i) => q.required && !answers[i]?.trim());

  if (submitted) {
    return (
      <motion.div
        className="rounded-xl border border-ok/25 bg-ok-soft p-4 flex items-center gap-3"
        initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
      >
        <CheckCircle2 size={16} className="text-ok shrink-0" />
        <span className="text-sm font-medium text-ok">Answers received — agent resuming</span>
      </motion.div>
    );
  }

  return (
    <motion.div
      className="rounded-xl border-l-4 border-l-warn border border-warn/25 bg-warn-soft/30 overflow-hidden"
      initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }}
    >
      <div className="flex items-center gap-2 px-4 py-3 border-b border-warn/15">
        <Bot size={14} className="text-warn" />
        <span className="text-sm font-semibold text-warn">Needs clarification</span>
      </div>
      <div className="px-4 py-4 space-y-4">
        {normalized.map((q, i) => (
          <div key={q.id} className="space-y-2">
            <div className="flex items-center gap-2">
              <p className="text-sm text-tx-1 font-medium">{q.prompt}</p>
              {q.secret && <span className="badge bg-warn-soft text-warn border border-warn/20"><Lock size={9} /> secret</span>}
              {q.connectorType && <span className="badge bg-info-soft text-info border border-info/20"><Plug size={9} /> {q.connectorType}</span>}
            </div>
            {q.helperText && <p className="text-xs text-tx-3">{q.helperText}</p>}
            {q.connectorType && onNavigateSettings && (
              <button onClick={onNavigateSettings} className="inline-flex items-center gap-1.5 text-xs font-medium text-accent hover:text-accent-text transition-colors">
                {q.actionLabel || `Connect ${q.connectorType} in Settings`} <ArrowRight size={10} />
              </button>
            )}
            <input
              value={answers[i]}
              onChange={e => { const n = [...answers]; n[i] = e.target.value; setAnswers(n); }}
              onKeyDown={e => { if (e.key === 'Enter') submit(); }}
              type={q.secret ? 'password' : 'text'}
              placeholder={q.placeholder}
              className="input-field"
            />
            {q.options.length > 0 && (
              <div className="flex flex-wrap gap-2">
                {q.options.map(opt => (
                  <button key={opt} onClick={() => { const n = [...answers]; n[i] = opt; setAnswers(n); }}
                    className={clsx('rounded-full border px-2.5 py-1 text-xs transition-colors',
                      answers[i] === opt ? 'border-accent bg-accent-soft text-accent' : 'border-border text-tx-3 hover:border-border-md'
                    )}>
                    {opt}
                  </button>
                ))}
              </div>
            )}
          </div>
        ))}
        {err && <p className="text-xs text-err">{err}</p>}
        <button onClick={submit} disabled={loading || hasMissing}
          className="btn-primary flex items-center gap-2 disabled:opacity-50">
          {loading ? <Loader2 size={12} className="animate-spin" /> : <Send size={12} />}
          Submit answers
        </button>
      </div>
    </motion.div>
  );
}
