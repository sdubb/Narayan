//! ShipStation connector - shipping and fulfillment workflows.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct ShipStationConnector {
    http: reqwest::Client,
}

impl ShipStationConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn api_key(config: &ConnectorConfig) -> Option<String> {
        config
            .credentials
            .get("api_key")
            .or_else(|| config.credentials.get("access_token"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn api_secret(config: &ConnectorConfig) -> Option<String> {
        config
            .credentials
            .get("api_secret")
            .or_else(|| config.credentials.get("secret"))
            .or_else(|| config.credentials.get("auth_token"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn api_base() -> &'static str {
        "https://api.shipstation.com/v2"
    }
}

#[async_trait]
impl Connector for ShipStationConnector {
    fn connector_type(&self) -> &str {
        "shipstation"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "order_imported" | "order_created" => {
                let order_id = event.payload.get("orderId").and_then(|v| v.as_i64()).unwrap_or_default();
                let order_number = event.payload.get("orderNumber").and_then(|v| v.as_str()).unwrap_or("unknown");
                Ok(Some(format!(
                    "ShipStation order {order_number} (id {order_id}) imported. Validate address, service level, and fulfillment readiness."
                )))
            }
            "shipment_created" | "fulfillment_created" => {
                let shipment_id = event.payload.get("shipmentId").and_then(|v| v.as_i64()).unwrap_or_default();
                Ok(Some(format!(
                    "ShipStation shipment {shipment_id} created. Check tracking, carrier handoff, and customer notification."
                )))
            }
            "shipment_delayed" | "delivery_exception" => {
                let shipment_id = event.payload.get("shipmentId").and_then(|v| v.as_i64()).unwrap_or_default();
                Ok(Some(format!(
                    "ShipStation shipment {shipment_id} has a delay or exception. Investigate carrier status and draft a customer update."
                )))
            }
            _ => Ok(None),
        }
    }

    async fn deliver_output(
        &self,
        config: &ConnectorConfig,
        external_id: &str,
        output: &str,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        let api_key = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing ShipStation api_key"))?;
        let api_secret = Self::api_secret(config).ok_or_else(|| anyhow::anyhow!("missing ShipStation api_secret"))?;

        let delivery_type = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("log");
        if delivery_type != "fulfillment" {
            tracing::info!(delivery_type, external_id, "ShipStation delivery logged without API write");
            return Ok(());
        }

        let shipment_id = metadata.get("shipment_id").and_then(|v| v.as_i64()).unwrap_or_default();
        if shipment_id == 0 {
            tracing::info!(external_id, "ShipStation fulfillment missing shipment_id; logged only");
            return Ok(());
        }

        let body = serde_json::json!({
            "shipmentId": shipment_id,
            "internalNotes": output,
            "orderId": external_id,
        });
        let resp = self
            .http
            .post(format!("{}/fulfillments", Self::api_base()))
            .basic_auth(api_key, Some(api_secret))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("ShipStation delivery failed: {}", resp.status());
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let api_key = Self::api_key(config).ok_or_else(|| anyhow::anyhow!("missing api_key"))?;
        let api_secret = Self::api_secret(config).ok_or_else(|| anyhow::anyhow!("missing api_secret"))?;

        let resp = self
            .http
            .get(format!("{}/carriers", Self::api_base()))
            .basic_auth(api_key, Some(api_secret))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("ShipStation auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for ShipStationConnector {
    fn default() -> Self {
        Self::new()
    }
}
