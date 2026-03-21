import { useState, useEffect } from 'react';
import AuthPage     from './pages/AuthPage';
import ChatPage     from './pages/ChatPage';
import SettingsPage from './pages/SettingsPage';
import { health }   from './api';

export default function App() {
  const [page, setPage]       = useState('loading');
  const [planError, setPlanError] = useState(''); // step-limit 402 errors surfaced globally

  useEffect(() => {
    const token = localStorage.getItem('narayan_token');
    if (!token) { setPage('auth'); return; }
    health.check()
      .then(() => setPage('chat'))
      .catch(() => setPage('chat'));
  }, []);

  // Listen for 401 events emitted by the API client when JWT expires
  useEffect(() => {
    function handleUnauth(event) {
      const currentToken = localStorage.getItem('narayan_token');
      const sessionStartedAt = Number(localStorage.getItem('narayan_session_started_at') || '0');
      const detail = event?.detail || {};
      if (detail.token && currentToken && detail.token !== currentToken) {
        return;
      }
      if (detail.at && sessionStartedAt && detail.at < sessionStartedAt) {
        return;
      }
      localStorage.removeItem('narayan_token');
      localStorage.removeItem('narayan_tenant_id');
      localStorage.removeItem('narayan_session_started_at');
      setPage('auth');
    }
    window.addEventListener('narayan:unauthenticated', handleUnauth);
    return () => window.removeEventListener('narayan:unauthenticated', handleUnauth);
  }, []);

  function onAuth({ token, tenant_id }) {
    if (token)     localStorage.setItem('narayan_token',     token);
    if (tenant_id) localStorage.setItem('narayan_tenant_id', tenant_id);
    localStorage.setItem('narayan_session_started_at', String(Date.now()));
    setPage('chat');
  }

  function onNavigate(dest) {
    if (dest === 'logout') {
      localStorage.clear();
      setPage('auth');
    } else {
      setPage(dest);
    }
  }

  if (page === 'loading') {
    return (
      <div className="min-h-screen bg-bg flex items-center justify-center">
        <div className="size-8 rounded-xl bg-accent/10 border border-accent/20 animate-pulse" />
      </div>
    );
  }

  if (page === 'auth')     return <AuthPage onAuth={onAuth} />;
  if (page === 'settings') return <SettingsPage onBack={() => setPage('chat')} />;
  return <ChatPage onNavigate={onNavigate} onPlanError={setPlanError} />;
}
