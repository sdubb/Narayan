import { useCallback, useEffect, useMemo, useState } from 'react';
import { Responsive, WidthProvider } from 'react-grid-layout';
import { AnimatePresence, motion } from 'framer-motion';
import clsx from 'clsx';
import 'react-grid-layout/css/styles.css';
import 'react-resizable/css/styles.css';
import {
  Activity,
  ArrowRight,
  Bell,
  Bot,
  ChevronDown,
  Database,
  FolderTree,
  GripVertical,
  LayoutGrid,
  Loader2,
  PencilLine,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Trash2,
  Workflow,
} from 'lucide-react';
import {
  agentDefs as agentDefsApi,
  connections as connectionsApi,
  connectors as connectorsApi,
  reviews as reviewsApi,
  swarm as swarmApi,
} from '../api';
import { Sidebar } from '../components/layout';
import SavingsCard from '../components/cards/SavingsCard';

const ResponsiveGridLayout = WidthProvider(Responsive);
const BREAKPOINTS = { lg: 1280, md: 1024, sm: 768, xs: 480, xxs: 0 };
const COLS = { lg: 12, md: 10, sm: 6, xs: 4, xxs: 2 };

const WIDGETS = [
  { id: 'overview', title: 'Overview', icon: LayoutGrid },
  { id: 'agent', title: 'Selected agent', icon: Bot },
  { id: 'agents', title: 'Agent roster', icon: Workflow },
  { id: 'runs', title: 'Recent runs', icon: Activity },
  { id: 'workspace', title: 'Workspace files', icon: FolderTree },
  { id: 'reviews', title: 'Pending reviews', icon: Bell },
  { id: 'connectors', title: 'Connectors', icon: Database },
  { id: 'health', title: 'System health', icon: ShieldCheck },
  { id: 'assistant', title: 'Assistant', icon: Sparkles },
  { id: 'savings', title: 'Savings', icon: ArrowRight },
];

const DEFAULT_ACTIVE_WIDGETS = [
  'overview',
  'agent',
  'agents',
  'runs',
  'workspace',
  'reviews',
  'connectors',
  'assistant',
  'savings',
];

const DASHBOARD_ACTIONS = [
  { id: 'add_widget', label: 'Add widget', hint: 'Add a dashboard panel to the canvas.' },
  { id: 'remove_widget', label: 'Remove widget', hint: 'Hide a widget from the current layout.' },
  { id: 'move_widget', label: 'Move widget', hint: 'Nudge a widget up, down, left, or right.' },
  { id: 'connect_data_source', label: 'Connect data source', hint: 'Open settings to attach a connector.' },
  { id: 'change_widget_filter', label: 'Change widget filter', hint: 'Apply a local filter to a widget feed.' },
];

const DIRECTION_OFFSETS = {
  up: { x: 0, y: -1 },
  down: { x: 0, y: 1 },
  left: { x: -1, y: 0 },
  right: { x: 1, y: 0 },
};

function fmtCount(value) {
  return new Intl.NumberFormat('en-US').format(Number(value || 0));
}

function fmtTime(iso) {
  if (!iso) return 'just now';
  const delta = Date.now() - new Date(iso).getTime();
  const minutes = Math.max(0, Math.round(delta / 60000));
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

function toTitle(text) {
  if (!text) return '';
  return String(text).replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

function safeArray(value) {
  return Array.isArray(value) ? value : [];
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function buildDefaultLayouts(activeWidgets) {
  const ids = activeWidgets.length ? activeWidgets : DEFAULT_ACTIVE_WIDGETS;
  const use = id => ids.includes(id);

  const lg = [];
  if (use('overview'))  lg.push({ i: 'overview', x: 0, y: 0, w: 12, h: 4, minW: 6, minH: 4 });
  if (use('agent'))     lg.push({ i: 'agent', x: 0, y: 4, w: 4, h: 6, minW: 3, minH: 4 });
  if (use('agents'))    lg.push({ i: 'agents', x: 4, y: 4, w: 4, h: 6, minW: 3, minH: 4 });
  if (use('runs'))      lg.push({ i: 'runs', x: 8, y: 4, w: 4, h: 6, minW: 3, minH: 4 });
  if (use('workspace')) lg.push({ i: 'workspace', x: 0, y: 10, w: 5, h: 6, minW: 3, minH: 4 });
  if (use('reviews'))   lg.push({ i: 'reviews', x: 5, y: 10, w: 4, h: 6, minW: 3, minH: 4 });
  if (use('connectors'))lg.push({ i: 'connectors', x: 9, y: 10, w: 3, h: 6, minW: 3, minH: 4 });
  if (use('assistant')) lg.push({ i: 'assistant', x: 0, y: 16, w: 7, h: 7, minW: 4, minH: 4 });
  if (use('savings'))   lg.push({ i: 'savings', x: 7, y: 16, w: 5, h: 7, minW: 4, minH: 4 });
  if (use('health'))    lg.push({ i: 'health', x: 0, y: 23, w: 12, h: 3, minW: 6, minH: 3 });

  const compact = id => lg.find(item => item.i === id) || null;
  const order = ['overview', 'agent', 'agents', 'runs', 'workspace', 'reviews', 'connectors', 'assistant', 'savings', 'health'];

  const build = (columns, itemWidth) =>
    order
      .filter(use)
      .map((id, index) => {
        const base = compact(id) || { i: id, w: itemWidth, h: 5 };
        const row = Math.floor(index / columns);
        const col = index % columns;
        return { ...base, x: col * itemWidth, y: row * 5, w: itemWidth };
      });

  return {
    lg,
    md: build(2, 5),
    sm: build(1, 6),
    xs: build(1, 4),
    xxs: build(1, 2),
  };
}

function moveLayout(layouts, widgetId, direction) {
  const delta = DIRECTION_OFFSETS[direction];
  if (!delta) return layouts;

  const next = {};
  for (const [breakpoint, items] of Object.entries(layouts)) {
    const cols = COLS[breakpoint] || COLS.lg;
    next[breakpoint] = items.map(item => {
      if (item.i !== widgetId) return item;
      const width = item.w ?? 4;
      const height = item.h ?? 4;
      const x = clamp((item.x ?? 0) + delta.x, 0, Math.max(0, cols - width));
      const y = Math.max(0, (item.y ?? 0) + delta.y);
      return { ...item, x, y, w: width, h: height };
    });
  }
  return next;
}

function filterItems(items, filterText) {
  const query = String(filterText || '').trim().toLowerCase();
  if (!query) return items;
  return items.filter(item => {
    const fields = [
      item.title,
      item.name,
      item.goal,
      item.summary,
      item.reason,
      item.status,
      item.role_name,
      item.type,
      item.path,
      item.kind,
    ]
      .filter(Boolean)
      .map(value => String(value).toLowerCase());
    return fields.some(field => field.includes(query));
  });
}

function useDashboardStorage(key, fallback) {
  const [value, setValue] = useState(() => {
    if (typeof window === 'undefined') return fallback;
    try {
      const raw = window.localStorage.getItem(key);
      return raw ? JSON.parse(raw) : fallback;
    } catch {
      return fallback;
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem(key, JSON.stringify(value));
    } catch {}
  }, [key, value]);

  return [value, setValue];
}

function WidgetShell({ icon: Icon, title, subtitle, editing, onRemove, children, rightSlot }) {
  return (
    <div className="group flex h-full min-h-0 flex-col overflow-hidden rounded-[1.5rem] border border-border/80 bg-bg-card/90 shadow-[0_20px_40px_rgba(19,17,13,0.05)] backdrop-blur">
      <div className="dashboard-widget-handle flex shrink-0 items-start justify-between gap-3 border-b border-border/70 px-4 py-3">
        <div className="flex min-w-0 items-center gap-2">
          <div className="flex size-8 shrink-0 items-center justify-center rounded-2xl border border-accent/15 bg-accent-soft text-accent">
            <Icon size={14} />
          </div>
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-tx-1">{title}</p>
            {subtitle && <p className="truncate text-[11px] text-tx-4">{subtitle}</p>}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {rightSlot}
          {editing && onRemove && (
            <button
              type="button"
              onClick={onRemove}
              className="rounded-lg p-1.5 text-tx-4 transition-colors hover:bg-err-soft hover:text-err"
              title={`Remove ${title}`}
            >
              <Trash2 size={14} />
            </button>
          )}
          <GripVertical size={14} className="text-tx-5 opacity-0 transition-opacity group-hover:opacity-100" />
        </div>
      </div>
      <div className="min-h-0 flex-1 px-4 py-4">{children}</div>
    </div>
  );
}

function StatBlock({ label, value, hint }) {
  return (
    <div className="min-w-0">
      <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">{label}</p>
      <p className="mt-1 truncate text-2xl font-semibold text-tx-1">{value}</p>
      {hint && <p className="mt-1 text-[11px] leading-5 text-tx-4">{hint}</p>}
    </div>
  );
}

function ListLine({ title, detail, tone = 'text-tx-1', meta }) {
  return (
    <div className="flex items-start justify-between gap-3 py-2">
      <div className="min-w-0">
        <p className={clsx('truncate text-sm font-medium', tone)}>{title}</p>
        {detail && <p className="mt-0.5 text-[11px] text-tx-4">{detail}</p>}
      </div>
      {meta && <span className="shrink-0 text-[11px] text-tx-4">{meta}</span>}
    </div>
  );
}

export default function DashboardPage({ onNavigate, canCreateAgents = true }) {
  const [agents, setAgents] = useState([]);
  const [summary, setSummary] = useState(null);
  const [reviews, setReviews] = useState([]);
  const [connectors, setConnectors] = useState([]);
  const [customConnections, setCustomConnections] = useState([]);
  const [swarm, setSwarm] = useState(null);
  const [selectedAgentId, setSelectedAgentId] = useDashboardStorage('narayan.dashboard.selected-agent', '');
  const [editing, setEditing] = useDashboardStorage('narayan.dashboard.editing', false);
  const [activeWidgets, setActiveWidgets] = useDashboardStorage('narayan.dashboard.widgets', DEFAULT_ACTIVE_WIDGETS);
  const [widgetFilters, setWidgetFilters] = useDashboardStorage('narayan.dashboard.filters', {});
  const [actionLog, setActionLog] = useDashboardStorage('narayan.dashboard.action-log', []);
  const [layouts, setLayouts] = useDashboardStorage(
    'narayan.dashboard.layouts',
    buildDefaultLayouts(DEFAULT_ACTIVE_WIDGETS),
  );
  const [actionType, setActionType] = useState('add_widget');
  const [actionTarget, setActionTarget] = useState('reviews');
  const [actionDetail, setActionDetail] = useState('pending');
  const [refreshing, setRefreshing] = useState(true);
  const [assistantReply, setAssistantReply] = useState('Choose an action and apply it to the dashboard only.');

  const selectedAgent = useMemo(
    () => agents.find(agent => agent.id === selectedAgentId) || null,
    [agents, selectedAgentId],
  );

  const loadDashboard = useCallback(async () => {
    setRefreshing(true);
    const [agentsRes, reviewsRes, connectorsRes, customConnectionsRes, swarmRes] = await Promise.allSettled([
      agentDefsApi.list(),
      reviewsApi.list(),
      connectorsApi.list(),
      connectionsApi.list(),
      swarmApi.status(),
    ]);

    const agentList = agentsRes.status === 'fulfilled' ? safeArray(agentsRes.value.agents) : [];
    setAgents(agentList);
    if (agentList.length > 0) {
      setSelectedAgentId(prev => prev || agentList[0].id);
    }
    setReviews(reviewsRes.status === 'fulfilled' ? safeArray(reviewsRes.value.reviews) : []);
    setConnectors(connectorsRes.status === 'fulfilled' ? safeArray(connectorsRes.value.connectors || connectorsRes.value.items) : []);
    setCustomConnections(customConnectionsRes.status === 'fulfilled' ? safeArray(customConnectionsRes.value.connectors || customConnectionsRes.value.items) : []);
    setSwarm(swarmRes.status === 'fulfilled' ? swarmRes.value : null);
    setRefreshing(false);
  }, [setSelectedAgentId]);

  useEffect(() => {
    loadDashboard();
    const timer = window.setInterval(loadDashboard, 30000);
    return () => window.clearInterval(timer);
  }, [loadDashboard]);

  useEffect(() => {
    let cancelled = false;
    async function loadSummary() {
      if (!selectedAgentId) {
        setSummary(null);
        return;
      }
      try {
        const data = await agentDefsApi.summary(selectedAgentId);
        if (!cancelled) {
          setSummary(data);
        }
      } catch {
        if (!cancelled) {
          setSummary(null);
        }
      }
    }
    loadSummary();
    return () => { cancelled = true; };
  }, [selectedAgentId]);

  useEffect(() => {
    if (!activeWidgets || activeWidgets.length === 0) {
      setActiveWidgets(DEFAULT_ACTIVE_WIDGETS);
      setLayouts(buildDefaultLayouts(DEFAULT_ACTIVE_WIDGETS));
      return;
    }
    setLayouts(prev => {
      const built = buildDefaultLayouts(activeWidgets);
      const merged = { ...built };
      for (const bp of Object.keys(built)) {
        const prevLayout = prev?.[bp] || [];
        const prevById = new Map(prevLayout.map(item => [item.i, item]));
        merged[bp] = built[bp].map(item => prevById.get(item.i) || item);
      }
      return merged;
    });
  }, [activeWidgets, setActiveWidgets, setLayouts]);

  const activeAction = DASHBOARD_ACTIONS.find(action => action.id === actionType) || DASHBOARD_ACTIONS[0];

  function toggleWidget(id) {
    setActiveWidgets(prev => {
      if (prev.includes(id)) return prev.filter(widgetId => widgetId !== id);
      return [...prev, id];
    });
  }

  function ensureWidget(id) {
    if (!activeWidgets.includes(id)) {
      setActiveWidgets(prev => [...prev, id]);
    }
  }

  function pushActionLog(entry) {
    setActionLog(prev => [entry, ...prev].slice(0, 8));
  }

  function applyDashboardAction() {
    const target = String(actionTarget || '').trim();
    const detail = String(actionDetail || '').trim();

    if (actionType === 'add_widget') {
      const widget = WIDGETS.find(item => item.id === target || item.title.toLowerCase() === target.toLowerCase());
      if (!widget) {
        setAssistantReply('Pick a widget id such as reviews, workspace, or connectors.');
        return;
      }
      ensureWidget(widget.id);
      setAssistantReply(`Added ${widget.title} to the dashboard.`);
      pushActionLog({ action: actionType, target: widget.id, detail: '', at: Date.now() });
      setActionTarget(widget.id);
      return;
    }

    if (actionType === 'remove_widget') {
      const widget = WIDGETS.find(item => item.id === target || item.title.toLowerCase() === target.toLowerCase());
      if (!widget) {
        setAssistantReply('Pick a widget to remove.');
        return;
      }
      toggleWidget(widget.id);
      setAssistantReply(`Removed ${widget.title} from the dashboard.`);
      pushActionLog({ action: actionType, target: widget.id, detail: '', at: Date.now() });
      setActionTarget(widget.id);
      return;
    }

    if (actionType === 'move_widget') {
      const widget = WIDGETS.find(item => item.id === target || item.title.toLowerCase() === target.toLowerCase());
      const direction = detail.toLowerCase();
      if (!widget || !DIRECTION_OFFSETS[direction]) {
        setAssistantReply('Use a widget id and one of: up, down, left, right.');
        return;
      }
      setLayouts(prev => moveLayout(prev, widget.id, direction));
      setAssistantReply(`Moved ${widget.title} ${direction}.`);
      pushActionLog({ action: actionType, target: widget.id, detail: direction, at: Date.now() });
      return;
    }

    if (actionType === 'connect_data_source') {
      setAssistantReply('Open settings to connect a data source, then come back to the dashboard.');
      pushActionLog({ action: actionType, target: target || 'settings', detail, at: Date.now() });
      onNavigate('settings');
      return;
    }

    if (actionType === 'change_widget_filter') {
      const widget = WIDGETS.find(item => item.id === target || item.title.toLowerCase() === target.toLowerCase());
      if (!widget) {
        setAssistantReply('Pick a widget to filter, such as runs, reviews, or workspace.');
        return;
      }
      setWidgetFilters(prev => ({ ...prev, [widget.id]: detail }));
      setAssistantReply(`Applied "${detail}" to ${widget.title}.`);
      pushActionLog({ action: actionType, target: widget.id, detail, at: Date.now() });
      return;
    }
  }

  function clearWidgetFilter(widgetId) {
    setWidgetFilters(prev => {
      const next = { ...prev };
      delete next[widgetId];
      return next;
    });
  }

  const pendingReviews = reviews.filter(review => review.status === 'pending');
  const activeAgentCount = agents.filter(agent => agent.status === 'active').length;
  const filteredAgents = filterItems(agents, widgetFilters.agents);
  const roleCount = safeArray(summary?.roles).length;
  const runCount = safeArray(summary?.recent_runs).length;
  const fileCount = Number(summary?.workspace_files?.count || 0);
  const peerCount = safeArray(summary?.peers).length;
  const connectorCount = connectors.length + customConnections.length;
  const latestRun = safeArray(summary?.recent_runs)[0] || null;
  const filteredRuns = filterItems(safeArray(summary?.recent_runs), widgetFilters.runs);
  const filteredWorkspace = filterItems(safeArray(summary?.workspace_files?.files), widgetFilters.workspace);
  const filteredReviews = filterItems(pendingReviews, widgetFilters.reviews);
  const widgetIds = activeWidgets.length ? activeWidgets : DEFAULT_ACTIVE_WIDGETS;

  return (
    <div className="flex h-screen overflow-hidden bg-[radial-gradient(circle_at_top_left,_rgba(201,106,46,0.12),_transparent_30%),linear-gradient(180deg,_#f7f4ef_0%,_#f5f0e8_100%)]">
      <Sidebar
        agents={agents}
        selectedAgentId={selectedAgentId}
        onSelectAgent={setSelectedAgentId}
        onNewAgent={() => onNavigate(canCreateAgents ? 'chat' : 'settings')}
        onNavigate={onNavigate}
        pendingReviews={pendingReviews}
        loading={refreshing && agents.length === 0}
        canCreateAgents={canCreateAgents}
      />

      <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-border bg-bg-card/85 px-6 py-4 backdrop-blur">
          <div className="min-w-0">
            <p className="text-[11px] uppercase tracking-[0.26em] text-tx-4">Workspace</p>
            <h1 className="mt-1 font-serif text-3xl text-tx-1">Command center</h1>
            <p className="mt-1 max-w-2xl text-sm leading-6 text-tx-3">
              Monitor agents, approvals, connectors, workspace files, and runtime traces from one premium surface.
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => setEditing(prev => !prev)}
              className={clsx(
                'inline-flex items-center gap-2 rounded-full border px-3 py-2 text-xs font-medium transition-colors',
                editing
                  ? 'border-accent/25 bg-accent-soft text-accent'
                  : 'border-border bg-bg text-tx-2 hover:bg-bg-hover',
              )}
            >
              <PencilLine size={13} />
              {editing ? 'Editing canvas' : 'Edit canvas'}
            </button>
            <button
              type="button"
              onClick={loadDashboard}
              className="inline-flex items-center gap-2 rounded-full border border-border bg-bg px-3 py-2 text-xs font-medium text-tx-2 transition-colors hover:bg-bg-hover"
            >
              <RefreshCw size={13} className={clsx(refreshing && 'animate-spin')} />
              Sync
            </button>
            <button
              type="button"
              onClick={() => onNavigate('chat')}
              className="inline-flex items-center gap-2 rounded-full border border-accent/20 bg-gradient-to-r from-accent to-accent-text px-3 py-2 text-xs font-medium text-white shadow-[0_10px_24px_rgba(201,106,46,0.18)] transition-transform hover:-translate-y-0.5"
            >
              <Sparkles size={13} />
              Open agent studio
            </button>
          </div>
        </div>

        <div className="border-b border-border bg-bg-card/55 px-6 py-3">
          <div className="flex flex-wrap items-center gap-2">
            {WIDGETS.map(widget => {
              const active = widgetIds.includes(widget.id);
              const Icon = widget.icon;
              return (
                <button
                  key={widget.id}
                  type="button"
                  onClick={() => toggleWidget(widget.id)}
                  className={clsx(
                    'inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-[11px] font-medium transition-colors',
                    active
                      ? 'border-accent/20 bg-accent-soft text-accent'
                      : 'border-border bg-bg text-tx-3 hover:bg-bg-hover hover:text-tx-1',
                  )}
                >
                  <Icon size={12} />
                  {widget.title}
                </button>
              );
            })}
          </div>
        </div>

        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-4 py-4 lg:px-6">
          <ResponsiveGridLayout
            className="dashboard-grid"
            layouts={layouts}
            breakpoints={BREAKPOINTS}
            cols={COLS}
            rowHeight={40}
            margin={[16, 16]}
            containerPadding={[0, 0]}
            useCSSTransforms
            compactType="vertical"
            isDraggable={editing}
            isResizable={editing}
            draggableHandle=".dashboard-widget-handle"
            onLayoutChange={(_, allLayouts) => setLayouts(allLayouts)}
          >
            {widgetIds.includes('overview') && (
              <div key="overview">
                <WidgetShell
                  icon={LayoutGrid}
                  title="Overview"
                  subtitle="Tenant-wide snapshot"
                  editing={editing}
                  onRemove={() => toggleWidget('overview')}
                  rightSlot={refreshing ? <Loader2 size={13} className="animate-spin text-tx-4" /> : null}
                >
                  <div className="grid h-full gap-4 md:grid-cols-5">
                    <StatBlock label="Agents" value={fmtCount(agents.length)} hint={`${fmtCount(activeAgentCount)} active`} />
                    <StatBlock label="Roles" value={fmtCount(roleCount)} hint={selectedAgent ? selectedAgent.name : 'Select an agent'} />
                    <StatBlock label="Reviews" value={fmtCount(pendingReviews.length)} hint="Waiting for resolution" />
                    <StatBlock label="Connectors" value={fmtCount(connectorCount)} hint="Installed sources" />
                    <StatBlock label="Files" value={fmtCount(fileCount)} hint={selectedAgent ? 'Workspace inventory' : 'No workspace selected'} />
                  </div>
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('agent') && (
              <div key="agent">
                <WidgetShell
                  icon={Bot}
                  title="Selected agent"
                  subtitle={selectedAgent ? selectedAgent.name : 'Pick an agent'}
                  editing={editing}
                  onRemove={() => toggleWidget('agent')}
                >
                  {selectedAgent ? (
                    <div className="flex h-full flex-col gap-4">
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <p className="truncate text-xl font-semibold text-tx-1">{selectedAgent.name}</p>
                          <p className="mt-1 text-sm leading-6 text-tx-3">
                            {selectedAgent.persona || 'No persona set.'}
                          </p>
                        </div>
                        <span className={clsx(
                          'rounded-full px-2 py-1 text-[10px] font-medium uppercase tracking-[0.18em]',
                          selectedAgent.status === 'active'
                            ? 'bg-ok-soft text-ok'
                            : 'bg-bg-active text-tx-4',
                        )}>
                          {selectedAgent.status || 'draft'}
                        </span>
                      </div>
                      <div className="grid gap-3 sm:grid-cols-3">
                        <StatBlock label="Roles" value={fmtCount(roleCount)} hint="Configured on this agent" />
                        <StatBlock label="Peers" value={fmtCount(peerCount)} hint="Other agents in the tenant" />
                        <StatBlock label="Runs" value={fmtCount(runCount)} hint="Recent goal instances" />
                      </div>
                      <div className="rounded-2xl border border-border bg-bg px-3 py-3">
                        <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Persona</p>
                        <p className="mt-2 line-clamp-4 text-sm leading-6 text-tx-2">
                          {selectedAgent.persona || 'Use the agent drawer to define how it should behave.'}
                        </p>
                      </div>
                      <div className="mt-auto flex items-center gap-2">
                        <button
                          type="button"
                          onClick={() => onNavigate('chat')}
                          className="inline-flex items-center gap-2 rounded-full border border-accent/20 bg-accent-soft px-3 py-2 text-xs font-medium text-accent transition-colors hover:bg-accent/10"
                        >
                          Open agent workspace
                          <ArrowRight size={12} />
                        </button>
                        <button
                          type="button"
                          onClick={() => setSelectedAgentId(selectedAgent.id)}
                          className="inline-flex items-center gap-2 rounded-full border border-border bg-bg px-3 py-2 text-xs font-medium text-tx-2 transition-colors hover:bg-bg-hover"
                        >
                          Keep focused
                        </button>
                      </div>
                    </div>
                  ) : (
                    <div className="flex h-full items-center justify-center rounded-2xl border border-dashed border-border bg-bg/60 px-4 py-8 text-center">
                      <div>
                        <p className="text-sm font-medium text-tx-1">No agent selected</p>
                        <p className="mt-1 text-xs leading-6 text-tx-4">Choose one from the rail to load live details.</p>
                      </div>
                    </div>
                  )}
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('agents') && (
              <div key="agents">
                <WidgetShell
                  icon={Workflow}
                  title="Agent roster"
                  subtitle="Definitions in this tenant"
                  editing={editing}
                  onRemove={() => toggleWidget('agents')}
                >
                  <div className="space-y-1">
                    {filteredAgents.slice(0, 8).map(agent => (
                      <button
                        key={agent.id}
                        type="button"
                        onClick={() => setSelectedAgentId(agent.id)}
                        className={clsx(
                          'flex w-full items-center justify-between gap-3 rounded-xl border px-3 py-2 text-left transition-colors',
                          agent.id === selectedAgentId
                            ? 'border-accent/20 bg-accent-soft'
                            : 'border-transparent bg-bg hover:border-border hover:bg-bg-hover',
                        )}
                      >
                        <div className="min-w-0">
                          <p className="truncate text-sm font-medium text-tx-1">{agent.name}</p>
                          <p className="truncate text-[11px] text-tx-4">
                            {safeArray(agent.roles).length} role{safeArray(agent.roles).length === 1 ? '' : 's'}
                          </p>
                        </div>
                        <span className="shrink-0 text-[10px] uppercase tracking-[0.18em] text-tx-4">
                          {agent.status || 'draft'}
                        </span>
                      </button>
                    ))}
                  </div>
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('runs') && (
              <div key="runs">
                <WidgetShell
                  icon={Activity}
                  title="Recent runs"
                  subtitle={latestRun ? fmtTime(latestRun.created_at || latestRun.updated_at) : 'No recent runs'}
                  editing={editing}
                  onRemove={() => toggleWidget('runs')}
                >
                  <div className="space-y-1">
                    {filteredRuns.slice(0, 6).map(run => (
                      <div key={run.id} className="rounded-xl border border-border bg-bg px-3 py-2.5">
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0">
                            <p className="truncate text-sm font-medium text-tx-1">
                              {run.title || run.goal || run.name || run.id}
                            </p>
                            <p className="mt-0.5 text-[11px] text-tx-4">
                              {run.status || 'unknown'} {run.role_name ? `· ${run.role_name}` : ''}
                            </p>
                          </div>
                          <span className="shrink-0 text-[11px] text-tx-4">
                            {fmtTime(run.created_at || run.updated_at)}
                          </span>
                        </div>
                      </div>
                    ))}
                    {filteredRuns.length === 0 && (
                      <div className="rounded-xl border border-dashed border-border bg-bg/60 px-3 py-4 text-sm text-tx-4">
                        No runs yet for this agent.
                      </div>
                    )}
                    {widgetFilters.runs && (
                      <div className="mt-2 rounded-xl border border-accent/15 bg-accent-soft px-3 py-2 text-[11px] text-accent">
                        Filter: {widgetFilters.runs}
                        <button
                          type="button"
                          onClick={() => clearWidgetFilter('runs')}
                          className="ml-2 font-medium underline decoration-accent/30 underline-offset-2"
                        >
                          Clear
                        </button>
                      </div>
                    )}
                  </div>
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('workspace') && (
              <div key="workspace">
                <WidgetShell
                  icon={FolderTree}
                  title="Workspace files"
                  subtitle={summary?.workspace_files?.count ? `${fmtCount(summary.workspace_files.count)} files` : 'No files yet'}
                  editing={editing}
                  onRemove={() => toggleWidget('workspace')}
                >
                  <div className="space-y-1">
                    {filteredWorkspace.slice(0, 8).map(item => (
                      <div
                        key={item.path}
                        className="flex items-start justify-between gap-3 rounded-xl border border-border bg-bg px-3 py-2"
                      >
                        <div className="min-w-0">
                          <p className="truncate text-sm font-medium text-tx-1">
                            {item.is_dir ? 'Folder' : 'File'} · {item.name}
                          </p>
                          <p className="mt-0.5 truncate text-[11px] text-tx-4">{item.path}</p>
                        </div>
                        {!item.is_dir && (
                          <div className="shrink-0 text-right text-[11px] text-tx-4">
                            <p>{fmtCount(item.size)} bytes</p>
                            <p>{fmtTime(item.modified)}</p>
                          </div>
                        )}
                      </div>
                    ))}
                    {filteredWorkspace.length === 0 && (
                      <div className="rounded-xl border border-dashed border-border bg-bg/60 px-3 py-4 text-sm text-tx-4">
                        Workspace files will appear here once the agent writes to disk.
                      </div>
                    )}
                    {widgetFilters.workspace && (
                      <div className="mt-2 rounded-xl border border-accent/15 bg-accent-soft px-3 py-2 text-[11px] text-accent">
                        Filter: {widgetFilters.workspace}
                        <button
                          type="button"
                          onClick={() => clearWidgetFilter('workspace')}
                          className="ml-2 font-medium underline decoration-accent/30 underline-offset-2"
                        >
                          Clear
                        </button>
                      </div>
                    )}
                  </div>
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('reviews') && (
              <div key="reviews">
                <WidgetShell
                  icon={Bell}
                  title="Pending reviews"
                  subtitle={`${fmtCount(pendingReviews.length)} waiting`}
                  editing={editing}
                  onRemove={() => toggleWidget('reviews')}
                >
                  <div className="space-y-1">
                    {filteredReviews.slice(0, 6).map(review => (
                      <div key={review.review_id || review.id} className="rounded-xl border border-border bg-bg px-3 py-2.5">
                        <p className="text-sm font-medium text-tx-1">
                          {review.summary || review.title || 'Review item'}
                        </p>
                        <p className="mt-0.5 text-[11px] text-tx-4">
                          {review.reason || review.rule_id || 'Awaiting resolution'}
                        </p>
                      </div>
                    ))}
                    {filteredReviews.length === 0 && (
                      <div className="rounded-xl border border-dashed border-border bg-bg/60 px-3 py-4 text-sm text-tx-4">
                        No pending reviews right now.
                      </div>
                    )}
                    {widgetFilters.reviews && (
                      <div className="mt-2 rounded-xl border border-accent/15 bg-accent-soft px-3 py-2 text-[11px] text-accent">
                        Filter: {widgetFilters.reviews}
                        <button
                          type="button"
                          onClick={() => clearWidgetFilter('reviews')}
                          className="ml-2 font-medium underline decoration-accent/30 underline-offset-2"
                        >
                          Clear
                        </button>
                      </div>
                    )}
                  </div>
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('connectors') && (
              <div key="connectors">
                <WidgetShell
                  icon={Database}
                  title="Connectors"
                  subtitle={`${fmtCount(connectorCount)} data sources`}
                  editing={editing}
                  onRemove={() => toggleWidget('connectors')}
                >
                  <div className="space-y-3">
                    <div className="rounded-2xl border border-border bg-bg px-3 py-3">
                      <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Installed</p>
                      <div className="mt-2 space-y-1">
                        {connectors.slice(0, 3).map(connector => (
                          <ListLine
                            key={connector.id || connector.name}
                            title={connector.name || connector.id || 'Connector'}
                            detail={connector.type || connector.provider || 'Connected'}
                            meta={connector.status || 'live'}
                          />
                        ))}
                        {connectors.length === 0 && <p className="text-sm text-tx-4">No installed connectors.</p>}
                      </div>
                    </div>
                    <div className="rounded-2xl border border-border bg-bg px-3 py-3">
                      <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Custom sources</p>
                      <div className="mt-2 space-y-1">
                        {customConnections.slice(0, 3).map(connector => (
                          <ListLine
                            key={connector.name || connector.id}
                            title={connector.name || connector.id || 'Connection'}
                            detail={toTitle(connector.kind || connector.type || 'connection')}
                            meta={connector.allow_writes ? 'writes on' : 'read only'}
                          />
                        ))}
                        {customConnections.length === 0 && <p className="text-sm text-tx-4">No custom sources configured.</p>}
                      </div>
                    </div>
                  </div>
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('assistant') && (
              <div key="assistant">
                <WidgetShell
                  icon={Sparkles}
                  title="Dashboard assistant"
                  subtitle={activeAction.hint}
                  editing={editing}
                  onRemove={() => toggleWidget('assistant')}
                >
                  <div className="flex h-full flex-col gap-3">
                    <div className="grid gap-2 sm:grid-cols-2">
                      {DASHBOARD_ACTIONS.map(action => (
                        <button
                          key={action.id}
                          type="button"
                          onClick={() => {
                            setActionType(action.id);
                            setAssistantReply(action.hint);
                          }}
                          className={clsx(
                            'rounded-2xl border px-3 py-3 text-left transition-colors',
                            actionType === action.id
                              ? 'border-accent/20 bg-accent-soft text-accent'
                              : 'border-border bg-bg hover:bg-bg-hover',
                          )}
                        >
                          <p className="text-sm font-medium">{action.label}</p>
                          <p className="mt-1 text-[11px] leading-5 text-tx-4">{action.hint}</p>
                        </button>
                      ))}
                    </div>

                    <div className="grid gap-2 sm:grid-cols-2">
                      <div className="rounded-2xl border border-border bg-bg px-3 py-3">
                        <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Target</p>
                        <input
                          value={actionTarget}
                          onChange={e => setActionTarget(e.target.value)}
                          list="dashboard-widget-targets"
                          placeholder="reviews"
                          className="mt-2 w-full border-0 bg-transparent p-0 text-sm text-tx-1 outline-none placeholder:text-tx-5"
                        />
                      </div>
                      <div className="rounded-2xl border border-border bg-bg px-3 py-3">
                        <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">
                          {actionType === 'move_widget' ? 'Direction' : 'Detail'}
                        </p>
                        <input
                          value={actionDetail}
                          onChange={e => setActionDetail(e.target.value)}
                          list="dashboard-action-details"
                          placeholder={actionType === 'move_widget' ? 'right' : 'status:pending'}
                          className="mt-2 w-full border-0 bg-transparent p-0 text-sm text-tx-1 outline-none placeholder:text-tx-5"
                        />
                      </div>
                    </div>

                    <datalist id="dashboard-widget-targets">
                      {WIDGETS.map(widget => (
                        <option key={widget.id} value={widget.id} />
                      ))}
                    </datalist>
                    <datalist id="dashboard-action-details">
                      <option value="up" />
                      <option value="down" />
                      <option value="left" />
                      <option value="right" />
                      <option value="status:pending" />
                      <option value="role:ops" />
                    </datalist>

                    <div className="flex flex-wrap gap-2">
                      <button
                        type="button"
                        onClick={applyDashboardAction}
                        className="inline-flex items-center gap-2 rounded-full border border-accent/20 bg-gradient-to-r from-accent to-accent-text px-3 py-2 text-xs font-medium text-white shadow-[0_10px_24px_rgba(201,106,46,0.16)] transition-transform hover:-translate-y-0.5"
                      >
                        Apply action
                        <ArrowRight size={12} />
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          setActionType('add_widget');
                          setActionTarget('reviews');
                          setActionDetail('');
                          setAssistantReply('Ready to add the reviews widget.');
                        }}
                        className="inline-flex items-center gap-2 rounded-full border border-border bg-bg px-3 py-2 text-xs font-medium text-tx-2 transition-colors hover:bg-bg-hover"
                      >
                        Add reviews
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          setActionType('change_widget_filter');
                          setActionTarget('runs');
                          setActionDetail('pending');
                          setAssistantReply('Ready to filter the runs widget.');
                        }}
                        className="inline-flex items-center gap-2 rounded-full border border-border bg-bg px-3 py-2 text-xs font-medium text-tx-2 transition-colors hover:bg-bg-hover"
                      >
                        Filter runs
                      </button>
                    </div>

                    <div className="rounded-2xl border border-border bg-bg px-3 py-3">
                      <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Assistant state</p>
                      <p className="mt-2 text-sm leading-6 text-tx-2">{assistantReply}</p>
                    </div>

                    <div className="rounded-2xl border border-border bg-bg px-3 py-3">
                      <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Recent actions</p>
                      <div className="mt-2 space-y-1">
                        {actionLog.slice(0, 4).map(entry => (
                          <div key={`${entry.action}-${entry.at}`} className="flex items-start justify-between gap-3 rounded-xl border border-border/70 bg-bg-card px-3 py-2">
                            <div className="min-w-0">
                              <p className="truncate text-sm font-medium text-tx-1">{toTitle(entry.action)}</p>
                              <p className="mt-0.5 text-[11px] text-tx-4">
                                {entry.target}{entry.detail ? ` · ${entry.detail}` : ''}
                              </p>
                            </div>
                            <span className="shrink-0 text-[11px] text-tx-4">{fmtTime(entry.at)}</span>
                          </div>
                        ))}
                        {actionLog.length === 0 && <p className="text-sm text-tx-4">No actions yet.</p>}
                      </div>
                    </div>
                  </div>
                </WidgetShell>
              </div>
            )}

            {widgetIds.includes('savings') && (
              <div key="savings">
                <div className="h-full">
                  <SavingsCard className="h-full" />
                </div>
              </div>
            )}

            {widgetIds.includes('health') && (
              <div key="health">
                <WidgetShell
                  icon={ShieldCheck}
                  title="System health"
                  subtitle="Current runtime state"
                  editing={editing}
                  onRemove={() => toggleWidget('health')}
                  rightSlot={swarm ? <ChevronDown size={13} className="text-tx-4" /> : null}
                >
                  <div className="grid gap-3 md:grid-cols-3">
                    <StatBlock label="Pending reviews" value={fmtCount(pendingReviews.length)} hint="Needs attention" />
                    <StatBlock label="Active agents" value={fmtCount(activeAgentCount)} hint="Healthy and working" />
                    <StatBlock label="Data sources" value={fmtCount(connectorCount)} hint="Connectors and custom sources" />
                  </div>
                  {swarm && (
                    <div className="mt-4 rounded-2xl border border-border bg-bg px-3 py-3">
                      <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Swarm status</p>
                      <p className="mt-2 text-sm leading-6 text-tx-2">
                        {typeof swarm === 'string'
                          ? swarm
                          : swarm.status || swarm.message || 'Operational'}
                      </p>
                    </div>
                  )}
                </WidgetShell>
              </div>
            )}
          </ResponsiveGridLayout>
        </div>

        <AnimatePresence>
          {!editing && (
            <motion.div
              initial={{ opacity: 0, y: 16 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 16 }}
              className="pointer-events-none fixed bottom-5 right-5 max-w-sm rounded-2xl border border-border bg-bg-card/95 px-4 py-3 shadow-[0_24px_60px_rgba(19,17,13,0.12)] backdrop-blur"
            >
              <p className="text-[11px] uppercase tracking-[0.22em] text-tx-4">Dashboard mode</p>
              <p className="mt-1 text-sm leading-6 text-tx-2">
                Turn on layout editing to drag widgets, then save automatically in this browser.
              </p>
            </motion.div>
          )}
        </AnimatePresence>
      </main>
    </div>
  );
}
