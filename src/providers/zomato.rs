use super::food_delivery::{Client, FoodDeliveryError, FoodDeliveryOrder, Session, parse_orders};
use crate::accounts::{ConnectorDescriptor, FreshConnectionError, FreshConnectionsService};
use serde_json::{Value, json};
use uuid::Uuid;

pub const LABEL: &str = "Zomato";

#[derive(Clone)]
pub struct ZomatoClient {
    pub(crate) client: Client,
}
impl ZomatoClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            client: Client::new(
                http,
                "zomato",
                "https://mcp-server.zomato.com/mcp",
                "https://mcp-server.zomato.com",
                std::env::var("ZOMATO_MCP_ENABLED").is_ok_and(|v| v == "true"),
                true,
            ),
        }
    }
    pub async fn fetch_orders(
        &self,
        token: &str,
    ) -> Result<Vec<FoodDeliveryOrder>, FoodDeliveryError> {
        self.client.fetch_orders(token).await
    }
}

impl FreshConnectionsService {
    pub(crate) fn zomato_descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            id: "zomato".to_string(),
            name: LABEL.to_string(),
            description: "Read food orders through an approved Zomato account integration."
                .to_string(),
            supported_features: vec!["timeline_sync".to_string(), "assistant_read".to_string()],
            auth_type: "oauth2".to_string(),
            available: self.cipher.is_some()
                && self.core_api_url.is_some()
                && self.zomato.client.enabled(),
        }
    }
    pub async fn sync_zomato(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<usize, FreshConnectionError> {
        self.sync_food(user_id, id, "zomato").await
    }
}

pub(crate) async fn read_orders(
    session: &mut Session<'_>,
) -> Result<Vec<FoodDeliveryOrder>, FoodDeliveryError> {
    let tools = session.rpc("tools/list", json!({})).await?;
    let tools = tools
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| FoodDeliveryError::Parse("missing tool catalogue".into()))?;
    let tool = tools
        .iter()
        .find(|t| {
            matches!(
                t.get("name").and_then(Value::as_str),
                Some("get_orders" | "get_order_history" | "get_recent_orders")
            ) && t
                .pointer("/annotations/readOnlyHint")
                .and_then(Value::as_bool)
                == Some(true)
                && t.pointer("/inputSchema/required")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty)
        })
        .ok_or_else(|| {
            FoodDeliveryError::Parse(
                "Zomato has not exposed a supported read-only history tool".into(),
            )
        })?;
    let name = tool.get("name").and_then(Value::as_str).unwrap();
    parse_orders(&session.call(name, json!({})).await?, "zomato")
}
