import { motion } from 'framer-motion';
import clsx from 'clsx';

const variants = {
  hidden: { opacity: 0, y: 12 },
  visible: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -8 },
};

export default function AnimatedCard({ children, className, delay = 0, layout = false, onClick, ...props }) {
  return (
    <motion.div
      className={clsx('card', className)}
      variants={variants}
      initial="hidden"
      animate="visible"
      exit="exit"
      transition={{ duration: 0.2, delay, ease: [0.25, 0.1, 0.25, 1] }}
      layout={layout}
      onClick={onClick}
      {...props}
    >
      {children}
    </motion.div>
  );
}
