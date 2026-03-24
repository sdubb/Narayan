import { motion } from 'framer-motion';
import { CheckCircle2, BookOpen, FileText, ExternalLink } from 'lucide-react';

export default function GoalCompleteCard({ event, agentDetail }) {
  const summary = event.summary || agentDetail?.final_answer || agentDetail?.metadata?.final_answer || '';
  const keyFindings = agentDetail?.metadata?.key_findings || [];
  const workspacePath = agentDetail?.workspace_path;

  return (
    <motion.div
      className="rounded-xl border border-ok/25 bg-ok-soft shadow-glow-green overflow-hidden"
      initial={{ opacity: 0, y: 12, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
    >
      {/* Banner */}
      <div className="flex items-start gap-3 px-5 py-4">
        <motion.div
          initial={{ scale: 0 }} animate={{ scale: 1 }}
          transition={{ delay: 0.15, type: 'spring', stiffness: 300 }}
        >
          <CheckCircle2 size={20} className="text-ok mt-0.5" />
        </motion.div>
        <div className="flex-1">
          <p className="text-base font-semibold text-ok">Goal completed</p>
          {summary && <p className="text-sm text-tx-2 mt-1 leading-relaxed">{summary}</p>}
        </div>
      </div>

      {/* Key findings */}
      {keyFindings.length > 0 && (
        <div className="border-t border-ok/15">
          <div className="flex items-center gap-2 px-5 py-3">
            <BookOpen size={13} className="text-tx-3" />
            <span className="text-xs font-semibold text-tx-2">Key findings</span>
            <span className="ml-auto text-[10px] text-tx-4 font-mono">{keyFindings.length}</span>
          </div>
          <div className="px-5 pb-4 space-y-2">
            {keyFindings.map((f, i) => (
              <motion.div
                key={i}
                className="flex items-start gap-3"
                initial={{ opacity: 0, x: -8 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ delay: 0.1 + i * 0.05 }}
              >
                <span className="font-mono text-xs text-accent w-5 shrink-0 text-right">{i + 1}.</span>
                <p className="text-xs text-tx-2 leading-relaxed">{f}</p>
              </motion.div>
            ))}
          </div>
        </div>
      )}

      {/* Workspace path */}
      {workspacePath && (
        <div className="border-t border-ok/15 px-5 py-3 flex items-center gap-2">
          <FileText size={12} className="text-tx-3 shrink-0" />
          <span className="font-mono text-xs text-tx-3 truncate flex-1">{workspacePath}</span>
          <ExternalLink size={11} className="text-tx-4 shrink-0" />
        </div>
      )}
    </motion.div>
  );
}
