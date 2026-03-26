import { motion } from 'framer-motion';
import { useEffect, useState } from 'react';
import {
  ArrowRight,
  Bot,
  CheckCircle2,
  FileText,
  Layers3,
  MessageSquareText,
  Scale,
  Search,
  ShieldCheck,
  Sparkles,
  Workflow,
  Zap,
} from 'lucide-react';

const stats = [
  { value: 'Write', label: 'Describe the job in plain English. Agent learns the playbook.' },
  { value: 'Validate', label: 'Test connections, verify workflows. Catch issues before deployment.' },
  { value: 'Deploy', label: 'Agent executes 24/7. Audit trails log every decision and action.' },
];

const pillars = [
  {
    icon: Workflow,
    title: 'No salary, benefits, or overhead',
    text: 'Pay only for work done. Agents on-demand, not headcount. Scale without hiring delays.',
  },
  {
    icon: ShieldCheck,
    title: 'Deterministic & auditable',
    text: 'Every decision is recorded. Replay any step. Compliance-first by design.',
  },
  {
    icon: Layers3,
    title: 'Any role, any workflow',
    text: 'Finance, support, legal, sales, research—same agent platform powers them all.',
  },
];

const steps = [
  { title: 'Write the job', text: 'Describe what you need done in plain language. Define approval rules and escalations.' },
  { title: 'Agent learns it', text: 'System validates connections, tests permissions, simulates workflows before deployment.' },
  { title: 'It runs 24/7', text: 'Agent executes your job specification autonomously. Every action is logged and audited.' },
];

function AnimatedConnector({ fromX, fromY, toX, toY, delay = 0 }) {
  return (
    <svg className="absolute inset-0 size-full pointer-events-none" style={{ overflow: 'visible' }}>
      <defs>
        <linearGradient id={`grad-${delay}`} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="rgba(59, 130, 246, 0)" />
          <stop offset="40%" stopColor="rgba(59, 130, 246, 0.8)" />
          <stop offset="100%" stopColor="rgba(59, 130, 246, 0)" />
        </linearGradient>
      </defs>
      <motion.line
        x1={fromX}
        y1={fromY}
        x2={toX}
        y2={toY}
        stroke={`url(#grad-${delay})`}
        strokeWidth="3"
        strokeLinecap="round"
        initial={{ pathLength: 0, opacity: 0 }}
        animate={{ pathLength: 1, opacity: 1 }}
        transition={{
          duration: 2,
          delay: delay,
          repeat: Infinity,
          repeatDelay: 1.5,
        }}
      />
    </svg>
  );
}

function ArchitectureDiagram() {
  return (
    <div className="relative w-full rounded-2xl border border-border/40 bg-gradient-to-br from-bg-card/60 via-bg/40 to-accent/5 overflow-hidden backdrop-blur-sm"
         style={{ perspective: '1200px' }}>
      <style>{`
        @keyframes pulse-glow {
          0%, 100% { filter: drop-shadow(0 0 8px rgba(201, 106, 46, 0.4)); }
          50% { filter: drop-shadow(0 0 24px rgba(201, 106, 46, 0.7)); }
        }
        @keyframes float-up {
          0%, 100% { transform: translateY(0px); }
          50% { transform: translateY(-8px); }
        }
        @keyframes rotate-slow {
          from { transform: rotateZ(0deg); }
          to { transform: rotateZ(360deg); }
        }
        .pulse-glow { animation: pulse-glow 2.5s ease-in-out infinite; }
        .float-animation { animation: float-up 3s ease-in-out infinite; }
        .rotate-animation { animation: rotate-slow 20s linear infinite; }
      `}</style>

      {/* 3D Background */}
      <div className="absolute inset-0 opacity-40">
        <div className="absolute inset-0 bg-gradient-to-b from-info/5 via-transparent to-accent/5" />
        <motion.div
          className="absolute inset-0 bg-gradient-to-r from-accent/10 via-transparent to-info/10"
          animate={{ opacity: [0.3, 0.5, 0.3] }}
          transition={{ duration: 4, repeat: Infinity }}
        />
      </div>

      {/* Main Container */}
      <div className="relative h-80 w-full flex items-center justify-center p-8">
        <svg viewBox="0 0 1000 350" className="w-full h-full" preserveAspectRatio="xMidYMid meet">
          <defs>
            <radialGradient id="central-glow">
              <stop offset="0%" stopColor="rgba(201, 106, 46, 0.4)" />
              <stop offset="60%" stopColor="rgba(201, 106, 46, 0.15)" />
              <stop offset="100%" stopColor="rgba(201, 106, 46, 0)" />
            </radialGradient>
            <filter id="glow-filter">
              <feGaussianBlur stdDeviation="3" result="coloredBlur" />
              <feMerge>
                <feMergeNode in="coloredBlur" />
                <feMergeNode in="SourceGraphic" />
              </feMerge>
            </filter>
          </defs>

          {/* Central Agent - Animated Pulse */}
          <motion.circle
            cx="500"
            cy="175"
            r="50"
            fill="url(#central-glow)"
            stroke="url(#central-glow)"
            strokeWidth="2"
            initial={{ r: 50 }}
            animate={{ r: 55 }}
            transition={{ duration: 2.5, repeat: Infinity }}
          />
          <circle cx="500" cy="175" r="50" fill="rgba(201, 106, 46, 0.2)" stroke="rgba(201, 106, 46, 0.6)" strokeWidth="2" />

          {/* Core rings */}
          <motion.circle
            cx="500"
            cy="175"
            r="50"
            fill="none"
            stroke="rgba(201, 106, 46, 0.3)"
            strokeWidth="1"
            initial={{ r: 50 }}
            animate={{ r: 70 }}
            transition={{ duration: 2, repeat: Infinity }}
            opacity="0.5"
          />

          <text x="500" y="170" textAnchor="middle" className="text-xl font-bold" fill="rgba(0,0,0,0.8)">
            Agent
          </text>
          <text x="500" y="190" textAnchor="middle" className="text-sm" fill="rgba(0,0,0,0.5)">
            Multi-role
          </text>

          {/* LEFT: Inbound Connectors */}
          {[
            { name: 'Zendesk', y: 80 },
            { name: 'Salesforce', y: 175 },
            { name: 'GitHub', y: 270 },
          ].map((connector, idx) => (
            <g key={`left-${idx}`}>
              {/* Animated connector line */}
              <motion.line
                x1="180"
                y1={connector.y}
                x2="450"
                y2="175"
                stroke="rgb(59, 130, 246)"
                strokeWidth="3"
                opacity="0.3"
                initial={{ strokeDashoffset: 50 }}
                animate={{ strokeDashoffset: 0 }}
                transition={{ duration: 3, repeat: Infinity }}
                strokeDasharray="10,5"
              />

              {/* Animated arrow pulse */}
              <motion.circle
                cx="180"
                cy={connector.y}
                r="5"
                fill="rgb(59, 130, 246)"
                initial={{ opacity: 0 }}
                animate={{ opacity: [0.2, 1, 0.2] }}
                transition={{ duration: 2, delay: idx * 0.4, repeat: Infinity }}
              />

              {/* Connector node */}
              <motion.circle
                cx="140"
                cy={connector.y}
                r="22"
                fill="rgba(59, 130, 246, 0.1)"
                stroke="rgb(59, 130, 246)"
                strokeWidth="2"
                animate={{ r: [22, 26, 22] }}
                transition={{ duration: 2.5, delay: idx * 0.3, repeat: Infinity }}
              />
              <text x="140" y={connector.y + 4} textAnchor="middle" className="text-xs font-semibold" fill="rgb(59, 130, 246)">
                {connector.name}
              </text>
            </g>
          ))}

          {/* RIGHT: Outbound Connectors */}
          {[
            { name: 'Email', y: 80 },
            { name: 'Slack', y: 175 },
            { name: 'Audit', y: 270 },
          ].map((connector, idx) => (
            <g key={`right-${idx}`}>
              {/* Animated connector line */}
              <motion.line
                x1="550"
                y1="175"
                x2="820"
                y2={connector.y}
                stroke="rgb(59, 130, 246)"
                strokeWidth="3"
                opacity="0.3"
                initial={{ strokeDashoffset: 50 }}
                animate={{ strokeDashoffset: 0 }}
                transition={{ duration: 3, delay: 1, repeat: Infinity }}
                strokeDasharray="10,5"
              />

              {/* Animated arrow pulse */}
              <motion.circle
                cx="820"
                cy={connector.y}
                r="5"
                fill="rgb(59, 130, 246)"
                initial={{ opacity: 0 }}
                animate={{ opacity: [0.2, 1, 0.2] }}
                transition={{ duration: 2, delay: 1 + idx * 0.4, repeat: Infinity }}
              />

              {/* Connector node */}
              <motion.circle
                cx="860"
                cy={connector.y}
                r="22"
                fill="rgba(59, 130, 246, 0.1)"
                stroke="rgb(59, 130, 246)"
                strokeWidth="2"
                animate={{ r: [22, 26, 22] }}
                transition={{ duration: 2.5, delay: 1 + idx * 0.3, repeat: Infinity }}
              />
              <text x="860" y={connector.y + 4} textAnchor="middle" className="text-xs font-semibold" fill="rgb(59, 130, 246)">
                {connector.name}
              </text>
            </g>
          ))}

          {/* Role layer - Animated bottom bar */}
          <motion.g opacity="0.6">
            <rect x="280" y="290" width="440" height="40" rx="8" fill="none" stroke="rgba(201, 106, 46, 0.4)" strokeWidth="2" strokeDasharray="8,4" />
            <text x="360" y="315" className="text-xs font-semibold" fill="rgba(201, 106, 46, 0.7)">
              Finance
            </text>
            <text x="500" y="315" className="text-xs font-semibold" fill="rgba(201, 106, 46, 0.7)" textAnchor="middle">
              Support
            </text>
            <text x="640" y="315" className="text-xs font-semibold" fill="rgba(201, 106, 46, 0.7)" textAnchor="end">
              Sales
            </text>
          </motion.g>
        </svg>
      </div>

      {/* Legend with animation */}
      <motion.div
        className="absolute bottom-4 left-4 right-4 flex flex-wrap items-center gap-6 text-xs"
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.3 }}
      >
        <div className="flex items-center gap-2">
          <motion.div
            className="h-1 w-6 rounded-full bg-info"
            animate={{ opacity: [0.5, 1, 0.5] }}
            transition={{ duration: 2, repeat: Infinity }}
          />
          <span className="text-tx-3">Data flow (animated)</span>
        </div>
        <div className="flex items-center gap-2">
          <motion.div
            className="size-4 rounded-full bg-accent/20 border border-accent/60"
            animate={{ scale: [1, 1.15, 1] }}
            transition={{ duration: 2.5, repeat: Infinity }}
          />
          <span className="text-tx-3">Connector</span>
        </div>
        <div className="flex items-center gap-2">
          <div className="size-4 rounded border border-dashed border-accent/40" />
          <span className="text-tx-3">Role layer</span>
        </div>
      </motion.div>
    </div>
  );
}

const examples = [
  {
    icon: FileText,
    tag: 'Finance',
    title: 'Invoice Processor',
    trigger: 'Gmail webhook',
    connectors: ['gmail', 'quickbooks', 'workspace'],
    summary: 'Job: Process incoming invoices. Read email, match PO, post to QuickBooks if approved, flag exceptions.',
    outline: ['read invoice from email', 'match purchase order', 'post approved items', 'flag mismatches'],
    metrics: ['47 processed', '3 escalated', '$847.3K handled'],
  },
  {
    icon: MessageSquareText,
    tag: 'Support',
    title: 'Support Ticket Responder',
    trigger: 'Zendesk ticket_created',
    connectors: ['zendesk', 'docs', 'slack'],
    summary: 'Job: Triage and respond to support tickets. Summarize issue, search docs, draft reply, escalate urgent ones.',
    outline: ['fetch ticket history', 'search knowledge base', 'draft response', 'route to human if needed'],
    metrics: ['24 drafts', '6 escalations', '12s avg'],
  },
  {
    icon: Scale,
    tag: 'Legal',
    title: 'Contract Risk Reviewer',
    trigger: 'User uploads contract',
    connectors: ['workspace'],
    summary: 'Job: Review contracts for risk. Extract clauses, flag severity, deliver plain-language summary.',
    outline: ['read contract PDF', 'extract key terms', 'rate clause severity', 'save summary'],
    metrics: ['5 flags', '1 page', 'saved'],
  },
  {
    icon: Search,
    tag: 'Sales',
    title: 'Prospect Researcher',
    trigger: 'User request',
    connectors: ['web', 'crm', 'workspace'],
    summary: 'Job: Research prospects before outreach. Gather company data, identify decision-makers, write pitch angle.',
    outline: ['clarify target', 'search web & LinkedIn', 'compile findings', 'write recommendation'],
    metrics: ['5 prospects', '3+ sources', 'ready'],
  },
];

function ProductSurface() {
  const [tick, setTick] = useState(0);

  useEffect(() => {
    const interval = setInterval(() => {
      setTick(prev => (prev + 1) % examples.length);
    }, 2600);
    return () => clearInterval(interval);
  }, []);

  const current = examples[tick];
  const Icon = current.icon;

  return (
    <div className="rounded-[2rem] border border-border bg-bg-card/95 p-5 shadow-card">
      <div className="flex items-center justify-between gap-3 border-b border-border pb-4">
        <div className="flex items-center gap-3">
          <div className="flex size-11 items-center justify-center rounded-2xl bg-accent-soft text-accent">
            <Icon className="size-5" />
          </div>
          <div>
            <p className="text-xs uppercase tracking-[0.22em] text-tx-4">{current.tag} Agent</p>
            <p className="text-lg font-medium text-tx-1">{current.title}</p>
          </div>
        </div>
        <div className="badge bg-info-soft text-info">
          <div className="mr-1 size-2 rounded-full bg-info" />
          Job Spec
        </div>
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-[1.1fr_0.9fr]">
        <div className="space-y-4">
          <div className="rounded-2xl border border-border bg-bg p-4">
            <p className="text-xs font-semibold uppercase tracking-[0.2em] text-accent">job description</p>
            <p className="mt-2 text-sm text-tx-2">{current.trigger}</p>
            <p className="mt-3 text-sm leading-6 text-tx-2">{current.summary}</p>
          </div>

          <div className="rounded-2xl border border-border bg-bg p-4">
            <p className="text-xs font-semibold uppercase tracking-[0.2em] text-accent">execution steps</p>
            <div className="mt-3 space-y-2">
              {current.outline.map(step => (
                <div key={step} className="flex items-center gap-2 rounded-xl border border-border bg-bg-card px-3 py-2 text-sm text-tx-2">
                  <CheckCircle2 className="size-4 text-accent" />
                  {step}
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="space-y-4">
          <div className="rounded-2xl border border-border bg-bg p-4">
            <p className="text-xs font-semibold uppercase tracking-[0.2em] text-accent">pre-deployment checks</p>
            <div className="mt-3 space-y-3">
              <CheckRow label="connectors" text={current.connectors.join(', ')} />
              <CheckRow label="permissions" text="API keys validated" />
              <CheckRow label="workflow" text="steps simulated & verified" />
              <CheckRow label="audit" text="ready to log actions" />
            </div>
          </div>

          <div className="rounded-2xl border border-border bg-bg p-4">
            <p className="text-xs font-semibold uppercase tracking-[0.2em] text-accent">systems connected</p>
            <div className="mt-3 flex flex-wrap gap-2">
              {current.connectors.map(connector => (
                <span key={connector} className="rounded-full border border-border bg-bg-card px-3 py-1 text-xs text-tx-2">
                  {connector}
                </span>
              ))}
            </div>
          </div>

          <div className="rounded-2xl border border-ok-soft/40 bg-ok-soft/15 p-4">
            <div className="grid grid-cols-3 gap-3">
              {current.metrics.map(metric => (
                <div key={metric} className="text-center">
                  <p className="text-lg font-bold text-ok">{metric}</p>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>

      <div className="mt-4 flex items-center gap-2 text-xs text-tx-4">
        <Zap className="size-3.5" />
        Write job, validate connections, deploy agent, audit all actions
      </div>
    </div>
  );
}

function CheckRow({ label, text }) {
  return (
    <div className="flex items-start gap-3 rounded-xl border border-border bg-bg-card px-3 py-2">
      <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-ok" />
      <div>
        <p className="text-xs uppercase tracking-[0.18em] text-tx-4">{label}</p>
        <p className="text-sm text-tx-2">{text}</p>
      </div>
    </div>
  );
}

function ExampleCard({ example }) {
  const Icon = example.icon;

  return (
    <div className="card card-hover border-border/80 bg-bg-card/90 p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-3">
          <div className="flex size-11 items-center justify-center rounded-2xl bg-accent-soft text-accent">
            <Icon className="size-5" />
          </div>
          <div>
            <p className="text-xs uppercase tracking-[0.24em] text-tx-4">{example.tag} agent</p>
            <h3 className="mt-1 text-lg font-medium text-tx-1">{example.title}</h3>
          </div>
        </div>
        <div className="badge bg-info-soft text-info">Deployed</div>
      </div>

      <p className="mt-4 text-sm leading-6 text-tx-2">{example.summary}</p>

      <div className="mt-4 flex flex-wrap gap-2">
        {example.connectors.map(connector => (
          <span key={connector} className="rounded-full border border-border bg-bg px-3 py-1 text-xs text-tx-2">
            {connector}
          </span>
        ))}
      </div>

      <div className="mt-4 space-y-2">
        {example.outline.slice(0, 3).map(step => (
          <div key={step} className="flex items-center gap-2 text-xs text-tx-3">
            <CheckCircle2 className="size-3.5 text-accent" />
            {step}
          </div>
        ))}
      </div>
    </div>
  );
}

export default function LandingPage({ onEnterApp, onSignIn }) {
  return (
    <main className="relative min-h-screen overflow-hidden bg-[radial-gradient(circle_at_top_left,_rgba(201,106,46,0.16),_transparent_28%),radial-gradient(circle_at_top_right,_rgba(59,130,246,0.12),_transparent_24%),linear-gradient(180deg,_#f9f6f2_0%,_#f4f0ea_48%,_#efe8de_100%)] text-tx-1">
      <div className="absolute inset-0 pointer-events-none">
        <div className="absolute left-[-6rem] top-24 h-64 w-64 rounded-full bg-accent/10 blur-3xl" />
        <div className="absolute right-[-5rem] top-10 h-72 w-72 rounded-full bg-info/10 blur-3xl" />
        <div className="absolute bottom-0 left-1/2 h-80 w-80 -translate-x-1/2 rounded-full bg-vio/10 blur-3xl" />
      </div>

      <div className="relative mx-auto flex min-h-screen max-w-7xl flex-col px-6 py-6 lg:px-10">
        <header className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="flex size-11 items-center justify-center rounded-2xl border border-border bg-bg-card shadow-card">
              <Bot className="size-5 text-accent" />
            </div>
            <div>
              <p className="font-serif text-2xl leading-none">Narayan</p>
              <p className="text-xs uppercase tracking-[0.24em] text-tx-4">Hire digital employees. Write the job.</p>
            </div>
          </div>

          <div className="hidden items-center gap-3 md:flex">
            <button onClick={onSignIn} className="btn-ghost">
              Sign in
            </button>
            <button onClick={onEnterApp} className="btn-primary inline-flex items-center gap-2">
              Open app <ArrowRight className="size-4" />
            </button>
          </div>
        </header>

        <section className="grid flex-1 items-center gap-8 py-12 lg:grid-cols-[0.9fr_1.1fr] lg:py-14">
          <motion.div
            initial={{ opacity: 0, y: 18 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.45, ease: 'easeOut' }}
            className="max-w-xl"
          >
            <div className="mb-4 inline-flex items-center gap-2 rounded-full border border-border bg-bg-card/90 px-3 py-1.5 text-xs font-medium text-tx-2 shadow-card">
              <Sparkles className="size-3.5 text-accent" />
              Hire digital employees for any job
            </div>

            <h1 className="font-serif text-4xl leading-[0.95] text-tx-1 sm:text-5xl lg:text-6xl">
              Write a job description.
              <span className="block text-accent">Agent does the work.</span>
            </h1>

            <p className="mt-5 max-w-lg text-base leading-7 text-tx-2 sm:text-lg">
              No salary. No benefits. No hiring cycle. Deploy agents for finance, support, legal, sales—any role, any workflow. Every action logged. Always auditable.
            </p>

            <div className="mt-7 flex flex-col gap-3 sm:flex-row">
              <button onClick={onEnterApp} className="btn-primary inline-flex items-center justify-center gap-2 px-5 py-3">
                Get started <ArrowRight className="size-4" />
              </button>
              <button onClick={onSignIn} className="btn-secondary inline-flex items-center justify-center gap-2 px-5 py-3">
                Sign in to workspace
              </button>
            </div>

            <div className="mt-8 grid gap-4 sm:grid-cols-3">
              {stats.map(item => (
                <div key={item.value} className="card border-border/80 bg-bg-card/90 p-4">
                  <p className="font-serif text-2xl text-accent">{item.value}</p>
                  <p className="mt-2 text-sm leading-6 text-tx-2">{item.label}</p>
                </div>
              ))}
            </div>
          </motion.div>

          <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5, ease: 'easeOut', delay: 0.08 }}
            className="relative"
          >
            <ProductSurface />
          </motion.div>
        </section>

        <section className="pb-5">
          <div className="mb-4 flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent">Job Examples</p>
              <h2 className="mt-2 font-serif text-2xl text-tx-1">Agents at work. Real roles, real results.</h2>
            </div>
            <p className="max-w-xl text-sm leading-6 text-tx-3">
              Write these job specs once. Agents execute them 24/7 with full audit trails and compliance built-in.
            </p>
          </div>

          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
            {examples.map(example => (
              <ExampleCard key={example.title} example={example} />
            ))}
          </div>
        </section>

        {/* Visual Divider */}
        <div className="my-12 flex items-center gap-4">
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
          <Layers3 className="size-4 text-accent/40" />
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        {/* Architecture Section */}
        <section className="pb-12">
          <div className="mb-8">
            <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent">How Agents Connect</p>
            <h2 className="mt-2 font-serif text-3xl text-tx-1">Multi-role agents. Any connector. Any system.</h2>
            <p className="mt-3 max-w-2xl text-base leading-7 text-tx-2">
              One agent can handle multiple roles. Each role reads from incoming connectors (Zendesk, GitHub, Salesforce)
              and writes to outbound channels (Email, Slack, Audit). All deterministic. All logged.
            </p>
          </div>
          <ArchitectureDiagram />
        </section>

        {/* Visual Divider */}
        <div className="my-12 flex items-center gap-4">
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
          <Workflow className="size-4 text-accent/40" />
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        {/* How it Works Section */}
        <section className="pb-12">
          <div className="mb-8">
            <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent">How It Works</p>
            <h2 className="mt-2 font-serif text-3xl text-tx-1">Three steps from job spec to production.</h2>
          </div>
          <div className="grid gap-4 md:grid-cols-3">
            {steps.map((step, idx) => (
              <div key={step.title} className="card bg-bg-card/90 p-5 relative">
                <div className="absolute -left-2 -top-2 flex size-8 items-center justify-center rounded-full bg-accent text-white font-semibold text-xs">
                  {idx + 1}
                </div>
                <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent pt-2">{step.title}</p>
                <p className="mt-3 text-sm leading-6 text-tx-2">{step.text}</p>
              </div>
            ))}
          </div>
        </section>

        {/* Visual Divider */}
        <div className="my-12 flex items-center gap-4">
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
          <ShieldCheck className="size-4 text-accent/40" />
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        {/* Pillars Section */}
        <section className="grid gap-6 pb-5 lg:grid-cols-3">
          {pillars.map(({ icon: Icon, title, text }) => (
            <div key={title} className="card card-hover bg-bg-card/90 p-5">
              <div className="flex size-11 items-center justify-center rounded-2xl bg-accent-soft text-accent">
                <Icon className="size-5" />
              </div>
              <h2 className="mt-4 text-xl font-medium text-tx-1">{title}</h2>
              <p className="mt-2 text-sm leading-6 text-tx-2">{text}</p>
            </div>
          ))}
        </section>

        {/* Visual Divider */}
        <div className="my-12 flex items-center gap-4">
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
          <Sparkles className="size-4 text-accent/40" />
          <div className="flex-1 h-px bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        {/* Final CTA Section */}
        <section className="pb-20 pt-8">
          <div className="rounded-2xl border border-accent/20 bg-gradient-to-br from-accent/5 to-info/5 p-8 text-center sm:p-10 lg:p-12">
            <h2 className="font-serif text-3xl text-tx-1 sm:text-4xl">
              Hire your first digital employee
            </h2>
            <p className="mt-4 max-w-2xl mx-auto text-base leading-7 text-tx-2">
              Write a job spec. Validate connections. Deploy. Your agent works 24/7, audits everything, escalates when needed. No salary. No benefits. Pure productivity.
            </p>
            <div className="mt-8 flex flex-col gap-3 justify-center sm:flex-row">
              <button onClick={onEnterApp} className="btn-primary inline-flex items-center justify-center gap-2 px-6 py-3 text-base">
                Start free <ArrowRight className="size-4" />
              </button>
              <button onClick={onSignIn} className="btn-secondary inline-flex items-center justify-center gap-2 px-6 py-3 text-base">
                Sign in to workspace
              </button>
            </div>
            <p className="mt-6 text-xs text-tx-4">
              No credit card required. Deploy your first agent in under 5 minutes.
            </p>
          </div>
        </section>
      </div>
    </main>
  );
}
