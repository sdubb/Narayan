import { useState, useEffect, useCallback, useRef } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { SkipBack, ChevronLeft, Play, Pause, ChevronRight, SkipForward } from 'lucide-react';

function buildTrackNodes(groupedEvents) {
  if (!groupedEvents) return [];
  const nodes = [];
  let idx = 0;

  if (groupedEvents.preflight?.started) {
    nodes.push({ index: idx++, label: 'Pre', phase: 'preflight', status: groupedEvents.preflight.failed ? 'failed' : groupedEvents.preflight.passed ? 'passed' : 'active' });
  }
  if (groupedEvents.plan?.stepCount > 0) {
    nodes.push({ index: idx++, label: 'Plan', phase: 'planning', status: 'passed' });
  }
  (groupedEvents.steps || []).forEach((step, i) => {
    nodes.push({ index: idx++, label: `Step ${step.index ?? i}`, phase: 'execution', status: step.completed ? 'passed' : 'active' });
  });
  if (groupedEvents.terminal?.type) {
    nodes.push({ index: idx++, label: 'End', phase: 'done', status: groupedEvents.terminal.type === 'complete' ? 'passed' : 'failed' });
  }
  return nodes;
}

export default function ReplayScrubber({ groupedEvents, totalEvents = 0, onVisibleUpToChange }) {
  const trackNodes = buildTrackNodes(groupedEvents);
  const maxIndex = Math.max(totalEvents - 1, 0);
  const [position, setPosition] = useState(maxIndex);
  const [isPlaying, setIsPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const intervalRef = useRef(null);

  useEffect(() => { setPosition(maxIndex); }, [maxIndex]);

  useEffect(() => {
    onVisibleUpToChange?.(position);
  }, [position, onVisibleUpToChange]);

  useEffect(() => {
    if (!isPlaying) { clearInterval(intervalRef.current); return; }
    intervalRef.current = setInterval(() => {
      setPosition(p => {
        if (p >= maxIndex) { setIsPlaying(false); return p; }
        return p + 1;
      });
    }, 1000 / speed);
    return () => clearInterval(intervalRef.current);
  }, [isPlaying, speed, maxIndex]);

  const jumpToStart = () => { setPosition(0); setIsPlaying(false); };
  const prevStep = () => setPosition(p => Math.max(0, p - 1));
  const nextStep = () => setPosition(p => Math.min(maxIndex, p + 1));
  const jumpToEnd = () => { setPosition(maxIndex); setIsPlaying(false); };
  const cycleSpeed = () => setSpeed(s => s === 1 ? 2 : s === 2 ? 4 : 1);

  const currentNode = trackNodes.find((n, i) => {
    const nextNode = trackNodes[i + 1];
    if (!nextNode) return true;
    return position <= n.index;
  }) || trackNodes[trackNodes.length - 1];

  return (
    <motion.div
      className="rounded-xl border border-border bg-bg-card shadow-sm px-4 py-3"
      initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
    >
      {/* Controls */}
      <div className="flex items-center gap-1 mb-3">
        <button onClick={jumpToStart} className="p-1.5 rounded-lg hover:bg-bg-hover text-tx-3 hover:text-tx-1 transition-colors">
          <SkipBack size={14} />
        </button>
        <button onClick={prevStep} className="p-1.5 rounded-lg hover:bg-bg-hover text-tx-3 hover:text-tx-1 transition-colors">
          <ChevronLeft size={14} />
        </button>
        <button onClick={() => setIsPlaying(p => !p)}
          className={clsx('p-1.5 rounded-lg transition-colors', isPlaying ? 'bg-accent text-white' : 'hover:bg-bg-hover text-tx-3 hover:text-tx-1')}>
          {isPlaying ? <Pause size={14} /> : <Play size={14} />}
        </button>
        <button onClick={nextStep} className="p-1.5 rounded-lg hover:bg-bg-hover text-tx-3 hover:text-tx-1 transition-colors">
          <ChevronRight size={14} />
        </button>
        <button onClick={jumpToEnd} className="p-1.5 rounded-lg hover:bg-bg-hover text-tx-3 hover:text-tx-1 transition-colors">
          <SkipForward size={14} />
        </button>
        <button onClick={cycleSpeed}
          className="ml-2 rounded-full border border-border px-2 py-0.5 text-[10px] font-mono text-tx-3 hover:text-tx-1 hover:border-border-md transition-colors">
          {speed}x
        </button>
        <span className="ml-auto text-[11px] font-mono text-tx-4">
          {position + 1} / {maxIndex + 1}
        </span>
      </div>

      {/* Track */}
      <div className="flex items-center gap-0.5">
        {trackNodes.map((node, i) => {
          const isCurrent = node === currentNode;
          const isPast = i < trackNodes.indexOf(currentNode);
          return (
            <button
              key={i}
              onClick={() => setPosition(Math.min(node.index, maxIndex))}
              className="flex items-center gap-0.5 group"
            >
              {i > 0 && <div className={clsx('w-4 h-px', isPast || isCurrent ? 'bg-accent' : 'bg-border')} />}
              <span className={clsx(
                'size-3 rounded-full border-2 transition-colors',
                node.status === 'failed' ? 'border-err bg-err' :
                node.status === 'passed' ? 'border-ok bg-ok' :
                isCurrent ? 'border-accent bg-accent' : 'border-border bg-bg',
              )} />
              <span className={clsx(
                'text-[9px] font-medium ml-0.5',
                isCurrent ? 'text-accent' : 'text-tx-4',
              )}>
                {node.label}
              </span>
            </button>
          );
        })}
      </div>

      {/* Context */}
      {currentNode && (
        <p className="text-[10px] text-tx-3 mt-2 font-mono">
          {currentNode.phase}: {currentNode.label}
        </p>
      )}
    </motion.div>
  );
}
