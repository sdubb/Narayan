import { useState, useEffect } from 'react';
import { Loader2, Mail, CheckCircle2 } from 'lucide-react';
import { agentMessages } from '../../api';

export default function AgentMessagesTab({ agentId }) {
  const [messages, setMessages] = useState([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => { load(); }, [agentId]);

  async function load() {
    setLoading(true);
    try {
      const res = await agentMessages.list(agentId, { limit: 50 });
      setMessages(res.messages || []);
    } catch (e) {
      console.error('Failed to load messages', e);
    } finally {
      setLoading(false);
    }
  }

  async function handleAck(messageId) {
    try {
      await agentMessages.ack(agentId, messageId);
      load();
    } catch (e) {
      console.error('Failed to ack message', e);
    }
  }

  if (loading) {
    return <div className="flex items-center justify-center py-8"><Loader2 size={16} className="animate-spin text-tx-4" /></div>;
  }

  if (messages.length === 0) {
    return <div className="py-8 text-center text-sm text-tx-4">No messages found.</div>;
  }

  return (
    <div className="space-y-3">
      {messages.map(msg => (
        <div key={msg.id} className="p-4 bg-bg-card border border-border rounded-xl">
          <div className="flex items-start justify-between gap-3 mb-2">
            <div className="flex items-center gap-2">
              <Mail size={14} className={msg.delivered_at ? 'text-tx-4' : 'text-accent'} />
              <p className="text-sm font-semibold text-tx-1">{msg.kind}</p>
              {!msg.delivered_at && (
                <span className="text-[10px] bg-accent-soft text-accent px-1.5 py-0.5 rounded">Unread</span>
              )}
            </div>
            <div className="text-xs text-tx-4">{new Date(msg.created_at).toLocaleString()}</div>
          </div>
          <div className="text-xs text-tx-3 bg-bg p-3 rounded-lg overflow-auto max-h-40 whitespace-pre-wrap font-mono">
            {JSON.stringify(msg.payload, null, 2)}
          </div>
          {!msg.delivered_at && (
            <button
              onClick={() => handleAck(msg.id)}
              className="mt-3 flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-lg bg-ok-soft text-ok hover:bg-ok/20 transition-colors"
            >
              <CheckCircle2 size={12} />
              Acknowledge
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
