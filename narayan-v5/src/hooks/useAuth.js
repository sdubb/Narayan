import { useState, useEffect } from 'react';

export function useAuth() {
  const [token, setToken] = useState(() => localStorage.getItem('narayan_token'));
  const [apiKey, setApiKey] = useState(() => localStorage.getItem('narayan_api_key'));
  const [tenantId, setTenantId] = useState(() => localStorage.getItem('narayan_tenant_id'));

  const isAuthed = !!(token || apiKey);

  function saveSession({ token: t, api_key: k, tenant_id: tid }) {
    if (t) { localStorage.setItem('narayan_token', t); setToken(t); }
    if (k) { localStorage.setItem('narayan_api_key', k); setApiKey(k); }
    if (tid) { localStorage.setItem('narayan_tenant_id', tid); setTenantId(tid); }
  }

  function logout() {
    localStorage.removeItem('narayan_token');
    localStorage.removeItem('narayan_api_key');
    localStorage.removeItem('narayan_tenant_id');
    setToken(null); setApiKey(null); setTenantId(null);
  }

  return { token, apiKey, tenantId, isAuthed, saveSession, logout };
}
