import { useState, useCallback, useRef, useEffect } from 'react';

function buildTrackNodes(groupedEvents) {
  if (!groupedEvents) return [];
  const nodes = [];
  let idx = 0;

  if (groupedEvents.preflight?.started) {
    nodes.push({
      index: idx++, label: 'Pre', phase: 'preflight',
      status: groupedEvents.preflight.failed ? 'failed' : groupedEvents.preflight.passed ? 'passed' : 'active',
    });
  }
  if (groupedEvents.plan?.stepCount > 0) {
    nodes.push({ index: idx++, label: 'Plan', phase: 'planning', status: 'passed' });
  }
  (groupedEvents.steps || []).forEach((step, i) => {
    nodes.push({
      index: idx++,
      label: `Step ${step.index ?? i}`,
      phase: 'execution',
      status: step.completed ? 'passed' : 'active',
    });
  });
  if (groupedEvents.terminal?.type) {
    nodes.push({
      index: idx++, label: 'End', phase: 'done',
      status: groupedEvents.terminal.type === 'complete' ? 'passed' : 'failed',
    });
  }
  return nodes;
}

export function useReplayScrubber(groupedEvents, totalEvents = 0) {
  const maxIndex = Math.max(totalEvents - 1, 0);
  const [visibleUpTo, setVisibleUpTo] = useState(maxIndex);
  const [isPlaying, setIsPlaying] = useState(false);
  const [playbackSpeed, setPlaybackSpeed] = useState(1);
  const intervalRef = useRef(null);

  const trackNodes = buildTrackNodes(groupedEvents);

  useEffect(() => { setVisibleUpTo(maxIndex); }, [maxIndex]);

  useEffect(() => {
    if (!isPlaying) { clearInterval(intervalRef.current); return; }
    intervalRef.current = setInterval(() => {
      setVisibleUpTo(p => {
        if (p >= maxIndex) { setIsPlaying(false); return p; }
        return p + 1;
      });
    }, 1000 / playbackSpeed);
    return () => clearInterval(intervalRef.current);
  }, [isPlaying, playbackSpeed, maxIndex]);

  const currentStep = trackNodes.reduce((acc, n) => visibleUpTo >= n.index ? n.index : acc, 0);
  const currentPhase = trackNodes.find(n => n.index === currentStep)?.phase || 'preflight';

  const controls = {
    jumpToStart: useCallback(() => { setVisibleUpTo(0); setIsPlaying(false); }, []),
    prevStep: useCallback(() => setVisibleUpTo(p => Math.max(0, p - 1)), []),
    nextStep: useCallback(() => setVisibleUpTo(p => Math.min(maxIndex, p + 1)), [maxIndex]),
    jumpToEnd: useCallback(() => { setVisibleUpTo(maxIndex); setIsPlaying(false); }, [maxIndex]),
    togglePlay: useCallback(() => setIsPlaying(p => !p), []),
    cycleSpeed: useCallback(() => setPlaybackSpeed(s => s === 1 ? 2 : s === 2 ? 4 : 1), []),
    seekTo: useCallback((index) => setVisibleUpTo(Math.min(index, maxIndex)), [maxIndex]),
  };

  return { visibleUpTo, currentStep, currentPhase, isPlaying, playbackSpeed, controls, trackNodes };
}
