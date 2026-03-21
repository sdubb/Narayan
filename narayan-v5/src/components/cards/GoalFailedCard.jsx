import { motion } from 'framer-motion';
import { AlertCircle } from 'lucide-react';

export default function GoalFailedCard({ event, stepsCompleted = 0 }) {
  return (
    <motion.div
      className="rounded-xl border border-err/25 bg-err-soft shadow-glow-red overflow-hidden"
      initial={{ opacity: 0, y: 12, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
    >
      <div className="flex items-start gap-3 px-5 py-4">
        <motion.div
          initial={{ scale: 0, rotate: -90 }}
          animate={{ scale: 1, rotate: 0 }}
          transition={{ delay: 0.1, type: 'spring', stiffness: 300 }}
        >
          <AlertCircle size={20} className="text-err mt-0.5" />
        </motion.div>
        <div className="flex-1">
          <p className="text-base font-semibold text-err">Goal failed</p>
          {event.reason && (
            <p className="text-sm text-tx-2 mt-1 leading-relaxed">{event.reason}</p>
          )}
          {stepsCompleted > 0 && (
            <p className="text-xs text-tx-3 mt-2">
              Completed {stepsCompleted} step{stepsCompleted !== 1 ? 's' : ''} before failure.
            </p>
          )}
        </div>
      </div>
    </motion.div>
  );
}
