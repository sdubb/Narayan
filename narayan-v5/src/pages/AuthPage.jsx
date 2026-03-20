import { useState } from 'react';
import { Eye, EyeOff, Loader2, ArrowRight, Copy, CheckCircle2, AlertCircle } from 'lucide-react';
import { auth } from '../api';

export default function AuthPage({ onAuth }) {
  const [mode, setMode]     = useState('login');
  const [form, setForm]     = useState({ name:'', email:'', api_key:'' });
  const [loading, setLoading] = useState(false);
  const [error, setError]   = useState('');
  const [showKey, setShowKey] = useState(false);
  const [newKey, setNewKey] = useState('');
  const [copied, setCopied] = useState(false);
  const set = k => e => setForm(p => ({ ...p, [k]: e.target.value }));

  async function handleRegister(e) {
    e.preventDefault(); setError(''); setLoading(true);
    try {
      const res = await auth.register(form.name, form.email);
      setNewKey(res.api_key);
      localStorage.setItem('narayan_api_key', res.api_key);
      localStorage.setItem('narayan_tenant_id', res.tenant_id);
    } catch (err) { setError(err.message); }
    finally { setLoading(false); }
  }

  async function handleLogin(e) {
    e.preventDefault(); setError(''); setLoading(true);
    try {
      const res = await auth.token(form.api_key);
      onAuth({ token: res.token, api_key: form.api_key });
    } catch (err) { setError(err.message); }
    finally { setLoading(false); }
  }

  async function copyKey() {
    await navigator.clipboard.writeText(newKey);
    setCopied(true); setTimeout(() => setCopied(false), 2000);
  }

  if (newKey) {
    return (
      <div className="min-h-screen bg-bg flex items-center justify-center p-6">
        <div className="w-full max-w-md animate-in">
          <div className="bg-bg-card rounded-xl shadow-card p-8 space-y-6">
            <div className="flex items-center gap-3">
              <div className="size-10 rounded-lg bg-ok-soft flex items-center justify-center">
                <CheckCircle2 size={18} className="text-ok" />
              </div>
              <div>
                <p className="font-medium text-tx-1">Account created</p>
                <p className="text-sm text-tx-3">Save your API key — it's shown once only</p>
              </div>
            </div>

            <div className="space-y-2">
              <p className="text-xs font-medium text-tx-3 uppercase tracking-wide">Your API Key</p>
              <div className="flex items-center gap-2 bg-bg rounded-lg border border-border p-3">
                <code className="flex-1 font-mono text-xs text-tx-2 break-all">
                  {showKey ? newKey : newKey.replace(/(?<=.{16})./g, '•')}
                </code>
                <button onClick={() => setShowKey(s=>!s)} className="text-tx-3 hover:text-tx-2 transition-colors p-1 shrink-0">
                  {showKey ? <EyeOff size={14}/> : <Eye size={14}/>}
                </button>
                <button onClick={copyKey}
                  className="flex items-center gap-1.5 rounded-md bg-accent-soft border border-accent/20 px-2.5 py-1.5 text-xs font-medium text-accent-text hover:bg-accent/20 transition-colors shrink-0">
                  {copied ? <CheckCircle2 size={12}/> : <Copy size={12}/>}
                  {copied ? 'Copied' : 'Copy'}
                </button>
              </div>
              <p className="text-xs text-err">This key cannot be recovered if lost.</p>
            </div>

            <button onClick={() => onAuth({ api_key: newKey })}
              className="w-full flex items-center justify-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 active:scale-[0.98] transition-all">
              I've saved my key — continue <ArrowRight size={15}/>
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-bg flex items-center justify-center p-6">
      <div className="w-full max-w-sm animate-in">

        {/* Logo */}
        <div className="text-center mb-8">
          <p className="font-serif text-4xl text-tx-1 mb-1">Narayan</p>
          <p className="text-sm text-tx-3">Autonomous AI Employee Platform</p>
        </div>

        {/* Tab toggle */}
        <div className="flex rounded-lg bg-bg-active p-1 mb-5">
          {['login','register'].map(m => (
            <button key={m} onClick={() => { setMode(m); setError(''); }}
              className={`flex-1 rounded-md py-1.5 text-sm font-medium transition-all ${
                mode===m ? 'bg-bg-card shadow-sm text-tx-1' : 'text-tx-3 hover:text-tx-2'
              }`}>
              {m === 'login' ? 'Sign in' : 'Register'}
            </button>
          ))}
        </div>

        {/* Form */}
        <form onSubmit={mode==='login' ? handleLogin : handleRegister}
          className="bg-bg-card rounded-xl shadow-card p-6 space-y-4">

          {mode === 'register' && (<>
            <Field label="Name" value={form.name} onChange={set('name')} placeholder="Acme Corp" required />
            <Field label="Email" type="email" value={form.email} onChange={set('email')} placeholder="admin@acme.com" required />
          </>)}

          {mode === 'login' && (
            <Field label="API Key" value={form.api_key} onChange={set('api_key')}
              placeholder="nar_abc123_…" type={showKey?'text':'password'} required
              suffix={
                <button type="button" onClick={()=>setShowKey(s=>!s)} className="text-tx-3 hover:text-tx-2 p-1 transition-colors">
                  {showKey ? <EyeOff size={14}/> : <Eye size={14}/>}
                </button>
              } />
          )}

          {error && (
            <div className="flex items-start gap-2 rounded-lg bg-err-soft border border-err/20 px-3 py-2.5">
              <AlertCircle size={14} className="text-err mt-0.5 shrink-0"/>
              <p className="text-xs text-err">{error}</p>
            </div>
          )}

          <button type="submit" disabled={loading}
            className="w-full flex items-center justify-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed">
            {loading ? <Loader2 size={15} className="animate-spin"/> : null}
            {loading ? 'Working…' : mode==='login' ? 'Sign in' : 'Create account'}
          </button>
        </form>

        <p className="text-center text-xs text-tx-4 mt-4">Your API key encrypts provider credentials at rest.</p>
      </div>
    </div>
  );
}

function Field({ label, suffix, ...props }) {
  return (
    <div>
      <label className="block text-xs font-medium text-tx-2 mb-1.5">{label}</label>
      <div className="flex items-center gap-1 rounded-lg border border-border bg-bg px-3 focus-within:border-border-md focus-within:ring-2 focus-within:ring-accent/10 transition-all">
        <input {...props} className="flex-1 bg-transparent py-2.5 text-sm text-tx-1 placeholder-tx-4 outline-none" />
        {suffix}
      </div>
    </div>
  );
}
