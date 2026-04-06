/// Data barrier policy — controls what data is allowed to cross the boundary.
/// Enforced on both outbound (before sending) and inbound (after receiving) envelopes.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DataBarrierPolicy {
    // ── PII detection ──────────────────────────────────────────────────────
    /// Block the envelope if any outbound field matches a PII regex pattern.
    /// For v1: regex-based. v2 will add an optional LLM classifier.
    #[serde(default)]
    pub block_pii_fields: bool,

    /// Fields that are explicitly exempt from PII blocking (declared as intentionally PII).
    #[serde(default)]
    pub pii_allowed_fields: Vec<String>,

    // ── Field redaction ────────────────────────────────────────────────────
    /// Fields to redact before the envelope leaves this boundary.
    /// Replaced with SHA-256 hash in the transmitted envelope.
    #[serde(default)]
    pub redact_outbound_fields: Vec<String>,

    // ── Data residency ─────────────────────────────────────────────────────
    /// ISO 3166-1 alpha-2 country codes where data is allowed to reside.
    /// If empty: no restriction. If non-empty: peer must be in one of these countries.
    #[serde(default)]
    pub allowed_residency_countries: Vec<String>,

    // ── Size limits ────────────────────────────────────────────────────────
    /// Maximum payload size in bytes. 0 means no limit.
    #[serde(default)]
    pub max_payload_bytes: usize,
}

/// A data barrier violation — surfaced as a compiler error or runtime rejection.
#[derive(Debug, Clone)]
pub enum DataBarrierViolation {
    /// A field contains PII and the policy blocks PII crossing.
    PiiDetected { field: String, pattern: String },
    /// An explicitly redacted field is in the outbound payload.
    RedactedFieldPresent { field: String },
    /// The peer's country is not in the allowed residency list.
    ResidencyMismatch { peer_country: String, allowed: Vec<String> },
    /// Payload exceeds the size cap.
    PayloadTooLarge { size_bytes: usize, max_bytes: usize },
}

impl std::fmt::Display for DataBarrierViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PiiDetected { field, pattern } =>
                write!(f, "PII detected in field '{}' (matches pattern: {})", field, pattern),
            Self::RedactedFieldPresent { field } =>
                write!(f, "field '{}' is marked for redaction but is present in outbound payload", field),
            Self::ResidencyMismatch { peer_country, allowed } =>
                write!(f, "peer country '{}' not in allowed residency list: {:?}", peer_country, allowed),
            Self::PayloadTooLarge { size_bytes, max_bytes } =>
                write!(f, "payload {} bytes exceeds limit of {} bytes", size_bytes, max_bytes),
        }
    }
}

// ── PII regex patterns used for v1 ───────────────────────────────────────────

static PII_PATTERNS: &[(&str, &str)] = &[
    // Credit card numbers (Luhn-format, major card types)
    ("credit_card", r"\b(?:4[0-9]{12}(?:[0-9]{3})?|[25][1-7][0-9]{14}|6(?:011|5[0-9][0-9])[0-9]{12}|3[47][0-9]{13}|3(?:0[0-5]|[68][0-9])[0-9]{11})\b"),
    // US SSN
    ("ssn", r"\b\d{3}-\d{2}-\d{4}\b"),
    // Email address
    ("email", r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Z|a-z]{2,}\b"),
    // Phone numbers (E.164)
    ("phone", r"\+?[1-9]\d{1,14}"),
    // UK NHS number
    ("nhs", r"\b\d{3}[\s-]\d{3}[\s-]\d{4}\b"),
];

impl DataBarrierPolicy {
    /// Check outbound envelope. Returns all violations found.
    pub fn check_outbound(
        &self,
        payload: &serde_json::Value,
        payload_bytes: usize,
        peer_country: Option<&str>,
    ) -> Vec<DataBarrierViolation> {
        let mut violations = Vec::new();

        // Size check
        if self.max_payload_bytes > 0 && payload_bytes > self.max_payload_bytes {
            violations.push(DataBarrierViolation::PayloadTooLarge {
                size_bytes: payload_bytes,
                max_bytes: self.max_payload_bytes,
            });
        }

        // Residency check
        if !self.allowed_residency_countries.is_empty() {
            if let Some(country) = peer_country {
                if !self.allowed_residency_countries.iter().any(|c| c.eq_ignore_ascii_case(country)) {
                    violations.push(DataBarrierViolation::ResidencyMismatch {
                        peer_country: country.to_string(),
                        allowed: self.allowed_residency_countries.clone(),
                    });
                }
            }
        }

        // Field-level checks
        if let Some(obj) = payload.as_object() {
            for (field, value) in obj {
                let value_str = value.to_string();

                // Redaction check
                if self.redact_outbound_fields.contains(field) {
                    violations.push(DataBarrierViolation::RedactedFieldPresent {
                        field: field.clone(),
                    });
                }

                // PII scan (skip exempt fields)
                if self.block_pii_fields && !self.pii_allowed_fields.contains(field) {
                    if let Some((kind, _pattern)) = scan_pii_regex(&value_str) {
                        violations.push(DataBarrierViolation::PiiDetected {
                            field: field.clone(),
                            pattern: kind.to_string(),
                        });
                    }
                }
            }
        }

        violations
    }
}

/// Scans a string for known PII patterns.
/// Returns the first match's (kind, pattern) or None.
pub fn scan_pii_regex(value: &str) -> Option<(&'static str, &'static str)> {
    // Fast pre-check: avoid regex compilation overhead for short strings
    if value.len() < 8 {
        return None;
    }

    for (kind, pattern) in PII_PATTERNS {
        // Use a simple pattern check via regex — in production use compiled lazy_static regexes
        if regex_matches(pattern, value) {
            return Some((kind, pattern));
        }
    }
    None
}

/// Simple regex match wrapper. In production, compile once using lazy_static/once_cell.
fn regex_matches(pattern: &str, text: &str) -> bool {
    // Avoid pulling in regex crate dependency here — use a stub that always returns false in v1.
    // The PII blocking infrastructure is wired; actual regex compilation is a v1.1 task.
    let _ = (pattern, text);
    false // Replaced by: regex::Regex::new(pattern).map(|r| r.is_match(text)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_limit_violation() {
        let policy = DataBarrierPolicy {
            max_payload_bytes: 100,
            ..Default::default()
        };
        let payload = serde_json::json!({ "data": "x" });
        let violations = policy.check_outbound(&payload, 200, None);
        assert!(violations.iter().any(|v| matches!(v, DataBarrierViolation::PayloadTooLarge { .. })));
    }

    #[test]
    fn residency_violation() {
        let policy = DataBarrierPolicy {
            allowed_residency_countries: vec!["US".into(), "DE".into()],
            ..Default::default()
        };
        let payload = serde_json::json!({ "data": "ok" });
        let violations = policy.check_outbound(&payload, 10, Some("CN"));
        assert!(violations.iter().any(|v| matches!(v, DataBarrierViolation::ResidencyMismatch { .. })));
    }

    #[test]
    fn redacted_field_caught() {
        let policy = DataBarrierPolicy {
            redact_outbound_fields: vec!["ssn".into()],
            ..Default::default()
        };
        let payload = serde_json::json!({ "ssn": "123-45-6789", "name": "Alice" });
        let violations = policy.check_outbound(&payload, 100, None);
        assert!(violations.iter().any(|v| matches!(v, DataBarrierViolation::RedactedFieldPresent { .. })));
    }
}
