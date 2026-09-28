/**
* Connected apps: provider-verified OAuth connections to remote MCP servers
* and the tools they expose to the user's agent.
*
* Flow: `begin` discovers the app's authorization server, registers Vox as a
* client (dynamically, or with a configured client), and returns the provider
* login URL. The provider redirects back to the host with a code, and
* `complete` verifies the single-use state, exchanges the code with PKCE,
* opens an MCP session with the new token and records the tools the server
* reports. Operator review and an agent-specific grant are separate steps;
* authorization alone never activates agent tool use.
*/
use crate::{
    identity::RequestScope,
    remote_extensions::{
        ExtensionCapability, RemoteExtension, RemoteExtensionError, RemoteExtensionService,
    },
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::Row;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use url::Url;
use uuid::Uuid;

pub mod crypto;
pub mod mcp;
pub mod oauth;

use crypto::{CredentialCipher, pkce_challenge, random_token, sha256_hex};
use mcp::McpDiscoveryClient;
use oauth::{ConfiguredClient, TokenRequest, TokenSet};

const SESSION_TTL_MINUTES: i64 = 10;
/// Connecting lists every tool once; give slow servers room.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum ConnectedAppError {
    #[error("connected app request is invalid")]
    Invalid,
    #[error("connected app not found")]
    NotFound,
    #[error("connected apps are not configured on this server")]
    NotConfigured,
    #[error("this app needs an OAuth client configured for Vox")]
    ClientNotConfigured,
    #[error("the app rejected the credentials")]
    Unauthorized,
    #[error("the sign-in link expired or was already used")]
    Expired,
    #[error("credential encryption failed")]
    Crypto,
    #[error("{0}")]
    Provider(String),
    #[error(transparent)]
    Extension(#[from] RemoteExtensionError),
    #[error("the app took too long to respond")]
    Timeout,
    #[error("the app no longer has that tool")]
    UnknownTool,
    #[error("the selected agent has no grant for this connection and tool")]
    GrantRequired,
    #[error("this tool changes external state and requires an approved execution")]
    WriteRequiresApproval,
    #[error("the app session expired")]
    SessionExpired,
    #[error("connected app storage unavailable")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone, Debug, Serialize)]
pub struct AuthorizationStart {
    pub authorization_url: String,
    pub expires_at: DateTime<Utc>,
}

/// Deployment-supplied OAuth and credential settings, independent of the host app.
#[derive(Clone, Default)]
pub struct ConnectedAppsOptions {
    pub credential_key: Option<String>,
    pub redirect_uris: Vec<String>,
    pub oauth_clients: Option<String>,
}

#[derive(Clone)]
pub struct ConnectedAppsService {
    db: PgPool,
    extensions: RemoteExtensionService,
    cipher: Option<CredentialCipher>,
    redirect_uris: Vec<String>,
    configured: HashMap<String, ConfiguredClient>,
    allow_local: bool,
    discovery: McpDiscoveryClient,
}

impl ConnectedAppsService {
    pub fn from_options(db: PgPool, options: ConnectedAppsOptions) -> Self {
        let cipher = options.credential_key.as_deref().and_then(|key| {
            CredentialCipher::from_hex_key(key)
                .inspect_err(|_| {
                    tracing::error!("connector credential key must be a 32-byte hex key")
                })
                .ok()
        });
        let configured = options.oauth_clients.as_deref().and_then(|raw| {
            serde_json::from_str::<HashMap<String, ConfiguredClient>>(raw)
                .inspect_err(|err| tracing::error!(%err, "connector OAuth clients configuration is invalid"))
                .ok()
        }).unwrap_or_default();
        Self {
            extensions: RemoteExtensionService::new(db.clone()),
            db,
            cipher,
            redirect_uris: options.redirect_uris,
            configured,
            allow_local: false,
            discovery: McpDiscoveryClient::default(),
        }
    }

    pub fn new(
        db: PgPool,
        cipher: CredentialCipher,
        redirect_uris: Vec<String>,
        configured: HashMap<String, ConfiguredClient>,
    ) -> Self {
        Self {
            extensions: RemoteExtensionService::new(db.clone()),
            db,
            cipher: Some(cipher),
            redirect_uris,
            configured,
            allow_local: false,
            discovery: McpDiscoveryClient::default(),
        }
    }

    /// Loopback mock servers in integration tests only.
    pub fn with_local_endpoints_for_testing(mut self) -> Self {
        self.allow_local = true;
        self.extensions = self.extensions.with_local_endpoints_for_testing();
        self
    }

    /// MCP endpoint hosts that have an OAuth client configured out of band.
    pub fn configured_hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self.configured.keys().cloned().collect();
        hosts.sort();
        hosts
    }

    fn cipher(&self) -> Result<&CredentialCipher, ConnectedAppError> {
        self.cipher.as_ref().ok_or(ConnectedAppError::NotConfigured)
    }

    fn configured_for(&self, endpoint: &str) -> Option<&ConfiguredClient> {
        let host = Url::parse(endpoint).ok()?.host_str()?.to_ascii_lowercase();
        self.configured.get(&host)
    }

    pub async fn begin(
        &self,
        context: &impl RequestScope,
        extension_id: Uuid,
        redirect_uri: &str,
    ) -> Result<AuthorizationStart, ConnectedAppError> {
        let cipher = self.cipher()?;
        if !self
            .redirect_uris
            .iter()
            .any(|allowed| allowed == redirect_uri)
        {
            return Err(ConnectedAppError::Invalid);
        }
        let row = sqlx::query(
            "SELECT endpoint_url, protocol, lifecycle_state, current_version FROM remote_extensions \
             WHERE id = $1 AND user_context_id = $2",
        )
        .bind(extension_id)
        .bind(context.request_context().id.0)
        .fetch_optional(&self.db)
        .await?
        .ok_or(ConnectedAppError::NotFound)?;
        let endpoint: String = row.get("endpoint_url");
        let protocol: String = row.get("protocol");
        let state: String = row.get("lifecycle_state");
        let version: i32 = row.get("current_version");
        if protocol != "mcp" || state == "removed" || state == "quarantined" {
            return Err(ConnectedAppError::Invalid);
        }

        let configured = self.configured_for(&endpoint);
        let discovery = oauth::discover(&endpoint, configured, self.allow_local).await?;
        let client_id = match configured {
            Some(client) => client.client_id.clone(),
            None => {
                self.dynamic_client(&discovery.metadata, redirect_uri)
                    .await?
                    .client_id
            }
        };

        let session_id = Uuid::new_v4();
        let verifier = random_token()?;
        let oauth_state = random_token()?;
        let expires_at = Utc::now() + ChronoDuration::minutes(SESSION_TTL_MINUTES);
        sqlx::query(
            "INSERT INTO mcp_authorization_sessions (
                id, extension_id, user_context_id, state_hash, code_verifier_ciphertext,
                issuer, token_endpoint, client_id, redirect_uri, resource,
                endpoint_url, extension_version, expires_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(session_id)
        .bind(extension_id)
        .bind(context.request_context().id.0)
        .bind(sha256_hex(&oauth_state))
        .bind(cipher.seal(session_id.as_bytes(), &verifier)?)
        .bind(&discovery.metadata.issuer)
        .bind(&discovery.metadata.token_endpoint)
        .bind(&client_id)
        .bind(redirect_uri)
        .bind(&discovery.resource)
        .bind(&endpoint)
        .bind(version)
        .bind(expires_at)
        .execute(&self.db)
        .await?;

        let mut url = Url::parse(&discovery.metadata.authorization_endpoint)
            .map_err(|_| ConnectedAppError::Provider("invalid authorization endpoint".into()))?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("state", &oauth_state)
                .append_pair("code_challenge", &pkce_challenge(&verifier))
                .append_pair("code_challenge_method", "S256");
            if !discovery.scopes.is_empty() {
                query.append_pair("scope", &discovery.scopes.join(" "));
            }
            if configured.is_none_or(|c| c.send_resource) {
                query.append_pair("resource", &discovery.resource);
            }
            if let Some(client) = configured {
                for (key, value) in &client.authorize_params {
                    query.append_pair(key, value);
                }
            }
        }
        Ok(AuthorizationStart {
            authorization_url: url.to_string(),
            expires_at,
        })
    }

    async fn dynamic_client(
        &self,
        metadata: &oauth::AuthServerMetadata,
        redirect_uri: &str,
    ) -> Result<oauth::RegisteredClient, ConnectedAppError> {
        if let Some(existing) = self.stored_client(&metadata.issuer, redirect_uri).await? {
            return Ok(existing);
        }
        let registered = oauth::register(metadata, redirect_uri, self.allow_local).await?;
        let id = Uuid::new_v4();
        let secret = match &registered.client_secret {
            Some(secret) => Some(self.cipher()?.seal(id.as_bytes(), secret)?),
            None => None,
        };
        // A concurrent registration may have won; keep the stored one.
        sqlx::query(
            "INSERT INTO mcp_oauth_clients (id, issuer, redirect_uri, client_id, \
             client_secret_ciphertext, token_endpoint_auth_method) VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (issuer, redirect_uri) DO NOTHING",
        )
        .bind(id)
        .bind(&metadata.issuer)
        .bind(redirect_uri)
        .bind(&registered.client_id)
        .bind(secret)
        .bind(&registered.auth_method)
        .execute(&self.db)
        .await?;
        self.stored_client(&metadata.issuer, redirect_uri)
            .await?
            .ok_or(ConnectedAppError::ClientNotConfigured)
    }

    async fn stored_client(
        &self,
        issuer: &str,
        redirect_uri: &str,
    ) -> Result<Option<oauth::RegisteredClient>, ConnectedAppError> {
        let Some(row) = sqlx::query(
            "SELECT id, client_id, client_secret_ciphertext, token_endpoint_auth_method \
             FROM mcp_oauth_clients WHERE issuer = $1 AND redirect_uri = $2",
        )
        .bind(issuer)
        .bind(redirect_uri)
        .fetch_optional(&self.db)
        .await?
        else {
            return Ok(None);
        };
        let id: Uuid = row.get("id");
        let secret: Option<Vec<u8>> = row.get("client_secret_ciphertext");
        let client_secret = match secret {
            Some(stored) => Some(self.cipher()?.open(id.as_bytes(), &stored)?),
            None => None,
        };
        Ok(Some(oauth::RegisteredClient {
            client_id: row.get("client_id"),
            client_secret,
            auth_method: row.get("token_endpoint_auth_method"),
        }))
    }

    /// Client credentials for token calls, from configuration or the stored
    /// dynamic registration.
    async fn client_credentials(
        &self,
        endpoint: &str,
        issuer: &str,
        client_id: &str,
        redirect_uri: Option<&str>,
    ) -> Result<(Option<String>, String, bool), ConnectedAppError> {
        if let Some(client) = self.configured_for(endpoint) {
            let metadata = oauth::AuthServerMetadata {
                issuer: issuer.to_string(),
                authorization_endpoint: String::new(),
                token_endpoint: String::new(),
                registration_endpoint: None,
                token_endpoint_auth_methods: Vec::new(),
            };
            return Ok((
                client.client_secret.clone(),
                oauth::configured_auth_method(client, &metadata),
                client.send_resource,
            ));
        }
        let row = match redirect_uri {
            Some(redirect) => self.stored_client(issuer, redirect).await?,
            None => {
                let id: Option<(String,)> = sqlx::query_as(
                    "SELECT redirect_uri FROM mcp_oauth_clients WHERE issuer = $1 AND client_id = $2",
                )
                .bind(issuer)
                .bind(client_id)
                .fetch_optional(&self.db)
                .await?;
                match id {
                    Some((redirect,)) => self.stored_client(issuer, &redirect).await?,
                    None => None,
                }
            }
        };
        let client = row.ok_or(ConnectedAppError::ClientNotConfigured)?;
        Ok((client.client_secret, client.auth_method, true))
    }

    pub async fn complete(
        &self,
        context: &impl RequestScope,
        state: &str,
        code: &str,
    ) -> Result<RemoteExtension, ConnectedAppError> {
        self.complete_with_issuer(context, state, code, None).await
    }

    pub async fn complete_with_issuer(
        &self,
        context: &impl RequestScope,
        state: &str,
        code: &str,
        issuer_response: Option<&str>,
    ) -> Result<RemoteExtension, ConnectedAppError> {
        let cipher = self.cipher()?;
        if state.is_empty() || code.is_empty() {
            return Err(ConnectedAppError::Invalid);
        }
        let mut tx = self.db.begin().await?;
        let session = sqlx::query(
            "SELECT s.id, s.extension_id, s.code_verifier_ciphertext, s.issuer, s.token_endpoint, \
                    s.client_id, s.redirect_uri, s.resource, s.expires_at, s.consumed_at, \
                    s.endpoint_url, s.extension_version, e.endpoint_url AS current_endpoint, \
                    e.current_version, e.lifecycle_state \
             FROM mcp_authorization_sessions s JOIN remote_extensions e ON e.id = s.extension_id \
             WHERE s.state_hash = $1 AND s.user_context_id = $2 FOR UPDATE OF s",
        )
        .bind(sha256_hex(state))
        .bind(context.request_context().id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ConnectedAppError::Expired)?;
        let consumed: Option<DateTime<Utc>> = session.get("consumed_at");
        let expires_at: DateTime<Utc> = session.get("expires_at");
        if consumed.is_some() || expires_at < Utc::now() {
            return Err(ConnectedAppError::Expired);
        }
        let endpoint: String = session.get("endpoint_url");
        let version: i32 = session.get("extension_version");
        if endpoint != session.get::<String, _>("current_endpoint")
            || version != session.get::<i32, _>("current_version")
            || matches!(
                session.get::<String, _>("lifecycle_state").as_str(),
                "removed" | "quarantined"
            )
        {
            return Err(ConnectedAppError::Expired);
        }
        let session_id: Uuid = session.get("id");
        sqlx::query("UPDATE mcp_authorization_sessions SET consumed_at = now() WHERE id = $1")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;

        let extension_id: Uuid = session.get("extension_id");
        let issuer: String = session.get("issuer");
        if issuer_response.is_some_and(|actual| actual != issuer) {
            return Err(ConnectedAppError::Unauthorized);
        }
        let token_endpoint: String = session.get("token_endpoint");
        let client_id: String = session.get("client_id");
        let redirect_uri: String = session.get("redirect_uri");
        let resource: String = session.get("resource");
        let verifier = cipher.open(
            session_id.as_bytes(),
            &session.get::<Vec<u8>, _>("code_verifier_ciphertext"),
        )?;

        let (secret, auth_method, send_resource) = self
            .client_credentials(&endpoint, &issuer, &client_id, Some(&redirect_uri))
            .await?;
        let request = TokenRequest {
            token_endpoint: &token_endpoint,
            client_id: &client_id,
            client_secret: secret.as_deref(),
            auth_method: &auth_method,
            resource: send_resource.then_some(resource.as_str()),
        };
        let tokens =
            oauth::exchange_code(&request, code, &verifier, &redirect_uri, self.allow_local)
                .await?;

        // The connection only counts once the app itself accepts the token.
        let probe = self
            .discovery
            .discover(
                &endpoint,
                &tokens.access_token,
                CONNECT_TIMEOUT,
                self.allow_local,
            )
            .await;
        let (server_info, tools) = probe?;

        self.store_tokens(
            session_id,
            extension_id,
            &endpoint,
            version,
            &issuer,
            &token_endpoint,
            &client_id,
            &resource,
            &tokens,
            &server_info,
            &tools,
        )
        .await?;
        // OAuth and tools/list prove connectivity, not the declared effects or
        // safety of the tools. Only the operator conformance path may enable
        // this extension for execution.
        Ok(self.extensions.get(context, extension_id).await?)
    }

    #[allow(clippy::too_many_arguments)]
    async fn store_tokens(
        &self,
        session_id: Uuid,
        extension_id: Uuid,
        expected_endpoint: &str,
        expected_version: i32,
        issuer: &str,
        token_endpoint: &str,
        client_id: &str,
        resource: &str,
        tokens: &TokenSet,
        server_info: &Value,
        tools: &[Value],
    ) -> Result<(), ConnectedAppError> {
        let cipher = self.cipher()?;
        let access = cipher.seal(&aad(extension_id, "access"), &tokens.access_token)?;
        let refresh = match &tokens.refresh_token {
            Some(token) => Some(cipher.seal(&aad(extension_id, "refresh"), token)?),
            None => None,
        };
        let expires_at = tokens
            .expires_in
            .map(|seconds| Utc::now() + ChronoDuration::seconds(seconds));
        let mut tx = self.db.begin().await?;
        let current = sqlx::query(
            "SELECT endpoint_url,current_version,lifecycle_state,user_context_id, \
                    (SELECT capabilities FROM remote_extension_versions \
                     WHERE extension_id=$1 AND version=remote_extensions.current_version) AS capabilities \
             FROM remote_extensions \
             WHERE id=$1 FOR UPDATE",
        )
        .bind(extension_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ConnectedAppError::Expired)?;
        let current_endpoint: String = current.get("endpoint_url");
        let current_version: i32 = current.get("current_version");
        let state: String = current.get("lifecycle_state");
        if current_endpoint != expected_endpoint
            || expected_version != current_version
            || matches!(state.as_str(), "removed" | "quarantined")
        {
            return Err(ConnectedAppError::Expired);
        }
        let session_still_valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM mcp_authorization_sessions \
             WHERE id=$1 AND extension_id=$2 AND consumed_at IS NOT NULL)",
        )
        .bind(session_id)
        .bind(extension_id)
        .fetch_one(&mut *tx)
        .await?;
        if !session_still_valid {
            return Err(ConnectedAppError::Expired);
        }
        sqlx::query(
            "INSERT INTO remote_extension_credentials (
                        extension_id, issuer, token_endpoint, client_id, resource,
                        access_token_ciphertext, refresh_token_ciphertext, scope, expires_at,
                        server_info, tools
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                    ON CONFLICT (extension_id) DO UPDATE SET
                        issuer = EXCLUDED.issuer, token_endpoint = EXCLUDED.token_endpoint,
                        client_id = EXCLUDED.client_id, resource = EXCLUDED.resource,
                        access_token_ciphertext = EXCLUDED.access_token_ciphertext,
                        refresh_token_ciphertext = EXCLUDED.refresh_token_ciphertext,
                        scope = EXCLUDED.scope, expires_at = EXCLUDED.expires_at,
                        server_info = EXCLUDED.server_info, tools = EXCLUDED.tools,
                        tools_refreshed_at = now(), connected_at = now(), updated_at = now()",
        )
        .bind(extension_id)
        .bind(issuer)
        .bind(token_endpoint)
        .bind(client_id)
        .bind(resource)
        .bind(access)
        .bind(refresh)
        .bind(&tokens.scope)
        .bind(expires_at)
        .bind(server_info)
        .bind(serde_json::to_value(tools).unwrap_or(Value::Array(vec![])))
        .execute(&mut *tx)
        .await?;

        // A provider's tools/list is observed inventory, never a source of
        // undeclared authority. Only the intersection can be service-authorized;
        // operator state and per-agent grants are checked separately at use.
        let declared: Vec<ExtensionCapability> =
            serde_json::from_value(current.get("capabilities"))
                .map_err(|_| ConnectedAppError::Invalid)?;
        let reported: HashSet<&str> = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        let authorized: Vec<String> = declared
            .into_iter()
            .filter(|capability| reported.contains(capability.external_key.as_str()))
            .map(|capability| capability.external_key)
            .collect();
        let mut account_hasher = Sha256::new();
        account_hasher.update(extension_id.as_bytes());
        account_hasher.update(resource.as_bytes());
        let account_hash = account_hasher.finalize().to_vec();
        sqlx::query(
            "INSERT INTO external_connections \
             (user_context_id, remote_extension_id, external_account_hash, credential_custody, \
              authorization_state, authorized_capabilities) \
             VALUES ($1,$2,$3,'platform_held','authorized',$4) \
             ON CONFLICT (user_context_id, remote_extension_id) \
             WHERE remote_extension_id IS NOT NULL DO UPDATE SET \
                 external_account_hash=EXCLUDED.external_account_hash, \
                 authorization_state='authorized', authorized_capabilities=EXCLUDED.authorized_capabilities, \
                 expires_at=NULL, revoked_at=NULL, failure_code=NULL, updated_at=now()",
        )
        .bind(current.get::<Uuid, _>("user_context_id"))
        .bind(extension_id)
        .bind(account_hash)
        .bind(authorized)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The user's authorized accounts with the tools each app reported.
    /// Reported tools are display data, not execution authority.
    pub async fn connections(
        &self,
        context: &impl RequestScope,
    ) -> Result<Vec<Value>, ConnectedAppError> {
        let rows = sqlx::query(
            "SELECT c.extension_id, x.id AS connection_id, c.tools, c.connected_at, c.tools_refreshed_at, \
                    e.lifecycle_state \
             FROM remote_extension_credentials c JOIN remote_extensions e ON e.id = c.extension_id \
             JOIN external_connections x ON x.remote_extension_id=e.id AND x.user_context_id=e.user_context_id \
             WHERE e.user_context_id = $1 AND e.lifecycle_state <> 'removed' \
             AND x.authorization_state='authorized'",
        )
        .bind(context.request_context().id.0)
        .fetch_all(&self.db)
        .await?;
        Ok(rows
            .iter()
            .map(|row| {
                let tools: Value = row.get("tools");
                let summary: Vec<Value> = tools
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .map(|tool| {
                                json!({
                                    "name": tool.get("name"),
                                    "title": tool.get("title").or_else(|| tool.pointer("/annotations/title")),
                                    "description": tool.get("description"),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                json!({
                    "extension_id": row.get::<Uuid, _>("extension_id"),
                    "connection_id": row.get::<Uuid, _>("connection_id"),
                    "lifecycle_state": row.get::<String, _>("lifecycle_state"),
                    "connected_at": row.get::<DateTime<Utc>, _>("connected_at"),
                    "tools_refreshed_at": row.get::<DateTime<Utc>, _>("tools_refreshed_at"),
                    "tools": summary,
                })
            })
            .collect())
    }

    /// A host-facing read seam. The server, not the model, resolves the
    /// connection, selected agent, declaration, grant and OAuth credential.
    pub async fn read_tool(
        &self,
        context: &impl RequestScope,
        agent_external_key: &str,
        connection_id: Uuid,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, ConnectedAppError> {
        self.invoke_tool(
            context,
            agent_external_key,
            connection_id,
            tool_name,
            arguments,
            false,
        )
        .await
    }

    /// Only call after Core has consumed an exact approval and claimed its
    /// durable execution. This module still checks the current tool authority.
    pub async fn approved_tool(
        &self,
        context: &impl RequestScope,
        agent_external_key: &str,
        connection_id: Uuid,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, ConnectedAppError> {
        self.invoke_tool(
            context,
            agent_external_key,
            connection_id,
            tool_name,
            arguments,
            true,
        )
        .await
    }

    async fn invoke_tool(
        &self,
        context: &impl RequestScope,
        agent_external_key: &str,
        connection_id: Uuid,
        tool_name: &str,
        arguments: Value,
        approved_write: bool,
    ) -> Result<Value, ConnectedAppError> {
        if agent_external_key.trim().is_empty()
            || tool_name.trim().is_empty()
            || !arguments.is_object()
            || serde_json::to_vec(&arguments).map_or(true, |data| data.len() > 2 * 1024 * 1024)
        {
            return Err(ConnectedAppError::Invalid);
        }
        let mut tx = self.db.begin().await?;
        let row = sqlx::query(
            "SELECT e.id AS extension_id,e.endpoint_url,e.conformance_status,e.operator_enabled,v.capabilities, \
                    c.access_token_ciphertext,c.tools \
             FROM external_connections x \
             JOIN remote_extensions e ON e.id=x.remote_extension_id AND e.user_context_id=x.user_context_id \
             JOIN remote_extension_versions v ON v.extension_id=e.id AND v.version=e.current_version \
             JOIN remote_extension_credentials c ON c.extension_id=e.id \
             WHERE x.id=$1 AND x.user_context_id=$2 AND x.authorization_state='authorized' \
               AND (x.expires_at IS NULL OR x.expires_at>now()) \
               AND $3=ANY(x.authorized_capabilities) \
               AND e.lifecycle_state='active' AND e.consent_status='consented' \
               AND NOT EXISTS (SELECT 1 FROM connector_package_installations pi \
                   JOIN connector_packages p ON p.deployment_id=pi.deployment_id \
                     AND p.external_key=pi.external_key AND p.version=pi.version \
                   WHERE pi.extension_id=e.id AND (NOT p.enabled \
                     OR p.manifest->>'endpoint_url'<>e.endpoint_url \
                     OR p.manifest->'capabilities'<>v.capabilities)) \
               AND EXISTS (SELECT 1 FROM agent_capability_grants g \
                   JOIN agent_definitions a ON a.id=g.agent_definition_id \
                   JOIN deployment_agent_selections s ON s.agent_definition_id=a.id \
                   WHERE g.connection_id=x.id AND g.user_context_id=x.user_context_id \
                     AND g.capability_external_key=$3 AND g.state='enabled' \
                     AND a.external_key=$4 AND a.deployment_id=$5 AND a.state='enabled' \
                     AND ($3=ANY(a.requested_capability_categories) \
                          OR '*'=ANY(a.requested_capability_categories))) \
             FOR SHARE OF x,e",
        )
        .bind(connection_id)
        .bind(context.request_context().id.0)
        .bind(tool_name)
        .bind(agent_external_key)
        .bind(context.request_context().subject.deployment_id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ConnectedAppError::GrantRequired)?;
        let declared: Vec<ExtensionCapability> = serde_json::from_value(row.get("capabilities"))
            .map_err(|_| ConnectedAppError::Invalid)?;
        let cap = declared
            .iter()
            .find(|cap| cap.external_key == tool_name)
            .ok_or(ConnectedAppError::UnknownTool)?;
        let consequential = cap.consequential || cap.effect.is_consequential();
        if consequential && !approved_write {
            return Err(ConnectedAppError::WriteRequiresApproval);
        }
        if approved_write
            && (!consequential
                || row.get::<String, _>("conformance_status") != "passed"
                || !row.get::<bool, _>("operator_enabled"))
        {
            return Err(ConnectedAppError::WriteRequiresApproval);
        }
        let reported: Value = row.get("tools");
        if !reported.as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool.get("name").and_then(Value::as_str) == Some(tool_name))
        }) {
            return Err(ConnectedAppError::UnknownTool);
        }
        let extension_id: Uuid = row.get("extension_id");
        let endpoint: String = row.get("endpoint_url");
        let token = self.cipher()?.open(
            &aad(extension_id, "access"),
            &row.get::<Vec<u8>, _>("access_token_ciphertext"),
        )?;
        let result = self
            .discovery
            .call_tool(
                &endpoint,
                &token,
                tool_name,
                arguments,
                CONNECT_TIMEOUT,
                self.allow_local,
            )
            .await?;
        if result.get("resultType").and_then(Value::as_str) == Some("input_required") {
            return Err(ConnectedAppError::Provider(
                "tool requires an interactive continuation that Vox cannot complete".into(),
            ));
        }
        tx.commit().await?;
        Ok(result)
    }
}

fn aad(extension_id: Uuid, purpose: &str) -> Vec<u8> {
    let mut data = extension_id.as_bytes().to_vec();
    data.extend_from_slice(purpose.as_bytes());
    data
}
