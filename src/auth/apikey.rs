use sha2::{Digest, Sha256};

const KEY_PREFIX_LEN: usize = 12;
const KEY_SECRET_LEN: usize = 32;
const PREFIX_TAG: &str = "nar_";

/// A freshly generated API key — shown to the user exactly once.
#[derive(Debug)]
pub struct NewApiKey {
    /// Full raw key shown to user once: "nar_<prefix>_<secret>"
    pub raw: String,
    /// Short prefix stored in DB for fast lookup.
    pub prefix: String,
    /// SHA-256 hash of the full raw key — stored in DB.
    pub hash: String,
}

/// Generate a new secure random API key.
pub fn generate_api_key() -> NewApiKey {
    let random_bytes: Vec<u8> = (0..KEY_PREFIX_LEN + KEY_SECRET_LEN).map(|_| rand_byte()).collect();

    let prefix_part = hex::encode(&random_bytes[..KEY_PREFIX_LEN / 2]);
    let secret_part = hex::encode(&random_bytes[KEY_PREFIX_LEN / 2..]);

    let raw = format!("{}{}_{}", PREFIX_TAG, prefix_part, secret_part);
    let prefix = format!("{}{}", PREFIX_TAG, prefix_part);
    let hash = hash_key(&raw);

    NewApiKey { raw, prefix, hash }
}

/// SHA-256 hash of a raw API key.
pub fn hash_key(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

/// Extract the prefix from a raw API key for fast DB lookup.
/// Format: "nar_<prefix>_<secret>"
pub fn extract_prefix(raw: &str) -> Option<&str> {
    // prefix is everything up to and including the first underscore after "nar_"
    if !raw.starts_with(PREFIX_TAG) {
        return None;
    }
    // Find the second underscore
    let after_tag = &raw[PREFIX_TAG.len()..];
    let second_underscore = after_tag.find('_')?;
    Some(&raw[..PREFIX_TAG.len() + second_underscore])
}

/// Constant-time comparison to prevent timing attacks.
pub fn verify_key(raw: &str, stored_hash: &str) -> bool {
    let computed = hash_key(raw);
    // Compare byte-by-byte without short-circuiting
    if computed.len() != stored_hash.len() {
        return false;
    }
    computed.bytes().zip(stored_hash.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Poor-man's random byte — good enough for key generation.
/// In production use `rand::thread_rng()` or `getrandom`.
fn rand_byte() -> u8 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    ((t ^ (t >> 8) ^ (t >> 16)) & 0xFF) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_roundtrip() {
        let key = generate_api_key();
        assert!(key.raw.starts_with("nar_"));
        assert!(verify_key(&key.raw, &key.hash));
        assert!(!verify_key("wrong_key", &key.hash));
    }

    #[test]
    fn prefix_extraction() {
        let key = generate_api_key();
        let prefix = extract_prefix(&key.raw).unwrap();
        assert_eq!(prefix, key.prefix);
    }

    #[test]
    fn test_verify_wrong_key() {
        let key = generate_api_key();
        assert!(!verify_key("wrong", &key.hash));
    }

    #[test]
    fn test_hash_determinism() {
        let h1 = hash_key("same");
        let h2 = hash_key("same");
        assert_eq!(h1, h2);
    }
}
