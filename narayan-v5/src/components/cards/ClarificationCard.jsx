import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { Bot, Lock, Plug, Send, Loader2, CheckCircle2, ArrowRight } from 'lucide-react';
import { agents } from '../../api';

function normalizeQuestion(question, index) {
  if (typeof question === 'string') {
    return {
      id: `q_${index}`,
      prompt: question,
      placeholder: 'Your answer...',
      helperText: '',
      options: [],
      required: true,
      secret: false,
      connectorType: '',
      actionLabel: '',
      backendKind: '',
      questionType: 'text',
      multiSelect: false,
    };
  }
  const options = Array.isArray(question?.options)
    ? question.options.map(o => typeof o === 'string' ? o : o?.label).filter(Boolean)
    : [];
  const questionType = String(question?.question_type || question?.type || '').trim().toLowerCase();
  const inferredBackendKind = String(question?.backend_kind || question?.backendKind || question?.id || '')
    .toLowerCase()
    .includes('database') ? 'database'
    : String(question?.backend_kind || question?.backendKind || question?.id || '').toLowerCase().includes('api') ? 'api'
    : String(question?.backend_kind || question?.backendKind || question?.id || '').toLowerCase().includes('mcp') ? 'mcp'
    : String(question?.backend_kind || question?.backendKind || question?.id || '').toLowerCase().includes('acp') ? 'acp'
    : String(question?.backend_kind || question?.backendKind || '').toLowerCase();
  const derivedType = questionType.includes('card_open')
    ? 'card_open'
    : questionType.includes('multi_select') || question?.multi_select || question?.multiSelect
      ? 'multi_select'
      : questionType.includes('approval') || questionType.includes('decision')
        ? (options.length > 0 ? 'mcq' : 'text')
      : questionType.includes('text')
        ? 'text'
        : questionType.includes('mcq') || questionType.includes('choice') || options.length > 0
        ? 'mcq'
        : 'text';
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
    cardType: question?.card_type || question?.cardType || '',
    backendKind: inferredBackendKind,
    requiredFields: Array.isArray(question?.required_fields)
      ? question.required_fields
      : Array.isArray(question?.requiredFields)
        ? question.requiredFields
        : [],
    bindingTarget: question?.binding_target || question?.bindingTarget || '',
    resumeToken: question?.resume_token || question?.resumeToken || '',
    questionType: derivedType,
    multiSelect: Boolean(question?.multi_select || question?.multiSelect || derivedType === 'multi_select'),
  };
}

export default function ClarificationCard({ agentId, questions, onDone, onNavigateSettings }) {
  const normalized = questions.map(normalizeQuestion);
  const [answers, setAnswers] = useState(normalized.map(() => ''));
  const [selectedOptions, setSelectedOptions] = useState(normalized.map(() => []));
  const [loading, setLoading] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [err, setErr] = useState('');

  function openSetup(q) {
    onNavigateSettings?.({
      cardType: q.cardType,
      backendKind: q.backendKind,
      bindingTarget: q.bindingTarget,
      requiredFields: q.requiredFields,
      resumeToken: q.resumeToken,
      connectorType: q.connectorType,
    });
  }

  useEffect(() => {
    setAnswers(normalized.map(() => ''));
    setSelectedOptions(normalized.map(() => []));
    setSubmitted(false);
    setErr('');
  }, [questions]);

  function updateOptionAnswer(index, option, multiSelect) {
    if (!multiSelect) {
      setAnswers(prev => {
        const next = [...prev];
        next[index] = option;
        return next;
      });
      return;
    }

    setSelectedOptions(prevSelected => {
      const current = Array.isArray(prevSelected[index]) ? prevSelected[index] : [];
      const nextSelected = current.includes(option)
        ? current.filter(item => item !== option)
        : [...current, option];
      setAnswers(prev => {
        const next = [...prev];
        next[index] = nextSelected.join(', ');
        return next;
      });
      const updated = [...prevSelected];
      updated[index] = nextSelected;
      return updated;
    });
  }

  async function submit() {
    setLoading(true); setErr('');
    try {
      await agents.clarify(agentId, answers);
      setSubmitted(true);
      setTimeout(onDone, 1500);
    } catch (e) { setErr(e.message); }
    finally { setLoading(false); }
  }

  const hasMissing = normalized.some((q, i) => {
    if (!q.required) return false;
    if (q.questionType === 'card_open') return false;
    return !answers[i]?.trim();
  });

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
              {q.cardType && <span className="badge bg-accent-soft text-accent border border-accent/20">{q.cardType.replace(/_/g, ' ')}</span>}
              {q.backendKind && <span className="badge bg-accent-soft text-accent border border-accent/20">{q.backendKind.toUpperCase()}</span>}
              {q.questionType && <span className="badge bg-bg-card text-tx-3 border border-border">{q.questionType.replace(/_/g, ' ')}</span>}
            </div>
            {q.helperText && <p className="text-xs text-tx-3">{q.helperText}</p>}
            <div className="flex flex-wrap items-center gap-2 text-[11px] text-tx-4">
              {q.bindingTarget && <span>Binding target: {q.bindingTarget}</span>}
              {q.requiredFields.length > 0 && <span>Required: {q.requiredFields.join(', ')}</span>}
              {q.resumeToken && <span className="font-mono">Resume: {q.resumeToken}</span>}
            </div>
            {q.questionType === 'card_open' && (q.connectorType || q.cardType || q.bindingTarget || q.backendKind) && onNavigateSettings && (
              <button onClick={() => openSetup(q)} className="inline-flex items-center gap-1.5 text-xs font-medium text-accent hover:text-accent-text transition-colors">
                {q.actionLabel || `Open ${q.cardType ? q.cardType.replace(/_/g, ' ') : (q.backendKind || q.connectorType)} setup`} <ArrowRight size={10} />
              </button>
            )}
            {q.questionType !== 'card_open' && (
              <input
                value={answers[i]}
                onChange={e => { const n = [...answers]; n[i] = e.target.value; setAnswers(n); }}
                onKeyDown={e => { if (e.key === 'Enter') submit(); }}
                type={q.secret ? 'password' : 'text'}
                placeholder={q.placeholder}
                className="input-field"
              />
            )}
            {q.options.length > 0 && q.questionType !== 'card_open' && (
              <div className="flex flex-wrap gap-2">
                {q.options.map(opt => (
                  <button key={opt} onClick={() => updateOptionAnswer(i, opt, q.multiSelect)}
                    className={clsx('rounded-full border px-2.5 py-1 text-xs transition-colors',
                      q.multiSelect
                        ? selectedOptions[i]?.includes(opt)
                          ? 'border-accent bg-accent-soft text-accent'
                          : 'border-border text-tx-3 hover:border-border-md'
                        : answers[i] === opt
                          ? 'border-accent bg-accent-soft text-accent'
                          : 'border-border text-tx-3 hover:border-border-md'
                    )}>
                    {opt}
                  </button>
                ))}
              </div>
            )}
            {q.questionType === 'card_open' && !(q.connectorType || q.cardType || q.bindingTarget || q.backendKind) && (
              <p className="text-xs text-tx-4">Open the matching setup card in settings, then come back here to continue.</p>
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
