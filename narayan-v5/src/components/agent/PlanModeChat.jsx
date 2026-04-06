import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import {
  Loader2, Send, CheckCircle2, Sparkles, X, ArrowRight,
  Bot, User, AlertCircle, Search, Zap, Paperclip, FileText, Trash2,
  ListChecks, ShieldCheck, BookOpen, ChevronRight,
} from 'lucide-react';
import { planMode as planModeApi } from '../../api';
import {
  ConnectorSetupModal,
  extractConnectorIdsFromText,
} from '../connectors/ConnectorSetupModal';
import DatabaseConnectionCard from '../cards/DatabaseConnectionCard';
import CustomConnectionCard from '../cards/CustomConnectionCard';

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

function phaseLabel(phase) {
  if (!phase) return 'Unknown';
  return PHASE_LABELS[phase] || phase.replace(/_/g, ' ');
}

function formatModeLabel(value, fallback = 'n/a') {
  if (!value) return fallback;
  return String(value)
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/\b\w/g, c => c.toUpperCase());
}

function stripOptionMarkers(line) {
  return String(line || '')
    .replace(/^\s*([A-Z])\)\s*/i, '')
    .replace(/^\s*\d+[.)]\s*/i, '')
    .replace(/^\s*[-*•]\s*/i, '')
    .replace(/\(recommended\)/ig, '')
    .trim();
}

function parseStructuredQuestion(question) {
  const text = String(question || '').trim();
  if (!text) return { prompt: '', options: [], preview: '' };

  const lines = text.split('\n').map(line => line.trim()).filter(Boolean);
  const options = [];
  const promptLines = [];
  let preview = '';

  for (const line of lines) {
    if (/^(preview|example)\s*:/i.test(line)) {
      preview = line.replace(/^(preview|example)\s*:\s*/i, '').trim();
      continue;
    }

    const optionMatch = line.match(/^([A-Z])\)\s*(.+)$/i)
      || line.match(/^(\d+)[.)]\s*(.+)$/)
      || line.match(/^[-*•]\s*(.+)$/);

    if (optionMatch) {
      const raw = optionMatch[2] || optionMatch[1] || '';
      const label = stripOptionMarkers(raw);
      if (label) {
        const value = optionMatch[1] && /^[A-Z]$/i.test(optionMatch[1])
          ? optionMatch[1].toUpperCase()
          : label;
        options.push({
          value,
          label,
          recommended: /recommended/i.test(line),
        });
      }
      continue;
    }

    promptLines.push(line);
  }

  if (options.length > 1) {
    options.sort((a, b) => Number(b.recommended) - Number(a.recommended));
  }

  if (options.length === 0) {
    const slashParts = promptLines.join(' ').split(' / ').map(stripOptionMarkers).filter(Boolean);
    if (slashParts.length >= 2 && slashParts.length <= 6) {
      slashParts.forEach((part, idx) => {
        options.push({
          value: part,
          label: part,
          recommended: idx === 0 && /recommended/i.test(text),
        });
      });
    }
  }

  return {
    prompt: promptLines.join(' ').trim() || text,
    options,
    preview,
  };
}

function normalizeQuestionType(step, parsedOptions = []) {
  const rawType = String(step?.question_type || step?.questionType || step?.type || '').trim().toLowerCase();
  const hasOptions = Array.isArray(parsedOptions) && parsedOptions.length > 0;

  if (rawType.includes('card_open') || rawType === 'card-open') return 'card_open';
  if (rawType.includes('multi_select') || rawType.includes('multiselect')) return 'multi_select';
  if (rawType.includes('hybrid')) return 'hybrid';
  if (rawType.includes('approval') || rawType.includes('decision')) {
    return hasOptions || step?.multi_select || step?.multiSelect
      ? (step?.multi_select || step?.multiSelect ? 'multi_select' : 'mcq')
      : 'text';
  }
  if (rawType.includes('text')) return 'text';
  if (rawType.includes('mcq') || rawType.includes('choice') || rawType.includes('select')) {
    return step?.multi_select || step?.multiSelect ? 'multi_select' : 'mcq';
  }

  if (step?.multi_select || step?.multiSelect) return 'multi_select';
  if (hasOptions) return 'mcq';
  return 'text';
}

function formatQuestionMode(mode) {
  if (!mode) return 'text';
  return String(mode).replace(/_/g, ' ');
}

function extractResearchMemo(snapshot) {
  return snapshot?.intent_cache?._adaptive_research_memo || null;
}

function extractCompiledWorkflow(snapshot) {
  const draftRole = snapshot?.draft_role || null;
  return snapshot?.compiled_workflow
    || draftRole?.compiled_workflow
    || draftRole?.execution_guidelines?.compiled_workflow
    || null;
}

function buildRuntimePolicyText(role) {
  if (!role?.execution_guidelines) return '';
  const executionStrategy = formatModeLabel(role.execution_guidelines.execution_strategy);
  const toolPool = formatModeLabel(role.execution_guidelines.tool_pool);
  const permissionMode = formatModeLabel(role.execution_guidelines.permission_mode);
  return `execution: ${executionStrategy} | tool pool: ${toolPool} | permission: ${permissionMode}`;
}

function PlanQuestionCard({ step, onChoose, onCustomAnswer, onOpenSetup }) {
  if (!step?.question) return null;
  const parsed = parseStructuredQuestion(step.question);
  const explicitOptions = Array.isArray(step?.options)
    ? step.options
        .map(opt => typeof opt === 'string' ? { value: opt, label: opt } : {
          value: opt?.value || opt?.label || '',
          label: opt?.label || opt?.value || '',
          recommended: Boolean(opt?.recommended),
        })
        .filter(opt => opt.label)
    : [];
  const resolvedOptions = explicitOptions.length > 0 ? explicitOptions : parsed.options;
  const mode = normalizeQuestionType(step, resolvedOptions);
  const isCardOpen = mode === 'card_open';
  const isMultiSelect = mode === 'multi_select';
  const isHybrid = mode === 'hybrid';
  const [selectedOptions, setSelectedOptions] = useState([]);
  const [customAnswer, setCustomAnswer] = useState('');

  useEffect(() => {
    setSelectedOptions([]);
    setCustomAnswer('');
  }, [step?.id, step?.question]);

  function handleOptionClick(opt) {
    const value = opt.value || opt.label;
    if (isMultiSelect) {
      setSelectedOptions(prev => (
        prev.includes(value) ? prev.filter(item => item !== value) : [...prev, value]
      ));
      return;
    }
    onChoose?.(value, opt.label);
  }

  function submitMultiSelect() {
    if (selectedOptions.length === 0) return;
    const joined = selectedOptions.join(', ');
    onChoose?.(joined, joined);
  }

  function submitCustomAnswer() {
    const value = customAnswer.trim();
    if (!value) return;
    onChoose?.(value, value);
  }

  const showCustomInput = !isCardOpen && (mode === 'text' || isHybrid || (!resolvedOptions.length && !isMultiSelect));

  return (
    <div className="rounded-xl border border-accent/20 bg-accent-soft/10 px-4 py-3 space-y-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-[10px] font-semibold uppercase tracking-wide text-accent">Current question</p>
          <p className="mt-1 text-sm font-medium text-tx-1 whitespace-pre-wrap">{parsed.prompt}</p>
          {step.hint ? <p className="mt-1 text-[11px] text-tx-4">Hint: {step.hint}</p> : null}
          {parsed.preview ? <p className="mt-1 text-[11px] text-tx-3">Preview: {parsed.preview}</p> : null}
        </div>
        <div className="shrink-0 flex flex-col items-end gap-1">
          <span className="badge bg-bg-card text-tx-3 border border-border text-[10px]">
            {step.required === false ? 'Optional' : 'Required'}
          </span>
          <span className="badge bg-accent-soft text-accent border border-accent/20 text-[10px]">
            {formatQuestionMode(mode)}
          </span>
          {step.field?.field_type ? (
            <span className="text-[10px] text-tx-4 uppercase tracking-wide">
              {step.field.field_type.replace(/_/g, ' ')}
            </span>
          ) : null}
        </div>
      </div>

      {isCardOpen && (
        <div className="rounded-lg border border-border bg-bg-card px-3 py-2 space-y-2">
          <p className="text-xs text-tx-2">
            This is a setup question. Open the matching frontend card, finish the setup, and plan mode can resume.
          </p>
          <div className="flex flex-wrap items-center gap-2 text-[11px] text-tx-4">
            {(step.card_type || step.cardType) && <span>Card: {String(step.card_type || step.cardType).replace(/_/g, ' ')}</span>}
            {(step.binding_target || step.bindingTarget) && <span>Binding target: {step.binding_target || step.bindingTarget}</span>}
            {(step.connector_type || step.connectorType) && <span>Connector: {step.connector_type || step.connectorType}</span>}
          </div>
          {onOpenSetup && (
            <button
              type="button"
              onClick={() => onOpenSetup(step)}
              className="inline-flex items-center gap-1.5 text-xs font-medium text-accent hover:text-accent-text transition-colors"
            >
              {step.action_label || step.actionLabel || 'Open setup card'} <ChevronRight size={11} />
            </button>
          )}
        </div>
      )}

      {!isCardOpen && resolvedOptions.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {resolvedOptions.map(opt => (
            <button
              key={`${step.id}-${opt.value}`}
              type="button"
              onClick={() => handleOptionClick(opt)}
              className={clsx(
                'inline-flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs transition-colors',
                selectedOptions.includes(opt.value || opt.label)
                  ? 'border-accent bg-accent text-white hover:bg-accent-text'
                  : opt.recommended
                    ? 'border-accent bg-accent text-white hover:bg-accent-text'
                    : 'border-border bg-bg-card text-tx-2 hover:border-accent/40 hover:text-accent',
              )}
            >
              {(selectedOptions.includes(opt.value || opt.label) || opt.recommended) ? <ShieldCheck size={11} /> : null}
              <span>{opt.label}</span>
            </button>
          ))}
        </div>
      )}

      {isMultiSelect && selectedOptions.length > 0 && (
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={submitMultiSelect}
            className="inline-flex items-center gap-1.5 rounded-full border border-accent bg-accent px-3 py-1.5 text-xs font-medium text-white hover:bg-accent-text transition-colors"
          >
            Submit selected choices <ChevronRight size={11} />
          </button>
        </div>
      )}

      {showCustomInput && (
        <div className="space-y-2">
          <input
            value={customAnswer}
            onChange={e => setCustomAnswer(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') submitCustomAnswer(); }}
            placeholder={step.placeholder || 'Type a custom answer...'}
            className="input-field"
          />
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={submitCustomAnswer}
              className="inline-flex items-center gap-1.5 text-xs font-medium text-accent hover:text-accent-text transition-colors"
            >
              Send custom answer <ChevronRight size={11} />
            </button>
            {onCustomAnswer && (
              <button
                type="button"
                onClick={onCustomAnswer}
                className="inline-flex items-center gap-1.5 text-xs font-medium text-tx-4 hover:text-tx-2 transition-colors"
              >
                Focus composer
              </button>
            )}
          </div>
        </div>
      )}

      {!isCardOpen && !showCustomInput && onCustomAnswer && (
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onCustomAnswer}
            className="inline-flex items-center gap-1.5 text-xs font-medium text-accent hover:text-accent-text transition-colors"
          >
            Type a custom answer <ChevronRight size={11} />
          </button>
        </div>
      )}
    </div>
  );
}

function PlanInspector({ snapshot, currentQuestion, selectedTemplate, testResult, phase }) {
  const draftRole = snapshot?.draft_role || null;
  const roleName = draftRole?.name || snapshot?.draft_agent?.name || selectedTemplate?.name || 'New agent';
  const executionGuidelines = draftRole?.execution_guidelines || {};
  const compiledWorkflow = extractCompiledWorkflow(snapshot);
  const workflowSteps = Array.isArray(compiledWorkflow?.steps)
    ? compiledWorkflow.steps
    : Array.isArray(snapshot?.intent_cache?.workflow_dsl)
      ? snapshot.intent_cache.workflow_dsl
      : [];
  const connectors = Array.isArray(draftRole?.connectors) ? draftRole.connectors : [];
  const memo = extractResearchMemo(snapshot);
  const pendingSteps = Array.isArray(snapshot?.pending_steps) ? snapshot.pending_steps : [];
  const currentPhase = phaseLabel(snapshot?.phase || phase);
  const currentQuestionParsed = currentQuestion ? parseStructuredQuestion(currentQuestion.question) : { options: [] };
  const currentQuestionOptions = Array.isArray(currentQuestion?.options) && currentQuestion.options.length > 0
    ? currentQuestion.options
    : currentQuestionParsed.options;
  const currentQuestionMode = normalizeQuestionType(currentQuestion, currentQuestionOptions);
  const runtimePolicy = buildRuntimePolicyText(draftRole);
  const toolPool = formatModeLabel(executionGuidelines.tool_pool);
  const permissionMode = formatModeLabel(executionGuidelines.permission_mode);
  const strategy = formatModeLabel(executionGuidelines.execution_strategy);
  const workflowVersion = compiledWorkflow?.workflow_version || compiledWorkflow?.version || snapshot?.workflow_version || '';
  const runtimeVersion = compiledWorkflow?.runtime_version || '';
  const recompileMode = formatModeLabel(compiledWorkflow?.recompile_policy?.mode);
  const variantMode = compiledWorkflow?.variant_policy?.fallback
    ? formatModeLabel(compiledWorkflow.variant_policy.fallback)
    : '';
  const llmWorkerSteps = workflowSteps.filter(step =>
    step?.tool === 'llm_worker'
    || step?.dsl_type === 'llm_worker'
    || step?.type === 'llm_worker'
    || step?.llm_role
    || step?.llm_generation?.role
  ).length;

  function stepLabel(step, idx) {
    const base = step?.description || step?.summary || step?.name || step?.instruction || step?.prompt || step?.id || `Step ${idx + 1}`;
    const type = step?.dsl_type || step?.type || '';
    const tool = step?.tool || '';
    const llmRole = step?.llm_role || step?.llm_generation?.role || '';
    const executionIntent = step?.execution_intent || step?.llm_generation?.execution_intent || '';
    const budgetTier = step?.budget_tier || step?.llm_generation?.budget_tier || '';
    const parts = [base];
    if (type && type !== base) parts.push(type.replace(/_/g, ' '));
    if (tool) parts.push(tool);
    if (llmRole) parts.push(`role: ${formatModeLabel(llmRole)}`);
    if (executionIntent) parts.push(`intent: ${formatModeLabel(executionIntent)}`);
    if (budgetTier) parts.push(`budget: ${formatModeLabel(budgetTier)}`);
    return parts.join(' · ');
  }

  return (
    <div className="space-y-3 rounded-xl border border-border bg-bg-card/70 px-3 py-3">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Compiled workflow</p>
          <p className="text-sm font-semibold text-tx-1 truncate">{roleName}</p>
        </div>
        <span className="badge bg-accent-soft text-accent border border-accent/20 text-[10px]">
          {currentPhase}
        </span>
      </div>

      <div className="grid grid-cols-2 gap-2 text-[11px] md:grid-cols-4">
        <div className="rounded-lg border border-border bg-bg px-2.5 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Strategy</p>
          <p className="mt-1 text-tx-1 capitalize">{strategy || 'n/a'}</p>
        </div>
        <div className="rounded-lg border border-border bg-bg px-2.5 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Tool pool</p>
          <p className="mt-1 text-tx-1 capitalize">{toolPool || 'n/a'}</p>
        </div>
        <div className="rounded-lg border border-border bg-bg px-2.5 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Permission</p>
          <p className="mt-1 text-tx-1 capitalize">{permissionMode || 'n/a'}</p>
        </div>
        <div className="rounded-lg border border-border bg-bg px-2.5 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Workflow</p>
          <p className="mt-1 text-tx-1 capitalize">{workflowVersion || 'v1'}</p>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2 text-[11px] md:grid-cols-3">
        <div className="rounded-lg border border-border bg-bg px-2.5 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Runtime</p>
          <p className="mt-1 text-tx-1 capitalize">{runtimeVersion || 'v1'}</p>
        </div>
        <div className="rounded-lg border border-border bg-bg px-2.5 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">LLM workers</p>
          <p className="mt-1 text-tx-1 capitalize">{llmWorkerSteps}</p>
        </div>
        <div className="rounded-lg border border-border bg-bg px-2.5 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Recompile</p>
          <p className="mt-1 text-tx-1 capitalize">{recompileMode || variantMode || 'fork on structural failure'}</p>
        </div>
      </div>

      {runtimePolicy && (
        <div className="rounded-lg border border-border bg-bg px-3 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Runtime policy</p>
          <p className="mt-1 text-[11px] leading-relaxed text-tx-2 whitespace-pre-wrap">{runtimePolicy}</p>
        </div>
      )}

      {currentQuestion && (
        <div className="rounded-lg border border-border bg-bg px-3 py-2 space-y-1.5">
          <div className="flex items-center gap-2">
            <ListChecks size={12} className="text-accent" />
            <p className="text-[10px] uppercase tracking-wide text-tx-4">Current step</p>
          </div>
          <p className="text-[11px] text-tx-2 leading-relaxed">{currentQuestion.question}</p>
          <div className="flex flex-wrap items-center gap-2 text-[10px] text-tx-4">
            <span className="badge bg-bg-card text-tx-3 border border-border text-[10px]">
              {formatQuestionMode(currentQuestionMode)}
            </span>
            {(currentQuestion.card_type || currentQuestion.cardType) && (
              <span>Card: {String(currentQuestion.card_type || currentQuestion.cardType).replace(/_/g, ' ')}</span>
            )}
            {(currentQuestion.binding_target || currentQuestion.bindingTarget) && (
              <span>Binding target: {currentQuestion.binding_target || currentQuestion.bindingTarget}</span>
            )}
          </div>
        </div>
      )}

      {(workflowSteps.length > 0 || pendingSteps.length > 0) && (
        <div className="rounded-lg border border-border bg-bg px-3 py-2 space-y-2">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <BookOpen size={12} className="text-accent" />
              <p className="text-[10px] uppercase tracking-wide text-tx-4">Compiled steps</p>
            </div>
            <span className="text-[10px] text-tx-4">
              {workflowSteps.length} step{workflowSteps.length === 1 ? '' : 's'}
            </span>
          </div>
          <div className="space-y-1">
            {workflowSteps.slice(0, 5).map((step, idx) => (
              <div key={`${step?.name || step?.description || idx}-${idx}`} className="flex items-start gap-2 text-[11px] text-tx-2">
                <span className="mt-1 size-1.5 rounded-full bg-accent shrink-0" />
                <span className="leading-relaxed">{stepLabel(step, idx)}</span>
              </div>
            ))}
            {pendingSteps.length > 0 && (
              <p className="text-[10px] text-tx-4">
                {pendingSteps.length} question{pendingSteps.length === 1 ? '' : 's'} still pending.
              </p>
            )}
          </div>
        </div>
      )}

      {memo && (
        <div className="rounded-lg border border-border bg-bg px-3 py-2 space-y-1.5">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Research memo</p>
          {memo.summary ? <p className="text-[11px] text-tx-2 leading-relaxed whitespace-pre-wrap">{memo.summary}</p> : null}
          {Array.isArray(memo.findings) && memo.findings.length > 0 && (
            <p className="text-[10px] text-tx-4 leading-relaxed">
              Findings: {memo.findings.slice(0, 3).join(' | ')}
            </p>
          )}
          {Array.isArray(memo.risks) && memo.risks.length > 0 && (
            <p className="text-[10px] text-tx-4 leading-relaxed">
              Risks: {memo.risks.slice(0, 3).join(' | ')}
            </p>
          )}
        </div>
      )}

      {connectors.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {connectors.slice(0, 5).map(connector => (
            <span key={connector} className="badge bg-bg text-tx-3 border border-border text-[10px]">
              {connector}
            </span>
          ))}
        </div>
      )}

      {testResult && (
        <div className="rounded-lg border border-border bg-bg px-3 py-2">
          <p className="text-[10px] uppercase tracking-wide text-tx-4">Latest test</p>
          <p className="mt-1 text-[11px] text-tx-2 leading-relaxed">
            {String(testResult.status || 'partial').replace(/_/g, ' ')} · {String(testResult.confidence || 'partial').replace(/_/g, ' ')} confidence
          </p>
        </div>
      )}
    </div>
  );
}

function TestResultPanel({ result, onRevise, revising = false }) {
  if (!result) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-bg px-3 py-3 text-xs text-tx-3">
        Run the deterministic test to validate the compiled workflow before saving.
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
        <div className="mb-3">
          <p className="text-[10px] font-semibold uppercase tracking-[0.24em] text-accent">Start here</p>
          <p className="mt-1 text-sm leading-6 text-tx-3">
            Pick a pattern, then we'll turn it into a structured plan you can test and save.
          </p>
        </div>
        <div className="relative">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-tx-4" />
          <input
            type="text"
            placeholder="Search by job, team, or system..."
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
            <p className="text-xs text-tx-4 mt-1">Try a broader search or start from scratch.</p>
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
                    <p className="mt-2 text-[10px] uppercase tracking-[0.18em] text-tx-4">
                      {t.category || 'Workflow template'}
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
          Start from scratch
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
export default function PlanModeChat({
  agentName = 'New Agent',
  existingAgentId = null,
  onComplete,
  onCancel,
  onNavigateSettings = null,
  presentation = 'modal',
}) {
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
  const [sessionSnapshot, setSessionSnapshot] = useState(null);
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
  const [inlineConnectorIds, setInlineConnectorIds] = useState([]);
  const [inlineConnectionKinds, setInlineConnectionKinds] = useState([]);
  const [showDatabaseCard, setShowDatabaseCard] = useState(false);
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
  const loadSessionSnapshot = useCallback(async (targetSessionId) => {
    if (!targetSessionId) return null;
    try {
      const snapshot = await planModeApi.get(targetSessionId);
      setSessionSnapshot(snapshot);
      return snapshot;
    } catch (e) {
      console.warn('Failed to load plan mode session snapshot:', e);
      return null;
    }
  }, []);

  const routeSetupRequest = useCallback((request = {}) => {
    const cardType = String(request.cardType || request.card_type || '').toLowerCase().trim();
    const bindingTarget = String(request.bindingTarget || request.binding_target || '').toLowerCase().trim();
    const connectorType = String(request.connectorType || request.connector_type || '').toLowerCase().trim();
    const requiredFields = Array.isArray(request.requiredFields || request.required_fields)
      ? (request.requiredFields || request.required_fields)
      : [];
    const requiredConnectors = [
      ...(Array.isArray(request.requiredConnectors || request.required_connectors) ? (request.requiredConnectors || request.required_connectors) : []),
      ...(connectorType ? [connectorType] : []),
    ].filter(Boolean);
    const uniqueConnectors = [...new Set(requiredConnectors)];

    const routeToSettings = () => {
      if (typeof onNavigateSettings === 'function') {
        onNavigateSettings(request);
        return true;
      }
      return false;
    };

    if (cardType === 'database' || bindingTarget === 'database' || request.needs_database_connection) {
      setShowDatabaseCard(true);
      setInlineConnectionKinds([]);
      setInlineConnectorIds([]);
      setShowConnectorModal(false);
      return true;
    }

    if (cardType === 'mcp' || bindingTarget === 'mcp') {
      setShowDatabaseCard(false);
      setShowConnectorModal(false);
      setInlineConnectionKinds(prev => [...new Set([...prev, 'mcp'])]);
      return true;
    }

    if (cardType === 'api_auth' || cardType === 'api' || bindingTarget === 'api') {
      setShowDatabaseCard(false);
      setShowConnectorModal(false);
      setInlineConnectionKinds(prev => [...new Set([...prev, 'api'])]);
      return true;
    }

    if (cardType === 'connector' || uniqueConnectors.length > 0 || connectorType) {
      setShowDatabaseCard(false);
      setInlineConnectorIds(uniqueConnectors);
      setShowConnectorModal(uniqueConnectors.length > 0);
      return uniqueConnectors.length > 0 || routeToSettings();
    }

    if (requiredFields.length > 0 && routeToSettings()) {
      return true;
    }

    return routeToSettings();
  }, [onNavigateSettings]);

  function syncInlineSetupFromTurn(turn = {}) {
    const reply = String(turn.reply || turn.message || '');
    const lower = reply.toLowerCase();
    const inlineSetup = turn.inline_setup || {};
    const connectionKinds = [
      ...(Array.isArray(inlineSetup.connection_kinds) ? inlineSetup.connection_kinds : []),
      ...(Array.isArray(turn.connection_kinds) ? turn.connection_kinds : []),
    ];
    const requiredConnectors = [
      ...(Array.isArray(inlineSetup.required_connectors) ? inlineSetup.required_connectors : []),
      ...(Array.isArray(turn.required_connectors) ? turn.required_connectors : []),
      ...extractConnectorIdsFromText(reply),
    ];
    const uniqueConnectors = [...new Set(requiredConnectors.filter(Boolean))];
    const uniqueKinds = [...new Set(connectionKinds.filter(kind => ['db', 'api', 'mcp'].includes(kind)))];
    const setupRequest = {
      cardType: inlineSetup.card_type || inlineSetup.cardType || turn.card_type || turn.cardType || '',
      bindingTarget: inlineSetup.binding_target || inlineSetup.bindingTarget || turn.binding_target || turn.bindingTarget || '',
      connectorType: inlineSetup.connector_type || inlineSetup.connectorType || turn.connector_type || turn.connectorType || '',
      requiredFields: inlineSetup.required_fields || inlineSetup.requiredFields || turn.required_fields || turn.requiredFields || [],
      resumeToken: inlineSetup.resume_token || inlineSetup.resumeToken || turn.resume_token || turn.resumeToken || '',
      requiredConnectors: uniqueConnectors,
      needs_database_connection: Boolean(inlineSetup.needs_database_connection ?? turn.needs_database_connection),
    };
    const pendingInlineSetup =
      typeof inlineSetup.pending === 'boolean'
        ? inlineSetup.pending
        : typeof turn.inline_setup_pending === 'boolean'
          ? turn.inline_setup_pending
          : (
            uniqueConnectors.length > 0
            || uniqueKinds.length > 0
            || Boolean(setupRequest.cardType || setupRequest.bindingTarget || setupRequest.connectorType || setupRequest.requiredFields.length > 0)
            || Boolean(inlineSetup.needs_database_connection ?? turn.needs_database_connection)
            || lower.includes('database connection')
            || lower.includes('inline connection card')
            || lower.includes('database card')
            || lower.includes('connect your db')
            || lower.includes('database using the inline connection card')
          );

    if (!pendingInlineSetup) {
      setInlineConnectorIds([]);
      setInlineConnectionKinds([]);
      setShowDatabaseCard(false);
      setShowConnectorModal(false);
      return;
    }

    setInlineConnectorIds(uniqueConnectors);
    setInlineConnectionKinds(uniqueKinds);
    routeSetupRequest(setupRequest);
  }

  const startSession = useCallback(async (template = null, agentIdOverride = null) => {
    setLoading(true);
    setError('');
    try {
      const res = await planModeApi.start(agentName, agentIdOverride ?? activeAgentId, template?.id || null);
      setSessionId(res.session_id);
      setActiveAgentId(res.agent_id || agentIdOverride || activeAgentId);
      setMessages([{ role: 'assistant', content: res.message || 'What should this agent do?', isNew: true }]);
      setPhase(res.phase || 'capturing_intent');
      syncInlineSetupFromTurn(res);
      setTestResult(null);
      setSelectedTemplate(template);
      setSessionSnapshot(null);
      setStep('chat');
      await loadSessionSnapshot(res.session_id);
    } catch (e) {
      setError(e.message || 'Failed to start session');
    } finally {
      setLoading(false);
    }
  }, [agentName, activeAgentId, loadSessionSnapshot, syncInlineSetupFromTurn]);

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
      appendAssistantReply(res);
      setPendingAttachments([]);
      setTestResult(null);
      await loadSessionSnapshot(sessionId);

      if (res.complete || res.phase === 'complete') {
        setComplete(true);
      }
    } catch (e) {
      setError(e.message || 'Something went wrong. Try again.');
      // Remove user bubble on error so they can retry
      setMessages(prev => prev.slice(0, -1));
    } finally {
      setSending(false);
    }
  }, [sessionId, sending, pendingAttachments, attachmentsBusy, appendAssistantReply, loadSessionSnapshot]);

  // ── Save and deploy ────────────────────────────────────────────────────
  function appendAssistantReply(turn) {
    const reply = String(turn?.reply || turn?.message || '');
    const phaseToSet = turn?.phase || phase;
    setPhase(phaseToSet);
    setMessages(prev => [...prev, { role: 'assistant', content: reply, isNew: true }]);
    syncInlineSetupFromTurn(turn);
    setTestResult(null);
    if (phaseToSet === 'complete') {
      setComplete(true);
    }
  }

  const resumeWithInlineSetup = useCallback(async (answer, userLabel = answer) => {
    if (!sessionId || sending) return;
    setError('');
    setSending(true);
    setMessages(prev => [...prev, {
      role: 'user',
      content: `Connected ${userLabel}`,
      isNew: true,
    }]);

    try {
      const res = await planModeApi.turn(sessionId, answer);
      appendAssistantReply(res);
      setPendingAttachments([]);
      await loadSessionSnapshot(sessionId);
      if (res.complete || res.phase === 'complete') {
        setComplete(true);
      }
    } catch (e) {
      setError(e.message || 'Something went wrong. Try again.');
      setMessages(prev => prev.slice(0, -1));
    } finally {
      setSending(false);
    }
  }, [sessionId, sending, appendAssistantReply, loadSessionSnapshot]);

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
      appendAssistantReply(res);
      setTestResult(null);
      setComplete((res.phase || phase) === 'complete');
      await loadSessionSnapshot(sessionId);
    } catch (e) {
      setError(e.message || 'Failed to revise plan');
      setMessages(prev => prev.slice(0, -1));
    } finally {
      setRevising(false);
    }
  }, [sessionId, revising, testResult, phase, appendAssistantReply, loadSessionSnapshot]);

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
      await loadSessionSnapshot(sessionId);
    } catch (e) {
      setError(e.message || 'Failed to save agent');
    } finally {
      setSaving(false);
    }
  }, [sessionId, testResult, phase, requiredConnectors, connectorVerified, continueOrComplete, loadSessionSnapshot]);

  // ── Keyboard submit ────────────────────────────────────────────────────
  function onKeyDown(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      if (input.trim() || pendingAttachments.length > 0) {
        e.preventDefault();
        sendMessage(input);
      }
    }
  }

  const pendingSteps = useMemo(() => Array.isArray(sessionSnapshot?.pending_steps) ? sessionSnapshot.pending_steps : [], [sessionSnapshot]);
  const currentQuestion = pendingSteps[0] || null;
  const workflowSteps = useMemo(() => {
    const compiledWorkflow = extractCompiledWorkflow(sessionSnapshot);
    if (Array.isArray(compiledWorkflow?.steps)) return compiledWorkflow.steps;
    return Array.isArray(sessionSnapshot?.intent_cache?.workflow_dsl) ? sessionSnapshot.intent_cache.workflow_dsl : [];
  }, [sessionSnapshot]);
  const unresolvedDatabaseSelection = useMemo(() => {
    const intentDb = sessionSnapshot?.intent_cache?.uses_external_db;
    const draftTools = Array.isArray(sessionSnapshot?.draft_role?.tools) ? sessionSnapshot.draft_role.tools : [];
    return intentDb === 'external_db'
      || draftTools.includes('external_db:external_db')
      || draftTools.includes('external_db:true');
  }, [sessionSnapshot]);

  const sendChoice = useCallback((choice) => {
    if (!choice || sending || loading || complete) return;
    sendMessage(String(choice));
  }, [sendMessage, sending, loading, complete]);

  const focusComposer = useCallback(() => {
    inputRef.current?.focus();
  }, []);

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
          .then(() => loadSessionSnapshot(sessionId))
          .catch(e => {
            setError(e.message || 'Failed to save agent');
          })
          .finally(() => setSaving(false));
      }, 300);
    }
  };

  // ─────────────────────────────────────────────────────────────────────
  const isDrawer = presentation === 'drawer';
  const isInline = presentation === 'inline';

  return (
    <div className={clsx(
      isInline
        ? 'relative h-full w-full flex'
        : 'fixed inset-0 z-50 flex',
      isDrawer
        ? 'items-stretch justify-end bg-transparent'
        : isInline
          ? 'items-stretch justify-end'
          : 'items-center justify-center bg-tx-1/40 backdrop-blur-sm'
    )}>
      <motion.div
        className={clsx(
          'relative flex flex-col border border-border bg-bg shadow-md overflow-hidden',
          isInline
            ? 'h-full w-full rounded-none border-l shadow-2xl'
            : isDrawer
            ? 'h-full w-full max-w-[44rem] rounded-none border-l shadow-2xl'
            : 'w-full max-w-3xl mx-4 h-[85vh] rounded-2xl'
        )}
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
                  ? 'Select a starting pattern, or skip to build from a blank slate'
                  : (isAddingRole
                      ? 'Describe what this role should do in plain language'
                      : 'Describe what this agent does and we’ll structure the workflow')}
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
                  {(sessionSnapshot || currentQuestion || workflowSteps.length > 0) && (
                    <PlanInspector
                      snapshot={sessionSnapshot}
                      currentQuestion={currentQuestion}
                      selectedTemplate={selectedTemplate}
                      testResult={testResult}
                      phase={phase}
                    />
                  )}
                  {(showDatabaseCard || unresolvedDatabaseSelection) && (
                    <DatabaseConnectionCard
                      onConnected={({ name }) => resumeWithInlineSetup(name, name)}
                    />
                  )}
                  {inlineConnectionKinds.includes('mcp') && (
                    <CustomConnectionCard
                      kind="mcp"
                      onConnected={({ name }) => resumeWithInlineSetup(name, name)}
                    />
                  )}
                  {inlineConnectionKinds.includes('api') && (
                    <CustomConnectionCard
                      kind="api"
                      onConnected={({ name }) => resumeWithInlineSetup(name, name)}
                    />
                  )}
                  {inlineConnectorIds.length > 0 && (
                    <ConnectorSetupModal
                      requiredConnectors={inlineConnectorIds}
                      onVerified={() => resumeWithInlineSetup(inlineConnectorIds.join(', '), inlineConnectorIds.join(', '))}
                      mode="inline"
                    />
                  )}
                  {currentQuestion && (
                    <PlanQuestionCard
                      step={currentQuestion}
                      onChoose={sendChoice}
                      onCustomAnswer={focusComposer}
                      onOpenSetup={routeSetupRequest}
                    />
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
                      placeholder={loading
                        ? 'Starting…'
                        : attachmentsBusy
                          ? 'Reading files…'
                          : currentQuestion
                            ? 'Type a custom answer or pick an option above'
                            : pendingAttachments.length > 0
                              ? 'Add a note or press Enter to send files'
                              : 'Reply…'}
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
                  {currentQuestion
                    ? 'Pick an option above or press Enter to send your custom answer'
                    : 'Enter to send | Shift+Enter for new line | Attach PDFs, spreadsheets, CSVs, and docs'}
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
