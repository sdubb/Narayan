import { useState, useEffect, useCallback } from 'react';
import {
  Key, Network, BarChart3, Trash2, Plus, Loader2,
  CheckCircle2, AlertCircle, Eye, EyeOff, ChevronLeft,
  Save, Activity, DollarSign, Zap, BookOpen, Download, Upload,
  Bell, Link2, Plug, Clock, RotateCcw, Shield, RefreshCw,
  ChevronDown, GitBranch, Database, ArrowRight, CreditCard,
  ExternalLink, Copy, CheckCheck, X,
} from 'lucide-react';
import { credentials, routing, metrics, skills, reviews, citations, swarm, agents, autoApprovals, connectors, billing } from '../api';
import clsx from 'clsx';

const PROVIDERS = [
  { id:'anthropic',  label:'Anthropic',  models:['claude-sonnet-4-20250514','claude-opus-4-20250514','claude-haiku-4-5-20251001'] },
  { id:'openai',     label:'OpenAI',     models:['gpt-4o','gpt-4o-mini','o1','o3-mini'] },
  { id:'groq',       label:'Groq',       models:['llama-3.3-70b-versatile','llama-3.1-8b-instant','mixtral-8x7b-32768'] },
  { id:'gemini',     label:'Gemini',     models:['gemini-2.0-flash','gemini-2.0-pro','gemini-1.5-pro'] },
  { id:'nvidia',     label:'NVIDIA',     models:['meta/llama-3.1-70b-instruct','meta/llama-3.1-8b-instruct','nvidia/llama-3.1-nemotron-70b-instruct'] },
  { id:'openrouter', label:'OpenRouter', models:['openai/gpt-4o','anthropic/claude-3-5-sonnet','meta-llama/llama-3.3-70b-instruct'] },
  { id:'ollama',     label:'Ollama',     models:['llama3.3','qwen2.5-coder','deepseek-r1'] },
  { id:'compatible', label:'Compatible', models:['custom-model'] },
];

const COMPLEXITY = {
  simple:  {label:'Simple',  desc:'Evaluator, preflight, clarifier'},
  medium:  {label:'Medium',  desc:'Reflector calls'},
  complex: {label:'Complex', desc:'Planner calls'},
  fallback:{label:'Fallback',desc:'If preferred provider fails'},
};

function Spinner() {
  return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin"/></div>;
}

export default function SettingsPage({onBack}) {
  const [tab,setTab]     = useState('credentials');
  const [error,setError] = useState('');
  const [ok,setOk]       = useState('');
  function flash(m){setOk(m);setTimeout(()=>setOk(''),3000);}

  const tabs = [
    {id:'credentials',  label:'Credentials',    icon:Key},
    {id:'routing',      label:'Routing',         icon:Network},
    {id:'usage',        label:'Usage',           icon:BarChart3},
    {id:'skills',       label:'Skills',          icon:BookOpen},
    {id:'reviews',      label:'Reviews',         icon:Bell},
    {id:'citations',    label:'Citations',       icon:Link2},
    {id:'connectors',   label:'Connectors',      icon:Plug},
    {id:'autoapprovals',label:'Auto-approvals',  icon:Shield},
    {id:'billing',      label:'Billing',         icon:DollarSign},
  ];

  return (
    <div className="min-h-screen bg-bg">
      {/* Header */}
      <div className="border-b border-border bg-bg-card sticky top-0 z-10">
        <div className="max-w-2xl mx-auto px-6 py-4 flex items-center gap-4">
          <button onClick={onBack} className="flex items-center gap-1.5 text-sm text-tx-3 hover:text-tx-1 transition-colors">
            <ChevronLeft size={15}/> Back
          </button>
          <p className="font-serif text-xl text-tx-1 flex-1">Settings</p>
        </div>
        <div className="max-w-2xl mx-auto px-6 flex gap-0 overflow-x-auto">
          {tabs.map(t=>{
            const Icon=t.icon;
            return(
              <button key={t.id} onClick={()=>{setError('');setTab(t.id);}}
                className={clsx('flex items-center gap-1.5 px-4 py-2.5 text-sm font-medium border-b-2 transition-all whitespace-nowrap',
                  tab===t.id?'border-accent text-accent':'border-transparent text-tx-3 hover:text-tx-1')}>
                <Icon size={14}/>{t.label}
              </button>
            );
          })}
        </div>
      </div>

      <div className="max-w-2xl mx-auto px-6 py-8 space-y-6">
        {error&&(
          <div className="flex items-start gap-2 rounded-xl bg-err-soft border border-err/20 px-4 py-3 text-sm text-err animate-fade">
            <AlertCircle size={14} className="mt-0.5 shrink-0"/>{error}
          </div>
        )}
        {ok&&(
          <div className="flex items-center gap-2 rounded-xl bg-ok-soft border border-ok/20 px-4 py-3 text-sm text-ok animate-fade">
            <CheckCircle2 size={14}/>{ok}
          </div>
        )}
        {tab==='credentials'  && <CredTab setError={setError} flash={flash}/>}
        {tab==='routing'      && <RoutTab setError={setError} flash={flash}/>}
        {tab==='usage'        && <UsageTab setError={setError}/>}
        {tab==='skills'       && <SkillsTab setError={setError} flash={flash}/>}
        {tab==='reviews'      && <ReviewsTab setError={setError} flash={flash}/>}
        {tab==='citations'    && <CitationsTab setError={setError}/>}
        {tab==='connectors'   && <ConnectorsTab setError={setError} flash={flash}/>}
        {tab==='autoapprovals'&& <AutoApprovalsTab setError={setError} flash={flash}/>}
        {tab==='billing'      && <BillingTab setError={setError} flash={flash}/>}
      </div>
    </div>
  );
}

function CredTab({setError,flash}) {
  const [creds,setCreds]   = useState([]);
  const [loading,setLoading] = useState(true);
  const [form,setForm]     = useState({provider:'anthropic',api_key:'',model:'',label:''});
  const [showKey,setShowKey] = useState(false);
  const [adding,setAdding] = useState(false);

  const load = useCallback(async()=>{
    setLoading(true);
    try{const r=await credentials.list();setCreds(r.credentials||[]);}
    catch(e){setError(e.message);}
    finally{setLoading(false);}
  },[]);
  useEffect(()=>{load();},[]);

  async function add(e) {
    e.preventDefault(); if(!form.api_key.trim()) return;
    setAdding(true); setError('');
    try {
      const model=form.model||PROVIDERS.find(p=>p.id===form.provider)?.models[0]||'';
      await credentials.set(form.provider,form.api_key,model,form.label||form.provider);
      setForm({provider:'anthropic',api_key:'',model:'',label:''});
      await load(); flash('Credential saved.');
    } catch(e){setError(e.message);}
    finally{setAdding(false);}
  }

  async function del(provider) {
    try{await credentials.delete(provider);setCreds(p=>p.filter(c=>c.provider!==provider));flash(`${provider} removed.`);}
    catch(e){setError(e.message);}
  }

  if(loading) return <Spinner/>;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-serif text-xl text-tx-1 mb-1">Provider Credentials</h2>
        <p className="text-sm text-tx-3">Narayan is BYOK — your keys are encrypted at rest and never leave your instance.</p>
      </div>

      {creds.length>0&&(
        <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
          <div className="px-4 py-3 border-b border-border">
            <span className="text-xs font-medium text-tx-3 uppercase tracking-wide">Active</span>
          </div>
          <div className="divide-y divide-border/60">
            {creds.map(c=>(
              <div key={c.provider} className="flex items-center gap-4 px-4 py-3.5">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-0.5">
                    <span className="text-sm font-medium text-tx-1 capitalize">{c.provider}</span>
                    {c.enabled&&<CheckCircle2 size={13} className="text-ok"/>}
                  </div>
                  <p className="text-xs text-tx-3">{c.model} · {c.label}</p>
                </div>
                <button onClick={()=>del(c.provider)}
                  className="p-1.5 rounded-lg text-tx-4 hover:text-err hover:bg-err-soft transition-all">
                  <Trash2 size={14}/>
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="rounded-xl border border-border bg-bg-card p-5 shadow-sm">
        <h3 className="text-sm font-medium text-tx-1 mb-4 flex items-center gap-2">
          <Plus size={14} className="text-accent"/> Add key
        </h3>
        <form onSubmit={add} className="space-y-4">
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="block text-xs font-medium text-tx-3 mb-1.5">Provider</label>
              <select value={form.provider}
                onChange={e=>{const p=PROVIDERS.find(p=>p.id===e.target.value);setForm(f=>({...f,provider:e.target.value,model:p?.models[0]||''}));}}
                className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all">
                {PROVIDERS.map(p=><option key={p.id} value={p.id}>{p.label}</option>)}
              </select>
            </div>
            <div>
              <label className="block text-xs font-medium text-tx-3 mb-1.5">Model</label>
              <select value={form.model} onChange={e=>setForm(f=>({...f,model:e.target.value}))}
                className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all">
                {(PROVIDERS.find(p=>p.id===form.provider)?.models||[]).map(m=><option key={m} value={m}>{m}</option>)}
              </select>
            </div>
          </div>
          <div>
            <label className="block text-xs font-medium text-tx-3 mb-1.5">API Key</label>
            <div className="flex items-center rounded-lg border border-border bg-bg px-3 focus-within:border-border-md focus-within:ring-2 focus-within:ring-accent/10 transition-all">
              <input type={showKey?'text':'password'} value={form.api_key} onChange={e=>setForm(f=>({...f,api_key:e.target.value}))}
                placeholder="sk-…" required className="flex-1 bg-transparent py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none font-mono"/>
              <button type="button" onClick={()=>setShowKey(s=>!s)} className="text-tx-4 hover:text-tx-2 p-1 transition-colors">
                {showKey?<EyeOff size={14}/>:<Eye size={14}/>}
              </button>
            </div>
          </div>
          <div>
            <label className="block text-xs font-medium text-tx-3 mb-1.5">Label (optional)</label>
            <input value={form.label} onChange={e=>setForm(f=>({...f,label:e.target.value}))}
              placeholder="Production key"
              className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all"/>
          </div>
          <button type="submit" disabled={adding||!form.api_key.trim()}
            className="flex items-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all disabled:opacity-50 active:scale-[0.98]">
            {adding?<Loader2 size={14} className="animate-spin"/>:<Save size={14}/>}
            {adding?'Saving…':'Save credential'}
          </button>
        </form>
      </div>
    </div>
  );
}

function RoutTab({setError,flash}) {
  const [creds,setCreds]     = useState([]);
  const [cfg,setCfg]         = useState({simple:'',medium:'',complex:'',fallback:''});
  const [loading,setLoading] = useState(true);
  const [saving,setSaving]   = useState(false);

  useEffect(()=>{
    (async()=>{
      try{const r=await credentials.list();setCreds(r.credentials||[]);}
      catch(e){setError(e.message);}
      finally{setLoading(false);}
    })();
  },[]);

  async function save(){
    setSaving(true);
    try{await routing.update(cfg);flash('Routing saved.');}
    catch(e){setError(e.message);}
    finally{setSaving(false);}
  }

  if(loading) return <Spinner/>;
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-serif text-xl text-tx-1 mb-1">LLM Routing</h2>
        <p className="text-sm text-tx-3">Choose which provider handles each task complexity tier.</p>
      </div>
      <div className="rounded-xl border border-border bg-bg-card p-5 shadow-sm space-y-5">
        {Object.entries(COMPLEXITY).map(([key,info])=>(
          <div key={key}>
            <div className="mb-1.5">
              <span className="text-sm font-medium text-tx-1">{info.label}</span>
              <p className="text-xs text-tx-3 mt-0.5">{info.desc}</p>
            </div>
            <select value={cfg[key]||''} onChange={e=>setCfg(r=>({...r,[key]:e.target.value}))}
              className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all">
              <option value="">— select provider —</option>
              {creds.map(c=><option key={c.provider} value={c.provider}>{c.provider} ({c.model})</option>)}
            </select>
          </div>
        ))}
        <button onClick={save} disabled={saving}
          className="flex items-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all disabled:opacity-50 active:scale-[0.98]">
          {saving?<Loader2 size={14} className="animate-spin"/>:<Save size={14}/>}
          {saving?'Saving…':'Save routing'}
        </button>
      </div>
    </div>
  );
}

function UsageTab({setError}) {
  const [m,setM]         = useState(null);
  const [c,setC]         = useState(null);
  const [loading,setLoading] = useState(true);

  useEffect(()=>{
    (async()=>{
      try{const[mv,cv]=await Promise.all([metrics.get(),metrics.costs()]);setM(mv);setC(cv);}
      catch(e){setError(e.message);}
      finally{setLoading(false);}
    })();
  },[]);

  if(loading) return <Spinner/>;
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-serif text-xl text-tx-1 mb-1">Usage & Costs</h2>
        <p className="text-sm text-tx-3">Live data from your Narayan instance.</p>
      </div>

      {/* Spend limit bar */}
      {c && c.spend_limit_usd > 0 && (
        <div className="rounded-xl border border-border bg-bg-card p-4 shadow-sm">
          <div className="flex items-center justify-between mb-2">
            <span className="text-sm font-medium text-tx-1">Spend limit</span>
            <span className="font-mono text-sm text-tx-2">
              ${(c.current_spend_usd??0).toFixed(4)} / ${c.spend_limit_usd.toFixed(2)}
            </span>
          </div>
          <div className="h-1.5 rounded-full bg-bg-active overflow-hidden">
            <div
              className={clsx('h-full rounded-full transition-all',
                (c.pct_used??0) >= 90 ? 'bg-err' : (c.pct_used??0) >= 70 ? 'bg-warn' : 'bg-ok')}
              style={{width:`${Math.min(c.pct_used??0,100)}%`}}
            />
          </div>
          <p className="text-xs text-tx-4 mt-1">{(c.pct_used??0).toFixed(1)}% used</p>
        </div>
      )}

      {/* Metrics cards — use aliases from fixed backend */}
      {m&&(
        <div className="grid grid-cols-2 gap-3">
          {[
            {label:'Goals created',   value:m.goals_total??m.agents_started??'—',    icon:Activity},
            {label:'Agents running',  value:m.agents_running??'—',                   icon:CheckCircle2},
            {label:'Steps completed', value:m.steps_total??m.steps_completed??'—',   icon:Zap},
            {label:'Steps / min',     value:m.steps_per_minute??'—',                 icon:BarChart3},
          ].map(item=>{
            const Icon=item.icon;
            return(
              <div key={item.label} className="rounded-xl border border-border bg-bg-card p-4 shadow-sm">
                <div className="flex items-center gap-2 mb-2">
                  <Icon size={13} className="text-tx-3"/>
                  <span className="text-xs text-tx-3">{item.label}</span>
                </div>
                <p className="font-serif text-3xl text-tx-1">{item.value}</p>
              </div>
            );
          })}
        </div>
      )}

      {/* Cost breakdown — new shape: { total_usd, usage: { provider: { usd, input_tokens, output_tokens } } } */}
      {c&&(
        <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
          <div className="flex items-center justify-between px-4 py-3 border-b border-border">
            <span className="text-sm font-medium text-tx-1 flex items-center gap-2">
              <DollarSign size={14} className="text-tx-3"/> Token spend
            </span>
            <span className="font-mono text-sm font-medium text-accent">
              ${(c.total_usd??c.current_spend_usd??0).toFixed(4)}
            </span>
          </div>
          <div className="divide-y divide-border/60">
            {Object.entries(c.usage||{}).map(([provider,data])=>(
              <div key={provider} className="flex items-center gap-4 px-4 py-3.5">
                <div className="flex-1">
                  <span className="text-sm font-medium text-tx-1 capitalize">{provider}</span>
                  <div className="flex items-center gap-3 mt-0.5">
                    <span className="text-xs text-tx-3 font-mono">↑ {(data.input_tokens??0).toLocaleString()}</span>
                    <span className="text-xs text-tx-3 font-mono">↓ {(data.output_tokens??0).toLocaleString()}</span>
                  </div>
                </div>
                <span className="font-mono text-sm text-tx-2">${(data.usd??0).toFixed(4)}</span>
              </div>
            ))}
            {!Object.keys(c.usage||{}).length&&(
              <div className="px-4 py-8 text-sm text-tx-3 text-center">No usage recorded yet.</div>
            )}
          </div>
        </div>
      )}

      {/* LLM stats */}
      {m && (m.llm_calls_total > 0 || m.llm_cache_hits > 0) && (
        <div className="rounded-xl border border-border bg-bg-card p-4 shadow-sm">
          <p className="text-xs font-medium text-tx-3 mb-3 uppercase tracking-wide">LLM gateway</p>
          <div className="grid grid-cols-3 gap-4 text-center">
            {[
              {label:'Total calls',  value:(m.llm_calls_total??0).toLocaleString()},
              {label:'Cache hits',   value:(m.llm_cache_hits??0).toLocaleString()},
              {label:'Uptime',       value:`${Math.floor((m.uptime_secs??0)/3600)}h ${Math.floor(((m.uptime_secs??0)%3600)/60)}m`},
            ].map(item=>(
              <div key={item.label}>
                <p className="font-mono text-lg text-tx-1">{item.value}</p>
                <p className="text-xs text-tx-4 mt-0.5">{item.label}</p>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function SkillsTab({setError,flash}) {
  const [marketplace,setMarketplace] = useState([]);
  const [registry,setRegistry]       = useState([]);
  const [loading,setLoading]         = useState(true);
  const [installing,setInstalling]   = useState({});
  const [showForm,setShowForm]       = useState(false);
  const [form,setForm]               = useState({name:'',description:'',steps:'',author:''});
  const [uploading,setUploading]     = useState(false);

  const load = useCallback(async()=>{
    setLoading(true);
    try{const[m,r]=await Promise.all([skills.list(),skills.registry()]);setMarketplace(m.skills||[]);setRegistry(r.skills||[]);}
    catch(e){setError(e.message);}
    finally{setLoading(false);}
  },[]);
  useEffect(()=>{load();},[]);

  async function install(name){
    setInstalling(p=>({...p,[name]:true}));
    try{await skills.install(name);await load();flash(`${name} installed.`);}
    catch(e){setError(e.message);}
    finally{setInstalling(p=>({...p,[name]:false}));}
  }

  async function publish(e){
    e.preventDefault();
    setUploading(true);
    try{
      await skills.upload(form.name.trim(),form.description,form.steps.split('\n').map(s=>s.trim()).filter(Boolean),form.author||undefined);
      setForm({name:'',description:'',steps:'',author:''});setShowForm(false);await load();flash('Skill published.');
    }catch(e){setError(e.message);}
    finally{setUploading(false);}
  }

  const installedNames = new Set(registry.map(s=>s.name));
  if(loading) return <Spinner/>;

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <h2 className="font-serif text-xl text-tx-1 mb-1">Skills</h2>
          <p className="text-sm text-tx-3">Reusable step sequences for common job types.</p>
        </div>
        <button onClick={()=>setShowForm(s=>!s)}
          className="flex items-center gap-2 rounded-lg border border-border bg-bg-card px-3.5 py-2 text-sm text-tx-2 hover:text-tx-1 hover:border-border-md transition-all shadow-sm">
          <Upload size={13}/> Publish
        </button>
      </div>

      {showForm&&(
        <div className="rounded-xl border border-border bg-bg-card p-5 shadow-sm animate-in">
          <h3 className="text-sm font-medium text-tx-1 mb-4">New skill</h3>
          <form onSubmit={publish} className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-xs font-medium text-tx-3 mb-1.5">Name</label>
                <input value={form.name} onChange={e=>setForm(f=>({...f,name:e.target.value}))}
                  placeholder="github_pr_creator" required
                  className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all font-mono"/>
              </div>
              <div>
                <label className="block text-xs font-medium text-tx-3 mb-1.5">Author</label>
                <input value={form.author} onChange={e=>setForm(f=>({...f,author:e.target.value}))}
                  placeholder="optional"
                  className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all"/>
              </div>
            </div>
            <div>
              <label className="block text-xs font-medium text-tx-3 mb-1.5">Description</label>
              <input value={form.description} onChange={e=>setForm(f=>({...f,description:e.target.value}))}
                placeholder="What does this skill do?"
                className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all"/>
            </div>
            <div>
              <label className="block text-xs font-medium text-tx-3 mb-1.5">Steps <span className="font-normal text-tx-4">(one per line)</span></label>
              <textarea value={form.steps} onChange={e=>setForm(f=>({...f,steps:e.target.value}))}
                placeholder={"clone the repository\nmodify the file\ncommit\nopen pull request"}
                rows={4} required
                className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all resize-none font-mono"/>
            </div>
            <div className="flex items-center gap-3">
              <button type="submit" disabled={uploading||!form.name.trim()||!form.steps.trim()}
                className="flex items-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all disabled:opacity-50 active:scale-[0.98]">
                {uploading?<Loader2 size={14} className="animate-spin"/>:<Upload size={14}/>}
                {uploading?'Publishing…':'Publish'}
              </button>
              <button type="button" onClick={()=>setShowForm(false)} className="text-sm text-tx-3 hover:text-tx-1 transition-colors">Cancel</button>
            </div>
          </form>
        </div>
      )}

      {registry.length>0&&(
        <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
          <div className="px-4 py-3 border-b border-border flex items-center gap-2">
            <CheckCircle2 size={13} className="text-ok"/>
            <span className="text-sm font-medium text-tx-1">Installed ({registry.length})</span>
          </div>
          <div className="divide-y divide-border/60">
            {registry.map(s=>(
              <div key={s.name} className="px-4 py-3.5">
                <div className="flex items-center gap-2 mb-0.5">
                  <span className="text-sm font-medium text-tx-1 font-mono">{s.name}</span>
                  <span className="text-xs text-tx-4">v{s.version}</span>
                  <span className="ml-auto text-xs text-tx-4">{s.step_count} steps</span>
                </div>
                {s.description&&<p className="text-xs text-tx-3">{s.description}</p>}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
        <div className="px-4 py-3 border-b border-border flex items-center gap-2">
          <BookOpen size={13} className="text-tx-3"/>
          <span className="text-sm font-medium text-tx-1">Marketplace ({marketplace.length})</span>
        </div>
        {marketplace.length===0 ? (
          <div className="px-4 py-8 text-sm text-tx-3 text-center">No skills yet. Publish one above.</div>
        ) : (
          <div className="divide-y divide-border/60">
            {marketplace.map(s=>{
              const isInst=installedNames.has(s.name);
              return(
                <div key={s.name} className="flex items-center gap-4 px-4 py-3.5">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-0.5">
                      <span className="text-sm font-medium text-tx-1 font-mono">{s.name}</span>
                      {s.author&&<span className="text-xs text-tx-4">by {s.author}</span>}
                      <span className="ml-auto text-xs text-tx-4">{s.step_count} steps</span>
                    </div>
                    {s.description&&<p className="text-xs text-tx-3 truncate">{s.description}</p>}
                  </div>
                  <button onClick={()=>!isInst&&install(s.name)} disabled={isInst||installing[s.name]}
                    className={clsx('flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition-all shrink-0',
                      isInst?'border-ok/30 bg-ok-soft text-ok cursor-default':'border-border bg-bg text-tx-2 hover:border-accent/40 hover:text-accent disabled:opacity-50')}>
                    {installing[s.name]?<Loader2 size={10} className="animate-spin"/>:isInst?<CheckCircle2 size={10}/>:<Download size={10}/>}
                    {isInst?'Installed':'Install'}
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── REVIEWS TAB ──────────────────────────────────────────
// Full review queue with all 4 resolution options + notes
// ═══════════════════════════════════════════════════════════
function ReviewsTab({setError, flash}) {
  const [items,        setItems]        = useState([]);
  const [loading,      setLoading]      = useState(true);
  const [filter,       setFilter]       = useState('pending');
  const [bulkResolving,setBulkResolving]= useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try { const r = await reviews.list(); setItems(r.reviews||[]); }
    catch(e) { setError(e.message); }
    finally { setLoading(false); }
  }, []);

  useEffect(()=>{ load(); },[]);

  async function resolveAll() {
    if (!window.confirm('Approve all pending reviews? This cannot be undone.')) return;
    setBulkResolving(true);
    try {
      await reviews.resolveAll('approved', 'Bulk approved from Settings');
      await load();
      flash('All pending reviews approved.');
    } catch(e) { setError(e.message); }
    finally { setBulkResolving(false); }
  }

  const shown = filter==='pending' ? items.filter(i=>i.status==='pending') : items;
  const pendingCount = items.filter(i=>i.status==='pending').length;

  if (loading) return <Spinner/>;

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <h2 className="font-serif text-xl text-tx-1 mb-1">Review Queue</h2>
          <p className="text-sm text-tx-3">Policy-gated actions waiting for your approval before the agent proceeds.</p>
        </div>
        <div className="flex items-center gap-2">
          {pendingCount > 1 && (
            <button onClick={resolveAll} disabled={bulkResolving}
              className="flex items-center gap-1.5 rounded-lg bg-ok-soft border border-ok/30 px-3 py-1.5 text-xs font-medium text-ok hover:bg-ok/10 transition-all disabled:opacity-50">
              {bulkResolving ? <Loader2 size={11} className="animate-spin"/> : <CheckCircle2 size={11}/>}
              Approve all ({pendingCount})
            </button>
          )}
          <div className="flex rounded-lg border border-border bg-bg overflow-hidden text-xs font-medium">
            {['pending','all'].map(f=>(
              <button key={f} onClick={()=>setFilter(f)}
                className={clsx('px-3 py-1.5 capitalize transition-colors',
                  filter===f?'bg-bg-active text-tx-1':'text-tx-3 hover:text-tx-1')}>
                {f}
              </button>
            ))}
          </div>
          <button onClick={load} className="p-1.5 rounded-lg border border-border text-tx-3 hover:text-tx-1 hover:bg-bg-hover transition-all" title="Refresh">
            <RefreshCw size={13}/>
          </button>
        </div>
      </div>

      {shown.length===0 ? (
        <div className="rounded-xl border border-border bg-bg-card py-16 text-center shadow-sm">
          <CheckCircle2 size={24} className="text-ok mx-auto mb-3"/>
          <p className="text-sm text-tx-2 font-medium">Queue is clear</p>
          <p className="text-xs text-tx-4 mt-1">No {filter==='pending'?'pending ':''}reviews.</p>
        </div>
      ) : (
        <div className="space-y-3">
          {shown.map(item => <ReviewItem key={item.id} item={item} onResolved={()=>{load();flash('Review resolved.');}}/>)}
        </div>
      )}
    </div>
  );
}

function ReviewItem({item, onResolved}) {
  const [open,       setOpen]       = useState(item.status==='pending');
  const [resolving,  setResolving]  = useState(false);
  const [resolution, setResolution] = useState(item.status!=='pending'?item.status:null);
  const [note,       setNote]       = useState('');
  const [showNote,   setShowNote]   = useState(false);
  const [err,        setErr]        = useState('');

  const isDone = resolution && resolution!=='pending';

  const STATUS_STYLE = {
    pending:            'bg-warn-soft text-warn border-warn/25',
    approved:           'bg-ok-soft text-ok border-ok/25',
    auto_approved:      'bg-ok-soft text-ok border-ok/25',
    changes_requested:  'bg-warn-soft text-warn border-warn/25',
    rejected:           'bg-err-soft text-err border-err/25',
  };

  async function resolve(status) {
    setResolving(true); setErr('');
    try {
      await reviews.resolve(item.id, status, note.trim()||`Resolved (${status}) from Settings`);
      setResolution(status);
      onResolved?.();
    } catch(e){ setErr(e.message); }
    finally { setResolving(false); }
  }

  return (
    <div className={clsx('rounded-xl border bg-bg-card shadow-sm overflow-hidden',
      item.status==='pending' ? 'border-warn/25' : 'border-border')}>
      {/* Header row */}
      <button onClick={()=>setOpen(o=>!o)}
        className="w-full flex items-start gap-3 px-4 py-3.5 hover:bg-bg-hover transition-colors text-left">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-sm font-medium text-tx-1">{item.summary||item.message||'Review item'}</span>
            <span className={clsx('inline-flex items-center text-[10px] font-semibold px-2 py-0.5 rounded border capitalize', STATUS_STYLE[item.status]||STATUS_STYLE.pending)}>
              {item.status?.replace('_',' ')||'pending'}
            </span>
          </div>
          <div className="flex items-center gap-3 mt-1 text-[11px] text-tx-4">
            {item.rule_id && <span className="font-mono">{item.rule_id}</span>}
            {item.agent_id && <span>agent {item.agent_id.slice(0,10)}…</span>}
            {item.created_at && <span>{new Date(item.created_at).toLocaleString()}</span>}
          </div>
        </div>
        <ChevronDown size={13} className={clsx('text-tx-4 mt-0.5 transition-transform shrink-0', !open&&'-rotate-90')}/>
      </button>

      {open && (
        <div className="border-t border-border px-4 pt-3 pb-4 space-y-3">
          {item.message && (
            <div className="rounded-lg bg-bg px-3 py-2.5">
              <p className="text-xs text-tx-3 font-medium mb-0.5 uppercase tracking-wide">Context</p>
              <p className="text-sm text-tx-2">{item.message}</p>
            </div>
          )}

          {/* Note field */}
          <div>
            <button onClick={()=>setShowNote(o=>!o)}
              className="flex items-center gap-1.5 text-[11px] text-tx-3 hover:text-tx-2 transition-colors mb-2">
              <ChevronDown size={10} className={clsx('transition-transform', !showNote&&'-rotate-90')}/>
              {showNote ? 'Hide note' : 'Add note for agent'}
            </button>
            {showNote && (
              <textarea value={note} onChange={e=>setNote(e.target.value)}
                placeholder="Optional context or instructions for the agent when it retries…"
                rows={2}
                className="w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md resize-none transition-all"/>
            )}
          </div>

          {err && <p className="text-xs text-err">{err}</p>}

          {isDone ? (
            <div className={clsx('flex items-center gap-2 rounded-lg px-3 py-2.5', STATUS_STYLE[resolution])}>
              <CheckCircle2 size={13}/>
              <span className="text-sm font-medium capitalize">{resolution?.replace('_',' ')} — recorded</span>
              {note && <span className="text-xs ml-auto opacity-70 truncate max-w-xs">"{note}"</span>}
            </div>
          ) : (
            <div className="grid grid-cols-2 gap-2">
              <button onClick={()=>resolve('auto_approved')} disabled={resolving}
                className="flex flex-col items-start gap-0.5 rounded-xl border border-ok/30 bg-ok-soft px-3 py-2.5 hover:border-ok/50 hover:bg-ok/10 transition-all disabled:opacity-50 text-left">
                <div className="flex items-center gap-1.5">
                  <Zap size={11} className="text-ok"/>
                  <span className="text-[12px] font-semibold text-ok">Auto-approve</span>
                </div>
                <span className="text-[10px] text-ok/70 leading-tight">Approve & skip for this rule going forward</span>
              </button>

              <button onClick={()=>resolve('approved')} disabled={resolving}
                className="flex flex-col items-start gap-0.5 rounded-xl border border-ok/30 bg-ok-soft px-3 py-2.5 hover:border-ok/50 hover:bg-ok/10 transition-all disabled:opacity-50 text-left">
                <div className="flex items-center gap-1.5">
                  {resolving ? <Loader2 size={11} className="text-ok animate-spin"/> : <CheckCircle2 size={11} className="text-ok"/>}
                  <span className="text-[12px] font-semibold text-ok">Approve</span>
                </div>
                <span className="text-[10px] text-ok/70 leading-tight">Proceed once, ask again next time</span>
              </button>

              <button onClick={()=>{ if(!note.trim()){setShowNote(true);return;} resolve('changes_requested'); }} disabled={resolving}
                className="flex flex-col items-start gap-0.5 rounded-xl border border-warn/30 bg-warn-soft px-3 py-2.5 hover:border-warn/50 hover:bg-warn/10 transition-all disabled:opacity-50 text-left">
                <div className="flex items-center gap-1.5">
                  <RotateCcw size={11} className="text-warn"/>
                  <span className="text-[12px] font-semibold text-warn">Request changes</span>
                </div>
                <span className="text-[10px] text-warn/70 leading-tight">Retry with your note as context</span>
              </button>

              <button onClick={()=>resolve('rejected')} disabled={resolving}
                className="flex flex-col items-start gap-0.5 rounded-xl border border-err/30 bg-err-soft px-3 py-2.5 hover:border-err/50 hover:bg-err/10 transition-all disabled:opacity-50 text-left">
                <div className="flex items-center gap-1.5">
                  <AlertCircle size={11} className="text-err"/>
                  <span className="text-[12px] font-semibold text-err">Reject</span>
                </div>
                <span className="text-[10px] text-err/70 leading-tight">Block this action, agent fails step</span>
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CITATIONS TAB ────────────────────────────────────────
// Full audit trail — pick an agent, see every sourced claim
// ═══════════════════════════════════════════════════════════
function CitationsTab({setError}) {
  const [agentList, setAgentList] = useState([]);
  const [agentId,   setAgentId]   = useState('');
  const [cites,     setCites]     = useState([]);
  const [loading,   setLoading]   = useState(false);
  const [loadingAgents, setLoadingAgents] = useState(true);

  useEffect(()=>{
    agents.list()
      .then(r => {
        const list = r.agents || [];
        setAgentList(list);
        if (list.length > 0) setAgentId(list[0].id);
      })
      .catch(e => setError(e.message))
      .finally(() => setLoadingAgents(false));
  },[]);

  useEffect(()=>{
    if (!agentId) return;
    setLoading(true);
    citations.listForAgent(agentId)
      .then(r=>setCites(r.citations||[]))
      .catch(e=>setError(e.message))
      .finally(()=>setLoading(false));
  },[agentId]);

  const CONF_COLOR = c => c>=0.9?'text-ok':c>=0.6?'text-warn':'text-err';

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-serif text-xl text-tx-1 mb-1">Citation Audit Trail</h2>
        <p className="text-sm text-tx-3">Every sourced claim an agent made — tool used, confidence score, and step index.</p>
      </div>

      {/* Agent selector */}
      <div>
        <label className="block text-xs font-medium text-tx-3 mb-1.5">Select agent</label>
        {loadingAgents ? <Spinner/> : (
          <select value={agentId} onChange={e=>setAgentId(e.target.value)}
            className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all">
            {agentList.length===0 && <option value="">No agents yet</option>}
            {agentList.map(a=>(
              <option key={a.id} value={a.id}>{a.goal?.slice(0,60)} — {a.status}</option>
            ))}
          </select>
        )}
      </div>

      {loading ? <Spinner/> : cites.length===0 ? (
        <div className="rounded-xl border border-border bg-bg-card py-16 text-center shadow-sm">
          <Link2 size={22} className="text-tx-4 mx-auto mb-3"/>
          <p className="text-sm text-tx-2 font-medium">No citations yet</p>
          <p className="text-xs text-tx-4 mt-1">Citations are recorded as the agent executes tool calls.</p>
        </div>
      ) : (
        <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
          <div className="px-4 py-3 border-b border-border flex items-center gap-2">
            <Link2 size={13} className="text-vio"/>
            <span className="text-sm font-medium text-tx-1">{cites.length} citations</span>
            <span className="ml-auto text-xs text-tx-4">
              Avg confidence: {Math.round(cites.reduce((s,c)=>s+(c.confidence??1),0)/cites.length*100)}%
            </span>
          </div>
          <div className="divide-y divide-border/60 max-h-[60vh] overflow-y-auto">
            {cites.map((c,i)=>(
              <div key={i} className="flex items-start gap-3 px-4 py-3.5 hover:bg-bg-hover">
                <div className="flex flex-col items-center gap-1 shrink-0 pt-0.5">
                  <span className="font-mono text-[10px] text-accent w-5 text-right">{i+1}</span>
                  {c.step_index!=null && (
                    <span className="font-mono text-[9px] text-tx-4 bg-bg-active rounded px-1">s{c.step_index}</span>
                  )}
                </div>
                <div className="flex-1 min-w-0">
                  <p className="text-sm text-tx-1 leading-snug">{c.claim||c.summary}</p>
                  <div className="flex items-center gap-3 mt-1">
                    <span className="inline-flex items-center gap-1 text-[11px] font-mono text-tx-3">
                      <Database size={9}/> {c.source_ref||c.source_type||'tool_output'}
                    </span>
                    {c.confidence!=null && (
                      <span className={clsx('text-[11px] font-semibold', CONF_COLOR(c.confidence))}>
                        {Math.round(c.confidence*100)}% confidence
                      </span>
                    )}
                  </div>
                  {c.source_content && (
                    <p className="font-mono text-[10px] text-tx-4 mt-1 truncate">{c.source_content.slice(0,120)}</p>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── CONNECTORS TAB ───────────────────────────────────────
// Swarm queue depth + connector list + webhook test panel
// ═══════════════════════════════════════════════════════════
function ConnectorsTab({setError, flash}) {
  const [installed, setInstalled] = useState([]);
  const [swarmData, setSwarmData] = useState(null);
  const [loading,   setLoading]   = useState(true);
  // Webhook test panel
  const [testType,    setTestType]    = useState('github');
  const [testPayload, setTestPayload] = useState('');
  const [testing,     setTesting]     = useState(false);
  const [testResult,  setTestResult]  = useState(null);
  // API key install
  const [installType,   setInstallType]   = useState('');
  const [installKey,    setInstallKey]    = useState('');
  const [installSettings, setInstallSettings] = useState('{}');
  const [installing,    setInstalling]    = useState(false);
  const [webhookResult, setWebhookResult] = useState(null);
  // Webhook-only connector install
  const [webhookInstallType, setWebhookInstallType] = useState('');
  const [webhookInstalling,  setWebhookInstalling]  = useState(false);
  const [webhookInstallResult, setWebhookInstallResult] = useState(null); // { webhook_url, webhook_secret }
  const [copied, setCopied] = useState('');

  const CONNECTOR_META = {
    // ── OAuth connectors ─────────────────────────────────────────────────
    // Connect once → token stored → agents use MCP + poller checks for new events
    slack:       { label:'Slack',             auth:'oauth',   poll:true,  desc:'Messages, channels, DMs',              color:'bg-ok-soft text-ok',         mcp:true  },
    google:      { label:'Google',            auth:'oauth',   poll:true,  desc:'Gmail, Drive, Sheets, Docs, Calendar', color:'bg-info-soft text-info',     mcp:true  },
    microsoft:   { label:'Microsoft',         auth:'oauth',   poll:true,  desc:'Outlook, Teams, OneDrive',             color:'bg-info-soft text-info',     mcp:true  },
    salesforce:  { label:'Salesforce',        auth:'oauth',   poll:true,  desc:'CRM — leads, opportunities',           color:'bg-info-soft text-info',     mcp:true  },
    hubspot:     { label:'HubSpot',           auth:'oauth',   poll:true,  desc:'Contacts, deals, marketing',           color:'bg-accent-soft text-accent', mcp:true  },
    atlassian:   { label:'Jira / Confluence', auth:'oauth',   poll:true,  desc:'Issues, sprints, wiki pages',          color:'bg-info-soft text-info',     mcp:true  },
    notion:      { label:'Notion',            auth:'oauth',   poll:false, desc:'Pages, databases, search',             color:'bg-vio-soft text-vio',       mcp:true  },
    quickbooks:  { label:'QuickBooks',        auth:'oauth',   poll:true,  desc:'Invoices, expenses, month-end close',  color:'bg-ok-soft text-ok',         mcp:false },
    docusign:    { label:'DocuSign',          auth:'oauth',   poll:true,  desc:'Contracts, envelopes, signing',        color:'bg-info-soft text-info',     mcp:false },
    // ── API key connectors ───────────────────────────────────────────────
    // Paste API key → token stored → agents use MCP/direct API + poller polls
    // Webhook URL available as bonus for instant push (optional)
    github:      { label:'GitHub',    auth:'api_key', poll:true,  desc:'Repos, PRs, issues, CI',              color:'bg-ok-soft text-ok',         mcp:true,  webhook:true  },
    linear:      { label:'Linear',    auth:'api_key', poll:true,  desc:'Issues, projects, cycles',             color:'bg-vio-soft text-vio',       mcp:true,  webhook:false },
    zendesk:     { label:'Zendesk',   auth:'api_key', poll:true,  desc:'Support tickets — needs subdomain setting', color:'bg-info-soft text-info', mcp:false, webhook:true  },
    servicenow:  { label:'ServiceNow',auth:'api_key', poll:true,  desc:'ITSM incidents — needs instance_url setting', color:'bg-vio-soft text-vio',  mcp:false, webhook:true  },
    pagerduty:   { label:'PagerDuty', auth:'api_key', poll:true,  desc:'Incidents, on-call, postmortems',      color:'bg-err-soft text-err',       mcp:false, webhook:true  },
    greenhouse:  { label:'Greenhouse',auth:'api_key', poll:true,  desc:'Applications, interviews, offers',     color:'bg-info-soft text-info',     mcp:false, webhook:true  },
    dbt_cloud:   { label:'dbt Cloud', auth:'api_key', poll:true,  desc:'Pipeline failures — needs account_id', color:'bg-warn-soft text-warn',     mcp:false, webhook:true  },
  };

  // Settings placeholders shown in the API key form for connectors that need extra config
  const SETTINGS_PLACEHOLDER = {
    zendesk:    '{"subdomain":"yourcompany"}',
    servicenow: '{"instance_url":"https://yourinstance.service-now.com"}',
    dbt_cloud:  '{"account_id":"123456"}',
  };

  const SAMPLE_PAYLOADS = {
    github:    JSON.stringify({action:'opened',pull_request:{title:'Fix auth bug',number:42}}, null, 2),
    zendesk:   JSON.stringify({type:'ticket.created',ticket:{id:1001,subject:'Login broken'}}, null, 2),
    pagerduty: JSON.stringify({messages:[{event:'incident.trigger',incident:{title:'DB spike'}}]}, null, 2),
    salesforce:JSON.stringify({event:'OpportunityCreated',opportunity:{name:'Acme Corp',amount:50000}}, null, 2),
    hubspot:   JSON.stringify({subscriptionType:'deal.creation',objectId:12345}, null, 2),
    notion:    JSON.stringify({type:'page_created',page:{id:'page-xyz',title:'Q4 Research'}}, null, 2),
    greenhouse:JSON.stringify({action:'application_created',payload:{application:{id:9999}}}, null, 2),
    dbt_cloud: JSON.stringify({eventType:'job.errored',data:{jobId:55,runId:88}}, null, 2),
    servicenow:JSON.stringify({type:'incident.created',incident:{number:'INC001',short_description:'DB down'}}, null, 2),
  };

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [inst, sw] = await Promise.all([
        connectors.list().catch(() => ({ connectors: [] })),
        swarm.status().catch(() => null),
      ]);
      setInstalled(inst.connectors || []);
      setSwarmData(sw);
    } catch(e) { setError(e.message); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => { load(); }, []);
  useEffect(() => {
    setTestPayload(SAMPLE_PAYLOADS[testType] || '{}');
  }, [testType]);

  function connectOAuth(provider) {
    // Redirect to backend OAuth start — backend stores CSRF state, then redirects to provider
    const url = connectors.oauthStartUrl(provider);
    window.location.href = url;
  }

  async function installApiKey(e) {
    e.preventDefault();
    if (!installType || !installKey.trim()) return;
    setInstalling(true);
    try {
      let settings = {};
      try { settings = JSON.parse(installSettings || '{}'); } catch {}
      await connectors.installApiKey(installType, installKey.trim(), settings);
      setInstallKey(''); setInstallType(''); setInstallSettings('{}');
      await load();
      flash(`${installType} connected.`);
    } catch(e) { setError(e.message); }
    finally { setInstalling(false); }
  }

  async function installWebhookConnector(type) {
    setWebhookInstalling(true); setWebhookInstallResult(null);
    try {
      const res = await connectors.installWebhook(type);
      setWebhookInstallResult({ type, ...res });
      await load();
    } catch(e) { setError(e.message); }
    finally { setWebhookInstalling(false); }
  }

  function copyText(text, key) {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(key);
      setTimeout(() => setCopied(''), 2000);
    });
  }

  async function uninstall(type) {
    if (!window.confirm(`Disconnect ${type}? Stored tokens will be deleted.`)) return;
    try {
      await connectors.uninstall(type);
      await load();
      flash(`${type} disconnected.`);
    } catch(e) { setError(e.message); }
  }

  async function testWebhook(e) {
    e.preventDefault();
    setTesting(true); setTestResult(null);
    try {
      const payload = JSON.parse(testPayload);
      await connectors.testWebhook(testType, payload);
      setTestResult({ ok: true, msg: `Webhook delivered to ${testType} — agent goal created.` });
      flash(`Test webhook sent to ${testType}.`);
    } catch(e) { setTestResult({ ok: false, msg: e.message }); }
    finally { setTesting(false); }
  }

  const installedMap = Object.fromEntries(installed.map(i => [i.connector_type, i]));

  if (loading) return <Spinner/>;

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-serif text-xl text-tx-1 mb-1">Connectors</h2>
        <p className="text-sm text-tx-3 leading-relaxed">
          Connect external services so agents can use them as tools and automatically trigger on new events.
          OAuth connectors use one-click login. API key connectors need a token.
          Once connected, the agent scheduler polls each service on a schedule —
          no webhook configuration needed. Webhook URLs are available as an optional bonus for instant push triggers.
        </p>
      </div>

      {/* Swarm stat */}
      {swarmData && (
        <div className="grid grid-cols-3 gap-3">
          {[
            { label:'Queue depth',  value: swarmData.queue_depth ?? '—', icon:GitBranch, note:'Tasks waiting' },
            { label:'Workers',      value: swarmData.pool_size   ?? '—', icon:Zap,       note:'Pool size' },
            { label:'Backend',      value: swarmData.queue_backed ? 'Redis' : 'Memory', icon:Database, note:'Queue driver' },
          ].map(item => { const Icon = item.icon; return (
            <div key={item.label} className="rounded-xl border border-border bg-bg-card p-4 shadow-sm">
              <div className="flex items-center gap-2 mb-2"><Icon size={13} className="text-tx-3"/><span className="text-xs text-tx-3">{item.label}</span></div>
              <p className="font-serif text-2xl text-tx-1">{item.value}</p>
              <p className="text-[11px] text-tx-4 mt-1">{item.note}</p>
            </div>
          );})}
        </div>
      )}

      {/* Connector grid */}
      <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
        <div className="px-4 py-3 border-b border-border flex items-center gap-2">
          <Plug size={13} className="text-tx-3"/>
          <span className="text-sm font-medium text-tx-1">All connectors</span>
          <span className="ml-auto text-xs text-tx-4 font-mono">{installed.length} connected</span>
        </div>
        {Object.entries(CONNECTOR_META).map(([id, meta]) => {
          const install = installedMap[id];
          const isConnected = !!install?.connected;
          return (
            <div key={id} className="flex items-center gap-3 px-4 py-3.5 border-b border-border/60 last:border-0 hover:bg-bg-hover">
              <div className={clsx('size-8 rounded-lg flex items-center justify-center shrink-0 text-[11px] font-bold uppercase', meta.color)}>
                {id.slice(0,2)}
              </div>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-tx-1">{meta.label}</span>
                  {meta.mcp  && <span className="text-[9px] font-bold uppercase tracking-wide text-vio bg-vio-soft px-1.5 py-0.5 rounded">MCP</span>}
                  {meta.poll && <span className="text-[9px] font-bold uppercase tracking-wide text-info bg-info-soft px-1.5 py-0.5 rounded">Polls</span>}
                  {meta.webhook && isConnected && <span className="text-[9px] font-bold uppercase tracking-wide text-tx-4 bg-bg-active px-1.5 py-0.5 rounded">+ Webhook</span>}
                </div>
                <p className="text-xs text-tx-3">{meta.desc}</p>
                {isConnected && install?.last_polled_at && (
                  <p className="text-[10px] text-tx-4 mt-0.5">
                    Last polled {new Date(install.last_polled_at).toLocaleTimeString()}
                  </p>
                )}
              </div>
              <div className="flex items-center gap-2 shrink-0">
                <span className={clsx('inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded border',
                  isConnected ? 'bg-ok-soft text-ok border-ok/25' : 'bg-bg-active text-tx-4 border-border')}>
                  <span className={clsx('size-1.5 rounded-full', isConnected ? 'bg-ok' : 'bg-tx-4')}/>
                  {isConnected ? 'connected' : 'not connected'}
                </span>
                {isConnected ? (
                  <div className="flex items-center gap-2 shrink-0">
                    {/* Secondary webhook URL button for connectors that support it */}
                    {meta.webhook && (
                      <button
                        onClick={() => installWebhookConnector(id)}
                        disabled={webhookInstalling}
                        title="Get webhook URL for instant push triggers (optional)"
                        className="text-[11px] text-tx-4 hover:text-accent transition-colors px-2 py-1 rounded border border-border hover:border-accent/40">
                        {webhookInstalling ? <Loader2 size={10} className="animate-spin inline"/> : 'Webhook URL'}
                      </button>
                    )}
                    <button onClick={() => uninstall(id)}
                      className="text-[11px] text-err hover:underline transition-colors">Disconnect</button>
                  </div>
                ) : meta.auth === 'oauth' ? (
                  <button onClick={() => connectOAuth(id)}
                    className="flex items-center gap-1 rounded-lg border border-border bg-bg px-3 py-1.5 text-[12px] font-medium text-tx-2 hover:border-accent/40 hover:text-accent transition-all">
                    <ArrowRight size={11}/> Connect
                  </button>
                ) : (
                  <button onClick={() => setInstallType(id)}
                    className="flex items-center gap-1 rounded-lg border border-border bg-bg px-3 py-1.5 text-[12px] font-medium text-tx-2 hover:border-accent/40 hover:text-accent transition-all">
                    <Key size={11}/> Add key
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {/* Webhook install result — shown after clicking "Get URL" */}
      {webhookInstallResult && (
        <div className="rounded-xl border border-ok/25 bg-ok-soft p-5 shadow-sm animate-in space-y-3">
          <div className="flex items-center gap-2">
            <CheckCircle2 size={14} className="text-ok shrink-0"/>
            <span className="text-sm font-medium text-ok">{CONNECTOR_META[webhookInstallResult.type]?.label} webhook configured</span>
            <button onClick={() => setWebhookInstallResult(null)} className="ml-auto text-ok/60 hover:text-ok"><X size={13}/></button>
          </div>
          <p className="text-xs text-tx-3 leading-relaxed">
            Paste these values into the <strong className="text-tx-2">{CONNECTOR_META[webhookInstallResult.type]?.label}</strong> webhook settings.
          </p>
          {[
            { label: 'Webhook URL', value: webhookInstallResult.webhook_url, key: 'url' },
            { label: 'Webhook Secret', value: webhookInstallResult.webhook_secret, key: 'secret' },
          ].map(item => (
            <div key={item.key}>
              <p className="text-xs font-medium text-tx-3 mb-1">{item.label}</p>
              <div className="flex items-center gap-2 rounded-lg border border-border bg-bg px-3 py-2">
                <code className="text-[12px] font-mono text-tx-1 flex-1 truncate">{item.value}</code>
                <button onClick={() => copyText(item.value, item.key)}
                  className="text-tx-4 hover:text-accent transition-colors shrink-0">
                  {copied === item.key ? <CheckCheck size={13} className="text-ok"/> : <Copy size={13}/>}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* API key install form */}
      {installType && (
        <div className="rounded-xl border border-border bg-bg-card p-5 shadow-sm animate-in">
          <h3 className="text-sm font-medium text-tx-1 mb-4 flex items-center gap-2">
            <Key size={13} className="text-accent"/> Connect {CONNECTOR_META[installType]?.label || installType}
          </h3>
          <form onSubmit={installApiKey} className="space-y-4">
            <div>
              <label className="block text-xs font-medium text-tx-3 mb-1.5">API Key / Token</label>
              <input value={installKey} onChange={e => setInstallKey(e.target.value)} type="password"
                placeholder="Paste your API key or personal access token"
                className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 font-mono placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all"/>
            </div>
            {['servicenow','zendesk','dbt_cloud'].includes(installType) && (
              <div>
                <label className="block text-xs font-medium text-tx-3 mb-1.5">Settings (JSON)</label>
                <textarea value={installSettings} onChange={e => setInstallSettings(e.target.value)}
                  placeholder={SETTINGS_PLACEHOLDER[installType] || '{}'}
                  rows={2} className="w-full rounded-lg border border-border bg-bg px-3 py-2 text-sm text-tx-1 font-mono placeholder-tx-4 outline-none focus:border-border-md resize-none transition-all"/>
              </div>
            )}
            <div className="flex items-center gap-3">
              <button type="submit" disabled={installing || !installKey.trim()}
                className="flex items-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all disabled:opacity-50">
                {installing ? <Loader2 size={14} className="animate-spin"/> : <CheckCircle2 size={14}/>}
                {installing ? 'Connecting…' : 'Connect'}
              </button>
              <button type="button" onClick={() => { setInstallType(''); setInstallKey(''); }}
                className="text-sm text-tx-3 hover:text-tx-1 transition-colors">Cancel</button>
            </div>
          </form>
        </div>
      )}

      {/* Webhook test panel */}
      <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
        <div className="px-4 py-3 border-b border-border flex items-center gap-2">
          <Zap size={13} className="text-accent"/>
          <span className="text-sm font-medium text-tx-1">Test inbound webhook</span>
          <span className="ml-auto text-xs text-tx-4">Fire a sample payload to trigger an agent</span>
        </div>
        <form onSubmit={testWebhook} className="p-4 space-y-4">
          <div>
            <label className="block text-xs font-medium text-tx-3 mb-1.5">Connector</label>
            <select value={testType} onChange={e => setTestType(e.target.value)}
              className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 outline-none focus:border-border-md transition-all">
              {Object.entries(CONNECTOR_META).filter(([,m]) => m.auth !== 'oauth' || ['salesforce','hubspot','notion'].includes('xxx')).map(([id,meta]) => (
                <option key={id} value={id}>{meta.label}</option>
              ))}
            </select>
          </div>
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <label className="text-xs font-medium text-tx-3">Payload (JSON)</label>
              <button type="button" onClick={() => setTestPayload(SAMPLE_PAYLOADS[testType] || '{}')}
                className="text-[11px] text-accent hover:underline">Reset to sample</button>
            </div>
            <textarea value={testPayload} onChange={e => setTestPayload(e.target.value)}
              rows={5} spellCheck={false}
              className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-[12px] text-tx-1 font-mono outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 resize-none transition-all"/>
          </div>
          {testResult && (
            <div className={clsx('flex items-start gap-2 rounded-lg px-3 py-2.5 text-sm',
              testResult.ok ? 'bg-ok-soft border border-ok/25 text-ok' : 'bg-err-soft border border-err/25 text-err')}>
              {testResult.ok ? <CheckCircle2 size={14} className="mt-0.5 shrink-0"/> : <AlertCircle size={14} className="mt-0.5 shrink-0"/>}
              {testResult.msg}
            </div>
          )}
          <button type="submit" disabled={testing || !testPayload.trim()}
            className="flex items-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all disabled:opacity-50">
            {testing ? <Loader2 size={14} className="animate-spin"/> : <Zap size={14}/>}
            {testing ? 'Sending…' : `Send test to ${testType}`}
          </button>
        </form>
      </div>
    </div>
  );
}
// ═══════════════════════════════════════════════════════════
// ── AUTO-APPROVALS TAB ───────────────────────────────────
// Manage saved "don't ask again" rules for policy reviews
// ═══════════════════════════════════════════════════════════
function AutoApprovalsTab({setError, flash}) {
  const [rules,    setRules]    = useState([]);
  const [loading,  setLoading]  = useState(true);
  const [deleting, setDeleting] = useState({});
  // Manual add form
  const [showForm, setShowForm] = useState(false);
  const [newRule,  setNewRule]  = useState('');
  const [newNote,  setNewNote]  = useState('');
  const [adding,   setAdding]   = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try { const r = await autoApprovals.list(); setRules(r.rules || []); }
    catch(e) { setError(e.message); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => { load(); }, []);

  async function del(rule_id) {
    setDeleting(p => ({ ...p, [rule_id]: true }));
    try {
      await autoApprovals.delete(rule_id);
      setRules(p => p.filter(r => r.rule_id !== rule_id));
      flash(`Auto-approval for "${rule_id}" removed.`);
    } catch(e) { setError(e.message); }
    finally { setDeleting(p => ({ ...p, [rule_id]: false })); }
  }

  async function add(e) {
    e.preventDefault();
    if (!newRule.trim()) return;
    setAdding(true);
    try {
      await autoApprovals.create(newRule.trim(), newNote.trim() || 'Manually added');
      setNewRule(''); setNewNote('');
      setShowForm(false);
      await load();
      flash('Auto-approval rule saved.');
    } catch(e) { setError(e.message); }
    finally { setAdding(false); }
  }

  if (loading) return <Spinner/>;

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <h2 className="font-serif text-xl text-tx-1 mb-1">Auto-approvals</h2>
          <p className="text-sm text-tx-3">
            Policy rules that are automatically approved without entering the review queue.
            Rules are saved when you click "Auto-approve" on a review item — or you can add them manually here.
          </p>
        </div>
        <button onClick={() => setShowForm(s => !s)}
          className="flex items-center gap-2 rounded-lg border border-border bg-bg-card px-3.5 py-2 text-sm text-tx-2 hover:text-tx-1 hover:border-border-md transition-all shadow-sm shrink-0">
          <Plus size={13}/> Add rule
        </button>
      </div>

      {/* Manual add form */}
      {showForm && (
        <div className="rounded-xl border border-border bg-bg-card p-5 shadow-sm animate-in">
          <h3 className="text-sm font-medium text-tx-1 mb-4">New auto-approval rule</h3>
          <form onSubmit={add} className="space-y-4">
            <div>
              <label className="block text-xs font-medium text-tx-3 mb-1.5">Rule ID</label>
              <input value={newRule} onChange={e => setNewRule(e.target.value)}
                placeholder="e.g. web_search_external or file_write_workspace"
                required
                className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all font-mono"/>
              <p className="text-[11px] text-tx-4 mt-1">
                Must match the <code className="font-mono">rule_id</code> field emitted by your plane_guard policy.
              </p>
            </div>
            <div>
              <label className="block text-xs font-medium text-tx-3 mb-1.5">Note (optional)</label>
              <input value={newNote} onChange={e => setNewNote(e.target.value)}
                placeholder="Why this rule is always safe to approve…"
                className="w-full rounded-lg border border-border bg-bg px-3 py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none focus:border-border-md focus:ring-2 focus:ring-accent/10 transition-all"/>
            </div>
            <div className="flex items-center gap-3">
              <button type="submit" disabled={adding || !newRule.trim()}
                className="flex items-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all disabled:opacity-50 active:scale-[0.98]">
                {adding ? <Loader2 size={14} className="animate-spin"/> : <Save size={14}/>}
                {adding ? 'Saving…' : 'Save rule'}
              </button>
              <button type="button" onClick={() => setShowForm(false)}
                className="text-sm text-tx-3 hover:text-tx-1 transition-colors">Cancel</button>
            </div>
          </form>
        </div>
      )}

      {/* Rules list */}
      {rules.length === 0 ? (
        <div className="rounded-xl border border-border bg-bg-card py-16 text-center shadow-sm">
          <Shield size={24} className="text-tx-4 mx-auto mb-3"/>
          <p className="text-sm text-tx-2 font-medium">No auto-approval rules</p>
          <p className="text-xs text-tx-4 mt-1 max-w-xs mx-auto leading-relaxed">
            When you auto-approve a review, the rule is saved here so it won't enter the queue again.
          </p>
        </div>
      ) : (
        <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
          <div className="px-4 py-3 border-b border-border flex items-center gap-2">
            <Zap size={13} className="text-ok"/>
            <span className="text-sm font-medium text-tx-1">Saved rules ({rules.length})</span>
            <span className="ml-auto text-xs text-tx-4">These rule IDs skip the review queue automatically</span>
          </div>
          <div className="divide-y divide-border/60">
            {rules.map(rule => (
              <div key={rule.rule_id} className="flex items-start gap-4 px-4 py-3.5 hover:bg-bg-hover group">
                <div className="size-7 rounded-md bg-ok-soft flex items-center justify-center shrink-0 mt-0.5">
                  <Zap size={12} className="text-ok"/>
                </div>
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium text-tx-1 font-mono">{rule.rule_id}</p>
                  {rule.notes && (
                    <p className="text-xs text-tx-3 mt-0.5 truncate">"{rule.notes}"</p>
                  )}
                  {rule.created_at && (
                    <p className="text-[11px] text-tx-4 mt-0.5">
                      Added {new Date(rule.created_at).toLocaleDateString()}
                    </p>
                  )}
                </div>
                <button
                  onClick={() => del(rule.rule_id)}
                  disabled={deleting[rule.rule_id]}
                  className="p-1.5 rounded-lg text-tx-4 hover:text-err hover:bg-err-soft transition-all opacity-0 group-hover:opacity-100 shrink-0"
                  title="Remove rule">
                  {deleting[rule.rule_id]
                    ? <Loader2 size={13} className="animate-spin"/>
                    : <Trash2 size={13}/>}
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Info box */}
      <div className="rounded-xl border border-border bg-bg-card p-4 shadow-sm">
        <p className="text-xs font-medium text-tx-3 mb-1.5">How auto-approvals work</p>
        <div className="space-y-1.5 text-sm text-tx-2 leading-relaxed">
          <p>When an agent action triggers a plane_guard policy rule, it normally enters the review queue and the agent pauses. If the rule ID is in this list, the action is approved automatically without pausing.</p>
          <p className="text-xs text-tx-4 mt-2">
            Rule IDs come from your plane_guard configuration. Common values:
            <code className="font-mono ml-1 text-accent">web_search_external</code>,{' '}
            <code className="font-mono text-accent">file_write_workspace</code>,{' '}
            <code className="font-mono text-accent">shell_exec_sandbox</code>
          </p>
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════
// ── BILLING TAB ──────────────────────────────────────────
// Plan info, upgrade, invoices, credit top-ups
// ═══════════════════════════════════════════════════════════
function BillingTab({setError, flash}) {
  const [sub,        setSub]        = useState(null);
  const [invoices,   setInvoices]   = useState([]);
  const [credits,    setCredits]    = useState(null);
  const [loading,    setLoading]    = useState(true);
  const [checkingOut, setCheckingOut] = useState('');
  const [cancelling,  setCancelling]  = useState(false);
  const [buyingCredits, setBuyingCredits] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const [s, inv, c] = await Promise.all([
          billing.subscription().catch(() => ({ plan: 'free', status: 'active' })),
          billing.invoices().catch(() => ({ invoices: [] })),
          billing.credits().catch(() => null),
        ]);
        setSub(s);
        setInvoices(inv.invoices || []);
        setCredits(c);
      } catch(e) { setError(e.message); }
      finally { setLoading(false); }
    })();
  }, []);

  const PLANS = [
    {
      id: 'free', label: 'Free', price: '$0', period: 'forever',
      steps: '1,000', agents: '3',
      color: 'border-border', badge: 'bg-bg-active text-tx-3',
    },
    {
      id: 'go', label: 'Go', price: '$15', period: '/month',
      steps: '20,000', agents: '20',
      color: 'border-accent/40', badge: 'bg-accent-soft text-accent',
      highlight: true,
    },
    {
      id: 'pro', label: 'Pro', price: '$79', period: '/month',
      steps: '150,000', agents: '200',
      color: 'border-vio/40', badge: 'bg-vio-soft text-vio',
    },
    {
      id: 'enterprise', label: 'Enterprise', price: 'Custom', period: '',
      steps: 'Unlimited', agents: 'Unlimited',
      color: 'border-border', badge: 'bg-bg-active text-tx-3',
    },
  ];

  async function checkout(planId) {
    if (planId === 'enterprise') {
      window.open('mailto:sales@narayan.ai?subject=Enterprise%20plan', '_blank');
      return;
    }
    setCheckingOut(planId);
    try {
      const res = await billing.checkout(planId, 'paypal');
      if (res.redirect_url) {
        window.location.href = res.redirect_url;
      }
    } catch(e) { setError(e.message); }
    finally { setCheckingOut(''); }
  }

  async function cancelSub() {
    if (!window.confirm('Cancel your subscription? You will keep access until the end of the billing period.')) return;
    setCancelling(true);
    try {
      await billing.cancelSubscription();
      flash('Subscription cancelled — access continues until period end.');
      setSub(s => ({ ...s, status: 'cancelled' }));
    } catch(e) { setError(e.message); }
    finally { setCancelling(false); }
  }

  async function buyCredits() {
    setBuyingCredits(true);
    try {
      const res = await billing.purchaseCredits();
      if (res.redirect_url) window.location.href = res.redirect_url;
    } catch(e) { setError(e.message); }
    finally { setBuyingCredits(false); }
  }

  if (loading) return <Spinner/>;

  const currentPlanId = sub?.plan || 'free';
  const isActive = sub?.status === 'active';

  return (
    <div className="space-y-8">
      <div>
        <h2 className="font-serif text-xl text-tx-1 mb-1">Billing</h2>
        <p className="text-sm text-tx-3">
          Plans are step-based — you bring your own LLM keys, we charge for platform execution.
          Every plan gets all connectors and the full compliance stack.
        </p>
      </div>

      {/* Current plan banner */}
      {sub && (
        <div className="rounded-xl border border-border bg-bg-card p-4 shadow-sm flex items-center gap-4">
          <div className="flex-1">
            <div className="flex items-center gap-2 mb-0.5">
              <span className="text-sm font-semibold text-tx-1 capitalize">{currentPlanId} plan</span>
              <span className={clsx('text-[10px] font-bold px-2 py-0.5 rounded uppercase tracking-wide',
                isActive ? 'bg-ok-soft text-ok' : 'bg-warn-soft text-warn')}>
                {sub.status || 'active'}
              </span>
            </div>
            {sub.current_period_end && (
              <p className="text-xs text-tx-4">
                {sub.status === 'cancelled' ? 'Access until' : 'Renews'}{' '}
                {new Date(sub.current_period_end).toLocaleDateString()}
              </p>
            )}
          </div>
          {credits && credits.extra_steps > 0 && (
            <div className="text-right">
              <p className="text-sm font-semibold text-accent">{credits.extra_steps.toLocaleString()}</p>
              <p className="text-xs text-tx-4">bonus steps</p>
            </div>
          )}
          {currentPlanId !== 'free' && sub.status !== 'cancelled' && (
            <button onClick={cancelSub} disabled={cancelling}
              className="text-[11px] text-tx-4 hover:text-err transition-colors shrink-0">
              {cancelling ? <Loader2 size={11} className="animate-spin inline"/> : 'Cancel'}
            </button>
          )}
        </div>
      )}

      {/* Plan cards */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {PLANS.map(plan => {
          const isCurrent = plan.id === currentPlanId;
          return (
            <div key={plan.id}
              className={clsx('rounded-xl border p-4 shadow-sm flex flex-col',
                plan.highlight ? 'bg-accent-soft/20' : 'bg-bg-card',
                plan.color)}>
              <div className="flex items-center justify-between mb-3">
                <span className="text-sm font-semibold text-tx-1">{plan.label}</span>
                {isCurrent && (
                  <span className="text-[9px] font-bold px-1.5 py-0.5 rounded bg-ok-soft text-ok uppercase tracking-wide">Current</span>
                )}
              </div>
              <p className="font-serif text-2xl text-tx-1 mb-0.5">{plan.price}</p>
              <p className="text-xs text-tx-4 mb-4">{plan.period}</p>
              <div className="space-y-1 mb-4 flex-1">
                <div className="flex items-center gap-1.5 text-xs text-tx-3">
                  <CheckCircle2 size={10} className="text-ok shrink-0"/>
                  {plan.steps} steps/month
                </div>
                <div className="flex items-center gap-1.5 text-xs text-tx-3">
                  <CheckCircle2 size={10} className="text-ok shrink-0"/>
                  {plan.agents} concurrent agents
                </div>
                <div className="flex items-center gap-1.5 text-xs text-tx-3">
                  <CheckCircle2 size={10} className="text-ok shrink-0"/>
                  All 20 connectors
                </div>
                <div className="flex items-center gap-1.5 text-xs text-tx-3">
                  <CheckCircle2 size={10} className="text-ok shrink-0"/>
                  Full compliance stack
                </div>
              </div>
              {isCurrent ? (
                <div className="rounded-lg bg-bg-active px-3 py-2 text-center text-[11px] text-tx-4">Active plan</div>
              ) : plan.id === 'free' ? (
                <div className="rounded-lg bg-bg-active px-3 py-2 text-center text-[11px] text-tx-4">Downgrade</div>
              ) : (
                <button
                  onClick={() => checkout(plan.id)}
                  disabled={!!checkingOut}
                  className={clsx(
                    'rounded-lg px-3 py-2 text-[12px] font-semibold transition-all disabled:opacity-50',
                    plan.highlight
                      ? 'bg-accent text-bg-card hover:bg-accent/90'
                      : 'bg-tx-1 text-bg-card hover:bg-tx-2'
                  )}>
                  {checkingOut === plan.id
                    ? <Loader2 size={12} className="animate-spin inline"/>
                    : plan.id === 'enterprise' ? 'Contact sales' : `Upgrade to ${plan.label}`}
                </button>
              )}
            </div>
          );
        })}
      </div>

      {/* Credit top-up */}
      <div className="rounded-xl border border-border bg-bg-card p-5 shadow-sm">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h3 className="text-sm font-semibold text-tx-1 mb-1">Step credit top-up</h3>
            <p className="text-sm text-tx-3 leading-relaxed">
              Hit your monthly limit? Buy 5,000 extra steps for $8. Works on any paid plan and never expires.
            </p>
          </div>
          <button
            onClick={buyCredits}
            disabled={buyingCredits || currentPlanId === 'free'}
            title={currentPlanId === 'free' ? 'Upgrade to a paid plan first' : ''}
            className="shrink-0 flex items-center gap-2 rounded-lg bg-accent px-4 py-2.5 text-sm font-semibold text-bg-card hover:bg-accent/90 transition-all disabled:opacity-50">
            {buyingCredits ? <Loader2 size={13} className="animate-spin"/> : <CreditCard size={13}/>}
            Buy 5,000 steps — $8
          </button>
        </div>
        {credits && (
          <div className="mt-3 pt-3 border-t border-border flex items-center gap-2 text-xs text-tx-4">
            <Zap size={11} className="text-accent"/>
            You have <strong className="text-tx-1 mx-1">{credits.extra_steps.toLocaleString()}</strong> bonus steps remaining
          </div>
        )}
      </div>

      {/* Invoices */}
      <div className="rounded-xl border border-border bg-bg-card overflow-hidden shadow-sm">
        <div className="px-4 py-3 border-b border-border flex items-center gap-2">
          <DollarSign size={13} className="text-tx-3"/>
          <span className="text-sm font-medium text-tx-1">Invoices</span>
          <span className="ml-auto text-xs text-tx-4 font-mono">{invoices.length}</span>
        </div>
        {invoices.length === 0 ? (
          <div className="px-4 py-10 text-center">
            <p className="text-sm text-tx-3">No invoices yet.</p>
            <p className="text-xs text-tx-4 mt-1">Your first invoice will appear here after your first payment.</p>
          </div>
        ) : (
          <div className="divide-y divide-border/60">
            {invoices.map(inv => (
              <div key={inv.id} className="flex items-center gap-4 px-4 py-3 hover:bg-bg-hover">
                <div className="flex-1 min-w-0">
                  <p className="text-sm text-tx-1">${(inv.amount_usd || 0).toFixed(2)}</p>
                  <p className="text-xs text-tx-4">
                    {new Date(inv.period_start).toLocaleDateString()} –{' '}
                    {new Date(inv.period_end).toLocaleDateString()}
                  </p>
                </div>
                <span className={clsx('text-[10px] font-semibold px-2 py-0.5 rounded border capitalize',
                  inv.status === 'paid' ? 'bg-ok-soft text-ok border-ok/25' : 'bg-warn-soft text-warn border-warn/25')}>
                  {inv.status}
                </span>
                {inv.pdf_url && (
                  <a href={inv.pdf_url} target="_blank" rel="noopener noreferrer"
                    className="text-tx-4 hover:text-accent transition-colors">
                    <ExternalLink size={13}/>
                  </a>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      <p className="text-xs text-tx-4 text-center">
        Payments processed securely by PayPal. Questions?{' '}
        <a href="mailto:billing@narayan.ai" className="text-accent hover:underline">billing@narayan.ai</a>
      </p>
    </div>
  );
}
