//! Plan mode — the agent configuration conversation.
//!
//! Plan mode is the one-time setup phase where a user describes what an agent
//! should do in plain business language.  The system internally figures out
//! which connectors, tools, triggers, and constraints are needed.
//! The user never sees tool names or connector IDs.
//!
//! ## Flow
//!
//!   POST /plan-mode/sessions          → create PlanModeSession
//!   POST /plan-mode/sessions/:id/turn → send user message, get assistant reply
//!   POST /plan-mode/sessions/:id/save → save AgentDefinition + AgentRole, close session
//!
//! ## Phases
//!
//!   CapturingIntent        → "What should this agent do?"
//!   ResolvingConnectors    → system resolves internally, maybe one clarifying Q
//!   CapturingClarifications → combined: trigger confirm + output questions + multi-role suggestion
//!   CapturingConstraints   → domain skill mandatory questions + user constraints
//!   Reviewing              → show the full config for user confirmation
//!   Complete               → save and close

use std::{collections::BTreeMap, sync::Arc};

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::{
    agent::definition::{
        AgentDefinition, AgentDefinitionStatus, AgentRole, ConnectorAuthType, EndpointDef,
        OutputDestination, OutputFormat, OutputSpec, PlanModeMessage, PlanModePhase,
        PlanModeSession, RoleCategory, RoleStatus, TenantConnector, TriggerDef, TriggerType,
    },
    connectors::ConnectorInstallStore,
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    providers::Message,
    storage::PostgresStore,
    tools::ToolRegistry,
};

// ── Built-in connector catalogue ─────────────────────────────────────────────
// Delegate to connector_tool::ALL_CONNECTORS — single source of truth.
// The ConnectorDef type has .name, .category, .keywords, .summary fields
// that the ConnectorResolver uses — same field names, no other changes needed.
use crate::tools::connector_tool::ALL_CONNECTORS as BUILTIN_CONNECTORS;

// ── IntentExtractor ────────────────────────────────────────────────────────

/// Extracts structured intent from a free-form business description.
/// Returns a JSON object with: data_sources, actions, trigger_hint, output_hint, constraints
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
        capability_directory: &str,
    ) -> Result<serde_json::Value> {
        let capability_section = if capability_directory.is_empty() {
            String::new()
        } else {
            format!("\n\nCAPABILITY DIRECTORY:\n{}", capability_directory)
        };

        let system = format!(r#"You are a business analyst helping configure an AI automation agent.
Extract structured intent AND generate specific clarifying questions.

Work in two stages internally:
1. Infer the business workflow shape and the capability categories needed.
2. Pick exact connectors or tools only when the directory/context makes them clear.

Respond ONLY with valid JSON. Schema:
{{
  "data_sources": ["systems the agent reads from"],
  "write_targets": ["systems the agent writes to"],
  "actions": ["what the agent does, plain English verbs"],
  "category": "sales_revops|customer_support|devops|finance_accounting|hr_people_ops|legal_contract|research_analyst|software_engineer|marketing|general",
  "preferred_tool_categories": ["tool category names such as data, web, automation"],
  "preferred_tools": ["exact tool names from the capability directory only"],
  "needed_connector_categories": ["connector category suffixes such as crm, support, communication"],
  "candidate_connectors": ["exact installed or built-in connector names if likely"],
  "missing_capabilities": ["custom_db|custom_api|connector/<category>|tool/<category>"],
  "workflow_outline": ["short ordered workflow hints, e.g. fetch records, enrich them, update CRM, notify Slack"],
  "uses_external_db": "registered database name or null",
  "uses_external_api": "registered API name or null",
  "trigger_hint": "schedule|webhook|user_message|manual",
  "trigger_cron": "best-guess cron expression if schedule, else null",
  "trigger_source": "connector name if webhook, else null",
  "trigger_event": "event name if webhook e.g. lead_created, else null",
  "trigger_confidence": "high|medium|low",
  "trigger_confirmation": "confirmation question if medium/low confidence, else null",
  "output_hint": "workspace|connector_record|slack_message|email_draft|email_send|report|notification",
  "output_destination_hint": "where exactly — workspace path, connector name, or channel",
  "output_questions": ["specific missing output detail questions, empty array if clear"],
  "responsibilities": [
    {{"name": "short role name", "actions": ["verbs"], "trigger_hint": "schedule|webhook|manual"}}
  ],
  "multi_role_suggested": false,
  "multi_role_reason": "why split is recommended, or null",
  "clarifying_questions": []
}}{}

Rules:
- Use exact tool names only from the capability directory or detailed context
- Use exact connector names only when they are clearly supported by the context
- If the user likely needs a database not in the installed connectors, prefer missing_capabilities=["custom_db"]
- If the user likely needs a custom REST backend, prefer missing_capabilities=["custom_api"]
- If the needed connector category is clear but no installed connector is obvious, add connector/<category> to missing_capabilities
- workflow_outline should be high-level and ordered, not low-level tool calls
- trigger_confidence is high only when cron/event is fully unambiguous
- trigger_confidence medium: parsed but missing detail (no time, no connector named)
- trigger_confidence low: trigger type itself unclear
- output_questions: only ask what you cannot infer
- multi_role_suggested: true only if 2+ clearly distinct responsibilities with different triggers or outputs
- responsibilities: always list at least one entry"#, capability_section);

        let user = format!("Configure an agent to do:\n\n{}", description);

        let first_pass = GatewayRequest::new(
            session_id.to_string(),
            tenant_id.to_string(),
            TaskComplexity::Medium,
            vec![Message::system(system), Message::user(user)],
        );

        self.parse_json_response(self.gateway.chat(first_pass).await?.content.unwrap_or_default())
    }

    pub async fn refine(
        &self,
        session_id: &str,
        tenant_id: &str,
        description: &str,
        initial: &serde_json::Value,
        detailed_context: &str,
    ) -> Result<serde_json::Value> {
        let refine_system = format!(r#"You are refining a previously inferred agent configuration.
Use the detailed capability context below to keep what was right, correct what was vague,
and choose exact tools/connectors where supported.

Return ONLY valid JSON with the exact same schema as before.

Detailed capability context:
{}

Rules:
- Preserve the original business intent unless the detailed context proves it impossible
- Fill preferred_tools with exact tool names only when the tool is clearly relevant
- Fill candidate_connectors with exact names only when the connector is clearly relevant
- Keep missing_capabilities accurate if no installed/custom option satisfies the need
- Keep workflow_outline ordered and practical
"#, detailed_context);

        let refine_user = format!(
            "Original request:\n{}\n\nPreliminary inference JSON:\n{}",
            description,
            serde_json::to_string_pretty(&initial).unwrap_or_else(|_| initial.to_string())
        );

        let second_pass = GatewayRequest::new(
            session_id.to_string(),
            tenant_id.to_string(),
            TaskComplexity::Medium,
            vec![Message::system(refine_system), Message::user(refine_user)],
        );

        self.parse_json_response(self.gateway.chat(second_pass).await?.content.unwrap_or_default())
    }

    fn parse_json_response(&self, raw: String) -> Result<serde_json::Value> {
        let cleaned = raw.trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        serde_json::from_str(cleaned).map_err(|e| {
            anyhow::anyhow!("intent extraction returned invalid JSON: {} — raw: {}", e, &raw[..raw.len().min(200)])
        })
    }
}

// ── ConnectorResolver ──────────────────────────────────────────────────────

/// Maps extracted intent to specific connector names + tool overrides.
/// Returns (resolved_connectors, tool_overrides, clarifying_question)
/// tool_overrides are non-connector tools like external_db, external_api
pub struct ConnectorResolver;

impl ConnectorResolver {
    /// Resolve which connectors and special tools are needed for the extracted intent.
    pub async fn resolve(
        intent: &serde_json::Value,
        installed: &[String],
        tenant_connectors: &[TenantConnector],
    ) -> (Vec<String>, Vec<String>, Option<String>) {
        let sources: Vec<String> = intent["data_sources"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
            .unwrap_or_default();
        let writes: Vec<String> = intent["write_targets"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
            .unwrap_or_default();
        let actions: Vec<String> = intent["actions"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_lowercase())).collect())
            .unwrap_or_default();

        let all_terms: Vec<&str> = sources.iter()
            .chain(writes.iter())
            .chain(actions.iter())
            .map(String::as_str)
            .collect();
        let candidate_connectors: Vec<String> = intent["candidate_connectors"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let needed_connector_categories: Vec<String> = intent["needed_connector_categories"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let missing_capabilities: Vec<String> = intent["missing_capabilities"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        // ── Tool overrides for external_db and external_api ─────────────
        let mut tool_overrides: Vec<String> = Vec::new();

        // If the intent explicitly named an external_db
        if let Some(db_name) = intent["uses_external_db"].as_str() {
            if !db_name.is_empty() && db_name != "null" {
                tool_overrides.push(format!("external_db:{}", db_name));
            }
        }
        // If the intent explicitly named an external_api
        if let Some(api_name) = intent["uses_external_api"].as_str() {
            if !api_name.is_empty() && api_name != "null" {
                tool_overrides.push(format!("external_api:{}", api_name));
            }
        }

        // Detect database mentions in tenant_connectors (category = connector/database)
        for tc in tenant_connectors {
            if tc.category == "connector/database" {
                if !tool_overrides.iter().any(|t| t.contains(&tc.name)) {
                    // Check if any intent term matches the db name or summary
                    if terms_match_connector(&all_terms, tc) {
                        tool_overrides.push(format!("external_db:{}", tc.name));
                    }
                }
            }
        }

        // ── Score built-in connectors ────────────────────────────────────
        let mut scored: Vec<(usize, &crate::tools::connector_tool::ConnectorDef)> = {
            let mut v: Vec<(usize, &crate::tools::connector_tool::ConnectorDef)> =
                BUILTIN_CONNECTORS
                    .iter()
                    .map(|entry| {
                        let score = entry.keywords.iter()
                            .filter(|kw| all_terms.iter().any(|t| {
                                t.contains(**kw) || kw.contains(t)
                            }))
                            .count();
                        (score, entry)
                    })
                    .filter(|(score, _)| *score > 0)
                    .collect();
            v.sort_by(|a, b| b.0.cmp(&a.0));
            v
        };

        let mut resolved: Vec<String> = Vec::new();
        let mut ambiguous_categories: Vec<(&str, Vec<&str>)> = Vec::new();
        let mut resolved_categories: std::collections::HashSet<&str> = Default::default();

        for requested in &candidate_connectors {
            if installed.iter().any(|name| name == requested) || tenant_connectors.iter().any(|tc| tc.name == *requested) {
                resolved.push(requested.clone());
                if let Some(entry) = BUILTIN_CONNECTORS.iter().find(|entry| entry.name == requested.as_str()) {
                    resolved_categories.insert(entry.category);
                }
            }
        }

        for (_, entry) in &scored {
            let is_installed = installed.iter().any(|i| i == entry.name);
            if !is_installed { continue; }

            if resolved_categories.contains(entry.category) {
                if let Some(cat_entry) = ambiguous_categories.iter_mut().find(|(c, _)| *c == entry.category) {
                    cat_entry.1.push(entry.name);
                }
                continue;
            }
            resolved_categories.insert(entry.category);
            resolved.push(entry.name.to_string());
            ambiguous_categories.push((entry.category, vec![entry.name]));
        }

        // Add matching tenant custom connectors (non-database ones)
        for tc in tenant_connectors {
            if tc.category == "connector/database" { continue; } // handled as tool_override above
            if terms_match_connector(&all_terms, tc) && !resolved.contains(&tc.name) {
                resolved.push(tc.name.clone());
            }
        }

        // Build clarifying question if multiple connectors in same category
        let clarifying = ambiguous_categories
            .iter()
            .find(|(_, names)| names.len() > 1)
            .map(|(cat, names)| {
                let display_cat = cat.strip_prefix("connector/").unwrap_or(cat);
                format!(
                    "You have multiple {} integrations installed: {}. Which one should this agent use?",
                    display_cat,
                    names.join(", ")
                )
            })
            .or_else(|| build_missing_connector_question(
                &needed_connector_categories,
                &missing_capabilities,
                installed,
                tenant_connectors,
            ));

        resolved.sort();
        resolved.dedup();
        tool_overrides.sort();
        tool_overrides.dedup();

        (resolved, tool_overrides, clarifying)
    }
}

fn build_missing_connector_question(
    needed_connector_categories: &[String],
    missing_capabilities: &[String],
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> Option<String> {
    for category in needed_connector_categories {
        let full_category = format!("connector/{}", category);
        let installed_builtin: Vec<&str> = BUILTIN_CONNECTORS.iter()
            .filter(|entry| entry.category == full_category)
            .filter(|entry| installed.iter().any(|name| name == entry.name))
            .map(|entry| entry.name)
            .collect();
        let installed_tenant: Vec<&str> = tenant_connectors.iter()
            .filter(|connector| connector.category == full_category)
            .map(|connector| connector.name.as_str())
            .collect();

        if installed_builtin.is_empty() && installed_tenant.is_empty() {
            let suggestions: Vec<&str> = BUILTIN_CONNECTORS.iter()
                .filter(|entry| entry.category == full_category)
                .map(|entry| entry.name)
                .take(3)
                .collect();
            let suggestion_text = if suggestions.is_empty() {
                "a custom connector".to_string()
            } else {
                suggestions.join(", ")
            };
            return Some(format!(
                "This sounds like it needs a {} connector, but none is installed. Should we use a custom database/API, or should you connect {}?",
                category,
                suggestion_text,
            ));
        }
    }

    if missing_capabilities.iter().any(|value| value == "custom_db") {
        return Some(
            "This may need a custom database connection. If you already have one registered, tell me its name; otherwise add it as a custom DB connector.".into()
        );
    }
    if missing_capabilities.iter().any(|value| value == "custom_api") {
        return Some(
            "This may need a custom API connection. If you already have one registered, tell me its name; otherwise add it as a custom API connector.".into()
        );
    }

    None
}

/// Returns true if any intent term meaningfully matches the connector's name/summary.
/// Uses proper tokenization (split on non-alphanumeric) rather than whitespace.
fn terms_match_connector(all_terms: &[&str], tc: &TenantConnector) -> bool {
    // Tokenize the summary into words
    let summary_words: Vec<String> = tc.summary
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(|s| s.to_lowercase())
        .collect();

    // Also include the connector name itself
    let name_lower = tc.name.to_lowercase();

    all_terms.iter().any(|term| {
        let term_lower = term.to_lowercase();
        // Exact name match
        term_lower == name_lower ||
        name_lower.contains(&term_lower) ||
        term_lower.contains(&name_lower) ||
        // Summary word match (both directions, min 4 chars to avoid noise)
        (term_lower.len() >= 4 && summary_words.iter().any(|w| {
            w.contains(&term_lower) || term_lower.contains(w.as_str())
        }))
    })
}

// ── PlanModeManager ────────────────────────────────────────────────────────

/// Manages the multi-turn plan mode conversation.
/// Each turn advances the session through PlanModePhase states.
pub struct PlanModeManager {
    gateway:        Arc<dyn LlmGateway>,
    store:          Arc<PostgresStore>,
    installs:       Arc<ConnectorInstallStore>,
    tools:          Arc<ToolRegistry>,
    extractor:      IntentExtractor,
    skill_registry: Option<Arc<tokio::sync::RwLock<crate::skills::registry::SkillRegistry>>>,
}

impl PlanModeManager {
    pub fn new(
        gateway:  Arc<dyn LlmGateway>,
        store:    Arc<PostgresStore>,
        installs: Arc<ConnectorInstallStore>,
        tools:    Arc<ToolRegistry>,
    ) -> Self {
        let extractor = IntentExtractor::new(Arc::clone(&gateway));
        Self { gateway, store, installs, tools, extractor, skill_registry: None }
    }

    pub fn with_skill_registry(
        mut self,
        registry: Arc<tokio::sync::RwLock<crate::skills::registry::SkillRegistry>>,
    ) -> Self {
        self.skill_registry = Some(registry);
        self
    }

    /// Build the clarification step queue for the given intent, store it in the
    /// session, and return the first question. Shared by handle_intent and
    /// handle_connector_clarification.
    async fn build_step_queue_and_ask(
        &self,
        session: &mut PlanModeSession,
        intent:  &serde_json::Value,
    ) -> String {
        let installed: Vec<String> = self.installs
            .list_for_tenant(&session.tenant_id).await
            .unwrap_or_default()
            .into_iter().map(|c| c.connector_type).collect();

        // Load existing roles on this agent so the step pipeline can ask
        // about workforce event filters and depends_on ordering
        let existing_role_names: Vec<String> = self.store
            .list_roles_for_agent(&session.tenant_id, &session.draft_agent.id).await
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.name)
            .collect();

        let steps = crate::agent::plan_mode_steps::generate_steps(
            intent,
            intent["category"].as_str().unwrap_or("general"),
            &installed,
            &existing_role_names,
        );

        session.pending_steps = steps.iter()
            .filter_map(|s| serde_json::to_value(s).ok())
            .collect();

        steps.first()
            .map(|s| s.question.clone())
            .unwrap_or_else(|| "Any constraints or rules for this agent?".into())
    }

    /// Look up the domain plan-mode skill for the given intent category.
    async fn domain_skill_text(&self, category: &str) -> Option<String> {
        let reg = self.skill_registry.as_ref()?.read().await;
        // Domain skills are named "planmode:<category>"
        let key = format!("planmode:{}", category);
        if let Some(skill) = reg.get(&key) {
            let text = skill.steps.iter()
                .map(|s| s.description())
                .collect::<Vec<_>>()
                .join("\n\n");
            return Some(text);
        }
        // Fallback: fuzzy match via aliases
        reg.find_matching(category).map(|skill| {
            skill.steps.iter()
                .map(|s| s.description())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
    }

    /// Create a new plan mode session for a tenant.
    pub fn new_session(&self, tenant_id: &str, agent_name: &str) -> PlanModeSession {
        let session_id = Uuid::new_v4().to_string();
        let agent_id   = Uuid::new_v4().to_string();
        let now        = Utc::now();

        let mut draft_agent = AgentDefinition::new(agent_id, tenant_id.to_string(), agent_name.to_string());
        draft_agent.memory_ref = format!("agent:{}", &draft_agent.id[..8]);

        PlanModeSession {
            id:           session_id,
            tenant_id:    tenant_id.to_string(),
            draft_agent,
            draft_role:   None,
            conversation: Vec::new(),
            phase:        PlanModePhase::CapturingIntent,
            intent_cache: None,
            pending_steps: Vec::new(),
            created_at:   now,
            updated_at:   now,
        }
    }

    /// Process one user turn.  Returns the assistant's reply and the updated session.
    pub async fn turn(
        &self,
        mut session: PlanModeSession,
        user_message: &str,
    ) -> Result<(String, PlanModeSession)> {
        session.conversation.push(PlanModeMessage {
            role:    "user".into(),
            content: user_message.to_string(),
        });

        let reply = match session.phase {
            PlanModePhase::CapturingIntent => {
                self.handle_intent(&mut session, user_message).await?
            }
            PlanModePhase::ResolvingConnectors => {
                // User answered the connector clarification question
                self.handle_connector_clarification(&mut session, user_message).await?
            }
            PlanModePhase::CapturingClarifications => {
                // User answered the combined trigger/output/multi-role questions
                self.handle_clarifications(&mut session, user_message).await?
            }
            PlanModePhase::CapturingConstraints => {
                self.handle_constraints(&mut session, user_message).await?
            }
            PlanModePhase::Reviewing => {
                self.handle_review(&mut session, user_message).await?
            }
            PlanModePhase::Complete => {
                "This session is complete. The agent has been saved.".into()
            }
        };

        session.conversation.push(PlanModeMessage {
            role:    "assistant".into(),
            content: reply.clone(),
        });
        session.updated_at = Utc::now();

        Ok((reply, session))
    }

    // ── Phase handlers ─────────────────────────────────────────────────────

    async fn handle_intent(
        &self,
        session: &mut PlanModeSession,
        description: &str,
    ) -> Result<String> {
        // Load tenant's custom connections upfront — used for both context injection
        // and connector resolution
        let installed: Vec<String> = self.installs
            .list_for_tenant(&session.tenant_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.connector_type)
            .collect();

        let tenant_connectors = self.store
            .list_tenant_connectors(&session.tenant_id)
            .await
            .unwrap_or_default();

        let capability_directory = build_capability_directory(&self.tools, &installed, &tenant_connectors);
        let initial_intent = self.extractor.extract_initial(
            &session.id,
            &session.tenant_id,
            description,
            &capability_directory,
        ).await?;
        let detail_context =
            build_detailed_capability_context(&self.tools, &initial_intent, &installed, &tenant_connectors);
        let intent = if detail_context.trim().is_empty() {
            initial_intent
        } else {
            self.extractor.refine(
                &session.id,
                &session.tenant_id,
                description,
                &initial_intent,
                &detail_context,
            ).await?
        };

        // Store intent in the draft role
        let role_id = Uuid::new_v4().to_string();
        let mut role = AgentRole::new(
            role_id,
            session.draft_agent.id.clone(),
            session.tenant_id.clone(),
            "Primary Role".into(),
        );
        role.purpose = description.to_string();
        role.role_category = RoleCategory::from_slug(
            intent["category"].as_str().unwrap_or("general"),
        );
        apply_role_policy_defaults(&mut session.draft_agent, &mut role);

        // Resolve connectors and tool overrides
        let (resolved_connectors, tool_overrides, clarifying_q) =
            ConnectorResolver::resolve(&intent, &installed, &tenant_connectors).await;

        // Set connectors on agent (allowed universe) and role (relevant subset)
        session.draft_agent.connectors = resolved_connectors.clone();
        role.connectors = resolved_connectors.clone();

        let mut inferred_tools = inferred_preferred_tools(&self.tools, &intent);
        for tool_override in &tool_overrides {
            if !inferred_tools.contains(tool_override) {
                inferred_tools.push(tool_override.clone());
            }
        }
        if !inferred_tools.is_empty() {
            role.tools = inferred_tools;
        }

        // Build execution guidelines from actions
        let mut guidelines: Vec<String> = Vec::new();
        if let Some(actions) = intent["actions"].as_array() {
            guidelines.extend(actions.iter().filter_map(|a| a.as_str().map(String::from)));
        }
        // Add tool hints for external connections
        for override_spec in &tool_overrides {
            if let Some(db_name) = override_spec.strip_prefix("external_db:") {
                guidelines.push(format!(
                    "Use tool external_db with db='{}'. Start by calling operation='schema' to discover tables.",
                    db_name
                ));
            } else if let Some(api_name) = override_spec.strip_prefix("external_api:") {
                guidelines.push(format!(
                    "Use tool external_api with api='{}' for all HTTP calls to this backend.",
                    api_name
                ));
            }
        }
        // Populate structured ExecutionGuidelines from extracted actions + tool overrides
        for item in guidelines {
            role.execution_guidelines.add_rule(
                crate::agent::definition::GuidelineRule::always(item)
            );
        }
        apply_execution_hints(&mut role, &intent);

        // Apply trigger from intent (with confidence) — will be confirmed in clarifications phase
        let (parsed_trigger, confidence) = intent_to_trigger(&intent);
        role.trigger = parsed_trigger;
        role.trigger.confidence = confidence;

        session.draft_role = Some(role);

        // Cache the extracted intent — used throughout all subsequent phases
        session.intent_cache = Some(intent.clone());

        if let Some(q) = clarifying_q {
            session.phase = PlanModePhase::ResolvingConnectors;
            return Ok(q);
        }

        // Move to the combined clarifications phase — steps queue drives it
        session.phase = PlanModePhase::CapturingClarifications;
        Ok(self.build_step_queue_and_ask(session, &intent).await)
    }
    async fn handle_connector_clarification(
        &self,
        session: &mut PlanModeSession,
        answer: &str,
    ) -> Result<String> {
        // User picked a specific connector — update role.connectors
        if let Some(role) = session.draft_role.as_mut() {
            let answer_lower = answer.to_lowercase();
            // Find matching built-in connector by name mention
            for entry in BUILTIN_CONNECTORS {
                if answer_lower.contains(entry.name) || answer_lower.contains(entry.summary.to_lowercase().as_str()) {
                    // Replace category duplicates with the chosen one
                    role.connectors.retain(|c| {
                        BUILTIN_CONNECTORS
                            .iter()
                            .find(|e| e.name == c.as_str())
                            .map(|e| e.category != entry.category)
                            .unwrap_or(true)
                    });
                    role.connectors.push(entry.name.to_string());
                    session.draft_agent.connectors = role.connectors.clone();
                    break;
                }
            }
        }

        // Regenerate the step queue now that the connector is confirmed
        let intent = session.intent_cache.clone()
            .unwrap_or_else(|| serde_json::json!({ "trigger_hint": "manual" }));
        session.phase = PlanModePhase::CapturingClarifications;
        Ok(self.build_step_queue_and_ask(session, &intent).await)
    }

    async fn handle_clarifications(
        &self,
        session: &mut PlanModeSession,
        answer: &str,
    ) -> Result<String> {
        use crate::agent::plan_mode_steps::{ClarificationStep, parse_and_apply};

        // Pop the front step — that's the one we're answering now
        let current_step: Option<ClarificationStep> = if !session.pending_steps.is_empty() {
            let raw = session.pending_steps.remove(0);
            serde_json::from_value(raw).ok()
        } else {
            None
        };

        if let Some(step) = current_step {
            // Parse and apply the answer for this step
            let mut agent_constraints = session.draft_agent.constraints.clone();
            let mut pending_roles: Option<Vec<serde_json::Value>> = None;

            let summary = if let Some(role) = session.draft_role.as_mut() {
                parse_and_apply(
                    &step,
                    answer,
                    role,
                    &mut agent_constraints,
                    session.intent_cache.as_ref().unwrap_or(&serde_json::json!({})),
                    &mut pending_roles,
                )
            } else {
                "Step processed.".into()
            };

            session.draft_agent.constraints = agent_constraints;

            // If user chose to split roles, stash pending responsibilities
            if let Some(remaining) = pending_roles {
                if !session.draft_agent.memory_ref.contains("|pending_roles:") {
                    let meta = session.draft_agent.memory_ref.clone();
                    session.draft_agent.memory_ref = format!(
                        "{}|pending_roles:{}", meta,
                        serde_json::to_string(&remaining).unwrap_or_default()
                    );
                }
            }

            // Advance to next step or move to review
            if let Some(next_raw) = session.pending_steps.first() {
                if let Ok(next_step) = serde_json::from_value::<ClarificationStep>(next_raw.clone()) {
                    // Show confirmation + next question
                    return Ok(format!("✓ {}\n\n{}", summary, next_step.question));
                }
            }

            // No more steps — inject domain skill execution brief then go to constraints
            let category = session.intent_cache.as_ref()
                .and_then(|i| i["category"].as_str())
                .unwrap_or("general");

            if let Some(skill_text) = self.domain_skill_text(category).await {
                let brief: String = skill_text.lines()
                    .skip_while(|l| !l.starts_with("EXECUTION BRIEF"))
                    .collect::<Vec<_>>().join("\n");
                if !brief.is_empty() {
                    if let Some(role) = session.draft_role.as_mut() {
                        let parsed = crate::agent::definition::ExecutionGuidelines::from_skill_text(&brief);
                        role.execution_guidelines.extend_dedup(parsed);
                    }
                }
                // Also auto-generate default completion criteria if none yet
                if let Some(role) = session.draft_role.as_mut() {
                    if role.execution_guidelines.completion_criteria.is_empty() {
                        let defaults = crate::agent::plan_mode_steps::default_completion_criteria(role);
                        for c in defaults { role.execution_guidelines.add_completion(c); }
                    }
                }
            }

            session.phase = PlanModePhase::Reviewing;
            return Ok(format!("✓ {}\n\n{}", summary, self.build_review_summary(session)));
        }

        // pending_steps was already empty — go straight to review
        session.phase = PlanModePhase::Reviewing;
        Ok(self.build_review_summary(session))
    }
    async fn handle_constraints(
        &self,
        session: &mut PlanModeSession,
        answer: &str,
    ) -> Result<String> {
        let lower = answer.to_lowercase();
        let is_empty = lower.contains("no constraint") || lower.contains("none")
            || lower.contains("n/a") || lower.contains("defaults") || answer.trim().len() < 4;

        if !is_empty {
            // Parse domain skill answers + user constraints into structured guidelines
            let from_user = crate::agent::definition::ExecutionGuidelines::from_user_constraints(answer);
            if let Some(role) = session.draft_role.as_mut() {
                role.execution_guidelines.extend_dedup(from_user);
            }

            // Also parse plain constraint strings into agent.constraints
            // (for hard rules that should be visible in the review card)
            let constraint_items: Vec<String> = answer
                .split(&[',', ';', '\n'][..])
                .map(|s| s.trim().trim_end_matches('.').to_string())
                .filter(|s| s.len() > 8)
                .filter(|s| {
                    let l = s.to_lowercase();
                    !l.starts_with("mandatory") && !l.starts_with("before confirm")
                        && !l.starts_with("execution brief")
                })
                .collect();
            session.draft_agent.constraints.extend(constraint_items);
        }

        session.phase = PlanModePhase::Reviewing;
        Ok(self.build_review_summary(session))
    }

    /// Public wrapper for build_review_summary — used by the template fast-path in routes.rs
    pub fn build_review_summary_pub(&self, session: &PlanModeSession) -> String {
        self.build_review_summary(session)
    }

    fn build_review_summary(&self, session: &PlanModeSession) -> String {
        let agent = &session.draft_agent;
        let role  = match session.draft_role.as_ref() {
            Some(r) => r,
            None    => return "Configuration incomplete — no role defined.".into(),
        };

        let trigger_desc = match &role.trigger.trigger_type {
            TriggerType::Webhook  => format!(
                "triggered by {} {}",
                role.trigger.source_connector.as_deref().unwrap_or("external event"),
                role.trigger.event_filter.as_deref().unwrap_or("")
            ),
            TriggerType::Schedule => format!(
                "runs on schedule: {}",
                role.trigger.cron.as_deref().unwrap_or("daily")
            ),
            TriggerType::UserMessage  => "runs when you ask it to".into(),
            TriggerType::Manual       => "runs on-demand".into(),
            TriggerType::WorkforceEvent => {
                match &role.trigger.workforce_event_filter {
                    Some(f) if f.contains("role_name") => {
                        // Extract the role name from the filter expression
                        let name = f
                            .split("role_name == '").nth(1)
                            .and_then(|s| s.split('\'').next())
                            .unwrap_or("another role");
                        format!("runs after '{}' completes", name)
                    }
                    Some(f) => format!("runs on workforce event: {}", f),
                    None => "runs after another role completes".into(),
                }
            }
        };

        let connectors = if role.connectors.is_empty() {
            "none (uses built-in tools only)".into()
        } else {
            role.connectors.join(", ")
        };

        // Show external databases and APIs from tool overrides
        let tools_section = if role.tools.is_empty() {
            String::new()
        } else {
            let mut parts: Vec<String> = Vec::new();
            for t in &role.tools {
                if let Some(db_name) = t.strip_prefix("external_db:") {
                    parts.push(format!("database '{}'", db_name));
                } else if let Some(api_name) = t.strip_prefix("external_api:") {
                    parts.push(format!("REST API '{}'", api_name));
                } else {
                    parts.push(t.clone());
                }
            }
            format!("\n**Your connections:** {}", parts.join(", "))
        };

        let constraints = if agent.constraints.is_empty() {
            "none".into()
        } else {
            agent.constraints.join("; ")
        };

        // Show which compliance services will be active for this category
        let services_line = {
            let category = session.intent_cache.as_ref()
                .and_then(|i| i["category"].as_str())
                .unwrap_or("general");
            let services = active_services_for_category(category);
            if services.is_empty() {
                String::new()
            } else {
                format!("\n**Active services:** {}", services.join(", "))
            }
        };

        format!(
            "Here's what I've configured:\n\n\
            **Agent:** {name}\n\
            **Role:** {purpose}\n\
            **Trigger:** {trigger}\n\
            **Connectors:** {connectors}{tools}\n\
            **Output:** {output}\n\
            **Constraints:** {constraints}{services}\n\n\
            Does this look right? Say **yes** to save, or tell me what to change.",
            name        = agent.name,
            purpose     = role.purpose,
            trigger     = trigger_desc,
            connectors  = connectors,
            tools       = tools_section,
            output      = role.output_spec.description,
            constraints = constraints,
            services    = services_line,
        )
    }

    async fn handle_review(
        &self,
        session: &mut PlanModeSession,
        answer: &str,
    ) -> Result<String> {
        let lower = answer.to_lowercase();
        if lower.contains("yes") || lower.contains("save") || lower.contains("looks good")
            || lower.contains("correct") || lower.contains("confirmed")
        {
            session.phase = PlanModePhase::Complete;
            return Ok("✓ Agent saved. You can find it in your agent list. \
                       Add more roles anytime from the agent settings page.".into());
        }

        // User wants to change something — re-extract from their correction
        session.phase = PlanModePhase::CapturingIntent;
        let reply = self.handle_intent(session, answer).await?;
        Ok(format!(
            "Updated. Let me reconfigure based on your correction.\n\n{}",
            reply
        ))
    }

    /// Finalise and save the session — creates AgentDefinition + AgentRole in DB.
    pub async fn save(&self, mut session: PlanModeSession) -> Result<(AgentDefinition, AgentRole)> {
        let mut agent = session.draft_agent.clone();
        agent.status = AgentDefinitionStatus::Active;
        agent.updated_at = Utc::now();

        self.store.upsert_agent_definition(&agent).await?;

        let role = match session.draft_role.take() {
            Some(mut r) => {
                r.status   = RoleStatus::Active;
                r.updated_at = Utc::now();

                // Resolve "name:Role Name" hints in depends_on_role_id to actual IDs
                if let Some(hint) = r.trigger.depends_on_role_id.clone() {
                    if let Some(name) = hint.strip_prefix("name:") {
                        let existing = self.store
                            .list_roles_for_agent(&agent.tenant_id, &agent.id).await
                            .unwrap_or_default();
                        if let Some(found) = existing.iter().find(|er| {
                            er.name.to_lowercase() == name.to_lowercase()
                        }) {
                            r.trigger.depends_on_role_id = Some(found.id.clone());
                        } else {
                            // Named role not found — clear the hint rather than save a bad ref
                            r.trigger.depends_on_role_id = None;
                            tracing::warn!(
                                role_name = %name,
                                "depends_on_role_id: named role not found — cleared"
                            );
                        }
                    }
                }

                self.store.upsert_agent_role(&r).await?;

                // Sync workforce event subscription if needed
                crate::events::workforce::sync_subscriptions_for_role(&r, &self.store).await?;
                r
            }
            None => {
                anyhow::bail!("cannot save plan mode session with no role defined")
            }
        };

        Ok((agent, role))
    }
}

// ── Free helper functions ───────────────────────────────────────────────────

fn apply_role_policy_defaults(agent: &mut AgentDefinition, role: &mut AgentRole) {
    if agent.persona.trim().is_empty() {
        agent.persona = role.role_category.default_persona().to_string();
    }

    role.memory_scope = role.role_category.default_memory_scope();

    if role.execution_limits == crate::agent::definition::ExecutionLimits::default() {
        role.execution_limits = role.role_category.default_execution_limits();
    }
}

/// Build a human-readable summary of the tenant's custom connections
/// to inject into the IntentExtractor prompt so the LLM knows what's available
/// and can match user descriptions to registered names exactly.
fn build_custom_context(_installed: &[String], tenant_connectors: &[TenantConnector]) -> String {
    if tenant_connectors.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();

    // Databases (tool: external_db)
    let dbs: Vec<&TenantConnector> = tenant_connectors.iter()
        .filter(|tc| tc.category == "connector/database")
        .collect();
    if !dbs.is_empty() {
        lines.push("Databases (use external_db tool, reference by name):".into());
        for db in &dbs {
            lines.push(format!("  - name='{}' — {}", db.name, db.summary));
        }
    }

    // REST APIs (tool: external_api)
    let apis: Vec<&TenantConnector> = tenant_connectors.iter()
        .filter(|tc| !tc.category.contains("database") && !tc.category.contains("mcp"))
        .collect();
    if !apis.is_empty() {
        lines.push("Custom REST APIs (use external_api tool, reference by name):".into());
        for api in &apis {
            lines.push(format!("  - name='{}' — {}", api.name, api.summary));
        }
    }

    // MCP servers (use as named connector)
    let mcps: Vec<&TenantConnector> = tenant_connectors.iter()
        .filter(|tc| tc.category.contains("mcp"))
        .collect();
    if !mcps.is_empty() {
        lines.push("MCP servers (available as connector tools):".into());
        for mcp in &mcps {
            lines.push(format!("  - name='{}' — {}", mcp.name, mcp.summary));
        }
    }

    lines.join("\n")
}

fn build_capability_directory(
    registry: &ToolRegistry,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    let mut lines: Vec<String> = vec![
        "Use categories first. Do not assume every connector is installed or every tool is needed.".into(),
        "Only installed connectors and registered custom connections are immediately usable.".into(),
        "If no installed connector fits, prefer missing_capabilities such as custom_db, custom_api, or connector/<category>.".into(),
    ];

    let mut tool_categories: Vec<(String, Vec<String>)> = registry.by_category()
        .into_iter()
        .filter(|(category, _)| !category.starts_with("connector/"))
        .map(|(category, names)| {
            (
                category.to_string(),
                names.into_iter()
                    .filter(|name| !name.starts_with("request_more_") && *name != "list_connectors_in_category")
                    .take(4)
                    .map(String::from)
                    .collect(),
            )
        })
        .filter(|(_, names)| !names.is_empty())
        .collect();
    tool_categories.sort_by(|a, b| a.0.cmp(&b.0));
    lines.push("Core tool categories (examples only, more detail comes later if relevant):".into());
    for (category, names) in tool_categories {
        lines.push(format!("  - {}: {}", category, names.join(", ")));
    }

    let mut connector_groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for entry in BUILTIN_CONNECTORS {
        let cat = entry.category.strip_prefix("connector/").unwrap_or(entry.category);
        let status = if installed.iter().any(|name| name == entry.name) { "installed" } else { "available" };
        connector_groups.entry(cat).or_default()
            .push(format!("{} ({}, {})", entry.name, status, entry.summary));
    }
    lines.push("Built-in connector categories:".into());
    for (category, connectors) in connector_groups {
        let preview = connectors.into_iter().take(4).collect::<Vec<_>>();
        lines.push(format!("  - {}: {}", category, preview.join("; ")));
    }

    let custom_context = build_custom_context(installed, tenant_connectors);
    if !custom_context.is_empty() {
        lines.push("Tenant custom connections:".into());
        lines.push(custom_context);
    }

    lines.join("\n")
}

fn build_detailed_capability_context(
    registry: &ToolRegistry,
    intent: &serde_json::Value,
    installed: &[String],
    tenant_connectors: &[TenantConnector],
) -> String {
    let mut lines: Vec<String> = Vec::new();

    let tool_categories: Vec<String> = intent["preferred_tool_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    for category in tool_categories {
        let specs = registry.tool_specs_for_category(&category);
        if specs.is_empty() {
            continue;
        }
        lines.push(format!("Detailed tools for category '{}':", category));
        for spec in specs.into_iter().take(8) {
            let params = spec.parameters["required"].as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let required = if params.is_empty() { "none".to_string() } else { params };
            lines.push(format!(
                "  - {}: {} | required args: {}",
                spec.name,
                spec.description,
                required,
            ));
        }
    }

    let mut requested_connector_names: Vec<String> = intent["candidate_connectors"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let requested_connector_categories: Vec<String> = intent["needed_connector_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    for category in &requested_connector_categories {
        for entry in BUILTIN_CONNECTORS {
            let cat = entry.category.strip_prefix("connector/").unwrap_or(entry.category);
            if cat == category && !requested_connector_names.iter().any(|name| name == entry.name) {
                requested_connector_names.push(entry.name.to_string());
            }
        }
    }

    for connector_name in requested_connector_names {
        if let Some(entry) = BUILTIN_CONNECTORS.iter().find(|entry| entry.name == connector_name) {
            let installed_status = if installed.iter().any(|name| name == entry.name) { "installed" } else { "not_installed" };
            lines.push(format!(
                "Connector '{}': category={} status={} summary={} operations={}",
                entry.name,
                entry.category,
                installed_status,
                entry.summary,
                entry.operations.join("; "),
            ));
        } else if let Some(connector) = tenant_connectors.iter().find(|connector| connector.name == connector_name) {
            let operations = connector.endpoints.iter()
                .map(|endpoint| endpoint.path.as_str())
                .take(6)
                .collect::<Vec<_>>();
            let operation_text = if operations.is_empty() {
                "custom endpoints configured".to_string()
            } else {
                operations.join(", ")
            };
            lines.push(format!(
                "Tenant connector '{}': category={} summary={} endpoints={}",
                connector.name,
                connector.category,
                connector.summary,
                operation_text,
            ));
        }
    }

    let missing_capabilities: Vec<String> = intent["missing_capabilities"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if !missing_capabilities.is_empty() {
        lines.push(format!(
            "Missing capability hints already inferred: {}",
            missing_capabilities.join(", ")
        ));
    }

    lines.join("\n")
}

fn inferred_preferred_tools(registry: &ToolRegistry, intent: &serde_json::Value) -> Vec<String> {
    intent["preferred_tools"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str())
                .filter(|tool_name| registry.get(tool_name).is_some())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn apply_execution_hints(role: &mut AgentRole, intent: &serde_json::Value) {
    let workflow_outline: Vec<String> = intent["workflow_outline"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    for item in workflow_outline.into_iter().take(5) {
        role.execution_guidelines.add_priority(item);
    }

    let tool_categories: Vec<String> = intent["preferred_tool_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if !tool_categories.is_empty() {
        role.execution_guidelines.add_rule(
            crate::agent::definition::GuidelineRule::always(format!(
                "Prefer these tool categories when relevant: {}.",
                tool_categories.join(", ")
            ))
        );
    }

    let connector_categories: Vec<String> = intent["needed_connector_categories"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if !connector_categories.is_empty() {
        role.execution_guidelines.add_rule(
            crate::agent::definition::GuidelineRule::always(format!(
                "Prefer connectors from these categories when relevant: {}.",
                connector_categories.join(", ")
            ))
        );
    }
}

/// Parse a natural-language trigger description into a `TriggerDef`.
pub(crate) fn parse_trigger_from_text(answer: &str) -> TriggerDef {
    let lower = answer.to_lowercase();

    // Workforce event — "after another role", "when X finishes/completes"
    if lower.contains("after") && (lower.contains("role") || lower.contains("finish") || lower.contains("complet")) {
        return TriggerDef {
            trigger_type:     TriggerType::WorkforceEvent,
            cron:             None,
            source_connector: None,
            event_filter:     None,
            input_mapping:    None,
            ..Default::default()
        };
    }

    // Schedule — contains time/day keywords
    let schedule_keywords = ["every", "daily", "weekly", "monthly", "hourly",
        "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
        "midnight", "noon", "morning", "evening", "at ", "am", "pm", "cron"];
    if schedule_keywords.iter().any(|kw| lower.contains(kw)) {
        let cron = natural_to_cron(&lower);
        return TriggerDef {
            trigger_type:     TriggerType::Schedule,
            cron:             Some(cron),
            source_connector: None,
            event_filter:     None,
            input_mapping:    None,
            ..Default::default()
        };
    }

    // Webhook — "when X happens", "on new Y", connector name mentioned
    let webhook_keywords = ["when ", "on new", "on a new", "webhook",
        "salesforce", "hubspot", "github", "zendesk", "stripe",
        "intercom", "freshdesk", "pagerduty", "created", "updated", "received"];
    if webhook_keywords.iter().any(|kw| lower.contains(kw)) {
        // Try to detect the source connector
        let connector_names = [
            "salesforce", "hubspot", "github", "zendesk", "slack", "jira",
            "notion", "gmail", "stripe", "intercom", "freshdesk", "pagerduty",
            "servicenow", "greenhouse", "docusign", "quickbooks", "dbt_cloud", "outlook",
        ];
        let source_connector = connector_names.iter()
            .find(|&&c| lower.contains(c))
            .map(|&c| c.to_string());

        // Extract event filter (e.g. "lead created" → "lead_created")
        let event_filter = extract_event_filter(&lower);

        return TriggerDef {
            trigger_type:     TriggerType::Webhook,
            cron:             None,
            source_connector,
            event_filter,
            input_mapping:    None,
            ..Default::default()
        };
    }

    // User message / on-demand
    if lower.contains("ask") || lower.contains("message") || lower.contains("chat") {
        return TriggerDef {
            trigger_type:     TriggerType::UserMessage,
            cron:             None,
            source_connector: None,
            event_filter:     None,
            input_mapping:    None,
            ..Default::default()
        };
    }

    // Default: manual
    TriggerDef {
        trigger_type:     TriggerType::Manual,
        cron:             None,
        source_connector: None,
        event_filter:     None,
        input_mapping:    None,
        ..Default::default()
    }
}

/// Convert natural-language schedule descriptions to cron expressions.
pub(crate) fn natural_to_cron(text: &str) -> String {
    let lower = text.to_lowercase();

    // Specific time extraction: "at 9am", "at 14:00", "at 3pm"
    let hour = if let Some(h) = extract_hour(&lower) { h } else { 9u32 };

    if lower.contains("every hour") || lower.contains("hourly") {
        return format!("0 * * * *");
    }
    if lower.contains("every 30 min") || lower.contains("every half hour") {
        return format!("*/30 * * * *");
    }
    if lower.contains("midnight") { return "0 0 * * *".into(); }
    if lower.contains("noon")     { return "0 12 * * *".into(); }

    // Day of week
    let day = if lower.contains("monday")    { Some("1") }
         else if lower.contains("tuesday")   { Some("2") }
         else if lower.contains("wednesday") { Some("3") }
         else if lower.contains("thursday")  { Some("4") }
         else if lower.contains("friday")    { Some("5") }
         else if lower.contains("saturday")  { Some("6") }
         else if lower.contains("sunday")    { Some("0") }
         else { None };

    if let Some(d) = day {
        return format!("0 {} * * {}", hour, d);
    }
    if lower.contains("weekly") { return format!("0 {} * * 1", hour); }
    if lower.contains("daily") || lower.contains("every day") {
        return format!("0 {} * * *", hour);
    }
    if lower.contains("monthly") || lower.contains("every month") {
        return format!("0 {} 1 * *", hour);
    }
    if lower.contains("every") && lower.contains("hour") {
        if let Some(n) = extract_number(&lower) {
            return format!("0 */{} * * *", n);
        }
    }

    // Default: daily at 9am
    format!("0 {} * * *", hour)
}

fn extract_hour(text: &str) -> Option<u32> {
    // Match "9am", "9 am", "14:00", "3pm", "3 pm"
    let re_12h = regex::Regex::new(r"(\d{1,2})\s*(am|pm)").ok()?;
    let re_24h = regex::Regex::new(r"(\d{1,2}):(\d{2})").ok()?;

    if let Some(cap) = re_24h.captures(text) {
        let h: u32 = cap[1].parse().ok()?;
        return Some(h);
    }
    if let Some(cap) = re_12h.captures(text) {
        let h: u32 = cap[1].parse().ok()?;
        let is_pm = &cap[2] == "pm";
        return Some(if is_pm && h != 12 { h + 12 } else if !is_pm && h == 12 { 0 } else { h });
    }
    None
}

fn extract_number(text: &str) -> Option<u32> {
    text.split_whitespace()
        .find_map(|w| w.parse::<u32>().ok())
}

fn extract_event_filter(text: &str) -> Option<String> {
    // "lead created" → "lead_created", "pr opened" → "pr_opened", etc.
    let patterns = [
        ("lead created", "lead_created"),
        ("lead updated", "lead_updated"),
        ("opportunity", "opportunity_updated"),
        ("ticket created", "ticket_created"),
        ("ticket updated", "ticket_updated"),
        ("pr opened", "pull_request"),
        ("pull request", "pull_request"),
        ("issue created", "issues"),
        ("payment failed", "payment_intent.payment_failed"),
        ("subscription cancelled", "customer.subscription.deleted"),
        ("invoice failed", "invoice.payment_failed"),
        ("dispute", "charge.dispute.created"),
    ];
    patterns.iter()
        .find(|(pattern, _)| text.contains(pattern))
        .map(|(_, filter)| filter.to_string())
}

// ── Intent-to-trigger converter ────────────────────────────────────────────

/// Build a TriggerDef from the IntentExtractor JSON output.
/// Returns the trigger and its confidence level.
pub(crate) fn intent_to_trigger(
    intent: &serde_json::Value,
) -> (TriggerDef, crate::agent::definition::TriggerConfidence) {
    use crate::agent::definition::TriggerConfidence;

    let confidence = match intent["trigger_confidence"].as_str().unwrap_or("medium") {
        "high"   => TriggerConfidence::High,
        "low"    => TriggerConfidence::Low,
        _        => TriggerConfidence::Medium,
    };

    let trigger_type = match intent["trigger_hint"].as_str().unwrap_or("manual") {
        "schedule"    => TriggerType::Schedule,
        "webhook"     => TriggerType::Webhook,
        "user_message" => TriggerType::UserMessage,
        _             => TriggerType::Manual,
    };

    let trigger = match trigger_type {
        TriggerType::Schedule => TriggerDef {
            trigger_type: TriggerType::Schedule,
            cron: intent["trigger_cron"].as_str().map(String::from),
            source_connector: None,
            event_filter: None,
            input_mapping: None,
            confidence: confidence.clone(),
            ..Default::default()
        },
        TriggerType::Webhook => TriggerDef {
            trigger_type: TriggerType::Webhook,
            source_connector: intent["trigger_source"].as_str().map(String::from),
            event_filter: intent["trigger_event"].as_str().map(String::from),
            cron: None,
            input_mapping: None,
            confidence: confidence.clone(),
            ..Default::default()
        },
        other => TriggerDef {
            trigger_type: other,
            cron: None,
            source_connector: None,
            event_filter: None,
            input_mapping: None,
            confidence: confidence.clone(),
            ..Default::default()
        },
    };

    (trigger, confidence)
}

/// Build the combined clarification question shown after intent extraction.
/// Covers trigger confirmation (if needed), output questions, and multi-role suggestion.
pub(crate) fn build_clarification_question(intent: &serde_json::Value) -> String {
    use crate::agent::definition::TriggerConfidence;

    let mut parts: Vec<String> = Vec::new();

    // Multi-role suggestion
    if intent["multi_role_suggested"].as_bool().unwrap_or(false) {
        if let Some(reason) = intent["multi_role_reason"].as_str() {
            let names: Vec<&str> = intent["responsibilities"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|r| r["name"].as_str()).collect())
                .unwrap_or_default();
            parts.push(format!(
                "**I see {} distinct responsibilities** — {}\n\n\
                 • **A) One role** — simpler, but mixes concerns\n\
                 • **B) {} separate roles** (recommended) — cleaner, easier to debug\n\
                 Which do you prefer?",
                names.len(),
                reason,
                names.len(),
            ));
        }
    }

    // Trigger confirmation (only if not high confidence)
    let confidence = intent["trigger_confidence"].as_str().unwrap_or("medium");
    if confidence != "high" {
        if let Some(q) = intent["trigger_confirmation"].as_str() {
            parts.push(q.to_string());
        } else {
            // Fallback: build confirmation from what we parsed
            let trigger_hint = intent["trigger_hint"].as_str().unwrap_or("manual");
            let cron = intent["trigger_cron"].as_str();
            match (trigger_hint, cron) {
                ("schedule", Some(c)) => {
                    parts.push(format!(
                        "**When should this run?** I guessed: `{}` — is that right? \
                         Or describe it differently (e.g. 'Every weekday at 8am London time').",
                        c
                    ));
                }
                ("schedule", None) => {
                    parts.push("**When should this run?** e.g. 'Every Monday at 9am', 'Daily at midnight'.".into());
                }
                ("webhook", _) => {
                    let src = intent["trigger_source"].as_str().unwrap_or("a connector");
                    let evt = intent["trigger_event"].as_str().unwrap_or("an event");
                    parts.push(format!(
                        "**Trigger confirmation:** Run when {} fires `{}`? Or describe the trigger.",
                        src, evt
                    ));
                }
                _ => {
                    parts.push("**When should this run?** Schedule / webhook / on-demand / after another role?".into());
                }
            }
        }
    }

    // Output questions
    // FIX: use an explicit has_output_questions flag so we always fall through
    // to the fallback when the LLM returns an empty array
    let output_questions: Vec<&str> = intent["output_questions"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|q| q.as_str()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    if !output_questions.is_empty() {
        parts.push(format!(
            "**Output details:**\n{}",
            output_questions.iter().map(|q| format!("- {}", q)).collect::<Vec<_>>().join("\n")
        ));
    } else {
        // Fallback — always ask if LLM returned no output questions or empty array
        let hint = intent["output_hint"].as_str().unwrap_or("workspace");
        let dest = intent["output_destination_hint"].as_str().unwrap_or("");
        if dest.is_empty() {
            let q = match hint {
                "email_draft" | "email_send" => "Where should the emails go — drafts in workspace, or sent via Gmail/Outlook?",
                "connector_record"           => "Which record should I update, and which field?",
                "slack_message"              => "Which Slack channel?",
                "report"                     => "Where should the report be saved? (e.g. workspace/reports/ or email to stakeholders)",
                "notification"               => "Where should notifications go — Slack, email, or in-app?",
                _                            => "Where should the output go, and in what format?",
            };
            parts.push(format!("**Output:** {}", q));
        }
        // If dest is known, no question needed — output is clear enough
    }

    if parts.is_empty() {
        "How should this run and where should the output go?".into()
    } else {
        parts.join("\n\n")
    }
}

/// Returns the list of compliance/platform services that will be automatically
/// active for a given job category. Shown in the review summary so users know
/// what's running on their behalf.
fn active_services_for_category(category: &str) -> Vec<&'static str> {
    match category {
        "customer_support" => vec![
            "SLA tracking (1hr first-response)",
            "PII redaction",
            "Citation recording",
            "Human review queue",
        ],
        "sales_revops" => vec![
            "PII redaction",
            "Citation recording",
            "Human review queue",
        ],
        "finance_accounting" => vec![
            "PII redaction",
            "Citation recording",
            "Evidence packaging",
            "Human review queue",
        ],
        "legal_contract" => vec![
            "PII redaction",
            "Citation recording",
            "Evidence packaging",
            "Human review queue",
        ],
        "hr_people_ops" => vec![
            "PII redaction",
            "Citation recording",
            "Human review queue",
        ],
        "devops" | "it_ops_itsm" => vec![
            "SLA tracking",
            "Citation recording",
            "Evidence packaging",
            "Human review queue",
        ],
        "research_analyst" => vec![
            "PII redaction",
            "Citation recording",
            "Evidence packaging",
            "Human review queue",
        ],
        "software_engineer" => vec![
            "Human review queue",
        ],
        _ => vec![],
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connector_resolver_matches_salesforce() {
        let intent = serde_json::json!({
            "data_sources": ["Salesforce CRM leads"],
            "write_targets": ["Salesforce"],
            "actions": ["query lead records", "update lead description"],
        });
        let installed = vec!["salesforce".into(), "slack".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, clarifying) = rt.block_on(
            ConnectorResolver::resolve(&intent, &installed, &[])
        );
        assert!(resolved.contains(&"salesforce".to_string()));
        assert!(clarifying.is_none());
    }

    #[test]
    fn test_connector_resolver_no_installed_match() {
        let intent = serde_json::json!({
            "data_sources": ["Salesforce"],
            "write_targets": [],
            "actions": ["query records"],
        });
        let installed: Vec<String> = vec!["slack".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, _) = rt.block_on(
            ConnectorResolver::resolve(&intent, &installed, &[])
        );
        assert!(!resolved.contains(&"salesforce".to_string()));
    }

    #[test]
    fn test_connector_resolver_tenant_connector_matched() {
        let intent = serde_json::json!({
            "data_sources": ["Acme ERP orders"],
            "write_targets": [],
            "actions": ["query orders"],
        });
        let installed: Vec<String> = vec![];
        let tc = TenantConnector {
            id:                  "tc-1".into(),
            tenant_id:           "t-1".into(),
            name:                "acme_erp".into(),
            category:            "connector/erp".into(),
            base_url:            "https://erp.acme.com".into(),
            auth_type:           ConnectorAuthType::Bearer,
            auth_credential_key: None,
            source:              crate::agent::definition::ConnectorSource::Manual,
            source_docs:         None,
            endpoints:           Vec::new(),
            summary:             "Acme ERP: orders inventory customers".into(),
            created_at:          Utc::now(),
            updated_at:          Utc::now(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, _) = rt.block_on(
            ConnectorResolver::resolve(&intent, &installed, &[tc])
        );
        assert!(resolved.contains(&"acme_erp".to_string()));
    }

    #[test]
    fn test_db_connector_returns_tool_override() {
        let intent = serde_json::json!({
            "data_sources": ["our production postgres"],
            "uses_external_db": "prod_db",
            "write_targets": [],
            "actions": ["query leads table"],
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_resolved, tools, _) = rt.block_on(
            ConnectorResolver::resolve(&intent, &[], &[])
        );
        assert!(tools.contains(&"external_db:prod_db".to_string()));
    }

    #[test]
    fn test_connector_resolver_uses_candidate_connector_hint() {
        let intent = serde_json::json!({
            "data_sources": ["customer data"],
            "write_targets": [],
            "actions": ["sync records"],
            "candidate_connectors": ["hubspot"],
        });
        let installed = vec!["hubspot".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (resolved, _tools, clarifying) = rt.block_on(
            ConnectorResolver::resolve(&intent, &installed, &[])
        );
        assert!(resolved.contains(&"hubspot".to_string()));
        assert!(clarifying.is_none());
    }

    #[test]
    fn test_connector_resolver_prompts_for_missing_connector_category() {
        let intent = serde_json::json!({
            "data_sources": ["pipeline data"],
            "write_targets": [],
            "actions": ["update CRM records"],
            "needed_connector_categories": ["crm"],
        });
        let installed: Vec<String> = vec!["slack".into()];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_resolved, _tools, clarifying) = rt.block_on(
            ConnectorResolver::resolve(&intent, &installed, &[])
        );
        let question = clarifying.expect("should ask for missing crm connector");
        assert!(question.to_lowercase().contains("crm connector"));
    }

    #[test]
    fn test_parse_trigger_schedule() {
        let trigger = parse_trigger_from_text("every friday at 9am");
        assert_eq!(trigger.trigger_type, TriggerType::Schedule);
        assert_eq!(trigger.cron.as_deref(), Some("0 9 * * 5"));
    }

    #[test]
    fn test_parse_trigger_webhook() {
        let trigger = parse_trigger_from_text("when a new Salesforce lead is created");
        assert_eq!(trigger.trigger_type, TriggerType::Webhook);
        assert_eq!(trigger.source_connector.as_deref(), Some("salesforce"));
    }

    #[test]
    fn test_parse_trigger_manual() {
        let trigger = parse_trigger_from_text("on demand");
        assert_eq!(trigger.trigger_type, TriggerType::Manual);
    }

    #[test]
    fn test_parse_trigger_workforce_event() {
        let trigger = parse_trigger_from_text("after the lead enrichment role completes");
        assert_eq!(trigger.trigger_type, TriggerType::WorkforceEvent);
    }

    #[test]
    fn test_natural_to_cron_friday() { assert_eq!(natural_to_cron("every friday"), "0 9 * * 5"); }

    #[test]
    fn test_natural_to_cron_midnight() { assert_eq!(natural_to_cron("daily at midnight"), "0 0 * * *"); }

    #[test]
    fn test_build_custom_context_empty() {
        let ctx = build_custom_context(&[], &[]);
        assert!(ctx.is_empty());
    }
}
