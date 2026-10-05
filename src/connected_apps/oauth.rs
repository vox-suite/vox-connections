use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use url::Url;

use super::ConnectedAppError;
use crate::remote_extensions::adapters::transport::client_for_endpoint;

const TIMEOUT: Duration = Duration::from_secs(15);

/// An OAuth client registered out of band for an MCP server that does not
/// allow dynamic registration (e.g. Spotify, Google). Keyed by the MCP
/// endpoint host in `VOX_MCP_OAUTH_CLIENTS`.
#[derive(Clone, Debug, Deserialize)]
pub struct ConfiguredClient {
    /// Authorization server issuer. Needed when the MCP server does not
    /// publish protected resource metadata.
    #[serde(default)]
    pub issuer: Option<String>,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Authentication for both code exchange and refresh. Defaults to Basic
    /// for confidential clients; providers such as GitHub require Post.
    #[serde(default)]
    pub token_endpoint_auth_method: Option<ConfiguredTokenAuth>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Send the RFC 8707 `resource` parameter. Some providers reject it.
    #[serde(default = "default_true")]
    pub send_resource: bool,
    /// Extra authorization request parameters, e.g. Google's
    /// `access_type=offline` so a refresh token is issued.
    #[serde(default)]
    pub authorize_params: HashMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub enum ConfiguredTokenAuth {
    #[serde(rename = "client_secret_basic")]
    Basic,
    #[serde(rename = "client_secret_post")]
    Post,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug)]
pub struct AuthServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub token_endpoint_auth_methods: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Discovery {
    pub resource: String,
    pub scopes: Vec<String>,
    pub metadata: AuthServerMetadata,
}

#[derive(Clone, Debug)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth_method: String,
}

#[derive(Clone, Debug)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
    pub scope: Option<String>,
}

async fn http_for(url: &str, allow_local: bool) -> Result<reqwest::Client, ConnectedAppError> {
    client_for_endpoint(url, TIMEOUT, allow_local)
        .await
        .map_err(|e| ConnectedAppError::Provider(e.to_string()))
}

async fn get_json(url: &str, allow_local: bool) -> Option<Value> {
    let response = http_for(url, allow_local)
        .await
        .ok()?
        .get(url)
        .header("accept", "application/json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().await.ok()
}

fn origin(url: &Url) -> String {
    let mut origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        origin.push_str(&format!(":{port}"));
    }
    origin
}

/// Parse `resource_metadata` and `scope` from a Bearer challenge.
fn parse_challenge(header: &str) -> (Option<String>, Option<String>) {
    let param = |name: &str| {
        let needle = format!("{name}=\"");
        header.find(&needle).and_then(|start| {
            let rest = &header[start + needle.len()..];
            rest.find('"').map(|end| rest[..end].to_string())
        })
    };
    (param("resource_metadata"), param("scope"))
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Discover where and how to authorize for an MCP server (MCP authorization
/// spec: RFC 9728 protected resource metadata, then RFC 8414 / OIDC metadata).
pub async fn discover(
    endpoint: &str,
    configured: Option<&ConfiguredClient>,
    allow_local: bool,
) -> Result<Discovery, ConnectedAppError> {
    let url = Url::parse(endpoint).map_err(|_| ConnectedAppError::Invalid)?;
    let mut resource = endpoint.to_string();
    let mut scopes: Vec<String> = configured.map(|c| c.scopes.clone()).unwrap_or_default();
    let mut issuer = configured.and_then(|c| c.issuer.clone());

    if issuer.is_none() {
        let (metadata_url, challenge_scope) = probe_challenge(endpoint, allow_local).await;
        let mut candidates = Vec::new();
        if let Some(u) = metadata_url {
            candidates.push(u);
        }
        let path = url.path().trim_end_matches('/');
        if !path.is_empty() {
            candidates.push(format!(
                "{}/.well-known/oauth-protected-resource{path}",
                origin(&url)
            ));
        }
        candidates.push(format!(
            "{}/.well-known/oauth-protected-resource",
            origin(&url)
        ));
        for candidate in candidates {
            if let Some(prm) = get_json(&candidate, allow_local).await
                && let Some(server) = prm
                    .get("authorization_servers")
                    .and_then(Value::as_array)
                    .and_then(|s| s.first())
                    .and_then(Value::as_str)
            {
                issuer = Some(server.to_string());
                if let Some(r) = prm.get("resource").and_then(Value::as_str) {
                    resource = r.to_string();
                }
                break;
            }
        }
        if scopes.is_empty()
            && let Some(scope) = challenge_scope
        {
            scopes = scope.split_whitespace().map(str::to_string).collect();
        }
    }
    // Servers from before protected resource metadata act as their own
    // authorization server.
    let issuer = issuer.unwrap_or_else(|| origin(&url));
    let metadata = authorization_server_metadata(&issuer, allow_local).await?;
    Ok(Discovery {
        resource,
        scopes,
        metadata,
    })
}

async fn probe_challenge(endpoint: &str, allow_local: bool) -> (Option<String>, Option<String>) {
    let Ok(http) = http_for(endpoint, allow_local).await else {
        return (None, None);
    };
    let Ok(response) = http
        .post(endpoint)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "Vox", "version": env!("CARGO_PKG_VERSION")}}}),
        )
        .send()
        .await
    else {
        return (None, None);
    };
    response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .map(parse_challenge)
        .unwrap_or((None, None))
}

async fn authorization_server_metadata(
    issuer: &str,
    allow_local: bool,
) -> Result<AuthServerMetadata, ConnectedAppError> {
    let url = Url::parse(issuer).map_err(|_| ConnectedAppError::Invalid)?;
    let base = origin(&url);
    let path = url.path().trim_end_matches('/');
    let mut candidates = Vec::new();
    if path.is_empty() {
        candidates.push(format!("{base}/.well-known/oauth-authorization-server"));
        candidates.push(format!("{base}/.well-known/openid-configuration"));
    } else {
        candidates.push(format!(
            "{base}/.well-known/oauth-authorization-server{path}"
        ));
        candidates.push(format!("{base}/.well-known/openid-configuration{path}"));
        candidates.push(format!("{base}{path}/.well-known/openid-configuration"));
    }
    for candidate in candidates {
        let Some(doc) = get_json(&candidate, allow_local).await else {
            continue;
        };
        let text = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_string);
        if let (Some(authorization_endpoint), Some(token_endpoint)) =
            (text("authorization_endpoint"), text("token_endpoint"))
        {
            return Ok(AuthServerMetadata {
                issuer: text("issuer").unwrap_or_else(|| issuer.to_string()),
                authorization_endpoint,
                token_endpoint,
                registration_endpoint: text("registration_endpoint"),
                token_endpoint_auth_methods: strings(
                    doc.get("token_endpoint_auth_methods_supported"),
                ),
            });
        }
    }
    Err(ConnectedAppError::Provider(
        "the app does not publish OAuth metadata".into(),
    ))
}

/// RFC 7591 dynamic client registration. Vox registers as a public client
/// using PKCE, falling back to a confidential client when required.
pub async fn register(
    metadata: &AuthServerMetadata,
    redirect_uri: &str,
    allow_local: bool,
) -> Result<RegisteredClient, ConnectedAppError> {
    let endpoint = metadata
        .registration_endpoint
        .as_deref()
        .ok_or(ConnectedAppError::ClientNotConfigured)?;
    let http = http_for(endpoint, allow_local).await?;
    let mut last_error = String::new();
    for method in ["none", "client_secret_post"] {
        let response = http
            .post(endpoint)
            .json(&json!({
                "client_name": "Vox",
                "client_uri": "https://app.voxagent.in",
                "redirect_uris": [redirect_uri],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": method,
            }))
            .send()
            .await
            .map_err(|_| ConnectedAppError::Provider("client registration failed".into()))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success()
            && let Some(client_id) = body.get("client_id").and_then(Value::as_str)
        {
            let client_secret = body
                .get("client_secret")
                .and_then(Value::as_str)
                .map(str::to_string);
            let auth_method = body
                .get("token_endpoint_auth_method")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if client_secret.is_some() {
                        "client_secret_post".into()
                    } else {
                        "none".into()
                    }
                });
            return Ok(RegisteredClient {
                client_id: client_id.to_string(),
                client_secret,
                auth_method,
            });
        }
        last_error = format!("HTTP {}", status.as_u16());
    }
    Err(ConnectedAppError::Provider(format!(
        "client registration was refused ({last_error})"
    )))
}

pub fn configured_auth_method(client: &ConfiguredClient) -> Result<String, ConnectedAppError> {
    match (&client.client_secret, client.token_endpoint_auth_method) {
        (None, None) => Ok("none".into()),
        (None, Some(_)) => Err(ConnectedAppError::ClientNotConfigured),
        (Some(_), Some(ConfiguredTokenAuth::Post)) => Ok("client_secret_post".into()),
        (Some(_), _) => Ok("client_secret_basic".into()),
    }
}

pub struct TokenRequest<'a> {
    pub token_endpoint: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub auth_method: &'a str,
    pub resource: Option<&'a str>,
}

async fn token_call(
    request: &TokenRequest<'_>,
    mut form: Vec<(&str, String)>,
    allow_local: bool,
) -> Result<TokenSet, ConnectedAppError> {
    let http = http_for(request.token_endpoint, allow_local).await?;
    if let Some(resource) = request.resource {
        form.push(("resource", resource.to_string()));
    }
    let mut builder = http
        .post(request.token_endpoint)
        .header("accept", "application/json");
    match (request.auth_method, request.client_secret) {
        ("client_secret_basic", Some(secret)) => {
            builder = builder.basic_auth(request.client_id, Some(secret));
        }
        ("client_secret_post", Some(secret)) => {
            form.push(("client_id", request.client_id.to_string()));
            form.push(("client_secret", secret.to_string()));
        }
        _ => form.push(("client_id", request.client_id.to_string())),
    }
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form.iter().map(|(k, v)| (*k, v.as_str())))
        .finish();
    let response = builder
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|_| ConnectedAppError::Provider("token request failed".into()))?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let reason = body
            .get("error_description")
            .or_else(|| body.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("rejected");
        return Err(if status.as_u16() == 400 || status.as_u16() == 401 {
            ConnectedAppError::Unauthorized
        } else {
            ConnectedAppError::Provider(format!("token endpoint: {reason}"))
        });
    }
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| ConnectedAppError::Provider("token response has no access token".into()))?;
    Ok(TokenSet {
        access_token: access_token.to_string(),
        refresh_token: body
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        expires_in: body.get("expires_in").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        }),
        scope: body
            .get("scope")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub async fn exchange_code(
    request: &TokenRequest<'_>,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    allow_local: bool,
) -> Result<TokenSet, ConnectedAppError> {
    token_call(
        request,
        vec![
            ("grant_type", "authorization_code".into()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
            ("code_verifier", code_verifier.to_string()),
        ],
        allow_local,
    )
    .await
}

pub async fn refresh(
    request: &TokenRequest<'_>,
    refresh_token: &str,
    allow_local: bool,
) -> Result<TokenSet, ConnectedAppError> {
    token_call(
        request,
        vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh_token.to_string()),
        ],
        allow_local,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_authentication_is_validated() {
        let parse = |value| serde_json::from_value::<ConfiguredClient>(value).unwrap();
        let public = parse(json!({"client_id":"public"}));
        assert_eq!(configured_auth_method(&public).unwrap(), "none");
        let basic = parse(json!({"client_id":"private","client_secret":"test-secret"}));
        assert_eq!(
            configured_auth_method(&basic).unwrap(),
            "client_secret_basic"
        );
        let missing =
            parse(json!({"client_id":"private","token_endpoint_auth_method":"client_secret_post"}));
        assert!(configured_auth_method(&missing).is_err());
        assert!(
            serde_json::from_value::<ConfiguredClient>(
                json!({"client_id":"private","token_endpoint_auth_method":"typo"})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn configured_post_authenticates_code_exchange_and_refresh() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for grant in ["authorization_code", "refresh_token"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let (header_end, body_len) = loop {
                    let mut chunk = [0; 4096];
                    let size = stream.read(&mut chunk).unwrap();
                    assert!(size > 0);
                    request.extend_from_slice(&chunk[..size]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let len: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + len {
                            assert!(!headers.contains("authorization:"));
                            break (end + 4, len);
                        }
                    }
                };
                let form: HashMap<_, _> =
                    url::form_urlencoded::parse(&request[header_end..header_end + body_len])
                        .into_owned()
                        .collect();
                assert_eq!(form["client_id"], "test-client");
                assert_eq!(form["client_secret"], "test-secret");
                assert_eq!(form["grant_type"], grant);
                let body = r#"{"access_token":"test-access","expires_in":3600}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let client: ConfiguredClient = serde_json::from_value(json!({"client_id":"test-client","client_secret":"test-secret","token_endpoint_auth_method":"client_secret_post"})).unwrap();
        let method = configured_auth_method(&client).unwrap();
        let request = TokenRequest {
            token_endpoint: &endpoint,
            client_id: &client.client_id,
            client_secret: client.client_secret.as_deref(),
            auth_method: &method,
            resource: None,
        };
        assert_eq!(
            exchange_code(
                &request,
                "test-code",
                "test-verifier",
                "https://host.example/callback",
                true
            )
            .await
            .unwrap()
            .access_token,
            "test-access"
        );
        assert_eq!(
            refresh(&request, "test-refresh", true)
                .await
                .unwrap()
                .access_token,
            "test-access"
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn advertised_scopes_are_not_requested_as_user_authority() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let server_base = base.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let count = stream.read(&mut request).unwrap();
                let line = String::from_utf8_lossy(&request[..count]);
                let (status, headers, body) = if line.starts_with("POST /mcp ") {
                    (
                        "401 Unauthorized",
                        format!(
                            "WWW-Authenticate: Bearer resource_metadata=\"{server_base}/.well-known/oauth-protected-resource/mcp\"\r\n"
                        ),
                        String::new(),
                    )
                } else if line.starts_with("GET /.well-known/oauth-protected-resource/mcp ") {
                    (
                        "200 OK",
                        String::new(),
                        json!({"resource":format!("{server_base}/mcp"),"authorization_servers":[format!("{server_base}/oauth")],"scopes_supported":["read","write","admin"]}).to_string(),
                    )
                } else if line.starts_with("GET /.well-known/oauth-authorization-server/oauth ") {
                    (
                        "200 OK",
                        String::new(),
                        json!({"issuer":format!("{server_base}/oauth"),"authorization_endpoint":format!("{server_base}/authorize"),"token_endpoint":format!("{server_base}/token")}).to_string(),
                    )
                } else {
                    ("404 Not Found", String::new(), String::new())
                };
                write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len()).unwrap();
                if line.starts_with("GET /.well-known/oauth-authorization-server/oauth ") {
                    break;
                }
            }
        });
        let discovery = discover(&format!("{base}/mcp"), None, true).await.unwrap();
        assert!(discovery.scopes.is_empty());
        server.join().unwrap();
    }
}
