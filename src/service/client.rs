use crate::{
    capability_grants::{CapabilityGrant, CreateGrantRequest},
    connections::Connection,
    identity::RequestContext,
    packages::{ConnectorPackage, PublishPackage},
    remote_extensions::RemoteExtension,
    service::{
        auth::{HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP, HmacSigner},
        routes::{
            ContextRequest, CreateGrantBody, PackageInstallBody, SetupCallbackBody, SetupStartBody,
            WithdrawPackageBody,
        },
    },
    setup::{SetupRequest, SetupResult},
    skills::SkillListing,
};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Serialize, de::DeserializeOwned};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Server returned error ({status}): {body}")]
    ServerError { status: u16, body: String },
    #[error("Failed to sign request: {0}")]
    SigningError(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct ConnectionsServiceClient {
    client: reqwest::Client,
    base_url: String,
    hmac_secret: String,
}

impl ConnectionsServiceClient {
    pub fn new(base_url: impl Into<String>, hmac_secret: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            hmac_secret: hmac_secret.into(),
        }
    }

    pub async fn health_live(&self) -> Result<bool, ClientError> {
        let url = format!("{}/health/live", self.base_url);
        let resp = self.client.get(&url).send().await?;
        Ok(resp.status().is_success())
    }

    pub async fn health_ready(&self) -> Result<bool, ClientError> {
        let url = format!("{}/health/ready", self.base_url);
        let resp = self.client.get(&url).send().await?;
        Ok(resp.status().is_success())
    }

    pub async fn signed_post<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp, ClientError> {
        let body_bytes = serde_json::to_vec(body)?;
        let url = format!("{}{}", self.base_url, path);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let nonce = Uuid::new_v4().to_string();

        let signature = HmacSigner::sign(
            self.hmac_secret.as_bytes(),
            "POST",
            path,
            &body_bytes,
            now,
            &nonce,
        )
        .map_err(ClientError::SigningError)?;

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            HEADER_SIGNATURE,
            HeaderValue::from_str(&signature)
                .map_err(|e| ClientError::SigningError(e.to_string()))?,
        );
        headers.insert(
            HEADER_TIMESTAMP,
            HeaderValue::from_str(&now.to_string())
                .map_err(|e| ClientError::SigningError(e.to_string()))?,
        );
        headers.insert(
            HEADER_NONCE,
            HeaderValue::from_str(&nonce).map_err(|e| ClientError::SigningError(e.to_string()))?,
        );

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(body_bytes)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::ServerError {
                status: status.as_u16(),
                body,
            });
        }

        if status == reqwest::StatusCode::NO_CONTENT {
            return serde_json::from_str("null").map_err(ClientError::Serialization);
        }

        let resp_json = resp.json::<Resp>().await?;
        Ok(resp_json)
    }

    // Connections
    pub async fn list_connections(
        &self,
        context: &RequestContext,
    ) -> Result<Vec<Connection>, ClientError> {
        self.signed_post(
            "/v1/connections/list",
            &ContextRequest { context: *context },
        )
        .await
    }

    pub async fn disconnect_connection(
        &self,
        context: &RequestContext,
        id: Uuid,
    ) -> Result<(), ClientError> {
        self.signed_post(
            &format!("/v1/connections/{id}/disconnect"),
            &ContextRequest { context: *context },
        )
        .await
    }

    // Capability Grants
    pub async fn create_grant(
        &self,
        context: &RequestContext,
        grant: CreateGrantRequest,
    ) -> Result<CapabilityGrant, ClientError> {
        self.signed_post(
            "/v1/capability-grants",
            &CreateGrantBody {
                context: *context,
                grant,
            },
        )
        .await
    }

    pub async fn revoke_grant(
        &self,
        context: &RequestContext,
        grant: CreateGrantRequest,
    ) -> Result<(), ClientError> {
        self.signed_post(
            "/v1/capability-grants/revoke",
            &CreateGrantBody {
                context: *context,
                grant,
            },
        )
        .await
    }

    pub async fn effective_grants(
        &self,
        context: &RequestContext,
        agent_key: &str,
    ) -> Result<Vec<CapabilityGrant>, ClientError> {
        self.signed_post(
            &format!("/v1/agents/{agent_key}/effective-capability-grants"),
            &ContextRequest { context: *context },
        )
        .await
    }

    // Packages
    pub async fn publish_package(
        &self,
        pkg: PublishPackage,
    ) -> Result<ConnectorPackage, ClientError> {
        self.signed_post("/v1/connector-packages/publish", &pkg)
            .await
    }

    pub async fn list_packages(
        &self,
        context: &RequestContext,
    ) -> Result<Vec<ConnectorPackage>, ClientError> {
        self.signed_post(
            "/v1/connector-packages/list",
            &ContextRequest { context: *context },
        )
        .await
    }

    pub async fn install_package(
        &self,
        context: &RequestContext,
        external_key: &str,
        version: i32,
        digest: &str,
    ) -> Result<RemoteExtension, ClientError> {
        self.signed_post(
            "/v1/connector-packages/install",
            &PackageInstallBody {
                context: *context,
                external_key: external_key.to_string(),
                version,
                digest: digest.to_string(),
            },
        )
        .await
    }

    pub async fn withdraw_package(
        &self,
        deployment_id: Uuid,
        external_key: &str,
        version: i32,
    ) -> Result<(), ClientError> {
        self.signed_post(
            "/v1/connector-packages/withdraw",
            &WithdrawPackageBody {
                deployment_id,
                external_key: external_key.to_string(),
                version,
            },
        )
        .await
    }

    // Setup
    pub async fn setup_start(
        &self,
        context: &RequestContext,
        req: SetupRequest,
    ) -> Result<SetupResult, ClientError> {
        self.signed_post(
            "/v1/connector-packages/setup",
            &SetupStartBody {
                context: *context,
                request: req,
            },
        )
        .await
    }

    pub async fn setup_callback(
        &self,
        context: &RequestContext,
        state: &str,
        code: &str,
        iss: Option<&str>,
    ) -> Result<SetupResult, ClientError> {
        self.signed_post(
            "/v1/connector-packages/setup/callback",
            &SetupCallbackBody {
                context: *context,
                state: state.to_string(),
                code: code.to_string(),
                iss: iss.map(|s| s.to_string()),
            },
        )
        .await
    }

    // Remote Extensions
    pub async fn list_extensions(
        &self,
        context: &RequestContext,
    ) -> Result<Vec<RemoteExtension>, ClientError> {
        self.signed_post(
            "/v1/remote-extensions/list",
            &ContextRequest { context: *context },
        )
        .await
    }

    // Skills
    pub async fn list_skills(
        &self,
        context: &RequestContext,
    ) -> Result<Vec<SkillListing>, ClientError> {
        self.signed_post("/v1/skills/list", &ContextRequest { context: *context })
            .await
    }
}
