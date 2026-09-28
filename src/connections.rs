use crate::identity::{RequestContext, UserContextId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialCustody {
    PlatformHeld,
    ExternalOperator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationState {
    Pending,
    Authorized,
    Expired,
    Revoked,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Connection {
    pub id: Uuid,
    pub user_context_id: UserContextId,
    pub integration_external_key: String,
    pub account_display_id: Option<String>,
    pub credential_custody: CredentialCustody,
    pub authorization_state: AuthorizationState,
    pub authorized_capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub failure_code: Option<String>,
}

#[derive(Clone)]
pub struct ConnectionService {
    db: sqlx::PgPool,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("connection request invalid")]
    Invalid,
    #[error("integration unavailable")]
    IntegrationUnavailable,
    #[error("connection unavailable")]
    NotFound,
    #[error("connection storage unavailable")]
    Database(#[from] sqlx::Error),
}

impl ConnectionService {
    pub fn new(db: sqlx::PgPool) -> Self {
        Self { db }
    }

    /// Returns only connections owned by this authenticated host user context.
    pub async fn list(&self, context: &RequestContext) -> Result<Vec<Connection>, ConnectionError> {
        let rows = sqlx::query(
            "SELECT c.id, COALESCE(i.external_key,e.external_key) AS external_key, c.account_display_id, c.credential_custody, c.authorization_state, \
             c.authorized_capabilities, c.expires_at, c.failure_code \
             FROM external_connections c LEFT JOIN integration_definitions i ON i.id=c.integration_id \
             LEFT JOIN remote_extensions e ON e.id=c.remote_extension_id \
             WHERE c.user_context_id=$1 ORDER BY c.created_at DESC, c.id DESC LIMIT 100",
        )
        .bind(context.id.0)
        .fetch_all(&self.db)
        .await?;
        rows.iter()
            .map(|row| connection_from_row(context.id, row))
            .collect()
    }

    /// Returns a single connection owned by this authenticated host user context.
    pub async fn get(
        &self,
        context: &RequestContext,
        connection_id: Uuid,
    ) -> Result<Connection, ConnectionError> {
        let row = sqlx::query(
            "SELECT c.id, COALESCE(i.external_key,e.external_key) AS external_key, c.account_display_id, c.credential_custody, c.authorization_state, \
             c.authorized_capabilities, c.expires_at, c.failure_code \
             FROM external_connections c LEFT JOIN integration_definitions i ON i.id=c.integration_id \
             LEFT JOIN remote_extensions e ON e.id=c.remote_extension_id \
             WHERE c.id=$1 AND c.user_context_id=$2",
        )
        .bind(connection_id)
        .bind(context.id.0)
        .fetch_optional(&self.db)
        .await?
        .ok_or(ConnectionError::NotFound)?;

        connection_from_row(context.id, &row)
    }

    /// Revocation is context-scoped and idempotent. It blocks new Core attempts;
    /// it cannot claim to revoke an external operator's independent access.
    pub async fn disconnect(
        &self,
        context: &RequestContext,
        connection_id: Uuid,
    ) -> Result<Connection, ConnectionError> {
        let mut tx = self.db.begin().await?;
        // Serialize with OAuth callback storage, which locks the extension
        // before writing credentials and the connection record.
        let remote_extension_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT remote_extension_id FROM external_connections WHERE id=$1 AND user_context_id=$2",
        )
        .bind(connection_id)
        .bind(context.id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ConnectionError::NotFound)?;
        if let Some(extension_id) = remote_extension_id {
            sqlx::query("SELECT id FROM remote_extensions WHERE id=$1 FOR UPDATE")
                .bind(extension_id)
                .fetch_one(&mut *tx)
                .await?;
        }
        let row = sqlx::query(
            "UPDATE external_connections SET authorization_state='revoked', \
             authorized_capabilities='{}'::text[], expires_at=NULL, failure_code=NULL, \
             revoked_at=COALESCE(revoked_at,now()), updated_at=now() \
             WHERE id=$1 AND user_context_id=$2 \
             RETURNING id, integration_id, remote_extension_id, account_display_id, credential_custody, authorization_state, \
             authorized_capabilities, expires_at, failure_code",
        )
        .bind(connection_id)
        .bind(context.id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ConnectionError::NotFound)?;

        // A later reconnection must not revive the old agent grants.
        sqlx::query(
            "UPDATE agent_capability_grants SET state='revoked', \
             revoked_at=COALESCE(revoked_at,now()), updated_at=now() \
             WHERE connection_id=$1 AND user_context_id=$2 AND state='enabled'",
        )
        .bind(connection_id)
        .bind(context.id.0)
        .execute(&mut *tx)
        .await?;

        if let Some(extension_id) = remote_extension_id {
            sqlx::query("DELETE FROM remote_extension_credentials WHERE extension_id=$1")
                .bind(extension_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM mcp_authorization_sessions WHERE extension_id=$1")
                .bind(extension_id)
                .execute(&mut *tx)
                .await?;
        }
        let integration_key: String = sqlx::query_scalar(
            "SELECT COALESCE(i.external_key,e.external_key) FROM external_connections c \
             LEFT JOIN integration_definitions i ON i.id=c.integration_id \
             LEFT JOIN remote_extensions e ON e.id=c.remote_extension_id WHERE c.id=$1",
        )
        .bind(connection_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(Connection {
            id: row.get("id"),
            user_context_id: context.id,
            integration_external_key: integration_key,
            account_display_id: row.try_get("account_display_id").ok(),
            credential_custody: parse_custody(row.get("credential_custody"))?,
            authorization_state: AuthorizationState::Revoked,
            authorized_capabilities: row.get("authorized_capabilities"),
            expires_at: row.get("expires_at"),
            failure_code: row.get("failure_code"),
        })
    }
}

fn connection_from_row(
    context_id: UserContextId,
    row: &sqlx::postgres::PgRow,
) -> Result<Connection, ConnectionError> {
    Ok(Connection {
        id: row.get("id"),
        user_context_id: context_id,
        integration_external_key: row.get("external_key"),
        account_display_id: row.try_get("account_display_id").ok(),
        credential_custody: parse_custody(row.get("credential_custody"))?,
        authorization_state: parse_state(row.get("authorization_state"))?,
        authorized_capabilities: row.get("authorized_capabilities"),
        expires_at: row.get("expires_at"),
        failure_code: row.get("failure_code"),
    })
}

fn parse_custody(value: &str) -> Result<CredentialCustody, ConnectionError> {
    match value {
        "platform_held" => Ok(CredentialCustody::PlatformHeld),
        "external_operator" => Ok(CredentialCustody::ExternalOperator),
        _ => Err(ConnectionError::Invalid),
    }
}

fn parse_state(value: &str) -> Result<AuthorizationState, ConnectionError> {
    match value {
        "pending" => Ok(AuthorizationState::Pending),
        "authorized" => Ok(AuthorizationState::Authorized),
        "expired" => Ok(AuthorizationState::Expired),
        "revoked" => Ok(AuthorizationState::Revoked),
        "cancelled" => Ok(AuthorizationState::Cancelled),
        "failed" => Ok(AuthorizationState::Failed),
        _ => Err(ConnectionError::Invalid),
    }
}
