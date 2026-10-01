//! Fixed Google REST reads behind the existing MCP connection contract.
//! Access tokens are request-local and forwarded only to the fixed Google API.
use crate::remote_extensions::adapters::transport::client_for_endpoint;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

pub const CALENDAR_TOOL: &str = "google_calendar.list_events";
pub const DRIVE_TOOL: &str = "google_drive.list_files";
pub const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.readonly";
pub const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.metadata.readonly";
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct GoogleReadAdapter {
    base: String,
    local_test: bool,
}
impl Default for GoogleReadAdapter {
    fn default() -> Self {
        Self::new()
    }
}
impl GoogleReadAdapter {
    pub fn new() -> Self {
        Self {
            base: "https://www.googleapis.com".into(),
            local_test: false,
        }
    }
    /// Only isolated loopback fixtures may override the fixed production upstream.
    pub fn with_local_upstream_for_testing(base: &str) -> Result<Self, &'static str> {
        let url = url::Url::parse(base).map_err(|_| "invalid fixture URL")?;
        if url.scheme() != "http"
            || !url.host_str().is_some_and(|host| {
                host.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            })
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err("fixture upstream must be a loopback HTTP origin");
        }
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            local_test: true,
        })
    }
    pub fn router(self) -> Router {
        Router::new()
            .route("/calendar/mcp", post(calendar_mcp))
            .route("/drive/mcp", post(drive_mcp))
            .layer(DefaultBodyLimit::max(16 * 1024))
            .with_state(self)
    }
    async fn get(
        &self,
        token: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, ReadError> {
        tokio::time::timeout(TIMEOUT, async {
            let endpoint = format!("{}{path}", self.base);
            let http = client_for_endpoint(&endpoint, TIMEOUT, self.local_test)
                .await
                .map_err(|_| ReadError::Unavailable)?;
            let response = http
                .get(endpoint)
                .bearer_auth(token)
                .query(query)
                .send()
                .await
                .map_err(|_| ReadError::Unavailable)?;
            match response.status().as_u16() {
                401 => return Err(ReadError::Unauthorized),
                403 => return Err(ReadError::Forbidden),
                429 => return Err(ReadError::RateLimited),
                _ if !response.status().is_success() => return Err(ReadError::Unavailable),
                _ => (),
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| ReadError::Unavailable)?;
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                    return Err(ReadError::Unavailable);
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).map_err(|_| ReadError::Unavailable)
        })
        .await
        .map_err(|_| ReadError::Unavailable)?
    }
    async fn verify(&self, token: &str, calendar: bool) -> Result<(), ReadError> {
        // Confirm the provider accepts this token for the endpoint's actual read.
        // Static tools/list alone would falsely report revoked/reduced-scope tokens as linked.
        let (path, query) = if calendar {
            (
                "/calendar/v3/calendars/primary/events",
                vec![("maxResults", "1".into()), ("fields", "items(id)".into())],
            )
        } else {
            (
                "/drive/v3/files",
                vec![
                    ("pageSize", "1".into()),
                    ("q", "trashed = false".into()),
                    ("fields", "files(id)".into()),
                ],
            )
        };
        self.get(token, path, &query).await.map(|_| ())
    }
    async fn calendar(&self, token: &str, arguments: Value) -> Result<Value, ReadError> {
        let args: CalendarArgs =
            serde_json::from_value(arguments).map_err(|_| ReadError::Invalid)?;
        if args.time_min.len() > 64 || args.time_max.len() > 64 {
            return Err(ReadError::Invalid);
        }
        let min =
            chrono::DateTime::parse_from_rfc3339(&args.time_min).map_err(|_| ReadError::Invalid)?;
        let max =
            chrono::DateTime::parse_from_rfc3339(&args.time_max).map_err(|_| ReadError::Invalid)?;
        if max <= min
            || max - min > chrono::Duration::days(31)
            || !(1..=100).contains(&args.page_size)
            || args
                .page_token
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 2048)
        {
            return Err(ReadError::Invalid);
        }
        let mut query = vec![
            ("timeMin", args.time_min),
            ("timeMax", args.time_max),
            ("singleEvents", "true".into()),
            ("orderBy", "startTime".into()),
            ("showDeleted", "false".into()),
            ("maxResults", args.page_size.to_string()),
            (
                "fields",
                "nextPageToken,timeZone,items(id,summary,start,end,status)".into(),
            ),
        ];
        if let Some(page) = args.page_token {
            query.push(("pageToken", page));
        }
        let result = self
            .get(token, "/calendar/v3/calendars/primary/events", &query)
            .await?;
        let items = result
            .get("items")
            .and_then(Value::as_array)
            .ok_or(ReadError::Unavailable)?;
        if items.len() > 100 {
            return Err(ReadError::Unavailable);
        }
        Ok(
            json!({"calendar_id":"primary", "time_min":min.to_rfc3339(), "time_max":max.to_rfc3339(), "time_zone":result.get("timeZone"), "events":items.iter().map(|item| project(item, &["id","summary","start","end","status"])).collect::<Vec<_>>(), "next_page_token":page_token(&result)?, "complete":result.get("nextPageToken").is_none()}),
        )
    }
    async fn drive(&self, token: &str, arguments: Value) -> Result<Value, ReadError> {
        let args: DriveArgs = serde_json::from_value(arguments).map_err(|_| ReadError::Invalid)?;
        if !(1..=100).contains(&args.page_size)
            || args
                .name_contains
                .as_ref()
                .is_some_and(|v| v.trim().is_empty() || v.len() > 256)
            || args
                .page_token
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 2048)
        {
            return Err(ReadError::Invalid);
        }
        let mut q = "trashed = false".to_string();
        if let Some(name) = args.name_contains {
            q.push_str(&format!(
                " and name contains '{}'",
                name.replace('\\', "\\\\").replace('\'', "\\'")
            ));
        }
        let mut query = vec![
            ("q", q),
            ("corpora", "user".into()),
            ("pageSize", args.page_size.to_string()),
            ("orderBy", "modifiedTime desc".into()),
            (
                "fields",
                "nextPageToken,incompleteSearch,files(id,name,mimeType,modifiedTime,webViewLink)"
                    .into(),
            ),
        ];
        if let Some(page) = args.page_token {
            query.push(("pageToken", page));
        }
        let result = self.get(token, "/drive/v3/files", &query).await?;
        let files = result
            .get("files")
            .and_then(Value::as_array)
            .ok_or(ReadError::Unavailable)?;
        if files.len() > 100 {
            return Err(ReadError::Unavailable);
        }
        let incomplete = result
            .get("incompleteSearch")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(
            json!({"files":files.iter().map(|item| project(item, &["id","name","mimeType","modifiedTime","webViewLink"])).collect::<Vec<_>>(), "next_page_token":page_token(&result)?, "incomplete_search":incomplete, "complete":!incomplete && result.get("nextPageToken").is_none(), "content_included":false}),
        )
    }
}
fn page_token(result: &Value) -> Result<Option<&str>, ReadError> {
    match result.get("nextPageToken") {
        None => Ok(None),
        Some(Value::String(value)) if !value.is_empty() && value.len() <= 2048 => Ok(Some(value)),
        _ => Err(ReadError::Unavailable),
    }
}
fn project(value: &Value, keys: &[&str]) -> Value {
    let mut object = serde_json::Map::new();
    for key in keys {
        if let Some(value) = value.get(key) {
            object.insert((*key).into(), value.clone());
        }
    }
    Value::Object(object)
}
fn default_page_size() -> usize {
    50
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CalendarArgs {
    time_min: String,
    time_max: String,
    #[serde(default = "default_page_size")]
    page_size: usize,
    #[serde(default)]
    page_token: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriveArgs {
    #[serde(default)]
    name_contains: Option<String>,
    #[serde(default = "default_page_size")]
    page_size: usize,
    #[serde(default)]
    page_token: Option<String>,
}
#[derive(Clone, Copy)]
enum ReadError {
    Invalid,
    Unauthorized,
    Forbidden,
    RateLimited,
    Unavailable,
}
impl ReadError {
    fn response(self, id: Value) -> Response {
        let (status, code, message) = match self {
            Self::Invalid => (StatusCode::OK, -32602, "Invalid bounded read arguments"),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                -32001,
                "Google account authorization is unavailable",
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                -32001,
                "Google read access is unavailable",
            ),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                -32000,
                "Google rate limit reached",
            ),
            Self::Unavailable => (
                StatusCode::BAD_GATEWAY,
                -32000,
                "Google read is unavailable",
            ),
        };
        (
            status,
            Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})),
        )
            .into_response()
    }
}
pub fn tool_definition(calendar: bool) -> Value {
    let schema = if calendar {
        json!({"type":"object","properties":{"time_min":{"type":"string","format":"date-time","maxLength":64},"time_max":{"type":"string","format":"date-time","maxLength":64},"page_size":{"type":"integer","minimum":1,"maximum":100},"page_token":{"type":"string","minLength":1,"maxLength":2048}},"required":["time_min","time_max"],"additionalProperties":false})
    } else {
        json!({"type":"object","properties":{"name_contains":{"type":"string","minLength":1,"maxLength":256},"page_size":{"type":"integer","minimum":1,"maximum":100},"page_token":{"type":"string","minLength":1,"maxLength":2048}},"additionalProperties":false})
    };
    json!({"name":if calendar {CALENDAR_TOOL} else {DRIVE_TOOL},"description":if calendar {"Read primary-calendar events for an explicit range of at most 31 days; follow page tokens before claiming a total."} else {"Search nontrashed Drive file metadata; no file content is read; follow page tokens before claiming completeness."},"inputSchema":schema,"annotations":{"readOnlyHint":true,"destructiveHint":false}})
}
async fn calendar_mcp(
    State(adapter): State<GoogleReadAdapter>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    handle(adapter, headers, request, true).await
}
async fn drive_mcp(
    State(adapter): State<GoogleReadAdapter>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    handle(adapter, headers, request, false).await
}
async fn handle(
    adapter: GoogleReadAdapter,
    headers: HeaderMap,
    request: Value,
    calendar: bool,
) -> Response {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(token) = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|t| !t.is_empty() && t.len() <= 16 * 1024)
    else {
        return ReadError::Unauthorized.response(id);
    };
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return ReadError::Invalid.response(id);
    }
    let result = match request.get("method").and_then(Value::as_str) {
        Some("server/discover") => return Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Use the 2025-11-25 MCP handshake"}})).into_response(),
        Some("initialize") => {
            if request.pointer("/params/protocolVersion").and_then(Value::as_str)!=Some("2025-11-25") { return ReadError::Invalid.response(id); }
            json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"vox-google-rest-reads","version":"1"}})
        }
        Some("notifications/initialized") => return StatusCode::ACCEPTED.into_response(),
        Some("tools/list") => {
            if let Err(error)=adapter.verify(token, calendar).await {return error.response(id)};
            json!({"tools":[tool_definition(calendar)]})
        }
        Some("tools/call") => {
            if request.pointer("/params/name").and_then(Value::as_str)!=Some(if calendar {CALENDAR_TOOL} else {DRIVE_TOOL}) {return ReadError::Invalid.response(id)}
            let args=request.pointer("/params/arguments").cloned().unwrap_or(Value::Null);
            let outcome=if calendar {adapter.calendar(token,args).await} else {adapter.drive(token,args).await};
            match outcome { Ok(data)=>json!({"content":[{"type":"text","text":data.to_string()}],"structuredContent":data,"isError":false}), Err(error)=>return error.response(id) }
        }
        _ => return Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported MCP method"}})).into_response(),
    };
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connected_apps::mcp::{McpDiscoveryClient, ToolCall};
    use axum::{extract::OriginalUri, routing::get};
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Provider {
        requests: Arc<Mutex<Vec<(String, String)>>>,
        status: Arc<Mutex<Option<StatusCode>>>,
    }
    async fn google(
        State(provider): State<Provider>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
    ) -> Response {
        provider.requests.lock().unwrap().push((
            uri.to_string(),
            headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .into(),
        ));
        if let Some(status) = *provider.status.lock().unwrap() {
            return (status, "private-error-with-token-canary").into_response();
        }
        if uri.path().starts_with("/calendar/") {
            Json(json!({"items":[{"id":"event1","summary":"Review","start":{"dateTime":"2026-10-01T10:00:00Z"},"end":{"dateTime":"2026-10-01T11:00:00Z"},"description":"unneeded","access_token":"do-not-return"}],"timeZone":"UTC","nextPageToken":"calendar-page-2"})).into_response()
        } else {
            Json(json!({"files":[{"id":"file1","name":"Plan","mimeType":"application/pdf","modifiedTime":"2026-10-01T00:00:00Z","access_token":"do-not-return"}],"nextPageToken":"drive-page-2","incompleteSearch":false})).into_response()
        }
    }
    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (origin, task)
    }
    #[tokio::test]
    async fn real_rest_reads_use_fixed_gets_projection_and_explicit_pagination() {
        let provider = Provider::default();
        let (origin, provider_task) = serve(
            Router::new()
                .route("/calendar/v3/calendars/primary/events", get(google))
                .route("/drive/v3/files", get(google))
                .with_state(provider.clone()),
        )
        .await;
        let (adapter, adapter_task) = serve(
            GoogleReadAdapter::with_local_upstream_for_testing(&origin)
                .unwrap()
                .router(),
        )
        .await;
        let client = McpDiscoveryClient::default();
        for (path, name, arguments) in [
            (
                "/calendar/mcp",
                CALENDAR_TOOL,
                json!({"time_min":"2026-10-01T00:00:00Z","time_max":"2026-10-02T00:00:00Z","page_size":10,"page_token":"prior-page"}),
            ),
            (
                "/drive/mcp",
                DRIVE_TOOL,
                json!({"name_contains":"O'Reilly\\plan","page_size":10,"page_token":"prior-page"}),
            ),
        ] {
            let endpoint = format!("{adapter}{path}");
            let (info, tools) = client
                .discover(&endpoint, "fixture-google-token", TIMEOUT, true)
                .await
                .unwrap();
            assert_eq!(info["vox_protocol_version"], "2025-11-25");
            assert_eq!(tools.len(), 1);
            let result = client
                .call_tool(
                    &endpoint,
                    "fixture-google-token",
                    ToolCall {
                        name,
                        expected_protocol: Some("2025-11-25"),
                        reviewed_schema: &tools[0]["inputSchema"],
                        arguments,
                    },
                    TIMEOUT,
                    true,
                )
                .await
                .unwrap();
            assert_eq!(result["structuredContent"]["complete"], false);
            assert!(result["structuredContent"]["next_page_token"].is_string());
            assert!(!result.to_string().contains("access_token"));
            assert!(!result.to_string().contains("description"));
        }
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 6); // discover validation, dispatch validation, actual read per tool
        assert!(
            requests
                .iter()
                .all(|(_, auth)| auth == "Bearer fixture-google-token")
        );
        let calendar = url::Url::parse(&format!("http://fixture{}", requests[2].0)).unwrap();
        let params = calendar
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(params["pageToken"], "prior-page");
        assert_eq!(params["singleEvents"], "true");
        let drive = url::Url::parse(&format!("http://fixture{}", requests[5].0)).unwrap();
        let params = drive
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            params["q"],
            "trashed = false and name contains 'O\\'Reilly\\\\plan'"
        );
        assert_eq!(params["corpora"], "user");
        drop(requests);
        adapter_task.abort();
        provider_task.abort();
    }
    #[tokio::test]
    async fn revoked_scopes_and_rate_limits_fail_without_provider_error_content() {
        let provider = Provider::default();
        let (origin, task) = serve(
            Router::new()
                .route("/drive/v3/files", get(google))
                .with_state(provider.clone()),
        )
        .await;
        let adapter = GoogleReadAdapter::with_local_upstream_for_testing(&origin).unwrap();
        for (status, expected) in [
            (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED),
            (StatusCode::FORBIDDEN, StatusCode::FORBIDDEN),
            (StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS),
            (StatusCode::INTERNAL_SERVER_ERROR, StatusCode::BAD_GATEWAY),
        ] {
            *provider.status.lock().unwrap() = Some(status);
            let error = adapter
                .drive("fixture-token", json!({}))
                .await
                .err()
                .unwrap();
            let response = error.response(json!(1));
            assert_eq!(response.status(), expected);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            assert!(!String::from_utf8_lossy(&body).contains("canary"));
            assert!(adapter.verify("fixture-token", false).await.is_err());
        }
        task.abort();
    }
    #[tokio::test]
    async fn bounded_arguments_and_missing_auth_do_not_contact_google() {
        let adapter =
            GoogleReadAdapter::with_local_upstream_for_testing("http://127.0.0.1:1").unwrap();
        for args in [
            json!({"time_min":"tomorrow","time_max":"later"}),
            json!({"time_min":"2026-10-02T00:00:00Z","time_max":"2026-10-01T00:00:00Z"}),
            json!({"time_min":"2026-10-01T00:00:00Z","time_max":"2027-10-01T00:00:00Z"}),
            json!({"time_min":"2026-10-01T00:00:00Z","time_max":"2026-10-02T00:00:00Z","url":"http://evil"}),
        ] {
            assert!(matches!(
                adapter.calendar("token", args).await,
                Err(ReadError::Invalid)
            ));
        }
        for args in [
            json!({"page_size":101}),
            json!({"name_contains":""}),
            json!({"q":"arbitrary provider query"}),
            json!({"page_token":""}),
        ] {
            assert!(matches!(
                adapter.drive("token", args).await,
                Err(ReadError::Invalid)
            ));
        }
        let response = handle(
            adapter,
            HeaderMap::new(),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            false,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            GoogleReadAdapter::with_local_upstream_for_testing("https://evil.example").is_err()
        );
        assert!(
            GoogleReadAdapter::with_local_upstream_for_testing("http://127.0.0.1:1/other").is_err()
        );
    }
    #[test]
    fn candidate_packages_match_actual_served_schemas_and_are_read_only() {
        for (calendar, text) in [
            (
                true,
                include_str!("../examples/packages/google-calendar-reader/vox-package.json"),
            ),
            (
                false,
                include_str!("../examples/packages/google-drive-metadata-reader/vox-package.json"),
            ),
        ] {
            let package: Value = serde_json::from_str(text).unwrap();
            let manifest: crate::remote_extensions::InstallExtensionRequest =
                serde_json::from_value(package["manifest"].clone()).unwrap();
            crate::remote_extensions::RemoteExtensionService::validate_install_request(
                &manifest, false,
            )
            .unwrap();
            assert_eq!(manifest.capabilities.len(), 1);
            assert_eq!(
                package["manifest"]["capabilities"][0]["input_schema"],
                tool_definition(calendar)["inputSchema"]
            );
            assert_eq!(
                package["manifest"]["capabilities"][0]["external_key"],
                tool_definition(calendar)["name"]
            );
            assert!(!manifest.capabilities[0].consequential);
        }
    }
}
