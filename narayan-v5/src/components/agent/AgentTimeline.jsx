import { useState, useEffect, useRef, useMemo } from 'react';
import { AnimatePresence } from 'framer-motion';
import clsx from 'clsx';
import { Activity, Loader2, Network, Layers, Zap, Wrench, RotateCcw, Bell, Cpu } from 'lucide-react';
import { useAgentTimeline } from '../../hooks/useAgentTimeline';
import { PhaseBar } from '../layout';
import { PlanCard, StepCard, ClarificationCard, ReviewCard, GoalCompleteCard, GoalFailedCard, ConnectorTriggerCard, PolicyCard, CitationCard } from '../cards';
import CostCounter from './CostCounter';
import ReplayScrubber from './ReplayScrubber';
import { agents as agentsApi } from '../../api';

const TERMINAL = new Set(['completed', 'failed']);

function Badge({ label, color = 'gray', icon: Icon }) {
  const cls = {
    gray: 'bg-bg-active text-tx-3 border-border', amber: 'bg-warn-soft text-warn border-warn/25',
    green: 'bg-ok-soft text-ok border-ok/25', red: 'bg-err-soft text-err border-err/25',
    blue: 'bg-info-soft text-info border-info/25', violet: 'bg-vio-soft text-vio border-vio/25',
  }[color] || 'bg-bg-active text-tx-3 border-border';
  return (
    <span className={clsx('inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded shrink-0 tracking-wide uppercase border', cls)}>
      {Icon && <Icon size={9} />}{label}
    </span>
  );
}

function RunOverview({ events, liveStatus }) {
  const stats = events.reduce((acc, ev) => {
    if (ev.type === 'step_started') acc.steps += 1;
    if (ev.type === 'tool_called') acc.tools += 1;
    if (ev.type === 'step_retrying') acc.retries += 1;
    if (ev.type === 'review_required') acc.reviews += 1;
    return acc;
  }, { steps: 0, tools: 0, retries: 0, reviews: 0 });
  const plan = events.find(ev => ev.type === 'plan_created');
  return (
    <div className="mb-3 rounded-xl border border-border bg-bg-card px-3.5 py-3 flex flex-wrap items-center gap-2">
      <Badge label={liveStatus} color={liveStatus === 'failed' ? 'red' : liveStatus === 'completed' ? 'green' : 'blue'} icon={Activity} />
      {plan?.job_type && <Badge label={plan.job_type.replace(/_/g, ' ')} color="violet" icon={Cpu} />}
      {plan?.step_count != null && <Badge label={`${plan.step_count} planned`} color="gray" icon={Layers} />}
      <Badge label={`${stats.steps} started`} color="gray" icon={Zap} />
      <Badge label={`${stats.tools} tools`} color="gray" icon={Wrench} />
      {stats.retries > 0 && <Badge label={`${stats.retries} retries`} color="amber" icon={RotateCcw} />}
      {stats.reviews > 0 && <Badge label={`${stats.reviews} review`} color="amber" icon={Bell} />}
    </div>
  );
}

function ConnectionBanner({ meta }) {
  if (!meta || ['idle', 'live', 'closed'].includes(meta.state)) return null;
  const title = meta.state === 'hydrating' ? 'Loading history...'
    : meta.state === 'reconnecting' ? `Reconnecting... attempt ${meta.retryAttempt || 1}`
    : meta.state === 'auth_lost' ? 'Session expired' : 'Connecting...';
  return (
    <div className="mb-3 rounded-xl border border-border bg-bg-card px-3.5 py-3 flex items-center gap-2">
      <Loader2 size={13} className="text-tx-4 animate-spin shrink-0" />
      <p className="text-xs text-tx-2">{title}</p>
    </div>
  );
}

function ThinkingDots() {
  return (
    <div className="flex items-center gap-1.5 px-2 py-3">
      {[0, 1, 2].map(i => (
        <div key={i} className="size-1.5 rounded-full bg-tx-4 animate-pulse-dot" style={{ animationDelay: `${i * 0.22}s` }} />
      ))}
    </div>
  );
}

export default function AgentTimeline({ agentId, initialStatus, onStatusChange, onTerminal, onNavigateSettings }) {
  const bottomRef = useRef(null);
  const [agentDetail, setAgentDetail] = useState(null);
  const [replayMode, setReplayMode] = useState(false);
  const [visibleUpTo, setVisibleUpTo] = useState(Infinity);

  const {
    events, questions, liveStatus, isThinking, setQuestions, connectionMeta, groupedEvents,
  } = useAgentTimeline(agentId, initialStatus, { onStatusChange, onTerminal });

  const isTerminal = TERMINAL.has(liveStatus);

  useEffect(() => { bottomRef.current?.scrollIntoView({ behavior: 'smooth' }); }, [events, isThinking]);

  useEffect(() => {
    if (isTerminal && agentId) {
      agentsApi.get(agentId).then(setAgentDetail).catch(() => {});
    }
  }, [isTerminal, agentId]);

  const cost = useMemo(() => {
    return events.reduce((sum, ev) => {
      if (ev.cost_usd) return sum + ev.cost_usd;
      return sum;
    }, 0);
  }, [events]);

  if (events.length === 0 && !questions.length) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <div className="size-10 rounded-xl bg-bg-active flex items-center justify-center mb-3">
          <Activity size={17} className="text-tx-4" />
        </div>
        <p className="text-xs text-tx-3">Waiting for agent to start...</p>
      </div>
    );
  }

  const { preflight, plan, steps = [], delegation, terminal } = groupedEvents || {};

  return (
    <div className="py-2">
      <PhaseBar groupedEvents={groupedEvents} />

      {/* Replay scrubber for terminal agents */}
      {isTerminal && events.length > 0 && (
        <div className="mb-3">
          <ReplayScrubber
            groupedEvents={groupedEvents}
            totalEvents={events.length}
            onVisibleUpToChange={setVisibleUpTo}
          />
        </div>
      )}

      <RunOverview events={events} liveStatus={liveStatus} />
      <ConnectionBanner meta={connectionMeta} />

      {/* Cost counter */}
      <div className="flex justify-end mb-2">
        <CostCounter cost={cost} isRunning={!isTerminal} />
      </div>

      {/* Connector trigger */}
      {events.filter(e => e.type === 'connector_trigger').slice(0, visibleUpTo + 1).map((ev, i) => (
        <div key={`conn-${i}`} className="mb-2">
          <ConnectorTriggerCard event={ev} />
        </div>
      ))}

      {/* Plan card */}
      {plan?.stepCount > 0 && (
        <div className="mb-3">
          <PlanCard event={events.find(e => e.type === 'plan_created') || { step_count: plan.stepCount, rationale: plan.rationale, steps: plan.steps, job_type: plan.jobType }} />
        </div>
      )}

      {/* Step cards */}
      <AnimatePresence>
        <div className="space-y-3">
          {steps.slice(0, visibleUpTo + 1).map((step, i) => (
            <StepCard key={`step-${step.index ?? i}`} step={step} />
          ))}
        </div>
      </AnimatePresence>

      {/* Reviews */}
      {events.filter(e => e.type === 'review_required').map((ev, i) => (
        <div key={`review-${i}`} className="my-3">
          <ReviewCard event={ev} />
        </div>
      ))}

      {/* Clarification */}
      {questions.length > 0 && liveStatus === 'clarifying' && (
        <div className="my-3">
          <ClarificationCard
            agentId={agentId}
            questions={questions}
            onDone={() => { setQuestions([]); onStatusChange?.('waiting'); }}
            onNavigateSettings={onNavigateSettings}
          />
        </div>
      )}

      {/* Terminal cards */}
      {terminal?.type === 'complete' && (
        <div className="mt-3">
          <GoalCompleteCard event={terminal} agentDetail={agentDetail} />
        </div>
      )}
      {terminal?.type === 'failed' && (
        <div className="mt-3">
          <GoalFailedCard event={terminal} stepsCompleted={steps.filter(s => s.completed).length} />
        </div>
      )}

      {isThinking && !isTerminal && <ThinkingDots />}
      <div ref={bottomRef} />
    </div>
  );
}
