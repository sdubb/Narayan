import { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import { ArrowRight, Bot, CheckCircle2, Database, FileText, Layers3, MessageSquareText, Plug, Scale, Search, ShieldCheck, Sparkles, Workflow, Zap, } from 'lucide-react';
import BenefitsScroller from '../components/BenefitsScroller';

const stats = [
  { value: 'Write', label: 'Describe the job in plain English. The agent learns the playbook.' },
  { value: 'Check', label: 'Test connections, verify the setup, and catch issues before launch.' },
  { value: 'Launch', label: 'The agent runs 24/7 and keeps a clear record of what happened.' },
];

const pillars = [
  {
    icon: Workflow,
    title: 'Scale without adding headcount',
    text: 'Pay only for work done. Agents stay on demand, not on payroll, so teams can scale without hiring delays.',
  },
  {
    icon: ShieldCheck,
    title: 'Clear and traceable',
    text: 'Every decision is recorded, replayable, and tied back to the inputs that produced it.',
  },
  {
    icon: Layers3,
    title: 'Fits many teams',
    text: 'Finance, support, legal, sales, and research all run on the same platform and controls.',
  },
  {
    icon: Plug,
    title: 'Works with your existing systems',
    text: 'Custom APIs, databases, MCP servers, and webhooks. Narayan is an intelligence layer on top of your backend, not a replacement.',
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
              Write a job description.
              <span className="block text-accent">Agent does the work.</span>
            </h1>

            <p className="mt-5 max-w-lg text-base leading-7 text-tx-2 sm:text-lg">
              Deploy agents for finance, support, legal, sales, and research without adding another person to the queue.
              Every action is logged. Every workflow stays auditable.
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













