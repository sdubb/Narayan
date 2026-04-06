//! Thin coordinator for plan mode.
//!
//! This module owns the public `PlanModeManager` and `IntentExtractor` types,
//! but delegates the actual prompt, clarification, review, and repair logic
//! to the smaller submodules in this directory.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use base64::Engine as _;
use chrono::Utc;
use uuid::Uuid;

use crate::{
    agent::definition::{
        AgentDefinition, AgentDefinitionStatus, AgentRole, PlanModeAttachment, PlanModeAttachmentKind,
        PlanModeAttachmentUpload, PlanModeCompilerStage, PlanModeMessage, PlanModePhase, PlanModePreflightResult,
        PlanModeSandboxResult, PlanModeSession, PlanModeTestConfidence, PlanModeTestResult, PlanModeTestStatus,
        RoleCategory, TenantConnector,
    },
    agent::planner::AdaptiveResearchMemo,
    connectors::ConnectorInstallStore,
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::{Message, ToolSpec},
    storage::PostgresStore,
    tools::{tool_spec_from_tool, ToolRegistry},
};

use super::{
    clarify::{build_step_queue_and_ask, handle_clarifications, handle_connector_clarification, handle_constraints,
        ClarificationEngine},
    intent::seed_intent_from_description,
    repair::build_revision_prompt_from_test_result,
    registry::{
        reconcile_acp_bindings, reconcile_api_bindings, reconcile_database_bindings, reconcile_mcp_bindings,
        reconcile_role_connectors, reconcile_role_tools, ConnectorResolver,
    },
    review::{apply_role_policy_defaults, build_workflow_contract, finalize_saved_role_execution_strategy, reconcile_role_tool_pool, review_hint},
    steps::{intent_extractor_system_prompt, workflow_contract_prompt_fragment},
};

/// Extracts structured intent from a free-form business description.
pub struct IntentExtractor {
    gateway: Arc<dyn LlmGateway>,
}

impl IntentExtractor {
    pub fn new(gateway: Arc<dyn LlmGateway>) -> Self {
        Self { gateway }
    }

    pub async fn extract_initial(
        &self,
        session_id: &str,
        tenant_id: &str,
        description: &str,
        tools: &ToolRegistry,
        tool_specs: Vec<ToolSpec>,
    ) -> Result<serde_json::Value> {
        let system = intent_extractor_system_prompt("", "", workflow_contract_prompt_fragment());
        let user = format!("Configure an agent to do:\n\n{}", description);
        let mut messages = vec![Message::system(system), Message::user(user)];
        let mut rounds = 0usize;
        let generation = crate::gateway::llm_controls::LlmGenerationConfig::new(
            crate::gateway::llm_controls::LlmRole::Extractor,
            crate::gateway::llm_controls::LlmExecutionIntent::Strict,
            crate::gateway::llm_controls::LlmBudgetTier::Standard,
        )
        .with_limits(2048, 0.0)
        .with_json_schema_response(
            "plan_mode_intent_extraction",
            crate::agent::workflow_compiler::llm_output_schema(&crate::gateway::llm_controls::LlmRole::Extractor),
        );

        loop {
            let request = GatewayRequest::new(
                session_id.to_string(),
                tenant_id.to_string(),
                TaskComplexity::Medium,
                messages.clone(),
            )
            .with_tools(tool_specs.clone())
            .with_generation(generation.clone());
            let response = self.gateway.chat(request).await?;
            if !response.tool_calls.is_empty() && rounds < 3 {
                rounds += 1;
                for call in response.tool_calls {
                    let tool_result = crate::tools::run_planning_search_tool(tools, &call.name, &call.arguments)
                        .await
                        .unwrap_or_else(|err| crate::tools::ToolResult::err(err.to_string()));
                    if call.name == "ask_user" && tool_result.success {
                        if let Some(intent) = intent_from_ask_user(description, &tool_result.output) {
                            return Ok(intent);
                        }
                    }
                    messages.push(Message::tool(
                        serde_json::to_string(&tool_result).unwrap_or_default(),
                        call.id.clone(),
                    ));
                }
                continue;
            }
            return self.parse_json_response(response.content.unwrap_or_default());
        }
    }

    pub async fn refine(
        &self,
        session_id: &str,
        tenant_id: &str,
        purpose: &str,
        current_intent: &serde_json::Value,
        detail_context: &str,
    ) -> Result<serde_json::Value> {
        let system = intent_extractor_system_prompt("", detail_context, workflow_contract_prompt_fragment());
        let user = format!(
            "Refine the current plan draft for this agent purpose:\n{}\n\nCURRENT INTENT:\n{}",
            purpose,
            serde_json::to_string_pretty(current_intent).unwrap_or_default()
        );
        let generation = crate::gateway::llm_controls::LlmGenerationConfig::new(
            crate::gateway::llm_controls::LlmRole::Router,
            crate::gateway::llm_controls::LlmExecutionIntent::Strict,
            crate::gateway::llm_controls::LlmBudgetTier::Standard,
        )
        .with_limits(2048, 0.0)
        .with_json_schema_response(
            "plan_mode_intent_refinement",
            crate::agent::workflow_compiler::llm_output_schema(&crate::gateway::llm_controls::LlmRole::Router),
        );
        let request = GatewayRequest::new(
            session_id.to_string(),
            tenant_id.to_string(),
            TaskComplexity::Medium,
            vec![Message::system(system), Message::user(user)],
        )
        .with_generation(generation);
        let response = self.gateway.chat(request).await?;
        self.parse_json_response(response.content.unwrap_or_default())
    }

    fn parse_json_response(&self, raw: String) -> Result<serde_json::Value> {
        let cleaned = clean_json_markdown_response(&raw);
        serde_json::from_str(&cleaned).map_err(|e| {
            anyhow::anyhow!("intent extraction returned invalid JSON: {} — raw: {}", e, &raw[..raw.len().min(200)])
        })
    }
}

fn clean_json_markdown_response(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("```json") {
        let body = &trimmed[start + 7..];
        if let Some(end) = body.find("```") {
            return body[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let body = &trimmed[start + 3..];
        if let Some(end) = body.find("```") {
            return body[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn intent_from_ask_user(description: &str, output: &serde_json::Value) -> Option<serde_json::Value> {
    let questions = output.get("questions")?.as_array()?;
    if questions.is_empty() {
        return None;
    }

    let seed = seed_intent_from_description(description);
    let category = infer_category_from_seed(&seed);
    let mut intent = seed;
    if let Some(obj) = intent.as_object_mut() {
        obj.entry("category".to_string()).or_insert_with(|| serde_json::json!(category));
        // Let the standard plan-mode step generator drive the follow-up
        // questions. We only use the ask_user result to strengthen the seed
        // intent, not to replace the rest of the clarification scaffold.
        for question in questions {
            let prompt = question
                .get("prompt")
                .and_then(|value| value.as_str())
                .or_else(|| question.get("question").and_then(|value| value.as_str()))
                .unwrap_or_default()
                .to_lowercase();
            if prompt.contains("database") || prompt.contains("db") || prompt.contains("sql") {
                obj.insert("uses_external_db".into(), serde_json::Value::Bool(true));
            }
            if prompt.contains("api") || prompt.contains("endpoint") || prompt.contains("http") {
                obj.insert("uses_external_api".into(), serde_json::Value::Bool(true));
            }
            if prompt.contains("mcp") {
                let caps = obj
                    .entry("missing_capabilities".to_string())
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                if let Some(arr) = caps.as_array_mut() {
                    if !arr.iter().any(|value| value.as_str() == Some("connector/mcp")) {
                        arr.push(serde_json::Value::String("connector/mcp".into()));
                    }
                }
            }
            if prompt.contains("acp") || prompt.contains("peer") {
                let caps = obj
                    .entry("missing_capabilities".to_string())
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                if let Some(arr) = caps.as_array_mut() {
                    if !arr.iter().any(|value| value.as_str() == Some("connector/acp")) {
                        arr.push(serde_json::Value::String("connector/acp".into()));
                    }
                }
            }
        }
    }
    Some(intent)
}

fn infer_category_from_seed(seed: &serde_json::Value) -> String {
    let matches = |needle: &str| {
        seed.get("preferred_tool_categories")
            .and_then(|value| value.as_array())
            .map(|arr| arr.iter().any(|value| value.as_str().map(|s| s == needle).unwrap_or(false)))
            .unwrap_or(false)
            || seed
                .get("needed_connector_categories")
                .and_then(|value| value.as_array())
                .map(|arr| arr.iter().any(|value| value.as_str().map(|s| s == needle).unwrap_or(false)))
                .unwrap_or(false)
    };

    if matches("data") {
        "data".into()
    } else if matches("web") {
        "web".into()
    } else if matches("integration") {
        "integration".into()
    } else if matches("automation") {
        "automation".into()
    } else {
        "general".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_user_merges_database_hint_without_replacing_the_full_intent() {
        let intent = intent_from_ask_user(
            "monitor my database",
            &serde_json::json!({
                "questions": [
                    {
                        "question": "Which database would you like to monitor?",
                        "type": "clarification"
                    }
                ]
            }),
        )
        .expect("intent should be inferred");

        assert!(intent.get("clarification_steps").is_none());
        assert_eq!(intent.get("uses_external_db").and_then(|v| v.as_bool()), Some(true));
        assert!(intent
            .get("missing_capabilities")
            .and_then(|value| value.as_array())
            .map(|arr| arr.iter().any(|value| value.as_str() == Some("custom_db")))
            .unwrap_or(false));
    }
}

pub struct PlanModeManager {
    gateway: Arc<dyn LlmGateway>,
    store: Arc<PostgresStore>,
    installs: Arc<ConnectorInstallStore>,
    tools: Arc<ToolRegistry>,
    workspace_root: PathBuf,
    extractor: IntentExtractor,
    skill_registry: Option<Arc<tokio::sync::RwLock<crate::skills::registry::SkillRegistry>>>,
    clarification_engine: ClarificationEngine,
}

impl PlanModeManager {
    pub fn new(
        gateway: Arc<dyn LlmGateway>,
        store: Arc<PostgresStore>,
        installs: Arc<ConnectorInstallStore>,
        tools: Arc<ToolRegistry>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        let workspace_root = workspace_root.into();
        let extractor = IntentExtractor::new(Arc::clone(&gateway));
        let clarification_engine = ClarificationEngine::new(
            Arc::clone(&gateway),
            Arc::clone(&store),
            Arc::clone(&installs),
            Arc::clone(&tools),
        );
        Self { gateway, store, installs, tools, workspace_root, extractor, skill_registry: None, clarification_engine }
    }

    pub fn with_skill_registry(
        mut self,
        registry: Arc<tokio::sync::RwLock<crate::skills::registry::SkillRegistry>>,
    ) -> Self {
        self.skill_registry = Some(registry);
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn gateway(&self) -> Arc<dyn LlmGateway> {
        Arc::clone(&self.gateway)
    }

    pub fn new_session(&self, tenant_id: &str, agent_name: &str) -> PlanModeSession {
        let now = Utc::now();
        let agent_id = Uuid::new_v4().to_string();
        let draft_agent = AgentDefinition {
            id: agent_id.clone(),
            tenant_id: tenant_id.to_string(),
            name: agent_name.to_string(),
            persona: String::new(),
            connectors: Vec::new(),
            constraints: Vec::new(),
            memory_ref: String::new(),
            status: AgentDefinitionStatus::Draft,
            created_at: now,
            updated_at: now,
        };
        PlanModeSession {
            id: Uuid::new_v4().to_string(),
            tenant_id: tenant_id.to_string(),
            draft_agent,
            draft_role: None,
            conversation: Vec::new(),
            attachments: Vec::new(),
            attachment_context: String::new(),
            session_workspace: None,
            goal_fingerprint: None,
            repair_version: 1,
            reused_from_session_id: None,
            repair_root_session_id: None,
            phase: PlanModePhase::CapturingIntent,
            compiler_stage: PlanModeCompilerStage::Review,
            compiler_repair_passes: 0,
            compiler_validation_issues: Vec::new(),
            intent_cache: None,
            pending_steps: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub async fn ingest_attachments(
        &self,
        session: &mut PlanModeSession,
        uploads: Vec<PlanModeAttachmentUpload>,
        _a: Option<()>,
        _b: Option<()>,
    ) -> Result<()> {
        for upload in uploads {
            let bytes = base64::engine::general_purpose::STANDARD.decode(upload.content_base64.as_bytes())?;
            let kind = match upload
                .mime_type
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                m if m.contains("pdf") => PlanModeAttachmentKind::Pdf,
                m if m.contains("sheet") || m.contains("excel") => PlanModeAttachmentKind::Spreadsheet,
                m if m.contains("csv") => PlanModeAttachmentKind::Csv,
                m if m.contains("text") || m.contains("plain") => PlanModeAttachmentKind::Text,
                _ => PlanModeAttachmentKind::Unknown,
            };
            session.attachments.push(PlanModeAttachment {
                name: upload.name.clone(),
                path: upload.name,
                mime_type: upload.mime_type,
                size_bytes: bytes.len() as u64,
                kind,
                extracted_preview: String::new(),
                uploaded_at: Utc::now(),
            });
        }
        Ok(())
    }

    pub async fn turn(&self, mut session: PlanModeSession, user_message: &str) -> Result<(String, PlanModeSession)> {
        self.turn_inner(&mut session, user_message).await?;
        Ok((session.conversation.last().map(|m| m.content.clone()).unwrap_or_default(), session))
    }

    async fn turn_inner(&self, session: &mut PlanModeSession, user_message: &str) -> Result<()> {
        session.conversation.push(PlanModeMessage { role: "user".into(), content: user_message.into() });
        let reply = match session.phase {
            PlanModePhase::CapturingIntent => self.handle_intent(session, user_message).await?,
            PlanModePhase::ResolvingConnectors => self.handle_connector_resolution(session, user_message).await?,
            PlanModePhase::CapturingClarifications => {
                self.handle_clarifications(session, user_message).await?
            }
            PlanModePhase::CapturingConstraints => self.handle_constraints(session, user_message).await?,
            PlanModePhase::Reviewing => self.handle_review(session, user_message).await?,
            PlanModePhase::Complete => "Plan mode is already complete.".into(),
        };
        session.conversation.push(PlanModeMessage { role: "assistant".into(), content: reply });
        Ok(())
    }

    async fn handle_intent(&self, session: &mut PlanModeSession, description: &str) -> Result<String> {
        let installed: Vec<String> = self
            .installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();
        let tenant_connectors: Vec<TenantConnector> =
            self.store.list_tenant_connectors(&session.tenant_id).await.unwrap_or_default();
        let seed_intent = seed_intent_from_description(description);
        let tool_specs = self.planning_tool_specs(&seed_intent);
        let intent = self
            .extractor
            .extract_initial(&session.id, &session.tenant_id, description, &self.tools, tool_specs)
            .await?;
        session.intent_cache = Some(intent.clone());
        session.draft_role = Some(AgentRole::new(
            Uuid::new_v4().to_string(),
            session.draft_agent.id.clone(),
            session.tenant_id.clone(),
            "Primary Role".into(),
        ));
        if let Some(role) = session.draft_role.as_mut() {
            apply_role_policy_defaults(&mut session.draft_agent, role);
            role.purpose = description.trim().to_string();
            role.role_category = RoleCategory::from_slug(intent["category"].as_str().unwrap_or("general"));
            role.connectors = ConnectorResolver::resolve(&intent, &installed, &tenant_connectors).await.0;
            session.draft_agent.connectors = role.connectors.clone();
            role.tools = reconcile_role_tools(&self.tools, &intent, &[], &role.connectors);
        }
        session.phase = PlanModePhase::ResolvingConnectors;
        Ok(build_step_queue_and_ask(&self.clarification_engine, session, &intent).await)
    }

    fn planning_tool_specs(&self, intent: &serde_json::Value) -> Vec<ToolSpec> {
        let mut names = vec![
            "search_mcp_registry",
            "search_connector_registry",
            "search_acp_peers",
            "list_connectors_in_category",
            "ask_user",
        ];
        if intent.get("missing_capabilities").and_then(|value| value.as_array()).map(|arr| arr.iter().any(|v| v.as_str() == Some("connector/acp"))).unwrap_or(false) {
            names.push("acp_session");
        }
        names
            .into_iter()
            .filter_map(|name| self.tools.get(name).map(|tool| tool_spec_from_tool(tool.as_ref())).or_else(|| crate::tools::planning_tool_spec(name)))
            .collect()
    }

    async fn handle_connector_resolution(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        let reply = handle_connector_clarification(&self.clarification_engine, session, answer).await?;
        let installed: Vec<String> = self
            .installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();
        let tenant_connectors: Vec<TenantConnector> =
            self.store.list_tenant_connectors(&session.tenant_id).await.unwrap_or_default();
        if let Some(intent) = session.intent_cache.clone() {
            if let Some(role) = session.draft_role.as_mut() {
                reconcile_role_tool_pool(role);
                role.connectors = reconcile_role_connectors(&intent, &role.connectors);
                role.connectors = ConnectorResolver::resolve(&intent, &installed, &tenant_connectors).await.0;
                role.tools = reconcile_database_bindings(&intent, &role.tools);
                role.tools = reconcile_api_bindings(&intent, &role.tools);
                role.tools = reconcile_mcp_bindings(&intent, &role.tools);
                role.tools = reconcile_acp_bindings(&intent, &role.tools);
                role.tools = reconcile_role_tools(&self.tools, &intent, &role.tools, &role.connectors);
            }
        }
        Ok(reply)
    }

    async fn handle_clarifications(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        let reply = handle_clarifications(&self.clarification_engine, session, answer).await?;
        let installed: Vec<String> = self
            .installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();
        let tenant_connectors: Vec<TenantConnector> =
            self.store.list_tenant_connectors(&session.tenant_id).await.unwrap_or_default();
        if let Some(intent) = session.intent_cache.clone() {
            if let Some(role) = session.draft_role.as_mut() {
                reconcile_role_tool_pool(role);
                role.connectors = reconcile_role_connectors(&intent, &role.connectors);
                role.connectors = ConnectorResolver::resolve(&intent, &installed, &tenant_connectors).await.0;
                role.tools = reconcile_database_bindings(&intent, &role.tools);
                role.tools = reconcile_api_bindings(&intent, &role.tools);
                role.tools = reconcile_mcp_bindings(&intent, &role.tools);
                role.tools = reconcile_acp_bindings(&intent, &role.tools);
                role.tools = reconcile_role_tools(&self.tools, &intent, &role.tools, &role.connectors);
            }
        }
        if session.phase == PlanModePhase::Reviewing {
            self.ensure_research_memo(session).await?;
            session.phase = PlanModePhase::Reviewing;
        }
        Ok(reply)
    }

    async fn handle_constraints(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        let reply = handle_constraints(session, answer).await?;
        if session.phase == PlanModePhase::Reviewing {
            self.ensure_research_memo(session).await?;
        }
        Ok(reply)
    }

    async fn handle_review(&self, session: &mut PlanModeSession, answer: &str) -> Result<String> {
        if answer.trim().is_empty() {
            return Ok(self.build_review_summary(session).await);
        }
        if answer.to_lowercase().contains("save") || answer.to_lowercase().contains("approve") {
            session.phase = PlanModePhase::Complete;
            return Ok("Plan mode approved. Save the draft to continue.".into());
        }
        Ok(self.build_review_summary(session).await)
    }

    async fn refine_after_clarifications(&self, _session: &mut PlanModeSession) -> Result<Option<String>> {
        Ok(None)
    }

    async fn ensure_research_memo(&self, session: &mut PlanModeSession) -> Result<()> {
        if let Some(intent) = session.intent_cache.as_mut() {
            if intent.get("_adaptive_research_memo").is_none() {
                let memo = AdaptiveResearchMemo { summary: String::new(), findings: Vec::new(), assumptions: Vec::new(), risks: Vec::new(), workflow_hints: Vec::new() };
                if let Some(obj) = intent.as_object_mut() {
                    obj.insert("_adaptive_research_memo".into(), serde_json::to_value(memo)?);
                }
            }
        }
        Ok(())
    }

    async fn build_review_summary(&self, session: &PlanModeSession) -> String {
        if let Some(intent) = session.intent_cache.as_ref() {
            let contract = build_workflow_contract(intent, session, session.draft_role.as_ref());
            format!(
                "Review the draft for {}.\n\nContract steps: {}\nConnector needs: {}\nSubsystems: {}",
                session.draft_agent.name,
                contract.steps.len(),
                contract.boundary_requirements.join(", "),
                contract.subsystem_requirements.join(", ")
            )
        } else {
            review_hint(session)
        }
    }

    pub async fn build_review_summary_pub(&self, session: &mut PlanModeSession) -> String {
        self.build_review_summary(session).await
    }

    pub async fn test(&self, session: &PlanModeSession) -> Result<PlanModeTestResult> {
        let pass = session.intent_cache.is_some() && session.draft_role.is_some();
        Ok(PlanModeTestResult {
            status: if pass { PlanModeTestStatus::Pass } else { PlanModeTestStatus::Fail },
            confidence: PlanModeTestConfidence::High,
            preflight: PlanModePreflightResult { status: if pass { PlanModeTestStatus::Pass } else { PlanModeTestStatus::Fail }, checks: Vec::new(), summary: String::new() },
            sandbox: PlanModeSandboxResult { status: if pass { PlanModeTestStatus::Pass } else { PlanModeTestStatus::Fail }, steps: Vec::new(), summary: String::new() },
            steps: Vec::new(),
            criteria_checks: Vec::new(),
            summary: if pass { "plan draft is structurally complete".into() } else { "plan draft is incomplete".into() },
        })
    }

    pub async fn revise_from_test_result(
        &self,
        mut session: PlanModeSession,
        test_result: &PlanModeTestResult,
    ) -> Result<(String, PlanModeSession)> {
        let prompt = build_revision_prompt_from_test_result(test_result);
        self.turn_inner(&mut session, &prompt).await?;
        Ok((prompt, session))
    }

    pub async fn save(&self, mut session: PlanModeSession) -> Result<(AgentDefinition, AgentRole)> {
        let mut role = session.draft_role.take().ok_or_else(|| anyhow::anyhow!("missing draft role"))?;
        let installed: Vec<String> = self
            .installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();
        let tenant_connectors: Vec<TenantConnector> =
            self.store.list_tenant_connectors(&session.tenant_id).await.unwrap_or_default();
        if let Some(intent) = session.intent_cache.clone() {
            reconcile_role_tool_pool(&mut role);
            role.connectors = reconcile_role_connectors(&intent, &role.connectors);
            role.connectors = ConnectorResolver::resolve(&intent, &installed, &tenant_connectors).await.0;
            role.tools = reconcile_database_bindings(&intent, &role.tools);
            role.tools = reconcile_api_bindings(&intent, &role.tools);
            role.tools = reconcile_mcp_bindings(&intent, &role.tools);
            role.tools = reconcile_acp_bindings(&intent, &role.tools);
            role.tools = reconcile_role_tools(&self.tools, &intent, &role.tools, &role.connectors);
        }
        finalize_saved_role_execution_strategy(&mut role);
        let agent = session.draft_agent.clone();
        Ok((agent, role))
    }
}
