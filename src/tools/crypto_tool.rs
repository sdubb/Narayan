//! crypto_tool — Hashing, signing, encrypting, and generating secrets using `ring`.

use async_trait::async_trait;
use base64::Engine;
use ring::{aead, digest, hmac, pbkdf2, rand};

use crate::tools::{ParameterSchema, Tool, ToolResult};

pub struct CryptoTool;

#[async_trait]
impl Tool for CryptoTool {
    fn name(&self) -> &str {
        "crypto_tool"
    }
    fn description(&self) -> &str {
        "Cryptographic operations: hash files/text (SHA-256/SHA-512/MD5), \
         HMAC-SHA256 signing, AES-256-GCM encryption/decryption, \
         and secure random secret generation. All via ring (audited library)."
    }
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("action", "string", "Action: hash | hmac | encrypt | decrypt | random | pbkdf2"),
            ParameterSchema::optional("input", "string", "Input text or base64 data."),
            ParameterSchema::optional("file", "string", "Input file path (for hash)."),
            ParameterSchema::optional("algorithm", "string", "Hash algorithm: sha256 (default) | sha512 | sha1"),
            ParameterSchema::optional("key", "string", "Secret key (base64) for hmac/encrypt/decrypt/pbkdf2."),
            ParameterSchema::optional("length", "integer", "Random bytes length (default: 32)."),
            ParameterSchema::optional("format", "string", "Output format: hex (default) | base64 | raw"),
            ParameterSchema::optional("salt", "string", "Salt (base64) for pbkdf2. Auto-generated if omitted."),
            ParameterSchema::optional("iterations", "integer", "PBKDF2 iterations (default: 100000)."),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = match args["action"].as_str() {
            Some(a) => a,
            None => return Ok(ToolResult::err("'action' required")),
        };
        let format = args["format"].as_str().unwrap_or("hex");

        match action {
            "hash" => {
                let alg = args["algorithm"].as_str().unwrap_or("sha256");
                let data = if let Some(f) = args["file"].as_str() {
                    tokio::fs::read(f).await.map_err(|e| anyhow::anyhow!("read: {}", e))?
                } else {
                    args["input"].as_str().unwrap_or("").as_bytes().to_vec()
                };

                let hash = match alg {
                    "sha512" => digest::digest(&digest::SHA512, &data).as_ref().to_vec(),
                    "sha1" => digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, &data).as_ref().to_vec(),
                    _ => digest::digest(&digest::SHA256, &data).as_ref().to_vec(),
                };

                let out = encode_output(&hash, format);
                Ok(ToolResult::ok(
                    serde_json::json!({"algorithm": alg, "hash": out, "format": format, "bytes": data.len()}),
                ))
            }

            "hmac" => {
                let key_b64 = match args["key"].as_str() {
                    Some(k) => k,
                    None => return Ok(ToolResult::err("'key' required")),
                };
                let input = args["input"].as_str().unwrap_or("");
                let key_bytes = base64::engine::general_purpose::STANDARD
                    .decode(key_b64)
                    .map_err(|e| anyhow::anyhow!("key base64: {}", e))?;
                let key = hmac::Key::new(hmac::HMAC_SHA256, &key_bytes);
                let sig = hmac::sign(&key, input.as_bytes());
                let out = encode_output(sig.as_ref(), format);
                Ok(ToolResult::ok(serde_json::json!({"signature": out, "format": format})))
            }

            "encrypt" => {
                let key_b64 = match args["key"].as_str() {
                    Some(k) => k,
                    None => return Ok(ToolResult::err("'key' required")),
                };
                let plaintext = args["input"].as_str().unwrap_or("");
                let key_bytes = base64::engine::general_purpose::STANDARD
                    .decode(key_b64)
                    .map_err(|e| anyhow::anyhow!("key base64: {}", e))?;

                let rng = rand::SystemRandom::new();
                let mut nonce_bytes = [0u8; 12];
                rand::SecureRandom::fill(&rng, &mut nonce_bytes).map_err(|_| anyhow::anyhow!("rng failed"))?;

                let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
                    .map_err(|_| anyhow::anyhow!("invalid key length — must be 32 bytes"))?;
                let key = aead::LessSafeKey::new(unbound);
                let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);

                let mut data = plaintext.as_bytes().to_vec();
                key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut data)
                    .map_err(|_| anyhow::anyhow!("encryption failed"))?;

                // Output: nonce (12 bytes) + ciphertext+tag
                let mut combined = nonce_bytes.to_vec();
                combined.extend_from_slice(&data);
                let b64 = base64::engine::general_purpose::STANDARD.encode(&combined);
                Ok(ToolResult::ok(serde_json::json!({"ciphertext_b64": b64, "algorithm": "AES-256-GCM"})))
            }

            "decrypt" => {
                let key_b64 = match args["key"].as_str() {
                    Some(k) => k,
                    None => return Ok(ToolResult::err("'key' required")),
                };
                let cipher_b64 = match args["input"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::err("'input' (ciphertext_b64) required")),
                };
                let key_bytes = base64::engine::general_purpose::STANDARD
                    .decode(key_b64)
                    .map_err(|e| anyhow::anyhow!("key: {}", e))?;
                let mut combined = base64::engine::general_purpose::STANDARD
                    .decode(cipher_b64)
                    .map_err(|e| anyhow::anyhow!("input: {}", e))?;
                if combined.len() < 12 {
                    return Ok(ToolResult::err("ciphertext too short"));
                }
                let nonce_bytes: [u8; 12] = combined[..12].try_into().unwrap();
                let ciphertext = &mut combined[12..];
                let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
                    .map_err(|_| anyhow::anyhow!("invalid key"))?;
                let key = aead::LessSafeKey::new(unbound);
                let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
                let plain = key
                    .open_in_place(nonce, aead::Aad::empty(), ciphertext)
                    .map_err(|_| anyhow::anyhow!("decryption failed — wrong key or corrupted data"))?;
                let text = String::from_utf8_lossy(plain).into_owned();
                Ok(ToolResult::ok(serde_json::json!({"plaintext": text})))
            }

            "random" => {
                let n = args["length"].as_u64().unwrap_or(32).min(1024) as usize;
                let rng = rand::SystemRandom::new();
                let mut bytes = vec![0u8; n];
                rand::SecureRandom::fill(&rng, &mut bytes).map_err(|_| anyhow::anyhow!("rng failed"))?;
                let out = encode_output(&bytes, format);
                Ok(ToolResult::ok(serde_json::json!({"secret": out, "format": format, "length": n})))
            }

            "pbkdf2" => {
                let password = args["input"].as_str().unwrap_or("");
                let iters = args["iterations"].as_u64().unwrap_or(100_000) as u32;
                let key_len = args["length"].as_u64().unwrap_or(32) as usize;

                let salt_bytes: Vec<u8> = if let Some(s) = args["salt"].as_str() {
                    base64::engine::general_purpose::STANDARD.decode(s).unwrap_or_default()
                } else {
                    let rng = rand::SystemRandom::new();
                    let mut s = vec![0u8; 16];
                    rand::SecureRandom::fill(&rng, &mut s).map_err(|_| anyhow::anyhow!("rng"))?;
                    s
                };

                let mut key = vec![0u8; key_len];
                pbkdf2::derive(
                    pbkdf2::PBKDF2_HMAC_SHA256,
                    std::num::NonZeroU32::new(iters).unwrap(),
                    &salt_bytes,
                    password.as_bytes(),
                    &mut key,
                );

                Ok(ToolResult::ok(serde_json::json!({
                    "key_b64":  base64::engine::general_purpose::STANDARD.encode(&key),
                    "salt_b64": base64::engine::general_purpose::STANDARD.encode(&salt_bytes),
                    "iterations": iters,
                })))
            }

            other => Ok(ToolResult::err(format!(
                "unknown action '{}' — use: hash|hmac|encrypt|decrypt|random|pbkdf2",
                other
            ))),
        }
    }
}

fn encode_output(bytes: &[u8], format: &str) -> String {
    match format {
        "base64" => base64::engine::general_purpose::STANDARD.encode(bytes),
        "raw" => String::from_utf8_lossy(bytes).into_owned(),
        _ => hex::encode(bytes),
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::*;

    #[tokio::test]
    async fn test_sha256_hash() {
        let tool = CryptoTool;
        let result = tool
            .execute(serde_json::json!({
                "action": "hash",
                "input": "hello"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output["hash"].as_str().is_some(), "expected 'hash' field in output");
    }

    #[tokio::test]
    async fn test_hmac_signing() {
        let tool = CryptoTool;
        let key_bytes = [0u8; 32];
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(&key_bytes);
        let result = tool
            .execute(serde_json::json!({
                "action": "hmac",
                "key": key_b64,
                "input": "test message"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output["signature"].as_str().is_some(), "expected 'signature' field");
    }

    #[tokio::test]
    async fn test_encrypt_decrypt_roundtrip() {
        let tool = CryptoTool;
        let key_bytes = [0u8; 32];
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(&key_bytes);
        let plaintext = "secret message";

        // Encrypt
        let enc_result = tool
            .execute(serde_json::json!({
                "action": "encrypt",
                "key": key_b64,
                "input": plaintext
            }))
            .await
            .unwrap();
        assert!(enc_result.success);
        let ciphertext_b64 = enc_result.output["ciphertext_b64"].as_str().unwrap();

        // Decrypt
        let dec_result = tool
            .execute(serde_json::json!({
                "action": "decrypt",
                "key": key_b64,
                "input": ciphertext_b64
            }))
            .await
            .unwrap();
        assert!(dec_result.success);
        assert_eq!(dec_result.output["plaintext"].as_str().unwrap(), plaintext);
    }

    #[tokio::test]
    async fn test_random_generation() {
        let tool = CryptoTool;
        let result = tool
            .execute(serde_json::json!({
                "action": "random",
                "length": 16
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output["secret"].as_str().is_some(), "expected 'secret' field");
    }

    #[tokio::test]
    async fn test_pbkdf2_derive() {
        let tool = CryptoTool;
        let result = tool
            .execute(serde_json::json!({
                "action": "pbkdf2",
                "input": "test"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output["key_b64"].as_str().is_some(), "expected 'key_b64' field");
    }
}
