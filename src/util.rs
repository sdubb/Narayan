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

/// Create a reqwest HTTP client with default timeout and error handling.
/// This replaces hardcoded `.build().unwrap()` calls across the codebase.
pub fn create_reqwest_client(timeout_secs: u64) -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to create http client: {}", e))
}

/// Create a JSON error response for HTTP handlers.
/// Standardizes error format across all API endpoints.
///
/// # Example
/// ```ignore
/// if user.is_none() {
///     return http_error(StatusCode::NOT_FOUND, "user not found");
/// }
/// ```
pub fn http_error(code: axum::http::StatusCode, msg: impl Into<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    (code, axum::Json(serde_json::json!({ "error": msg.into() }))).into_response()
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
