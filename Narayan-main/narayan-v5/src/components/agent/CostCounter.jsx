import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';

export default function CostCounter({ cost = 0, isRunning = false }) {
  if (!isRunning && cost === 0) return null;

  return (
    <AnimatePresence>
      <motion.div
        className={clsx(
          'inline-flex items-center gap-1.5 rounded-full px-3 py-1.5',
          'border border-accent/20 bg-accent-soft/60',
          isRunning && 'shadow-glow-amber',
        )}
        initial={{ opacity: 0, scale: 0.9 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.9 }}
        key="cost"
      >
        <motion.span
          className="font-mono text-xs font-semibold text-accent"
          key={cost.toFixed(4)}
          initial={{ scale: 1.15 }}
          animate={{ scale: 1 }}
          transition={{ duration: 0.2 }}
        >
          ${cost.toFixed(3)}
        </motion.span>
        {isRunning && (
          <motion.span
            className="size-1.5 rounded-full bg-accent-glow"
            animate={{ opacity: [0.4, 1, 0.4], scale: [0.8, 1.2, 0.8] }}
            transition={{ duration: 1, repeat: Infinity, ease: 'easeInOut' }}
          />
        )}
      </motion.div>
    </AnimatePresence>
  );
}
