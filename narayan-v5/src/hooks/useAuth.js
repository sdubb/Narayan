import { useState, useEffect } from 'react';

export function useAuth() {
  const [token, setToken] = useState(() => localStorage.getItem('narayan_token'));
  const [tenantId, setTenantId] = useState(() => localStorage.getItem('narayan_tenant_id'));

  const isAuthed = !!token;

  function saveSession({ token: t, tenant_id: tid }) {
    if (t) { localStorage.setItem('narayan_token', t); setToken(t); }
    if (tid) { localStorage.setItem('narayan_tenant_id', tid); setTenantId(tid); }
  }

  function logout() {
    localStorage.removeItem('narayan_token');
    localStorage.removeItem('narayan_tenant_id');
    setToken(null); setTenantId(null);
  }

  return { token, tenantId, isAuthed, saveSession, logout };
}
