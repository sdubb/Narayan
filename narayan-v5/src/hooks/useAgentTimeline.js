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
    goal_complete: 'completed',
    goal_failed: 'failed',
    step_started: 'running',
    clarification_needed: 'clarifying',
    clarification_received: 'running',
    child_spawned: 'delegating',
    planning_started: 'running',
    step_retrying: 'waiting',
    step_completed: 'waiting',
  }[type];
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
        type: event.type,
        step_index: event.step_index,
        tool_name: event.tool_name,
        description: event.description,
        summary: event.summary,
        reason: event.reason,
        ts: event.ts,
        historical: event.historical,
      });
      const alreadyExists = current.some((item) => JSON.stringify({
        type: item.type,
        step_index: item.step_index,
        tool_name: item.tool_name,
        description: item.description,
        summary: item.summary,
        reason: item.reason,
        ts: item.ts,
        historical: item.historical,
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
        if (replayEvents.length) {
          setEvents(replayEvents);
        }
      } catch {
        // Replay is optional; live stream still proceeds.
      }
    }

    function scheduleReconnect(lastError) {
      if (cancelled || isTerminalStatus(liveStatusRef.current)) return;
      const nextAttempt = retryAttemptRef.current + 1;
      const delay = Math.min(30000, 1000 * (2 ** Math.min(nextAttempt, 5)));
      setRetryAttempt(nextAttempt);
      retryAttemptRef.current = nextAttempt;
      setConnectionState('reconnecting');
      appendEvent({
        type: 'stream_status',
        ts: nowTs(),
        summary: `Stream disconnected. Reconnecting in ${Math.round(delay / 1000)}s.`,
        detail: lastError?.message || '',
      });
      reconnectTimer.current = setTimeout(() => {
        if (!cancelled) connect();
      }, delay);
    }

    function connect() {
      if (cancelled || isTerminalStatus(initialStatus)) {
        setConnectionState('idle');
        return;
      }

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

          if (event.type === 'clarification_needed' && event.questions) {
            setQuestions(event.questions);
          }

          if (event.type === 'tool_result' && event.tool_name === 'suggest_connectors') {
            try {
              const payload = JSON.parse(event.raw_output || '{}');
              if (payload.groups) setConnectorGroups(payload.groups);
            } catch {
              // ignore parse failures
            }
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
          if (error?.message?.includes('sign in again')) {
            setConnectionState('auth_lost');
            return;
          }
          appendEvent({ type: 'stream_error', ts: nowTs(), summary: error?.message || 'Stream error' });
          scheduleReconnect(error);
        },
      );
    }

    hydrateReplay().finally(() => {
      if (!cancelled) connect();
    });

    return () => {
      cancelled = true;
      cleanupStream();
      clearTimeout(thinkTimer.current);
    };
  }, [agentId, initialStatus]);

  const connectionMeta = useMemo(
    () => ({ state: connectionState, retryAttempt }),
    [connectionState, retryAttempt],
  );

  return {
    events,
    questions,
    connectorGroups,
    liveStatus,
    isThinking,
    setQuestions,
    connectionMeta,
  };
}
