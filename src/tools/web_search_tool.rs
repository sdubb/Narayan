use async_trait::async_trait;

use crate::tools::{ParameterSchema, Tool, ToolResult};

/// Web search via SerpAPI (set SERPAPI_KEY env var) or falls back to DuckDuckGo Instant.
pub struct WebSearchTool {
    api_key: Option<String>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self { api_key: std::env::var("SERPAPI_KEY").ok() }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search_tool"
    }
    fn description(&self) -> &str {
        "Search the web for information. Returns titles, URLs, and snippets for the top results. \
         Set SERPAPI_KEY env var for real results; otherwise uses DuckDuckGo Instant Answer API."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("query", "string", "Search query."),
            ParameterSchema::optional("count", "integer", "Number of results (default: 10, max: 20)."),
            ParameterSchema::optional("region", "string", "Region code, e.g. 'us' (default: 'us')."),
        ]
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = match args["query"].as_str() {
            Some(q) => q,
            None => return Ok(ToolResult::err("'query' is required")),
        };
        let count = args["count"].as_u64().unwrap_or(10).min(20) as usize;

        if let Some(ref key) = self.api_key {
            return search_serpapi(query, count, key).await;
        }
        search_duckduckgo(query).await
    }
}

async fn search_serpapi(query: &str, count: usize, key: &str) -> anyhow::Result<ToolResult> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build()?;
    let url = format!("https://serpapi.com/search.json?q={}&num={}&api_key={}", urlencoding(query), count, key);
    let resp = client.get(&url).send().await?.json::<serde_json::Value>().await?;
    let results: Vec<serde_json::Value> = resp["organic_results"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .take(count)
        .map(|r| {
            serde_json::json!({
                "title":   r["title"].as_str().unwrap_or(""),
                "url":     r["link"].as_str().unwrap_or(""),
                "snippet": r["snippet"].as_str().unwrap_or(""),
            })
        })
        .collect();
    Ok(ToolResult::ok(serde_json::json!({"query": query, "count": results.len(), "results": results})))
}

async fn search_duckduckgo(query: &str) -> anyhow::Result<ToolResult> {
    let client =
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).user_agent("Narayan/1.0").build()?;
    let url = format!("https://api.duckduckgo.com/?q={}&format=json&no_redirect=1&no_html=1", urlencoding(query));
    let resp = match client.get(&url).send().await {
        Ok(r) => r.json::<serde_json::Value>().await.unwrap_or_default(),
        Err(e) => return Ok(ToolResult::err(format!("search failed: {e}"))),
    };
    let mut results = Vec::new();
    if let Some(abstract_text) = resp["AbstractText"].as_str() {
        if !abstract_text.is_empty() {
            results.push(serde_json::json!({
                "title":   resp["Heading"].as_str().unwrap_or(""),
                "url":     resp["AbstractURL"].as_str().unwrap_or(""),
                "snippet": abstract_text,
            }));
        }
    }
    for topic in resp["RelatedTopics"].as_array().unwrap_or(&vec![]).iter().take(9) {
        if let Some(text) = topic["Text"].as_str() {
            results.push(serde_json::json!({
                "title":   text.split(" - ").next().unwrap_or(text),
                "url":     topic["FirstURL"].as_str().unwrap_or(""),
                "snippet": text,
            }));
        }
    }
    Ok(ToolResult::ok(
        serde_json::json!({"query": query, "count": results.len(), "results": results, "source": "duckduckgo"}),
    ))
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}
