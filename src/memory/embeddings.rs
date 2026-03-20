use anyhow::Result;
use async_trait::async_trait;

/// Embedding model abstraction.
#[async_trait]
pub trait EmbeddingModel: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }
    fn dimension(&self) -> usize;
    fn model_name(&self) -> &str;
}

// ── OpenAI Embeddings ─────────────────────────────────────────────────────────
// text-embedding-3-small: 1536 dims, $0.02 / 1M tokens
// text-embedding-3-large: 3072 dims, $0.13 / 1M tokens

pub struct OpenAiEmbeddingModel {
    api_key: String,
    model: String,
    dimensions: usize,
}

impl OpenAiEmbeddingModel {
    /// text-embedding-3-small (1536 dims) — best cost/quality tradeoff.
    pub fn small(api_key: String) -> Self {
        Self { api_key, model: "text-embedding-3-small".into(), dimensions: 1536 }
    }
    /// text-embedding-3-large (3072 dims) — highest quality.
    pub fn large(api_key: String) -> Self {
        Self { api_key, model: "text-embedding-3-large".into(), dimensions: 3072 }
    }
    /// Custom model + dimension.
    pub fn custom(api_key: String, model: String, dimensions: usize) -> Self {
        Self { api_key, model, dimensions }
    }
}

#[async_trait]
impl EmbeddingModel for OpenAiEmbeddingModel {
    fn dimension(&self) -> usize {
        self.dimensions
    }
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let res = self.embed_batch(&[text]).await?;
        res.into_iter().next().ok_or_else(|| anyhow::anyhow!("empty embedding response"))
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "model":      self.model,
            "input":      texts,
            "dimensions": self.dimensions,
        });
        let resp: serde_json::Value = client
            .post("https://api.openai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = resp.get("error") {
            anyhow::bail!("OpenAI embeddings error: {}", err);
        }

        resp["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing 'data' in response"))?
            .iter()
            .map(|item| {
                item["embedding"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("missing embedding array"))?
                    .iter()
                    .map(|v| v.as_f64().map(|f| f as f32).ok_or_else(|| anyhow::anyhow!("non-numeric embedding value")))
                    .collect::<Result<Vec<f32>>>()
            })
            .collect()
    }
}

// ── Anthropic Embeddings (Voyage) ──────────────────────────────────────────────
// Anthropic uses Voyage AI for embeddings. voyage-3: 1024 dims
// Much cheaper than OpenAI for high-volume use.

pub struct VoyageEmbeddingModel {
    api_key: String,
    model: String,
    dimensions: usize,
}

impl VoyageEmbeddingModel {
    /// voyage-3 (1024 dims) — Anthropic's recommended embedding model.
    pub fn voyage3(api_key: String) -> Self {
        Self { api_key, model: "voyage-3".into(), dimensions: 1024 }
    }
    /// voyage-3-lite (512 dims) — fastest, cheapest.
    pub fn voyage3_lite(api_key: String) -> Self {
        Self { api_key, model: "voyage-3-lite".into(), dimensions: 512 }
    }
    /// voyage-code-3 — optimized for code.
    pub fn voyage_code3(api_key: String) -> Self {
        Self { api_key, model: "voyage-code-3".into(), dimensions: 1024 }
    }
}

#[async_trait]
impl EmbeddingModel for VoyageEmbeddingModel {
    fn dimension(&self) -> usize {
        self.dimensions
    }
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let res = self.embed_batch(&[text]).await?;
        res.into_iter().next().ok_or_else(|| anyhow::anyhow!("empty response"))
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "model": self.model,
            "input": texts,
        });
        let resp: serde_json::Value = client
            .post("https://api.voyageai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = resp.get("error") {
            anyhow::bail!("Voyage embeddings error: {}", err);
        }

        resp["data"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing 'data'"))?
            .iter()
            .map(|item| {
                item["embedding"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("missing embedding"))?
                    .iter()
                    .map(|v| v.as_f64().map(|f| f as f32).ok_or_else(|| anyhow::anyhow!("non-numeric")))
                    .collect::<Result<Vec<f32>>>()
            })
            .collect()
    }
}

// ── Ollama Local Embeddings ───────────────────────────────────────────────────
// Free, local, no API cost. nomic-embed-text: 768 dims
// Run: ollama pull nomic-embed-text

pub struct OllamaEmbeddingModel {
    base_url: String,
    model: String,
    dimensions: usize,
}

impl OllamaEmbeddingModel {
    pub fn nomic(base_url: Option<String>) -> Self {
        Self {
            base_url: base_url.unwrap_or_else(|| "http://localhost:11434".into()),
            model: "nomic-embed-text".into(),
            dimensions: 768,
        }
    }
    pub fn custom(base_url: String, model: String, dimensions: usize) -> Self {
        Self { base_url, model, dimensions }
    }
}

#[async_trait]
impl EmbeddingModel for OllamaEmbeddingModel {
    fn dimension(&self) -> usize {
        self.dimensions
    }
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({ "model": self.model, "prompt": text });
        let resp: serde_json::Value =
            client.post(format!("{}/api/embeddings", self.base_url)).json(&payload).send().await?.json().await?;

        resp["embedding"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Ollama: missing 'embedding' field"))?
            .iter()
            .map(|v| v.as_f64().map(|f| f as f32).ok_or_else(|| anyhow::anyhow!("non-numeric")))
            .collect()
    }
}

// ── Stub (dev / testing) ──────────────────────────────────────────────────────

pub struct StubEmbeddingModel {
    pub dim: usize,
}
impl StubEmbeddingModel {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}
impl Default for StubEmbeddingModel {
    fn default() -> Self {
        Self::new(1536)
    }
}

#[async_trait]
impl EmbeddingModel for StubEmbeddingModel {
    fn dimension(&self) -> usize {
        self.dim
    }
    fn model_name(&self) -> &str {
        "stub"
    }
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0_f32; self.dim])
    }
}

// ── Factory: build from tenant provider config ────────────────────────────────

pub fn build_embedding_model(provider: &str, api_key: &str, model_hint: Option<&str>) -> Box<dyn EmbeddingModel> {
    match provider {
        "openai" => match model_hint {
            Some(m) if m.contains("large") => Box::new(OpenAiEmbeddingModel::large(api_key.into())),
            _ => Box::new(OpenAiEmbeddingModel::small(api_key.into())),
        },
        "anthropic" | "voyage" => match model_hint {
            Some(m) if m.contains("lite") => Box::new(VoyageEmbeddingModel::voyage3_lite(api_key.into())),
            Some(m) if m.contains("code") => Box::new(VoyageEmbeddingModel::voyage_code3(api_key.into())),
            _ => Box::new(VoyageEmbeddingModel::voyage3(api_key.into())),
        },
        "ollama" => Box::new(OllamaEmbeddingModel::nomic(Some(api_key.into()))),
        _ => {
            tracing::warn!(provider, "unknown embedding provider — using stub");
            Box::new(StubEmbeddingModel::default())
        }
    }
}
