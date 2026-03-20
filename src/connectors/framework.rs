//! Connector framework — pluggable external service integrations.
//!
//! Connectors receive inbound events (webhooks from GitHub, Zendesk, etc.)
//! and can trigger agent goals or deliver agent outputs to external systems.

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Configuration for an installed connector instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorConfig {
    pub id: String,
    pub tenant_id: String,
    pub connector_type: String,
    pub credentials: serde_json::Value,
    pub settings: serde_json::Value,
    pub enabled: bool,
}

/// An event received from an external system via a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorEvent {
    pub connector_type: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub tenant_id: String,
    pub external_id: Option<String>,
}

/// Trait for connector implementations.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Unique type identifier (e.g., "github", "zendesk").
    fn connector_type(&self) -> &str;

    /// Process an inbound webhook/event and return an optional goal description
    /// for the agent to work on.
    async fn handle_inbound(&self, event: &ConnectorEvent, config: &ConnectorConfig) -> Result<Option<String>>;

    /// Deliver an agent's output to the external system.
    async fn deliver_output(
        &self,
        config: &ConnectorConfig,
        external_id: &str,
        output: &str,
        metadata: &serde_json::Value,
    ) -> Result<()>;

    /// Validate connector configuration/credentials.
    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()>;
}

/// Registry of available connectors.
pub struct ConnectorRegistry {
    connectors: HashMap<String, Arc<dyn Connector>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self { connectors: HashMap::new() }
    }

    pub fn register(&mut self, connector: Arc<dyn Connector>) {
        self.connectors.insert(connector.connector_type().to_string(), connector);
    }

    pub fn get(&self, connector_type: &str) -> Option<Arc<dyn Connector>> {
        self.connectors.get(connector_type).cloned()
    }

    pub fn list(&self) -> Vec<&str> {
        self.connectors.keys().map(|k| k.as_str()).collect()
    }
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
