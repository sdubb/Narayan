import { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { ArrowRight, Bot, CheckCircle2, Database, FileText, Layers3, MessageSquareText, Plug, Scale, Search, ShieldCheck, Sparkles, Workflow, Zap, } from 'lucide-react';
import BenefitsScroller from '../components/BenefitsScroller';

const stats = [
  { value: 'Cloud', label: '100% cloud-based with zero upfront setup or infrastructure costs.' },
  { value: 'Secure', label: 'Enterprise-grade security that open-source alternatives cannot match.' },
  { value: 'Simple', label: 'Just tell us what you need done, and we will do it.' },
];

const pillars = [
  {
    icon: Workflow,
    title: 'We improve your company',
    text: 'Scale operations without adding headcount. We turn plain-English instructions into fully deployed digital employees, transforming how your business operates.',
  },
  {
    icon: ShieldCheck,
    title: 'Unmatched security',
    text: 'Enterprise-grade security that open-source tools simply cannot provide. Every action is recorded, auditable, and strictly sandboxed.',
  },
  {
    icon: Layers3,
    title: 'Cloud-based, zero setup costs',
    text: 'No clunky infrastructure to deploy or servers to manage. Narayan operates entirely in the cloud, so you can skip the setup time and upfront capital.',
  },
  {
    icon: Plug,
    title: 'Tell us what to do',
    text: 'Just tell us what needs to be done, and we will do it. Our agents instantly wire themselves to execute your requested jobs 24/7.',
  },
];

const steps = [
  { title: 'Write the job', text: 'Describe what you need done in plain language and define the approval rules.' },
  { title: 'Check the setup', text: 'Validate connectors, test permissions, and simulate the run before launch.' },
  { title: 'It runs 24/7', text: 'The agent executes the job on its own and keeps a full log.' },
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

function CommandSurface() {
  const [stage, setStage] = useState(0);

  useEffect(() => {
    const id = window.setInterval(() => {
      setStage(prev => (prev + 1) % 3);
    }, 2400);
    return () => window.clearInterval(id);
  }, []);

  const stageItems = [
    {
      title: 'Job spec',
      copy: 'Process incoming invoices, route exceptions, and post approved items automatically.',
      status: 'Ready to validate',
    },
    {
      title: 'Execution plan',
      copy: 'Connect Gmail, QuickBooks, and Workspace. Verify permissions before the first run.',
      status: 'Checks passing',
    },
    {
      title: 'Audit trail',
      copy: 'Every action stays replayable, timestamped, and attached to the original decision.',
      status: 'Logging live',
    },
  ];

  return (
    <motion.div
      animate={{ y: [0, -8, 0] }}
      transition={{ duration: 6, repeat: Infinity, ease: "easeInOut" }}
      className="relative overflow-hidden rounded-[2rem] border border-border/60 bg-[#171311] p-4 text-white shadow-[0_30px_80px_rgba(26,23,20,0.2)]"
    >
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_top_left,_rgba(201,106,46,0.32),_transparent_28%),radial-gradient(circle_at_bottom_right,_rgba(59,130,246,0.18),_transparent_24%)] opacity-80" />
      <div className="absolute inset-x-8 top-10 h-px bg-gradient-to-r from-transparent via-white/15 to-transparent" />
      <motion.div
        animate={{ opacity: [0.3, 0.6, 0.3] }}
        transition={{ duration: 3, repeat: Infinity }}
        className="absolute inset-0 bg-[radial-gradient(circle_at_center,_rgba(201,106,46,0.15),_transparent_50%)]"
      />

      <div className="relative">
        <div className="flex items-center justify-between gap-4 border-b border-white/10 pb-4">
          <div>
            <p className="text-[0.7rem] uppercase tracking-[0.28em] text-white/45">Live workflow</p>
            <p className="mt-1 text-lg font-medium">Narayan operations view</p>
          </div>
          <div className="rounded-full border border-white/10 bg-white/5 px-3 py-1 text-xs text-white/75">
            {stageItems[stage].status}
          </div>
        </div>

        <div className="grid gap-5 py-5 lg:grid-cols-[1.05fr_0.95fr]">
          <div className="rounded-[1.5rem] border border-white/10 bg-white/[0.03] p-5">
            <p className="text-[0.7rem] uppercase tracking-[0.28em] text-white/45">Current step</p>
            <p className="mt-4 text-2xl leading-tight text-white">{stageItems[stage].copy}</p>

            <div className="mt-6 flex items-center gap-3 text-sm text-white/70">
              <span className="size-2 rounded-full bg-emerald-400" />
              Checks are on
              <span className="size-1.5 rounded-full bg-white/20" />
              Replay available
            </div>

            <div className="mt-6 space-y-3">
              {['Email intake', 'Role routing', 'Approval logic'].map((item, idx) => (
                <div key={item} className="flex items-center gap-3 rounded-2xl border border-white/8 bg-black/20 px-4 py-3">
                  <span className="flex size-7 items-center justify-center rounded-full bg-white/10 text-xs font-semibold text-white/80">
                    {idx + 1}
                  </span>
                  <span className="text-sm text-white/85">{item}</span>
                  <span className="ml-auto text-xs text-white/45">queued</span>
                </div>
              ))}
            </div>
          </div>

          <div className="rounded-[1.5rem] border border-white/10 bg-white/[0.03] p-5">
            <p className="text-[0.7rem] uppercase tracking-[0.28em] text-white/45">Connected systems</p>
            <div className="mt-4 flex flex-wrap gap-2">
              {['Gmail', 'QuickBooks', 'Workspace', 'Slack'].map(item => (
                <span key={item} className="rounded-full border border-white/10 bg-white/5 px-3 py-1 text-xs text-white/70">
                  {item}
                </span>
              ))}
            </div>

            <div className="mt-6 rounded-[1.5rem] border border-white/10 bg-black/20 p-4">
              <p className="text-[0.7rem] uppercase tracking-[0.28em] text-white/45">Workflow state</p>
              <div className="mt-4 space-y-4">
                {steps.map((item, idx) => (
                  <div key={item.title} className="flex gap-3">
                    <div className="flex flex-col items-center">
                      <div className={`size-2.5 rounded-full ${idx <= stage ? 'bg-amber-300' : 'bg-white/20'}`} />
                      {idx < steps.length - 1 ? <div className="mt-2 h-10 w-px bg-white/10" /> : null}
                    </div>
                    <div className="pb-4">
                      <p className="text-sm font-medium text-white">{item.title}</p>
                      <p className="mt-1 text-sm leading-6 text-white/60">{item.text}</p>
                    </div>
                  </div>
                ))}
              </div>
            </div>

            <div className="mt-4 grid grid-cols-3 gap-3">
              {['47 processed', '3 escalated', '$847K handled'].map(metric => (
                <div key={metric} className="rounded-2xl border border-white/10 bg-white/[0.03] px-3 py-3 text-center">
                  <p className="text-sm font-medium text-white">{metric}</p>
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2 border-t border-white/10 pt-4 text-xs text-white/50">
          <Zap className="size-3.5 text-amber-300" />
          Plan, check, run, review
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

        <section className="grid flex-1 items-center gap-8 py-12 lg:grid-cols-[0.92fr_1.08fr] lg:py-14">
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
              Just tell us what needs to be done.
              <span className="block text-accent">We will do it.</span>
            </h1>

            <p className="mt-5 max-w-lg text-base leading-7 text-tx-2 sm:text-lg">
              We fundamentally improve how your company operates. Skip the setup costs of traditional software with our secure, cloud-based platform that open-source tools can't match.
              Deploy intelligent agents across any department, instantly.
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
            title="Agents at work. Real roles, real results."
            text="Write these job specs once. Agents execute them 24/7 with logs and approvals built in."
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
            title="Multi-role agents. Any connector. Any system."
            text="One agent can handle multiple tasks. Each task reads from its connectors and sends output to the right place."
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
            eyebrow="Open Standard"
            title="Built on ACP: agents that collaborate, not just compute"
            text="Narayan is built natively on the Agent Communication Protocol (ACP) — an open IBM/BeeAI standard that lets autonomous agents delegate, stream results, and coordinate across trust boundaries without any centralised controller."
          />

          <div className="mt-12 grid items-stretch gap-8 lg:grid-cols-2">
            {/* Left: how Narayan uses ACP */}
            <div className="space-y-4">
              {[
                {
                  badge: 'Delegation',
                  color: 'bg-accent-soft text-accent',
                  title: 'One agent, many specialists',
                  body: 'Your top-level Narayan agent breaks a job into sub-tasks and spawns specialist agents via ACP. The invoice agent calls a PO-matching agent. The support agent calls a knowledge-search agent. Each runs independently and returns a typed result.',
                },
                {
                  badge: 'Streaming',
                  color: 'bg-info-soft text-info',
                  title: 'Real-time progress, not black boxes',
                  body: 'ACP supports server-sent streaming, so Narayan surfaces live progress as agents work — partial answers, intermediate steps, and status changes appear in your audit trail the moment they happen.',
                },
                {
                  badge: 'Orchestration',
                  color: 'bg-vio/15 text-vio',
                  title: 'Multi-agent DAG execution',
                  body: 'Complex workflows fan out across agent networks. Narayan\'s DAG engine schedules ACP calls in parallel, waits for dependencies, and merges results — all expressed in the same job spec you write in plain English.',
                },
                {
                  badge: 'Fault isolation',
                  color: 'bg-accent-soft text-accent',
                  title: 'Agents fail safely, not silently',
                  body: 'Because every ACP call is an isolated request with a typed response, a failing specialist agent never crashes the whole workflow. Narayan catches, logs, and either retries or escalates — with a full trace attached.',
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

            {/* Right: 3-D ACP diagram */}
            <ACPDiagram />
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
                  Hire your first digital employee
                </h2>
                <p className="mt-4 text-base leading-7 text-tx-2">
                  Write a job spec. Check the connections. Launch. Your agent works 24/7, keeps a record, and escalates
                  when needed.
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













