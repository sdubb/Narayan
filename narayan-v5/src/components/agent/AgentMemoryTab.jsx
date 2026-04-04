import { useState, useEffect } from 'react';
import { Loader2, BrainCircuit, Play } from 'lucide-react';
import { memory } from '../../api';

export default function AgentMemoryTab({ agentId }) {
  const [topics, setTopics] = useState([]);
  const [loading, setLoading] = useState(true);
  const [consolidating, setConsolidating] = useState(false);
  const [result, setResult] = useState(null);

  useEffect(() => {
    if (!agentId) {
      setTopics([]);
      setLoading(false);
      return;
    }
    load();
  }, [agentId]);

  async function load() {
    setLoading(true);
    try {
      const res = await memory.topics(agentId);
      setTopics(res.topics || []);
    } catch (e) {
      console.error('Failed to load memory topics', e);
    } finally {
      setLoading(false);
    }
  }

  async function handleConsolidate() {
    if (!agentId) return;
    setConsolidating(true);
    setResult(null);
    try {
      const res = await memory.consolidate(agentId);
      setResult(res);
      load();
    } catch (e) {
      console.error('Consolidation failed', e);
      setResult({ error: e.message });
    } finally {
      setConsolidating(false);
    }
  }

  return (
    <div className="space-y-4">
      {!agentId && (
        <div className="rounded-xl border border-border bg-bg-card p-4 text-center text-sm text-tx-4">
          No live run selected yet.
        </div>
      )}
      <div className="flex items-center justify-between bg-bg-card p-4 rounded-xl border border-border">
        <div>
          <h3 className="text-sm font-semibold text-tx-1 flex items-center gap-2">
            <BrainCircuit size={16} className="text-accent" />
            Memory Consolidation
          </h3>
          <p className="text-xs text-tx-3 mt-1">Force an immediate consolidation of recent session tasks and agent steps.</p>
        </div>
        <button
          onClick={handleConsolidate}
          disabled={consolidating || !agentId}
          className="btn-primary flex items-center gap-2 text-xs py-1.5 px-3"
        >
          {consolidating ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
          {consolidating ? 'Consolidating...' : agentId ? 'Consolidate Now' : 'No live run'}
        </button>
      </div>

      {result && (
        <div className={`p-4 rounded-xl border text-xs ${result.error ? 'bg-err-soft border-err/20 text-err' : 'bg-ok-soft border-ok/20 text-ok'}`}>
          <p className="font-semibold mb-1">{result.error ? 'Error' : 'Consolidation complete'}</p>
          <p>{result.error || result.summary}</p>
        </div>
      )}

      <div>
        <h4 className="text-xs font-semibold text-tx-2 uppercase tracking-wide mb-3">Durable Memory Topics</h4>
        {loading ? (
          <div className="py-4 flex justify-center"><Loader2 size={16} className="animate-spin text-tx-4" /></div>
        ) : topics.length === 0 ? (
          <div className="py-8 text-center text-sm text-tx-4 bg-bg rounded-xl border border-border">No memory topics found.</div>
        ) : (
          <div className="space-y-3">
            {topics.map(topic => (
              <div key={topic.key} className="p-4 bg-bg-card border border-border rounded-xl">
                <p className="text-sm font-semibold text-tx-1 mb-1">{topic.title || topic.key}</p>
                <p className="text-xs text-tx-3">Hook: {topic.hook}</p>
                <div className="mt-3 text-xs font-mono bg-bg p-3 rounded-lg overflow-auto max-h-40 whitespace-pre-wrap">
                  {topic.content}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
