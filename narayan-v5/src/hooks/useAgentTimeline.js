import { useEffect, useMemo, useRef, useState } from 'react';
import { agents, streamAgent as openAgentStream } from '../api';

function nowTs() {
  return new Date().toLocaleTimeString('en', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

function normalizeEvent(ev) {
  if (!ev || typeof ev !== 'object') return ev;
  return ev.type ? ev : { ...ev, type: ev.event };
}

function replayToEvents(replay) {
  const steps = Array.isArray(replay?.steps) ? replay.steps : [];
  return steps.map((step, index) => ({
    type: 'replay_step',
    historical: true,
    step_index: step.step_index ?? index,
    description: step.action,
    output_preview: step.result,
    ts: step.timestamp ? step.timestamp.slice(11, 19) : nowTs(),
    timestamp: step.timestamp,
  }));
}

function isTerminalStatus(status) {
  return status === 'completed' || status === 'failed';
}

function nextStatusFromEvent(type) {
  return {
    goal_complete:          'completed',
    goal_failed:            'failed',
    step_started:           'running',
    clarification_needed:   'clarifying',
    clarification_received: 'running',
    child_spawned:          'delegating',
    planning_started:       'running',
    step_retrying:          'waiting',
    step_completed:         'waiting',
    plan_approval_needed:   'plan_approval_needed',
    plan_approved:          'waiting',
    plan_rejected:          'plan_rejected',
  }[type];
}

// ── Group flat events into structured data ────────────────────────────────
function buildGroupedEvents(events) {
  const grouped = {
    preflight: { started: false, passed: false, failed: false, failReason: '', questions: [] },
    plan: {
      stepCount: 0, rationale: '', jobType: '', steps: [],
      approvalNeeded: false, replanning: false,
      rejectionCount: 0, missingCredentials: [], stepConfidence: [],
    },
    steps: [],
    delegation: { children: [], allComplete: false },
    terminal: { type: null, summary: '', reason: '' },
  };

  let currentStepIndex = -1;

  for (const ev of events) {
    const t = ev.type;

    // Preflight
    if (t === 'preflight_started') grouped.preflight.started = true;
    if (t === 'preflight_passed') grouped.preflight.passed = true;
    if (t === 'preflight_failed') { grouped.preflight.failed = true; grouped.preflight.failReason = ev.reason || ''; }
    if (t === 'clarification_needed') grouped.preflight.questions = ev.questions || [];
    if (t === 'clarification_received') grouped.preflight.questions = [];

    // Plan
    if (t === 'plan_created') {
      grouped.plan.stepCount = ev.step_count || 0;
      grouped.plan.rationale = ev.rationale || '';
      grouped.plan.jobType   = ev.job_type  || '';
      grouped.plan.steps     = ev.steps     || [];
    }
    if (t === 'plan_approval_needed') {
      grouped.plan.approvalNeeded       = true;
      grouped.plan.replanning           = false;
      grouped.plan.stepCount            = ev.step_count            || grouped.plan.stepCount;
      grouped.plan.rationale            = ev.rationale             || grouped.plan.rationale;
      grouped.plan.steps                = ev.steps                 || grouped.plan.steps;
      grouped.plan.jobType              = ev.job_type              || grouped.plan.jobType;
      grouped.plan.rejectionCount       = ev.rejection_count       || 0;
      grouped.plan.missingCredentials   = ev.missing_credentials   || [];
      grouped.plan.stepConfidence       = ev.step_confidence       || [];
    }
    if (t === 'plan_rejected') {
      grouped.plan.replanning     = true;
      grouped.plan.approvalNeeded = false;
      grouped.plan.rejectionCount = ev.rejection_count || 0;
    }
    if (t === 'plan_approved') {
      grouped.plan.approvalNeeded = false;
      grouped.plan.replanning     = false;
    }

    // Steps
    if (t === 'step_started') {
      currentStepIndex = grouped.steps.length;
      grouped.steps.push({
        index: ev.step_index ?? grouped.steps.length,
        description: ev.description || '',
        tools: [],
        policy: [],
        citations: [],
        piiEvents: [],
        slaEvents: [],
        reviews: [],
        completed: false,
        summary: '',
        retrying: false,
        retryDelay: 0,
        retryReason: '',
      });
    }

    const step = currentStepIndex >= 0 ? grouped.steps[currentStepIndex] : null;

    if (t === 'tool_called' && step) {
      step.tools.push({ name: ev.tool_name, args_preview: ev.args_preview, output_preview: null, success: null, error: null });
    }
    if (t === 'tool_result' && step) {
      const tool = step.tools.find(t2 => t2.name === ev.tool_name && t2.success === null);
      if (tool) {
        tool.output_preview = ev.output_preview;
        tool.success = ev.success;
        tool.error = ev.error;
      } else {
        step.tools.push({ name: ev.tool_name, args_preview: null, output_preview: ev.output_preview, success: ev.success, error: ev.error });
      }
    }
    if (t === 'policy_decision' && step) {
      step.policy.push({ decision: ev.decision, rule_id: ev.rule_id, reason: ev.reason, risk_level: ev.risk_level, tool: ev.tool });
    }
    if (t === 'pii_redacted' && step) {
      step.piiEvents.push({ tool: ev.tool, fields_redacted: ev.fields_redacted || [] });
    }
    if (t === 'sla_check' && step) {
      step.slaEvents.push({ pct_elapsed: ev.pct_elapsed, message: ev.message, action: ev.action, deadline: ev.deadline });
    }
    if (t === 'citation_recorded' && step) {
      step.citations.push({ claim: ev.claim, source_ref: ev.source_ref, source_type: ev.source_type, confidence: ev.confidence, step_index: ev.step_index });
    }
    if (t === 'review_required' && step) {
      step.reviews.push({ review_id: ev.review_id, summary: ev.summary, reason: ev.reason, rule_id: ev.rule_id });
    }
    if (t === 'step_completed' && step) {
      step.completed = true;
      step.summary = ev.summary || '';
    }
    if (t === 'step_retrying' && step) {
      step.retrying = true;
      step.retryDelay = ev.delay_secs || 10;
      step.retryReason = ev.reason || '';
    }

    // Delegation
    if (t === 'child_spawned') {
      grouped.delegation.children.push({ child_agent_id: ev.child_agent_id, sub_goal: ev.sub_goal });
    }
    if (t === 'children_complete') {
      grouped.delegation.allComplete = true;
    }

    // Terminal
    if (t === 'goal_complete') {
      grouped.terminal = { type: 'complete', summary: ev.summary || '', reason: '' };
    }
    if (t === 'goal_failed') {
      grouped.terminal = { type: 'failed', summary: '', reason: ev.reason || '' };
    }
  }

  return grouped;
}

export function useAgentTimeline(agentId, initialStatus, { onStatusChange, onTerminal } = {}) {
  const [events, setEvents] = useState([]);
  const [questions, setQuestions] = useState([]);
  const [connectorGroups, setConnectorGroups] = useState([]);
  const [liveStatus, setLiveStatus] = useState(initialStatus);
  const [isThinking, setIsThinking] = useState(false);
  const [connectionState, setConnectionState] = useState('idle');
  const [retryAttempt, setRetryAttempt] = useState(0);

  const streamRef = useRef(null);
  const thinkTimer = useRef(null);
  const reconnectTimer = useRef(null);
  const mountedRef = useRef(false);
  const liveStatusRef = useRef(initialStatus);
  const retryAttemptRef = useRef(0);

  function bumpThinking() {
    setIsThinking(true);
    clearTimeout(thinkTimer.current);
    thinkTimer.current = setTimeout(() => setIsThinking(false), 4000);
  }

  function cleanupStream() {
    streamRef.current?.close?.();
    streamRef.current = null;
    clearTimeout(reconnectTimer.current);
  }

  function appendEvent(event) {
    setEvents((current) => {
      const key = JSON.stringify({
        type: event.type, step_index: event.step_index, tool_name: event.tool_name,
        description: event.description, summary: event.summary, reason: event.reason,
        ts: event.ts, historical: event.historical,
      });
      const alreadyExists = current.some((item) => JSON.stringify({
        type: item.type, step_index: item.step_index, tool_name: item.tool_name,
        description: item.description, summary: item.summary, reason: item.reason,
        ts: item.ts, historical: item.historical,
      }) === key);
      return alreadyExists ? current : [...current, event];
    });
  }

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      cleanupStream();
      clearTimeout(thinkTimer.current);
    };
  }, []);

  useEffect(() => {
    if (!agentId) return undefined;

    setEvents([]);
    setQuestions([]);
    setConnectorGroups([]);
    setLiveStatus(initialStatus);
    liveStatusRef.current = initialStatus;
    setIsThinking(false);
    setConnectionState('hydrating');
    setRetryAttempt(0);
    retryAttemptRef.current = 0;
    cleanupStream();

    let cancelled = false;

    async function hydrateReplay() {
      try {
        const replay = await agents.replay(agentId);
        if (cancelled || !mountedRef.current) return;
        const replayEvents = replayToEvents(replay);
        if (replayEvents.length) setEvents(replayEvents);
      } catch {}
    }

    function scheduleReconnect(lastError) {
      if (cancelled || isTerminalStatus(liveStatusRef.current)) return;
      const nextAttempt = retryAttemptRef.current + 1;
      const delay = Math.min(30000, 1000 * (2 ** Math.min(nextAttempt, 5)));
      setRetryAttempt(nextAttempt);
      retryAttemptRef.current = nextAttempt;
      setConnectionState('reconnecting');
      appendEvent({ type: 'stream_status', ts: nowTs(), summary: `Reconnecting in ${Math.round(delay / 1000)}s.`, detail: lastError?.message || '' });
      reconnectTimer.current = setTimeout(() => { if (!cancelled) connect(); }, delay);
    }

    function connect() {
      if (cancelled || isTerminalStatus(initialStatus)) { setConnectionState('idle'); return; }
      cleanupStream();
      setConnectionState('live');
      streamRef.current = openAgentStream(
        agentId,
        (raw) => {
          if (cancelled || !mountedRef.current) return;
          const event = { ...normalizeEvent(raw), ts: nowTs() };
          appendEvent(event);
          bumpThinking();
          setConnectionState('live');
          setRetryAttempt(0);
          retryAttemptRef.current = 0;

          if (event.type === 'clarification_needed' && event.questions) setQuestions(event.questions);
          if (event.type === 'tool_result' && event.tool_name === 'suggest_connectors') {
            try { const p = JSON.parse(event.raw_output || '{}'); if (p.groups) setConnectorGroups(p.groups); } catch {}
          }

          const next = nextStatusFromEvent(event.type);
          if (next) {
            setLiveStatus(next);
            liveStatusRef.current = next;
            onStatusChange?.(next);
            if (isTerminalStatus(next)) {
              setIsThinking(false);
              clearTimeout(thinkTimer.current);
              setConnectionState('closed');
              onTerminal?.(event);
            }
          }
        },
        (error) => {
          if (cancelled || !mountedRef.current) return;
          setIsThinking(false);
          if (error?.message?.includes('sign in again')) { setConnectionState('auth_lost'); return; }
          appendEvent({ type: 'stream_error', ts: nowTs(), summary: error?.message || 'Stream error' });
          scheduleReconnect(error);
        },
      );
    }

    hydrateReplay().finally(() => { if (!cancelled) connect(); });

    return () => { cancelled = true; cleanupStream(); clearTimeout(thinkTimer.current); };
  }, [agentId, initialStatus]);

  const connectionMeta = useMemo(() => ({ state: connectionState, retryAttempt }), [connectionState, retryAttempt]);

  const groupedEvents = useMemo(() => buildGroupedEvents(events), [events]);

  return {
    events, questions, connectorGroups, liveStatus, isThinking,
    setQuestions, connectionMeta, groupedEvents,
  };
}
