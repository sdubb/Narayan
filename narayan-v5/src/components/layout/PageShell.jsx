import clsx from 'clsx';

export default function PageShell({ children, className }) {
  return (
    <div className={clsx('min-h-screen bg-bg', className)}>
      {children}
    </div>
  );
}
