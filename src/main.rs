use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use chrono::{Datelike, Timelike};
use tokio::sync::{Mutex, RwLock};

mod agent;
mod api;
mod audit;
mod auth;
mod billing;
mod browser;
mod cognition;
mod compliance;
mod config;
mod connectors;
mod debug;
mod events;
mod gateway;
mod knowledge;
mod memory;
mod metrics;
mod policy;
mod providers;
mod scheduler;
mod segments;
mod skill_evolution;
mod skill_marketplace;
mod skills;
mod state;
mod storage;
mod swarm;
mod tenant;
mod tools;
mod util;
mod webhooks;
mod worker;
mod workspace;

use agent::{AgentLoop, AgentManager, LlmClarifier, LlmEvaluator, LlmExecutor, LlmPreflight, LlmReflector};
use api::{routes::AppState, server::serve};
use audit::AuditLog;
use billing::{paypal::PayPalProvider, stripe::StripeProvider, BillingStore};
use browser::{BrowserPool, BrowserPoolConfig};
use config::AppConfig;
use connectors::{ConnectorInstallStore, ConnectorPoller};
use events::EventBus;
use gateway::{CostTracker, LlmGateway, NarayanGateway, ProviderLimits, RateLimiter, ResponseCache};
use knowledge::KnowledgeGraph;
use memory::{
    build_embedding_model, store::RedisMemoryStore, DistanceMetric, MemoryConsolidator, PgVectorStore, VectorStore,
};
use metrics::Metrics;
use scheduler::{DbPollingScheduler, InMemoryQueue, Queue, RedisBackedQueue, ScheduleTicker, Scheduler};
use skill_marketplace::SkillMarketplace;
use skills::registry::SkillRegistry;
use storage::PostgresStore;
use swarm::Swarm;
use tenant::TenantStore;
use tools::DelegateTool;
use worker::WorkerPool;
use workspace::manager::WorkspaceManager;

const DEFAULT_EMBED_PROVIDER: &str = "stub";
const DEFAULT_BROWSER_POOL_SIZE: usize = 4;
const KNOWN_PROVIDERS: [&str; 12] = [
    "anthropic",
    "openai",
    "groq",
    "gemini",
    "nvidia",
    "ollama",
    "openrouter",
    "copilot",
    "glm",
    "novita",
    "sglang",
    "compatible",
];

fn read_embed_config() -> (String, String, Option<String>) {
    let provider = std::env::var("NARAYAN_EMBED_PROVIDER").unwrap_or_else(|_| DEFAULT_EMBED_PROVIDER.into());
    let api_key = std::env::var("NARAYAN_EMBED_API_KEY").unwrap_or_default();
    let model = std::env::var("NARAYAN_EMBED_MODEL").ok();
    (provider, api_key, model)
}

fn parse_browser_pool_size(raw: Option<&str>) -> usize {
    raw.and_then(|s| s.parse::<usize>().ok()).unwrap_or(DEFAULT_BROWSER_POOL_SIZE)
}

fn browser_pool_size_from_env() -> usize {
    parse_browser_pool_size(std::env::var("NARAYAN_BROWSER_POOL_SIZE").ok().as_deref())
}

fn build_rate_limits(requests_per_sec: f64) -> HashMap<String, ProviderLimits> {
    KNOWN_PROVIDERS
        .iter()
        .map(|name| ((*name).to_string(), ProviderLimits { requests_per_sec, burst: requests_per_sec * 3.0 }))
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "narayan=info,tower_http=info".into()),
        )
        .init();

    tracing::info!("Narayan starting — BYOK mode");

    let cfg = AppConfig::load()?;

    let jwt_secret = std::env::var("NARAYAN_JWT_SECRET").expect("NARAYAN_JWT_SECRET must be set");
    let encrypt_key = std::env::var("NARAYAN_ENCRYPT_KEY").expect("NARAYAN_ENCRYPT_KEY must be set");

    // ── Infrastructure ─────────────────────────────────────────────────────
    let metrics = Arc::new(Metrics::new());
    let event_bus = Arc::new(EventBus::new());

    let store = Arc::new(PostgresStore::new(&cfg.database.url, cfg.database.max_connections).await?);
    store.migrate().await?;

    let tenant_store = Arc::new(TenantStore::new(store.pool()));
    tenant_store.migrate().await?;

    let audit_log = Arc::new(AuditLog::new(store.pool()));
    audit_log.migrate().await?;

    let webhook_store = Arc::new(webhooks::WebhookStore::new(store.pool()));
    webhook_store.migrate().await?;
    let webhook_dispatcher = Arc::new(webhooks::WebhookDispatcher::new(webhook_store.clone()));

    let citation_tracker = Arc::new(compliance::CitationTracker::new(store.pool()));
    citation_tracker.migrate().await?;
    let review_queue = Arc::new(compliance::ReviewQueue::new(store.pool()));
    review_queue.migrate().await?;

    // ── Connector install store (built early so McpSessionTool can reference it) ──
    let connector_installs = {
        let s = Arc::new(ConnectorInstallStore::new(store.pool(), encrypt_key.clone()));
        s.migrate().await?;
        s
    };

    // ── Segment plugin system ───────────────────────────────────────────────
    // Build shared dependencies once — all segment plugins share these instances.
    let shared_deps = segments::SharedDeps {
        policy_engine: Arc::new(policy::PolicyEngine::new()),
        citation_tracker: citation_tracker.clone(),
        review_queue: review_queue.clone(),
        evidence_packager: Arc::new(compliance::EvidencePackager::new(citation_tracker.clone(), audit_log.clone())),
        pii_redactor: Arc::new(compliance::PiiRedactor::new()),
    };

    // ── Active segments — add or remove plugins here per deployment ─────────
    // Each plugin contributes: connectors, services, policy rules, SLA policies.
    // Multiple segments can be active simultaneously — services are merged (union).
    // tenant_id used for SLA policy scoping — replace with actual tenant in multi-tenant setup.
    let tenant_id = "default";
    let segment_registry = segments::SegmentRegistry::builder()
        .add(segments::engineering::plugin(&shared_deps, tenant_id))
        .add(segments::customer_support::plugin(&shared_deps, tenant_id))
        .add(segments::compliance_ops::plugin(&shared_deps, tenant_id))
        .add(segments::sales_revops::plugin(&shared_deps, tenant_id))
        .add(segments::finance_accounting::plugin(&shared_deps, tenant_id))
        .add(segments::hr_people_ops::plugin(&shared_deps, tenant_id))
        .add(segments::legal_contract::plugin(&shared_deps, tenant_id))
        .add(segments::it_ops_itsm::plugin(&shared_deps, tenant_id))
        .add(segments::research_intelligence::plugin(&shared_deps, tenant_id))
        .add(segments::data_analytics::plugin(&shared_deps, tenant_id))
        .add(segments::marketing_growth::plugin(&shared_deps, tenant_id))
        .add(segments::procurement_vendor_ops::plugin(&shared_deps, tenant_id))
        .add(segments::security_ops_grc::plugin(&shared_deps, tenant_id))
        .add(segments::customer_success_renewals::plugin(&shared_deps, tenant_id))
        .add(segments::brand_protection::plugin(&shared_deps, tenant_id))
        .build();

    let agent_services = Arc::new(segment_registry.agent_services());

    tracing::info!(
        segments = segment_registry.domain_profiles().len(),
        domains = segment_registry.domain_profiles().len(),
        connectors = segment_registry.connector_registry.list().len(),
        "segment plugins loaded"
    );
    tracing::info!("database ready (audit, webhooks, compliance, segments enabled)");

    // ── Queue ──────────────────────────────────────────────────────────────
    let queue: Arc<dyn Queue> = if cfg.redis.enabled {
        match RedisBackedQueue::new(&cfg.redis.url) {
            Ok(q) => {
                tracing::info!("using Redis queue");
                Arc::new(q)
            }
            Err(e) => {
                tracing::warn!(error=%e, "Redis unavailable — using in-memory queue");
                Arc::new(InMemoryQueue::new())
            }
        }
    } else {
        Arc::new(InMemoryQueue::new())
    };

    // ── Agent memory store ─────────────────────────────────────────────────
    // RedisMemoryStore is the production default — survives restarts, visible
    // across all Narayan instances.  Falls back to InMemoryStore (dev only)
    // when Redis is disabled or unavailable.
    //
    // Memory is keyed as:  HSET narayan:mem:{agent_id} {key} {value}
    // TTL of 7 days keeps stale agent memory from accumulating indefinitely.
    let _memory_store: Arc<dyn memory::store::MemoryStore> = if cfg.redis.enabled {
        match RedisMemoryStore::new(&cfg.redis.url) {
            Ok(s) => {
                tracing::info!("agent memory: Redis-backed (durable, multi-instance)");
                Arc::new(s.with_ttl(60 * 60 * 24 * 7)) // 7-day TTL
            }
            Err(e) => {
                tracing::warn!(error=%e, "Redis unavailable — agent memory is in-process only (lost on restart)");
                Arc::new(memory::store::InMemoryStore::new())
            }
        }
    } else {
        tracing::warn!("Redis disabled — agent memory is in-process only (lost on restart)");
        Arc::new(memory::store::InMemoryStore::new())
    };

    // ── Swarm coordinator ──────────────────────────────────────────────────
    // Replaces the old global static Mutex<SwarmScheduler>.
    // Uses the same Arc<dyn Queue> as the WorkerPool — no separate lock,
    // durable via Redis, works across multiple Narayan instances.
    let swarm = Arc::new(Swarm::new(queue.clone()));
    tracing::info!("swarm coordinator ready (queue-backed)");

    // ── pgvector Store + Embedding Model ─────────────────────────────────────
    // Reads NARAYAN_EMBED_PROVIDER, NARAYAN_EMBED_API_KEY, NARAYAN_EMBED_MODEL
    // from env. Falls back to in-memory stub if not configured.
    let (embed_provider, embed_api_key, embed_model) = read_embed_config();

    let embedder: std::sync::Arc<dyn memory::EmbeddingModel> =
        std::sync::Arc::from(build_embedding_model(&embed_provider, &embed_api_key, embed_model.as_deref()));

    let vector_store = PgVectorStore::new(store.pool(), embedder.dimension(), DistanceMetric::Cosine);
    if let Err(e) = vector_store.migrate().await {
        tracing::warn!(error = %e, "pgvector migration failed — vector tools disabled (install pgvector extension)");
    } else {
        tracing::info!(
            provider   = %embed_provider,
            dimensions = embedder.dimension(),
            "pgvector store ready"
        );
    }

    // ── Workspace Manager ──────────────────────────────────────────────────
    // Reads NARAYAN__WORKSPACE__* env vars via AppConfig.
    // Automatically selects Local / Remote / Hybrid based on disk usage.
    tokio::fs::create_dir_all(&cfg.workspace.local_root).await?;
    let workspace_manager = Arc::new(WorkspaceManager::new(cfg.workspace.clone(), store.clone())?);
    tracing::info!(
        mode              = %cfg.workspace.mode,
        local_root        = %cfg.workspace.local_root,
        disk_threshold    = cfg.workspace.disk_threshold_pct,
        has_remote_storage = workspace_manager.has_remote_storage(),
        "workspace manager ready"
    );

    // ── Background workspace cleanup ───────────────────────────────────────
    {
        let wm = workspace_manager.clone();
        let hours = cfg.workspace.cleanup_after_hours;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(3600));
            loop {
                ticker.tick().await;
                if let Err(e) = wm.cleanup_old(hours).await {
                    tracing::error!(error = %e, "workspace cleanup failed");
                }
            }
        });
    }

    // ── Provider registry (BYOK) ───────────────────────────────────────────
    let fallback_providers: HashMap<String, Arc<dyn providers::Provider>> = HashMap::new();

    // ── LLM Gateway ────────────────────────────────────────────────────────
    let cache = Arc::new(ResponseCache::new(cfg.gateway.cache_ttl_secs, cfg.gateway.cache_max_entries));
    let cost_tracker = Arc::new(CostTracker::new(CostTracker::default_pricing()));
    // Load per-tenant step counts so plan enforcement survives restarts
    metrics.load_steps_from_db(&store.pool()).await;
    let rate_limits = build_rate_limits(cfg.gateway.requests_per_sec);
    let rate_limiter = Arc::new(RateLimiter::new(rate_limits));
    let gateway: Arc<dyn LlmGateway> = Arc::new(
        NarayanGateway::new(
            tenant_store.clone(),
            encrypt_key.clone(),
            cache,
            cost_tracker.clone(),
            rate_limiter,
            fallback_providers,
        )
        .with_event_bus(event_bus.clone()),
    );
    tracing::info!("LLM gateway ready (BYOK)");
    let memory_consolidator =
        MemoryConsolidator::new(gateway.clone(), vector_store.clone() as Arc<dyn VectorStore>, embedder.clone())
            .with_store(store.clone());

    // ── Browser Pool ───────────────────────────────────────────────────────
    // Shared Chromium instances — all browser tools borrow from here.
    // Set NARAYAN_BROWSER_POOL_SIZE to 0 to disable headless browser support.
    let browser_pool_size = browser_pool_size_from_env();
    let browser_pool: Option<std::sync::Arc<BrowserPool>> = if browser_pool_size > 0 {
        let pool_cfg = BrowserPoolConfig {
            size: browser_pool_size,
            headless: true,
            nav_timeout_ms: 30_000,
            viewport_width: 1440,
            viewport_height: 900,
        };
        match BrowserPool::new(pool_cfg).await {
            Ok(p) => {
                tracing::info!(pool_size = browser_pool_size, "browser pool ready");
                Some(p)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Chromium unavailable — browser tools disabled. Install chromium or set NARAYAN_BROWSER_POOL_SIZE=0");
                None
            }
        }
    } else {
        tracing::info!("browser pool disabled (NARAYAN_BROWSER_POOL_SIZE=0)");
        None
    };

    // ── Tools ──────────────────────────────────────────────────────────────
    let mut tool_registry = tools::default_registry();
    // DelegateTool gets the swarm so child agents are enqueued via Arc<Swarm>
    // instead of the old global crate::swarm::push() free function.
    tool_registry.register(Arc::new(DelegateTool::new(store.clone(), workspace_manager.clone(), swarm.clone())));
    tool_registry.register(Arc::new(tools::send_message::SendMessageTool::new(store.clone(), event_bus.clone())));
    tool_registry.register(Arc::new(tools::message_inbox::MessageInboxTool::new(
        store.clone(),
        swarm.clone(),
        event_bus.clone(),
    )));
    tool_registry.register(Arc::new(tools::session_tasks::TaskCreateTool::new(store.clone())));
    tool_registry.register(Arc::new(tools::session_tasks::TaskGetTool::new(store.clone())));
    tool_registry.register(Arc::new(tools::session_tasks::TaskListTool::new(store.clone())));
    tool_registry.register(Arc::new(tools::session_tasks::TaskUpdateTool::new(store.clone())));
    tool_registry.register(Arc::new(tools::session_tasks::TaskStopTool::new(store.clone())));
    tool_registry.register(Arc::new(tools::session_tasks::TaskOutputTool::new(store.clone())));
    // Memory tools removed (module files not present)
    // tool_registry.register(Arc::new(tools::memory_store::MemoryStoreTool));
    // tool_registry.register(Arc::new(tools::memory_consolidate::MemoryConsolidateTool::new(
    //     store.clone(),
    //     memory_consolidator.clone(),
    // )));
    // tool_registry.register(Arc::new(tools::memory_recall::MemoryRecallTool));
    // tool_registry.register(Arc::new(tools::memory_forget::MemoryForgetTool));
    // Vector tools removed (module files not present)
    // {
    //     let vs = vector_store.clone();
    //     let emb = embedder.clone();
    //     tool_registry.register(std::sync::Arc::new(tools::vector_store::VectorStoreTool {
    //         store: vs.clone(),
    //         embedder: emb.clone(),
    //     }));
    //     tool_registry.register(std::sync::Arc::new(tools::vector_search::VectorSearchTool {
    //         store: vs.clone(),
    //         embedder: emb.clone(),
    //     }));
    //     tool_registry.register(std::sync::Arc::new(tools::vector_delete::VectorDeleteTool { store: vs.clone() }));
    //     tracing::info!("vector tools registered (store, search, delete)");
    // }
    // Register browser tools if pool is available
    if let Some(ref bp) = browser_pool {
        tool_registry.register(Arc::new(tools::browser::BrowserTool { pool: bp.clone() }));
        tool_registry.register(Arc::new(tools::screenshot::ScreenshotTool { pool: bp.clone() }));
        tool_registry.register(Arc::new(tools::browser_interact::BrowserInteractTool { pool: bp.clone() }));
        tool_registry.register(Arc::new(tools::browser_pdf::BrowserPdfTool { pool: bp.clone() }));
        tool_registry.register(Arc::new(tools::browser_network::BrowserNetworkTool { pool: bp.clone() }));
        tracing::info!("browser tools registered (5 tools with Chromium pool)");
    }
    // Wire connector install store into McpSessionTool for auto token injection
    tool_registry
        .register(Arc::new(tools::mcp_session::McpSessionTool::new().with_install_store(connector_installs.clone())));

    // Wire stores into external DB and API tools
    tool_registry
        .register(Arc::new(tools::external_db::ExternalDbTool::with_install_store(connector_installs.clone())));
    tool_registry.register(Arc::new(
        tools::external_api::ExternalApiTool::new().with_stores(connector_installs.clone(), store.clone()),
    ));
    // tool_registry
    //     .register(Arc::new(tools::run_registered_wasm::RunRegisteredWasmTool::new().with_store(store.clone())));

    // Re-register all built-in connector tools with the install store so stored
    // OAuth tokens / API keys are injected automatically into mcp_session calls.
    // This overwrites the no-store versions registered by default_registry().
    tools::connector_tool::register_all_connectors(&mut tool_registry, Some(connector_installs.clone()));
    tracing::info!(
        "{} connector tools registered with OAuth token injection",
        tools::connector_tool::ALL_CONNECTORS.len()
    );

    let tool_registry = Arc::new(tool_registry);
    tracing::info!("{} tools registered", tool_registry.list().len());

    // ── Capability systems ─────────────────────────────────────────────────
    let skill_registry: Arc<RwLock<SkillRegistry>> = Arc::new(RwLock::new(SkillRegistry::with_curated_defaults()));
    let marketplace: Arc<Mutex<SkillMarketplace>> = Arc::new(Mutex::new(SkillMarketplace::new()));
    let knowledge_graph: Arc<Mutex<KnowledgeGraph>> = Arc::new(Mutex::new(KnowledgeGraph::new()));

    // ── Agent runtime ──────────────────────────────────────────────────────
    let executor = Arc::new(
        LlmExecutor::new(gateway.clone(), tool_registry.clone(), agent_services.clone())
            .with_tenant_store(tenant_store.clone())
            .with_event_bus(event_bus.clone())
            .with_store(store.clone()),
    );
    let evaluator = Arc::new(LlmEvaluator::new(gateway.clone()));
    let reflector = Arc::new(LlmReflector::new(gateway.clone()));
    let preflight = Arc::new(LlmPreflight::new(gateway.clone()));
    let clarifier = Arc::new(LlmClarifier::new(gateway.clone()));

    // ── Workflow Store (needed by both AgentLoop and DagEngine) ─────────
    let pg_workflow_store = crate::storage::dag_store::PgWorkflowStore::new(store.pool().clone());
    pg_workflow_store.migrate().await?;
    let workflow_store: Arc<dyn crate::storage::WorkflowStore> = Arc::new(pg_workflow_store);

    let agent_loop = Arc::new(
        AgentLoop::new(
            executor.clone(),
            evaluator,
            reflector,
            preflight,
            clarifier,
            tool_registry,
            event_bus.clone(),
            skill_registry.clone(),
            knowledge_graph.clone(),
            vector_store.clone(),
            embedder.clone(),
            agent_services.clone(),
        )
        .with_store(Arc::clone(&store))
        .with_memory_consolidator(memory_consolidator.clone())
        .with_workflow_store(workflow_store.clone())
        .with_limits(100, 3_600),
    );

    // AgentManager uses WorkspaceManager to create workspaces
    // agent_services provides SLA start-on-create per job type
    let manager =
        Arc::new(AgentManager::new(store.clone(), workspace_manager.clone(), agent_services.clone(), gateway.clone()));

    // ── Scheduler ──────────────────────────────────────────────────────────
    let scheduler = Arc::new(DbPollingScheduler::new(
        store.clone(),
        queue.clone(),
        event_bus.clone(),
        cfg.scheduler.poll_interval_ms,
        cfg.scheduler.max_batch_size,
    ));

    // ── Worker pool ────────────────────────────────────────────────────────
    // ── Billing ────────────────────────────────────────────────────────────
    let billing = {
        let mut store = BillingStore::new(store.pool());
        if let Some(paypal) = PayPalProvider::from_env() {
            tracing::info!("PayPal billing provider registered");
            store = store.register(Arc::new(paypal));
        }
        if let Some(stripe) = StripeProvider::from_env() {
            tracing::info!("Stripe billing provider registered");
            store = store.register(Arc::new(stripe));
        }
        let store = Arc::new(store);
        store.migrate().await?;
        store
    };

    // ── DAG Engine (shares workflow_store created above) ───────────────
    let dag_orchestrator = Arc::new(
        crate::agent::orchestrator::StepOrchestrator::new(
            executor.clone(),
            event_bus.clone(),
            knowledge_graph.clone(),
            agent_services.clone(),
            vector_store.clone(),
            embedder.clone(),
        )
        .with_store(Arc::clone(&store)),
    );
    let dag_engine = Arc::new(
        crate::agent::dag_engine::DagEngine::new(executor.clone(), workflow_store.clone(), event_bus.clone())
            .with_orchestrator(dag_orchestrator),
    );

    // ── Worker pool ────────────────────────────────────────────────────────
    let pool = Arc::new(WorkerPool::new(
        cfg.worker.pool_size,
        cfg.worker.node_name.clone(),
        store.clone(),
        queue.clone(),
        agent_loop,
        dag_engine,
        metrics.clone(),
        workspace_manager.clone(),
        agent_services.clone(),
        event_bus.clone(),
    ));

    // ── Connector poller ───────────────────────────────────────────────────
    let connector_poller = Arc::new(ConnectorPoller::new(connector_installs.clone(), manager.clone()));

    // ── Schedule ticker (cron-based role triggers) ───────────────────────
    let schedule_ticker = Arc::new(ScheduleTicker::new(store.clone(), manager.clone()));

    // ── API ────────────────────────────────────────────────────────────────
    let app_state = AppState {
        store: store.clone(),
        tenant_store: tenant_store.clone(),
        manager,
        cost_tracker: cost_tracker.clone(),
        metrics: metrics.clone(),
        jwt_secret,
        encrypt_key: encrypt_key.clone(),
        skill_registry,
        marketplace,
        audit_log: audit_log.clone(),
        webhook_store: webhook_store.clone(),
        webhook_dispatcher: webhook_dispatcher.clone(),
        review_queue: review_queue.clone(),
        swarm: swarm.clone(),
        connector_registry: Arc::new(segment_registry.connector_registry),
        citation_tracker: Some(citation_tracker.clone()),
        auto_approvals: {
            let s = Arc::new(crate::api::routes::AutoApprovalStore::new(store.pool()));
            s.migrate().await.unwrap_or_else(|e| tracing::warn!(error = %e, "auto_approvals migrate failed"));
            s
        },
        event_bus_handle: event_bus.clone(),
        billing,
        connector_installs,
    };

    tokio::spawn(Metrics::run_window_reset(metrics.clone()));

    // ── Workforce Event Dispatcher ─────────────────────────────────────────
    // Listen for RoleCompleted events and dispatch to dependent WorkforceEvent roles
    {
        let _store = store.clone();
        let _event_bus = event_bus.clone();

        // For now, the dispatcher is called directly when role completes.
        // No separate background task needed—the event is published and can be handled
        // by subscribers. In a future enhancement, could add a proper event subscription
        // mechanism here.
        tracing::info!("workforce event dispatcher armed (waits for RoleCompleted events)");
    }

    // Reset monthly step counters at midnight on the 1st of each month
    {
        let m = metrics.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // check hourly
            loop {
                ticker.tick().await;
                let now = chrono::Utc::now();
                if now.day() == 1 && now.hour() == 0 {
                    m.reset_monthly_steps();
                    tracing::info!("monthly step counters reset");
                }
            }
        });
    }

    // ── Shutdown orchestration ──────────────────────────────────────────
    let cancel_token = tokio_util::sync::CancellationToken::new();
    let shutdown_token = cancel_token.clone();

    tracing::info!(
        workers = cfg.worker.pool_size,
        port = cfg.server.port,
        memory = if cfg.redis.enabled { "redis" } else { "in-process" },
        swarm = swarm.is_queue_backed(),
        "all systems go 🚀"
    );

    tokio::select! {
        _ = tokio::signal::ctrl_c()  => {
            tracing::info!("shutdown signal received — draining workers");
            shutdown_token.cancel();
            // Wait a moment for workers to ack/requeue
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        },
        r = scheduler.run()          => tracing::error!("scheduler exited: {:?}", r),
        r = pool.run(cancel_token)   => tracing::error!("worker pool exited: {:?}", r),
        _ = connector_poller.run()   => tracing::error!("connector poller exited"),
        _ = schedule_ticker.run()    => tracing::error!("schedule ticker exited"),
        r = serve(
            app_state, tenant_store, event_bus, store,
            audit_log, cost_tracker, metrics,
            &cfg.server.host, cfg.server.port
        )                            => tracing::error!("API exited: {:?}", r),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_browser_pool_size_uses_default_for_missing_or_invalid_values() {
        assert_eq!(parse_browser_pool_size(None), DEFAULT_BROWSER_POOL_SIZE);
        assert_eq!(parse_browser_pool_size(Some("not-a-number")), DEFAULT_BROWSER_POOL_SIZE);
    }

    #[test]
    fn test_parse_browser_pool_size_respects_zero_and_positive_values() {
        assert_eq!(parse_browser_pool_size(Some("0")), 0);
        assert_eq!(parse_browser_pool_size(Some("9")), 9);
    }

    #[test]
    fn test_build_rate_limits_includes_all_known_providers() {
        let requests_per_sec = 7.5;
        let rate_limits = build_rate_limits(requests_per_sec);

        assert_eq!(rate_limits.len(), KNOWN_PROVIDERS.len());
        for provider in KNOWN_PROVIDERS {
            let limits = rate_limits.get(provider).unwrap_or_else(|| panic!("missing provider {provider}"));
            assert!((limits.requests_per_sec - requests_per_sec).abs() < f64::EPSILON);
            assert!((limits.burst - (requests_per_sec * 3.0)).abs() < f64::EPSILON);
        }
    }
}
