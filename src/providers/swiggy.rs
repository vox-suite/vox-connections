use super::food_delivery::{
    Client, FoodDeliveryError, FoodDeliveryOrder, Session, parse_orders, parse_status, string_field,
};
use crate::accounts::{ConnectorDescriptor, FreshConnectionError, FreshConnectionsService};
use serde_json::{Value, json};
use uuid::Uuid;

pub const LABEL: &str = "Swiggy";

#[derive(Clone)]
pub struct SwiggyClient {
    pub(crate) client: Client,
}
impl SwiggyClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            client: Client::new(
                http,
                "swiggy",
                "https://mcp.swiggy.com/food",
                "https://mcp.swiggy.com/auth",
                true,
                false,
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
    pub(crate) fn swiggy_descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            id: "swiggy".to_string(),
            name: LABEL.to_string(),
            description:
                "Read real food orders and delivery status through Swiggy account authorization."
                    .to_string(),
            supported_features: vec!["timeline_sync".to_string(), "assistant_read".to_string()],
            auth_type: "oauth2".to_string(),
            available: self.cipher.is_some()
                && self.core_api_url.is_some()
                && self.swiggy.client.enabled(),
        }
    }
    pub(crate) async fn sync_swiggy_in_context(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<usize, FreshConnectionError> {
        self.sync_food(user_id, id, "swiggy").await
    }
}

pub(crate) async fn read_orders(
    session: &mut Session<'_>,
) -> Result<Vec<FoodDeliveryOrder>, FoodDeliveryError> {
    let mut orders = Vec::new();
    let mut addresses = Vec::new();
    for page in 1..=20 {
        let data = session
            .call("get_addresses", json!({"page":page,"pageSize":10}))
            .await?;
        let batch = data
            .get("addresses")
            .and_then(Value::as_array)
            .ok_or_else(|| FoodDeliveryError::Parse("missing addresses array".into()))?;
        addresses.extend(batch.iter().cloned());
        let more = data
            .pointer("/pagination/hasMore")
            .and_then(Value::as_bool)
            .ok_or_else(|| FoodDeliveryError::Parse("missing address pagination".into()))?;
        if !more {
            break;
        }
        if page == 20 {
            return Err(FoodDeliveryError::Parse(
                "address pagination limit exceeded".into(),
            ));
        }
    }
    for address in addresses {
        let id = string_field(&address, &["id", "addressId", "address_id"])
            .ok_or_else(|| FoodDeliveryError::Parse("missing address ID".into()))?;
        let data = session
            .call("get_food_orders", json!({"addressId": id}))
            .await?;
        orders.extend(parse_orders(&data, "swiggy")?);
    }
    if orders.iter().any(|o| {
        o.status.is_active()
            || o.provider_data
                .get("isActiveOrder")
                .and_then(Value::as_bool)
                == Some(true)
    }) {
        let data = session.call("track_food_order", json!({})).await?;
        let tracked = data
            .get("orders")
            .and_then(Value::as_array)
            .ok_or_else(|| FoodDeliveryError::Parse("missing tracking orders".into()))?;
        for tracking in tracked {
            let id = string_field(tracking, &["orderId", "order_id"])
                .ok_or_else(|| FoodDeliveryError::Parse("missing tracking order ID".into()))?;
            if let Some(order) = orders.iter_mut().find(|o| o.order_id == id) {
                order.status = parse_status(tracking);
                order.provider_data["tracking"] = tracking.clone();
            }
        }
    }
    Ok(orders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::{Json, Router, routing::post};

    async fn server(response: Value) -> (Client, tokio::task::JoinHandle<()>) {
        let router = Router::new().route("/mcp", post(move |Json(request): Json<Value>| {
            let response = response.clone();
            async move {
                if request["method"] == "notifications/initialized" { return (StatusCode::ACCEPTED, Json(Value::Null)); }
                let result = if request["method"] == "initialize" { json!({"protocolVersion":"2025-03-26"}) } else if request["params"]["name"] == "get_addresses" {
                    json!({"structuredContent":{"success":true,"data":{"addresses":[{"id":"home","addressLine":"Provider address","phoneNumber":""}],"pagination":{"hasMore":false}}}})
                } else { response };
                (StatusCode::OK, Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut client = SwiggyClient::new(reqwest::Client::new()).client;
        client.endpoint = format!("http://{address}/mcp");
        (client, task)
    }
    #[tokio::test]
    async fn valid_empty_history_stays_empty() {
        let (client, task) =
            server(json!({"structuredContent":{"success":true,"data":{"orders":[]}}})).await;
        assert!(
            client
                .fetch_orders("real-test-session")
                .await
                .unwrap()
                .is_empty()
        );
        task.abort();
    }
    #[tokio::test]
    async fn malformed_history_is_an_error_instead_of_empty_success() {
        let (client, task) =
            server(json!({"structuredContent":{"success":true,"data":{"unexpected":[]}}})).await;
        assert!(matches!(
            client.fetch_orders("real-test-session").await,
            Err(FoodDeliveryError::Parse(_))
        ));
        task.abort();
    }
    #[tokio::test]
    async fn history_preserves_provider_values_without_inventing_missing_fields() {
        let (client, task) = server(json!({"content":[{"type":"text","text":json!({"success":true,"data":{"orders":[{
            "orderId":"901", "restaurantId":"r1", "restaurantName":"Provider restaurant", "orderTotal":"₹425.50", "orderStatus":"DELIVERED", "orderType":"FOOD", "orderedItems":"2 Dosas", "orderedTime":"2026-09-30T14:00:00+05:30", "isActiveOrder":false, "actions":[]
        }]}}).to_string()}]})).await;
        let orders = client.fetch_orders("real-test-session").await.unwrap();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].total_amount, Some(425.50));
        assert_eq!(orders[0].restaurant_location, None);
        assert_eq!(orders[0].delivered_time, None);
        assert_eq!(
            orders[0].order_time.unwrap().to_rfc3339(),
            "2026-09-30T08:30:00+00:00"
        );
        task.abort();
    }
}
