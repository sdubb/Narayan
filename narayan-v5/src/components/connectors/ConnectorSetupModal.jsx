import { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import {
  ArrowRight,
  Check,
  Clock,
  ExternalLink,
  Lock,
  X,
  AlertCircle,
  RefreshCw,
  Loader2,
} from 'lucide-react';
import { connectors, credentials as credentialsApi } from '../../api';

export const BUILT_IN_CONNECTORS = [
  // OAuth
  { id: 'github', name: 'GitHub', type: 'oauth', icon: '🐙', category: 'devops' },
  { id: 'slack', name: 'Slack', type: 'oauth', icon: '💬', category: 'communication' },
  { id: 'gmail', name: 'Gmail', type: 'oauth', icon: '✉️', category: 'communication' },
  { id: 'outlook', name: 'Outlook', type: 'oauth', icon: '📧', category: 'communication' },
  { id: 'salesforce', name: 'Salesforce', type: 'oauth', icon: '☁️', category: 'crm' },
  { id: 'hubspot', name: 'HubSpot', type: 'oauth', icon: '📊', category: 'crm' },
  { id: 'jira', name: 'Jira', type: 'oauth', icon: '🎯', category: 'devops' },
  { id: 'notion', name: 'Notion', type: 'oauth', icon: '📝', category: 'productivity' },
  { id: 'quickbooks', name: 'QuickBooks', type: 'oauth', icon: '💰', category: 'finance' },
  { id: 'docusign', name: 'DocuSign', type: 'oauth', icon: '✍️', category: 'legal' },
  { id: 'stripe', name: 'Stripe', type: 'oauth', icon: '💳', category: 'finance' },
  { id: 'intercom', name: 'Intercom', type: 'oauth', icon: '💭', category: 'support' },
  // API Key
  { id: 'linear', name: 'Linear', type: 'apikey', icon: '📋', category: 'devops' },
  { id: 'monday', name: 'monday.com', type: 'apikey', icon: '📅', category: 'productivity' },
  { id: 'zendesk', name: 'Zendesk', type: 'apikey', icon: '🎧', category: 'support' },
  { id: 'servicenow', name: 'ServiceNow', type: 'apikey', icon: '⚙️', category: 'itsm' },
  { id: 'pagerduty', name: 'PagerDuty', type: 'apikey', icon: '🚨', category: 'devops' },
  { id: 'freshdesk', name: 'Freshdesk', type: 'apikey', icon: '🎫', category: 'support' },
  { id: 'greenhouse', name: 'Greenhouse', type: 'apikey', icon: '🌱', category: 'hr' },
  { id: 'dbt_cloud', name: 'dbt Cloud', type: 'apikey', icon: '🔄', category: 'data' },
];

function normalizeConnectorText(text) {
  return String(text || '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim()
    .replace(/\s+/g, ' ');
}

export function extractConnectorIdsFromText(text) {
  const haystack = normalizeConnectorText(text);
  return BUILT_IN_CONNECTORS
    .filter(connector => {
      const idNeedle = normalizeConnectorText(connector.id);
      const nameNeedle = normalizeConnectorText(connector.name);
      return (idNeedle && haystack.includes(idNeedle)) || (nameNeedle && haystack.includes(nameNeedle));
    })
    .map(connector => connector.id);
}

export function ConnectorSetupModal({ requiredConnectors = [], onClose, onVerified, mode = 'modal' }) {
  const [connectorStates, setConnectorStates] = useState({});
  const [verifying, setVerifying] = useState(false);
  const [allVerified, setAllVerified] = useState(false);
  const [error, setError] = useState(null);
  const [apiKeys, setApiKeys] = useState({});
  const [savingApiKeyFor, setSavingApiKeyFor] = useState(null);

  // Initial load: check which connectors are installed
  useEffect(() => {
    verifyConnectors();
  }, []);

  // Auto-close/continue when all required connectors are verified
  useEffect(() => {
    if (allVerified && requiredConnectors.length > 0) {
      setTimeout(() => {
        if (onVerified) onVerified(true);
      }, 500);
    }
  }, [allVerified, requiredConnectors, onVerified]);

  const verifyConnectors = async () => {
    try {
      setVerifying(true);
      const installed = await connectors.list();
      const installedIds = new Set(installed.map(c => c.id || c.type));

      const states = {};
      const validationErrors = [];

      // Check all required connectors
      for (const id of requiredConnectors) {
        if (!installedIds.has(id)) {
          states[id] = 'pending';
        } else {
          // Deep validation: call POST /connectors/:type/validate
          try {
            const res = await connectors.validate(id);
            if (res.valid) {
              states[id] = 'connected';
            } else {
              states[id] = 'error';
              validationErrors.push(`${id}: ${res.error || 'Validation failed'}`);
            }
          } catch (err) {
            states[id] = 'error';
            validationErrors.push(`${id}: ${err.message}`);
          }
        }
      }

      setConnectorStates(states);

      // Check if all are verified AND valid
      const allConnected = requiredConnectors.every(id => states[id] === 'connected');
      setAllVerified(allConnected);

      if (validationErrors.length > 0) {
        setError(`Validation failed: ${validationErrors.join('; ')}`);
      } else {
        setError(null);
      }
    } catch (err) {
      setError(err.message || 'Failed to verify connectors');
      console.error('Verification error:', err);
    } finally {
      setVerifying(false);
    }
  };

  const handleConnectOAuth = async (connectorId) => {
    try {
      setConnectorStates(prev => ({ ...prev, [connectorId]: 'connecting' }));
      
      // Get OAuth start URL
      const url = connectors.oauthStartUrl(connectorId);
      
      // Open OAuth flow in new window
      const width = 600;
      const height = 700;
      const left = window.innerWidth / 2 - width / 2;
      const top = window.innerHeight / 2 - height / 2;
      
      const oauthWindow = window.open(
        url,
        'OAuthConnect',
        `width=${width},height=${height},left=${left},top=${top}`
      );

      // Poll for window closure and verification
      let checkCount = 0;
      const checkInterval = setInterval(() => {
        checkCount++;
        
        if (oauthWindow?.closed || checkCount > 120) {
          // Window closed or timeout
          clearInterval(checkInterval);
          setTimeout(() => verifyConnectors(), 500);
        }
      }, 500);
    } catch (err) {
      setConnectorStates(prev => ({ ...prev, [connectorId]: 'pending' }));
      setError(`Failed to connect ${connectorId}: ${err.message}`);
    }
  };

  const handleConnectApiKey = (connectorId) => {
    if (mode === 'inline') {
      setApiKeys(prev => (prev[connectorId] ? prev : { ...prev, [connectorId]: '' }));
      return;
    }

    // Open settings tab for API key entry
    window.open(`/settings?tab=connectors&setup=${connectorId}`, '_blank');

    // Re-verify after delay
    setTimeout(() => verifyConnectors(), 2000);
  };

  const handleSaveApiKey = async (connectorId, connectorLabel) => {
    const apiKey = (apiKeys[connectorId] || '').trim();
    if (!apiKey) return;

    try {
      setSavingApiKeyFor(connectorId);
      setError(null);

      try {
        await connectors.installApiKey(connectorId, apiKey);
      } catch {
        await credentialsApi.set(connectorId, apiKey, '', connectorLabel);
      }

      setConnectorStates(prev => ({ ...prev, [connectorId]: 'connected' }));
      setApiKeys(prev => ({ ...prev, [connectorId]: '' }));
      setTimeout(() => verifyConnectors(), 250);
    } catch (err) {
      setError(`Failed to save ${connectorId}: ${err.message}`);
      setConnectorStates(prev => ({ ...prev, [connectorId]: 'error' }));
    } finally {
      setSavingApiKeyFor(null);
    }
  };

  const getConnectorDef = (id) => BUILT_IN_CONNECTORS.find(c => c.id === id);

  const needsSetup = requiredConnectors.filter(id => connectorStates[id] !== 'connected');
  const setupProgress = requiredConnectors.length > 0 
    ? Math.round((requiredConnectors.length - needsSetup.length) / requiredConnectors.length * 100)
    : 0;

  if (!requiredConnectors || requiredConnectors.length === 0) {
    return null;
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -20 }}
      className={mode === 'modal' ? "fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm p-4" : ""}
    >
      <div className={`${mode === 'modal' ? 'bg-white rounded-2xl shadow-xl max-w-xl w-full' : 'w-full'} p-6 space-y-6`}>
        {/* Header */}
        <div className="flex items-start justify-between">
          <div>
            <h2 className="text-2xl font-bold text-tx-1">Connect Required Services</h2>
            <p className="mt-1 text-sm text-tx-3">
              {allVerified && !error ? '✅ All connected and verified!' : `${needsSetup.length} of ${requiredConnectors.length} pending`}
              {error && <span className="block text-err mt-1">⚠️ Some credentials failed validation</span>}
            </p>
          </div>
          {mode === 'modal' && (
            <button
              onClick={onClose}
              className="text-tx-4 hover:text-tx-1 transition"
              disabled={verifying}
            >
              <X className="size-5" />
            </button>
          )}
        </div>

        {/* Progress Bar */}
        <div className="space-y-2">
          <div className="flex items-center justify-between text-xs">
            <span className="text-tx-3">Connection Progress</span>
            <span className="font-medium text-accent">{setupProgress}%</span>
          </div>
          <div className="h-2 bg-bg rounded-full overflow-hidden">
            <motion.div
              className="h-full bg-gradient-to-r from-accent to-info rounded-full"
              initial={{ width: 0 }}
              animate={{ width: `${setupProgress}%` }}
              transition={{ duration: 0.5 }}
            />
          </div>
        </div>

        {/* Error Message */}
        {error && (
          <div className="rounded-lg border border-info-soft/50 bg-info-soft/30 p-3 flex gap-3">
            <AlertCircle className="size-5 text-info flex-shrink-0 mt-0.5" />
            <div className="text-sm text-info">{error}</div>
          </div>
        )}

        {/* Connector List */}
        <div className="space-y-2 max-h-96 overflow-y-auto">
          {requiredConnectors.map((connectorId, idx) => {
            const def = getConnectorDef(connectorId);
            const state = connectorStates[connectorId] || 'pending';

            if (!def) return null;

            return (
              <motion.div
                key={connectorId}
                initial={{ opacity: 0, x: -10 }}
                animate={{ opacity: 1, x: 0 }}
                transition={{ delay: idx * 0.05 }}
                className="space-y-3 rounded-lg border border-border/50 bg-bg-card/50 p-4 backdrop-blur-sm"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-3 flex-1 min-w-0">
                    <span className="text-2xl">{def.icon}</span>
                    <div className="flex-1 min-w-0">
                      <p className="font-medium text-tx-1">{def.name}</p>
                      <p className="text-xs text-tx-4 capitalize">{def.category}</p>
                    </div>
                  </div>

                  {/* Status + Button */}
                  <div className="flex items-center gap-2 shrink-0">
                  {state === 'connected' && (
                    <motion.div
                      initial={{ scale: 0 }}
                      animate={{ scale: 1 }}
                      className="flex items-center gap-2 rounded-full bg-ok-soft/40 px-3 py-1.5"
                    >
                      <Check className="size-4 text-ok" />
                      <span className="text-xs font-medium text-ok">Connected</span>
                    </motion.div>
                  )}

                  {state === 'pending' && (
                    def.type === 'oauth' ? (
                      <motion.button
                        whileHover={{ scale: 1.05 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={() => handleConnectOAuth(connectorId)}
                        disabled={verifying}
                        className="inline-flex items-center gap-2 rounded-lg bg-accent-soft/40 px-3 py-1.5 text-xs font-medium text-accent hover:bg-accent-soft/60 disabled:opacity-50 disabled:cursor-wait transition"
                      >
                        Connect <ExternalLink className="size-3.5" />
                      </motion.button>
                    ) : mode === 'inline' ? (
                      <span className="inline-flex items-center gap-2 rounded-lg border border-border bg-bg px-3 py-1.5 text-xs font-medium text-tx-3">
                        <Lock className="size-3.5" />
                        Paste the key below
                      </span>
                    ) : (
                      <motion.button
                        whileHover={{ scale: 1.05 }}
                        whileTap={{ scale: 0.95 }}
                        onClick={() => handleConnectApiKey(connectorId)}
                        disabled={verifying}
                        className="inline-flex items-center gap-2 rounded-lg bg-accent-soft/40 px-3 py-1.5 text-xs font-medium text-accent hover:bg-accent-soft/60 disabled:opacity-50 disabled:cursor-wait transition"
                      >
                        Setup <Lock className="size-3.5" />
                      </motion.button>
                    )
                  )}

                  {state === 'connecting' && (
                    <div className="flex items-center gap-2 rounded-full bg-info-soft/40 px-3 py-1.5">
                      <Clock className="size-4 text-info animate-spin" />
                      <span className="text-xs font-medium text-info">Connecting...</span>
                    </div>
                  )}

                  {state === 'error' && (
                    <div className="flex items-center gap-2 rounded-full bg-err-soft/40 px-3 py-1.5">
                      <AlertCircle className="size-4 text-err" />
                      <span className="text-xs font-medium text-err">Invalid</span>
                    </div>
                  )}
                  </div>
                </div>

                {mode === 'inline' && def.type === 'apikey' && state === 'pending' && (
                  <div className="space-y-2 rounded-lg border border-border/60 bg-bg px-3 py-3">
                    <p className="text-xs text-tx-3">
                      Paste the API key here. We’ll save it securely and continue the plan.
                    </p>
                    <div className="flex flex-col gap-2 sm:flex-row">
                      <input
                        type="password"
                        value={apiKeys[connectorId] || ''}
                        onChange={e => setApiKeys(prev => ({ ...prev, [connectorId]: e.target.value }))}
                        placeholder={`${def.name} API key`}
                        className="input-field flex-1"
                        autoComplete="off"
                      />
                      <button
                        type="button"
                        onClick={() => handleSaveApiKey(connectorId, def.name)}
                        disabled={savingApiKeyFor === connectorId || !(apiKeys[connectorId] || '').trim()}
                        className="btn-primary inline-flex items-center justify-center gap-2 disabled:opacity-50"
                      >
                        {savingApiKeyFor === connectorId ? <Loader2 size={12} className="animate-spin" /> : <Check className="size-3.5" />}
                        Save & continue
                      </button>
                    </div>
                  </div>
                )}
              </motion.div>
            );
          })}
        </div>

        {/* Action Buttons */}
        <div className="flex gap-3">
          {mode === 'modal' && (
            <motion.button
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={verifyConnectors}
              disabled={verifying}
              className="flex-1 inline-flex items-center justify-center gap-2 rounded-lg border border-border bg-bg-card/50 px-4 py-2.5 text-sm font-medium text-tx-1 hover:bg-bg-card disabled:opacity-50 disabled:cursor-wait transition"
            >
              <RefreshCw className={`size-4 ${verifying ? 'animate-spin' : ''}`} />
              Verify{verifying ? 'ing...' : ''}
            </motion.button>
          )}

          {allVerified && mode === 'modal' && (
            <motion.button
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={() => onVerified?.(true)}
              className="flex-1 inline-flex items-center justify-center gap-2 rounded-lg bg-gradient-to-r from-ok to-ok/80 px-4 py-2.5 text-sm font-medium text-white hover:shadow-lg transition"
            >
              Continue <ArrowRight className="size-4" />
            </motion.button>
          )}

          {!allVerified && mode === 'modal' && (
            <motion.button
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={onClose}
              className="flex-1 rounded-lg border border-border bg-white px-4 py-2.5 text-sm font-medium text-tx-1 hover:bg-bg transition"
            >
              Skip for Now
            </motion.button>
          )}
        </div>

        {/* Helper Text */}
        {!allVerified && (
          <p className="text-xs text-tx-4 text-center">
            💡 Connect these services to enable automation. You can update them later in Settings.
          </p>
        )}
      </div>
    </motion.div>
  );
}

export function useConnectorVerification(requiredConnectors = []) {
  const [verified, setVerified] = useState(false);
  const [missing, setMissing] = useState(requiredConnectors);
  const [loading, setLoading] = useState(false);

  const verify = async () => {
    try {
      setLoading(true);
      const installed = await connectors.list();
      const installedIds = new Set(installed.map(c => c.id || c.type));
      
      // Deep validation: test each installed connector
      const validationResults = {};
      for (const id of requiredConnectors) {
        if (installedIds.has(id)) {
          try {
            const res = await connectors.validate(id);
            validationResults[id] = res.valid;
          } catch {
            validationResults[id] = false;
          }
        } else {
          validationResults[id] = false;
        }
      }
      
      const missingIds = requiredConnectors.filter(id => !validationResults[id]);
      setMissing(missingIds);
      setVerified(missingIds.length === 0);
      
      return missingIds.length === 0;
    } catch (err) {
      console.error('Verification failed:', err);
      return false;
    } finally {
      setLoading(false);
    }
  };

  return { verified, missing, loading, verify };
}
