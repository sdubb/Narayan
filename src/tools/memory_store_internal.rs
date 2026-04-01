use std::sync::{Arc, OnceLock};

use dashmap::DashMap;

fn store() -> &'static Arc<DashMap<String, String>> {
    static CELL: OnceLock<Arc<DashMap<String, String>>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(DashMap::new()))
}

/// Get a value from the global in-memory store.
pub fn get(key: &str) -> Option<String> {
    store().get(key).map(|v| v.clone())
}

/// Insert a key-value pair.
pub fn insert(key: String, value: String) {
    store().insert(key, value);
}

/// Remove a key, returns true if it existed.
pub fn remove(key: &str) -> bool {
    store().remove(key).is_some()
}

/// DashMap ref for iteration (used by memory_recall, memory_forget).
pub fn with_store<F, R>(f: F) -> R
where
    F: FnOnce(&Arc<DashMap<String, String>>) -> R,
{
    f(store())
}

/// List key-value entries whose keys start with the provided prefix.
pub fn entries_with_prefix(prefix: &str) -> Vec<(String, String)> {
    store()
        .iter()
        .filter(|entry| entry.key().starts_with(prefix))
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect()
}
