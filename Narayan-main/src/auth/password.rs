use std::num::NonZeroU32;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ring::{
    digest, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};

const HASH_PREFIX: &str = "pbkdf2_sha256";
const SALT_LEN: usize = 16;
const HASH_LEN: usize = digest::SHA256_OUTPUT_LEN;
const ITERATIONS: u32 = 120_000;

pub fn hash_password(password: &str) -> Result<String> {
    if password.trim().is_empty() {
        return Err(anyhow!("password required"));
    }

    let rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    rng.fill(&mut salt).map_err(|_| anyhow!("failed to generate password salt"))?;

    let mut hash = [0u8; HASH_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(ITERATIONS).expect("non-zero iterations"),
        &salt,
        password.as_bytes(),
        &mut hash,
    );

    Ok(format!("{HASH_PREFIX}${ITERATIONS}${}${}", STANDARD.encode(salt), STANDARD.encode(hash)))
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    let mut parts = encoded.split('$');
    let Some(prefix) = parts.next() else { return false };
    let Some(iterations) = parts.next() else { return false };
    let Some(salt_b64) = parts.next() else { return false };
    let Some(hash_b64) = parts.next() else { return false };

    if prefix != HASH_PREFIX || parts.next().is_some() {
        return false;
    }

    let Ok(iterations) = iterations.parse::<u32>() else { return false };
    let Some(iterations) = NonZeroU32::new(iterations) else { return false };
    let Ok(salt) = STANDARD.decode(salt_b64) else { return false };
    let Ok(hash) = STANDARD.decode(hash_b64) else { return false };

    pbkdf2::verify(pbkdf2::PBKDF2_HMAC_SHA256, iterations, &salt, password.as_bytes(), &hash).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let hash = hash_password("correct horse battery staple").expect("password should hash");
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn rejects_empty_passwords() {
        assert!(hash_password("   ").is_err());
    }
}
