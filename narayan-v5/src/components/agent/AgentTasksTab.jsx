import { useState, useEffect } from 'react';
import { Loader2, ListTodo } from 'lucide-react';
import { sessionTasks } from '../../api';

export default function AgentTasksTab({ agentId }) {
  const [tasks, setTasks] = useState([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => { load(); }, [agentId]);

  async function load() {
    setLoading(true);
    try {
      const res = await sessionTasks.list(agentId);
      setTasks(res.tasks || []);
    } catch (e) {
      console.error('Failed to load tasks', e);
    } finally {
      setLoading(false);
    }
  }

  if (loading) {
    return <div className="flex items-center justify-center py-8"><Loader2 size={16} className="animate-spin text-tx-4" /></div>;
  }

  if (tasks.length === 0) {
    return <div className="py-8 text-center text-sm text-tx-4">No session tasks found.</div>;
  }

  return (
    <div className="space-y-3">
      {tasks.map(task => (
        <div key={task.id} className="p-4 bg-bg-card border border-border rounded-xl">
          <div className="flex items-start justify-between gap-3 mb-2">
            <div className="flex items-center gap-2">
              <ListTodo size={14} className="text-accent" />
              <p className="text-sm font-semibold text-tx-1">{task.subject}</p>
              <span className={`text-[10px] px-1.5 py-0.5 rounded border ${
                task.status === 'completed' ? 'bg-ok-soft text-ok border-ok/20' :
                task.status === 'in_progress' ? 'bg-info-soft text-info border-info/20' :
                'bg-bg text-tx-3 border-border'
              }`}>
                {task.status.replace('_', ' ')}
              </span>
            </div>
          </div>
          <p className="text-xs text-tx-3 mb-3">{task.description}</p>
          {task.output && (
            <div className="text-xs bg-bg p-3 rounded-lg text-tx-2">
              <p className="font-semibold mb-1">Output Result: {task.output.status}</p>
              <ul className="list-disc pl-4 space-y-1 text-tx-3">
                {task.output.findings?.map((f, i) => <li key={i}>{f}</li>)}
              </ul>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
