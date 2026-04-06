import { useState } from 'react';
import { motion } from 'framer-motion';
import { Eye, EyeOff, Loader2, AlertCircle, ArrowRight, Bot, CheckCircle2, ShieldCheck, Sparkles } from 'lucide-react';
import { auth } from '../api';

const REGISTER_DEFAULTS = { name: '', username: '', email: '', password: '', confirmPassword: '' };
const LOGIN_DEFAULTS = { identifier: '', password: '' };

const benefits = [
  'Describe the job once and reuse it across roles.',
  'Validate credentials before any live action runs.',
  'Keep every decision attached to an auditable trail.',
];

export default function AuthPage({ onAuth, onBack }) {
  const [mode, setMode] = useState('login');
  const [registerForm, setRegisterForm] = useState(REGISTER_DEFAULTS);
  const [loginForm, setLoginForm] = useState(LOGIN_DEFAULTS);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [showPassword, setShowPassword] = useState(false);

  const setRegister = key => e => setRegisterForm(prev => ({ ...prev, [key]: e.target.value }));
  const setLogin = key => e => setLoginForm(prev => ({ ...prev, [key]: e.target.value }));

  async function handleRegister(e) {
    e.preventDefault();
    setError('');

    if (registerForm.password !== registerForm.confirmPassword) {
      setError('Passwords do not match.');
      return;
    }

    setLoading(true);
    try {
      const res = await auth.register(
        registerForm.name,
        registerForm.username,
        registerForm.email,
        registerForm.password,
      );
      onAuth({ token: res.token, tenant_id: res.tenant_id });
    } catch (err) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  async function handleLogin(e) {
    e.preventDefault();
    setError('');
    setLoading(true);
    try {
      const res = await auth.login(loginForm.identifier, loginForm.password);
      onAuth({ token: res.token, tenant_id: res.tenant_id });
    } catch (err) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="relative min-h-screen overflow-hidden bg-[radial-gradient(circle_at_top_left,_rgba(201,106,46,0.14),_transparent_28%),radial-gradient(circle_at_top_right,_rgba(59,130,246,0.12),_transparent_24%),linear-gradient(180deg,_#faf7f2_0%,_#f5efe7_100%)] px-6 py-6 lg:px-10">
      <div className="pointer-events-none absolute inset-0">
        <div className="absolute left-[-6rem] top-20 h-64 w-64 rounded-full bg-accent/10 blur-3xl" />
        <div className="absolute right-[-5rem] top-16 h-72 w-72 rounded-full bg-info/10 blur-3xl" />
      </div>

      <div className="relative mx-auto grid min-h-[calc(100vh-3rem)] max-w-6xl gap-8 lg:grid-cols-[0.95fr_1.05fr] lg:items-center">
        <motion.section
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.45, ease: 'easeOut' }}
          className="max-w-xl"
        >
          {onBack ? (
            <button onClick={onBack} className="mb-8 inline-flex items-center gap-2 text-sm font-medium text-tx-3 transition-colors hover:text-tx-1">
              <ArrowRight className="size-4 rotate-180" />
              Back to landing
            </button>
          ) : null}

          <div className="inline-flex items-center gap-2 rounded-full border border-border bg-bg-card/90 px-3 py-1.5 text-xs font-medium text-tx-2 shadow-card">
            <Sparkles className="size-3.5 text-accent" />
            Secure workspace access
          </div>

          <div className="mt-6 flex items-center gap-3">
            <div className="flex size-12 items-center justify-center rounded-2xl border border-border bg-bg-card shadow-card">
              <Bot className="size-6 text-accent" />
            </div>
            <div>
              <p className="font-serif text-4xl leading-none text-tx-1">Narayan</p>
              <p className="mt-1 text-sm text-tx-3">The agent studio for structured enterprise work</p>
            </div>
          </div>

          <h1 className="mt-8 max-w-lg font-serif text-4xl leading-[0.95] text-tx-1 sm:text-5xl">
            Sign in to the workspace where work becomes easy to follow.
          </h1>

          <p className="mt-5 max-w-lg text-base leading-7 text-tx-2">
            Keep agents, approvals, and connector access in one place. The same account runs the workflow, the checks,
            and the audit trail.
          </p>

          <div className="mt-8 space-y-4">
            {benefits.map(item => (
              <div key={item} className="flex items-start gap-3 rounded-2xl border border-border/80 bg-bg-card/70 px-4 py-3">
                <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-ok" />
                <p className="text-sm leading-6 text-tx-2">{item}</p>
              </div>
            ))}
          </div>

          <div className="mt-8 flex items-center gap-2 text-xs text-tx-4">
            <ShieldCheck className="size-3.5 text-accent" />
            Login is isolated to your tenant and session.
          </div>
        </motion.section>

        <motion.section
          initial={{ opacity: 0, y: 18 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, ease: 'easeOut', delay: 0.08 }}
          className="overflow-hidden rounded-[2rem] border border-border bg-bg-card/90 shadow-[0_25px_60px_rgba(26,23,20,0.08)]"
        >
          <div className="border-b border-border px-6 pt-6">
            <div className="inline-flex rounded-full bg-bg-active p-1">
              {['login', 'register'].map(tab => (
                <button
                  key={tab}
                  onClick={() => { setMode(tab); setError(''); }}
                  className={`rounded-full px-4 py-2 text-sm font-medium transition-all ${
                    mode === tab ? 'bg-bg-card text-tx-1 shadow-card' : 'text-tx-3 hover:text-tx-1'
                  }`}
                >
                  {tab === 'login' ? 'Sign in' : 'Register'}
                </button>
              ))}
            </div>
          </div>

          <form onSubmit={mode === 'login' ? handleLogin : handleRegister} className="space-y-5 p-6 sm:p-8">
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.24em] text-accent">
                {mode === 'login' ? 'Welcome back' : 'Create your tenant'}
              </p>
              <h2 className="mt-2 font-serif text-2xl text-tx-1">
                {mode === 'login' ? 'Open your workspace' : 'Start with a new workspace'}
              </h2>
            </div>

            {mode === 'register' ? (
              <>
                <Field label="Name" value={registerForm.name} onChange={setRegister('name')} placeholder="Acme Corp" required />
                <Field label="Username" value={registerForm.username} onChange={setRegister('username')} placeholder="acme-admin" required />
                <Field label="Email" type="email" value={registerForm.email} onChange={setRegister('email')} placeholder="admin@acme.com" required />
                <Field
                  label="Password"
                  value={registerForm.password}
                  onChange={setRegister('password')}
                  placeholder="At least 8 characters"
                  type={showPassword ? 'text' : 'password'}
                  required
                  suffix={<PasswordToggle show={showPassword} onClick={() => setShowPassword(s => !s)} />}
                />
                <Field
                  label="Confirm Password"
                  value={registerForm.confirmPassword}
                  onChange={setRegister('confirmPassword')}
                  placeholder="Repeat your password"
                  type={showPassword ? 'text' : 'password'}
                  required
                />
              </>
            ) : (
              <>
                <Field
                  label="Username or Email"
                  value={loginForm.identifier}
                  onChange={setLogin('identifier')}
                  placeholder="acme-admin or admin@acme.com"
                  required
                />
                <Field
                  label="Password"
                  value={loginForm.password}
                  onChange={setLogin('password')}
                  placeholder="Your password"
                  type={showPassword ? 'text' : 'password'}
                  required
                  suffix={<PasswordToggle show={showPassword} onClick={() => setShowPassword(s => !s)} />}
                />
              </>
            )}

            {error && (
              <div className="flex items-start gap-2 rounded-2xl border border-err/20 bg-err-soft px-4 py-3">
                <AlertCircle size={14} className="mt-0.5 shrink-0 text-err" />
                <p className="text-xs text-err">{error}</p>
              </div>
            )}

            <button
              type="submit"
              disabled={loading}
              className="btn-primary flex w-full items-center justify-center gap-2 px-4 py-3 text-sm disabled:cursor-not-allowed disabled:opacity-50"
            >
              {loading ? <Loader2 size={15} className="animate-spin" /> : null}
              {loading ? 'Working...' : mode === 'login' ? 'Sign in' : 'Create account'}
            </button>

            <p className="text-center text-xs text-tx-4">
              {mode === 'login'
                ? 'Use your username or email plus password to enter the workspace.'
                : 'Create a tenant, connect credentials, and start your first workflow.'}
            </p>
          </form>
        </motion.section>
      </div>
    </div>
  );
}

function PasswordToggle({ show, onClick }) {
  return (
    <button type="button" onClick={onClick} className="rounded-md p-1 text-tx-3 transition-colors hover:text-tx-2">
      {show ? <EyeOff size={14} /> : <Eye size={14} />}
    </button>
  );
}

function Field({ label, suffix, ...props }) {
  return (
    <div>
      <label className="mb-1.5 block text-xs font-medium text-tx-2">{label}</label>
      <div className="flex items-center gap-1 rounded-xl border border-border bg-bg px-3 transition-all focus-within:border-border-md focus-within:ring-2 focus-within:ring-accent/10">
        <input {...props} className="flex-1 bg-transparent py-3 text-sm text-tx-1 placeholder:text-tx-4 outline-none" />
        {suffix}
      </div>
    </div>
  );
}
