//! Shopify connector - commerce and fulfillment workflows.

use anyhow::Result;
use async_trait::async_trait;

use crate::connectors::framework::{Connector, ConnectorConfig, ConnectorEvent};

pub struct ShopifyConnector {
    http: reqwest::Client,
}

impl ShopifyConnector {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    fn shop_domain(config: &ConnectorConfig) -> Option<String> {
        config
            .settings
            .get("shop_domain")
            .or_else(|| config.settings.get("shop"))
            .or_else(|| config.credentials.get("shop_domain"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim().trim_end_matches(".myshopify.com").to_string())
    }

    fn access_token(config: &ConnectorConfig) -> Option<String> {
        config
            .credentials
            .get("access_token")
            .or_else(|| config.credentials.get("api_key"))
            .or_else(|| config.credentials.get("token"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn api_base(shop_domain: &str) -> String {
        format!("https://{shop_domain}.myshopify.com/admin/api/2025-01")
    }
}

#[async_trait]
impl Connector for ShopifyConnector {
    fn connector_type(&self) -> &str {
        "shopify"
    }

    async fn handle_inbound(&self, event: &ConnectorEvent, _config: &ConnectorConfig) -> Result<Option<String>> {
        match event.event_type.as_str() {
            "orders/create" | "order_created" => {
                let order_id = event.payload.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
                let name = event.payload.get("name").and_then(|v| v.as_str()).unwrap_or("#unknown");
                let total = event.payload.get("total_price").and_then(|v| v.as_str()).unwrap_or("0");
                Ok(Some(format!(
                    "New Shopify order {name} (id {order_id}) for ${total}. Verify payment, shipping address, fraud risk, inventory, and fulfillment routing."
                )))
            }
            "orders/updated" | "order_updated" => {
                let order_id = event.payload.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
                Ok(Some(format!(
                    "Shopify order {order_id} was updated. Review changes, confirm fulfillment status, and check whether customer communication is needed."
                )))
            }
            "refunds/create" | "refund_created" => {
                let order_id = event.payload.get("order_id").and_then(|v| v.as_i64()).unwrap_or_default();
                Ok(Some(format!(
                    "Shopify refund created for order {order_id}. Confirm refund reason, customer communication, and inventory reconciliation."
                )))
            }
            "fulfillments/create" | "fulfillment_created" => {
                let order_id = event.payload.get("order_id").and_then(|v| v.as_i64()).unwrap_or_default();
                Ok(Some(format!(
                    "Shopify fulfillment created for order {order_id}. Check tracking, delivery estimates, and any exception handling needed."
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
        let shop_domain = Self::shop_domain(config).ok_or_else(|| anyhow::anyhow!("missing Shopify shop_domain"))?;
        let access_token = Self::access_token(config).ok_or_else(|| anyhow::anyhow!("missing Shopify access_token"))?;

        let delivery_type = metadata.get("delivery_type").and_then(|v| v.as_str()).unwrap_or("order_note");
        if delivery_type != "order_note" {
            tracing::info!(delivery_type, external_id, "Shopify delivery logged without API write");
            return Ok(());
        }

        let url = format!("{}/orders/{}.json", Self::api_base(&shop_domain), external_id);
        let body = serde_json::json!({
            "order": {
                "id": external_id,
                "note": output,
            }
        });
        let resp = self
            .http
            .put(&url)
            .header("X-Shopify-Access-Token", access_token)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Shopify delivery failed: {}", resp.status());
        }
        Ok(())
    }

    async fn validate_config(&self, config: &ConnectorConfig) -> Result<()> {
        let shop_domain = Self::shop_domain(config).ok_or_else(|| anyhow::anyhow!("missing shop_domain"))?;
        let access_token = Self::access_token(config).ok_or_else(|| anyhow::anyhow!("missing access_token"))?;

        let url = format!("{}/shop.json", Self::api_base(&shop_domain));
        let resp = self
            .http
            .get(&url)
            .header("X-Shopify-Access-Token", access_token)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Shopify auth validation failed: {}", resp.status());
        }
        Ok(())
    }
}

impl Default for ShopifyConnector {
    fn default() -> Self {
        Self::new()
    }
}
