use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, utoipa::ToSchema)]
pub struct GeoPoint {
    pub lat: f64,
    pub lng: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FoodDeliveryStatus {
    Unknown,
    Placed,
    Preparing,
    OutForDelivery,
    Delivered,
    Cancelled,
}
impl FoodDeliveryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Placed => "placed",
            Self::Preparing => "preparing",
            Self::OutForDelivery => "out_for_delivery",
            Self::Delivered => "delivered",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Placed | Self::Preparing | Self::OutForDelivery)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, utoipa::ToSchema)]
pub struct FoodDeliveryOrder {
    pub order_id: String,
    pub provider: String,
    pub restaurant_name: String,
    pub restaurant_location: Option<GeoPoint>,
    pub delivery_address: Option<String>,
    pub delivery_location: Option<GeoPoint>,
    pub rider_name: Option<String>,
    pub rider_phone: Option<String>,
    pub rider_location: Option<GeoPoint>,
    pub status: FoodDeliveryStatus,
    pub order_time: Option<DateTime<Utc>>,
    pub delivered_time: Option<DateTime<Utc>>,
    pub eta_minutes: Option<u32>,
    pub total_amount: Option<f64>,
    pub currency: Option<String>,
    pub items: Vec<String>,
    pub provider_data: Value,
}
#[derive(Debug, thiserror::Error)]
pub enum FoodDeliveryError {
    #[error("Food provider request failed: {0}")]
    Http(String),
    #[error("Food provider authorization expired or was revoked")]
    Unauthorized,
    #[error("Food provider response is unsupported: {0}")]
    Parse(String),
    #[error("Order not found: {0}")]
    NotFound(String),
}

#[derive(Clone)]
pub struct SwiggyClient {
    pub(crate) client: Client,
}
impl SwiggyClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            client: Client::new(http, "swiggy"),
        }
    }
    pub async fn fetch_orders(
        &self,
        token: &str,
    ) -> Result<Vec<FoodDeliveryOrder>, FoodDeliveryError> {
        self.client.fetch_orders(token).await
    }
    #[cfg(test)]
    fn parse_swiggy_order(v: &Value) -> Option<FoodDeliveryOrder> {
        parse_order(v, "swiggy")
    }
}
#[derive(Clone)]
pub struct ZomatoClient {
    pub(crate) client: Client,
}
impl ZomatoClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            client: Client::new(http, "zomato"),
        }
    }
    pub async fn fetch_orders(
        &self,
        token: &str,
    ) -> Result<Vec<FoodDeliveryOrder>, FoodDeliveryError> {
        self.client.fetch_orders(token).await
    }
    #[cfg(test)]
    fn parse_zomato_order(v: &Value) -> Option<FoodDeliveryOrder> {
        parse_order(v, "zomato")
    }
}

#[derive(Clone)]
pub(crate) struct Client {
    pub(crate) http: reqwest::Client,
    pub(crate) provider: &'static str,
    pub(crate) endpoint: String,
    pub(crate) auth_base: String,
    pub(crate) enabled: bool,
}
impl Client {
    fn new(http: reqwest::Client, provider: &'static str) -> Self {
        let endpoint = match provider {
            "swiggy" => "https://mcp.swiggy.com/food",
            _ => "https://mcp-server.zomato.com/mcp",
        };
        let auth_base = if provider == "swiggy" {
            "https://mcp.swiggy.com/auth"
        } else {
            "https://mcp-server.zomato.com"
        };
        let enabled = std::env::var(if provider == "swiggy" {
            "SWIGGY_MCP_ENABLED"
        } else {
            "ZOMATO_MCP_ENABLED"
        })
        .is_ok_and(|v| v == "true");
        Self {
            http,
            provider,
            endpoint: endpoint.into(),
            auth_base: auth_base.into(),
            enabled,
        }
    }
    pub(crate) async fn fetch_orders(
        &self,
        token: &str,
    ) -> Result<Vec<FoodDeliveryOrder>, FoodDeliveryError> {
        let mut session = Session::connect(self, token).await?;
        let mut orders = Vec::new();
        if self.provider == "swiggy" {
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
                orders.extend(parse_orders(&data, self.provider)?);
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
                    let id = string_field(tracking, &["orderId", "order_id"]).ok_or_else(|| {
                        FoodDeliveryError::Parse("missing tracking order ID".into())
                    })?;
                    if let Some(order) = orders.iter_mut().find(|o| o.order_id == id) {
                        order.status = parse_status(tracking);
                        order.provider_data["tracking"] = tracking.clone();
                    }
                }
            }
        } else {
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
            orders = parse_orders(&session.call(name, json!({})).await?, self.provider)?;
        }
        let mut seen = HashSet::new();
        orders.retain(|o| seen.insert(o.order_id.clone()));
        Ok(orders)
    }
}

struct Session<'a> {
    client: &'a Client,
    token: &'a str,
    id: Option<String>,
    version: String,
    next_id: u64,
}
impl<'a> Session<'a> {
    async fn connect(client: &'a Client, token: &'a str) -> Result<Self, FoodDeliveryError> {
        if token.trim().is_empty() {
            return Err(FoodDeliveryError::Unauthorized);
        }
        let mut session = Self {
            client,
            token,
            id: None,
            version: "2025-03-26".into(),
            next_id: 1,
        };
        let init = session.rpc("initialize", json!({"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"Vox","version":"0.1.0"}})).await?;
        session.version = init
            .get("protocolVersion")
            .and_then(Value::as_str)
            .filter(|v| matches!(*v, "2025-03-26" | "2025-06-18" | "2024-11-05"))
            .ok_or_else(|| FoodDeliveryError::Parse("unsupported MCP version".into()))?
            .into();
        session
            .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await?;
        Ok(session)
    }
    async fn send(&mut self, body: Value) -> Result<Option<Value>, FoodDeliveryError> {
        let mut request = self
            .client
            .http
            .post(&self.client.endpoint)
            .bearer_auth(self.token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", &self.version)
            .json(&body);
        if let Some(id) = &self.id {
            request = request.header("Mcp-Session-Id", id);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| FoodDeliveryError::Http("network unavailable".into()))?;
        if matches!(response.status().as_u16(), 401 | 403 | 419) {
            return Err(FoodDeliveryError::Unauthorized);
        }
        if !response.status().is_success() {
            return Err(FoodDeliveryError::Http(format!(
                "HTTP {}",
                response.status()
            )));
        }
        if let Some(id) = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
        {
            self.id = Some(id.to_string());
        }
        if body.get("id").is_none() {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| FoodDeliveryError::Http("response interrupted".into()))?
        {
            if bytes.len() + chunk.len() > 2_000_000 {
                return Err(FoodDeliveryError::Parse("response too large".into()));
            }
            bytes.extend_from_slice(&chunk);
            let text = std::str::from_utf8(&bytes).unwrap_or_default();
            if let Some(reply) = decode_rpc(text, &body["id"])? {
                return Ok(Some(reply));
            }
        }
        Err(FoodDeliveryError::Parse("missing JSON-RPC response".into()))
    }
    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, FoodDeliveryError> {
        let id = self.next_id;
        self.next_id += 1;
        let reply = self
            .send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?
            .ok_or_else(|| FoodDeliveryError::Parse("missing response".into()))?;
        if reply.pointer("/error/code").and_then(Value::as_i64) == Some(-32001) {
            return Err(FoodDeliveryError::Unauthorized);
        }
        if reply.get("error").is_some() {
            return Err(FoodDeliveryError::Http("MCP request rejected".into()));
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| FoodDeliveryError::Parse("missing result".into()))
    }
    async fn call(&mut self, name: &str, arguments: Value) -> Result<Value, FoodDeliveryError> {
        tool_data(
            self.rpc("tools/call", json!({"name":name,"arguments":arguments}))
                .await?,
        )
    }
}
fn decode_rpc(text: &str, id: &Value) -> Result<Option<Value>, FoodDeliveryError> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if &value["id"] == id {
            return Ok(Some(value));
        }
    }
    let normalized = text.replace("\r\n", "\n");
    for event in normalized
        .split("\n\n")
        .take(normalized.split("\n\n").count().saturating_sub(1))
    {
        let data = event
            .lines()
            .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            if &value["id"] == id {
                return Ok(Some(value));
            }
        }
    }
    Ok(None)
}
fn tool_data(result: Value) -> Result<Value, FoodDeliveryError> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(FoodDeliveryError::Http("tool request rejected".into()));
    }
    let value = if let Some(data) = result.get("structuredContent") {
        data.clone()
    } else {
        result
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .find_map(|t| serde_json::from_str::<Value>(t).ok())
            .ok_or_else(|| {
                FoodDeliveryError::Parse(
                    "history is not structured JSON; no timeline data was inferred".into(),
                )
            })?
    };
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(FoodDeliveryError::Http("provider read rejected".into()));
    }
    Ok(value.get("data").cloned().unwrap_or(value))
}
fn parse_orders(
    value: &Value,
    provider: &str,
) -> Result<Vec<FoodDeliveryOrder>, FoodDeliveryError> {
    let rows = value
        .get("orders")
        .and_then(Value::as_array)
        .ok_or_else(|| FoodDeliveryError::Parse("missing orders array".into()))?;
    rows.iter()
        .map(|row| {
            parse_order(row, provider).ok_or_else(|| {
                FoodDeliveryError::Parse("order missing ID or restaurant name".into())
            })
        })
        .collect()
}
fn string_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        v.get(*key).and_then(|value| match value {
            Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    })
}
fn point(v: &Value, lat: &str, lng: &str) -> Option<GeoPoint> {
    let lat = v.get(lat)?.as_f64()?;
    let lng = v.get(lng)?.as_f64()?;
    (lat.is_finite()
        && lng.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lng))
    .then_some(GeoPoint { lat, lng })
}
fn timestamp(v: &Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    keys.iter().find_map(|key| {
        let v = v.get(*key)?;
        if let Some(s) = v.as_str() {
            return DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc));
        }
        let n = v.as_i64()?;
        if n > 10_000_000_000 {
            DateTime::from_timestamp_millis(n)
        } else {
            DateTime::from_timestamp(n, 0)
        }
    })
}
fn parse_status(v: &Value) -> FoodDeliveryStatus {
    let status = string_field(
        v,
        &["order_status", "delivery_status", "orderStatus", "status"],
    )
    .unwrap_or_default();
    match status.to_uppercase().replace([' ', '-'], "_").as_str() {
        "PLACED" | "ORDER_PLACED" | "CONFIRMED" => FoodDeliveryStatus::Placed,
        "PREPARING" | "COOKING" => FoodDeliveryStatus::Preparing,
        "OUT_FOR_DELIVERY" | "PICKED_UP" | "IN_TRANSIT" | "ON_THE_WAY" | "DISPATCHED" => {
            FoodDeliveryStatus::OutForDelivery
        }
        "DELIVERED" => FoodDeliveryStatus::Delivered,
        "CANCELLED" | "CANCELED" => FoodDeliveryStatus::Cancelled,
        _ => FoodDeliveryStatus::Unknown,
    }
}
fn parse_order(v: &Value, provider: &str) -> Option<FoodDeliveryOrder> {
    let order_id = string_field(v, &["order_id", "orderId", "id"])?;
    let restaurant_name = string_field(v, &["restaurant_name", "restaurantName", "res_name"])
        .or_else(|| v.get("restaurant").and_then(|r| string_field(r, &["name"])))?;
    let total_amount = ["order_total", "orderTotal", "total_cost"]
        .iter()
        .find_map(|k| v.get(*k))
        .and_then(|n| {
            n.as_f64().or_else(|| {
                n.as_str().and_then(|s| {
                    s.trim()
                        .trim_start_matches('₹')
                        .replace(',', "")
                        .parse::<f64>()
                        .ok()
                })
            })
        })
        .filter(|n| n.is_finite() && *n >= 0.0);
    let items = ["order_items", "items"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_array))
        .map(|a| {
            a.iter()
                .filter_map(|v| string_field(v, &["name"]))
                .collect()
        })
        .or_else(|| string_field(v, &["orderedItems"]).map(|s| vec![s]))
        .unwrap_or_default();
    Some(FoodDeliveryOrder {
        order_id,
        provider: provider.into(),
        restaurant_name,
        restaurant_location: point(v, "restaurant_lat", "restaurant_lng")
            .or_else(|| point(v, "res_lat", "res_lng")),
        delivery_location: point(v, "delivery_lat", "delivery_lng"),
        rider_location: point(v, "delivery_boy_latitude", "delivery_boy_longitude")
            .or_else(|| point(v, "rider_lat", "rider_lng")),
        delivery_address: string_field(v, &["delivery_address"]),
        rider_name: string_field(v, &["delivery_boy_name", "rider_name"]),
        rider_phone: string_field(v, &["delivery_boy_phone", "rider_phone"]),
        status: parse_status(v),
        order_time: timestamp(v, &["order_time", "orderedTime", "placedAt"]),
        delivered_time: timestamp(v, &["delivered_time", "deliveredAt"]),
        eta_minutes: v
            .get("eta_minutes")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        total_amount,
        currency: string_field(v, &["currency"]),
        items,
        provider_data: v.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_coordinates_are_not_replaced_with_bengaluru() {
        let order = SwiggyClient::parse_swiggy_order(&json!({
            "order_id":"123", "restaurant_name":"Actual restaurant", "status":"PLACED"
        }))
        .unwrap();
        assert_eq!(order.restaurant_location, None);
        assert_eq!(order.delivery_location, None);
    }

    #[test]
    fn missing_status_is_not_reported_as_delivered() {
        let order = ZomatoClient::parse_zomato_order(&json!({
            "order_id":"123", "res_name":"Actual restaurant"
        }))
        .unwrap();
        assert_eq!(order.status.as_str(), "unknown");
        assert_eq!(order.delivered_time, None);
    }

    #[test]
    fn order_timestamp_comes_from_provider_and_is_stable() {
        let order = SwiggyClient::parse_swiggy_order(&json!({
            "order_id":"123", "restaurant_name":"Actual restaurant", "status":"DELIVERED",
            "order_time":"2026-10-01T12:30:00Z", "delivered_time":"2026-10-01T13:00:00Z"
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(order.order_time).unwrap(),
            json!("2026-10-01T12:30:00Z")
        );
        assert_eq!(
            serde_json::to_value(order.delivered_time).unwrap(),
            json!("2026-10-01T13:00:00Z")
        );
    }
}

#[cfg(test)]
mod mcp_tests {
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
        let mut client = Client::new(reqwest::Client::new(), "swiggy");
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
    #[test]
    fn sse_waits_for_complete_matching_response() {
        let input =
            "event: message\ndata: {\ndata: \"jsonrpc\":\"2.0\",\"id\":4,\"result\":{}}\n\n";
        assert!(decode_rpc(input, &json!(3)).unwrap().is_none());
        assert_eq!(
            decode_rpc(input, &json!(4)).unwrap().unwrap()["result"],
            json!({})
        );
    }
    #[test]
    fn missing_timestamp_and_amount_remain_unknown() {
        let order = parse_order(
            &json!({"orderId":"901","restaurantName":"Provider restaurant"}),
            "swiggy",
        )
        .unwrap();
        assert_eq!(order.order_time, None);
        assert_eq!(order.total_amount, None);
        assert_eq!(order.status, FoodDeliveryStatus::Unknown);
    }
}
