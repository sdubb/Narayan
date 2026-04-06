use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A single provider credential belonging to a tenant.
/// The secret value is stored encrypted at rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredential {
    pub provider: String,
    /// Encrypted API key — decrypted only at call time.
    pub secret_enc: String,
    /// Human-readable label (e.g. "Production Anthropic Key").
    pub label: String,
    pub model: String,
    pub enabled: bool,
}

/// Per-tenant LLM routing preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantRoutingConfig {
    /// Provider to use for simple tasks.
    pub simple: String,
    /// Provider to use for medium tasks.
    pub medium: String,
    /// Provider to use for complex tasks.
    pub complex: String,
    /// Fallback provider.
    pub fallback: String,
}

impl Default for TenantRoutingConfig {
    fn default() -> Self {
        Self {
            simple: "openrouter".into(),
            medium: "openrouter".into(),
            complex: "anthropic".into(),
            fallback: "openrouter".into(),
        }
    }
}

/// Full per-tenant configuration stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantConfig {
    pub tenant_id: String,
    /// Provider credentials keyed by provider name.
    pub credentials: HashMap<String, ProviderCredential>,
    /// LLM routing preferences.
    pub routing: TenantRoutingConfig,
    /// Custom metadata (webhook URLs, notification settings, etc.).
    pub metadata: serde_json::Value,
}

impl TenantConfig {
    pub fn new(tenant_id: String) -> Self {
        Self {
            tenant_id,
            credentials: HashMap::new(),
            routing: TenantRoutingConfig::default(),
            metadata: serde_json::Value::Object(Default::default()),
        }
    }

    /// Add or replace a provider credential.
    pub fn set_credential(&mut self, cred: ProviderCredential) {
        self.credentials.insert(cred.provider.clone(), cred);
    }

    pub fn get_credential(&self, provider: &str) -> Option<&ProviderCredential> {
        self.credentials.get(provider)
    }

    pub fn enabled_providers(&self) -> Vec<&str> {
        self.credentials.values().filter(|c| c.enabled).map(|c| c.provider.as_str()).collect()
    }
}

// ── AES-256-GCM credential encryption ─────────────────────────────────────
// Format: "v1:<hex(nonce ++ ciphertext ++ tag)>"
// Key is derived from the env-var passphrase via SHA-256.
// The "v1:" prefix allows future key rotation / algorithm upgrades.

const ENCRYPTION_VERSION: &str = "v1";

/// Derive a 256-bit key from an arbitrary-length passphrase using SHA-256.
fn derive_key(passphrase: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(passphrase.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

pub fn encrypt_secret(plaintext: &str, key: &str) -> String {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
    use ring::rand::{SecureRandom, SystemRandom};

    let key_bytes = derive_key(key);
    let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).expect("AES key construction");
    let aead_key = LessSafeKey::new(unbound);

    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes).expect("random nonce generation");
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.as_bytes().to_vec();
    aead_key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out).expect("AES-256-GCM seal");

    // Wire format: nonce (12 bytes) ++ ciphertext-with-tag
    let mut payload = Vec::with_capacity(NONCE_LEN + in_out.len());
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&in_out);

    format!("{}:{}", ENCRYPTION_VERSION, hex::encode(payload))
}

pub fn decrypt_secret(ciphertext: &str, key: &str) -> Result<String> {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};

    // Support versioned format ("v1:hex") and legacy bare-hex (XOR)
    let (version, hex_payload) = match ciphertext.split_once(':') {
        Some((v, h)) => (v, h),
        None => return decrypt_secret_legacy(ciphertext, key),
    };

    if version != ENCRYPTION_VERSION {
        anyhow::bail!("unsupported encryption version '{}' — expected '{}'", version, ENCRYPTION_VERSION);
    }

    let payload = hex::decode(hex_payload).map_err(|e| anyhow::anyhow!("invalid ciphertext hex: {}", e))?;
    if payload.len() < NONCE_LEN + 1 {
        anyhow::bail!("ciphertext too short");
    }

    let (nonce_bytes, sealed) = payload.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes).map_err(|_| anyhow::anyhow!("invalid nonce"))?;

    let key_bytes = derive_key(key);
    let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes).map_err(|_| anyhow::anyhow!("invalid AES key"))?;
    let aead_key = LessSafeKey::new(unbound);

    let mut in_out = sealed.to_vec();
    let plaintext_bytes = aead_key
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| anyhow::anyhow!("decryption failed — wrong key or corrupted data"))?;

    String::from_utf8(plaintext_bytes.to_vec()).map_err(|e| anyhow::anyhow!("decrypt produced invalid UTF-8: {}", e))
}

/// Legacy XOR decryption — auto-migrates old ciphertexts that lack a version prefix.
fn decrypt_secret_legacy(ciphertext: &str, key: &str) -> Result<String> {
    let bytes = hex::decode(ciphertext).map_err(|e| anyhow::anyhow!("invalid legacy ciphertext: {}", e))?;
    let key_bytes = key.as_bytes();
    let dec: Vec<u8> = bytes.iter().enumerate().map(|(i, b)| b ^ key_bytes[i % key_bytes.len()]).collect();
    tracing::warn!("decrypted legacy XOR credential — re-encrypt with PUT /credentials to upgrade to AES-256-GCM");
    String::from_utf8(dec).map_err(|e| anyhow::anyhow!("legacy decrypt failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes_gcm_roundtrip() {
        let key = "my-super-secret-encryption-key";
        let plaintext = "sk-ant-api03-xyzzy-1234567890";
        let encrypted = encrypt_secret(plaintext, key);
        assert!(encrypted.starts_with("v1:"), "should have version prefix");
        let decrypted = decrypt_secret(&encrypted, key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_aes_gcm_unique_nonces() {
        let key = "test-key";
        let a = encrypt_secret("same-plaintext", key);
        let b = encrypt_secret("same-plaintext", key);
        assert_ne!(a, b, "different nonces should produce different ciphertexts");
    }

    #[test]
    fn test_aes_gcm_wrong_key_fails() {
        let encrypted = encrypt_secret("secret", "correct-key");
        let result = decrypt_secret(&encrypted, "wrong-key");
        assert!(result.is_err());
    }

    #[test]
    fn test_aes_gcm_tampered_ciphertext_fails() {
        let encrypted = encrypt_secret("secret", "key");
        // Flip a byte in the hex payload
        let mut chars: Vec<char> = encrypted.chars().collect();
        let last = chars.len() - 1;
        chars[last] = if chars[last] == '0' { '1' } else { '0' };
        let tampered: String = chars.into_iter().collect();
        let result = decrypt_secret(&tampered, "key");
        assert!(result.is_err());
    }

    #[test]
    fn test_legacy_xor_decryption_still_works() {
        // Simulate old XOR-encrypted value (bare hex, no version prefix)
        let key = "testkey";
        let plaintext = "sk-1234";
        let key_bytes = key.as_bytes();
        let enc: Vec<u8> =
            plaintext.as_bytes().iter().enumerate().map(|(i, b)| b ^ key_bytes[i % key_bytes.len()]).collect();
        let legacy_ciphertext = hex::encode(enc);

        // Should NOT start with "v1:"
        assert!(!legacy_ciphertext.contains(':'));

        let decrypted = decrypt_secret(&legacy_ciphertext, key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_unsupported_version_fails() {
        let result = decrypt_secret("v99:deadbeef", "key");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported encryption version"));
    }

    #[test]
    fn test_empty_plaintext_roundtrip() {
        let key = "key";
        let encrypted = encrypt_secret("", key);
        let decrypted = decrypt_secret(&encrypted, key).unwrap();
        assert_eq!(decrypted, "");
    }
}
