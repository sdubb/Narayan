import { useState } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { CheckCircle2, Database, Loader2, Shield, Sparkles } from 'lucide-react';
import { databaseConnections } from '../../api';

export default function DatabaseConnectionCard({ onConnected }) {
  const [name, setName] = useState('');
  const [connectionString, setConnectionString] = useState('');
  const [allowWrites, setAllowWrites] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');

  const canSubmit = name.trim() && connectionString.trim() && !saving;

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
            <p className="text-sm text-tx-1 font-medium">Add the connection here and keep going</p>
            <p className="text-xs text-tx-3">
              This saves the database for this tenant, tests it, and then resumes plan mode with the saved name.
            </p>
          </div>
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
