import { useEffect, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, ChevronRight, Database, Loader2, RefreshCw, Shield, Sparkles } from 'lucide-react';
import { connections, databaseConnections } from '../../api';

export default function DatabaseConnectionCard({ onConnected }) {
  const [name, setName] = useState('');
  const [connectionString, setConnectionString] = useState('');
  const [allowWrites, setAllowWrites] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loadingExisting, setLoadingExisting] = useState(true);
  const [existingDatabases, setExistingDatabases] = useState([]);
  const [existingError, setExistingError] = useState('');
  const [showNewConnectionForm, setShowNewConnectionForm] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');

  const canSubmit = name.trim() && connectionString.trim() && !saving;

  useEffect(() => {
    let active = true;

    async function loadExistingDatabases() {
      setLoadingExisting(true);
      setExistingError('');
      try {
        const res = await connections.list();
        const dbs = (res.connectors || [])
          .filter(connector => connector?.category === 'connector/database')
          .slice()
          .sort((a, b) => String(a.name || '').localeCompare(String(b.name || '')));

        if (active) {
          setExistingDatabases(dbs);
          setShowNewConnectionForm(prev => prev || dbs.length === 0);
        }
      } catch (e) {
        if (active) {
          setExistingError(e.message || 'Failed to load saved databases');
        }
      } finally {
        if (active) {
          setLoadingExisting(false);
        }
      }
    }

    loadExistingDatabases();
    return () => {
      active = false;
    };
  }, []);

  function handleUseExistingDatabase(database) {
    if (!database?.name || saving) return;
    const savedName = database.name;
    setSuccess(`Using ${savedName}. Returning to plan mode...`);
    setError('');
    onConnected?.({
      name: savedName,
      existing: true,
      connector: database,
    });
  }

  async function handleConnect() {
    if (!canSubmit) return;
    setSaving(true);
    setError('');
    setSuccess('');

    const savedName = name.trim();
    const savedConnectionString = connectionString.trim();

    try {
      const test = await databaseConnections.test(savedConnectionString);
      if (!test?.connected) {
        throw new Error(test?.error || 'Database test failed');
      }

      await databaseConnections.register(savedName, savedConnectionString, allowWrites);
      setSuccess(`Connected ${savedName}. Returning to plan mode...`);
      onConnected?.({ name: savedName, connectionString: savedConnectionString, allowWrites });
    } catch (e) {
      setError(e.message || 'Failed to connect database');
    } finally {
      setSaving(false);
    }
  }

  const hasExistingDatabases = existingDatabases.length > 0;

  return (
    <motion.div
      className="rounded-xl border-l-4 border-l-accent border border-accent/20 bg-accent-soft/20 overflow-hidden"
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
    >
      <div className="flex items-center gap-2 px-4 py-3 border-b border-accent/15">
        <Database size={14} className="text-accent" />
        <span className="text-sm font-semibold text-accent">Connect your database inline</span>
      </div>

      <div className="px-4 py-4 space-y-4">
        <div className="flex items-start gap-2.5">
          <Sparkles size={14} className="mt-0.5 text-accent shrink-0" />
          <div className="space-y-1">
            <p className="text-sm text-tx-1 font-medium">Choose an existing database or add a new one</p>
            <p className="text-xs text-tx-3">
              Pick a saved database to keep moving, or connect a new one and resume plan mode with the saved name.
            </p>
          </div>
        </div>

        <div className="space-y-2 rounded-xl border border-border/70 bg-bg-card/70 px-3 py-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-xs font-semibold uppercase tracking-wide text-tx-4">Saved databases</p>
              <p className="text-[11px] text-tx-3">Pick one of your existing database connections.</p>
            </div>
            <button
              type="button"
              onClick={() => {
                setLoadingExisting(true);
                setExistingError('');
                connections.list()
                  .then(res => {
                    const dbs = (res.connectors || [])
                      .filter(connector => connector?.category === 'connector/database')
                      .slice()
                      .sort((a, b) => String(a.name || '').localeCompare(String(b.name || '')));
                    setExistingDatabases(dbs);
                  })
                  .catch(e => setExistingError(e.message || 'Failed to load saved databases'))
                  .finally(() => setLoadingExisting(false));
              }}
              className="inline-flex items-center gap-1.5 rounded-full border border-border bg-bg px-2.5 py-1 text-[11px] font-medium text-tx-3 hover:border-accent/40 hover:text-accent transition-colors"
            >
              <RefreshCw size={11} />
              Refresh
            </button>
          </div>

          {loadingExisting ? (
            <div className="flex items-center gap-2 text-xs text-tx-4">
              <Loader2 size={12} className="animate-spin" />
              Loading saved databases...
            </div>
          ) : hasExistingDatabases ? (
            <div className="flex flex-col gap-2">
              {existingDatabases.map(database => (
                <button
                  key={database.name}
                  type="button"
                  onClick={() => handleUseExistingDatabase(database)}
                  disabled={saving}
                  className={clsx(
                    'flex items-center justify-between gap-3 rounded-lg border px-3 py-2 text-left transition-colors',
                    saving
                      ? 'cursor-not-allowed border-border/70 bg-bg-card/60 opacity-70'
                      : 'border-border bg-bg hover:border-accent/40 hover:bg-accent-soft/20',
                  )}
                >
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-tx-1 truncate">{database.name}</p>
                    <p className="mt-0.5 text-[11px] text-tx-4 truncate">
                      {database.summary || database.base_url || 'Saved database connection'}
                    </p>
                  </div>
                  <span className="inline-flex items-center gap-1 rounded-full bg-accent-soft px-2 py-0.5 text-[10px] font-medium text-accent">
                    Use
                    <ChevronRight size={10} />
                  </span>
                </button>
              ))}
            </div>
          ) : (
            <p className="text-xs text-tx-4">
              No saved databases were found. Add one below to continue.
            </p>
          )}

          {existingError ? (
            <p className="text-xs text-err">{existingError}</p>
          ) : null}
        </div>

        {showNewConnectionForm ? (
          <>
            <div className="flex items-center gap-2 text-[11px] uppercase tracking-wide text-tx-4">
              <span className="h-px flex-1 bg-border" />
              add a new database
              <span className="h-px flex-1 bg-border" />
            </div>

            <div className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-tx-2">Database name</label>
                <input
                  value={name}
                  onChange={e => setName(e.target.value)}
                  placeholder="prod_db"
                  className="input-field"
                />
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-tx-2">Connection string</label>
                <input
                  type="password"
                  value={connectionString}
                  onChange={e => setConnectionString(e.target.value)}
                  placeholder="postgres://user:pass@host:5432/db"
                  className="input-field"
                  autoComplete="off"
                />
              </div>

              <label className="flex items-start gap-2 text-xs text-tx-2">
                <input
                  type="checkbox"
                  checked={allowWrites}
                  onChange={e => setAllowWrites(e.target.checked)}
                  className="mt-0.5 rounded border-border text-accent focus:ring-accent"
                />
                <span>
                  Allow writes
                  <span className="block text-[11px] text-tx-4">
                    Leave off for read-only monitoring, or enable it if the agent should update tables.
                  </span>
                </span>
              </label>
            </div>
          </>
        ) : (
          <div className="flex items-center justify-between gap-3 rounded-xl border border-dashed border-border bg-bg px-3 py-3">
            <div className="min-w-0">
              <p className="text-sm font-medium text-tx-1">Need a new database instead?</p>
              <p className="text-[11px] text-tx-4">
                You can connect a fresh one if none of the saved databases are right.
              </p>
            </div>
            <button
              type="button"
              onClick={() => setShowNewConnectionForm(true)}
              className="btn-secondary shrink-0"
            >
              Add new
            </button>
          </div>
        )}

        {error && <p className="text-xs text-err">{error}</p>}
        {success && (
          <div className="flex items-center gap-2 rounded-lg border border-ok/20 bg-ok-soft/30 px-3 py-2 text-xs text-ok">
            <CheckCircle2 size={12} />
            {success}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={handleConnect}
            disabled={!canSubmit}
            className={clsx(
              'btn-primary flex items-center gap-2 disabled:opacity-50',
              saving && 'cursor-wait',
            )}
          >
            {saving ? <Loader2 size={12} className="animate-spin" /> : <Shield size={12} />}
            {saving ? 'Testing & saving…' : 'Test & connect'}
          </button>
        </div>
      </div>
    </motion.div>
  );
}
