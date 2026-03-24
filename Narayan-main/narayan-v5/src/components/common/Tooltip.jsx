import { useState, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import clsx from 'clsx';

export default function Tooltip({ children, content, position = 'top', delay = 300 }) {
  const [show, setShow] = useState(false);
  const timeout = useRef(null);
  const onEnter = () => { timeout.current = setTimeout(() => setShow(true), delay); };
  const onLeave = () => { clearTimeout(timeout.current); setShow(false); };
  const pos = {
    top: 'bottom-full left-1/2 -translate-x-1/2 mb-2',
    bottom: 'top-full left-1/2 -translate-x-1/2 mt-2',
    left: 'right-full top-1/2 -translate-y-1/2 mr-2',
    right: 'left-full top-1/2 -translate-y-1/2 ml-2',
  };
  return (
    <span className="relative inline-flex" onMouseEnter={onEnter} onMouseLeave={onLeave}>
      {children}
      <AnimatePresence>
        {show && content && (
          <motion.span
            className={clsx(
              'absolute z-50 px-2.5 py-1.5 rounded-lg text-xs font-medium',
              'bg-tx-1 text-bg-card shadow-lg whitespace-nowrap pointer-events-none',
              pos[position]
            )}
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            transition={{ duration: 0.1 }}
          >
            {content}
          </motion.span>
        )}
      </AnimatePresence>
    </span>
  );
}
