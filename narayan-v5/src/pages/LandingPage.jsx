import { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { AlertTriangle, ArrowRight, Bot, CheckCircle2, ClipboardCheck, Clock, Database, FileText, Globe2, Layers3, Lock, MessageSquareText, Plug, Scale, Search, ShieldCheck, Sparkles, Users, Workflow, Zap } from 'lucide-react';
import BenefitsScroller from '../components/BenefitsScroller';

const stats = [
  { value: 'Structured', label: 'Every workflow is shaped before execution.' },
  { value: 'Traceable', label: 'Every step is logged and replayable across runs.' },
  { value: 'Guided', label: 'The system uses known capabilities instead of guessing.' },
];

const pillars = [
  {
    icon: Workflow,
    title: 'Execution, not chat',
    text: 'Turn plain-language intent into structured work that the backend can validate and run safely.',
  },
  {
    icon: ShieldCheck,
    title: 'Auditable by default',
    text: 'Every workflow keeps a clear record so teams can review what happened and why.',
  },
  {
    icon: Layers3,
    title: 'Connected to every system',
    text: 'Workflows can move through tools, connectors, databases, APIs, and internal systems in one flow.',
  },
  {
    icon: Plug,
    title: 'Guided recovery',
    text: 'When something is missing, the system asks for the exact next step instead of guessing.',
  },
];

const steps = [
  { title: 'Draft the workflow', text: 'Describe the work in plain language and turn it into a clear execution plan.' },
  { title: 'Check the connections', text: 'Validate tools, data sources, APIs, and approval rules before launch.' },
  { title: 'Run and recover', text: 'Execute the workflow, capture history, and repair only what is missing.' },
];

const riskSignals = [
  {
    label: 'Workflow pattern',
    value: 'Each workflow keeps a consistent shape that can be compared against similar jobs.',
  },
  {
    label: 'Pre-launch checks',
    value: 'The system can compare a new workflow against past outcomes and flag likely risks before launch.',
  },
  {
    label: 'Actionable fixes',
    value: 'Warnings can point to the step and suggest a safer change before anything runs.',
  },
];

const examples = [
  {
    icon: FileText,
    tag: 'Finance',
    title: 'Invoice Processor',
    trigger: 'Gmail webhook',
    connectors: ['Gmail', 'QuickBooks', 'Workspace'],
    summary: 'Process incoming invoices, match purchase orders, post approved items, and flag exceptions.',
    outline: ['Read invoice from email', 'Match purchase order', 'Post approved items', 'Flag mismatches'],
    metrics: ['47 processed', '3 escalated', '$847.3K handled'],
    accent: 'from-emerald-500/20 to-emerald-600/5',
    border: 'border-emerald-500/20',
  },
  {
    icon: MessageSquareText,
    tag: 'Support',
    title: 'Support Ticket Responder',
    trigger: 'Zendesk ticket_created',
    connectors: ['Zendesk', 'Docs', 'Slack'],
    summary: 'Triage and respond to support tickets, summarize the issue, search docs, and draft the reply.',
    outline: ['Fetch ticket history', 'Search knowledge base', 'Draft response', 'Route to human if needed'],
    metrics: ['24 drafts', '6 escalations', '12s avg'],
    accent: 'from-blue-500/20 to-blue-600/5',
    border: 'border-blue-500/20',
  },
  {
    icon: Scale,
    tag: 'Legal',
    title: 'Contract Risk Reviewer',
    trigger: 'User uploads contract',
    connectors: ['Workspace'],
    summary: 'Review contracts for risk, extract clauses, flag severity, and return a plain-language summary.',
    outline: ['Read contract PDF', 'Extract key terms', 'Rate clause severity', 'Save summary'],
    metrics: ['5 flags', '1 page', 'Saved'],
    accent: 'from-amber-500/20 to-amber-600/5',
    border: 'border-amber-500/20',
  },
  {
    icon: Search,
    tag: 'Sales',
    title: 'Prospect Researcher',
    trigger: 'User request',
    connectors: ['Web', 'CRM', 'Workspace'],
    summary: 'Research prospects before outreach, gather company data, identify decision-makers, and write the angle.',
    outline: ['Clarify target', 'Search web and LinkedIn', 'Compile findings', 'Write recommendation'],
    metrics: ['5 prospects', '3+ sources', 'Ready'],
    accent: 'from-violet-500/20 to-violet-600/5',
    border: 'border-violet-500/20',
  },
];

const PIPELINE_STEPS = [
  { label: 'Ingest batch' },
  { label: 'Score anomalies' },
  { label: 'Route review' },
  { label: 'Freeze & log' },
];

const LOG_LINES = [
  { t: '14:02:11', msg: 'Payment batch TXN-9913 received · 4,218 records', ok: false },
  { t: '14:02:12', msg: 'Anomaly scan: 11 flagged · risk score > 0.87', ok: false },
  { t: '14:02:13', msg: 'Escalation → compliance-review-agent · handoff sent ✓', ok: true },
  { t: '14:02:15', msg: 'Compliance ACK received · hold_and_review instruction', ok: true },
  { t: '14:02:16', msg: 'Accounts frozen: ACC-4471, ACC-8823, ACC-0091', ok: false },
  { t: '14:02:17', msg: 'Audit trail written · case ID: FR-2024-00441 ✓', ok: true },
];

const CONNECTORS = ['Stripe', 'Data Engine', 'Workflow Handoff', 'Compliance DB'];


function CommandSurface() {
  const [activeStep, setActiveStep] = useState(0);
  const [logCount, setLogCount]     = useState(1);
  const [secs, setSecs]             = useState(0);

  useEffect(() => {
    const tick = window.setInterval(() => {
      setActiveStep(s => (s + 1) % PIPELINE_STEPS.length);
      setLogCount(c => (c >= LOG_LINES.length ? 1 : c + 1));
      setSecs(s => s + 2);
    }, 2000);
    return () => window.clearInterval(tick);
  }, []);

  const uptime = `${String(Math.floor(secs / 60)).padStart(2, '0')}m ${String(secs % 60).padStart(2, '0')}s`;

  return (
    <motion.div
      animate={{ y: [0, -7, 0] }}
      transition={{ duration: 7, repeat: Infinity, ease: 'easeInOut' }}
      className="relative overflow-hidden rounded-[2rem] border border-white/10 bg-[#100e0c] text-white shadow-[0_40px_100px_rgba(0,0,0,0.35)]"
    >
      {/* Subtle ambient glows — kept minimal */}
      <div className="pointer-events-none absolute inset-0"
        style={{ background: 'radial-gradient(ellipse at 20% 0%, rgba(201,106,46,0.18) 0%, transparent 50%), radial-gradient(ellipse at 80% 100%, rgba(59,130,246,0.1) 0%, transparent 45%)' }}
      />

      {/* ── HEADER ───────────────────────────────────────────────────── */}
      <div className="relative flex items-center justify-between border-b border-white/[0.08] px-5 py-4">
        <div>
          <p className="text-[0.58rem] font-bold uppercase tracking-[0.32em] text-white/30">Agent · Risk &amp; Compliance</p>
          <p className="mt-0.5 text-base font-semibold text-white/85">Payment Fraud Detection</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-3 py-1">
            <motion.span
              animate={{ opacity: [1, 0.2, 1] }}
              transition={{ duration: 1.4, repeat: Infinity }}
              className="size-1.5 rounded-full bg-emerald-400"
            />
            <span className="text-[0.65rem] font-semibold text-emerald-400">RUNNING</span>
          </div>
          <span className="font-mono text-xs text-white/30">{uptime}</span>
        </div>
      </div>

      {/* ── PIPELINE ─────────────────────────────────────────────────── */}
      <div className="relative px-5 py-5">
        <p className="mb-4 text-[0.58rem] font-bold uppercase tracking-[0.28em] text-white/25">Workflow steps</p>
        <div className="flex items-center">
          {PIPELINE_STEPS.map((step, i) => {
            const done    = i < activeStep;
            const current = i === activeStep;
            return (
              <div key={step.label} className="flex flex-1 items-center last:flex-none">
                {/* Node */}
                <div className="flex flex-col items-center gap-1.5">
                  <motion.div
                    animate={current ? { boxShadow: ['0 0 0px rgba(224,117,64,0)', '0 0 14px rgba(224,117,64,0.55)', '0 0 0px rgba(224,117,64,0)'] } : {}}
                    transition={{ duration: 1.6, repeat: Infinity }}
                    className={`flex size-8 items-center justify-center rounded-full border text-xs font-bold transition-all duration-500
                      ${done    ? 'border-emerald-500/50 bg-emerald-500/20 text-emerald-400' : ''}
                      ${current ? 'border-amber-500/60  bg-amber-500/20  text-amber-400' : ''}
                      ${!done && !current ? 'border-white/10 bg-white/5 text-white/25' : ''}`}
                  >
                    {done ? '✓' : i + 1}
                  </motion.div>
                  <p className={`text-[0.58rem] whitespace-nowrap font-medium transition-colors duration-300
                    ${done || current ? 'text-white/65' : 'text-white/22'}`}>
                    {step.label}
                  </p>
                </div>

                {/* Connector */}
                {i < PIPELINE_STEPS.length - 1 && (
                  <div className="relative mx-2 h-px flex-1">
                    {/* Track */}
                    <div className={`absolute inset-0 rounded-full transition-colors duration-500 ${done ? 'bg-emerald-500/35' : 'bg-white/8'}`} />
                    {/* Travelling dot */}
                    {current && (
                      <motion.div
                        className="absolute top-1/2 size-2 -translate-y-1/2 rounded-full bg-amber-400"
                        style={{ boxShadow: '0 0 8px rgba(224,117,64,0.8)' }}
                        animate={{ left: ['0%', '100%'], opacity: [0.2, 1, 0.2] }}
                        transition={{ duration: 1.2, repeat: Infinity, ease: 'easeInOut' }}
                      />
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* ── BODY: Log + Stats ────────────────────────────────────────── */}
      <div className="relative grid grid-cols-[1fr_100px] border-t border-white/[0.08]">
        {/* Log panel */}
        <div className="border-r border-white/[0.08] px-5 py-4">
          <p className="mb-3 text-[0.58rem] font-bold uppercase tracking-[0.28em] text-white/25">Live output</p>
          <div className="space-y-2 font-mono">
            {LOG_LINES.slice(0, logCount).map((line, i) => (
              <motion.div
                key={i}
                initial={{ opacity: 0, x: -10 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ duration: 0.35 }}
                className="flex gap-2.5 text-[0.7rem]"
              >
                <span className="shrink-0 text-white/22">{line.t}</span>
                <span className={line.ok ? 'text-emerald-400/90' : 'text-white/60'}>{line.msg}</span>
              </motion.div>
            ))}
            {/* Blinking cursor */}
            <motion.span
              animate={{ opacity: [1, 0, 1] }}
              transition={{ duration: 0.8, repeat: Infinity }}
              className="inline-block text-[0.7rem] text-amber-400/70"
            >▌</motion.span>
          </div>
        </div>

        {/* Stats sidebar */}
        <div className="flex flex-col justify-center gap-4 px-4 py-4">
          {[
            { val: '11',    sub: 'flagged' },
            { val: '$4.2M', sub: 'protected' },
            { val: '3',     sub: 'frozen' },
          ].map(({ val, sub }) => (
            <div key={sub}>
              <p className="font-serif text-xl font-semibold text-amber-400/90 leading-none">{val}</p>
              <p className="mt-0.5 text-[0.58rem] text-white/30">{sub}</p>
            </div>
          ))}
        </div>
      </div>

      {/* ── FOOTER: Connected systems ─────────────────────────────────── */}
      <div className="relative flex items-center gap-3 border-t border-white/[0.08] px-5 py-3">
        <Zap className="size-3 shrink-0 text-amber-400/60" />
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          {CONNECTORS.map(c => (
            <div key={c} className="flex items-center gap-1.5">
              <motion.span
                animate={{ opacity: [0.5, 1, 0.5] }}
                transition={{ duration: 2.2, repeat: Infinity, delay: Math.random() * 1.5 }}
                className="size-1.5 rounded-full bg-emerald-400"
              />
              <span className="text-[0.65rem] text-white/40">{c}</span>
            </div>
          ))}
        </div>
      </div>
    </motion.div>
  );
}



function SectionHeading({ eyebrow, title, text }) {
  return (
    <div className="max-w-2xl">
      <p className="text-[0.7rem] font-semibold uppercase tracking-[0.28em] text-accent">{eyebrow}</p>
      <h2 className="mt-3 font-serif text-3xl text-tx-1 sm:text-4xl">{title}</h2>
      <p className="mt-3 text-base leading-7 text-tx-2">{text}</p>
    </div>
  );
}

function PillarRow({ icon: Icon, title, text, index }) {
  return (
    <article className="border-t border-border py-5 first:border-t-0 first:pt-0">
      <div className="flex items-start gap-4">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-2xl bg-accent-soft text-accent">
          <Icon className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-[0.7rem] uppercase tracking-[0.24em] text-tx-4">0{index + 1}</p>
          <h3 className="mt-1 text-lg font-medium text-tx-1">{title}</h3>
          <p className="mt-2 max-w-xl text-sm leading-6 text-tx-2">{text}</p>
        </div>
      </div>
    </article>
  );
}

function ExampleCard({ example }) {
  const Icon = example.icon;

  return (
    <motion.div
      initial={{ opacity: 0, y: 16 }}
      whileInView={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4 }}
      viewport={{ once: true }}
      className={`group relative overflow-hidden rounded-[1.75rem] border ${example.border} bg-bg-card/90 p-5 shadow-card transition-all duration-300 hover:shadow-lg hover:border-opacity-40`}
    >
      <div className={`absolute inset-0 bg-gradient-to-br ${example.accent} opacity-0 group-hover:opacity-100 transition-opacity duration-500`} />
      <div className="relative">
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

        <div className="mt-5 pt-4 border-t border-border/50">
          <div className="grid grid-cols-3 gap-2">
            {example.metrics.map(metric => (
              <div key={metric} className="text-center">
                <p className="text-xs font-medium text-tx-2">{metric}</p>
              </div>
            ))}
          </div>
        </div>
      </div>
    </motion.div>
  );
}

/* ─── ACP Flow Diagram ────────────────────────────────────────────────────── */
function ACPDiagram() {
  const W = 520, H = 410, CX = 260;
  const TRIG_CY = 46, ORCH_CY = 155, AGENT_CY = 292, OUT_CY = 380;
  const TRIG = { w: 130, h: 36 };
  const ORCH = { w: 160, h: 54 };
  const AGT  = { w: 82,  h: 42 };
  const OUT  = { w: 150, h: 30 };
  const agents = [
    { label: ['Invoice', 'Agent'], color: '#e07540', x: 66  },
    { label: ['Search',  'Agent'], color: '#5b9cf6', x: 188 },
    { label: ['Notify',  'Agent'], color: '#9b72f5', x: 332 },
    { label: ['Audit',   'Agent'], color: '#34d399', x: 454 },
  ];
  const trigBot = TRIG_CY + TRIG.h / 2;
  const orchTop = ORCH_CY - ORCH.h / 2;
  const orchBot = ORCH_CY + ORCH.h / 2;
  const agtTop  = AGENT_CY - AGT.h / 2;
  const agtBot  = AGENT_CY + AGT.h / 2;
  const outTop  = OUT_CY - OUT.h / 2;

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      whileInView={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.6 }}
      viewport={{ once: true }}
      className="relative flex h-full flex-col overflow-hidden rounded-[2rem] border border-white/10 bg-[#0d0b09]"
      style={{ minHeight: 0 }}
    >
      {/* Header */}
      <div className="relative flex items-center justify-between border-b border-white/[0.07] px-5 py-3.5">
        <div>
          <p className="text-[0.58rem] font-bold uppercase tracking-[0.3em] text-white/30">ACP Execution Flow</p>
          <p className="mt-0.5 text-sm font-semibold text-white/75">How a job runs through Narayan</p>
        </div>
        <div className="flex items-center gap-2">
          <motion.span
            animate={{ opacity: [1, 0.25, 1] }}
            transition={{ duration: 1.8, repeat: Infinity }}
            className="size-2 rounded-full bg-emerald-400"
          />
          <span className="text-[0.68rem] text-white/35">4 agents active</span>
        </div>
      </div>

      {/* SVG diagram */}
      <div className="relative flex-1 px-2 pb-3 pt-2">
        <svg viewBox={`0 0 ${W} ${H}`} className="h-full w-full" style={{ display: 'block' }}>
          <defs>
            <marker id="aAmber" markerWidth="7" markerHeight="7" refX="5" refY="3.5" orient="auto">
              <path d="M0,0.5 L6,3.5 L0,6.5 Z" fill="rgba(224,117,64,0.85)" />
            </marker>
            <marker id="aWhite" markerWidth="7" markerHeight="7" refX="5" refY="3.5" orient="auto">
              <path d="M0,0.5 L6,3.5 L0,6.5 Z" fill="rgba(255,255,255,0.3)" />
            </marker>
            {agents.map(({ color }, i) => (
              <marker key={i} id={`aa${i}`} markerWidth="7" markerHeight="7" refX="5" refY="3.5" orient="auto">
                <path d="M0,0.5 L6,3.5 L0,6.5 Z" fill={color + 'cc'} />
              </marker>
            ))}
            <radialGradient id="og" cx="50%" cy="50%" r="50%">
              <stop offset="0%"   stopColor="rgba(201,106,46,0.35)" />
              <stop offset="100%" stopColor="rgba(201,106,46,0)"    />
            </radialGradient>
          </defs>

          {/* Step labels (left margin) */}
          {[{ y: 95, t: '① RECEIVE' }, { y: 225, t: '② DELEGATE' }, { y: 340, t: '③ STREAM BACK' }].map(({ y, t }) => (
            <text key={t} x={14} y={y} fill="rgba(255,255,255,0.15)" fontSize="7" fontWeight="800" letterSpacing="1">{t}</text>
          ))}

          {/* ── JOB TRIGGER ──────────────────────────────────────────── */}
          <rect x={CX - TRIG.w / 2} y={TRIG_CY - TRIG.h / 2} width={TRIG.w} height={TRIG.h} rx={10}
            fill="rgba(255,255,255,0.05)" stroke="rgba(255,255,255,0.2)" strokeWidth="1.5" />
          <text x={CX} y={TRIG_CY - 5} textAnchor="middle" fill="rgba(255,255,255,0.75)" fontSize="9.5" fontWeight="700" letterSpacing="1.2">JOB TRIGGER</text>
          <text x={CX} y={TRIG_CY + 8} textAnchor="middle" fill="rgba(255,255,255,0.28)" fontSize="7.5">webhook · schedule · user</text>

          {/* Trigger → Orch line */}
          <line x1={CX} y1={trigBot + 2} x2={CX} y2={orchTop - 5}
            stroke="rgba(201,106,46,0.55)" strokeWidth="1.5" markerEnd="url(#aAmber)" />
          <text x={CX + 9} y={(trigBot + orchTop) / 2 + 4} fill="rgba(201,106,46,0.45)" fontSize="7.5" fontWeight="700" letterSpacing="0.8">ACP received</text>

          {/* Dot: trigger → orch */}
          <motion.circle r="4.5" fill="#e07540"
            style={{ filter: 'drop-shadow(0 0 6px #e07540)' }}
            animate={{ cx: [CX, CX], cy: [trigBot + 4, orchTop - 7], opacity: [0, 1, 1, 0] }}
            transition={{ duration: 1.4, repeat: Infinity, ease: 'easeInOut', repeatDelay: 1 }}
          />

          {/* ── ORCHESTRATOR ─────────────────────────────────────────── */}
          <motion.rect
            x={CX - ORCH.w / 2 - 12} y={orchTop - 12}
            width={ORCH.w + 24} height={ORCH.h + 24} rx={24}
            fill="url(#og)"
            animate={{ opacity: [0.35, 0.85, 0.35] }}
            transition={{ duration: 2.6, repeat: Infinity }}
          />
          <rect x={CX - ORCH.w / 2} y={orchTop} width={ORCH.w} height={ORCH.h} rx={16}
            fill="rgba(201,106,46,0.18)" stroke="rgba(201,106,46,0.7)" strokeWidth="2" />
          <text x={CX} y={ORCH_CY - 7} textAnchor="middle" fill="#e07540" fontSize="12" fontWeight="800" letterSpacing="1.6">NARAYAN</text>
          <text x={CX} y={ORCH_CY + 10} textAnchor="middle" fill="rgba(201,106,46,0.55)" fontSize="8.5" letterSpacing="1.2">ORCHESTRATOR</text>

          {/* ── DELEGATE SPOKES ──────────────────────────────────────── */}
          <text x={14} y={235} fill="rgba(224,117,64,0.4)" fontSize="7.5" fontWeight="700">ACP delegate →</text>

          {agents.map(({ x, color }, i) => (
            <g key={`dn-${i}`}>
              <line x1={CX} y1={orchBot + 2} x2={x} y2={agtTop - 4}
                stroke={color} strokeWidth="1.5" strokeOpacity="0.5" markerEnd={`url(#aa${i})`} />
              <motion.circle r="4" fill={color}
                style={{ filter: `drop-shadow(0 0 6px ${color})` }}
                animate={{ cx: [CX, x], cy: [orchBot + 4, agtTop - 6], opacity: [0, 1, 1, 0] }}
                transition={{ duration: 1.7, repeat: Infinity, delay: i * 0.48, ease: 'easeInOut', repeatDelay: 1.3 }}
              />
            </g>
          ))}

          {/* ── AGENT NODES ──────────────────────────────────────────── */}
          {agents.map(({ label, color, x }) => (
            <g key={`ag-${x}`}>
              <rect x={x - AGT.w / 2} y={agtTop} width={AGT.w} height={AGT.h} rx={12}
                fill={color + '1c'} stroke={color + '72'} strokeWidth="1.5"
                style={{ filter: `drop-shadow(0 0 12px ${color}2a)` }}
              />
              <text x={x} y={AGENT_CY - 6} textAnchor="middle" fill={color} fontSize="9" fontWeight="700" letterSpacing="0.5">{label[0]}</text>
              <text x={x} y={AGENT_CY + 9} textAnchor="middle" fill={color + '99'} fontSize="8">{label[1]}</text>
            </g>
          ))}

          {/* ── STREAM BACK ──────────────────────────────────────────── */}
          <text x={378} y={248} fill="rgba(100,200,140,0.38)" fontSize="7.5" fontWeight="700">← ACP stream</text>

          {agents.map(({ x, color }, i) => (
            <g key={`up-${i}`}>
              <line x1={x} y1={agtBot + 2} x2={CX} y2={orchBot + 5}
                stroke={color} strokeWidth="1" strokeOpacity="0.18" />
              <motion.circle r="3" fill={color}
                style={{ opacity: 0.75, filter: `drop-shadow(0 0 4px ${color})` }}
                animate={{ cx: [x, CX], cy: [agtBot + 4, orchBot + 7], opacity: [0, 0.85, 0.85, 0] }}
                transition={{ duration: 1.7, repeat: Infinity, delay: 0.85 + i * 0.48, ease: 'easeInOut', repeatDelay: 1.3 }}
              />
            </g>
          ))}

          {/* ── OUTPUT ───────────────────────────────────────────────── */}
          <line x1={CX} y1={orchBot + 14} x2={CX} y2={outTop - 4}
            stroke="rgba(255,255,255,0.2)" strokeWidth="1.5" markerEnd="url(#aWhite)" />
          <text x={CX + 9} y={(orchBot + 26 + outTop) / 2} fill="rgba(255,255,255,0.22)" fontSize="7.5" fontWeight="600">merged &amp; dispatched</text>

          <rect x={CX - OUT.w / 2} y={outTop} width={OUT.w} height={OUT.h} rx={9}
            fill="rgba(52,211,153,0.09)" stroke="rgba(52,211,153,0.38)" strokeWidth="1.5" />
          <text x={CX} y={OUT_CY + 6} textAnchor="middle" fill="rgba(52,211,153,0.8)" fontSize="9.5" fontWeight="700" letterSpacing="1">✓ WORKFLOW COMPLETE</text>

          <motion.circle r="3.5" fill="rgba(52,211,153,0.9)"
            style={{ filter: 'drop-shadow(0 0 5px #34d399)' }}
            animate={{ cx: [CX, CX], cy: [orchBot + 16, outTop - 6], opacity: [0, 1, 1, 0] }}
            transition={{ duration: 1.2, repeat: Infinity, delay: 4, ease: 'easeInOut', repeatDelay: 5.5 }}
          />
        </svg>
      </div>
    </motion.div>
  );
}



/* ─── Broker flow diagram ─────────────────────────────────────────────────────
   Shows: Any external agent → Narayan Broker (governance) → Internal workflow
   Narayan sits in the middle. Neither party needs to know the other internally.
────────────────────────────────────────────────────────────────────────────── */
const BROKER_LOG = [
  { t: '11:22:01', msg: 'External agent registered · platform: LangGraph · status: active', ok: true },
  { t: '11:22:14', msg: 'Handshake v2 · credit-check accepted by both parties ✓', ok: true },
  { t: '11:22:15', msg: 'Inbound envelope ENV-9921 · Ed25519 signature verified ✓', ok: true },
  { t: '11:22:15', msg: 'Data barrier: PII scan passed · SSN field redacted → hash', ok: true },
  { t: '11:22:16', msg: 'Approval policy: auto-approved · amount $42K within threshold', ok: true },
  { t: '11:22:16', msg: 'Routed to internal workflow · step: credit-decision-agent', ok: true },
  { t: '11:22:31', msg: 'Response: ApprovedWithConditions · delivered via webhook ✓', ok: true },
  { t: '11:22:31', msg: 'Bilateral audit written · chain hash verified on both sides ✓', ok: true },
];

const BROKER_PHASES = ['Register', 'Handshake', 'Receive', 'Govern', 'Route', 'Respond'];

function BrokerDiagram() {
  const [phase, setPhase] = useState(0);
  const [logCount, setLogCount] = useState(1);

  useEffect(() => {
    const t = setInterval(() => {
      setPhase(p => (p + 1) % BROKER_PHASES.length);
      setLogCount(c => (c >= BROKER_LOG.length ? 1 : c + 1));
    }, 1800);
    return () => clearInterval(t);
  }, []);

  const externalPlatforms = ['LangGraph', 'CrewAI', 'Custom ACP', 'n8n'];

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }} whileInView={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.6 }} viewport={{ once: true }}
      className="relative overflow-hidden rounded-[2rem] border border-white/10 bg-[#0b0908] text-white"
    >
      <div className="pointer-events-none absolute inset-0" style={{
        background: 'radial-gradient(ellipse at 50% 30%, rgba(99,102,241,0.18) 0%, transparent 60%), radial-gradient(ellipse at 20% 80%, rgba(234,179,8,0.08) 0%, transparent 50%)'
      }} />

      {/* Header */}
      <div className="flex items-center justify-between border-b border-white/[0.07] px-5 py-3.5">
        <div>
          <p className="text-[0.58rem] font-bold uppercase tracking-[0.3em] text-white/30">Universal Boundary Broker</p>
          <p className="mt-0.5 text-sm font-semibold text-white/75">Any agent, any platform — governed through Narayan</p>
        </div>
        <div className="flex items-center gap-2">
          <motion.span animate={{ opacity: [1, 0.2, 1] }} transition={{ duration: 1.4, repeat: Infinity }}
            className="size-2 rounded-full bg-indigo-400" />
          <span className="text-[0.68rem] text-white/35">broker active</span>
        </div>
      </div>

      {/* 3-party diagram */}
      <div className="px-5 py-5">
        <div className="grid grid-cols-[1fr_auto_1fr_auto_1fr] items-center gap-2">

          {/* Left: External agents */}
          <div className="space-y-1.5">
            <p className="mb-2 text-[0.58rem] font-bold uppercase tracking-[0.24em] text-white/25">External agents</p>
            {externalPlatforms.map((p, i) => (
              <motion.div key={p}
                animate={{ opacity: phase >= 1 && i === phase % externalPlatforms.length ? 1 : 0.35 }}
                transition={{ duration: 0.4 }}
                className={`flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-[0.65rem] font-medium
                  ${i === phase % externalPlatforms.length && phase >= 2 ? 'border-indigo-500/40 bg-indigo-500/12 text-indigo-200' : 'border-white/8 bg-white/3 text-white/40'}`}
              >
                <div className={`size-1.5 rounded-full ${i === phase % externalPlatforms.length && phase >= 2 ? 'bg-indigo-400' : 'bg-white/20'}`} />
                {p}
              </motion.div>
            ))}
          </div>

          {/* Arrow right */}
          <div className="flex flex-col items-center gap-1 px-1">
            <div className="relative h-16 w-8">
              <div className="absolute left-1/2 top-0 h-full w-px -translate-x-1/2 bg-white/8" />
              {(phase === 2 || phase === 3) && (
                <motion.div className="absolute left-1/2 size-2 -translate-x-1/2 rounded-full bg-indigo-400"
                  style={{ boxShadow: '0 0 6px rgba(99,102,241,0.9)' }}
                  animate={{ top: ['0%', '100%'], opacity: [0.2, 1, 0.2] }}
                  transition={{ duration: 0.9, repeat: Infinity }} />
              )}
            </div>
            <p className="text-[0.5rem] font-bold uppercase tracking-[0.15em] text-white/18">ACP</p>
          </div>

          {/* Center: Narayan Broker */}
          <div className="text-center">
            <motion.div
              animate={{ boxShadow: phase >= 3 ? ['0 0 0px rgba(99,102,241,0)', '0 0 20px rgba(99,102,241,0.4)', '0 0 0px rgba(99,102,241,0)'] : [] }}
              transition={{ duration: 1.6, repeat: Infinity }}
              className="mx-auto rounded-2xl border border-indigo-500/40 bg-indigo-500/15 px-3 py-3"
            >
              <p className="text-[0.6rem] font-bold uppercase tracking-[0.24em] text-indigo-300/60">Narayan</p>
              <p className="mt-0.5 text-xs font-bold text-indigo-200">Broker</p>
            </motion.div>
            {/* Governance steps */}
            <div className="mt-2 space-y-1">
              {['Verify sig', 'Data barrier', 'Rate limit', 'Approval', 'Audit'].map((s, i) => (
                <motion.div key={s} animate={{ opacity: phase >= i + 1 ? 1 : 0.2 }} transition={{ duration: 0.3 }}
                  className={`rounded-md px-2 py-0.5 text-[0.55rem] font-medium ${phase >= i + 1 ? 'bg-indigo-500/15 text-indigo-300' : 'text-white/20'}`}>
                  {s}
                </motion.div>
              ))}
            </div>
          </div>

          {/* Arrow right to internal */}
          <div className="flex flex-col items-center gap-1 px-1">
            <div className="relative h-16 w-8">
              <div className="absolute left-1/2 top-0 h-full w-px -translate-x-1/2 bg-white/8" />
              {(phase === 4 || phase === 5) && (
                <motion.div className="absolute left-1/2 size-2 -translate-x-1/2 rounded-full bg-emerald-400"
                  style={{ boxShadow: '0 0 6px rgba(52,211,153,0.9)' }}
                  animate={{ top: ['0%', '100%'], opacity: [0.2, 1, 0.2] }}
                  transition={{ duration: 0.9, repeat: Infinity }} />
              )}
            </div>
            <p className="text-[0.5rem] font-bold uppercase tracking-[0.15em] text-white/18">Internal</p>
          </div>

          {/* Right: Internal workflow */}
          <div className="space-y-1.5">
            <p className="mb-2 text-[0.58rem] font-bold uppercase tracking-[0.24em] text-white/25">Your workflow</p>
            {['Credit agent', 'Approval step', 'KYC validator', 'Response builder'].map((s, i) => (
              <motion.div key={s}
                animate={{ opacity: phase >= 4 && i === (phase - 4) % 4 ? 1 : 0.25 }}
                transition={{ duration: 0.4 }}
                className={`flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-[0.65rem] font-medium
                  ${phase >= 4 && i === (phase - 4) % 4 ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-200' : 'border-white/8 bg-white/3 text-white/40'}`}
              >
                <div className={`size-1.5 rounded-full ${phase >= 4 && i === (phase - 4) % 4 ? 'bg-emerald-400' : 'bg-white/20'}`} />
                {s}
              </motion.div>
            ))}
          </div>
        </div>

        {/* Phase track */}
        <div className="mt-4 flex gap-1">
          {BROKER_PHASES.map((p, i) => (
            <div key={p} className={`h-1 flex-1 rounded-full transition-colors duration-500 ${i < phase ? 'bg-indigo-500/55' : i === phase ? 'bg-indigo-400' : 'bg-white/8'}`} />
          ))}
        </div>
        <div className="mt-1.5 flex justify-between">
          {BROKER_PHASES.map((p, i) => (
            <p key={p} className={`text-[0.5rem] font-medium transition-colors duration-300 ${i <= phase ? 'text-white/50' : 'text-white/18'}`}>{p}</p>
          ))}
        </div>
      </div>

      {/* Live audit log */}
      <div className="border-t border-white/[0.07] px-5 py-3">
        <p className="mb-2 text-[0.58rem] font-bold uppercase tracking-[0.28em] text-white/25">Broker audit log</p>
        <div className="space-y-1 font-mono">
          {BROKER_LOG.slice(0, logCount).map((line, i) => (
            <motion.div key={i} initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }}
              transition={{ duration: 0.25 }} className="flex gap-2 text-[0.62rem]">
              <span className="shrink-0 text-white/20">{line.t}</span>
              <span className={line.ok ? 'text-emerald-400/80' : 'text-white/50'}>{line.msg}</span>
            </motion.div>
          ))}
          <motion.span animate={{ opacity: [1, 0, 1] }} transition={{ duration: 0.8, repeat: Infinity }}
            className="inline-block text-[0.62rem] text-indigo-400/60">▌</motion.span>
        </div>
      </div>
    </motion.div>
  );
}

/* ─── Feature cards ──────────────────────────────────────────────────────────── */
const BOUNDARY_FEATURES = [
  {
    icon: Globe2, badge: 'Off-platform agents',
    color: 'bg-indigo-500/10 text-indigo-400', border: 'border-indigo-500/20',
    title: 'Company B can stay on LangGraph, CrewAI, REST, or a custom ACP agent',
    body: 'Company A can run on Narayan while the other side stays on its own stack. The broker becomes the trusted medium, so neither side needs a Narayan install to connect.',
  },
  {
    icon: ClipboardCheck, badge: 'Typed handshake',
    color: 'bg-violet-500/10 text-violet-400', border: 'border-violet-500/20',
    title: 'The contract is signed before any payload moves',
    body: 'Request fields, response shape, timeout, visibility rules, and callback expectations are agreed up front. Narayan enforces the handshake before the step can run.',
  },
  {
    icon: ShieldCheck, badge: 'Structured approvals',
    color: 'bg-amber-500/10 text-amber-400', border: 'border-amber-500/20',
    title: 'Security approval is a typed workflow outcome, not an email thread',
    body: 'Approvals can be ApprovedWithConditions, PartiallyApproved, EscalatedTo, or DeferredUntil. The external agent gets structured results back, not a manual follow-up.',
  },
  {
    icon: Lock, badge: 'Bilateral audit',
    color: 'bg-emerald-500/10 text-emerald-400', border: 'border-emerald-500/20',
    title: 'Both companies keep the same chain-hashed record',
    body: 'Every envelope writes an immutable SHA-256 audit trail on both sides. Each party can verify the exchange independently, and neither side can rewrite history alone.',
  },
  {
    icon: AlertTriangle, badge: 'Data barrier',
    color: 'bg-red-500/10 text-red-400', border: 'border-red-500/20',
    title: 'PII scans, redaction, and residency checks happen before crossing',
    body: 'The broker inspects inbound and outbound envelopes, blocks residency violations, and hashes sensitive fields so only the allowed data leaves the boundary.',
  },
  {
    icon: Clock, badge: 'Long-running approvals',
    color: 'bg-sky-500/10 text-sky-400', border: 'border-sky-500/20',
    title: 'The flow can park for hours or days without losing state',
    body: 'Approval steps can pause the exchange, wait for a human, and resume when ready. The timeout policy is part of the handshake, so the workflow stays explicit.',
  },
];

/* ─── Enterprise Boundary + Broker Section ──────────────────────────────────── */
function EnterpriseBoundarySection() {
  return (
    <section className="pb-12">
      {/* Heading row */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
        <SectionHeading
          eyebrow="Universal boundary broker"
          title="Narayan sits between Company A and Company B."
          text="One side can run Narayan and the other can stay on LangGraph, CrewAI, REST, or a custom ACP agent. Narayan acts as the medium: it verifies identity, enforces the handshake, applies security approval, redacts sensitive data, and writes the audit trail."
        />
        <motion.div
          initial={{ opacity: 0, scale: 0.9 }} whileInView={{ opacity: 1, scale: 1 }}
          transition={{ duration: 0.5 }} viewport={{ once: true }}
          className="shrink-0 self-start rounded-2xl border border-indigo-500/30 bg-indigo-500/10 px-4 py-3 sm:self-auto"
        >
          <p className="text-[0.6rem] font-bold uppercase tracking-[0.28em] text-indigo-400">Industry first</p>
          <p className="mt-1 text-xs font-semibold text-indigo-200">Brokered company-to-company work, even when the other side is off-platform.</p>
        </motion.div>
      </div>

      {/* Scenarios: who can connect */}
      <motion.div
        initial={{ opacity: 0, y: 12 }} whileInView={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5, delay: 0.1 }} viewport={{ once: true }}
        className="mt-8 overflow-hidden rounded-2xl border border-indigo-500/20 bg-gradient-to-r from-indigo-500/8 via-violet-500/6 to-indigo-500/8 px-6 py-5"
      >
        <p className="mb-4 text-[0.6rem] font-bold uppercase tracking-[0.28em] text-tx-3">Narayan stays in the middle, even when only one side is on the platform</p>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {[
            { from: 'Company A on Narayan', to: 'Company B on Narayan', tag: 'Native', color: 'text-indigo-400', bg: 'bg-indigo-500/10' },
            { from: 'Company A on Narayan', to: 'Company B on LangGraph', tag: 'Brokered', color: 'text-violet-400', bg: 'bg-violet-500/10' },
            { from: 'Company A on Narayan', to: 'Company B on CrewAI', tag: 'Brokered', color: 'text-amber-400', bg: 'bg-amber-500/10' },
            { from: 'Company A on Narayan', to: 'Company B via REST', tag: 'Pure broker', color: 'text-emerald-400', bg: 'bg-emerald-500/10' },
          ].map(({ from, to, tag, color, bg }) => (
            <div key={tag} className={`rounded-xl ${bg} border border-white/8 px-3 py-2.5`}>
              <span className={`text-[0.58rem] font-bold uppercase tracking-[0.2em] ${color}`}>{tag}</span>
              <p className="mt-1.5 text-[0.7rem] font-medium text-tx-1">{from}</p>
              <div className="my-1 flex items-center gap-1">
                <div className="h-px flex-1 bg-white/10" />
                <span className="text-[0.55rem] text-white/30">Narayan broker</span>
                <div className="h-px flex-1 bg-white/10" />
              </div>
              <p className="text-[0.7rem] font-medium text-tx-1">{to}</p>
            </div>
          ))}
        </div>
      </motion.div>

      {/* Main grid: broker diagram + feature cards */}
      <div className="mt-10 grid gap-8 lg:grid-cols-[1.15fr_0.85fr]">
        <BrokerDiagram />
        <div className="space-y-3">
          {BOUNDARY_FEATURES.slice(0, 4).map(({ icon: Icon, badge, color, border, title, body }, idx) => (
            <motion.div key={badge}
              initial={{ opacity: 0, x: 16 }} whileInView={{ opacity: 1, x: 0 }}
              transition={{ duration: 0.4, delay: idx * 0.07 }} viewport={{ once: true }}
              className={`group relative overflow-hidden rounded-[1.5rem] border ${border} bg-bg-card/90 p-4 transition-all duration-300 hover:shadow-md`}
            >
              <div className="flex items-start gap-3">
                <div className={`flex size-8 shrink-0 items-center justify-center rounded-xl ${color}`}>
                  <Icon className="size-4" />
                </div>
                <div className="min-w-0">
                  <span className={`rounded-lg px-2 py-0.5 text-[0.58rem] font-bold uppercase tracking-[0.15em] ${color}`}>{badge}</span>
                  <h3 className="mt-1.5 text-sm font-semibold text-tx-1 leading-snug">{title}</h3>
                  <p className="mt-1 text-xs leading-5 text-tx-3">{body}</p>
                </div>
              </div>
            </motion.div>
          ))}
        </div>
      </div>

      {/* Bottom two wider cards */}
      <div className="mt-4 grid gap-4 md:grid-cols-2">
        {BOUNDARY_FEATURES.slice(4).map(({ icon: Icon, badge, color, border, title, body }, idx) => (
          <motion.div key={badge}
            initial={{ opacity: 0, y: 14 }} whileInView={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.4, delay: idx * 0.08 }} viewport={{ once: true }}
            className={`group relative overflow-hidden rounded-[1.5rem] border ${border} bg-bg-card/90 p-5 transition-all duration-300 hover:shadow-md`}
          >
            <div className="flex items-start gap-4">
              <div className={`flex size-10 shrink-0 items-center justify-center rounded-2xl ${color}`}>
                <Icon className="size-5" />
              </div>
              <div>
                <span className={`rounded-lg px-2.5 py-1 text-[0.6rem] font-bold uppercase tracking-[0.15em] ${color}`}>{badge}</span>
                <h3 className="mt-2 text-sm font-semibold text-tx-1">{title}</h3>
                <p className="mt-1.5 text-xs leading-5 text-tx-3">{body}</p>
              </div>
            </div>
          </motion.div>
        ))}
      </div>

      {/* Dark comparison: before vs after */}
      <motion.div
        initial={{ opacity: 0, y: 16 }} whileInView={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5, delay: 0.15 }} viewport={{ once: true }}
        className="mt-8 overflow-hidden rounded-[2rem] border border-white/10 bg-[#0f0d0b] px-8 py-7"
      >
        <div className="grid gap-6 sm:grid-cols-3">
          <div>
            <p className="text-[0.6rem] font-bold uppercase tracking-[0.28em] text-white/30">Without a broker</p>
            <div className="mt-4 space-y-2.5">
              {[
                'Direct API calls expose internal structure',
                'Manual key exchange between teams',
                'No governance on what crosses the boundary',
                'Either side can breach without detection',
                'No shared audit record',
                'Approval must be tracked in a spreadsheet',
              ].map(t => (
                <div key={t} className="flex items-center gap-2 text-xs text-white/38">
                  <div className="size-1 shrink-0 rounded-full bg-red-500/60" />
                  {t}
                </div>
              ))}
            </div>
          </div>
          <div>
            <p className="text-[0.6rem] font-bold uppercase tracking-[0.28em] text-violet-400/70">Narayan on both sides</p>
            <div className="mt-4 space-y-2.5">
              {[
                'Signed handshake, both parties accept',
                'Typed schema enforced at compile time',
                'Data barrier: PII scan + redaction',
                'Bilateral chain-hashed audit ledger',
                'Structured approval, not email',
                'Freeze or revoke unilaterally in seconds',
              ].map(t => (
                <div key={t} className="flex items-center gap-2 text-xs text-white/65">
                  <CheckCircle2 className="size-3 shrink-0 text-violet-400" />
                  {t}
                </div>
              ))}
            </div>
          </div>
          <div>
            <p className="text-[0.6rem] font-bold uppercase tracking-[0.28em] text-indigo-400/70">External agent via broker</p>
            <div className="mt-4 space-y-2.5">
              {[
                'Register once — any platform, any language',
                'Same signed handshake, same typed schema',
                'Same data barrier — Narayan enforces it',
                'Same bilateral audit ledger',
                'Same structured approval flow',
                'Webhook delivery or polling — agent chooses',
              ].map(t => (
                <div key={t} className="flex items-center gap-2 text-xs text-white/70">
                  <CheckCircle2 className="size-3 shrink-0 text-indigo-400" />
                  {t}
                </div>
              ))}
            </div>
          </div>
        </div>
      </motion.div>
    </section>
  );
}


export default function LandingPage({ onEnterApp, onSignIn }) {
  return (
    <main className="relative min-h-screen overflow-hidden bg-[radial-gradient(circle_at_top_left,_rgba(201,106,46,0.16),_transparent_28%),radial-gradient(circle_at_top_right,_rgba(59,130,246,0.12),_transparent_24%),linear-gradient(180deg,_#f9f6f2_0%,_#f4f0ea_48%,_#efe8de_100%)] text-tx-1">
      <div className="pointer-events-none absolute inset-0">
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
              <p className="text-xs uppercase tracking-[0.24em] text-tx-4">Enterprise work, orchestrated</p>
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

        <section className="grid flex-1 items-center gap-8 py-12 lg:grid-cols-[0.92fr_1.08fr] lg:py-14">
          <motion.div
            initial={{ opacity: 0, y: 18 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.45, ease: 'easeOut' }}
            className="max-w-xl"
          >
            <div className="mb-4 inline-flex items-center gap-2 rounded-full border border-border bg-bg-card/90 px-3 py-1.5 text-xs font-medium text-tx-2 shadow-card">
              <Sparkles className="size-3.5 text-accent" />
              Tell us what you need. Narayan builds the agent.
            </div>

            <h1 className="font-serif text-4xl leading-[0.95] text-tx-1 sm:text-5xl lg:text-6xl">
              Describe the job.
              <span className="block text-accent">We create the agent.</span>
            </h1>

            <p className="mt-5 max-w-lg text-base leading-7 text-tx-2 sm:text-lg">
              Agent creation is just telling Narayan what you need. Describe the outcome, connect the systems, and we will turn it into a structured workflow with approvals, audit, and recovery built in.
            </p>

            <div className="mt-7 flex flex-col gap-3 sm:flex-row">
              <button onClick={onEnterApp} className="btn-primary inline-flex items-center justify-center gap-2 px-5 py-3">
                Get started <ArrowRight className="size-4" />
              </button>
              <button onClick={onSignIn} className="btn-secondary inline-flex items-center justify-center gap-2 px-5 py-3">
                Sign in to workspace
              </button>
            </div>

            <p className="mt-4 text-sm leading-6 text-tx-3">
              No forms to learn. Just explain the task in plain language and we will shape the agent around it.
            </p>

            <div className="mt-8 grid gap-4 sm:grid-cols-3">
              {stats.map(item => (
                <div key={item.value} className="border-t border-border pt-4">
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
            <CommandSurface />
          </motion.div>
        </section>

        <section className="pb-5">
          <SectionHeading
            eyebrow="Job examples"
            title="Structured workflows. Real enterprise results."
            text="Write the workflow once. Narayan checks the plan, runs the job, and keeps the record attached."
          />

          <div className="mt-8 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
            {examples.map(example => (
              <ExampleCard key={example.title} example={example} />
            ))}
          </div>
        </section>

        <div className="my-12 flex items-center gap-4">
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
          <Layers3 className="size-4 text-accent/40" />
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        <section className="pb-12">
          <SectionHeading
            eyebrow="How agents connect"
            title="Every connection stays clear."
            text="One workflow can cross tools, connectors, databases, APIs, and internal systems without losing context."
          />

          <div className="mt-8 overflow-hidden rounded-[2rem] border border-border bg-bg-card/85">
            <div className="grid gap-0 lg:grid-cols-[1.05fr_0.95fr]">
              <div className="border-b border-border p-6 lg:border-b-0 lg:border-r">
                <p className="text-xs font-semibold uppercase tracking-[0.22em] text-accent">Inbound connectors</p>
                <div className="mt-5 grid gap-3">
                  {[
                    ['Zendesk', 'Support tickets and customer context'],
                    ['Salesforce', 'Accounts, opportunities, and notes'],
                    ['GitHub', 'Issues, pull requests, and code activity'],
                  ].map(([name, detail]) => (
                    <div key={name} className="flex items-center gap-3 rounded-2xl border border-border bg-bg px-4 py-3">
                      <div className="size-2.5 rounded-full bg-info" />
                      <div>
                        <p className="text-sm font-medium text-tx-1">{name}</p>
                        <p className="text-xs text-tx-3">{detail}</p>
                      </div>
                    </div>
                  ))}
                </div>
              </div>

              <div className="p-6">
                <p className="text-xs font-semibold uppercase tracking-[0.22em] text-accent">Outbound channels</p>
                <div className="mt-5 grid gap-3">
                  {[
                    ['Email', 'Send approvals, escalations, and summaries'],
                    ['Slack', 'Notify the right people in real time'],
                    ['Logs', 'Keep a record for review and replay'],
                  ].map(([name, detail]) => (
                    <div key={name} className="flex items-center gap-3 rounded-2xl border border-border bg-bg px-4 py-3">
                      <div className="size-2.5 rounded-full bg-accent" />
                      <div>
                        <p className="text-sm font-medium text-tx-1">{name}</p>
                        <p className="text-xs text-tx-3">{detail}</p>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </div>

          <div className="mt-6 flex flex-wrap items-center justify-center gap-3">
            {['Zendesk', 'Salesforce', 'GitHub', 'Slack', 'Gmail', 'HubSpot', 'Notion', 'Jira', 'QuickBooks', 'DocuSign', 'ServiceNow', 'PagerDuty', 'dbt Cloud', 'Greenhouse'].map((name, idx) => (
              <motion.div
                key={name}
                initial={{ opacity: 0, scale: 0.9 }}
                whileInView={{ opacity: 1, scale: 1 }}
                transition={{ duration: 0.3, delay: idx * 0.03 }}
                viewport={{ once: true }}
                className="flex items-center gap-2 rounded-full border border-border bg-bg-card/60 px-4 py-2 text-sm font-medium text-tx-2 shadow-sm hover:border-accent/30 hover:text-tx-1 transition-colors"
              >
                <div className="size-2 rounded-full bg-accent" />
                {name}
              </motion.div>
            ))}
          </div>
          <p className="mt-4 text-center text-xs text-tx-4">+ 20 more connectors with OAuth auto-injection</p>
        </section>

        <div className="my-12 flex items-center gap-4">
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
          <Workflow className="size-4 text-accent/40" />
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        <section className="pb-12">
          <SectionHeading
            eyebrow="How it works"
            title="Three steps from job spec to production."
            text="The interface keeps the sequence obvious so operators can understand what will happen before it happens."
          />
          <div className="mt-8 grid gap-4 md:grid-cols-3">
            {steps.map((step, idx) => (
              <div key={step.title} className="relative overflow-hidden rounded-[1.75rem] border border-border bg-bg-card/90 p-5">
                <div className="absolute -right-4 -top-4 flex size-14 items-center justify-center rounded-full bg-accent text-sm font-semibold text-white opacity-10">
                  {idx + 1}
                </div>
                <div className="flex items-center gap-3">
                  <div className="flex size-9 items-center justify-center rounded-full bg-accent text-xs font-semibold text-white">
                    {idx + 1}
                  </div>
                  <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent">{step.title}</p>
                </div>
                <p className="mt-4 text-sm leading-6 text-tx-2">{step.text}</p>
              </div>
            ))}
          </div>
        </section>

        <BenefitsScroller />

        <div className="my-12 flex items-center gap-4">
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
          <Layers3 className="size-4 text-accent/40" />
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        <section className="pb-12">
          <SectionHeading
            eyebrow="ACP-powered collaboration"
            title="Agents work together through ACP"
            text="Narayan uses ACP to coordinate secure handoffs between specialist agents, teams, and systems without losing progress or control."
          />

          <div className="mt-12 grid items-stretch gap-8 lg:grid-cols-2">
            {/* Left: how Narayan uses ACP */}
            <div className="space-y-4">
              {[
                {
                  badge: 'Delegation',
                  color: 'bg-accent-soft text-accent',
                  title: 'One workflow, many specialists',
                  body: 'ACP lets one workflow route each part of the job to the right specialist and bring the results back together in one place.',
                },
                {
                  badge: 'Streaming',
                  color: 'bg-info-soft text-info',
                  title: 'Live progress, not guesswork',
                  body: 'As agents work, progress and intermediate results can surface in real time so teams know what is happening as it happens.',
                },
                {
                  badge: 'Orchestration',
                  color: 'bg-vio/15 text-vio',
                  title: 'Parallel by design',
                  body: 'Complex work can fan out, wait on dependencies, and come back together in a single flow that stays easy to follow.',
                },
                {
                  badge: 'Fault isolation',
                  color: 'bg-accent-soft text-accent',
                  title: 'Safe by default',
                  body: 'If one specialist fails, Narayan can retry, escalate, or continue with a full trace attached.',
                },
              ].map(({ badge, color, title, body }) => (
                <div
                  key={badge}
                  className="group relative overflow-hidden rounded-[1.5rem] border border-border bg-bg-card/90 p-5 hover:border-accent/30 transition-all duration-300"
                >
                  <div className="flex items-start gap-4">
                    <span className={`shrink-0 rounded-xl px-2.5 py-1 text-[0.65rem] font-bold uppercase tracking-[0.15em] ${color}`}>{badge}</span>
                    <div>
                      <h3 className="text-sm font-semibold text-tx-1">{title}</h3>
                      <p className="mt-1.5 text-xs leading-6 text-tx-2">{body}</p>
                    </div>
                  </div>
                </div>
              ))}
            </div>

            {/* Right: ACP diagram */}
            <ACPDiagram />
          </div>
        </section>

        <div className="my-12 flex items-center gap-4">
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
          <Database className="size-4 text-accent/40" />
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        {/* ── ENTERPRISE BOUNDARY SECTION ──────────────────────────────────── */}
        <EnterpriseBoundarySection />

        <div className="my-12 flex items-center gap-4">
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
          <Database className="size-4 text-accent/40" />
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        <section className="pb-12">
          <SectionHeading
            eyebrow="Reliability advantage"
            title="Spot risk before the first run."
            text="Because Narayan validates workflow structure ahead of time, it can compare similar jobs and surface likely issues before a user clicks run."
          />

          <div className="mt-8 grid gap-4 lg:grid-cols-[1.1fr_0.9fr]">
            <div className="space-y-4">
              {riskSignals.map(({ label, value }) => (
                <div key={label} className="rounded-[1.5rem] border border-border bg-bg-card/90 p-5">
                  <p className="text-xs font-semibold uppercase tracking-[0.22em] text-accent">{label}</p>
                  <p className="mt-3 text-sm leading-6 text-tx-2">{value}</p>
                </div>
              ))}
            </div>

            <div className="rounded-[1.75rem] border border-accent/20 bg-gradient-to-br from-bg-card via-bg-card/95 to-accent/5 p-6">
              <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent">Example warning</p>
              <h3 className="mt-3 font-serif text-2xl text-tx-1">Step 4 may fail on missing input</h3>
              <p className="mt-3 text-sm leading-6 text-tx-2">
                Similar workflows with the same shape show a recurring failure at an aggregation step when upstream records do not include a required field.
              </p>
              <div className="mt-5 space-y-3">
                {[
                  'Insert a check before aggregation',
                  'Mark the field as optional where appropriate',
                  'Add a fallback step for incomplete records',
                ].map(item => (
                  <div key={item} className="flex items-center gap-2 text-sm text-tx-2">
                    <CheckCircle2 className="size-4 text-accent" />
                    {item}
                  </div>
                ))}
              </div>
            </div>
          </div>
        </section>

        <div className="my-12 flex items-center gap-4">
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
          <ShieldCheck className="size-4 text-accent/40" />
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        <section className="pb-5"><div className="grid gap-6 lg:grid-cols-2">
          {pillars.map(({ icon: Icon, title, text }, index) => (
            <motion.div key={title} initial={{ opacity: 0, y: 20 }} whileInView={{ opacity: 1, y: 0 }} transition={{ duration: 0.5, delay: index * 0.1 }} className="group relative overflow-hidden rounded-[2rem] border border-border/50 bg-gradient-to-br from-bg-card/80 via-bg-card/60 to-bg/40 p-8 hover:border-accent/30 transition-all duration-300" >
                <div className="absolute -right-12 -top-12 size-40 rounded-full bg-accent/5 blur-3xl group-hover:bg-accent/10 transition-all duration-500" />
                <div className="absolute -bottom-20 -left-20 size-48 rounded-full bg-info/5 blur-3xl group-hover:bg-info/8 transition-all duration-500" />
                
                <div className="relative z-10">
                  <div className="flex items-start justify-between gap-4 mb-4">
                    <div className="flex size-14 items-center justify-center rounded-2xl bg-gradient-to-br from-accent-soft to-accent/20 text-accent shadow-lg">
                      <Icon className="size-6" />
                    </div>
                    <div className="text-5xl font-serif text-accent/15 font-bold">0{index + 1}</div>
                  </div>
                  <h2 className="text-2xl font-medium text-tx-1 mb-3 leading-tight">{title}</h2>
                  <p className="text-base leading-7 text-tx-2">{text}</p>
                  
                  <div className="mt-6 flex items-center text-xs font-semibold uppercase tracking-[0.2em] text-accent/70 opacity-0 group-hover:opacity-100 transition-all duration-300">
                    <span>Learn more</span>
                    <ArrowRight className="size-3.5 ml-2 transform group-hover:translate-x-1 transition-transform" />
                  </div>
                </div>
              </motion.div>
            ))}
          </div>
        </section>

        <div className="my-12 flex items-center gap-4">
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
          <Sparkles className="size-4 text-accent/40" />
          <div className="h-px flex-1 bg-gradient-to-r from-transparent via-border to-transparent" />
        </div>

        <section className="pb-20 pt-8">
          <div className="rounded-[2rem] border border-accent/20 bg-[linear-gradient(135deg,_rgba(201,106,46,0.08),_rgba(59,130,246,0.04))] p-8 sm:p-10 lg:p-12">
            <div className="grid gap-8 lg:grid-cols-[1fr_auto] lg:items-center">
              <div className="max-w-2xl">
                <p className="text-xs font-semibold uppercase tracking-[0.28em] text-accent">Final CTA</p>
                <h2 className="mt-3 font-serif text-3xl text-tx-1 sm:text-4xl">
                  Build your first workflow
                </h2>
                <p className="mt-4 text-base leading-7 text-tx-2">
                  Write a workflow spec. Validate the connections. Launch. Narayan keeps the record, highlights risk, and escalates when needed.
                </p>
              </div>
              <div className="flex flex-col gap-3 sm:flex-row lg:flex-col">
                <button onClick={onEnterApp} className="btn-primary inline-flex items-center justify-center gap-2 px-6 py-3 text-base">
                  Start free <ArrowRight className="size-4" />
                </button>
                <button onClick={onSignIn} className="btn-secondary inline-flex items-center justify-center gap-2 px-6 py-3 text-base">
                  Sign in to workspace
                </button>
              </div>
            </div>
            <p className="mt-6 text-xs text-tx-4">No credit card required. Deploy your first agent in under 5 minutes.</p>
          </div>
        </section>
      </div>
    </main>
  );
}














