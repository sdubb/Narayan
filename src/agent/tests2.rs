use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::{sync::RwLock, time::sleep};

use crate::{
    agent::{
        evaluator::EvalVerdict,
        executor::LlmExecutor,
        preflight::PreflightResult,
        planner::Plan,
        prompts::StepHistory,
        r#loop::{AgentLoop, StepOutcome},
        test_helpers::*,
        PlanModeManager, PlanModePhase, PlanModeSession, PlanModeTestStatus,
    },
    connectors::ConnectorInstallStore,
    events::EventBus,
    gateway::{GatewayRequest, LlmGateway},
    knowledge::graph::KnowledgeGraph,
    memory::{embeddings::StubEmbeddingModel, vector::InMemoryVectorStore},
    providers::{build_provider, ChatResponse, Message, Provider, ToolSpec},
    segments::AgentServices,
    state::{AgentState, AgentStatus},
    skills::registry::SkillRegistry,
    storage::PostgresStore,
    tools::default_registry,
};

#[derive(Debug, Default)]
struct E2eTranscript {
    lines: Vec<String>,
}

impl E2eTranscript {
    fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    fn render(&self) -> String {
        self.lines.join("\n")
    }
}

fn log_block(transcript: &Arc<Mutex<E2eTranscript>>, title: &str, body: impl Into<String>) {
    let block = format!("\n=== {} ===\n{}", title, body.into());
    println!("{block}");
    transcript.lock().unwrap().push(block);
}

#[derive(Debug, Clone)]
struct PlanModeWebE2EConfig {
    database_url: String,
    encrypt_key: String,
    groq_api_key: String,
    groq_model: String,
    tenant_id: String,
    agent_name: String,
    intent: String,
    max_repair_rounds: usize,
}

impl PlanModeWebE2EConfig {
    fn from_env() -> Result<Self> {
        let _ = dotenv::dotenv();

        let database_url = env::var("DATABASE_URL")
            .or_else(|_| env::var("NARAYAN__DATABASE__URL"))
            .context("DATABASE_URL or NARAYAN__DATABASE__URL is required for the plan mode web e2e test")?;
        let groq_api_key = env::var("GROQ_API_KEY").context("GROQ_API_KEY is required for the plan mode web e2e test")?;
        let encrypt_key = env::var("NARAYAN_ENCRYPT_KEY").unwrap_or_default();
        let groq_model = env::var("GROQ_MODEL").unwrap_or_else(|_| "openai/gpt-oss-120b".into());
        let tenant_id = env::var("PLAN_MODE_WEB_E2E_TENANT_ID")
            .unwrap_or_else(|_| format!("plan-mode-web-e2e-{}", uuid::Uuid::new_v4().simple()));
        let agent_name = env::var("PLAN_MODE_WEB_E2E_AGENT_NAME")
            .unwrap_or_else(|_| format!("Plan Mode Web E2E Agent {}", uuid::Uuid::new_v4().simple()));
        let intent = env::var("PLAN_MODE_WEB_E2E_INTENT").unwrap_or_else(|_| {
            "Create a read-only plan-mode agent that must use web_search_tool to search the public web for the official Rust book homepage and summarize the result in chat. Do not use databases."
                .into()
        });
        let max_repair_rounds =
            env::var("PLAN_MODE_WEB_E2E_MAX_REPAIR_ROUNDS").ok().and_then(|value| value.parse::<usize>().ok()).unwrap_or(3);

        Ok(Self {
            database_url,
            encrypt_key,
            groq_api_key,
            groq_model,
            tenant_id,
            agent_name,
            intent,
            max_repair_rounds,
        })
    }
}

struct GroqLoggingGateway {
    provider: Arc<dyn Provider>,
    transcript: Arc<Mutex<E2eTranscript>>,
}

impl GroqLoggingGateway {
    fn new(provider: Arc<dyn Provider>, transcript: Arc<Mutex<E2eTranscript>>) -> Self {
        Self { provider, transcript }
    }
}

async fn record_chat(
    label: &str,
    provider: &Arc<dyn Provider>,
    transcript: &Arc<Mutex<E2eTranscript>>,
    messages: Vec<Message>,
    tools: Vec<ToolSpec>,
) -> Result<ChatResponse> {
    let mut request_block = String::new();
    request_block.push_str("messages:\n");
    for (index, message) in messages.iter().enumerate() {
        request_block.push_str(&format!("[{}] {:?}: {}\n", index, message.role, message.content));
    }
    if tools.is_empty() {
        request_block.push_str("tools: <none>\n");
    } else {
        request_block.push_str("tools:\n");
        request_block.push_str(&serde_json::to_string_pretty(&tools).unwrap_or_else(|_| "<unprintable tools>".into()));
        request_block.push('\n');
    }
    log_block(transcript, &format!("{} REQUEST", label), request_block);

    let mut attempt = 0usize;
    let response = loop {
        attempt += 1;
        match provider.chat(messages.clone(), tools.clone(), None).await {
            Ok(response) => break response,
            Err(error) => {
                let error_text = error.to_string();
                if attempt < 4 {
                    if let Some(delay) = groq_retry_delay_from_text(&error_text) {
                        log_block(
                            transcript,
                            &format!("{} RETRY", label),
                            format!("rate limited; retrying after {:?} (attempt {})", delay, attempt + 1),
                        );
                        sleep(delay).await;
                        continue;
                    }
                }
                return Err(error).with_context(|| format!("{} provider chat failed", label));
            }
        }
    };

    let mut response_block = String::new();
    response_block.push_str(&format!("content: {}\n", response.content.as_deref().unwrap_or("<none>")));
    if response.tool_calls.is_empty() {
        response_block.push_str("tool_calls: <none>\n");
    } else {
        response_block.push_str("tool_calls:\n");
        response_block.push_str(
            &serde_json::to_string_pretty(&response.tool_calls).unwrap_or_else(|_| "<unprintable tool calls>".into()),
        );
        response_block.push('\n');
    }
    response_block
        .push_str(&format!("usage: input_tokens={}, output_tokens={}", response.input_tokens, response.output_tokens));
    log_block(transcript, &format!("{} RESPONSE", label), response_block);

    Ok(response)
}

fn groq_retry_delay_from_text(text: &str) -> Option<Duration> {
    if !(text.contains("status=429") || text.contains("Too Many Requests") || text.contains("rate_limit_exceeded")) {
        return None;
    }

    let re = regex::Regex::new(r"try again in ([0-9]+(?:\.[0-9]+)?)s").ok()?;
    let secs = re
        .captures(text)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
        .unwrap_or(5.0)
        .max(1.0)
        .min(30.0);
    Some(Duration::from_secs_f64(secs))
}

#[async_trait]
impl LlmGateway for GroqLoggingGateway {
    async fn chat(&self, request: GatewayRequest) -> Result<ChatResponse> {
        let meta = format!(
            "agent_id={}\ntenant_id={}\ncomplexity={:?}\nbypass_cache={}",
            request.agent_id, request.tenant_id, request.complexity, request.bypass_cache
        );
        log_block(&self.transcript, "PLAN MODE GATEWAY", meta);
        record_chat("PLAN MODE GATEWAY", &self.provider, &self.transcript, request.messages, request.tools).await
    }
}

fn session_snapshot(session: &PlanModeSession) -> serde_json::Value {
    serde_json::json!({
        "id": session.id,
        "tenant_id": session.tenant_id,
        "phase": session.phase,
        "goal_fingerprint": session.goal_fingerprint,
        "repair_version": session.repair_version,
        "reused_from_session_id": session.reused_from_session_id,
        "repair_root_session_id": session.repair_root_session_id,
        "attachments": session.attachments,
        "attachment_context": session.attachment_context,
        "pending_steps": session.pending_steps,
    })
}

async fn groq_answer_clarification(
    provider: &Arc<dyn Provider>,
    transcript: &Arc<Mutex<E2eTranscript>>,
    intent: &str,
    session: &PlanModeSession,
    assistant_reply: &str,
) -> Result<String> {
    let system = "You are the original user answering a plan-mode clarification. Reply in one or two short sentences. Keep the same goal. Do not mention that you are an AI. Preserve these constraints: use a web search workflow, avoid databases, summarize results in chat, and keep the task read-only.";

    let user = format!(
        "Original user intent:\n{}\n\nCurrent plan-mode state:\n{}\n\nLatest assistant message:\n{}\n\nReply as the user with a concrete answer.",
        intent,
        serde_json::to_string_pretty(&session_snapshot(session)).unwrap_or_else(|_| "{}".into()),
        assistant_reply
    );

    let response = record_chat(
        "SIMULATED USER ANSWER",
        provider,
        transcript,
        vec![Message::system(system), Message::user(user)],
        vec![],
    )
    .await?;

    let mut answer = response.content.unwrap_or_default().trim().to_string();
    let assistant_lower = assistant_reply.to_ascii_lowercase();
    let intent_lower = intent.to_ascii_lowercase();

    if assistant_lower.contains("connector")
        && (intent_lower.contains("web") || intent_lower.contains("search") || intent_lower.contains("public"))
    {
        answer = "No external connectors are needed. Use web search and keep it read-only.".into();
    } else if answer.is_empty() {
        answer = if assistant_lower.contains("search") {
            "Use web search and summarize the result in chat.".into()
        } else if assistant_lower.contains("connector") {
            "No external connectors are needed. Use web search and keep it read-only.".into()
        } else if assistant_lower.contains("output") {
            "Return the result in chat and keep any notes in the workspace.".into()
        } else {
            "Use web search and keep the workflow read-only.".into()
        };
    }

    Ok(answer)
}

async fn drive_plan_mode_to_review(
    manager: &PlanModeManager,
    provider: &Arc<dyn Provider>,
    transcript: &Arc<Mutex<E2eTranscript>>,
    mut session: PlanModeSession,
    intent: &str,
    mut assistant_reply: String,
) -> Result<PlanModeSession> {
    let mut turn_count = 0usize;
    while !matches!(session.phase, PlanModePhase::Reviewing | PlanModePhase::Complete) {
        turn_count += 1;
        if turn_count > 16 {
            anyhow::bail!("plan mode did not reach review after 16 turns");
        }

        let user_answer = groq_answer_clarification(provider, transcript, intent, &session, &assistant_reply).await?;
        log_block(transcript, "SIMULATED USER", user_answer.clone());

        let (next_reply, next_session) = manager.turn(session, &user_answer).await?;
        log_block(transcript, "ASSISTANT", next_reply.clone());

        session = next_session;
        assistant_reply = next_reply;
    }

    Ok(session)
}

async fn flush_transcript(transcript: &Arc<Mutex<E2eTranscript>>, workspace_root: &str) -> Result<PathBuf> {
    let transcript_path = PathBuf::from(workspace_root).join("artifacts").join("plan_mode_web_e2e_transcript.md");
    if let Some(parent) = transcript_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let text = transcript.lock().unwrap().render();
    tokio::fs::write(&transcript_path, text).await?;
    Ok(transcript_path)
}

fn compiled_workflow_mentions_tool(compiled_workflow: &serde_json::Value, tool_name: &str) -> bool {
    compiled_workflow.to_string().contains(tool_name)
}

async fn run_runtime_smoke(
    cfg: &PlanModeWebE2EConfig,
    transcript: &Arc<Mutex<E2eTranscript>>,
    store: Arc<PostgresStore>,
    provider: Arc<dyn Provider>,
    agent: crate::agent::definition::AgentDefinition,
    role: crate::agent::definition::AgentRole,
) -> Result<()> {
    let tools = Arc::new(default_registry());
    let services = Arc::new(AgentServices::none());
    let event_bus = Arc::new(EventBus::new());
    let gateway = Arc::new(GroqLoggingGateway::new(provider, transcript.clone()));
    let executor = LlmExecutor::new(gateway, tools.clone(), services.clone())
        .with_store(store.clone())
        .with_event_bus(event_bus.clone());
    let evaluator = Arc::new(MockEvaluator::from_responses(vec![EvalVerdict::Continue; 16]));
    let preflight = Arc::new(MockPreflight::from_responses(vec![PreflightResult::Feasible]));
    let loop_runtime = AgentLoop::new(
        Arc::new(executor),
        evaluator,
        Arc::new(MockReflector::new()),
        preflight,
        Arc::new(MockClarifier::new()),
        tools,
        event_bus,
        Arc::new(RwLock::new(SkillRegistry::with_curated_defaults())),
        Arc::new(tokio::sync::Mutex::new(KnowledgeGraph::new())),
        Arc::new(InMemoryVectorStore::default()),
        Arc::new(StubEmbeddingModel::new(4)),
        services,
    )
    .with_limits(50, 300);

    let compiled_workflow = role
        .execution_guidelines
        .compiled_workflow
        .as_ref()
        .cloned()
        .context("saved role is missing compiled_workflow")?;

    let mut state = AgentState::new(
        uuid::Uuid::new_v4().to_string(),
        cfg.tenant_id.clone(),
        cfg.intent.clone(),
        std::env::temp_dir().join("narayan-plan-mode-web-runtime-smoke").display().to_string(),
    );
    state.status = AgentStatus::Pending;
    state.memory_ref = Some(agent.memory_ref.clone());
    state.metadata["agent_id"] = serde_json::json!(agent.id.clone());
    state.metadata["role_id"] = serde_json::json!(role.id.clone());
    state.metadata["role_name"] = serde_json::json!(role.name.clone());
    state.metadata["input_data"] = serde_json::json!({
        "intent": cfg.intent.clone(),
        "smoke_test": true,
    });
    state.metadata["compiled_workflow"] = serde_json::to_value(&compiled_workflow)?;

    let mut plan: Option<Plan> = None;
    let mut history = StepHistory::default();

    for step_index in 0..12 {
        let outcome = loop_runtime.run_step(&mut state, &mut plan, &mut history).await?;
        log_block(transcript, &format!("RUNTIME OUTCOME {}", step_index + 1), format!("{:?}", outcome));

        match outcome {
            StepOutcome::Complete => return Ok(()),
            StepOutcome::Continue { delay_secs } => {
                if delay_secs > 0 {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
            StepOutcome::Delegating { .. } | StepOutcome::PartiallyComplete { .. } => {}
            StepOutcome::PlanApprovalNeeded => anyhow::bail!("runtime requested plan approval after save"),
            StepOutcome::NeedsClarification { questions } => {
                anyhow::bail!("runtime requested clarification after save: {:?}", questions)
            }
            StepOutcome::Infeasible { reason } => anyhow::bail!("runtime reported infeasible workflow: {}", reason),
            StepOutcome::Failed(reason) => anyhow::bail!("runtime failed: {}", reason),
            StepOutcome::TransientError { reason, .. } => anyhow::bail!("runtime hit transient error: {}", reason),
            StepOutcome::PermanentError { reason } => anyhow::bail!("runtime hit permanent error: {}", reason),
            StepOutcome::PolicyViolation { reason } => anyhow::bail!("runtime hit policy violation: {}", reason),
            StepOutcome::RateLimited { reason, .. } => anyhow::bail!("runtime was rate limited: {}", reason),
        }
    }

    anyhow::bail!("runtime smoke did not complete within 12 steps")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires DATABASE_URL, NARAYAN_ENCRYPT_KEY, and GROQ_API_KEY"]
async fn test_plan_mode_and_runtime_smoke_web_search_groq_end_to_end() -> Result<()> {
    let cfg = PlanModeWebE2EConfig::from_env()?;
    let transcript = Arc::new(Mutex::new(E2eTranscript::default()));

    log_block(
        &transcript,
        "E2E CONFIG",
        format!(
            "database_url: <redacted>\ngroq_model: {}\ntenant_id: {}\nagent_name: {}\nintent: {}\nmax_repair_rounds: {}",
            cfg.groq_model,
            cfg.tenant_id,
            cfg.agent_name,
            cfg.intent,
            cfg.max_repair_rounds
        ),
    );

    let store = Arc::new(PostgresStore::new(&cfg.database_url, 10).await?);
    store.migrate().await?;

    let installs = Arc::new(ConnectorInstallStore::new(store.pool(), cfg.encrypt_key.clone()));
    installs.migrate().await?;

    let tools = Arc::new(default_registry());
    let skills = Arc::new(RwLock::new(SkillRegistry::with_curated_defaults()));
    let provider = build_provider("groq", cfg.groq_api_key.clone(), cfg.groq_model.clone())
        .context("failed to build Groq provider")?;
    let gateway = Arc::new(GroqLoggingGateway::new(provider.clone(), transcript.clone()));
    let manager = PlanModeManager::new(
        gateway,
        store.clone(),
        installs.clone(),
        tools,
        std::env::temp_dir().join("narayan-plan-mode-workspace"),
    )
    .with_skill_registry(skills);

    let mut session = manager.new_session(&cfg.tenant_id, &cfg.agent_name);
    log_block(
        &transcript,
        "INITIAL SESSION",
        serde_json::to_string_pretty(&session_snapshot(&session)).unwrap_or_default(),
    );

    let (first_reply, next_session) = manager.turn(session, &cfg.intent).await?;
    log_block(&transcript, "ASSISTANT", first_reply.clone());
    session = next_session;

    session = drive_plan_mode_to_review(&manager, &provider, &transcript, session, &cfg.intent, first_reply).await?;
    let review_summary = manager.build_review_summary_pub(&mut session).await;
    log_block(&transcript, "REVIEW SUMMARY", review_summary.clone());

    let mut auto_result = manager.test(&session).await?;
    log_block(
        &transcript,
        "AUTO VALIDATION",
        serde_json::to_string_pretty(&auto_result).unwrap_or_else(|_| auto_result.summary.clone()),
    );

    let mut attempts = 0usize;
    while attempts < cfg.max_repair_rounds && auto_result.status != PlanModeTestStatus::Pass {
        attempts += 1;
        log_block(
            &transcript,
            "REPAIR DECISION",
            format!(
                "attempt: {}\nauto_status: {:?}\nauto_summary: {}",
                attempts, auto_result.status, auto_result.summary
            ),
        );

        let repair_result = auto_result.clone();

        if attempts >= cfg.max_repair_rounds {
            break;
        }

        let (response, next_session) = manager
            .revise_from_test_result(session, &repair_result)
            .await
            .context("plan mode revise after validation")?;
        log_block(&transcript, "REPAIR RESPONSE", response.clone());

        session = next_session;
        session = drive_plan_mode_to_review(&manager, &provider, &transcript, session, &cfg.intent, response).await?;
        let review_summary = manager.build_review_summary_pub(&mut session).await;
        log_block(&transcript, "REVIEW SUMMARY", review_summary.clone());
        auto_result = manager.test(&session).await?;
        log_block(
            &transcript,
            "AUTO VALIDATION",
            serde_json::to_string_pretty(&auto_result).unwrap_or_else(|_| auto_result.summary.clone()),
        );
    }

    anyhow::ensure!(
        auto_result.status == PlanModeTestStatus::Pass,
        "plan mode did not compile a valid workflow before save: {:?} / {}",
        auto_result.status,
        auto_result.summary
    );

    let (saved_agent, saved_role) = manager.save(session).await?;
    log_block(
        &transcript,
        "SAVED ROLE",
        serde_json::to_string_pretty(&saved_role).unwrap_or_else(|_| saved_role.name.clone()),
    );

    let compiled_workflow = saved_role
        .execution_guidelines
        .compiled_workflow
        .as_ref()
        .context("saved role is missing compiled_workflow")?;
    let compiled_workflow_value = serde_json::to_value(compiled_workflow)?;
    anyhow::ensure!(
        compiled_workflow_mentions_tool(&compiled_workflow_value, "web_search_tool"),
        "saved compiled workflow did not mention web_search_tool"
    );

    run_runtime_smoke(&cfg, &transcript, store.clone(), provider.clone(), saved_agent, saved_role).await?;

    let transcript_path = flush_transcript(&transcript, "/tmp/narayan-plan-mode").await?;
    log_block(&transcript, "TRANSCRIPT PATH", format!("{}", transcript_path.display()));

    Ok(())
}
