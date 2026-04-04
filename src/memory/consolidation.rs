use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    gateway::{GatewayRequest, LlmGateway, TaskComplexity},
    memory::{EmbeddingModel, VectorDocument, VectorStore},
    providers::Message,
    state::{AgentState, AgentStatus, SessionTask, SessionTaskOutput},
    storage::PostgresStore,
};

pub const MEMORY_INDEX_KEY: &str = "memory_index";
pub const MEMORY_TOPIC_PREFIX: &str = "memory_topic/";

const MAX_TOPICS: usize = 8;
const MAX_STEP_OUTPUTS: usize = 20;
const MAX_TASKS: usize = 16;
const MAX_WORKER_MESSAGES: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ConsolidationPromptPayload {
    current_date: String,
    goal: String,
    final_answer: String,
    last_reflection: String,
    key_findings: Vec<String>,
    step_outputs: Vec<serde_json::Value>,
    worker_messages: Vec<serde_json::Value>,
    session_tasks: Vec<serde_json::Value>,
    existing_index: String,
    existing_topics: Vec<ExistingTopic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ExistingTopic {
    key: String,
    title: String,
    hook: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ConsolidationResponse {
    #[serde(default)]
    changed: bool,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    topics: Vec<TopicDraft>,
    #[serde(default)]
    prune: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TopicDraft {
    #[serde(default)]
    key: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    hook: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    facts: Vec<String>,
    #[serde(default)]
    decisions: Vec<String>,
    #[serde(default)]
    risks: Vec<String>,
    #[serde(default)]
    dates: Vec<String>,
    #[serde(default)]
    supersedes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConsolidationResult {
    pub changed: bool,
    pub skipped: bool,
    pub summary: String,
    pub topics_saved: Vec<String>,
    pub pruned_topics: Vec<String>,
    pub topic_keys: Vec<String>,
    pub index_key: String,
    pub signal_fingerprint: String,
}

impl ConsolidationResult {
    fn skipped(summary: impl Into<String>, signal_fingerprint: String, topic_keys: Vec<String>) -> Self {
        Self {
            changed: false,
            skipped: true,
            summary: summary.into(),
            topics_saved: Vec::new(),
            pruned_topics: Vec::new(),
            topic_keys,
            index_key: MEMORY_INDEX_KEY.into(),
            signal_fingerprint,
        }
    }
}

pub struct MemoryConsolidator {
    gateway: Arc<dyn LlmGateway>,
    vector_store: Arc<dyn VectorStore>,
    embedder: Arc<dyn EmbeddingModel>,
    store: Option<Arc<PostgresStore>>,
}

impl MemoryConsolidator {
    pub fn new(
        gateway: Arc<dyn LlmGateway>,
        vector_store: Arc<dyn VectorStore>,
        embedder: Arc<dyn EmbeddingModel>,
    ) -> Arc<Self> {
        Arc::new(Self { gateway, vector_store, embedder, store: None })
    }

    pub fn with_store(mut self: Arc<Self>, store: Arc<PostgresStore>) -> Arc<Self> {
        Arc::get_mut(&mut self).expect("memory consolidator should be uniquely owned during setup").store = Some(store);
        self
    }

    pub async fn consolidate_agent(&self, state: &AgentState, force: bool) -> Result<ConsolidationResult> {
        let payload = self.build_payload(state).await?;
        let fingerprint = payload_fingerprint(&payload);
        let existing_topic_keys = payload.existing_topics.iter().map(|topic| topic.key.clone()).collect::<Vec<_>>();

        if !matches!(state.status, AgentStatus::Completed) {
            return Ok(ConsolidationResult::skipped(
                "memory consolidation only runs after successful completion",
                fingerprint,
                existing_topic_keys,
            ));
        }

        if !force && self.already_consolidated(state, &fingerprint) {
            return Ok(ConsolidationResult::skipped(
                "memory consolidation skipped because no new successful signal was found",
                fingerprint,
                existing_topic_keys,
            ));
        }

        if payload.final_answer.trim().is_empty()
            && payload.last_reflection.trim().is_empty()
            && payload.key_findings.is_empty()
            && payload.step_outputs.is_empty()
            && payload.worker_messages.is_empty()
            && payload.session_tasks.is_empty()
        {
            return Ok(ConsolidationResult::skipped(
                "memory consolidation skipped because there was no durable successful signal to store",
                fingerprint,
                existing_topic_keys,
            ));
        }

        let request = GatewayRequest::new(
            state.id.clone(),
            state.tenant_id.clone(),
            TaskComplexity::Medium,
            vec![
                Message::system(system_prompt()),
                Message::user(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())),
            ],
        )
        .no_cache();

        let raw = self.gateway.chat(request).await?.content.unwrap_or_default();
        let response = parse_response(&raw)?;
        let topics = sanitize_topics(response.topics);
        let prune_keys = collect_prune_keys(&response.prune, &topics);

        let mut final_topics =
            payload.existing_topics.into_iter().map(|topic| (topic.key.clone(), topic)).collect::<BTreeMap<_, _>>();

        for prune_key in &prune_keys {
            final_topics.remove(prune_key);
            crate::tools::memory_store_internal::remove(&scoped_memory_key(&state.id, &topic_memory_key(prune_key)));
            let _ = self.vector_store.delete(&state.tenant_id, &vector_doc_id(&state.id, prune_key)).await;
        }

        let mut topics_saved = Vec::new();
        for topic in topics {
            let content = render_topic(&topic);
            crate::tools::memory_store_internal::insert(
                scoped_memory_key(&state.id, &topic_memory_key(&topic.key)),
                content.clone(),
            );
            self.persist_vector_topic(state, &topic, &content).await?;
            final_topics.insert(
                topic.key.clone(),
                ExistingTopic { key: topic.key.clone(), title: topic.title.clone(), hook: topic.hook.clone(), content },
            );
            topics_saved.push(topic.key);
        }

        let index_content = render_index(&final_topics);
        crate::tools::memory_store_internal::insert(scoped_memory_key(&state.id, MEMORY_INDEX_KEY), index_content);

        let topic_keys = final_topics.keys().cloned().collect::<Vec<_>>();
        Ok(ConsolidationResult {
            changed: response.changed || !topics_saved.is_empty() || !prune_keys.is_empty(),
            skipped: false,
            summary: consolidation_summary(&response.summary, &topics_saved, &prune_keys),
            topics_saved,
            pruned_topics: prune_keys.into_iter().collect(),
            topic_keys,
            index_key: MEMORY_INDEX_KEY.into(),
            signal_fingerprint: fingerprint,
        })
    }

    async fn build_payload(&self, state: &AgentState) -> Result<ConsolidationPromptPayload> {
        let existing_index = crate::tools::memory_store_internal::get(&scoped_memory_key(&state.id, MEMORY_INDEX_KEY))
            .unwrap_or_default();
        let existing_topics = existing_topics_for_agent(&state.id);
        let step_outputs = state
            .metadata
            .get("step_outputs")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .rev()
            .take(MAX_STEP_OUTPUTS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(trim_large_json)
            .collect();
        let worker_messages = state
            .metadata
            .get("worker_messages")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .rev()
            .take(MAX_WORKER_MESSAGES)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(trim_large_json)
            .collect();
        let session_tasks = if let Some(store) = &self.store {
            store
                .list_session_tasks_for_agent(&state.tenant_id, &state.id)
                .await
                .unwrap_or_default()
                .into_iter()
                .rev()
                .take(MAX_TASKS)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(task_to_json)
                .collect()
        } else {
            Vec::new()
        };

        Ok(ConsolidationPromptPayload {
            current_date: Utc::now().date_naive().to_string(),
            goal: state.goal.clone(),
            final_answer: state.final_answer().unwrap_or_default().to_string(),
            last_reflection: state
                .metadata
                .get("last_reflection")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            key_findings: state
                .metadata
                .get("key_findings")
                .and_then(|value| value.as_array())
                .map(|values| values.iter().filter_map(|value| value.as_str().map(str::to_string)).collect::<Vec<_>>())
                .unwrap_or_default(),
            step_outputs,
            worker_messages,
            session_tasks,
            existing_index,
            existing_topics,
        })
    }

    fn already_consolidated(&self, state: &AgentState, fingerprint: &str) -> bool {
        state
            .metadata
            .get("memory_consolidation")
            .and_then(|value| value.get("signal_fingerprint"))
            .and_then(|value| value.as_str())
            == Some(fingerprint)
    }

    async fn persist_vector_topic(&self, state: &AgentState, topic: &TopicDraft, content: &str) -> Result<()> {
        let embedding = self.embedder.embed(content).await?;
        let document = VectorDocument {
            id: vector_doc_id(&state.id, &topic.key),
            tenant_id: state.tenant_id.clone(),
            agent_id: state.id.clone(),
            content: content.to_string(),
            embedding,
            metadata: serde_json::json!({
                "source": "memory_consolidation",
                "memory_topic": topic.key,
                "memory_title": topic.title,
                "hook": topic.hook,
                "goal": state.goal,
                "successful_outcome": true,
                "consolidated_at": Utc::now().to_rfc3339(),
            }),
            created_at: Utc::now(),
        };
        self.vector_store.upsert(document).await
    }
}

pub fn apply_consolidation_metadata(state: &mut AgentState, result: &ConsolidationResult) {
    state.memory_ref = Some(scoped_memory_key(&state.id, &result.index_key));
    state.metadata["memory_consolidation"] = serde_json::json!({
        "last_run_at": Utc::now().to_rfc3339(),
        "changed": result.changed,
        "skipped": result.skipped,
        "summary": result.summary,
        "topics_saved": result.topics_saved,
        "pruned_topics": result.pruned_topics,
        "topic_keys": result.topic_keys,
        "index_key": result.index_key,
        "signal_fingerprint": result.signal_fingerprint,
    });
}

fn system_prompt() -> &'static str {
    "You are Narayan's memory consolidator.\n\
Follow this loop exactly:\n\
1. Orient to the current topic memories and the current memory index.\n\
2. Gather only recent successful signal that materially improved the outcome.\n\
3. Consolidate by merging new signal into durable topic memories.\n\
4. Prune stale, contradicted, or superseded memories and keep the index concise.\n\n\
Rules:\n\
- Persist only information that materially helped a successful outcome.\n\
- Prefer updating existing topics over creating duplicates.\n\
- Convert relative dates into absolute YYYY-MM-DD dates using current_date.\n\
- Delete contradicted facts at the source instead of storing both versions.\n\
- Each topic should stay practical: summary, concrete facts, decisions, risks, and dated notes when useful.\n\
- Keep the index as one-line hooks only; do not dump content into it.\n\
- If nothing durable changed, return changed=false.\n\n\
Return valid JSON only with this schema:\n\
{\n\
  \"changed\": true,\n\
  \"summary\": \"brief summary\",\n\
  \"topics\": [\n\
    {\n\
      \"key\": \"stable_topic_key\",\n\
      \"title\": \"Human Title\",\n\
      \"hook\": \"One-line reason this topic matters\",\n\
      \"summary\": \"Short durable summary\",\n\
      \"facts\": [\"fact\"],\n\
      \"decisions\": [\"decision\"],\n\
      \"risks\": [\"risk\"],\n\
      \"dates\": [\"2026-04-01: dated note\"],\n\
      \"supersedes\": [\"old_topic_key\"]\n\
    }\n\
  ],\n\
  \"prune\": [\"topic_key_to_remove\"]\n\
}"
}

fn parse_response(raw: &str) -> Result<ConsolidationResponse> {
    let cleaned = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    serde_json::from_str(cleaned).with_context(|| {
        format!("memory consolidation returned invalid JSON: {}", raw.chars().take(300).collect::<String>())
    })
}

fn existing_topics_for_agent(agent_id: &str) -> Vec<ExistingTopic> {
    let prefix = format!("{agent_id}:{MEMORY_TOPIC_PREFIX}");
    let mut topics = crate::tools::memory_store_internal::entries_with_prefix(&prefix)
        .into_iter()
        .filter_map(|(key, content)| {
            let topic_key = key.strip_prefix(&prefix)?.to_string();
            let (title, hook) = parse_title_hook(&content);
            Some(ExistingTopic { key: topic_key, title, hook, content })
        })
        .collect::<Vec<_>>();
    topics.sort_by(|left, right| left.key.cmp(&right.key));
    topics
}

fn parse_title_hook(content: &str) -> (String, String) {
    let mut title = String::new();
    let mut hook = String::new();
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Title:") {
            title = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("Hook:") {
            hook = value.trim().to_string();
        }
        if !title.is_empty() && !hook.is_empty() {
            break;
        }
    }
    (title, hook)
}

fn task_to_json(task: SessionTask) -> serde_json::Value {
    serde_json::json!({
        "id": task.id,
        "subject": task.subject,
        "description": task.description,
        "status": task.status,
        "owner": task.owner,
        "blocked_by": task.blocked_by,
        "blocks": task.blocks,
        "output": task.output.as_ref().map(trim_task_output),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

fn trim_task_output(output: &SessionTaskOutput) -> serde_json::Value {
    serde_json::json!({
        "status": output.status,
        "artifacts": output.artifacts.iter().take(8).cloned().collect::<Vec<_>>(),
        "findings": output.findings.iter().take(8).cloned().collect::<Vec<_>>(),
        "confidence": output.confidence,
        "note": output.note,
    })
}

fn trim_large_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => {
            let clipped = if text.chars().count() > 600 {
                format!("{}...(truncated)", text.chars().take(600).collect::<String>())
            } else {
                text
            };
            serde_json::Value::String(clipped)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().take(20).map(trim_large_json).collect())
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (index, (key, value)) in map.into_iter().enumerate() {
                if index >= 20 {
                    break;
                }
                out.insert(key, trim_large_json(value));
            }
            serde_json::Value::Object(out)
        }
        other => other,
    }
}

fn sanitize_topics(topics: Vec<TopicDraft>) -> Vec<TopicDraft> {
    let mut seen = BTreeSet::new();
    let mut sanitized = Vec::new();
    for mut topic in topics.into_iter().take(MAX_TOPICS) {
        topic.key = topic_slug(&topic.key, &topic.title);
        if topic.key.is_empty() || !seen.insert(topic.key.clone()) {
            continue;
        }
        if topic.title.trim().is_empty() {
            topic.title = humanize_key(&topic.key);
        }
        if topic.hook.trim().is_empty() {
            topic.hook = first_non_empty(&[&topic.summary, &topic.title]).unwrap_or_default();
        }
        topic.summary = topic.summary.trim().to_string();
        topic.facts = clean_list(topic.facts, 8);
        topic.decisions = clean_list(topic.decisions, 6);
        topic.risks = clean_list(topic.risks, 6);
        topic.dates = clean_list(topic.dates, 6);
        topic.supersedes = clean_list(topic.supersedes, 8)
            .into_iter()
            .map(|value| topic_slug(&value, &value))
            .filter(|value| !value.is_empty() && value != &topic.key)
            .collect();
        if topic.summary.is_empty() && topic.facts.is_empty() && topic.decisions.is_empty() && topic.risks.is_empty() {
            continue;
        }
        sanitized.push(topic);
    }
    sanitized
}

fn collect_prune_keys(prune: &[String], topics: &[TopicDraft]) -> BTreeSet<String> {
    let mut out =
        prune.iter().map(|value| topic_slug(value, value)).filter(|value| !value.is_empty()).collect::<BTreeSet<_>>();
    for topic in topics {
        for superseded in &topic.supersedes {
            out.insert(superseded.clone());
        }
    }
    for topic in topics {
        out.remove(&topic.key);
    }
    out
}

fn render_topic(topic: &TopicDraft) -> String {
    let mut lines = vec![
        format!("Title: {}", topic.title.trim()),
        format!("Hook: {}", topic.hook.trim()),
        format!("Updated: {}", Utc::now().date_naive()),
        String::new(),
        "Summary:".into(),
        topic.summary.trim().to_string(),
    ];
    append_section(&mut lines, "Facts", &topic.facts);
    append_section(&mut lines, "Decisions", &topic.decisions);
    append_section(&mut lines, "Risks", &topic.risks);
    append_section(&mut lines, "Dates", &topic.dates);
    lines.join("\n")
}

fn render_index(topics: &BTreeMap<String, ExistingTopic>) -> String {
    topics
        .values()
        .map(|topic| {
            let title = if topic.title.trim().is_empty() { humanize_key(&topic.key) } else { topic.title.clone() };
            let hook =
                if topic.hook.trim().is_empty() { "durable project memory".to_string() } else { topic.hook.clone() };
            format!("- [{}]({}) - {}", title.trim(), topic_memory_key(&topic.key), truncate(&hook, 140))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_section(lines: &mut Vec<String>, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("{heading}:"));
    for item in items {
        lines.push(format!("- {}", item.trim()));
    }
}

fn payload_fingerprint(payload: &ConsolidationPromptPayload) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

fn consolidation_summary(summary: &str, topics_saved: &[String], prune_keys: &BTreeSet<String>) -> String {
    if !summary.trim().is_empty() {
        summary.trim().to_string()
    } else if topics_saved.is_empty() && prune_keys.is_empty() {
        "memory consolidation found no durable updates".into()
    } else {
        format!("consolidated {} topic(s) and pruned {} topic(s)", topics_saved.len(), prune_keys.len())
    }
}

fn clean_list(values: Vec<String>, max_items: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .take(max_items)
        .collect()
}

fn topic_slug(key: &str, fallback: &str) -> String {
    let raw = if key.trim().is_empty() { fallback } else { key };
    raw.to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("_")
}

fn humanize_key(key: &str) -> String {
    key.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn first_non_empty(values: &[&str]) -> Option<String> {
    values.iter().map(|value| value.trim()).find(|value| !value.is_empty()).map(str::to_string)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn scoped_memory_key(agent_id: &str, key: &str) -> String {
    format!("{agent_id}:{key}")
}

fn topic_memory_key(topic_key: &str) -> String {
    format!("{MEMORY_TOPIC_PREFIX}{topic_key}")
}

fn vector_doc_id(agent_id: &str, topic_key: &str) -> String {
    format!("memory-topic:{agent_id}:{topic_key}")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::{
        memory::{embeddings::StubEmbeddingModel, vector::InMemoryVectorStore},
        providers::ChatResponse,
    };

    struct MockGateway {
        responses: Mutex<Vec<ChatResponse>>,
    }

    #[async_trait]
    impl LlmGateway for MockGateway {
        async fn chat(&self, _request: GatewayRequest) -> Result<ChatResponse> {
            Ok(self.responses.lock().expect("responses lock").remove(0))
        }
    }

    fn gateway_with(content: &str) -> Arc<dyn LlmGateway> {
        Arc::new(MockGateway {
            responses: Mutex::new(vec![ChatResponse {
                content: Some(content.to_string()),
                tool_calls: vec![],
                input_tokens: 0,
                output_tokens: 0,
            }]),
        })
    }

    fn completed_state() -> AgentState {
        let mut state = AgentState::new("agent-1".into(), "tenant-1".into(), "ship frontend".into(), "/tmp/ws".into());
        state.mark_completed();
        state.set_final_answer("Implemented the frontend state flow and verified it.");
        state.metadata["last_reflection"] = serde_json::json!("A single store now owns loading and empty states.");
        state.metadata["key_findings"] = serde_json::json!(["The page store now owns loading state"]);
        state.metadata["step_outputs"] = serde_json::json!([{"summary": "updated loading state flow", "processed": 1}]);
        state
    }

    #[tokio::test]
    async fn skips_non_completed_agents() {
        let mut state = completed_state();
        state.mark_failed();
        let consolidator = MemoryConsolidator::new(
            gateway_with(r#"{"changed":false,"summary":"no-op","topics":[],"prune":[]}"#),
            Arc::new(InMemoryVectorStore::default()),
            Arc::new(StubEmbeddingModel::new(4)),
        );
        let result = consolidator.consolidate_agent(&state, false).await.expect("consolidation should not fail");
        assert!(result.skipped);
    }

    #[tokio::test]
    async fn stores_topic_and_index() {
        let state = completed_state();
        let consolidator = MemoryConsolidator::new(
            gateway_with(
                r#"{
                    "changed": true,
                    "summary": "Saved frontend loading state memory.",
                    "topics": [{
                        "key": "frontend_loading_state",
                        "title": "Frontend Loading State",
                        "hook": "The page store owns loading and empty state transitions.",
                        "summary": "The frontend now drives loading state through a single page store.",
                        "facts": ["The page store owns loading state."],
                        "decisions": ["Use one state owner for loading and empty states."],
                        "risks": ["Keep server and client transitions aligned."],
                        "dates": ["2026-04-01: consolidated after successful UI work."],
                        "supersedes": []
                    }],
                    "prune": []
                }"#,
            ),
            Arc::new(InMemoryVectorStore::default()),
            Arc::new(StubEmbeddingModel::new(4)),
        );
        let result = consolidator.consolidate_agent(&state, false).await.expect("consolidation should succeed");
        assert!(result.changed);
        assert_eq!(result.topics_saved, vec!["frontend_loading_state".to_string()]);
        let topic = crate::tools::memory_store_internal::get("agent-1:memory_topic/frontend_loading_state")
            .expect("topic should be stored");
        assert!(topic.contains("page store"));
        let index = crate::tools::memory_store_internal::get("agent-1:memory_index").expect("index should be stored");
        assert!(index.contains("Frontend Loading State"));
    }

    #[tokio::test]
    async fn prunes_superseded_topics() {
        crate::tools::memory_store_internal::insert(
            "agent-1:memory_topic/old_ui_state".into(),
            "Title: Old UI State\nHook: old hook".into(),
        );
        let state = completed_state();
        let consolidator = MemoryConsolidator::new(
            gateway_with(
                r#"{
                    "changed": true,
                    "summary": "Replaced outdated UI memory.",
                    "topics": [{
                        "key": "frontend_loading_state",
                        "title": "Frontend Loading State",
                        "hook": "Current source of truth for loading state.",
                        "summary": "The new store owns loading state.",
                        "facts": ["Loading now lives in the page store."],
                        "decisions": [],
                        "risks": [],
                        "dates": [],
                        "supersedes": ["old_ui_state"]
                    }],
                    "prune": []
                }"#,
            ),
            Arc::new(InMemoryVectorStore::default()),
            Arc::new(StubEmbeddingModel::new(4)),
        );
        let result = consolidator.consolidate_agent(&state, false).await.expect("consolidation should succeed");
        assert!(result.pruned_topics.contains(&"old_ui_state".to_string()));
        assert!(crate::tools::memory_store_internal::get("agent-1:memory_topic/old_ui_state").is_none());
    }
}
