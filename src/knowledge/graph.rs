use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    nodes: HashMap<String, Node>,
    edges: Vec<(String, String)>,
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self { nodes: HashMap::new(), edges: Vec::new() }
    }

    pub fn add_node(&mut self, id: String, value: String) {
        self.nodes.insert(id.clone(), Node { id, value });
    }

    pub fn add_edge(&mut self, from: String, to: String) {
        self.edges.push((from, to));
    }

    pub fn get(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Return nodes whose id or value is related to the query.
    /// Simple keyword overlap — no embeddings needed.
    pub fn get_related(&self, query: &str) -> Vec<&Node> {
        let words: Vec<&str> = query.split_whitespace().collect();
        self.nodes
            .values()
            .filter(|n| {
                let combined = format!("{} {}", n.id, n.value).to_lowercase();
                words.iter().any(|w| combined.contains(&w.to_lowercase()))
            })
            .take(10)
            .collect()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_get_node() {
        let mut g = KnowledgeGraph::new();
        g.add_node("n1".into(), "some value".into());
        let node = g.get("n1").expect("node should exist");
        assert_eq!(node.id, "n1");
        assert_eq!(node.value, "some value");
        assert_eq!(g.node_count(), 1);
    }

    #[test]
    fn test_get_related() {
        let mut g = KnowledgeGraph::new();
        g.add_node("api_key".into(), "secret token".into());
        g.add_node("db_host".into(), "localhost".into());
        let results = g.get_related("api_key");
        assert!(!results.is_empty());
        assert!(results.iter().any(|n| n.id == "api_key"));
    }

    #[test]
    fn test_add_edge() {
        let mut g = KnowledgeGraph::new();
        g.add_node("a".into(), "node a".into());
        g.add_node("b".into(), "node b".into());
        g.add_edge("a".into(), "b".into());
        assert_eq!(g.edge_count(), 1);
    }
}
