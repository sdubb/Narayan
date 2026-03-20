use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

/// Object storage configuration.
#[derive(Debug, Clone)]
pub struct ObjectStorageConfig {
    pub bucket: String,
    /// S3-compatible endpoint: https://s3.amazonaws.com, https://r2.cloudflarestorage.com/account-id, http://minio:9000
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

/// Trait for any S3-compatible object storage backend.
#[async_trait]
pub trait ObjectStorage: Send + Sync {
    async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Vec<u8>>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
    async fn delete(&self, key: &str) -> Result<()>;
    fn public_url(&self, key: &str) -> String;
}

/// S3-compatible implementation using reqwest + manual HMAC-SHA256 signing.
/// Works with AWS S3, Cloudflare R2, MinIO, and any S3-compatible backend.
pub struct S3CompatibleStorage {
    cfg: ObjectStorageConfig,
    client: reqwest::Client,
}

impl S3CompatibleStorage {
    pub fn new(cfg: ObjectStorageConfig) -> Result<Self> {
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;
        Ok(Self { cfg, client })
    }

    fn object_url(&self, key: &str) -> String {
        format!("{}/{}/{}", self.cfg.endpoint.trim_end_matches('/'), self.cfg.bucket, key)
    }

    /// Compute AWS Signature V4 authorization header.
    fn sign_request(&self, method: &str, key: &str, content_type: &str, body: &[u8]) -> HashMap<String, String> {
        use sha2::Digest;
        let now = chrono::Utc::now();
        let datestamp = now.format("%Y%m%d").to_string();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();

        // Body hash
        let body_hash = hex::encode(sha2::Sha256::digest(body));

        // Canonical request
        let host = self.cfg.endpoint.replace("https://", "").replace("http://", "").trim_end_matches('/').to_string();
        let canonical_uri = format!("/{}/{}", self.cfg.bucket, key);
        let canonical_headers =
            format!("host:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n", host, body_hash, timestamp);
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request =
            format!("{}\n{}\n\n{}\n{}\n{}", method, canonical_uri, canonical_headers, signed_headers, body_hash);

        // String to sign
        let credential_scope = format!("{}/{}/s3/aws4_request", datestamp, self.cfg.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            timestamp,
            credential_scope,
            hex::encode(sha2::Sha256::digest(canonical_request.as_bytes()))
        );

        // Signing key
        let sign_key = hmac_sha256(
            &hmac_sha256(
                &hmac_sha256(
                    &hmac_sha256(format!("AWS4{}", self.cfg.secret_key).as_bytes(), datestamp.as_bytes()),
                    self.cfg.region.as_bytes(),
                ),
                b"s3",
            ),
            b"aws4_request",
        );

        let signature = hex::encode(hmac_sha256(&sign_key, string_to_sign.as_bytes()));
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{},SignedHeaders={},Signature={}",
            self.cfg.access_key, credential_scope, signed_headers, signature
        );

        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), authorization);
        headers.insert("x-amz-date".into(), timestamp);
        headers.insert("x-amz-content-sha256".into(), body_hash);
        if !content_type.is_empty() {
            headers.insert("Content-Type".into(), content_type.to_string());
        }
        headers
    }
}

#[async_trait]
impl ObjectStorage for S3CompatibleStorage {
    async fn put(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<()> {
        let headers = self.sign_request("PUT", key, content_type, &data);
        let url = self.object_url(key);
        let mut req = self.client.put(&url).body(data);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("S3 PUT {} failed: HTTP {}", key, resp.status());
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let headers = self.sign_request("GET", key, "", &[]);
        let url = self.object_url(key);
        let mut req = self.client.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!("object not found: {}", key);
        }
        if !resp.status().is_success() {
            anyhow::bail!("S3 GET {} failed: HTTP {}", key, resp.status());
        }
        Ok(resp.bytes().await?.to_vec())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let query = format!("?list-type=2&prefix={}", urlencoding(prefix));
        let url = format!("{}/{}/{}", self.cfg.endpoint.trim_end_matches('/'), self.cfg.bucket, query);
        let headers = self.sign_request("GET", &format!("?list-type=2&prefix={}", urlencoding(prefix)), "", &[]);
        let mut req = self.client.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?.text().await?;
        // Parse keys from XML response
        let keys =
            resp.split("<Key>").skip(1).filter_map(|chunk| chunk.split("</Key>").next()).map(String::from).collect();
        Ok(keys)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let headers = self.sign_request("DELETE", key, "", &[]);
        let url = self.object_url(key);
        let mut req = self.client.delete(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req.send().await?;
        Ok(())
    }

    fn public_url(&self, key: &str) -> String {
        format!("{}/{}/{}", self.cfg.endpoint.trim_end_matches('/'), self.cfg.bucket, key)
    }
}

/// Remote workspace — all I/O proxied through object storage.
pub struct RemoteWorkspace {
    pub prefix: String, // e.g. "workspaces/tenant_id/agent_id"
    pub storage: std::sync::Arc<dyn ObjectStorage>,
}

impl RemoteWorkspace {
    pub fn new(prefix: String, storage: std::sync::Arc<dyn ObjectStorage>) -> Self {
        Self { prefix, storage }
    }

    pub fn storage_key(&self, rel: &str) -> String {
        format!("{}/{}", self.prefix.trim_end_matches('/'), rel.trim_start_matches('/'))
    }

    pub async fn write(&self, rel: &str, data: Vec<u8>) -> Result<()> {
        let mime = mime_guess::from_path(rel).first_or_octet_stream().to_string();
        self.storage.put(&self.storage_key(rel), data, &mime).await
    }

    pub async fn read(&self, rel: &str) -> Result<Vec<u8>> {
        self.storage.get(&self.storage_key(rel)).await
    }

    pub async fn list(&self) -> Result<Vec<String>> {
        self.storage.list(&self.prefix).await
    }

    pub fn public_url(&self, rel: &str) -> String {
        self.storage.public_url(&self.storage_key(rel))
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    let block = 64usize;
    let mut k = if key.len() > block { sha2::Sha256::digest(key).to_vec() } else { key.to_vec() };
    k.resize(block, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let mut inner = sha2::Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let mut outer = sha2::Sha256::new();
    outer.update(&opad);
    outer.update(inner.finalize());
    outer.finalize().to_vec()
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
            c => format!("%{:02X}", c as u32),
        })
        .collect()
}
