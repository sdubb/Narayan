import { motion } from 'framer-motion';
import clsx from 'clsx';
import { Plug, ExternalLink } from 'lucide-react';

const CONNECTOR_COLORS = {
  github: 'ok', zendesk: 'ok', salesforce: 'info', quickbooks: 'ok',
  docusign: 'info', pagerduty: 'err', hubspot: 'accent', notion: 'vio',
  greenhouse: 'info', dbt_cloud: 'warn', slack: 'vio', gmail: 'err',
  outlook: 'info', jira: 'info', linear: 'vio', servicenow: 'ok',
};

const COLOR_STYLES = {
  ok: 'border-ok/25 bg-ok-soft/40 text-ok',
  err: 'border-err/25 bg-err-soft/40 text-err',
  warn: 'border-warn/25 bg-warn-soft/40 text-warn',
  info: 'border-info/25 bg-info-soft/40 text-info',
  vio: 'border-vio/25 bg-vio-soft/40 text-vio',
  accent: 'border-accent/25 bg-accent-soft/40 text-accent',
};

export default function ConnectorTriggerCard({ event }) {
  const colorKey = CONNECTOR_COLORS[event.connector_type] || 'ok';
  const style = COLOR_STYLES[colorKey] || COLOR_STYLES.ok;

  return (
    <motion.div
      className={clsx('rounded-xl border overflow-hidden shadow-sm', style.split(' ').slice(0, 2).join(' '))}
      initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.15 }}
    >
      <div className="flex items-center gap-2 px-3.5 py-2.5">
        <Plug size={12} className={clsx('shrink-0', style.split(' ')[2])} />
        <span className={clsx('text-xs font-bold tracking-wider uppercase flex-1', style.split(' ')[2])}>
          {event.connector_type} trigger
        </span>
      </div>
      <div className="px-3.5 py-2.5 border-t border-border/30 space-y-1.5">
        <p className="text-xs text-tx-1 font-medium">{event.event_type || 'Webhook received'}</p>
        <p className="text-xs text-tx-3">Agent created from inbound {event.connector_type} webhook</p>
        {event.external_id && (
          <div className="flex items-center gap-1.5 text-xs text-tx-4">
            <ExternalLink size={10} />
            <span className="font-mono">{event.external_id}</span>
          </div>
        )}
      </div>
    </motion.div>
  );
}
