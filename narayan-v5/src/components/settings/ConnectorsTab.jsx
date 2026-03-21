import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { Plug, Loader2, CheckCircle2, ExternalLink, Trash2, X } from 'lucide-react';
import { connectors } from '../../api';

const ALL_CONNECTORS = [
  { type: 'github', label: 'GitHub', auth: 'oauth', color: 'bg-tx-1 text-bg-card' },
  { type: 'slack', label: 'Slack', auth: 'oauth', color: 'bg-vio-soft text-vio' },
  { type: 'gmail', label: 'Gmail', auth: 'oauth', color: 'bg-err-soft text-err' },
  { type: 'outlook', label: 'Outlook', auth: 'oauth', color: 'bg-info-soft text-info' },
  { type: 'salesforce', label: 'Salesforce', auth: 'oauth', color: 'bg-info-soft text-info' },
  { type: 'hubspot', label: 'HubSpot', auth: 'oauth', color: 'bg-accent-soft text-accent' },
  { type: 'jira', label: 'Jira', auth: 'oauth', color: 'bg-info-soft text-info' },
  { type: 'notion', label: 'Notion', auth: 'oauth', color: 'bg-tx-1 text-bg-card' },
  { type: 'quickbooks', label: 'QuickBooks', auth: 'oauth', color: 'bg-ok-soft text-ok' },
  { type: 'docusign', label: 'DocuSign', auth: 'oauth', color: 'bg-info-soft text-info' },
  { type: 'zendesk', label: 'Zendesk', auth: 'apikey', color: 'bg-ok-soft text-ok' },
  { type: 'servicenow', label: 'ServiceNow', auth: 'apikey', color: 'bg-ok-soft text-ok' },
  { type: 'pagerduty', label: 'PagerDuty', auth: 'apikey', color: 'bg-ok-soft text-ok' },
  { type: 'greenhouse', label: 'Greenhouse', auth: 'apikey', color: 'bg-ok-soft text-ok' },
  { type: 'dbt_cloud', label: 'dbt Cloud', auth: 'apikey', color: 'bg-accent-soft text-accent' },
  { type: 'linear', label: 'Linear', auth: 'apikey', color: 'bg-vio-soft text-vio' },
];

export default function ConnectorsTab({ onFlash }) {
  const [installed, setInstalled] = useState([]);
  const [loading, setLoading] = useState(true);
  const [apiKeyForm, setApiKeyForm] = useState({ type: null, key: '' });

  useEffect(() => {
    connectors.list().then(d => setInstalled(d.connectors || [])).catch(() => {}).finally(() => setLoading(false));
  }, []);

  const installedTypes = new Set(installed.map(c => c.type || c.connector_type));

  async function installApiKey(type) {
    try {
      await connectors.installApiKey(type, apiKeyForm.key);
      setApiKeyForm({ type: null, key: '' });
      const r = await connectors.list();
      setInstalled(r.connectors || []);
      onFlash?.(`${type} connected`);
    } catch (e) { onFlash?.(e.message); }
  }

  async function uninstall(type) {
    try {
      await connectors.uninstall(type);
      setInstalled(l => l.filter(c => (c.type || c.connector_type) !== type));
      onFlash?.(`${type} disconnected`);
    } catch (e) { onFlash?.(e.message); }
  }

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  return (
    <div className="grid grid-cols-3 gap-3">
      {ALL_CONNECTORS.map(conn => {
        const isInstalled = installedTypes.has(conn.type);
        const showForm = apiKeyForm.type === conn.type;
        return (
          <div key={conn.type} className={clsx('card p-4 transition-all', isInstalled && 'ring-2 ring-ok/20')}>
            <div className="flex items-center gap-2.5 mb-3">
              <span className={clsx('size-8 rounded-lg flex items-center justify-center text-xs font-bold', conn.color)}>
                {conn.label.charAt(0)}
              </span>
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-tx-1">{conn.label}</p>
                <p className="text-[10px] text-tx-4 capitalize">{conn.auth === 'oauth' ? 'OAuth' : 'API Key'}</p>
              </div>
            </div>

            {isInstalled ? (
              <div className="flex items-center gap-2">
                <span className="badge bg-ok-soft text-ok border border-ok/20 flex-1 justify-center"><CheckCircle2 size={10} /> Connected</span>
                <button onClick={() => uninstall(conn.type)} className="p-1.5 rounded text-tx-4 hover:text-err transition-colors">
                  <Trash2 size={12} />
                </button>
              </div>
            ) : conn.auth === 'oauth' ? (
              <a href={connectors.oauthStartUrl(conn.type)} className="btn-primary w-full text-xs text-center flex items-center justify-center gap-1">
                Connect <ExternalLink size={10} />
              </a>
            ) : showForm ? (
              <div className="space-y-2">
                <input value={apiKeyForm.key} onChange={e => setApiKeyForm(f => ({ ...f, key: e.target.value }))}
                  placeholder="API key..." className="input-field text-xs" type="password" />
                <div className="flex gap-2">
                  <button onClick={() => installApiKey(conn.type)} disabled={!apiKeyForm.key.trim()} className="btn-primary text-xs flex-1 disabled:opacity-50">Save</button>
                  <button onClick={() => setApiKeyForm({ type: null, key: '' })} className="btn-secondary text-xs"><X size={10} /></button>
                </div>
              </div>
            ) : (
              <button onClick={() => setApiKeyForm({ type: conn.type, key: '' })} className="btn-secondary w-full text-xs">
                Add API key
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
