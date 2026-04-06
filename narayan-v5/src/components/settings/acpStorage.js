const STORAGE_KEY = 'narayan_acp_peer_config';

export function readAcpPeerConfig() {
  if (typeof window === 'undefined') {
    return { name: '', peer_url: '', token: '', summary: '' };
  }

  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return { name: '', peer_url: '', token: '', summary: '' };
    const parsed = JSON.parse(raw);
    return {
      name: parsed?.name || '',
      peer_url: parsed?.peer_url || '',
      token: parsed?.token || '',
      summary: parsed?.summary || '',
    };
  } catch {
    return { name: '', peer_url: '', token: '', summary: '' };
  }
}

export function writeAcpPeerConfig(config) {
  if (typeof window === 'undefined') return;
  const payload = {
    name: String(config?.name || '').trim(),
    peer_url: String(config?.peer_url || '').trim(),
    token: String(config?.token || '').trim(),
    summary: String(config?.summary || '').trim(),
    updated_at: new Date().toISOString(),
  };
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
}

export function clearAcpPeerConfig() {
  if (typeof window === 'undefined') return;
  window.localStorage.removeItem(STORAGE_KEY);
}
