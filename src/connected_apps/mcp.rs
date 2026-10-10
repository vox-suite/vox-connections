use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

use super::ConnectedAppError;
use crate::remote_extensions::adapters::transport::client_for_endpoint;

pub struct ToolCall<'a> {
    pub name: &'a str,
    pub expected_protocol: Option<&'a str>,
    pub reviewed_schema: &'a Value,
    pub arguments: Value,
}

const HANDSHAKE_PROTOCOL_VERSION: &str = "2025-11-25";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOOL_PAGES: usize = 10;
/// Upper bound for any single request; callers pass tighter per-call limits.
const CLIENT_CEILING: Duration = Duration::from_secs(60);
/// Re-resolve and re-pin DNS this often, so an address change is picked up.
const CLIENT_TTL: Duration = Duration::from_secs(300);
/// DNS-pinned HTTP clients for OAuth-time MCP tool discovery.
#[derive(Clone, Default)]
pub struct McpDiscoveryClient {
    clients: Arc<Mutex<HashMap<String, (reqwest::Client, Instant)>>>,
}

impl McpDiscoveryClient {
    async fn client(
        &self,
        endpoint: &str,
        allow_local: bool,
    ) -> Result<reqwest::Client, ConnectedAppError> {
        if let Some((client, created)) = self.clients.lock().unwrap().get(endpoint)
            && created.elapsed() < CLIENT_TTL
        {
            return Ok(client.clone());
        }
        let client = client_for_endpoint(endpoint, CLIENT_CEILING, allow_local)
            .await
            .map_err(|e| ConnectedAppError::Provider(e.to_string()))?;
        self.clients
            .lock()
            .unwrap()
            .insert(endpoint.to_string(), (client.clone(), Instant::now()));
        Ok(client)
    }

    /// Open a fresh session and list the server's tools with its info.
    pub async fn discover(
        &self,
        endpoint: &str,
        access_token: &str,
        timeout: Duration,
        allow_local: bool,
    ) -> Result<(Value, Vec<Value>), ConnectedAppError> {
        let http = self.client(endpoint, allow_local).await?;
        let mut session = McpSession::open(http, endpoint, access_token, timeout).await?;
        let tools = session.list_tools().await?;
        let mut info = session.server_info.clone();
        if !info.is_object() {
            info = json!({})
        }
        info["vox_protocol_version"] = json!(session.protocol_version);
        Ok((info, tools))
    }

    /// Execute one already-authorized tool only after its live schema matches
    /// the reviewed declaration in the same session. This checks advertised
    /// inventory, not real-world effects; authority checks belong to the caller.
    pub async fn call_tool(
        &self,
        endpoint: &str,
        access_token: &str,
        call: ToolCall<'_>,
        timeout: Duration,
        allow_local: bool,
    ) -> Result<Value, ConnectedAppError> {
        // Pagination and negotiation must share the call's deadline rather
        // than multiplying its timeout while the caller holds authority locks.
        tokio::time::timeout(timeout, async {
            let http = self.client(endpoint, allow_local).await?;
            let mut session = McpSession::open(http, endpoint, access_token, timeout).await?;
            if call
                .expected_protocol
                .is_some_and(|expected| expected != session.protocol_version)
            {
                return Err(ConnectedAppError::Invalid);
            }
            let tools = session.list_tools().await?;
            let mut matching = tools
                .iter()
                .filter(|tool| tool.get("name").and_then(Value::as_str) == Some(call.name));
            if matching.next().and_then(|tool| tool.get("inputSchema"))
                != Some(call.reviewed_schema)
                || matching.next().is_some()
            {
                return Err(ConnectedAppError::UnknownTool);
            }
            session
                .request(
                    "tools/call",
                    json!({"name": call.name, "arguments": call.arguments}),
                )
                .await
        })
        .await
        .map_err(|_| ConnectedAppError::Timeout)?
    }
}

/// One authenticated Streamable HTTP peer. Modern requests are stateless;
/// handshake requests retain the negotiated session identifier.
pub struct McpSession {
    http: reqwest::Client,
    endpoint: String,
    access_token: String,
    session_id: Option<String>,
    protocol_version: String,
    modern: bool,
    pub server_info: Value,
    timeout: Duration,
}

impl McpSession {
    async fn open(
        http: reqwest::Client,
        endpoint: &str,
        access_token: &str,
        timeout: Duration,
    ) -> Result<Self, ConnectedAppError> {
        let mut session = Self {
            http,
            endpoint: endpoint.to_string(),
            access_token: access_token.to_string(),
            session_id: None,
            protocol_version: HANDSHAKE_PROTOCOL_VERSION.to_string(),
            modern: false,
            server_info: Value::Null,
            timeout,
        };
        if let Some(server_info) = session.probe_modern().await? {
            session.protocol_version = MODERN_PROTOCOL_VERSION.to_string();
            session.modern = true;
            session.server_info = server_info;
            return Ok(session);
        }
        let result = session
            .request(
                "initialize",
                json!({
                    "protocolVersion": HANDSHAKE_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "Vox", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
            .await
            .map_err(|e| match e {
                ConnectedAppError::SessionExpired => {
                    ConnectedAppError::Provider("the app refused to start a session".into())
                }
                other => other,
            })?;
        if let Some(version) = result.get("protocolVersion").and_then(Value::as_str) {
            if version != HANDSHAKE_PROTOCOL_VERSION {
                return Err(ConnectedAppError::Provider(
                    "MCP protocol negotiation returned an unsupported version".into(),
                ));
            }
            session.protocol_version = version.to_string();
        }
        session.server_info = result.get("serverInfo").cloned().unwrap_or(Value::Null);
        session.notify("notifications/initialized").await?;
        Ok(session)
    }

    async fn probe_modern(&self) -> Result<Option<Value>, ConnectedAppError> {
        let id = Uuid::new_v4().to_string();
        let mut probe = self.http.post(&self.endpoint)
            .timeout(self.timeout)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", MODERN_PROTOCOL_VERSION)
            .header("mcp-method", "server/discover")
            .json(&json!({
                "jsonrpc": "2.0", "id": id, "method": "server/discover",
                "params": {"_meta": {
                    "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
                    "io.modelcontextprotocol/clientInfo": {"name": "Vox", "version": env!("CARGO_PKG_VERSION")},
                    "io.modelcontextprotocol/clientCapabilities": {}
                }}
            }))
            ;
        if !self.access_token.is_empty() {
            probe = probe.bearer_auth(&self.access_token);
        }
        let response = probe.send().await.map_err(network)?;
        if response.status().as_u16() == 401 {
            return Err(ConnectedAppError::Unauthorized);
        }
        let status = response.status();
        let is_event_stream = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let body = read_bounded(response).await?;
        let message = if is_event_stream {
            message_from_event_stream(&body, &id)?
        } else {
            serde_json::from_slice::<Value>(&body).map_err(|_| {
                ConnectedAppError::Provider("invalid modern MCP discovery response".into())
            })?
        };
        if message.get("id").and_then(Value::as_str) != Some(&id) {
            return Err(ConnectedAppError::Provider(
                "MCP discovery response id mismatch".into(),
            ));
        }
        if let Some(code) = message.pointer("/error/code").and_then(Value::as_i64) {
            if code == -32601 || code == -32022 {
                return Ok(None);
            }
            return Err(ConnectedAppError::Provider(format!(
                "modern MCP discovery error {code}"
            )));
        }
        if !status.is_success() {
            return Err(ConnectedAppError::Provider(format!(
                "modern MCP discovery HTTP {status}"
            )));
        }
        let result = message.get("result").ok_or_else(|| {
            ConnectedAppError::Provider("modern MCP discovery has no result".into())
        })?;
        if result.get("resultType").and_then(Value::as_str) != Some("complete") {
            return Err(ConnectedAppError::Provider(
                "modern MCP discovery is not complete".into(),
            ));
        }
        if let Some(versions) = result.get("supportedVersions").and_then(Value::as_array)
            && !versions
                .iter()
                .any(|version| version.as_str() == Some(MODERN_PROTOCOL_VERSION))
        {
            return Err(ConnectedAppError::Provider(
                "MCP server does not support the selected protocol version".into(),
            ));
        }
        Ok(Some(
            result
                .pointer("/_meta/io.modelcontextprotocol~1serverInfo")
                .cloned()
                .unwrap_or(Value::Null),
        ))
    }

    async fn list_tools(&mut self) -> Result<Vec<Value>, ConnectedAppError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            if let Some(page) = result.get("tools").and_then(Value::as_array) {
                tools.extend(page.iter().cloned());
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        if cursor.is_some() {
            return Err(ConnectedAppError::Provider(
                "MCP tool inventory exceeds the page limit".into(),
            ));
        }
        Ok(tools)
    }

    fn post(&self, initialize: bool) -> reqwest::RequestBuilder {
        let mut builder = self
            .http
            .post(&self.endpoint)
            .timeout(self.timeout)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        // The version header is only defined after negotiation.
        if !self.access_token.is_empty() {
            builder = builder.bearer_auth(&self.access_token);
        }
        if !initialize {
            builder = builder.header("mcp-protocol-version", &self.protocol_version);
        }
        if let Some(id) = &self.session_id {
            builder = builder.header("mcp-session-id", id);
        }
        builder
    }

    async fn notify(&mut self, method: &str) -> Result<(), ConnectedAppError> {
        let response = self
            .post(false)
            .json(&json!({"jsonrpc": "2.0", "method": method}))
            .send()
            .await
            .map_err(network)?;
        if response.status().as_u16() == 401 {
            return Err(ConnectedAppError::Unauthorized);
        }
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, ConnectedAppError> {
        let id = Uuid::new_v4().to_string();
        let mut params = params;
        let mut request = self.post(method == "initialize");
        if self.modern {
            if let Some(map) = params.as_object_mut() {
                map.insert("_meta".into(), json!({
                    "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
                    "io.modelcontextprotocol/clientInfo": {"name": "Vox", "version": env!("CARGO_PKG_VERSION")},
                    "io.modelcontextprotocol/clientCapabilities": {}
                }));
            }
            request = request.header("mcp-method", method);
            if let Some(name) = params.get("name").and_then(Value::as_str) {
                request = request.header("mcp-name", name);
            }
        }
        let response = request
            .json(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .send()
            .await
            .map_err(network)?;
        let status = response.status();
        if status.as_u16() == 401 {
            return Err(ConnectedAppError::Unauthorized);
        }
        // Spec: a request for a session the server no longer knows gets 404.
        if status.as_u16() == 404 && self.session_id.is_some() {
            return Err(ConnectedAppError::SessionExpired);
        }
        if let Some(session_id) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(session_id.to_string());
        }
        let is_event_stream = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let body = read_bounded(response).await?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&body).to_lowercase();
            if status.as_u16() == 400 && (text.contains("session") || text.contains("initializ")) {
                return Err(ConnectedAppError::SessionExpired);
            }
            return Err(ConnectedAppError::Provider(format!(
                "{method} returned HTTP {}",
                status.as_u16(),
            )));
        }
        let message = if is_event_stream {
            message_from_event_stream(&body, &id)?
        } else {
            serde_json::from_slice::<Value>(&body)
                .map_err(|_| ConnectedAppError::Provider(format!("{method}: invalid JSON")))?
        };
        rpc_result(message, &id, method)
    }
}

fn network(error: reqwest::Error) -> ConnectedAppError {
    if error.is_timeout() {
        ConnectedAppError::Timeout
    } else {
        ConnectedAppError::Provider("could not reach the app".into())
    }
}

async fn read_bounded(response: reqwest::Response) -> Result<Vec<u8>, ConnectedAppError> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(network)?;
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ConnectedAppError::Provider(
                "app response is too large".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Pick the JSON-RPC response for `id` out of an SSE body. Servers may send
/// notifications or progress events on the same stream before the response.
fn message_from_event_stream(body: &[u8], id: &str) -> Result<Value, ConnectedAppError> {
    let text = String::from_utf8_lossy(body);
    for event in text.split("\n\n") {
        let data: String = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        if let Ok(message) = serde_json::from_str::<Value>(&data)
            && message.get("id").and_then(Value::as_str) == Some(id)
        {
            return Ok(message);
        }
    }
    Err(ConnectedAppError::Provider(
        "app stream ended without a response".into(),
    ))
}

fn rpc_result(message: Value, id: &str, method: &str) -> Result<Value, ConnectedAppError> {
    if message.get("id").and_then(Value::as_str) != Some(id) {
        return Err(ConnectedAppError::Provider(format!(
            "{method}: response id mismatch"
        )));
    }
    if let Some(error) = message.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        let lower = text.to_lowercase();
        if method == "tools/call"
            && (code == -32601
                || lower.contains("unknown tool")
                || lower.contains("tool not found"))
        {
            return Err(ConnectedAppError::UnknownTool);
        }
        if (lower.contains("session") && (lower.contains("not found") || lower.contains("expired")))
            || lower.contains("not initialized")
        {
            return Err(ConnectedAppError::SessionExpired);
        }
        return Err(ConnectedAppError::Provider(format!(
            "{method}: MCP error {code}"
        )));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| ConnectedAppError::Provider(format!("{method}: missing result")))
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn mock_server(modern: bool) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock MCP server");
        let endpoint = format!("http://{}/mcp", listener.local_addr().expect("address"));
        let requests = if modern { 2 } else { 4 };
        let handle = std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().expect("accept MCP request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout");
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut one = [0_u8; 1];
                    stream.read_exact(&mut one).expect("request header");
                    bytes.push(one[0]);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break bytes.len();
                    }
                };
                let header = String::from_utf8_lossy(&bytes[..header_end]);
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                bytes.resize(header_end + length, 0);
                stream
                    .read_exact(&mut bytes[header_end..])
                    .expect("request body");
                let request: Value =
                    serde_json::from_slice(&bytes[header_end..]).expect("JSON request");
                let method = request
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("method");
                let id = request.get("id").cloned();
                let body = match method {
                    "server/discover" if modern => json!({
                        "jsonrpc":"2.0", "id":id, "result":{"resultType":"complete",
                        "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"modern"}}}
                    }),
                    "server/discover" => json!({"jsonrpc":"2.0","id":id,
                        "error":{"code":-32601,"message":"method not found"}}),
                    "initialize" => json!({"jsonrpc":"2.0","id":id,"result":{
                        "protocolVersion":HANDSHAKE_PROTOCOL_VERSION,"serverInfo":{"name":"legacy"}}}),
                    "notifications/initialized" => json!({}),
                    "tools/list" => json!({"jsonrpc":"2.0","id":id,"result":{
                        "tools":[{"name":"echo.read"}]}}),
                    _ => panic!("unexpected method {method}"),
                };
                let body = serde_json::to_vec(&body).expect("response JSON");
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).expect("response headers");
                stream.write_all(&body).expect("response body");
            }
        });
        (endpoint, handle)
    }

    #[tokio::test]
    async fn negotiates_modern_and_handshake_mcp_without_mixing_wire_eras() {
        for modern in [true, false] {
            let (endpoint, server) = mock_server(modern);
            let (server_info, tools) = McpDiscoveryClient::default()
                .discover(&endpoint, "test-token", Duration::from_secs(5), true)
                .await
                .expect("discover MCP server");
            assert_eq!(
                server_info["name"],
                if modern { "modern" } else { "legacy" }
            );
            assert_eq!(tools[0]["name"], "echo.read");
            server.join().expect("mock server finished");
        }
    }
}

#[cfg(test)]
mod dispatch_inventory_tests {
    use super::*;
    use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};

    #[derive(Clone)]
    struct Fixture {
        modern: bool,
        pages: Vec<Value>,
        delay: Duration,
        requests: Arc<Mutex<Vec<(String, HeaderMap)>>>,
    }

    struct Server {
        endpoint: String,
        fixture: Fixture,
        task: tokio::task::JoinHandle<()>,
    }

    impl Server {
        async fn start(modern: bool, pages: Vec<Value>, delay: Duration) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
            let fixture = Fixture {
                modern,
                pages,
                delay,
                requests: Arc::default(),
            };
            let router = Router::new()
                .route("/mcp", post(respond))
                .with_state(fixture.clone());
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            Self {
                endpoint,
                fixture,
                task,
            }
        }

        fn methods(&self) -> Vec<String> {
            self.fixture
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|(method, _)| method.clone())
                .collect()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn respond(
        State(fixture): State<Fixture>,
        headers: HeaderMap,
        Json(request): Json<Value>,
    ) -> (HeaderMap, Json<Value>) {
        let method = request["method"].as_str().unwrap();
        fixture
            .requests
            .lock()
            .unwrap()
            .push((method.into(), headers));
        let id = &request["id"];
        let mut response_headers = HeaderMap::new();
        let result = match method {
            "server/discover" if fixture.modern => {
                tokio::time::sleep(fixture.delay).await;
                json!({"resultType":"complete","supportedVersions":[MODERN_PROTOCOL_VERSION]})
            }
            "server/discover" => {
                return (
                    response_headers,
                    Json(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unsupported method"}}),
                    ),
                );
            }
            "initialize" => {
                response_headers.insert("mcp-session-id", "inventory-session".parse().unwrap());
                json!({"protocolVersion":HANDSHAKE_PROTOCOL_VERSION,"serverInfo":{}})
            }
            "notifications/initialized" => return (response_headers, Json(json!({}))),
            "tools/list" => {
                tokio::time::sleep(fixture.delay).await;
                let index = request
                    .pointer("/params/cursor")
                    .and_then(Value::as_str)
                    .map(|cursor| cursor.parse::<usize>().unwrap())
                    .unwrap_or(0);
                if fixture.pages.is_empty() {
                    return (
                        response_headers,
                        Json(
                            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":"inventory unavailable"}}),
                        ),
                    );
                }
                fixture.pages[index.min(fixture.pages.len() - 1)].clone()
            }
            "tools/call" => json!({"content":[{"type":"text","text":"fixture result"}]}),
            _ => panic!("unexpected method {method}"),
        };
        (
            response_headers,
            Json(json!({"jsonrpc":"2.0","id":id,"result":result})),
        )
    }

    fn schema() -> Value {
        json!({"type":"object","properties":{"text":{"type":"string"}}})
    }

    async fn dispatch(
        server: &Server,
        name: &str,
        protocol: &str,
        timeout: Duration,
    ) -> Result<Value, ConnectedAppError> {
        McpDiscoveryClient::default()
            .call_tool(
                &server.endpoint,
                "fixture-token",
                ToolCall {
                    name,
                    expected_protocol: Some(protocol),
                    reviewed_schema: &schema(),
                    arguments: json!({"text":"fixture"}),
                },
                timeout,
                true,
            )
            .await
    }

    #[tokio::test]
    async fn reads_and_writes_validate_inventory_in_the_dispatch_session_for_both_protocols() {
        for modern in [true, false] {
            for name in ["fixture.read", "fixture.write"] {
                let server = Server::start(
                    modern,
                    vec![json!({"tools":[{"name":name,"inputSchema":schema()}]})],
                    Duration::ZERO,
                )
                .await;
                let protocol = if modern {
                    MODERN_PROTOCOL_VERSION
                } else {
                    HANDSHAKE_PROTOCOL_VERSION
                };
                dispatch(&server, name, protocol, Duration::from_secs(5))
                    .await
                    .unwrap();
                assert_eq!(
                    server.methods(),
                    if modern {
                        vec!["server/discover", "tools/list", "tools/call"]
                    } else {
                        vec![
                            "server/discover",
                            "initialize",
                            "notifications/initialized",
                            "tools/list",
                            "tools/call",
                        ]
                    }
                );
                let requests = server.fixture.requests.lock().unwrap();
                for (method, headers) in requests
                    .iter()
                    .filter(|(method, _)| method == "tools/list" || method == "tools/call")
                {
                    assert_eq!(headers["authorization"], "Bearer fixture-token");
                    assert_eq!(headers["mcp-protocol-version"], protocol);
                    if modern {
                        assert_eq!(headers["mcp-method"], method.as_str());
                    } else {
                        assert_eq!(headers["mcp-session-id"], "inventory-session");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn unavailable_missing_changed_or_duplicate_inventory_never_dispatches() {
        for modern in [true, false] {
            for pages in [
                vec![],
                vec![json!({"tools":[]})],
                vec![json!({"tools":[{"name":"fixture.read","inputSchema":{"type":"object"}}]})],
                vec![
                    json!({"tools":[{"name":"fixture.read","inputSchema":schema()}],"nextCursor":"1"}),
                    json!({"tools":[{"name":"fixture.read","inputSchema":schema()}]}),
                ],
            ] {
                let server = Server::start(modern, pages, Duration::ZERO).await;
                let protocol = if modern {
                    MODERN_PROTOCOL_VERSION
                } else {
                    HANDSHAKE_PROTOCOL_VERSION
                };
                assert!(
                    dispatch(&server, "fixture.read", protocol, Duration::from_secs(5))
                        .await
                        .is_err()
                );
                assert!(!server.methods().iter().any(|method| method == "tools/call"));
            }
        }
    }

    #[tokio::test]
    async fn pagination_is_checked_before_dispatch_and_exhaustion_fails_closed() {
        let server = Server::start(
            true,
            vec![
                json!({"tools":[],"nextCursor":"1"}),
                json!({"tools":[{"name":"fixture.read","inputSchema":schema()}]}),
            ],
            Duration::ZERO,
        )
        .await;
        dispatch(
            &server,
            "fixture.read",
            MODERN_PROTOCOL_VERSION,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(
            server.methods(),
            ["server/discover", "tools/list", "tools/list", "tools/call"]
        );
        let repeated = Server::start(
            true,
            vec![
                json!({"tools":[{"name":"fixture.read","inputSchema":schema()}],"nextCursor":"1"}),
            ],
            Duration::ZERO,
        )
        .await;
        assert!(
            dispatch(
                &repeated,
                "fixture.read",
                MODERN_PROTOCOL_VERSION,
                Duration::from_secs(5)
            )
            .await
            .is_err()
        );
        assert_eq!(
            repeated
                .methods()
                .iter()
                .filter(|method| method.as_str() == "tools/list")
                .count(),
            MAX_TOOL_PAGES
        );
        assert!(
            !repeated
                .methods()
                .iter()
                .any(|method| method == "tools/call")
        );
    }

    #[tokio::test]
    async fn protocol_mismatch_prevents_inventory_and_dispatch() {
        let server = Server::start(
            true,
            vec![json!({"tools":[{"name":"fixture.read","inputSchema":schema()}]})],
            Duration::ZERO,
        )
        .await;
        assert!(matches!(
            dispatch(
                &server,
                "fixture.read",
                HANDSHAKE_PROTOCOL_VERSION,
                Duration::from_secs(5)
            )
            .await,
            Err(ConnectedAppError::Invalid)
        ));
        assert_eq!(server.methods(), ["server/discover"]);
    }

    #[tokio::test]
    async fn negotiation_and_inventory_share_one_deadline() {
        let server = Server::start(
            true,
            vec![json!({"tools":[{"name":"fixture.read","inputSchema":schema()}]})],
            Duration::from_millis(120),
        )
        .await;
        assert!(matches!(
            dispatch(
                &server,
                "fixture.read",
                MODERN_PROTOCOL_VERSION,
                Duration::from_millis(200)
            )
            .await,
            Err(ConnectedAppError::Timeout)
        ));
        let methods = server.methods();
        assert_eq!(methods.first().map(String::as_str), Some("server/discover"));
        assert!(!methods.iter().any(|method| method == "tools/call"));
    }
}
