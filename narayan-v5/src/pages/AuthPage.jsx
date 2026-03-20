import { useState } from 'react';
import { Eye, EyeOff, Loader2, AlertCircle } from 'lucide-react';
import { auth } from '../api';

const REGISTER_DEFAULTS = { name: '', username: '', email: '', password: '', confirmPassword: '' };
const LOGIN_DEFAULTS = { identifier: '', password: '' };

export default function AuthPage({ onAuth }) {
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
    <div className="min-h-screen bg-bg flex items-center justify-center p-6">
      <div className="w-full max-w-sm animate-in">
        <div className="text-center mb-8">
          <p className="font-serif text-4xl text-tx-1 mb-1">Narayan</p>
          <p className="text-sm text-tx-3">Autonomous AI Employee Platform</p>
        </div>

        <div className="flex rounded-lg bg-bg-active p-1 mb-5">
          {['login', 'register'].map(tab => (
            <button
              key={tab}
              onClick={() => { setMode(tab); setError(''); }}
              className={`flex-1 rounded-md py-1.5 text-sm font-medium transition-all ${
                mode === tab ? 'bg-bg-card shadow-sm text-tx-1' : 'text-tx-3 hover:text-tx-2'
              }`}
            >
              {tab === 'login' ? 'Sign in' : 'Register'}
            </button>
          ))}
        </div>

        <form
          onSubmit={mode === 'login' ? handleLogin : handleRegister}
          className="bg-bg-card rounded-xl shadow-card p-6 space-y-4"
        >
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
            <div className="flex items-start gap-2 rounded-lg bg-err-soft border border-err/20 px-3 py-2.5">
              <AlertCircle size={14} className="text-err mt-0.5 shrink-0" />
              <p className="text-xs text-err">{error}</p>
            </div>
          )}

          <button
            type="submit"
            disabled={loading}
            className="w-full flex items-center justify-center gap-2 rounded-lg bg-tx-1 px-4 py-2.5 text-sm font-medium text-bg-card hover:bg-tx-2 transition-all active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {loading ? <Loader2 size={15} className="animate-spin" /> : null}
            {loading ? 'Working…' : mode === 'login' ? 'Sign in' : 'Create account'}
          </button>
        </form>

        <p className="text-center text-xs text-tx-4 mt-4">Sign in now uses a standard email or username plus password flow.</p>
      </div>
    </div>
  );
}

function PasswordToggle({ show, onClick }) {
  return (
    <button type="button" onClick={onClick} className="text-tx-3 hover:text-tx-2 p-1 transition-colors">
      {show ? <EyeOff size={14} /> : <Eye size={14} />}
    </button>
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
