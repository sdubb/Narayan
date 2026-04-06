use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::providers::Provider;

/// How complex a task is — determines which model/provider is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskComplexity {
    /// Quick lookups, simple Q&A, single-tool calls → cheapest model.
    Simple,
    /// Multi-step reasoning, moderate context → mid-tier model.
    Medium,
    /// Deep analysis, long context, multi-tool orchestration → best model.
    Complex,
}

impl TaskComplexity {
    /// Infer complexity from a task description heuristic.
    pub fn infer(description: &str) -> Self {
        let desc = description.to_lowercase();
        let len = desc.len();

        let complex_keywords = [
            "analyze",
            "research",
            "design",
            "architect",
            "synthesize",
            "compare",
            "evaluate",
            "strategy",
            "comprehensive",
            "detailed",
        ];
        let medium_keywords =
            ["search", "fetch", "find", "check", "list", "summarize", "write", "edit", "update", "create"];

        if len > 300 || complex_keywords.iter().any(|kw| desc.contains(kw)) {
            TaskComplexity::Complex
        } else if len > 60 || medium_keywords.iter().any(|kw| desc.contains(kw)) {
            TaskComplexity::Medium
        } else {
            TaskComplexity::Simple
        }
    }
}

/// Maps task complexity to a preferred provider name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingTable {
    /// Provider name to use for simple tasks.
    pub simple: String,
    /// Provider name to use for medium tasks.
    pub medium: String,
    /// Provider name to use for complex tasks.
    pub complex: String,
    /// Fallback provider if preferred is unavailable.
    pub fallback: String,
}

impl Default for RoutingTable {
    fn default() -> Self {
        Self {
            simple: "openrouter".into(),
            medium: "openrouter".into(),
            complex: "anthropic".into(),
            fallback: "openrouter".into(),
        }
    }
}

/// Routes a request to the best available provider based on complexity.
pub struct ProviderRouter {
    providers: HashMap<String, Arc<dyn Provider>>,
    table: RoutingTable,
}

impl ProviderRouter {
    pub fn new(providers: HashMap<String, Arc<dyn Provider>>, table: RoutingTable) -> Self {
        Self { providers, table }
    }

    /// Resolve the best provider for the given complexity.
    pub fn resolve(&self, complexity: &TaskComplexity) -> Result<Arc<dyn Provider>> {
        let preferred = match complexity {
            TaskComplexity::Simple => &self.table.simple,
            TaskComplexity::Medium => &self.table.medium,
            TaskComplexity::Complex => &self.table.complex,
        };

        // Try preferred, then fallback
        if let Some(p) = self.providers.get(preferred) {
            return Ok(p.clone());
        }

        self.providers.get(&self.table.fallback).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "no provider available for complexity {:?} (tried '{}' and fallback '{}')",
                complexity,
                preferred,
                self.table.fallback
            )
        })
    }

    pub fn available_providers(&self) -> Vec<&str> {
        self.providers.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complexity_infer_simple() {
        assert_eq!(TaskComplexity::infer("hi"), TaskComplexity::Simple);
    }

    #[test]
    fn test_complexity_infer_medium() {
        assert_eq!(TaskComplexity::infer("search for files"), TaskComplexity::Medium);
    }

    #[test]
    fn test_complexity_infer_complex() {
        assert_eq!(
            TaskComplexity::infer("analyze and design the system architecture comprehensively"),
            TaskComplexity::Complex,
        );
    }
}
