import { useState, useEffect, useRef, useCallback } from 'react';
import {
  Send, Paperclip, X, Plus, Loader2,
  CheckCircle2, AlertCircle, Clock, Zap, Bot, Network,
  Pause, Play, RotateCcw, ChevronDown, ChevronRight,
  Settings, LogOut, Activity, FileText, Shield, Lock,
  ExternalLink, Plug, BookOpen, Layers, ArrowRight,
  Eye, FileCheck, Bell, AlertTriangle, Database,
  Cpu, GitBranch, Search, Link2,
} from 'lucide-react';
import { agents, conversations as conversationsApi, citations as citationsApi, reviews as reviewsApi, autoApprovals, swarm } from '../api';
import clsx from 'clsx';

// ── Status config ─────────────────────────────────────────────────────────
const STATUS = {
  pending:    { dot:'bg-tx-4',  label:'Pending',    spin:false },
  preflight:  { dot:'bg-info',  label:'Preflight',  spin:false },
  clarifying: { dot:'bg-warn',  label:'Clarifying', spin:false },
  running:    { dot:'bg-ok',    label:'Running',     spin:true  },
  waiting:    { dot:'bg-info',  label:'Scheduled',  spin:false },
  delegating: { dot:'bg-vio',   label:'Delegating', spin:true  },
  paused:     { dot:'bg-warn',  label:'Paused',     spin:false },
  completed:  { dot:'bg-ok',    label:'Done',       spin:false },
  failed:     { dot:'bg-err',   label:'Failed',     spin:false },
};

const TERMINAL = new Set(['completed','failed']);

// ── Helpers ───────────────────────────────────────────────────────────────
function timeAgo(iso) {
  const d = Date.now() - new Date(iso).getTime();
  const h = Math.floor(d/3600000), m = Math.floor((d%3600000)/60000);
  if (h>0) return `${h}h ago`; if (m>0) return `${m}m ago`; return 'just now';
}
function nowTs() {
  return new Date().toLocaleTimeString('en',{hour12:false,hour:'2-digit',minute:'2-digit',second:'2-digit'});
}
function extractText(ev) {
  return ev.summary||ev.description||ev.reason||ev.rationale
    ||ev.output_preview||ev.message||(ev.questions?.join(' / '))||ev.sub_goal||'';
}
function streamAgent(agentId, onEvent, onError) {
  const token = localStorage.getItem('narayan_token');
  const BASE  = import.meta.env.VITE_API_URL||'/api';
  let active  = true; const ctrl = new AbortController();
  (async () => {
    try {
      const res = await fetch(`${BASE}/agents/${agentId}/stream`,{headers:{Authorization:`Bearer ${token}`},signal:ctrl.signal});
      if (!res.ok){onError?.(new Error(`HTTP ${res.status}`));return;}
      const reader=res.body.getReader(),decoder=new TextDecoder(); let buf='';
      while(active){
        const{done,value}=await reader.read(); if(done)break;
        buf+=decoder.decode(value,{stream:true});
        const parts=buf.split('\n\n'); buf=parts.pop()??'';
        for(const part of parts) for(const line of part.split('\n'))
          if(line.startsWith('data: ')){const d=line.slice(6).trim();if(d&&d!=='[DONE]')try{onEvent(JSON.parse(d));}catch{}}
      }
    } catch(e){if(e.name!=='AbortError')onError?.(e);}
  })();
  return{close:()=>{active=false;ctrl.abort();}};
}

// ═══════════════════════════════════════════════════════════
// ── BADGE CHIP ───────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function Badge({label, color='gray', icon:Icon}) {
  const cls = {
    gray:   'bg-bg-active text-tx-3 border border-border',
    amber:  'bg-warn-soft text-warn border border-warn/25',
    green:  'bg-ok-soft text-ok border border-ok/25',
    red:    'bg-err-soft text-err border border-err/25',
    blue:   'bg-info-soft text-info border border-info/25',
    violet: 'bg-vio-soft text-vio border border-vio/25',
    orange: 'bg-accent-soft text-accent border border-accent/25',
  }[color]||'bg-bg-active text-tx-3 border border-border';
  return (
    <span className={clsx('inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded shrink-0 tracking-wide uppercase', cls)}>
      {Icon && <Icon size={9}/>}
      {label}
    </span>
  );
}

// ═══════════════════════════════════════════════════════════
// ── PHASE LABEL ──────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function PhaseLabel({text}) {
  return (
    <div className="flex items-center gap-2 mt-5 mb-2 first:mt-2">
      <span className="text-[10px] font-bold tracking-widest uppercase text-accent/80">{text}</span>
      <div className="flex-1 h-px bg-accent/15"/>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── STEP ROW (generic) ───────────────────────────────────
// ═══════════════════════════════════════════════════════════
function StepRow({badge, badgeColor, badgeIcon, title, detail, code, timestamp, success, collapsible, children}) {
  const [open, setOpen] = useState(true);
  const hasExtra = collapsible && children;
  return (
    <div className="flex items-start gap-2.5 py-2.5 border-b border-border/50 last:border-0 animate-in group">
      <div className="pt-0.5 shrink-0">
        <Badge label={badge} color={badgeColor} icon={badgeIcon}/>
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-start justify-between gap-2">
          <p className="text-[13px] font-medium text-tx-1 leading-snug">{title}</p>
          <div className="flex items-center gap-1.5 shrink-0">
            {timestamp && <span className="font-mono text-[10px] text-tx-4">{timestamp}</span>}
            {success===true  && <CheckCircle2 size={11} className="text-ok"/>}
            {success===false && <AlertCircle  size={11} className="text-err"/>}
            {hasExtra && (
              <button onClick={()=>setOpen(o=>!o)} className="text-tx-4 hover:text-tx-2 transition-colors">
                <ChevronDown size={11} className={clsx('transition-transform', !open&&'-rotate-90')}/>
              </button>
            )}
          </div>
        </div>
        {detail && <p className="text-[12px] text-tx-3 mt-0.5 leading-relaxed">{detail}</p>}
        {code   && <code className="block text-[11px] text-tx-3 font-mono bg-bg-active rounded px-2 py-1 mt-1.5 break-all">{code}</code>}
        {hasExtra && open && <div className="mt-1.5">{children}</div>}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── SEGMENT CARD WRAPPER ─────────────────────────────────
// ═══════════════════════════════════════════════════════════
function SegmentCard({color, icon:Icon, label, children}) {
  const border = {
    amber:  'border-warn/25 bg-warn-soft/40',
    green:  'border-ok/25 bg-ok-soft/40',
    red:    'border-err/25 bg-err-soft/40',
    blue:   'border-info/25 bg-info-soft/40',
    violet: 'border-vio/25 bg-vio-soft/40',
    orange: 'border-accent/25 bg-accent-soft/30',
    gray:   'border-border bg-bg-card',
  }[color]||'border-border bg-bg-card';
  const text = {
    amber:'text-warn', green:'text-ok', red:'text-err',
    blue:'text-info', violet:'text-vio', orange:'text-accent', gray:'text-tx-3',
  }[color]||'text-tx-3';
  return (
    <div className={clsx('rounded-xl border overflow-hidden shadow-sm my-2 animate-in', border)}>
      <div className={clsx('flex items-center gap-2 px-3.5 py-2.5 border-b', border.split(' ')[0])}>
        {Icon && <Icon size={12} className={clsx(text, 'shrink-0')}/>}
        <span className={clsx('text-[11px] font-bold tracking-wider uppercase', text)}>{label}</span>
      </div>
      <div className="px-3.5 py-0.5 divide-y divide-border/40">{children}</div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── PLAN CARD ────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function PlanCard({event}) {
  const [open,setOpen] = useState(true);
  return (
    <SegmentCard color="blue" icon={Layers} label={`Plan ready — ${event.step_count} steps`}>
      <div className="py-2.5">
        {event.rationale && <p className="text-[12px] text-tx-2 leading-relaxed mb-2">{event.rationale}</p>}
        <button onClick={()=>setOpen(o=>!o)} className="flex items-center gap-1.5 text-[11px] text-info/80 hover:text-info transition-colors">
          <ChevronDown size={11} className={clsx('transition-transform', !open&&'-rotate-90')}/>
          {open ? 'Hide steps' : 'Show steps'}
        </button>
        {open && event.steps?.length>0 && (
          <div className="mt-2 space-y-1">
            {event.steps.map((s,i)=>(
              <div key={i} className="flex items-start gap-2 text-[11px]">
                <span className="font-mono text-accent/70 shrink-0 w-5 text-right">{i}</span>
                <span className="text-tx-2 flex-1">{s.description}</span>
                {s.tool && <span className="font-mono text-tx-4 shrink-0">{s.tool}</span>}
              </div>
            ))}
          </div>
        )}
      </div>
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── POLICY CARD ──────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function PolicyCard({event}) {
  const isBlock   = event.decision==='block';
  const isApprove = event.decision==='require_approval';
  const color  = isBlock?'red':isApprove?'amber':'green';
  const icon   = isBlock?Lock:isApprove?Shield:CheckCircle2;
  const label  = isBlock?'Policy blocked':isApprove?'Awaiting approval':'Policy: allow';
  return (
    <SegmentCard color={color} icon={icon} label={label}>
      <StepRow badge="rule" badgeColor={color} title={event.rule_id||event.message||'Policy rule triggered'}
        detail={event.reason||event.message}/>
      {event.tool && <StepRow badge="tool" badgeColor="gray" title={`Tool: ${event.tool}`}
        detail={`Risk level: ${event.risk_level||'medium'}`}/>}
      {isApprove && <StepRow badge="review" badgeColor="amber" icon={Bell}
        title="Submitted to review queue"
        detail="Agent paused. Reviewer must approve via GET /reviews → POST /reviews/:id/resolve"
        code={`ReviewQueue.submit(reason="${event.rule_id}")`}/>}
      {isBlock && <StepRow badge="blocked" badgeColor="red" title="Tool call skipped"
        detail="Agent will adapt approach — tool not executed"/>}
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── PII CARD ─────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function PiiCard({event}) {
  const hadPii = event.fields_redacted?.length>0;
  return (
    <SegmentCard color={hadPii?'amber':'green'} icon={Eye} label={hadPii?'PII redacted':'PII scan — clean'}>
      <StepRow badge="scan" badgeColor="gray" title={`Tool args scanned: ${event.tool||'unknown tool'}`}
        detail={hadPii
          ? `Redacted ${event.fields_redacted.length} field(s): ${event.fields_redacted.join(', ')}`
          : 'No sensitive data found — args passed through unchanged'}/>
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── SLA CARD ─────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function SlaCard({event}) {
  const pct     = event.pct_elapsed||0;
  const isBreached = pct>=100;
  const isWarn    = pct>=80;
  const color  = isBreached?'red':isWarn?'amber':'green';
  const label  = isBreached?'SLA breached':isWarn?'SLA warning':'SLA check';
  return (
    <SegmentCard color={color} icon={Clock} label={label}>
      <StepRow badge={`${pct.toFixed(0)}%`} badgeColor={color}
        title={event.message||`SLA at ${pct.toFixed(0)}% elapsed`}
        detail={event.deadline ? `Deadline: ${new Date(event.deadline).toLocaleTimeString()}` : undefined}/>
      {event.action==='escalate' && (
        <StepRow badge="escalate" badgeColor="red" icon={AlertTriangle}
          title="Escalated to human review"
          detail={event.reason||'SLA threshold breached — review queue notified'}/>
      )}
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CITATION CARD ────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function CitationCard({event}) {
  return (
    <SegmentCard color="violet" icon={Link2} label={`Citation recorded — step ${event.step_index??'?'}`}>
      <StepRow badge="claim" badgeColor="gray" title={event.summary||event.claim||'Step finding cited'}
        detail={event.source_ref ? `Source: ${event.source_ref} (${event.source_type||'tool_output'})` : undefined}
        code={event.confidence!=null ? `confidence: ${event.confidence}` : undefined}/>
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── EVIDENCE CARD ────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function EvidenceCard({event}) {
  return (
    <SegmentCard color="violet" icon={FileCheck} label="Evidence packaged">
      <StepRow badge="citations" badgeColor="violet" title={`${event.citations||0} citations bundled`}
        detail={`${event.audit_entries||0} audit log entries included`}/>
      <StepRow badge="stored" badgeColor="green" title="Package stored for compliance review"
        detail="EvidencePackager.package() completed — available via GET /agents/:id/evidence"/>
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CONNECTOR TRIGGER CARD ───────────────────────────────
// ═══════════════════════════════════════════════════════════
function ConnectorTriggerCard({event}) {
  const connectorColors = {
    github:'green', zendesk:'green', salesforce:'blue',
    quickbooks:'green', docusign:'blue', pagerduty:'red',
    hubspot:'orange', notion:'violet', greenhouse:'blue', dbt_cloud:'amber',
  };
  const color = connectorColors[event.connector_type]||'gray';
  return (
    <SegmentCard color={color} icon={Plug} label={`${event.connector_type} → agent triggered`}>
      <StepRow badge="event" badgeColor={color} title={event.event_type||'webhook received'}
        detail={`Goal created from inbound ${event.connector_type} webhook`}/>
      {event.external_id && (
        <StepRow badge="id" badgeColor="gray" title={`External ID: ${event.external_id}`}
          detail="Output will be delivered back to this record via deliver_output()"/>
      )}
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── REVIEW QUEUE CARD ────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function ReviewQueueCard({event, agentId}) {
  const [resolving,  setResolving]  = useState(false);
  const [resolved,   setResolved]   = useState(false);
  const [resolution, setResolution] = useState(null); // 'approved'|'auto_approved'|'changes_requested'|'rejected'
  const [showNote,   setShowNote]   = useState(false);
  const [note,       setNote]       = useState('');
  const [err,        setErr]        = useState('');

  async function resolve(status, noteOverride) {
    setResolving(true); setErr('');
    try {
      const BASE  = import.meta.env.VITE_API_URL||'/api';
      const token = localStorage.getItem('narayan_token');
      const finalNote = noteOverride ?? (note.trim() || `Resolved (${status}) from UI`);
      await fetch(`${BASE}/reviews/${event.review_id}/resolve`, {
        method:'POST',
        headers:{'Authorization':`Bearer ${token}`,'Content-Type':'application/json'},
        body: JSON.stringify({status, notes: finalNote}),
      });
      setResolution(status);
      setResolved(true);
    } catch(e){setErr(e.message);}
    finally{setResolving(false);}
  }

  const RESOLUTION_LABELS = {
    approved:           { label:'Approved',            color:'green' },
    auto_approved:      { label:'Auto-approved',       color:'green' },
    changes_requested:  { label:'Changes requested',   color:'amber' },
    rejected:           { label:'Rejected',            color:'red'   },
  };

  if (resolved) {
    const r = RESOLUTION_LABELS[resolution]||{label:'Resolved',color:'green'};
    return (
      <SegmentCard color={r.color} icon={CheckCircle2} label={`Review ${r.label} — agent resuming`}>
        <StepRow badge={r.label.toLowerCase()} badgeColor={r.color}
          title="Review completed"
          detail={note.trim()||`Agent will retry on next scheduler tick`}/>
      </SegmentCard>
    );
  }

  return (
    <SegmentCard color="amber" icon={Bell} label="Human review required">
      <StepRow badge="pending" badgeColor="amber"
        title={event.summary||'Review item created'}
        detail={event.reason ? `Rule: ${event.reason}` : undefined}/>
      {event.message && <StepRow badge="context" badgeColor="gray" title={event.message}/>}

      {/* Optional note field */}
      <div className="py-2.5">
        <button onClick={()=>setShowNote(o=>!o)}
          className="flex items-center gap-1.5 text-[11px] text-tx-3 hover:text-tx-2 transition-colors mb-2">
          <ChevronDown size={10} className={clsx('transition-transform', !showNote&&'-rotate-90')}/>
          {showNote ? 'Hide note' : 'Add note for agent'}
        </button>
        {showNote && (
          <textarea value={note} onChange={e=>setNote(e.target.value)}
            placeholder="Optional — instructions or context for the agent to use when retrying…"
            rows={2}
            className="w-full rounded-lg border border-border bg-bg px-3 py-2 text-[12px] text-tx-1 placeholder-tx-4 outline-none focus:border-border-md resize-none transition-all"/>
        )}
      </div>

      {err && <p className="text-[11px] text-err pb-2">{err}</p>}

      {/* Action grid — 4 options in 2 rows */}
      <div className="grid grid-cols-2 gap-2 pb-3">

        {/* AUTO-APPROVE — persists rule so this never blocks again */}
        <button onClick={async()=>{
          // Persist auto-approval rule so future occurrences skip review
          autoApprovals.create(
            event.rule_id||event.reason||'unknown',
            `Auto-approved from chat — ${new Date().toISOString()}`
          ).catch(()=>{}); // best-effort, don't block UI
          resolve('auto_approved','Auto-approved: rule saved, will not block again');
        }}
          disabled={resolving}
          className="flex flex-col items-start gap-0.5 rounded-xl border border-ok/30 bg-ok-soft px-3 py-2.5 hover:border-ok/50 hover:bg-ok/10 transition-all disabled:opacity-50 text-left">
          <div className="flex items-center gap-1.5">
            <Zap size={11} className="text-ok"/>
            <span className="text-[12px] font-semibold text-ok">Auto-approve</span>
          </div>
          <span className="text-[10px] text-ok/70 leading-tight">Approve & don't ask again for this rule</span>
        </button>

        {/* APPROVE — standard approval */}
        <button onClick={()=>resolve('approved')} disabled={resolving}
          className="flex flex-col items-start gap-0.5 rounded-xl border border-ok/30 bg-ok-soft px-3 py-2.5 hover:border-ok/50 hover:bg-ok/10 transition-all disabled:opacity-50 text-left">
          <div className="flex items-center gap-1.5">
            {resolving
              ? <Loader2 size={11} className="text-ok animate-spin"/>
              : <CheckCircle2 size={11} className="text-ok"/>}
            <span className="text-[12px] font-semibold text-ok">Approve</span>
          </div>
          <span className="text-[10px] text-ok/70 leading-tight">Proceed this time, ask again next</span>
        </button>

        {/* REQUEST CHANGES — agent retries with note */}
        <button onClick={()=>{ if(!note.trim()){setShowNote(true);return;} resolve('changes_requested'); }}
          disabled={resolving}
          className="flex flex-col items-start gap-0.5 rounded-xl border border-warn/30 bg-warn-soft px-3 py-2.5 hover:border-warn/50 hover:bg-warn/10 transition-all disabled:opacity-50 text-left">
          <div className="flex items-center gap-1.5">
            <RotateCcw size={11} className="text-warn"/>
            <span className="text-[12px] font-semibold text-warn">Request changes</span>
          </div>
          <span className="text-[10px] text-warn/70 leading-tight">Retry with your note as context</span>
        </button>

        {/* REJECT — hard stop */}
        <button onClick={()=>resolve('rejected')} disabled={resolving}
          className="flex flex-col items-start gap-0.5 rounded-xl border border-err/30 bg-err-soft px-3 py-2.5 hover:border-err/50 hover:bg-err/10 transition-all disabled:opacity-50 text-left">
          <div className="flex items-center gap-1.5">
            <AlertCircle size={11} className="text-err"/>
            <span className="text-[12px] font-semibold text-err">Reject</span>
          </div>
          <span className="text-[10px] text-err/70 leading-tight">Block this action, agent fails step</span>
        </button>

      </div>
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CONNECTOR SUGGEST CARD (existing) ────────────────────
// ═══════════════════════════════════════════════════════════
function ConnectorCard({group, onNavigateSettings}) {
  const [open,setOpen] = useState(true);
  return (
    <div className="my-2 rounded-xl border border-border bg-bg-card shadow-sm overflow-hidden animate-in">
      <button onClick={()=>setOpen(o=>!o)}
        className="w-full flex items-center gap-2 px-3.5 py-2.5 hover:bg-bg-hover transition-colors">
        <Plug size={12} className="text-tx-3 shrink-0"/>
        <span className="text-[12px] font-medium text-tx-1 flex-1">{group.label||'Suggested connectors'}</span>
        <ChevronDown size={12} className={clsx('text-tx-4 transition-transform',!open&&'-rotate-90')}/>
      </button>
      {open&&(
        <div className="border-t border-border">
          {group.items?.map(item=>(
            <div key={item.name} className="flex items-center gap-3 px-3.5 py-3 border-b border-border/60 last:border-0">
              {item.icon_url
                ? <img src={item.icon_url} alt={item.name} className="size-7 rounded-md object-cover shrink-0"/>
                : <div className="size-7 rounded-md bg-bg-active flex items-center justify-center shrink-0"><Plug size={12} className="text-tx-3"/></div>}
              <div className="flex-1 min-w-0">
                <p className="text-[13px] font-medium text-tx-1">{item.name}</p>
                {item.description&&<p className="text-[11px] text-tx-3 truncate">{item.description}</p>}
              </div>
              <button onClick={onNavigateSettings} className="flex items-center gap-1 text-[11px] font-medium text-accent hover:text-accent-text transition-colors shrink-0">
                Add in Settings <ArrowRight size={10}/>
              </button>
            </div>
          ))}
          <div className="px-3.5 py-2.5 bg-bg">
            <p className="text-[11px] text-tx-3">
              Add credentials in{' '}
              <button onClick={onNavigateSettings} className="text-accent hover:underline font-medium">Settings → Credentials</button>
              {' '}to enable.
            </p>
          </div>
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CLARIFY CARD (existing, restyled) ────────────────────
// ═══════════════════════════════════════════════════════════
function ClarifyCard({agentId, questions, onDone}) {
  const [answers,setAnswers]     = useState(questions.map(()=>''));
  const [loading,setLoading]     = useState(false);
  const [submitted,setSubmitted] = useState(false);
  const [err,setErr]             = useState('');

  async function submit() {
    setLoading(true); setErr('');
    try { await agents.clarify(agentId,answers); setSubmitted(true); setTimeout(onDone,1600); }
    catch(e){setErr(e.message);}
    finally{setLoading(false);}
  }

  if(submitted) return (
    <SegmentCard color="green" icon={CheckCircle2} label="Answers received — agent resuming">
      <StepRow badge="ok" badgeColor="green" title="Answers submitted"/>
    </SegmentCard>
  );

  return (
    <SegmentCard color="amber" icon={Bot} label="Needs clarification">
      <div className="py-3 space-y-3">
        {questions.map((q,i)=>(
          <div key={i}>
            <p className="text-[13px] text-tx-1 mb-1.5">{q}</p>
            <input value={answers[i]}
              onChange={e=>{const n=[...answers];n[i]=e.target.value;setAnswers(n);}}
              onKeyDown={e=>{if(e.key==='Enter')submit();}}
              placeholder="Your answer…"
              className="w-full rounded-lg border border-border bg-bg-card px-3 py-2 text-[13px] text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all"/>
          </div>
        ))}
        {err&&<p className="text-[11px] text-err">{err}</p>}
        <button onClick={submit} disabled={loading||answers.every(a=>!a.trim())}
          className="flex items-center gap-2 rounded-lg bg-tx-1 px-4 py-2 text-[12px] font-medium text-bg-card hover:bg-tx-2 disabled:opacity-50 transition-all">
          {loading?<Loader2 size={11} className="animate-spin"/>:<Send size={11}/>}
          Submit answers
        </button>
      </div>
    </SegmentCard>
  );
}

// ═══════════════════════════════════════════════════════════
// ── THINKING DOTS ─────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function ThinkingDots() {
  return (
    <div className="flex items-center gap-1.5 px-2 py-3">
      {[0,1,2].map(i=>(
        <div key={i} className="size-1.5 rounded-full bg-tx-4 animate-pulse-dot"
          style={{animationDelay:`${i*0.22}s`}}/>
      ))}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── RESULT VIEW (completion) ─────────────────────────────
// ═══════════════════════════════════════════════════════════
function AgentResultView({agentId, terminalEvent}) {
  const [detail,setDetail]               = useState(null);
  const [replay,setReplay]               = useState(null);
  const [showReplay,setShowReplay]       = useState(false);
  const [loadingReplay,setLoadingReplay] = useState(false);
  const [cites,setCites]                 = useState([]);
  const [showCites,setShowCites]         = useState(false);

  useEffect(()=>{
    agents.get(agentId).then(setDetail).catch(()=>{});
    citationsApi.listForAgent(agentId).then(r=>setCites(r.citations||[])).catch(()=>{});
  },[agentId]);

  async function loadReplay() {
    setLoadingReplay(true);
    try{const r=await agents.replay(agentId);setReplay(r);setShowReplay(true);}
    catch{}finally{setLoadingReplay(false);}
  }

  const isSuccess   = terminalEvent?.type==='goal_complete';
  const summary     = terminalEvent?.summary||detail?.final_answer||detail?.metadata?.final_answer||detail?.metadata?.last_reflection||'';
  const keyFindings = detail?.metadata?.key_findings||[];

  return (
    <div className="p-6 max-w-2xl space-y-4 animate-in">

      {/* Banner */}
      <div className={clsx('flex items-start gap-3 rounded-xl border p-4',
        isSuccess ? 'border-ok/25 bg-ok-soft' : 'border-err/25 bg-err-soft')}>
        {isSuccess
          ? <CheckCircle2 size={17} className="text-ok mt-0.5 shrink-0"/>
          : <AlertCircle  size={17} className="text-err mt-0.5 shrink-0"/>}
        <div>
          <p className={clsx('font-medium text-[14px]',isSuccess?'text-ok':'text-err')}>
            {isSuccess?'Goal completed':'Goal failed'}
          </p>
          {summary&&<p className="text-[13px] text-tx-2 mt-1 leading-relaxed">{summary}</p>}
        </div>
      </div>

      {/* Key findings */}
      {keyFindings.length>0&&(
        <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
          <div className="px-4 py-3 border-b border-border flex items-center gap-2">
            <BookOpen size={13} className="text-tx-3"/>
            <span className="text-[13px] font-medium text-tx-1">Key findings</span>
            <span className="ml-auto text-[11px] text-tx-4 font-mono">{keyFindings.length}</span>
          </div>
          <div className="divide-y divide-border/60">
            {keyFindings.map((f,i)=>(
              <div key={i} className="flex items-start gap-3 px-4 py-3">
                <span className="font-mono text-[11px] text-accent mt-0.5 w-5 shrink-0">{i+1}.</span>
                <p className="text-[13px] text-tx-2 leading-relaxed">{f}</p>
              </div>
            ))}
          </div>
        </div>
      )}

      {detail?.workspace_path&&(
        <div className="flex items-center gap-2 rounded-lg border border-border bg-bg px-3.5 py-2.5">
          <FileText size={13} className="text-tx-3 shrink-0"/>
          <span className="font-mono text-[11px] text-tx-3 truncate flex-1">{detail.workspace_path}</span>
          <ExternalLink size={12} className="text-tx-4 shrink-0"/>
        </div>
      )}

      {/* Citations audit trail */}
      {cites.length>0 && (
        <div className="rounded-xl border border-vio/20 overflow-hidden shadow-sm">
          <button onClick={()=>setShowCites(o=>!o)}
            className="w-full flex items-center gap-2 px-4 py-3 bg-vio-soft/40 hover:bg-vio-soft/60 transition-colors border-b border-vio/15">
            <Link2 size={13} className="text-vio shrink-0"/>
            <span className="text-[13px] font-medium text-vio flex-1">Citations — {cites.length} sources recorded</span>
            <ChevronDown size={12} className={clsx('text-vio/60 transition-transform', !showCites&&'-rotate-90')}/>
          </button>
          {showCites && (
            <div className="divide-y divide-border/60 max-h-64 overflow-y-auto">
              {cites.map((c,i)=>(
                <div key={i} className="flex items-start gap-3 px-4 py-3 hover:bg-bg-hover">
                  <span className="font-mono text-[10px] text-vio/60 shrink-0 mt-0.5 w-5 text-right">{i+1}</span>
                  <div className="flex-1 min-w-0">
                    <p className="text-[12px] text-tx-1 leading-snug">{c.claim||c.summary}</p>
                    <div className="flex items-center gap-2 mt-0.5">
                      <span className="font-mono text-[10px] text-tx-4">{c.source_ref||c.source_type}</span>
                      {c.confidence!=null && (
                        <span className={clsx('text-[10px] font-medium', c.confidence>=0.9?'text-ok':c.confidence>=0.6?'text-warn':'text-err')}>
                          {Math.round(c.confidence*100)}%
                        </span>
                      )}
                      {c.step_index!=null && <span className="text-[10px] text-tx-4">step {c.step_index}</span>}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      <button onClick={showReplay?()=>setShowReplay(false):loadReplay} disabled={loadingReplay}
        className="flex items-center gap-1.5 text-[13px] text-tx-3 hover:text-tx-2 transition-colors disabled:opacity-50">
        {loadingReplay?<Loader2 size={13} className="animate-spin"/>:<RotateCcw size={13}/>}
        {showReplay?'Hide execution log':'Show execution log'}
      </button>

      {showReplay&&replay?.steps&&(
        <div className="rounded-xl border border-border overflow-hidden shadow-sm animate-in">
          <div className="max-h-72 overflow-y-auto divide-y divide-border/60">
            {replay.steps.map((step,i)=>(
              <div key={i} className="flex items-start gap-3 px-4 py-2.5 hover:bg-bg-hover">
                <span className="font-mono text-[11px] text-accent shrink-0 mt-0.5">{String(step.step_index).padStart(2,'0')}</span>
                <div className="flex-1 min-w-0">
                  <p className="text-[13px] text-tx-2">{step.action}</p>
                  {step.result&&<p className="text-[11px] text-tx-3 mt-0.5 font-mono truncate">
                    {typeof step.result==='string'?step.result.slice(0,140):JSON.stringify(step.result).slice(0,140)}
                  </p>}
                </div>
                {step.timestamp&&<span className="font-mono text-[11px] text-tx-4 shrink-0">{step.timestamp.slice(11,19)}</span>}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── EVENT RENDERER ───────────────────────────────────────
// Maps raw SSE event → the right rich card or step row
// ═══════════════════════════════════════════════════════════
function renderEvent(ev, idx, agentId, onNavigateSettings) {
  const ts = ev.ts;

  // ── Rich segment service events ─────────────────────────
  if (ev.type==='policy_decision')         return <PolicyCard     key={idx} event={ev}/>;
  if (ev.type==='pii_redacted')            return <PiiCard        key={idx} event={ev}/>;
  if (ev.type==='sla_check')               return <SlaCard        key={idx} event={ev}/>;
  if (ev.type==='citation_recorded')       return <CitationCard   key={idx} event={ev}/>;
  if (ev.type==='evidence_packaged')       return <EvidenceCard   key={idx} event={ev}/>;
  if (ev.type==='review_required')         return <ReviewQueueCard key={idx} event={ev} agentId={agentId}/>;
  if (ev.type==='connector_trigger')       return <ConnectorTriggerCard key={idx} event={ev}/>;
  if (ev.type==='plan_created')            return <PlanCard       key={idx} event={ev}/>;
  if (ev.type==='suggest_connectors')      return null; // handled separately via connectorGroups

  // ── Preflight & clarification ────────────────────────────
  if (ev.type==='preflight_started') return (
    <StepRow key={idx} badge="preflight" badgeColor="gray" icon={Zap} timestamp={ts}
      title="Running preflight check…" detail="Verifying goal is achievable with available tools"/>
  );
  if (ev.type==='preflight_passed') return (
    <StepRow key={idx} badge="preflight" badgeColor="green" icon={CheckCircle2} timestamp={ts}
      title="Preflight passed — goal is feasible"
      detail={ev.message||'All required tools available. Started wall-clock timeout.'} success={true}/>
  );
  if (ev.type==='preflight_failed') return (
    <StepRow key={idx} badge="preflight" badgeColor="red" icon={AlertCircle} timestamp={ts}
      title="Preflight failed — goal not achievable" detail={ev.reason} success={false}/>
  );
  if (ev.type==='clarification_needed') return null; // handled by ClarifyCard below
  if (ev.type==='clarification_received') return (
    <StepRow key={idx} badge="clarified" badgeColor="green" icon={CheckCircle2} timestamp={ts}
      title="Answers received — resuming planning" success={true}/>
  );

  // ── Planning ─────────────────────────────────────────────
  if (ev.type==='planning_started') return (
    <StepRow key={idx} badge="planning" badgeColor="blue" icon={Cpu} timestamp={ts}
      title="Creating execution plan…" detail="Selecting job type and step sequence"/>
  );

  // ── Step execution ────────────────────────────────────────
  if (ev.type==='step_started') return (
    <StepRow key={idx} badge={`step ${ev.step_index??''}`} badgeColor="gray" icon={Zap} timestamp={ts}
      title={ev.description||`Step ${ev.step_index} started`}
      detail={ev.tool ? `Tool: ${ev.tool}` : undefined}/>
  );
  if (ev.type==='tool_called') return (
    <StepRow key={idx} badge="calling" badgeColor="amber" timestamp={ts}
      title={`Calling ${ev.tool_name||'tool'}…`}
      detail={ev.args_preview ? `Args: ${ev.args_preview}` : undefined}/>
  );
  if (ev.type==='tool_result') return (
    <StepRow key={idx} badge="tool" badgeColor={ev.success?'green':'amber'} timestamp={ts}
      title={ev.tool_name ? `${ev.tool_name} → ${ev.success?'success':'failed'}` : 'Tool executed'}
      detail={ev.output_preview} success={ev.success}/>
  );
  if (ev.type==='step_completed') return (
    <StepRow key={idx} badge="done" badgeColor="green" icon={CheckCircle2} timestamp={ts}
      title={ev.summary||`Step ${ev.step_index??''} completed`} success={true}/>
  );
  if (ev.type==='step_retrying') return (
    <StepRow key={idx} badge="retry" badgeColor="amber" icon={RotateCcw} timestamp={ts}
      title={`Retrying in ${ev.delay_secs||10}s`} detail={ev.reason}/>
  );

  // ── Lag warning ───────────────────────────────────────────
  if (ev.type==='lag') return (
    <StepRow key={idx} badge="lag" badgeColor="amber" icon={AlertTriangle} timestamp={ts}
      title={`Stream lag — ${ev.missed??0} event(s) missed`}
      detail="The event buffer was full. Some events were dropped. Reload to see the full log."/>
  );

  // ── Delegation ────────────────────────────────────────────
  if (ev.type==='child_spawned') return (
    <StepRow key={idx} badge="delegate" badgeColor="violet" icon={GitBranch} timestamp={ts}
      title={`Sub-agent spawned: ${ev.child_agent_id?.slice(0,12)}…`}
      detail={ev.sub_goal} code={`parent → ${ev.child_agent_id}`}/>
  );
  if (ev.type==='children_complete') return (
    <StepRow key={idx} badge="merged" badgeColor="violet" icon={GitBranch} timestamp={ts}
      title="All sub-agents completed — parent resuming"
      detail={`${ev.child_ids?.length||0} child agent(s) finished`} success={true}/>
  );

  // ── Terminal events ───────────────────────────────────────
  if (ev.type==='goal_complete') return (
    <div key={idx} className="flex items-start gap-2.5 py-3 mt-2 rounded-xl border border-ok/25 bg-ok-soft px-3.5 animate-in">
      <CheckCircle2 size={15} className="text-ok mt-0.5 shrink-0"/>
      <div>
        <p className="text-[13px] font-semibold text-ok">Goal completed</p>
        {ev.summary && <p className="text-[12px] text-tx-2 mt-0.5 leading-relaxed">{ev.summary}</p>}
      </div>
      <span className="ml-auto font-mono text-[10px] text-tx-4 mt-0.5 shrink-0">{ts}</span>
    </div>
  );
  if (ev.type==='goal_failed') return (
    <div key={idx} className="flex items-start gap-2.5 py-3 mt-2 rounded-xl border border-err/25 bg-err-soft px-3.5 animate-in">
      <AlertCircle size={15} className="text-err mt-0.5 shrink-0"/>
      <div>
        <p className="text-[13px] font-semibold text-err">Goal failed</p>
        {ev.reason && <p className="text-[12px] text-tx-2 mt-0.5 leading-relaxed">{ev.reason}</p>}
      </div>
      <span className="ml-auto font-mono text-[10px] text-tx-4 mt-0.5 shrink-0">{ts}</span>
    </div>
  );

  // ── Fallback generic row ──────────────────────────────────
  const text = extractText(ev);
  if (!text) return null;
  return (
    <div key={idx} className="flex items-start gap-2.5 py-2 text-[12px] group animate-in">
      <span className="font-mono text-[10px] text-tx-4 shrink-0 w-14 pt-0.5">{ts}</span>
      <span className="font-mono text-[10px] text-tx-4 shrink-0 w-28 pt-0.5 truncate">{ev.type?.replace(/_/g,' ')}</span>
      <span className="text-tx-3 leading-relaxed flex-1 min-w-0">{text}</span>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── PHASE GROUPING LOGIC ─────────────────────────────────
// ═══════════════════════════════════════════════════════════
function groupByPhase(events) {
  const groups = [];
  let currentPhase = null;
  let currentStep  = null;

  for (const ev of events) {
    // Determine if this event starts a new phase label
    let phaseLabel = null;
    if (ev.type==='preflight_started'||ev.type==='preflight_passed'||ev.type==='preflight_failed'||ev.type==='clarification_needed'||ev.type==='clarification_received') {
      phaseLabel = 'Preflight';
    } else if (ev.type==='planning_started'||ev.type==='plan_created') {
      phaseLabel = 'Planning';
    } else if (ev.type==='connector_trigger') {
      phaseLabel = `${ev.connector_type||'Connector'} trigger`;
    } else if (ev.type==='step_started') {
      const desc = ev.description ? ` — ${ev.description.slice(0,40)}${ev.description.length>40?'…':''}` : '';
      phaseLabel = `Step ${ev.step_index??''}${desc}`;
      currentStep = ev.step_index;
    } else if ((ev.type==='goal_complete'||ev.type==='goal_failed'||ev.type==='evidence_packaged') && currentPhase!=='Completion') {
      phaseLabel = 'Completion';
    }

    if (phaseLabel && phaseLabel!==currentPhase) {
      currentPhase = phaseLabel;
      groups.push({label:phaseLabel, events:[]});
    }
    if (groups.length===0) groups.push({label:null, events:[]});
    groups[groups.length-1].events.push(ev);
  }
  return groups;
}

// ═══════════════════════════════════════════════════════════
// ── EVENT FEED ───────────────────────────────────────────
// ═══════════════════════════════════════════════════════════
function EventFeed({agentId, initialStatus, onStatusChange, onTerminal, onNavigateSettings}) {
  const [events,setEvents]                   = useState([]);
  const [questions,setQuestions]             = useState([]);
  const [connectorGroups,setConnectorGroups] = useState([]);
  const [liveStatus,setLiveStatus]           = useState(initialStatus);
  const [isThinking,setIsThinking]           = useState(false);
  const bottomRef  = useRef(null);
  const streamRef  = useRef(null);
  const thinkTimer = useRef(null);

  function bumpThinking() {
    setIsThinking(true); clearTimeout(thinkTimer.current);
    thinkTimer.current = setTimeout(()=>setIsThinking(false),4000);
  }

  useEffect(()=>{
    if(!agentId) return;
    setEvents([]); setQuestions([]); setConnectorGroups([]);
    setIsThinking(false); setLiveStatus(initialStatus);
    if(TERMINAL.has(initialStatus)) return;

    streamRef.current?.close();
    streamRef.current = streamAgent(agentId,(ev)=>{
      setEvents(p=>[...p,{...ev,ts:nowTs()}]);
      bumpThinking();

      if(ev.type==='clarification_needed'&&ev.questions) setQuestions(ev.questions);

      if(ev.type==='tool_result'&&ev.tool_name==='suggest_connectors') {
        try{const p=JSON.parse(ev.raw_output||'{}');if(p.groups)setConnectorGroups(p.groups);}catch{}
      }

      const next={
        goal_complete:'completed', goal_failed:'failed',
        step_started:'running', clarification_needed:'clarifying',
        clarification_received:'running',
        child_spawned:'delegating', planning_started:'running',
      }[ev.type];
      if(next){
        setLiveStatus(next); onStatusChange?.(next);
        if(next==='completed'||next==='failed'){
          setIsThinking(false); clearTimeout(thinkTimer.current); onTerminal?.(ev);
        }
      }
    },(err)=>{
      setIsThinking(false);
      setEvents(p=>[...p,{type:'stream_error',ts:nowTs(),summary:err.message}]);
    });

    return()=>{ streamRef.current?.close(); clearTimeout(thinkTimer.current); };
  },[agentId]);

  useEffect(()=>{ bottomRef.current?.scrollIntoView({behavior:'smooth'}); },[events,isThinking]);

  if(events.length===0&&!questions.length) return (
    <div className="flex flex-col items-center justify-center py-16 text-center">
      <div className="size-10 rounded-xl bg-bg-active flex items-center justify-center mb-3">
        <Activity size={17} className="text-tx-4"/>
      </div>
      <p className="text-[13px] text-tx-3">Waiting for agent to start…</p>
    </div>
  );

  const groups = groupByPhase(events);

  return (
    <div className="py-2">
      {groups.map((group, gi)=>(
        <div key={gi}>
          {group.label && <PhaseLabel text={group.label}/>}
          <div className="divide-y divide-border/30">
            {group.events.map((ev, ei)=>
              renderEvent(ev, `${gi}-${ei}`, agentId, onNavigateSettings)
            ).filter(Boolean)}
          </div>
        </div>
      ))}

      {questions.length>0&&liveStatus==='clarifying'&&(
        <ClarifyCard agentId={agentId} questions={questions}
          onDone={()=>{setQuestions([]);onStatusChange?.('waiting');}}/>
      )}

      {connectorGroups.map((group,i)=>(
        <ConnectorCard key={i} group={group} onNavigateSettings={onNavigateSettings}/>
      ))}

      {isThinking&&!TERMINAL.has(liveStatus)&&<ThinkingDots/>}
      <div ref={bottomRef}/>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CONVERSATION ROW (sidebar) ───────────────────────────
// ═══════════════════════════════════════════════════════════
function ConversationRow({conv, selected, onClick, latestStatus}) {
  const cfg = STATUS[latestStatus]||STATUS.pending;
  const title = conv.title || 'New conversation';
  return (
    <button onClick={onClick}
      className={clsx('w-full text-left px-3 py-2.5 rounded-lg transition-all',
        selected?'bg-bg-active':'hover:bg-bg-hover')}>
      <div className="flex items-center gap-2 mb-0.5">
        <div className={clsx('size-1.5 rounded-full shrink-0',cfg.dot,cfg.spin&&'animate-pulse')}/>
        <p className="text-[13px] text-tx-1 truncate flex-1">{title}</p>
      </div>
      <div className="flex items-center gap-2 text-[11px] text-tx-4 pl-3.5">
        <span>{conv.agent_count||0} message{(conv.agent_count||0)!==1?'s':''}</span>
        <span>·</span>
        <span>{timeAgo(conv.updated_at)}</span>
      </div>
    </button>
  );
}

function ImageChip({file, onRemove}) {
  const [url] = useState(()=>URL.createObjectURL(file));
  return (
    <div className="relative group size-12 rounded-lg overflow-hidden border border-border shrink-0">
      <img src={url} alt={file.name} className="size-full object-cover"/>
      <button onClick={onRemove}
        className="absolute inset-0 bg-tx-1/60 opacity-0 group-hover:opacity-100 flex items-center justify-center transition-opacity">
        <X size={13} className="text-white"/>
      </button>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CONVERSATION THREAD VIEW ────────────────────────────
// Shows all agents in a conversation as message pairs
// ═══════════════════════════════════════════════════════════
function ConversationThread({convId, agentStatuses, terminalEvents, onStatusChange, onTerminal, onNavigateSettings}) {
  const [convAgents, setConvAgents] = useState([]);
  const bottomRef = useRef(null);

  useEffect(()=>{
    if(!convId) return;
    let cancelled = false;
    const refresh = () => {
      conversationsApi.get(convId).then(data=>{
        if(!cancelled) setConvAgents(data.agents||[]);
      }).catch(()=>{});
    };
    refresh();
    const iv = setInterval(refresh, 3000);
    return ()=>{ cancelled=true; clearInterval(iv); };
  },[convId]);

  useEffect(()=>{ bottomRef.current?.scrollIntoView({behavior:'smooth'}); },[convAgents]);

  if(!convAgents.length) return (
    <div className="flex flex-col items-center justify-center h-full text-center px-8">
      <p className="font-serif text-2xl text-tx-1 mb-2">What should your agent do?</p>
      <p className="text-[13px] text-tx-3 max-w-xs leading-relaxed">
        Send a message to start the conversation.
      </p>
    </div>
  );

  return (
    <div className="px-6 py-4 space-y-6">
      {convAgents.map((agent, idx)=>{
        const status = agentStatuses[agent.id]||agent.status;
        const isTerminal = TERMINAL.has(status);
        const terminalEvent = terminalEvents[agent.id];
        const isLast = idx===convAgents.length-1;
        return (
          <div key={agent.id} className="animate-in">
            {/* User message bubble */}
            <div className="flex justify-end mb-3">
              <div className="max-w-lg rounded-2xl rounded-br-md bg-tx-1 text-bg-card px-4 py-3">
                <p className="text-[13px] leading-relaxed whitespace-pre-wrap">{agent.goal}</p>
                <p className="text-[10px] opacity-60 mt-1">{timeAgo(agent.created_at)}</p>
              </div>
            </div>

            {/* Agent response */}
            <div className="flex justify-start">
              <div className="max-w-2xl w-full">
                <div className="flex items-center gap-2 mb-1.5">
                  <Bot size={14} className="text-accent shrink-0"/>
                  <span className="text-[11px] font-semibold text-tx-3 uppercase tracking-wide">Narayan</span>
                  {(()=>{
                    const cfg = STATUS[status]||STATUS.pending;
                    return (
                      <span className="inline-flex items-center gap-1 text-[10px] text-tx-4">
                        <span className={clsx('size-1.5 rounded-full',cfg.dot,cfg.spin&&'animate-pulse')}/>
                        {cfg.label}
                      </span>
                    );
                  })()}
                </div>

                {isTerminal ? (
                  <div className="rounded-2xl rounded-bl-md border border-border bg-bg-card p-4">
                    {agent.final_answer ? (
                      <p className="text-[13px] text-tx-1 leading-relaxed whitespace-pre-wrap">{agent.final_answer}</p>
                    ) : terminalEvent?.summary ? (
                      <p className="text-[13px] text-tx-1 leading-relaxed whitespace-pre-wrap">{terminalEvent.summary}</p>
                    ) : status==='failed' ? (
                      <p className="text-[13px] text-err">{terminalEvent?.reason || 'Agent failed'}</p>
                    ) : (
                      <p className="text-[13px] text-tx-3">Completed</p>
                    )}
                  </div>
                ) : isLast ? (
                  <div className="rounded-2xl rounded-bl-md border border-border bg-bg-card overflow-hidden">
                    <div className="px-4 py-2">
                      <EventFeed agentId={agent.id}
                        initialStatus={status}
                        onStatusChange={s=>onStatusChange(agent.id,s)}
                        onTerminal={ev=>onTerminal(agent.id,ev)}
                        onNavigateSettings={onNavigateSettings}/>
                    </div>
                  </div>
                ) : (
                  <div className="rounded-2xl rounded-bl-md border border-border bg-bg-card p-4">
                    <p className="text-[13px] text-tx-3">{agent.final_answer || 'Processing...'}</p>
                  </div>
                )}
              </div>
            </div>
          </div>
        );
      })}
      <div ref={bottomRef}/>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── MAIN CHAT PAGE ───────────────────────────────────────
// ═══════════════════════════════════════════════════════════
export default function ChatPage({onNavigate}) {
  const [convList,setConvList]             = useState([]);
  const [selectedConvId,setSelectedConvId] = useState(null);
  const [input,setInput]                   = useState('');
  const [images,setImages]                 = useState([]);
  const [sending,setSending]               = useState(false);
  const [loading,setLoading]               = useState(true);
  const [error,setError]                   = useState('');
  const [agentStatuses,setAgentStatuses]   = useState({});
  const [terminalEvents,setTerminalEvents] = useState({});
  const [pendingReviews,setPendingReviews] = useState([]);
  const [swarmDepth,setSwarmDepth]         = useState(null);
  const [convLatestStatus,setConvLatestStatus] = useState({});
  const fileRef     = useRef(null);
  const textareaRef = useRef(null);
  const pollRef     = useRef(null);

  const loadConversations = useCallback(async(silent=false)=>{
    if(!silent) setLoading(true);
    try{
      const r = await conversationsApi.list();
      setConvList(r.conversations||[]);
    }
    catch(e){if(!silent)setError(e.message);}
    finally{if(!silent)setLoading(false);}
  },[]);

  useEffect(()=>{
    loadConversations();
    const poll = () => {
      reviewsApi.list().then(r=>setPendingReviews((r.reviews||[]).filter(rv=>rv.status==='pending'))).catch(()=>{});
      swarm.status().then(s=>setSwarmDepth(s.queue_depth??null)).catch(()=>{});
    };
    poll();
    pollRef.current = setInterval(()=>{ loadConversations(true); poll(); },12000);
    return()=>clearInterval(pollRef.current);
  },[]);

  // Auto-select first conversation
  useEffect(()=>{ if(!selectedConvId&&convList.length>0) setSelectedConvId(convList[0].id); },[convList]);

  function onStatusChange(agentId, status) {
    setAgentStatuses(p=>({...p,[agentId]:status}));
  }
  function onTerminal(agentId, ev) {
    setTerminalEvents(p=>({...p,[agentId]:ev}));
    onStatusChange(agentId, ev.type==='goal_complete'?'completed':'failed');
  }

  async function send() {
    if(!input.trim()) return;
    setSending(true); setError('');
    try {
      const imgs = await Promise.all(images.map(f=>new Promise(res=>{
        const r=new FileReader(); r.onload=()=>res({name:f.name,data:r.result}); r.readAsDataURL(f);
      })));
      const res = await agents.createGoal(input.trim(), imgs, selectedConvId);
      setInput(''); setImages([]);
      if(textareaRef.current) textareaRef.current.style.height='auto';
      // If we were on null (new conversation), select the new one
      if(!selectedConvId) setSelectedConvId(res.conversation_id);
      await loadConversations(true);
    } catch(e){
      if(e.message.startsWith('PAYMENT_REQUIRED:')) {
        setError('PLAN_LIMIT');
      } else {
        setError(e.message);
      }
    }
    finally{setSending(false);}
  }

  const selectedConv = selectedConvId ? convList.find(c=>c.id===selectedConvId) : null;

  return (
    <div className="flex h-screen bg-bg overflow-hidden">

      {/* ── Sidebar ──────────────────────────────────────────── */}
      <aside className="w-60 flex flex-col border-r border-border bg-bg-card shrink-0">
        <div className="flex items-center justify-between px-4 py-4 border-b border-border">
          <p className="font-serif text-xl text-tx-1">Narayan</p>
          <div className="flex items-center gap-0.5">
            {pendingReviews.length>0 && (
              <button onClick={()=>onNavigate('settings')}
                className="relative p-1.5 rounded-lg text-warn hover:bg-warn-soft transition-all" title={`${pendingReviews.length} pending review${pendingReviews.length>1?'s':''}`}>
                <Bell size={15}/>
                <span className="absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] rounded-full bg-warn text-bg-card text-[9px] font-bold flex items-center justify-center px-0.5">
                  {pendingReviews.length}
                </span>
              </button>
            )}
            <button onClick={()=>onNavigate('settings')}
              className="p-1.5 rounded-lg text-tx-3 hover:text-tx-1 hover:bg-bg-hover transition-all" title="Settings">
              <Settings size={15}/>
            </button>
            <button onClick={()=>onNavigate('logout')}
              className="p-1.5 rounded-lg text-tx-3 hover:text-err hover:bg-err-soft transition-all" title="Sign out">
              <LogOut size={15}/>
            </button>
          </div>
        </div>

        <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {loading ? (
            <div className="flex items-center justify-center py-8">
              <Loader2 size={16} className="text-tx-4 animate-spin"/>
            </div>
          ) : convList.length===0 ? (
            <div className="px-3 py-8 text-center">
              <p className="text-[13px] text-tx-3">No conversations yet.</p>
              <p className="text-[11px] text-tx-4 mt-1">Send your first message below.</p>
            </div>
          ) : convList.map(conv=>(
            <ConversationRow key={conv.id}
              conv={conv}
              selected={conv.id===selectedConvId}
              latestStatus={convLatestStatus[conv.id]||'completed'}
              onClick={()=>setSelectedConvId(conv.id)}/>
          ))}
        </div>

        <div className="p-2 border-t border-border space-y-1">
          <button onClick={()=>{setSelectedConvId(null);textareaRef.current?.focus();}}
            className="w-full flex items-center gap-2 rounded-lg px-3 py-2 text-[13px] text-tx-3 hover:text-tx-1 hover:bg-bg-hover transition-all">
            <Plus size={13}/> New conversation
          </button>
          {swarmDepth !== null && swarmDepth > 0 && (
            <div className="flex items-center gap-2 rounded-lg px-3 py-1.5 text-[11px] text-tx-4">
              <GitBranch size={11} className="text-vio shrink-0"/>
              <span className="text-vio font-mono">{swarmDepth}</span>
              <span>sub-agent{swarmDepth !== 1 ? 's' : ''} queued</span>
            </div>
          )}
        </div>
      </aside>

      {/* ── Main ─────────────────────────────────────────────── */}
      <main className="flex flex-col flex-1 min-w-0">

        {/* Header */}
        {selectedConv ? (
          <div className="flex items-center justify-between px-6 py-3 border-b border-border bg-bg-card/80 backdrop-blur shrink-0">
            <div className="min-w-0">
              <p className="text-[13px] font-medium text-tx-1 truncate max-w-lg">{selectedConv.title||'Conversation'}</p>
              <div className="flex items-center gap-2 mt-0.5 text-[11px] text-tx-4">
                <span>{selectedConv.agent_count||0} message{(selectedConv.agent_count||0)!==1?'s':''}</span>
                <span>·</span>
                <span>{timeAgo(selectedConv.updated_at)}</span>
              </div>
            </div>
          </div>
        ) : (
          <div className="px-6 py-3 border-b border-border bg-bg-card/80 shrink-0">
            <p className="text-[13px] text-tx-3">New conversation</p>
          </div>
        )}

        {/* Content */}
        <div className="flex-1 overflow-y-auto">
          {selectedConvId ? (
            <ConversationThread
              convId={selectedConvId}
              agentStatuses={agentStatuses}
              terminalEvents={terminalEvents}
              onStatusChange={onStatusChange}
              onTerminal={onTerminal}
              onNavigateSettings={()=>onNavigate('settings')}/>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-center px-8">
              <p className="font-serif text-2xl text-tx-1 mb-2">What should your agent do?</p>
              <p className="text-[13px] text-tx-3 max-w-xs leading-relaxed">
                Describe a goal. Your agent will plan, execute, and report back — no human steps needed.
              </p>
            </div>
          )}
        </div>

        {/* Input */}
        <div className="border-t border-border bg-bg-card px-4 py-4 shrink-0">
          {error==='PLAN_LIMIT' ? (
            <div className="flex items-center gap-3 rounded-xl bg-warn-soft border border-warn/25 px-4 py-3 mb-3 animate-fade">
              <AlertTriangle size={14} className="text-warn shrink-0"/>
              <div className="flex-1 min-w-0">
                <p className="text-[13px] font-medium text-warn">Step limit reached</p>
                <p className="text-[12px] text-warn/80">Upgrade your plan or buy a credit top-up to keep running agents.</p>
              </div>
              <button onClick={()=>onNavigate('settings')}
                className="shrink-0 rounded-lg bg-warn px-3 py-1.5 text-[12px] font-semibold text-bg-card hover:bg-warn/90 transition-all">
                Upgrade
              </button>
              <button onClick={()=>setError('')} className="text-warn/60 hover:text-warn transition-colors shrink-0">
                <X size={13}/>
              </button>
            </div>
          ) : error ? (
            <div className="flex items-center gap-2 rounded-lg bg-err-soft border border-err/20 px-3 py-2 mb-3 text-[13px] text-err animate-fade">
              <AlertCircle size={13}/>{error}
              <button onClick={()=>setError('')} className="ml-auto"><X size={13}/></button>
            </div>
          ) : null}
          {images.length>0&&(
            <div className="flex items-center gap-2 mb-3">
              {images.map((f,i)=>(
                <ImageChip key={i} file={f} onRemove={()=>setImages(p=>p.filter((_,j)=>j!==i))}/>
              ))}
            </div>
          )}

          <div className="flex items-end gap-2.5 rounded-xl border border-border bg-bg px-4 py-3 focus-within:border-border-md focus-within:ring-2 focus-within:ring-accent/10 transition-all">
            <button onClick={()=>fileRef.current?.click()}
              className="p-1 rounded-md text-tx-4 hover:text-tx-2 hover:bg-bg-active transition-all shrink-0 mb-0.5" title="Attach image">
              <Paperclip size={16}/>
            </button>
            <input ref={fileRef} type="file" accept="image/*" multiple className="hidden"
              onChange={e=>setImages(p=>[...p,...Array.from(e.target.files)].slice(0,5))}/>
            <textarea ref={textareaRef} value={input}
              onChange={e=>setInput(e.target.value)}
              onKeyDown={e=>{if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();send();}}}
              placeholder="Send a message…"
              rows={1}
              className="flex-1 bg-transparent text-[13px] text-tx-1 placeholder-tx-4 outline-none resize-none leading-relaxed max-h-32"
              style={{overflow:input.split('\n').length>4?'auto':'hidden'}}
              onInput={e=>{e.target.style.height='auto';e.target.style.height=Math.min(e.target.scrollHeight,128)+'px';}}/>
            <button onClick={send} disabled={sending||!input.trim()}
              className={clsx('p-2 rounded-lg transition-all shrink-0 mb-0.5',
                input.trim()&&!sending?'bg-tx-1 text-bg-card hover:bg-tx-2 active:scale-95':'bg-bg-active text-tx-4 cursor-not-allowed')}>
              {sending?<Loader2 size={15} className="animate-spin"/>:<Send size={15}/>}
            </button>
          </div>
          <p className="text-[11px] text-tx-4 mt-2 text-center">
            {selectedConvId ? 'Follow-up messages continue this conversation' : 'Start a new conversation'} · Shift+Enter for newline
          </p>
        </div>
      </main>
    </div>
  );
}
