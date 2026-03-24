use anyhow::Result;
use serde::{Deserialize, Serialize};

const JWT_EXPIRY_SECS: u64 = 86_400; // 24 hours

/// Claims embedded in a JWT token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Tenant ID (subject).
    pub sub: String,
    /// Tenant plan.
    pub plan: String,
    /// Issued-at (Unix timestamp).
    pub iat: u64,
    /// Expiry (Unix timestamp).
    pub exp: u64,
}

impl Claims {
    pub fn new(tenant_id: String, plan: String) -> Self {
        let now = unix_now();
        Self { sub: tenant_id, plan, iat: now, exp: now + JWT_EXPIRY_SECS }
    }

    pub fn is_expired(&self) -> bool {
        unix_now() > self.exp
    }
}

/// Issue a JWT for a tenant after successful authentication.
///
/// Uses a simple HMAC-SHA256 signed token without external JWT crate dependency.
/// The format is: base64url(header).base64url(claims).base64url(signature)
pub fn issue_token(tenant_id: &str, plan: &str, secret: &str) -> Result<String> {
    let claims = Claims::new(tenant_id.to_string(), plan.to_string());

    let header = base64url(&serde_json::to_vec(&serde_json::json!({"alg":"HS256","typ":"JWT"}))?);
    let payload = base64url(&serde_json::to_vec(&claims)?);

    let signing_input = format!("{}.{}", header, payload);
    let signature = hmac_sha256(signing_input.as_bytes(), secret.as_bytes());
    let sig_b64 = base64url(&signature);

    Ok(format!("{}.{}.{}", header, payload, sig_b64))
}

/// Validate and decode a JWT. Returns `Claims` if valid.
pub fn validate_token(token: &str, secret: &str) -> Result<Claims> {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() != 3 {
        anyhow::bail!("invalid token format");
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let expected_sig = hmac_sha256(signing_input.as_bytes(), secret.as_bytes());
    let provided_sig = base64url_decode(parts[2]).map_err(|_| anyhow::anyhow!("invalid signature encoding"))?;

    // Constant-time comparison
    if expected_sig.len() != provided_sig.len() {
        anyhow::bail!("signature mismatch");
    }
    let mismatch = expected_sig.iter().zip(provided_sig.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if mismatch != 0 {
        anyhow::bail!("invalid token signature");
    }

    let payload_bytes = base64url_decode(parts[1]).map_err(|_| anyhow::anyhow!("invalid payload encoding"))?;
    let claims: Claims =
        serde_json::from_slice(&payload_bytes).map_err(|e| anyhow::anyhow!("invalid claims: {}", e))?;

    if claims.is_expired() {
        anyhow::bail!("token expired");
    }

    Ok(claims)
}

// ── Internal helpers ───────────────────────────────────────────────────────

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn base64url(input: &[u8]) -> String {
    let b64 = base64_encode(input);
    b64.replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    let padded = match input.len() % 4 {
        2 => format!("{}==", input),
        3 => format!("{}=", input),
        _ => input.to_string(),
    };
    let std_b64 = padded.replace('-', "+").replace('_', "/");
    base64_decode(&std_b64).map_err(|e| e.to_string())
}

/// Minimal base64 encoder (no external dep).
fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        out.push(CHARS[b0 >> 2] as char);
        out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[b2 & 0x3f] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Result<Vec<u8>, &'static str> {
    const LOOKUP: [i8; 256] = {
        let mut t = [-1i8; 256];
        let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0usize;
        while i < chars.len() {
            t[chars[i] as usize] = i as i8;
            i += 1;
        }
        t
    };
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i + 3 < bytes.len() {
        let (a, b, c, d) = (
            LOOKUP[bytes[i] as usize],
            LOOKUP[bytes[i + 1] as usize],
            LOOKUP[bytes[i + 2] as usize],
            LOOKUP[bytes[i + 3] as usize],
        );
        if a < 0 || b < 0 {
            return Err("invalid char");
        }
        out.push(((a as u8) << 2) | ((b as u8) >> 4));
        if c >= 0 {
            out.push(((b as u8 & 0xf) << 4) | ((c as u8) >> 2));
        }
        if d >= 0 {
            out.push(((c as u8 & 3) << 6) | (d as u8));
        }
        i += 4;
    }
    Ok(out)
}

/// HMAC-SHA256 using sha2 crate.
fn hmac_sha256(message: &[u8], key: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    // Simplified HMAC: H((K XOR opad) || H((K XOR ipad) || message))
    let block_size = 64usize;
    let mut k = if key.len() > block_size {
        let mut h = sha2::Sha256::new();
        h.update(key);
        h.finalize().to_vec()
    } else {
        key.to_vec()
    };
    k.resize(block_size, 0);

    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();

    let mut inner = sha2::Sha256::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = sha2::Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash);
    outer.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_and_validate_token_roundtrip() {
        let token = issue_token("tenant-123", "pro", "super-secret").expect("token should be issued");
        let claims = validate_token(&token, "super-secret").expect("token should validate");

        assert_eq!(claims.sub, "tenant-123");
        assert_eq!(claims.plan, "pro");
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn test_validate_token_rejects_wrong_secret() {
        let token = issue_token("tenant-123", "free", "correct-secret").expect("token should be issued");
        let error = validate_token(&token, "wrong-secret").expect_err("wrong secret must fail validation");
        assert!(error.to_string().contains("signature"));
    }

    #[test]
    fn test_validate_token_rejects_invalid_format() {
        let error = validate_token("not-a-jwt", "secret").expect_err("invalid token format must fail");
        assert!(error.to_string().contains("invalid token format"));
    }

    #[test]
    fn test_claims_is_expired_respects_expiry_boundary() {
        let expired = Claims { sub: "tenant-123".into(), plan: "pro".into(), iat: 1, exp: 1 };
        assert!(expired.is_expired());

        let active = Claims { sub: "tenant-123".into(), plan: "pro".into(), iat: 1, exp: u64::MAX };
        assert!(!active.is_expired());
    }

    #[test]
    fn test_base64url_roundtrip_handles_padding_cases() {
        for input in [b"a".as_slice(), b"ab".as_slice(), b"abc".as_slice(), b"hello world".as_slice()] {
            let encoded = base64url(input);
            let decoded = base64url_decode(&encoded).expect("encoded value should decode");
            assert_eq!(decoded, input);
        }
    }
}
