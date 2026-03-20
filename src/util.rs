use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Generate a new unique agent ID.
pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

/// Return the current UTC timestamp.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Compute the next scheduled run time by adding `delay_secs` to now.
pub fn next_run_after(delay_secs: i64) -> DateTime<Utc> {
    Utc::now() + chrono::Duration::seconds(delay_secs)
}

/// Truncate a string to `max_chars` for safe logging.
pub fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        None => s,
        Some((idx, _)) => &s[..idx],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        let s = "hello";
        assert_eq!(truncate(s, 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let s = "hello world, this is a long string";
        let result = truncate(s, 5);
        assert_eq!(result.chars().count(), 5);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_next_run_after() {
        let before = Utc::now();
        let result = next_run_after(60);
        assert!(result > before);
    }
}
