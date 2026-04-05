import { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import {
  Layers, CheckCircle2, RotateCcw, XCircle, AlertTriangle,
  Loader2, ArrowRight, Play, Key, HelpCircle, ExternalLink,
} from 'lucide-react';
import { agents, credentials as credentialsApi, connectors as connectorsApi } from '../../api';

const MAX_REJECTIONS = 3;

// Known built-in connectors — mirrors connector_tool::ALL_CONNECTORS on the backend.
// Used to decide whether to offer inline key entry vs "we don't use this tool" path.
// Mirrors connector_tool::ALL_CONNECTORS on the backend.
// When you add a connector there, add it here too.
const KNOWN_CONNECTORS = new Set([
  // CRM
  'salesforce', 'hubspot',
  // Support
  'zendesk', 'intercom', 'freshdesk',
  // Dev tools
  'github',
  // Project management
  'jira', 'notion', 'asana', 'linear', 'monday',
  // Communication
  'slack', 'gmail', 'outlook',
  // Finance
  'quickbooks', 'stripe',
  // ITSM
  'servicenow', 'pagerduty',
  // HR
  'greenhouse',
  // Legal
  'docusign',
  // Data
  'dbt_cloud',
]);

const CONNECTOR_LABELS = {
  salesforce: 'Salesforce', hubspot:    'HubSpot',
  zendesk:    'Zendesk',    intercom:   'Intercom',    freshdesk: 'Freshdesk',
  github:     'GitHub',
  jira:       'Jira',       notion:     'Notion',      asana:     'Asana',
  linear:     'Linear',     monday:     'monday.com',
  slack:      'Slack',      gmail:      'Gmail',        outlook:   'Outlook',
  quickbooks: 'QuickBooks', stripe:     'Stripe',
  servicenow: 'ServiceNow', pagerduty:  'PagerDuty',
  greenhouse: 'Greenhouse',
  docusign:   'DocuSign',
  dbt_cloud:  'dbt Cloud',
};

const label = n => CONNECTOR_LABELS[n] || n;

function formatLabel(value) {
  return String(value || '')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/_/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

// ── Confidence dot ──────────────────────────────────────────────────────────
function ConfidenceDot({ colour }) {
  const cls = { green:'bg-ok', amber:'bg-warn', red:'bg-err' }[colour] || 'bg-tx-4';
  return <span className={clsx('inline-block size-2 rounded-full shrink-0 mt-1', cls)} title={colour} />;
}

// ── Step row ────────────────────────────────────────────────────────────────
function StepRow({ step, confidence }) {
  return (
    <div className="rounded-lg border border-border/60 bg-bg px-3 py-2.5 flex items-start gap-2.5">
      <ConfidenceDot colour={confidence || 'amber'} />
      <div className="flex-1 min-w-0">
        <p className="text-xs text-tx-1 leading-relaxed">{step.description}</p>
        {step.tool && (
          <span className="inline-block mt-1 font-mono text-[10px] text-tx-4 bg-bg-active rounded px-1.5 py-0.5">
            {step.tool}
          </span>
        )}
        {(step.llm_role || step.execution_intent || step.budget_tier || step.llm_generation) && (
          <div className="mt-1 flex flex-wrap gap-1">
            {(step.llm_role || step.llm_generation?.role) && <span className="badge bg-vio-soft text-vio border border-vio/20">{formatLabel(step.llm_role || step.llm_generation?.role)}</span>}
            {(step.execution_intent || step.llm_generation?.execution_intent) && <span className="badge bg-info-soft text-info border border-info/20">{formatLabel(step.execution_intent || step.llm_generation?.execution_intent)}</span>}
            {(step.budget_tier || step.llm_generation?.budget_tier) && <span className="badge bg-accent-soft text-accent border border-accent/20">{formatLabel(step.budget_tier || step.llm_generation?.budget_tier)}</span>}
          </div>
        )}
      </div>
    </div>
  );
}

// ── CredentialGap ────────────────────────────────────────────────────────────
// One missing credential, three modes:
//   choice  — "Do you have X?" → two buttons
//   connect — inline API key entry (for known connectors user claims to have)
//   wrong   — "What tool do you use?" (when user doesn't have this connector at all)
//
function CredentialGap({ name, onResolved, onWrongTool, onNavigateSettings }) {
  const [mode, setMode]       = useState('choice');
  const [apiKey, setApiKey]   = useState('');
  const [saving, setSaving]   = useState(false);
  const [saveErr, setSaveErr] = useState('');
  const [altTool, setAltTool] = useState('');

  async function handleSaveKey() {
    if (!apiKey.trim()) return;
    setSaving(true); setSaveErr('');
    try {
      try {
        await connectorsApi.installApiKey(name, apiKey.trim());
      } catch {
        await credentialsApi.set(name, apiKey.trim(), '', label(name));
      }
      onResolved(name);
    } catch (e) {
      setSaveErr(e.message || 'Failed to save. Check the key and try again.');
      setSaving(false);
    }
  }

  function handleWrongToolSubmit() {
    if (!altTool.trim()) return;
    onWrongTool(name, altTool.trim());
  }

  return (
    <div className="rounded-lg border border-warn/30 bg-warn-soft/20 overflow-hidden">

      {mode === 'choice' && (
        <div className="px-3 py-3 flex items-start gap-2.5">
          <AlertTriangle size={14} className="text-warn shrink-0 mt-0.5" />
          <div className="flex-1 space-y-2">
            <p className="text-xs font-medium text-tx-1">
              Step requires <span className="text-warn font-semibold">{label(name)}</span> credentials
            </p>
            <p className="text-[11px] text-tx-3">Do you use {label(name)}?</p>
            <div className="flex items-center gap-2 flex-wrap">
              <button
                onClick={() => setMode('connect')}
                className="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium
                           bg-accent text-white hover:bg-accent-text transition-colors"
              >
                <Key size={11} />
                Yes, connect it
              </button>
              <button
                onClick={() => setMode('wrong')}
                className="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium
                           border border-border text-tx-2 hover:border-border-md transition-colors"
              >
                <HelpCircle size={11} />
                No, we use something else
              </button>
            </div>
          </div>
        </div>
      )}

      {mode === 'connect' && (
        <div className="px-3 py-3 space-y-2.5">
          <div className="flex items-center gap-2">
            <Key size={13} className="text-accent shrink-0" />
            <p className="text-xs font-medium text-tx-1">Add your {label(name)} API key</p>
          </div>
          <input
            type="password"
            value={apiKey}
            onChange={e => setApiKey(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleSaveKey()}
            placeholder={`${label(name)} API key or access token`}
            className="input-field text-xs w-full"
            autoFocus
          />
          {saveErr && <p className="text-[11px] text-err">{saveErr}</p>}
          <div className="flex items-center gap-2 flex-wrap">
            <button
              onClick={handleSaveKey}
              disabled={!apiKey.trim() || saving}
              className="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium
                         bg-accent text-white hover:bg-accent-text disabled:opacity-40 transition-colors"
            >
              {saving ? <Loader2 size={11} className="animate-spin" /> : <CheckCircle2 size={11} />}
              Save &amp; continue
            </button>
            {onNavigateSettings && (
              <button
                onClick={onNavigateSettings}
                className="inline-flex items-center gap-1 text-xs text-tx-3 hover:text-tx-1 transition-colors"
              >
                Full settings <ExternalLink size={10} />
              </button>
            )}
            <button onClick={() => setMode('choice')} className="text-xs text-tx-4 hover:text-tx-2 transition-colors">
              Back
            </button>
          </div>
          <p className="text-[11px] text-tx-4">
            Stored securely and only used for this agent's steps.
          </p>
        </div>
      )}

      {mode === 'wrong' && (
        <div className="px-3 py-3 space-y-2.5">
          <div className="flex items-center gap-2">
            <HelpCircle size={13} className="text-info shrink-0" />
            <p className="text-xs font-medium text-tx-1">
              Which tool do you use instead of {label(name)}?
            </p>
          </div>
          <input
            type="text"
            value={altTool}
            onChange={e => setAltTool(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleWrongToolSubmit()}
            placeholder="e.g. Intercom, Freshdesk, HubSpot Service…"
            className="input-field text-xs w-full"
            autoFocus
          />
          <div className="flex items-center gap-2">
            <button
              onClick={handleWrongToolSubmit}
              disabled={!altTool.trim()}
              className="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium
                         bg-accent text-white hover:bg-accent-text disabled:opacity-40 transition-colors"
            >
              <RotateCcw size={11} />
              Update plan to use this
            </button>
            <button onClick={() => setMode('choice')} className="text-xs text-tx-4 hover:text-tx-2 transition-colors">
              Back
            </button>
          </div>
          <p className="text-[11px] text-tx-3">
            The plan will be revised to use {altTool || 'the right tool'}.
            If it also needs credentials you'll be prompted after replanning.
          </p>
        </div>
      )}

    </div>
  );
}

// ── Main component ───────────────────────────────────────────────────────────
export default function PlanApprovalCard({ agentId, plan, replanning, onDone, onNavigateSettings }) {
  const steps          = plan?.steps             || [];
  const missingCreds   = plan?.missingCredentials || [];
  const stepConfidence = plan?.stepConfidence     || [];
  const rejectionCount = plan?.rejectionCount     || 0;
  const rationale      = plan?.rationale          || '';
  const stepCount      = plan?.stepCount          || steps.length;
  const jobType        = plan?.jobType            || '';
  const runtimePolicy  = plan?.runtimePolicy      || plan?.runtime_policy || '';
  const researchSummary = plan?.researchSummary   || plan?.research_summary || '';
  const compilerStage = plan?.compilerStage || plan?.compiler_stage || '';
  const compilerRepairPasses = plan?.compilerRepairPasses ?? plan?.compiler_repair_passes ?? 0;
  const compilerValidationIssues = Array.isArray(plan?.compilerValidationIssues)
    ? plan.compilerValidationIssues
    : Array.isArray(plan?.compiler_validation_issues)
      ? plan.compiler_validation_issues
      : [];

  const [feedback, setFeedback]           = useState('');
  const [loading, setLoading]             = useState(null);
  const [confirmReject, setConfirmReject] = useState(false);
  const [credGaps, setCredGaps]           = useState(missingCreds);
  const [successState, setSuccessState]   = useState(null);
  const [error, setError]                 = useState('');
  const pollRef    = useRef(null);
  const mountedRef = useRef(true);

  useEffect(() => { setCredGaps(missingCreds); }, [missingCreds.join(',')]);

  // Background poll — catches keys added in Settings in another tab
  useEffect(() => {
    if (credGaps.length === 0) { clearInterval(pollRef.current); return; }
    clearInterval(pollRef.current);
    pollRef.current = setInterval(async () => {
      if (!mountedRef.current) return;
      try {
        const [cr, co] = await Promise.all([
          credentialsApi.list().catch(() => ({ credentials: [] })),
          connectorsApi.list().catch(() => ({ connectors: [] })),
        ]);
        const installed = [
          ...(cr.credentials || []).map(c => c.provider),
          ...(co.connectors  || []).map(c => c.connector_type),
        ];
        const still = credGaps.filter(g => !installed.includes(g));
        if (mountedRef.current) setCredGaps(still);
      } catch {}
    }, 5000);
    return () => clearInterval(pollRef.current);
  }, [credGaps.join(',')]);

  useEffect(() => () => { mountedRef.current = false; clearInterval(pollRef.current); }, []);

  useEffect(() => {
    if (steps.length === 0 && agentId) agents.get(agentId).catch(() => {});
  }, [agentId]);

  // ── Credential gap callbacks ───────────────────────────────────────────

  function handleCredResolved(name) {
    setCredGaps(g => g.filter(x => x !== name));
  }

  async function handleWrongTool(missingName, alternateTool) {
    const autoFeedback =
      `We don't use ${label(missingName)}. We use ${alternateTool} instead. ` +
      `Please update the plan to use ${alternateTool}.`;
    setFeedback(autoFeedback);
    setLoading('revise');
    setError('');
    try {
      await agents.approvePlan(agentId, false, autoFeedback, null, true);
      setSuccessState('replanning');
    } catch (e) {
      setError(e.message || 'Failed to request revision');
      setLoading(null);
    }
  }

  // ── Action handlers ────────────────────────────────────────────────────

  const canApprove = credGaps.length === 0 && !loading;

  const handleApprove = useCallback(async () => {
    if (!canApprove) return;
    setError(''); setLoading('approve');
    try {
      await agents.approvePlan(agentId, true, feedback);
      setSuccessState('approving');
      setTimeout(() => onDone?.(), 1500);
    } catch (e) {
      if (e.message?.includes('missing_credentials')) {
        try {
          const p = JSON.parse(e.message.replace(/^.*?(\{)/, '$1'));
          setCredGaps(p.missing || credGaps);
        } catch {}
        setError('Some required credentials are still missing.');
      } else {
        setError(e.message || 'Failed to approve plan');
      }
      setLoading(null);
    }
  }, [agentId, feedback, canApprove, onDone, credGaps]);

  const handleRevise = useCallback(async () => {
    setError(''); setLoading('revise');
    try {
      await agents.approvePlan(agentId, false, feedback, null, true);
      setSuccessState('replanning');
    } catch (e) {
      setError(e.message || 'Failed to request revision');
      setLoading(null);
    }
  }, [agentId, feedback]);

  const handleReject = useCallback(async () => {
    setError(''); setLoading('reject');
    try {
      await agents.approvePlan(agentId, false, feedback, null, false);
      setSuccessState('stopped');
    } catch (e) {
      setError(e.message || 'Failed to reject plan');
      setLoading(null);
    }
    setConfirmReject(false);
  }, [agentId, feedback]);

  // ── Terminal states ────────────────────────────────────────────────────

  if (replanning || successState === 'replanning') {
    return (
      <motion.div
        className="my-3 rounded-xl border border-warn/25 bg-warn-soft/20 p-4 flex items-center gap-3"
        initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
      >
        <Loader2 size={14} className="text-warn animate-spin shrink-0" />
        <span className="text-sm text-tx-2">Replanning with your feedback…</span>
      </motion.div>
    );
  }

  if (successState === 'stopped') {
    return (
      <motion.div
        className="my-3 rounded-xl border border-err/25 bg-err-soft/20 p-4 flex items-center gap-3"
        initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
      >
        <XCircle size={14} className="text-err shrink-0" />
        <span className="text-sm text-tx-2">Plan rejected — agent stopped.</span>
      </motion.div>
    );
  }

  if (successState === 'approving') {
    return (
      <motion.div
        className="my-3 rounded-xl border border-ok/25 bg-ok-soft/20 p-4 flex items-center gap-3"
        initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
      >
        <Loader2 size={14} className="text-ok animate-spin shrink-0" />
        <span className="text-sm text-tx-2">Starting execution…</span>
      </motion.div>
    );
  }

  const hasGaps = credGaps.length > 0;
  const isFinalRejection = rejectionCount + 1 >= MAX_REJECTIONS;

  return (
    <motion.div
      className="rounded-xl border-l-4 border-l-accent border border-accent/25 bg-bg-card overflow-hidden"
      initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: [0.25, 0.1, 0.25, 1] }}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-border/60">
        <div className="flex items-center gap-2">
          <Layers size={14} className="text-accent shrink-0" />
          <span className="text-sm font-semibold text-tx-1">
            Plan ready — {stepCount} step{stepCount !== 1 ? 's' : ''}
          </span>
          {jobType && (
            <span className="badge bg-info-soft text-info border border-info/20 text-[10px]">
              {jobType.replace(/_/g, ' ')}
            </span>
          )}
          {compilerStage && (
            <span className="badge bg-vio-soft text-vio border border-vio/20 text-[10px]">
              compiler {formatLabel(compilerStage)}
            </span>
          )}
          {rejectionCount > 0 && (
            <span className="badge bg-warn-soft text-warn border border-warn/20 text-[10px]">
              Revised {rejectionCount}x
            </span>
          )}
        </div>
        <span className="text-xs text-tx-4">
          {hasGaps ? 'Action required' : 'Awaiting review'}
        </span>
      </div>

      <div className="px-4 py-4 space-y-4">
        {rationale && <p className="text-xs text-tx-2 leading-relaxed">{rationale}</p>}

        {(runtimePolicy || researchSummary) && (
          <div className="space-y-2 rounded-xl border border-border bg-bg px-3 py-2.5">
            {runtimePolicy && (
              <div>
                <p className="text-[10px] uppercase tracking-wide text-tx-4">Runtime policy</p>
                <p className="mt-1 text-[11px] text-tx-2 leading-relaxed whitespace-pre-wrap">{runtimePolicy}</p>
              </div>
            )}
            {researchSummary && (
              <div>
                <p className="text-[10px] uppercase tracking-wide text-tx-4">Research summary</p>
                <p className="mt-1 text-[11px] text-tx-2 leading-relaxed whitespace-pre-wrap">{researchSummary}</p>
              </div>
            )}
          </div>
        )}

        {(compilerStage || compilerValidationIssues.length > 0) && (
          <div className="space-y-2 rounded-xl border border-vio/20 bg-vio-soft/10 px-3 py-2.5">
            <div className="flex items-center gap-2 flex-wrap">
              <p className="text-[10px] uppercase tracking-wide text-tx-4">Compiler</p>
              {compilerStage && (
                <span className="badge bg-vio-soft text-vio border border-vio/20 text-[10px]">
                  {formatLabel(compilerStage)}
                </span>
              )}
              <span className="text-[10px] text-tx-4">
                repair passes: {compilerRepairPasses}
              </span>
            </div>
            {compilerValidationIssues.length > 0 ? (
              <ul className="space-y-1 text-[11px] text-tx-2 leading-relaxed">
                {compilerValidationIssues.map((issue, index) => (
                  <li key={`${issue}-${index}`}>• {issue}</li>
                ))}
              </ul>
            ) : (
              <p className="text-[11px] text-tx-3 leading-relaxed">
                Validation passed. The compiler draft is ready for review.
              </p>
            )}
          </div>
        )}

        <div className="rounded-lg border border-border/60 bg-bg px-3 py-2 space-y-1">
          <p className="text-[11px] font-medium text-tx-1">Governance</p>
          <p className="text-[11px] text-tx-3 leading-relaxed">
            Approval freezes the current compiled workflow artifact, the worker checkpoints each step, and connector writes
            carry stable idempotency keys when the runtime can derive them.
          </p>
          <p className="text-[11px] text-tx-4 leading-relaxed">
            If you revise the plan later, Narayan creates a new version instead of mutating the saved one in place.
          </p>
        </div>

        {steps.length > 0 && (
          <div className="space-y-2">
            {steps.map((step, i) => (
              <StepRow key={step.index ?? i} step={step} confidence={stepConfidence[i]} />
            ))}
          </div>
        )}

        <div className="flex items-center gap-4 text-[10px] text-tx-4">
          <span className="flex items-center gap-1"><span className="size-2 rounded-full bg-ok inline-block" /> Skill match</span>
          <span className="flex items-center gap-1"><span className="size-2 rounded-full bg-warn inline-block" /> Improvised</span>
          <span className="flex items-center gap-1"><span className="size-2 rounded-full bg-err inline-block" /> Needs credentials</span>
        </div>

        {/* Per-gap banners — each handles its own resolution flow */}
        {credGaps.map(name => (
          <CredentialGap
            key={name}
            name={name}
            onResolved={handleCredResolved}
            onWrongTool={handleWrongTool}
            onNavigateSettings={onNavigateSettings}
          />
        ))}

        {hasGaps && (
          <p className="text-[11px] text-tx-4">
            Approve unlocks automatically once all credentials are connected. Checking every 5 s.
          </p>
        )}

        <div>
          <label className="text-[11px] font-medium text-tx-3 block mb-1.5">
            Feedback or suggestions (optional)
          </label>
          <textarea
            value={feedback}
            onChange={e => setFeedback(e.target.value)}
            placeholder="Add context, constraints, or revision notes…"
            rows={2}
            className="input-field resize-none text-xs"
          />
        </div>

        {error && <p className="text-xs text-err">{error}</p>}

        <div className="flex items-center gap-2 flex-wrap">
          <button
            onClick={handleApprove}
            disabled={!canApprove}
            title={hasGaps ? 'Resolve credential issues above first' : undefined}
            className="btn-primary flex items-center gap-1.5 disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {loading === 'approve' ? <Loader2 size={12} className="animate-spin" /> : <Play size={12} />}
            Approve &amp; Run
          </button>

          <button
            onClick={handleRevise}
            disabled={!!loading}
            className="btn-secondary flex items-center gap-1.5 disabled:opacity-40"
          >
            {loading === 'revise' ? <Loader2 size={12} className="animate-spin" /> : <RotateCcw size={12} />}
            Revise Plan
          </button>

          <AnimatePresence mode="wait">
            {confirmReject ? (
              <motion.div
                key="confirm" className="flex items-center gap-2"
                initial={{ opacity: 0, x: -4 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0 }}
              >
                <span className="text-xs text-tx-3">
                  {isFinalRejection ? 'Agent will stop. Confirm?' : 'Confirm rejection?'}
                </span>
                <button
                  onClick={handleReject} disabled={!!loading}
                  className="inline-flex items-center gap-1 px-2 py-1 text-xs font-medium rounded
                             bg-err/10 text-err hover:bg-err/20 disabled:opacity-40 transition-colors"
                >
                  {loading === 'reject' && <Loader2 size={11} className="animate-spin" />}
                  Yes, stop
                </button>
                <button onClick={() => setConfirmReject(false)} className="text-xs text-tx-4 hover:text-tx-2 transition-colors">
                  Cancel
                </button>
              </motion.div>
            ) : (
              <motion.button
                key="reject-btn" onClick={() => setConfirmReject(true)} disabled={!!loading}
                className="flex items-center gap-1.5 text-xs font-medium text-err hover:text-err/80 transition-colors disabled:opacity-40"
                initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
              >
                <XCircle size={12} />
                Reject
              </motion.button>
            )}
          </AnimatePresence>
        </div>

        {rejectionCount > 0 && (
          <p className="text-[11px] text-tx-4">
            Revision {rejectionCount} of {MAX_REJECTIONS} — agent stops after {MAX_REJECTIONS} rejections.
          </p>
        )}
      </div>
    </motion.div>
  );
}
