//! PII detection and redaction pipeline.
//!
//! Scans text for common PII patterns and redacts or masks them.
//! Used before external API calls, in audit logs, and for compliance exports.

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiiMatch {
    pub pii_type: PiiType,
    pub start: usize,
    pub end: usize,
    pub original: String,
    pub redacted: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PiiType {
    Email,
    Phone,
    Ssn,
    CreditCard,
    IpAddress,
    ApiKey,
}

pub struct PiiRedactor {
    patterns: Vec<(PiiType, Regex)>,
}

impl PiiRedactor {
    pub fn new() -> Self {
        let patterns = vec![
            (PiiType::Email, Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap()),
            (PiiType::Phone, Regex::new(r"\b\+?1?[-.\s]?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b").unwrap()),
            (PiiType::Ssn, Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap()),
            (PiiType::CreditCard, Regex::new(r"\b\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b").unwrap()),
            (PiiType::IpAddress, Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").unwrap()),
            (PiiType::ApiKey, Regex::new(r"(?i)(sk-|api[_-]?key[=:]\s*)[a-zA-Z0-9_-]{20,}").unwrap()),
        ];
        Self { patterns }
    }

    /// Scan text and return all PII matches found.
    pub fn scan(&self, text: &str) -> Vec<PiiMatch> {
        let mut matches = Vec::new();
        for (pii_type, re) in &self.patterns {
            for m in re.find_iter(text) {
                let original = m.as_str().to_string();
                let redacted = redact_value(pii_type, &original);
                matches.push(PiiMatch {
                    pii_type: pii_type.clone(),
                    start: m.start(),
                    end: m.end(),
                    original,
                    redacted,
                });
            }
        }
        // Sort by start position
        matches.sort_by_key(|m| m.start);
        matches
    }

    /// Redact all PII in a string, replacing with masked values.
    pub fn redact(&self, text: &str) -> String {
        let matches = self.scan(text);
        if matches.is_empty() {
            return text.to_string();
        }

        let mut result = String::with_capacity(text.len());
        let mut last_end = 0;

        for m in &matches {
            if m.start > last_end {
                result.push_str(&text[last_end..m.start]);
            }
            result.push_str(&m.redacted);
            last_end = m.end;
        }

        if last_end < text.len() {
            result.push_str(&text[last_end..]);
        }

        result
    }

    /// Check if text contains any PII.
    pub fn contains_pii(&self, text: &str) -> bool {
        self.patterns.iter().any(|(_, re)| re.is_match(text))
    }
}

impl Default for PiiRedactor {
    fn default() -> Self {
        Self::new()
    }
}

fn redact_value(pii_type: &PiiType, original: &str) -> String {
    match pii_type {
        PiiType::Email => {
            if let Some(at_pos) = original.find('@') {
                format!("***@{}", &original[at_pos + 1..])
            } else {
                "[REDACTED_EMAIL]".into()
            }
        }
        PiiType::Phone => "[REDACTED_PHONE]".into(),
        PiiType::Ssn => "***-**-****".into(),
        PiiType::CreditCard => {
            let last4 = &original[original.len().saturating_sub(4)..];
            format!("****-****-****-{}", last4)
        }
        PiiType::IpAddress => "[REDACTED_IP]".into(),
        PiiType::ApiKey => "[REDACTED_KEY]".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_detection() {
        let r = PiiRedactor::new();
        let matches = r.scan("Contact john@example.com for info");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pii_type, PiiType::Email);
        assert_eq!(matches[0].redacted, "***@example.com");
    }

    #[test]
    fn test_ssn_detection() {
        let r = PiiRedactor::new();
        let matches = r.scan("SSN: 123-45-6789");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pii_type, PiiType::Ssn);
    }

    #[test]
    fn test_redact_replaces_all() {
        let r = PiiRedactor::new();
        let result = r.redact("Email john@example.com, SSN 123-45-6789");
        assert!(!result.contains("john@example.com"));
        assert!(!result.contains("123-45-6789"));
        assert!(result.contains("***@example.com"));
        assert!(result.contains("***-**-****"));
    }

    #[test]
    fn test_no_pii_returns_original() {
        let r = PiiRedactor::new();
        let text = "This is clean text with no PII";
        assert_eq!(r.redact(text), text);
        assert!(!r.contains_pii(text));
    }

    #[test]
    fn test_credit_card_keeps_last_four() {
        let r = PiiRedactor::new();
        let matches = r.scan("Card: 4111-1111-1111-1234");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].redacted, "****-****-****-1234");
    }
}
