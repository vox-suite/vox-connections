use crate::{
    crypto::CredentialCipher,
    providers::{
        google_calendar::{Client as GoogleClient, GoogleCalendarEvent, build_auth_url},
        observations::{GameSnapshot, observed_activity},
        playstation::PlayStationGame,
    },
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum FreshConnectionError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Crypto error")]
    Crypto,
    #[error("Connector not configured: {0}")]
    NotConfigured(String),
    #[error("Connection not found")]
    NotFound,
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Invalid request: {0}")]
    Invalid(String),
    #[error("Unauthorized")]
    Unauthorized,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConnectorDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub supported_features: Vec<String>,
    pub auth_type: String,
    pub available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConnectionItem {
    pub id: Uuid,
    pub connector_id: String,
    pub account_display_id: Option<String>,
    pub authorization_state: String,
    pub sync_timeline: bool,
    pub assistant_read: bool,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub failure_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct StartConnectionRequest {
    pub connector_id: String,
    pub npsso: Option<String>,
    pub consent: bool,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct StartConnectionResponse {
    pub setup_id: Option<Uuid>,
    pub connection_id: Option<Uuid>,
    pub authorization_url: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct SetupStatusResponse {
    pub status: String,
    pub connection_id: Option<Uuid>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct PreferencesRequest {
    pub sync_timeline: Option<bool>,
    pub assistant_read: Option<bool>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct RefreshResponse {
    pub refreshed: bool,
    pub spans_created: usize,
}

#[async_trait::async_trait]
#[allow(clippy::too_many_arguments)]
pub trait TimelineIngestor: Send + Sync {
    async fn calendar(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: Uuid,
        connection_id: Uuid,
        account_id: &str,
        events: &[GoogleCalendarEvent],
        time_min: DateTime<Utc>,
        time_max: DateTime<Utc>,
    ) -> Result<usize, FreshConnectionError>;
    async fn gaming(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: Uuid,
        connection_id: Uuid,
        activities: &[crate::providers::observations::ObservedActivity],
    ) -> Result<usize, FreshConnectionError>;
    async fn game_history(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _user_id: Uuid,
        _connection_id: Uuid,
        _games: &[PlayStationGame],
    ) -> Result<usize, FreshConnectionError> {
        Ok(0)
    }
    async fn food_order(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _user_id: Uuid,
        _connection_id: Uuid,
        _orders: &[crate::providers::food_delivery::FoodDeliveryOrder],
    ) -> Result<usize, FreshConnectionError> {
        Ok(0)
    }
    async fn personal_activity(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _user_id: Uuid,
        _connection_id: Uuid,
        _connector: &str,
        _items: &[crate::providers::personal::PersonalActivity],
    ) -> Result<usize, FreshConnectionError> {
        Ok(0)
    }
    fn food_committed(
        &self,
        _user_id: Uuid,
        _orders: &[crate::providers::food_delivery::FoodDeliveryOrder],
    ) {
    }
}

#[derive(Clone)]
pub struct FreshConnectionsService {
    pool: PgPool,
    pub(crate) cipher: Option<CredentialCipher>,
    google: GoogleClient,
    psn: crate::providers::psn::Client,
    pub(crate) swiggy: crate::providers::swiggy::SwiggyClient,
    pub(crate) zomato: crate::providers::zomato::ZomatoClient,
    personal: crate::providers::personal::Client,
    ingestor: std::sync::Arc<dyn TimelineIngestor>,
    google_client_id: Option<String>,
    google_client_secret: Option<String>,
    pub(crate) core_api_url: Option<String>,
}

impl FreshConnectionsService {
    pub fn new(
        pool: PgPool,
        credential_key: Option<&str>,
        ingestor: std::sync::Arc<dyn TimelineIngestor>,
        google_client_id: Option<String>,
        google_client_secret: Option<String>,
        core_api_url: Option<String>,
    ) -> Result<Self, FreshConnectionError> {
        let cipher = credential_key
            .map(CredentialCipher::from_hex_key)
            .transpose()
            .map_err(|_| FreshConnectionError::Crypto)?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| FreshConnectionError::Provider("HTTP client unavailable".into()))?;

        Ok(Self {
            pool,
            cipher,
            google: GoogleClient::new(http.clone()),
            psn: crate::providers::psn::Client::new().map_err(map_psn_error)?,
            swiggy: crate::providers::swiggy::SwiggyClient::new(http.clone()),
            personal: crate::providers::personal::Client::new(
                http.clone(),
                google_client_id.clone(),
                google_client_secret.clone(),
            ),
            zomato: crate::providers::zomato::ZomatoClient::new(http),
            ingestor,
            google_client_id,
            google_client_secret,
            core_api_url,
        })
    }

    fn cipher(&self) -> Result<&CredentialCipher, FreshConnectionError> {
        self.cipher.as_ref().ok_or_else(|| {
            FreshConnectionError::NotConfigured("credential encryption is disabled".into())
        })
    }

    pub fn list_connectors(&self) -> Vec<ConnectorDescriptor> {
        let mut descriptors = vec![
            ConnectorDescriptor {
                id: "google_calendar".to_string(),
                name: "Google Calendar".to_string(),
                description: "Read-only access to your primary calendar for timeline synchronization and assistant context.".to_string(),
                supported_features: vec!["timeline_sync".to_string(), "assistant_read".to_string()],
                auth_type: "oauth2".to_string(),
                available: self.cipher.is_some() && self.google_client_id.is_some() && self.google_client_secret.is_some() && self.core_api_url.is_some(),
            },
            ConnectorDescriptor {
                id: "playstation".to_string(),
                name: "PlayStation Network".to_string(),
                description: "Track gaming activity and playtime from your PlayStation account via community NPSSO token.".to_string(),
                supported_features: vec!["timeline_sync".to_string(), "assistant_read".to_string()],
                auth_type: "npsso".to_string(),
                available: self.cipher.is_some(),
            },
            self.swiggy_descriptor(),
            self.zomato_descriptor(),
        ];
        for (id, name, description) in [
            (
                "spotify",
                "Spotify",
                "Recently played music with provider playback timestamps.",
            ),
            (
                "youtube",
                "YouTube",
                "Read playlists, likes and subscriptions. Watch history requires a Google Takeout import.",
            ),
        ] {
            descriptors.push(ConnectorDescriptor {
                id: id.into(),
                name: name.into(),
                description: description.into(),
                supported_features: vec!["timeline_sync".into(), "assistant_read".into()],
                auth_type: "oauth2".into(),
                available: self.cipher.is_some()
                    && self.core_api_url.is_some()
                    && self.personal.enabled(id),
            });
        }
        descriptors
    }

    pub async fn list_connections(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ConnectionItem>, FreshConnectionError> {
        let rows = sqlx::query(
            "SELECT id, connector_id, account_display_id, authorization_state, sync_timeline, assistant_read, last_synced_at, failure_code, created_at \
             FROM vox_connections WHERE user_id = $1 ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ConnectionItem {
                id: r.get("id"),
                connector_id: r.get("connector_id"),
                account_display_id: r.get("account_display_id"),
                authorization_state: r.get("authorization_state"),
                sync_timeline: r.get("sync_timeline"),
                assistant_read: r.get("assistant_read"),
                last_synced_at: r.get("last_synced_at"),
                failure_code: r.get("failure_code"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    pub async fn start(
        &self,
        user_id: Uuid,
        req: StartConnectionRequest,
    ) -> Result<StartConnectionResponse, FreshConnectionError> {
        self.cipher()?;
        if !req.consent {
            return Err(FreshConnectionError::Invalid(
                "Consent to timeline synchronization and assistant reads is required".into(),
            ));
        }
        match req.connector_id.as_str() {
            "google_calendar" => {
                let client_id = self.google_client_id.as_deref().ok_or_else(|| {
                    FreshConnectionError::NotConfigured(
                        "Google OAuth client ID is not configured".to_string(),
                    )
                })?;

                let redirect_uri = self.google_redirect()?;
                let state_token =
                    crate::crypto::random_token().map_err(|_| FreshConnectionError::Crypto)?;
                let verifier =
                    crate::crypto::random_token().map_err(|_| FreshConnectionError::Crypto)?;
                let verifier_cipher = self
                    .cipher()?
                    .seal(state_token.as_bytes(), &verifier)
                    .map_err(|_| FreshConnectionError::Crypto)?;
                let mut tx = self.pool.begin().await?;
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                    .bind(format!("setup:{user_id}:google_calendar"))
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("UPDATE vox_connection_setups SET status='cancelled', verifier_ciphertext=NULL WHERE user_id=$1 AND connector_id='google_calendar' AND status IN ('pending','exchanging')").bind(user_id).execute(&mut *tx).await?;
                let setup_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO vox_connection_setups(user_id,connector_id,state_token,verifier_ciphertext,redirect_uri,consented_at) VALUES($1,'google_calendar',$2,$3,$4,now()) RETURNING id")
                    .bind(user_id).bind(&state_token).bind(verifier_cipher).bind(&redirect_uri).fetch_one(&mut *tx).await?;
                tx.commit().await?;
                let auth_url = build_auth_url(client_id, &redirect_uri, &state_token, &verifier);
                Ok(StartConnectionResponse {
                    setup_id: Some(setup_id),
                    connection_id: None,
                    authorization_url: Some(auth_url),
                    status: "pending".to_string(),
                })
            }
            "playstation" => {
                let npsso = req
                    .npsso
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        FreshConnectionError::Invalid(
                            "NPSSO token is required for PlayStation linking".to_string(),
                        )
                    })?;

                let mut setup_tx = self.pool.begin().await?;
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                    .bind(format!("setup:{user_id}:playstation"))
                    .execute(&mut *setup_tx)
                    .await?;
                sqlx::query("UPDATE vox_connection_setups SET status='cancelled' WHERE user_id=$1 AND connector_id='playstation' AND status IN ('pending','exchanging')").bind(user_id).execute(&mut *setup_tx).await?;
                let setup_id:Uuid=sqlx::query_scalar("INSERT INTO vox_connection_setups(user_id,connector_id,state_token,status,consented_at) VALUES($1,'playstation',$2,'exchanging',now()) RETURNING id").bind(user_id).bind(crate::crypto::random_token()?).fetch_one(&mut *setup_tx).await?;
                setup_tx.commit().await?;
                let (account_id, display_id, access_token, refresh_token, access_expires_in) =
                    self.exchange_playstation_npsso(npsso).await?;

                let aad = format!("{user_id}:playstation");
                let access_cipher = self
                    .cipher()?
                    .seal(aad.as_bytes(), &access_token)
                    .map_err(|_| FreshConnectionError::Crypto)?;
                let refresh_cipher = self
                    .cipher()?
                    .seal(aad.as_bytes(), &refresh_token)
                    .map_err(|_| FreshConnectionError::Crypto)?;
                let access_expires_at = Utc::now() + Duration::seconds(access_expires_in);

                let initial_snapshots = self
                    .fetch_playstation_baseline(&access_token, &account_id)
                    .await?;

                let mut tx = self.pool.begin().await?;
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                    .bind(format!("setup:{user_id}:playstation"))
                    .execute(&mut *tx)
                    .await?;
                let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM vox_connection_setups WHERE id=$1 AND status='exchanging' AND expires_at>now() FOR UPDATE").bind(setup_id).fetch_optional(&mut *tx).await?;
                if valid.is_none() {
                    return Err(FreshConnectionError::Unauthorized);
                }
                let conn_id = sqlx::query_scalar::<_, Uuid>(
                    "INSERT INTO vox_connections \
                     (user_id, connector_id, account_id, account_display_id, access_ciphertext, refresh_ciphertext, access_expires_at, \
                      authorization_state, sync_timeline, assistant_read, metadata, last_synced_at, next_sync_at, failure_code, failure_count, updated_at, consented_at) \
                     VALUES ($1, 'playstation', $2, $7, $3, $4, $5, 'authorized', true, true, $6, now(), now() + interval '1 day', NULL, 0, now(), now()) \
                     ON CONFLICT (user_id, connector_id) DO UPDATE SET \
                      generation = gen_random_uuid(), credential_generation=gen_random_uuid(), lease_token = NULL, lease_until = NULL, consented_at = now(), sync_timeline = true, assistant_read = true, \
              account_id = EXCLUDED.account_id, \
                      account_display_id = EXCLUDED.account_display_id, \
                      access_ciphertext = EXCLUDED.access_ciphertext, \
                      refresh_ciphertext = EXCLUDED.refresh_ciphertext, \
                      access_expires_at = EXCLUDED.access_expires_at, \
                      authorization_state = 'authorized', \
                      metadata = EXCLUDED.metadata, \
                      last_synced_at = now(), \
                      next_sync_at = now() + interval '1 day', \
                      failure_code = NULL, \
                      failure_count = 0, \
                      updated_at = now() \
                     RETURNING id"
                )
                .bind(user_id)
                .bind(&account_id)
                .bind(access_cipher)
                .bind(refresh_cipher)
                .bind(access_expires_at)
                .bind(json!({ "snapshots": initial_snapshots }))
                .bind(display_id)
                .fetch_one(&mut *tx)
                .await?;

                sqlx::query("UPDATE vox_connection_setups SET status='authorized',connection_id=$2 WHERE id=$1").bind(setup_id).bind(conn_id).execute(&mut *tx).await?;
                tx.commit().await?;

                Ok(StartConnectionResponse {
                    setup_id: None,
                    connection_id: Some(conn_id),
                    authorization_url: None,
                    status: "authorized".to_string(),
                })
            }
            "spotify" | "youtube" => self.start_personal_oauth(user_id, &req.connector_id).await,
            "swiggy" | "zomato" => self.start_food_oauth(user_id, &req.connector_id).await,
            other => Err(FreshConnectionError::Invalid(format!(
                "Unknown connector '{other}'"
            ))),
        }
    }

    pub async fn get_setup_status(
        &self,
        user_id: Uuid,
        setup_id: Uuid,
    ) -> Result<SetupStatusResponse, FreshConnectionError> {
        sqlx::query("UPDATE vox_connection_setups SET status='expired', verifier_ciphertext=NULL, error='setup_expired' WHERE id=$1 AND user_id=$2 AND expires_at <= now() AND status IN ('pending','exchanging')").bind(setup_id).bind(user_id).execute(&self.pool).await?;
        let row = sqlx::query(
            "SELECT status, connection_id, error FROM vox_connection_setups WHERE id = $1 AND user_id = $2"
        )
        .bind(setup_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(FreshConnectionError::NotFound)?;

        Ok(SetupStatusResponse {
            status: row.get("status"),
            connection_id: row.get("connection_id"),
            error: row.get("error"),
        })
    }

    pub async fn update_preferences(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
        req: PreferencesRequest,
    ) -> Result<ConnectionItem, FreshConnectionError> {
        let result = sqlx::query("UPDATE vox_connections SET sync_timeline=COALESCE($3,sync_timeline), assistant_read=COALESCE($4,assistant_read), generation=gen_random_uuid(), metadata=CASE WHEN $3=true AND NOT sync_timeline AND connector_id='playstation' THEN jsonb_set(metadata,'{baseline_required}','true') ELSE metadata END, next_sync_at=now(), updated_at=now() WHERE id=$1 AND user_id=$2")
            .bind(connection_id).bind(user_id).bind(req.sync_timeline).bind(req.assistant_read).execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(FreshConnectionError::NotFound);
        }

        let updated = sqlx::query(
            "SELECT id, connector_id, account_display_id, authorization_state, sync_timeline, assistant_read, last_synced_at, failure_code, created_at \
             FROM vox_connections WHERE id = $1 AND user_id = $2"
        )
        .bind(connection_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(ConnectionItem {
            id: updated.get("id"),
            connector_id: updated.get("connector_id"),
            account_display_id: updated.get("account_display_id"),
            authorization_state: updated.get("authorization_state"),
            sync_timeline: updated.get("sync_timeline"),
            assistant_read: updated.get("assistant_read"),
            last_synced_at: updated.get("last_synced_at"),
            failure_code: updated.get("failure_code"),
            created_at: updated.get("created_at"),
        })
    }

    pub async fn refresh(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
    ) -> Result<RefreshResponse, FreshConnectionError> {
        let conn =
            sqlx::query("SELECT connector_id FROM vox_connections WHERE id = $1 AND user_id = $2")
                .bind(connection_id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(FreshConnectionError::NotFound)?;

        let connector_id: String = conn.get("connector_id");
        match connector_id.as_str() {
            "google_calendar" => {
                let count = self.sync_google_calendar(user_id, connection_id).await?;
                Ok(RefreshResponse {
                    refreshed: true,
                    spans_created: count,
                })
            }
            "playstation" => {
                let count = self.sync_playstation(user_id, connection_id).await?;
                Ok(RefreshResponse {
                    refreshed: true,
                    spans_created: count,
                })
            }
            "swiggy" => {
                let count = self.sync_swiggy(user_id, connection_id).await?;
                Ok(RefreshResponse {
                    refreshed: true,
                    spans_created: count,
                })
            }
            "zomato" => {
                let count = self.sync_zomato(user_id, connection_id).await?;
                Ok(RefreshResponse {
                    refreshed: true,
                    spans_created: count,
                })
            }
            "spotify" | "youtube" => {
                let count = self
                    .sync_personal(user_id, connection_id, &connector_id)
                    .await?;
                Ok(RefreshResponse {
                    refreshed: true,
                    spans_created: count,
                })
            }
            _ => Err(FreshConnectionError::NotFound),
        }
    }

    pub async fn disconnect(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
    ) -> Result<(), FreshConnectionError> {
        let mut tx = self.pool.begin().await?;
        let connector: Option<String> = sqlx::query_scalar(
            "SELECT connector_id FROM vox_connections WHERE id=$1 AND user_id=$2",
        )
        .bind(connection_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(connector) = connector {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                .bind(format!("setup:{user_id}:{connector}"))
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE vox_connection_setups SET status='cancelled',verifier_ciphertext=NULL,error='authorization_cancelled' WHERE user_id=$1 AND connector_id=$2 AND status IN ('pending','exchanging')").bind(user_id).bind(connector).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM vox_connections WHERE id=$1 AND user_id=$2")
                .bind(connection_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;

        Ok(())
    }

    pub async fn handle_google_callback(
        &self,
        code: &str,
        state: &str,
    ) -> Result<Uuid, FreshConnectionError> {
        let setup = sqlx::query(
            "UPDATE vox_connection_setups SET status='exchanging' WHERE state_token = $1 AND status = 'pending' AND expires_at > now() RETURNING id, user_id, verifier_ciphertext, redirect_uri"
        )
        .bind(state)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| FreshConnectionError::Invalid("Invalid or expired OAuth state token".to_string()))?;

        let setup_id: Uuid = setup.get("id");
        let result = self.complete_google_callback(code, state, &setup).await;
        if result.is_err() {
            sqlx::query("UPDATE vox_connection_setups SET status='failed',error='authorization_failed',verifier_ciphertext=NULL WHERE id=$1 AND status='exchanging'").bind(setup_id).execute(&self.pool).await?;
        }
        result
    }
    async fn complete_google_callback(
        &self,
        code: &str,
        state: &str,
        setup: &sqlx::postgres::PgRow,
    ) -> Result<Uuid, FreshConnectionError> {
        let setup_id: Uuid = setup.get("id");
        let user_id: Uuid = setup.get("user_id");

        let client_id = self.google_client_id.as_deref().ok_or_else(|| {
            FreshConnectionError::NotConfigured("Google OAuth client ID missing".to_string())
        })?;
        let client_secret = self.google_client_secret.as_deref().ok_or_else(|| {
            FreshConnectionError::NotConfigured("Google OAuth client secret missing".to_string())
        })?;

        let redirect_uri: String = setup.get("redirect_uri");
        let verifier_bytes: Vec<u8> = setup.get("verifier_ciphertext");
        let verifier = self
            .cipher()?
            .open(state.as_bytes(), &verifier_bytes)
            .map_err(|_| FreshConnectionError::Crypto)?;
        let tokens = self
            .google
            .exchange(client_id, client_secret, &redirect_uri, code, &verifier)
            .await
            .map_err(|e| match e {
                crate::providers::google_calendar::GoogleCalendarError::Unauthorized => {
                    FreshConnectionError::Unauthorized
                }
                _ => FreshConnectionError::Provider("Google request failed".into()),
            })?;

        if tokens.refresh_token.is_none() {
            return Err(FreshConnectionError::Unauthorized);
        }
        self.google
            .events(
                &tokens.access_token,
                Utc::now(),
                Utc::now() + Duration::days(1),
            )
            .await
            .map_err(|_| FreshConnectionError::Unauthorized)?;
        let profile = self
            .google
            .profile(&tokens.access_token)
            .await
            .map_err(|e| match e {
                crate::providers::google_calendar::GoogleCalendarError::Unauthorized => {
                    FreshConnectionError::Unauthorized
                }
                _ => FreshConnectionError::Provider("Google request failed".into()),
            })?;

        let aad = format!("{user_id}:google_calendar");
        let access_cipher = self
            .cipher()?
            .seal(aad.as_bytes(), &tokens.access_token)
            .map_err(|_| FreshConnectionError::Crypto)?;

        let refresh_cipher = match tokens.refresh_token.as_deref() {
            Some(rt) => Some(
                self.cipher()?
                    .seal(aad.as_bytes(), rt)
                    .map_err(|_| FreshConnectionError::Crypto)?,
            ),
            None => None,
        };

        let access_expires_at = Utc::now() + Duration::seconds(tokens.expires_in);
        let display = profile.email.clone().or(profile.name.clone());

        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user_id}:google_calendar"))
            .execute(&mut *tx)
            .await?;
        let valid: Option<Uuid> = sqlx::query_scalar("SELECT id FROM vox_connection_setups WHERE id=$1 AND status='exchanging' AND expires_at>now() FOR UPDATE").bind(setup_id).fetch_optional(&mut *tx).await?;
        if valid.is_none() {
            return Err(FreshConnectionError::Invalid(
                "Setup cancelled or expired".into(),
            ));
        }
        let connection_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO vox_connections \
             (user_id, connector_id, account_id, account_display_id, access_ciphertext, refresh_ciphertext, access_expires_at, \
              authorization_state, sync_timeline, assistant_read, updated_at, consented_at) \
             VALUES ($1, 'google_calendar', $2, $3, $4, $5, $6, 'authorized', true, true, now(), now()) \
             ON CONFLICT (user_id, connector_id) DO UPDATE SET \
              generation = gen_random_uuid(), credential_generation=gen_random_uuid(), lease_token = NULL, lease_until = NULL, consented_at = now(), sync_timeline = true, assistant_read = true, \
              account_id = EXCLUDED.account_id, \
              account_display_id = EXCLUDED.account_display_id, \
              access_ciphertext = EXCLUDED.access_ciphertext, \
              refresh_ciphertext = EXCLUDED.refresh_ciphertext, \
              access_expires_at = EXCLUDED.access_expires_at, \
              authorization_state = 'authorized', next_sync_at=now(), metadata='{}', last_synced_at=NULL, \
              failure_code = NULL, \
              failure_count = 0, \
              updated_at = now() \
             RETURNING id"
        )
        .bind(user_id)
        .bind(&profile.id)
        .bind(&display)
        .bind(access_cipher)
        .bind(refresh_cipher)
        .bind(access_expires_at)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            "UPDATE vox_connection_setups SET status = 'authorized', verifier_ciphertext=NULL, connection_id = $1 WHERE id = $2"
        )
        .bind(connection_id)
        .bind(setup_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        let _ = self.refresh(user_id, connection_id).await;

        Ok(connection_id)
    }

    pub async fn sync_google_calendar(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
    ) -> Result<usize, FreshConnectionError> {
        let conn = self.claim(user_id, connection_id).await?;
        let result = self
            .sync_google_calendar_claimed(user_id, connection_id, &conn)
            .await;
        if let Err(ref error) = result {
            self.release_failure(connection_id, &conn, error).await?;
        }
        result
    }
    async fn sync_google_calendar_claimed(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<usize, FreshConnectionError> {
        let sync_timeline: bool = conn.get("sync_timeline");
        let aad = format!("{user_id}:google_calendar");

        let mut access_token = {
            let ct: Option<Vec<u8>> = conn.get("access_ciphertext");
            match ct {
                Some(bytes) => self
                    .cipher()?
                    .open(aad.as_bytes(), &bytes)
                    .map_err(|_| FreshConnectionError::Crypto)?,
                None => return Err(FreshConnectionError::Unauthorized),
            }
        };

        let expires_at: Option<DateTime<Utc>> = conn.get("access_expires_at");
        if expires_at.is_none_or(|exp| exp <= Utc::now() + Duration::seconds(60)) {
            let rt_bytes: Option<Vec<u8>> = conn.get("refresh_ciphertext");
            let Some(rt_bytes) = rt_bytes else {
                return Err(FreshConnectionError::Unauthorized);
            };
            let refresh_token = self
                .cipher()?
                .open(aad.as_bytes(), &rt_bytes)
                .map_err(|_| FreshConnectionError::Crypto)?;

            let client_id = self.google_client_id.as_deref().unwrap_or_default();
            let client_secret = self.google_client_secret.as_deref().unwrap_or_default();
            let fresh = self
                .google
                .refresh(client_id, client_secret, &refresh_token)
                .await
                .map_err(|e| match e {
                    crate::providers::google_calendar::GoogleCalendarError::Unauthorized => {
                        FreshConnectionError::Unauthorized
                    }
                    _ => FreshConnectionError::Provider("Google request failed".into()),
                })?;

            access_token = fresh.access_token.clone();
            let new_access_cipher = self
                .cipher()?
                .seal(aad.as_bytes(), &fresh.access_token)
                .map_err(|_| FreshConnectionError::Crypto)?;
            let new_exp = Utc::now() + Duration::seconds(fresh.expires_in);

            let refresh_cipher = fresh
                .refresh_token
                .as_deref()
                .map(|v| {
                    self.cipher()?
                        .seal(aad.as_bytes(), v)
                        .map_err(|_| FreshConnectionError::Crypto)
                })
                .transpose()?;
            self.store_rotated(
                conn,
                connection_id,
                new_access_cipher,
                refresh_cipher,
                new_exp,
            )
            .await?;
        }

        let now = Utc::now();
        let time_min = now - Duration::days(30);
        let time_max = now + Duration::days(90);

        let events = self
            .google
            .events(&access_token, time_min, time_max)
            .await
            .map_err(|e| match e {
                crate::providers::google_calendar::GoogleCalendarError::Unauthorized => {
                    FreshConnectionError::Unauthorized
                }
                _ => FreshConnectionError::Provider("Google request failed".into()),
            })?;

        let mut tx = self.pool.begin().await?;
        self.validate_commit(&mut tx, user_id, connection_id, conn)
            .await?;
        let created_or_updated = if sync_timeline {
            self.ingestor
                .calendar(
                    &mut tx,
                    user_id,
                    connection_id,
                    &conn.get::<String, _>("account_id"),
                    &events,
                    time_min,
                    time_max,
                )
                .await?
        } else {
            0
        };
        sqlx::query(
            "UPDATE vox_connections SET last_synced_at = now(), next_sync_at = now() + interval '5 minutes', failure_code = NULL, failure_count = 0, lease_token=NULL, lease_until=NULL, updated_at = now() WHERE id = $1"
        )
        .bind(connection_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(created_or_updated)
    }

    pub async fn sync_playstation(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
    ) -> Result<usize, FreshConnectionError> {
        let conn = self.claim(user_id, connection_id).await?;
        let result = self
            .sync_playstation_claimed(user_id, connection_id, &conn)
            .await;
        if let Err(ref error) = result {
            self.release_failure(connection_id, &conn, error).await?;
        }
        result
    }
    async fn sync_playstation_claimed(
        &self,
        user_id: Uuid,
        connection_id: Uuid,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<usize, FreshConnectionError> {
        let account_id: String = conn.get("account_id");
        let sync_timeline: bool = conn.get("sync_timeline");
        let aad = format!("{user_id}:playstation");

        let rt_bytes: Option<Vec<u8>> = conn.get("refresh_ciphertext");
        let Some(rt_bytes) = rt_bytes else {
            return Err(FreshConnectionError::Unauthorized);
        };
        let refresh_token = self
            .cipher()?
            .open(aad.as_bytes(), &rt_bytes)
            .map_err(|_| FreshConnectionError::Crypto)?;

        let (new_access, new_refresh, exp_sec) =
            self.refresh_playstation_tokens(&refresh_token).await?;
        let access_cipher = self
            .cipher()?
            .seal(aad.as_bytes(), &new_access)
            .map_err(|_| FreshConnectionError::Crypto)?;
        let refresh_cipher = self
            .cipher()?
            .seal(aad.as_bytes(), &new_refresh)
            .map_err(|_| FreshConnectionError::Crypto)?;
        let exp_at = Utc::now() + Duration::seconds(exp_sec);

        self.store_rotated(
            conn,
            connection_id,
            access_cipher.clone(),
            Some(refresh_cipher.clone()),
            exp_at,
        )
        .await?;
        let games = self
            .fetch_playstation_games(&new_access, &account_id)
            .await?;

        let mut snapshots: BTreeMap<String, GameSnapshot> = conn
            .get::<Value, _>("metadata")
            .get("snapshots")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let now = Utc::now();
        let last_synced_at: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT last_synced_at FROM vox_connections WHERE id = $1")
                .bind(connection_id)
                .fetch_one(&self.pool)
                .await?;

        let baseline_required = conn
            .get::<Value, _>("metadata")
            .get("baseline_required")
            .and_then(Value::as_bool)
            == Some(true);
        let activities = if baseline_required {
            Vec::new()
        } else {
            observed_activity(&account_id, &snapshots, &games, last_synced_at, now)
        };

        let mut tx = self.pool.begin().await?;
        self.validate_commit(&mut tx, user_id, connection_id, conn)
            .await?;
        let spans_created = if sync_timeline {
            self.ingestor
                .gaming(&mut tx, user_id, connection_id, &activities)
                .await?
                + self
                    .ingestor
                    .game_history(&mut tx, user_id, connection_id, &games)
                    .await?
        } else {
            0
        };
        snapshots = crate::providers::observations::checkpoint(&snapshots, &games, now);

        sqlx::query(
            "UPDATE vox_connections SET access_ciphertext = $1, refresh_ciphertext = $2, access_expires_at = $3, metadata = $4, last_synced_at = now(), next_sync_at = now() + interval '30 minutes', failure_code = NULL, failure_count = 0, lease_token=NULL, lease_until=NULL, updated_at = now() WHERE id = $5"
        )
        .bind(access_cipher)
        .bind(refresh_cipher)
        .bind(exp_at)
        .bind(json!({ "snapshots": snapshots }))
        .bind(connection_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(spans_created)
    }

    fn food_client(
        &self,
        connector: &str,
    ) -> Result<&crate::providers::food_delivery::Client, FreshConnectionError> {
        match connector {
            "swiggy" => Ok(&self.swiggy.client),
            "zomato" => Ok(&self.zomato.client),
            _ => Err(FreshConnectionError::NotFound),
        }
    }
    async fn start_food_oauth(
        &self,
        user_id: Uuid,
        connector: &str,
    ) -> Result<StartConnectionResponse, FreshConnectionError> {
        let client = self.food_client(connector)?;
        if !client.enabled() {
            return Err(FreshConnectionError::NotConfigured(
                "Provider access and callback approval are required".into(),
            ));
        }
        let redirect = self
            .google_redirect()?
            .replace("/google/callback", &format!("/{connector}/callback"));
        let client_id = client.register(&redirect).await.map_err(map_food_error)?;
        let state = crate::crypto::random_token()?;
        let verifier = crate::crypto::random_token()?;
        let auth_url = client.auth_url(&client_id, &redirect, &state, &verifier);
        let credentials = serde_json::to_string(&crate::providers::food_oauth::SetupCredentials {
            verifier,
            client_id,
        })
        .map_err(|_| FreshConnectionError::Crypto)?;
        let sealed = self.cipher()?.seal(state.as_bytes(), &credentials)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user_id}:{connector}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE vox_connection_setups SET status='cancelled',verifier_ciphertext=NULL WHERE user_id=$1 AND connector_id=$2 AND status IN ('pending','exchanging')")
            .bind(user_id).bind(connector).execute(&mut *tx).await?;
        let id: Uuid = sqlx::query_scalar("INSERT INTO vox_connection_setups(user_id,connector_id,state_token,verifier_ciphertext,redirect_uri,consented_at) VALUES($1,$2,$3,$4,$5,now()) RETURNING id")
            .bind(user_id).bind(connector).bind(&state).bind(sealed).bind(redirect).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(StartConnectionResponse {
            setup_id: Some(id),
            connection_id: None,
            authorization_url: Some(auth_url),
            status: "pending".into(),
        })
    }
    pub async fn handle_food_callback(
        &self,
        connector: &str,
        code: Option<&str>,
        state: &str,
    ) -> Result<Uuid, FreshConnectionError> {
        let client = self.food_client(connector)?;
        if !client.enabled() {
            return Err(FreshConnectionError::NotConfigured(
                "Provider disabled".into(),
            ));
        }
        let setup = sqlx::query("UPDATE vox_connection_setups SET status='exchanging' WHERE connector_id=$1 AND state_token=$2 AND status='pending' AND expires_at>now() AND consented_at IS NOT NULL RETURNING *")
            .bind(connector).bind(state).fetch_optional(&self.pool).await?.ok_or(FreshConnectionError::Unauthorized)?;
        let setup_id: Uuid = setup.get("id");
        let result = self
            .complete_food_callback(connector, code, state, &setup)
            .await;
        if result.is_err() {
            sqlx::query("UPDATE vox_connection_setups SET status='failed',error='authorization_failed',verifier_ciphertext=NULL WHERE id=$1 AND status='exchanging'")
                .bind(setup_id).execute(&self.pool).await?;
        }
        result
    }
    async fn complete_food_callback(
        &self,
        connector: &str,
        code: Option<&str>,
        state: &str,
        setup: &sqlx::postgres::PgRow,
    ) -> Result<Uuid, FreshConnectionError> {
        let code = code
            .filter(|s| !s.is_empty())
            .ok_or(FreshConnectionError::Unauthorized)?;
        let user_id: Uuid = setup.get("user_id");
        let setup_id: Uuid = setup.get("id");
        let redirect: String = setup.get("redirect_uri");
        let encrypted: Vec<u8> = setup.get("verifier_ciphertext");
        let credentials: crate::providers::food_oauth::SetupCredentials =
            serde_json::from_str(&self.cipher()?.open(state.as_bytes(), &encrypted)?)
                .map_err(|_| FreshConnectionError::Crypto)?;
        let client = self.food_client(connector)?;
        let tokens = client
            .exchange(
                &credentials.client_id,
                &redirect,
                code,
                &credentials.verifier,
            )
            .await
            .map_err(map_food_error)?;
        let orders = client
            .fetch_orders(&tokens.access_token)
            .await
            .map_err(map_food_error)?;
        let aad = format!("{user_id}:{connector}");
        let access = self.cipher()?.seal(aad.as_bytes(), &tokens.access_token)?;
        let refresh = tokens
            .refresh_token
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| self.cipher()?.seal(aad.as_bytes(), t))
            .transpose()?;
        let expires = Utc::now() + Duration::seconds(tokens.expires_in);
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user_id}:{connector}"))
            .execute(&mut *tx)
            .await?;
        let valid: Option<Uuid> = sqlx::query_scalar("SELECT id FROM vox_connection_setups WHERE id=$1 AND status='exchanging' AND expires_at>now() FOR UPDATE").bind(setup_id).fetch_optional(&mut *tx).await?;
        if valid.is_none() {
            return Err(FreshConnectionError::Unauthorized);
        }
        let id: Uuid = sqlx::query_scalar("INSERT INTO vox_connections(user_id,connector_id,access_ciphertext,refresh_ciphertext,access_expires_at,authorization_state,sync_timeline,assistant_read,metadata,consented_at) VALUES($1,$2,$3,$4,$5,'authorized',true,true,$6,now()) ON CONFLICT(user_id,connector_id) DO UPDATE SET generation=gen_random_uuid(),credential_generation=gen_random_uuid(),lease_token=NULL,lease_until=NULL,access_ciphertext=EXCLUDED.access_ciphertext,refresh_ciphertext=EXCLUDED.refresh_ciphertext,access_expires_at=EXCLUDED.access_expires_at,authorization_state='authorized',sync_timeline=true,assistant_read=true,metadata=EXCLUDED.metadata,consented_at=now(),failure_code=NULL,failure_count=0,account_id=NULL,account_display_id=NULL,updated_at=now() RETURNING id")
            .bind(user_id).bind(connector).bind(access).bind(refresh).bind(expires)
            .bind(json!({"auth_type":"oauth2","client_id":credentials.client_id})).fetch_one(&mut *tx).await?;
        self.ingestor
            .food_order(&mut tx, user_id, id, &orders)
            .await?;
        let interval = food_sync_interval(&orders);
        sqlx::query("UPDATE vox_connections SET last_synced_at=now(),next_sync_at=now()+($2*interval '1 second') WHERE id=$1")
            .bind(id).bind(interval).execute(&mut *tx).await?;
        sqlx::query("UPDATE vox_connection_setups SET status='authorized',connection_id=$2,verifier_ciphertext=NULL WHERE id=$1")
            .bind(setup_id).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.ingestor.food_committed(user_id, &orders);
        Ok(id)
    }
    pub(crate) async fn sync_food(
        &self,
        user_id: Uuid,
        id: Uuid,
        connector: &str,
    ) -> Result<usize, FreshConnectionError> {
        let conn = self.claim(user_id, id).await?;
        let result = self.sync_food_claimed(user_id, id, connector, &conn).await;
        if let Err(ref error) = result {
            self.release_failure(id, &conn, error).await?;
        }
        result
    }
    async fn sync_food_claimed(
        &self,
        user_id: Uuid,
        id: Uuid,
        connector: &str,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<usize, FreshConnectionError> {
        let orders = self.food_orders_claimed(user_id, connector, conn).await?;
        let mut tx = self.pool.begin().await?;
        self.validate_commit(&mut tx, user_id, id, conn).await?;
        let count = self
            .ingestor
            .food_order(&mut tx, user_id, id, &orders)
            .await?;
        sqlx::query("UPDATE vox_connections SET last_synced_at=now(),next_sync_at=now()+($2*interval '1 second'),lease_token=NULL,lease_until=NULL,failure_count=0,failure_code=NULL,updated_at=now() WHERE id=$1")
            .bind(id).bind(food_sync_interval(&orders)).execute(&mut *tx).await?;
        tx.commit().await?;
        self.ingestor.food_committed(user_id, &orders);
        Ok(count)
    }
    async fn food_orders_claimed(
        &self,
        user_id: Uuid,
        connector: &str,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<Vec<crate::providers::food_delivery::FoodDeliveryOrder>, FreshConnectionError> {
        if conn.get::<String, _>("connector_id") != connector {
            return Err(FreshConnectionError::Unauthorized);
        }
        let metadata: Value = conn.get("metadata");
        if metadata.get("auth_type").and_then(Value::as_str) != Some("oauth2") {
            return Err(FreshConnectionError::Unauthorized);
        }
        let client = self.food_client(connector)?;
        if !client.enabled() {
            return Err(FreshConnectionError::NotConfigured(
                "Provider disabled".into(),
            ));
        }
        let aad = format!("{user_id}:{connector}");
        let access: Vec<u8> = conn
            .try_get("access_ciphertext")
            .map_err(|_| FreshConnectionError::Unauthorized)?;
        let mut access = self.cipher()?.open(aad.as_bytes(), &access)?;
        let exp: Option<DateTime<Utc>> = conn.get("access_expires_at");
        let mut refreshed = false;
        if exp.is_none_or(|exp| exp <= Utc::now() + Duration::seconds(60)) {
            access = self.refresh_food_token(user_id, connector, conn).await?;
            refreshed = true;
        }
        match client.fetch_orders(&access).await {
            Err(crate::providers::food_delivery::FoodDeliveryError::Unauthorized) if !refreshed => {
                access = self.refresh_food_token(user_id, connector, conn).await?;
                client.fetch_orders(&access).await.map_err(map_food_error)
            }
            result => result.map_err(map_food_error),
        }
    }
    async fn refresh_food_token(
        &self,
        user_id: Uuid,
        connector: &str,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<String, FreshConnectionError> {
        let aad = format!("{user_id}:{connector}");
        let refresh: Vec<u8> = conn
            .try_get("refresh_ciphertext")
            .map_err(|_| FreshConnectionError::Unauthorized)?;
        let refresh = self.cipher()?.open(aad.as_bytes(), &refresh)?;
        let metadata: Value = conn.get("metadata");
        let client_id = metadata
            .get("client_id")
            .and_then(Value::as_str)
            .ok_or(FreshConnectionError::Unauthorized)?;
        let tokens = self
            .food_client(connector)?
            .refresh(client_id, &refresh)
            .await
            .map_err(map_food_error)?;
        let access = self.cipher()?.seal(aad.as_bytes(), &tokens.access_token)?;
        let refresh = tokens
            .refresh_token
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| self.cipher()?.seal(aad.as_bytes(), t))
            .transpose()?;
        self.store_rotated(
            conn,
            conn.get("id"),
            access,
            refresh,
            Utc::now() + Duration::seconds(tokens.expires_in),
        )
        .await?;
        Ok(tokens.access_token)
    }

    async fn exchange_playstation_npsso(
        &self,
        npsso: &str,
    ) -> Result<(String, String, String, String, i64), FreshConnectionError> {
        let verified = self
            .psn
            .link(npsso)
            .await
            .map_err(|_| FreshConnectionError::Unauthorized)?;
        Ok((
            verified.account_id,
            verified.online_id,
            verified.tokens.access_token,
            verified.tokens.refresh_token,
            verified.tokens.expires_in,
        ))
    }
    async fn refresh_playstation_tokens(
        &self,
        refresh: &str,
    ) -> Result<(String, String, i64), FreshConnectionError> {
        let tokens = self.psn.refresh(refresh).await.map_err(map_psn_error)?;
        Ok((tokens.access_token, tokens.refresh_token, tokens.expires_in))
    }
    async fn fetch_playstation_games(
        &self,
        access: &str,
        _account: &str,
    ) -> Result<Vec<PlayStationGame>, FreshConnectionError> {
        self.psn
            .games(access)
            .await
            .map_err(|_| FreshConnectionError::Provider("PlayStation activity unavailable".into()))
    }
    async fn fetch_playstation_baseline(
        &self,
        access_token: &str,
        account_id: &str,
    ) -> Result<BTreeMap<String, GameSnapshot>, FreshConnectionError> {
        let games = self
            .fetch_playstation_games(access_token, account_id)
            .await?;
        let now = Utc::now();
        let mut map = BTreeMap::new();
        for g in games {
            map.insert(
                g.title_id.clone(),
                GameSnapshot {
                    total_seconds: g.play_duration_seconds,
                    observed_at: now,
                    last_played_at: g.last_played_at,
                    name: g.name,
                    platform: g.platform,
                },
            );
        }
        Ok(map)
    }

    fn google_redirect(&self) -> Result<String, FreshConnectionError> {
        let base = self.core_api_url.as_deref().ok_or_else(|| {
            FreshConnectionError::NotConfigured("Core callback URL is missing".into())
        })?;
        let url = url::Url::parse(base)
            .map_err(|_| FreshConnectionError::NotConfigured("Invalid Core URL".into()))?;
        if url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1")))
        {
            return Err(FreshConnectionError::NotConfigured(
                "Core callback must use HTTPS".into(),
            ));
        }
        Ok(format!(
            "{}/v1/connectors/google/callback",
            base.trim_end_matches('/')
        ))
    }
    pub async fn cancel_setup(
        &self,
        user_id: Uuid,
        setup_id: Uuid,
    ) -> Result<(), FreshConnectionError> {
        sqlx::query("UPDATE vox_connection_setups SET status='cancelled',error='authorization_cancelled',verifier_ciphertext=NULL WHERE id=$1 AND user_id=$2 AND status IN ('pending','exchanging')").bind(setup_id).bind(user_id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn fail_google_setup(&self, state: &str) -> Result<(), FreshConnectionError> {
        sqlx::query("UPDATE vox_connection_setups SET status='failed',error='authorization_failed',verifier_ciphertext=NULL WHERE state_token=$1 AND status='pending'").bind(state).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn assistant_read(
        &self,
        user_id: Uuid,
        connector: &str,
        limit: usize,
    ) -> Result<Value, FreshConnectionError> {
        self.cipher()?;
        let conn=sqlx::query("UPDATE vox_connections SET lease_token=$3,lease_until=now()+interval '15 minutes' WHERE user_id=$1 AND connector_id=$2 AND assistant_read AND authorization_state='authorized' AND consented_at IS NOT NULL AND (lease_until IS NULL OR lease_until<=now()) RETURNING *")
            .bind(user_id).bind(connector).bind(Uuid::new_v4()).fetch_optional(&self.pool).await?.ok_or(FreshConnectionError::Unauthorized)?;
        let id: Uuid = conn.get("id");
        let result = self
            .read_claimed(user_id, connector, limit.clamp(1, 100), &conn)
            .await;
        if let Err(ref error) = result {
            self.release_failure(id, &conn, error).await?;
        }
        result
    }
    async fn read_claimed(
        &self,
        user_id: Uuid,
        connector: &str,
        limit: usize,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<Value, FreshConnectionError> {
        let id: Uuid = conn.get("id");
        let now = Utc::now();
        if matches!(connector, "spotify" | "youtube" | "youtube_history") {
            return self.read_personal_claimed(user_id, id, conn).await;
        }
        let mut items = if matches!(connector, "swiggy" | "zomato") {
            serde_json::to_value(self.food_orders_claimed(user_id, connector, conn).await?)
                .map_err(|_| FreshConnectionError::Provider("Invalid food response".into()))?
        } else {
            let aad = format!("{user_id}:{connector}");
            let stored: Vec<u8> = conn
                .try_get("access_ciphertext")
                .map_err(|_| FreshConnectionError::Unauthorized)?;
            let mut access = self
                .cipher()?
                .open(aad.as_bytes(), &stored)
                .map_err(|_| FreshConnectionError::Crypto)?;
            if conn
                .get::<Option<DateTime<Utc>>, _>("access_expires_at")
                .is_none_or(|t| t <= Utc::now() + Duration::seconds(60))
            {
                let stored: Vec<u8> = conn
                    .try_get("refresh_ciphertext")
                    .map_err(|_| FreshConnectionError::Unauthorized)?;
                let refresh = self
                    .cipher()?
                    .open(aad.as_bytes(), &stored)
                    .map_err(|_| FreshConnectionError::Crypto)?;
                let (next, rotated, expiry) = match connector {
                    "google_calendar" => {
                        let tokens = self
                            .google
                            .refresh(
                                self.google_client_id
                                    .as_deref()
                                    .ok_or(FreshConnectionError::Unauthorized)?,
                                self.google_client_secret
                                    .as_deref()
                                    .ok_or(FreshConnectionError::Unauthorized)?,
                                &refresh,
                            )
                            .await
                            .map_err(map_google_error)?;
                        (
                            tokens.access_token,
                            tokens.refresh_token.unwrap_or(refresh),
                            tokens.expires_in,
                        )
                    }
                    "playstation" => self.refresh_playstation_tokens(&refresh).await?,
                    _ => return Err(FreshConnectionError::NotFound),
                };
                self.store_rotated(
                    conn,
                    id,
                    self.cipher()?
                        .seal(aad.as_bytes(), &next)
                        .map_err(|_| FreshConnectionError::Crypto)?,
                    Some(
                        self.cipher()?
                            .seal(aad.as_bytes(), &rotated)
                            .map_err(|_| FreshConnectionError::Crypto)?,
                    ),
                    Utc::now() + Duration::seconds(expiry),
                )
                .await?;
                access = next;
            }
            let items = match connector {
                "google_calendar" => serde_json::to_value(
                    self.google
                        .events(&access, now - Duration::days(7), now + Duration::days(30))
                        .await
                        .map_err(|_| {
                            FreshConnectionError::Provider("Calendar read failed".into())
                        })?,
                )
                .map_err(|_| FreshConnectionError::Invalid("Invalid calendar response".into()))?,
                "playstation" => serde_json::to_value(
                    self.fetch_playstation_games(
                        &access,
                        conn.get::<String, _>("account_id").as_str(),
                    )
                    .await?,
                )
                .map_err(|_| FreshConnectionError::Invalid("Invalid gaming response".into()))?,
                _ => return Err(FreshConnectionError::NotFound),
            };
            items
        };
        let list = items
            .as_array_mut()
            .ok_or(FreshConnectionError::Invalid("Invalid response".into()))?;
        let complete = list.len() <= limit && !matches!(connector, "swiggy" | "zomato");
        list.truncate(limit);
        let mut tx = self.pool.begin().await?;
        let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM vox_connections WHERE id=$1 AND user_id=$2 AND generation=$3 AND lease_token=$4 AND lease_until>now() AND assistant_read AND authorization_state='authorized' FOR UPDATE").bind(id).bind(user_id).bind(conn.get::<Uuid,_>("generation")).bind(conn.get::<Uuid,_>("lease_token")).fetch_optional(&mut *tx).await?;
        if valid.is_none() {
            return Err(FreshConnectionError::Unauthorized);
        }
        sqlx::query("UPDATE vox_connections SET lease_token=NULL,lease_until=NULL WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(
            json!({"connector_id":connector,"observed_at":now,"complete":complete,"limit":limit,"items":items,"session_times_known":connector!="playstation"}),
        )
    }

    async fn release_failure(
        &self,
        id: Uuid,
        conn: &sqlx::postgres::PgRow,
        error: &FreshConnectionError,
    ) -> Result<(), FreshConnectionError> {
        let code = if matches!(error, FreshConnectionError::Unauthorized) {
            "reconnect_required"
        } else {
            "sync_failed"
        };
        sqlx::query("UPDATE vox_connections SET lease_token=NULL,lease_until=NULL,failure_code=CASE WHEN generation=$3 THEN $1 ELSE failure_code END,failure_count=CASE WHEN generation=$3 THEN failure_count+1 ELSE failure_count END,next_sync_at=CASE WHEN generation=$3 THEN now()+interval '10 minutes' ELSE next_sync_at END,authorization_state=CASE WHEN generation=$3 AND $1='reconnect_required' THEN 'expired' ELSE authorization_state END WHERE id=$2 AND lease_token=$4")
            .bind(code).bind(id).bind(conn.get::<Uuid,_>("generation")).bind(conn.get::<Uuid,_>("lease_token")).execute(&self.pool).await?;
        Ok(())
    }
    async fn claim(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<sqlx::postgres::PgRow, FreshConnectionError> {
        self.cipher()?;
        sqlx::query("UPDATE vox_connections SET lease_token=$3, lease_until=now()+interval '15 minutes' WHERE id=$1 AND user_id=$2 AND authorization_state='authorized' AND sync_timeline AND consented_at IS NOT NULL AND (lease_until IS NULL OR lease_until<=now()) RETURNING *")
            .bind(id).bind(user_id).bind(Uuid::new_v4()).fetch_optional(&self.pool).await?.ok_or_else(|| FreshConnectionError::Invalid("Connection paused, unavailable, or already syncing".into()))
    }
    async fn validate_commit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: Uuid,
        id: Uuid,
        claimed: &sqlx::postgres::PgRow,
    ) -> Result<(), FreshConnectionError> {
        let valid: Option<Uuid> = sqlx::query_scalar("SELECT id FROM vox_connections WHERE id=$1 AND user_id=$2 AND generation=$3 AND lease_token=$4 AND lease_until>now() AND authorization_state='authorized' AND sync_timeline AND consented_at IS NOT NULL FOR UPDATE")
            .bind(id).bind(user_id).bind(claimed.get::<Uuid,_>("generation")).bind(claimed.get::<Uuid,_>("lease_token")).fetch_optional(&mut **tx).await?;
        if valid.is_none() {
            return Err(FreshConnectionError::Unauthorized);
        }
        Ok(())
    }
    async fn store_rotated(
        &self,
        claimed: &sqlx::postgres::PgRow,
        id: Uuid,
        access: Vec<u8>,
        refresh: Option<Vec<u8>>,
        expires: DateTime<Utc>,
    ) -> Result<(), FreshConnectionError> {
        let result=sqlx::query("UPDATE vox_connections SET access_ciphertext=$1,refresh_ciphertext=COALESCE($2,refresh_ciphertext),access_expires_at=$3 WHERE id=$4 AND credential_generation=$5 AND lease_token=$6 AND lease_until>now() AND authorization_state='authorized'")
            .bind(access).bind(refresh).bind(expires).bind(id).bind(claimed.get::<Uuid,_>("credential_generation")).bind(claimed.get::<Uuid,_>("lease_token")).execute(&self.pool).await?;
        if result.rows_affected() != 1 {
            return Err(FreshConnectionError::Unauthorized);
        }
        Ok(())
    }

    async fn start_personal_oauth(
        &self,
        user_id: Uuid,
        connector: &str,
    ) -> Result<StartConnectionResponse, FreshConnectionError> {
        let redirect = self
            .google_redirect()?
            .replace("/google/callback", &format!("/{connector}/callback"));
        let state = crate::crypto::random_token()?;
        let verifier = crate::crypto::random_token()?;
        let authorization_url = self
            .personal
            .auth_url(connector, &redirect, &state, &verifier)?;
        let sealed = self.cipher()?.seal(state.as_bytes(), &verifier)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user_id}:{connector}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE vox_connection_setups SET status='cancelled',verifier_ciphertext=NULL WHERE user_id=$1 AND connector_id=$2 AND status IN ('pending','exchanging')").bind(user_id).bind(connector).execute(&mut *tx).await?;
        let setup_id=sqlx::query_scalar("INSERT INTO vox_connection_setups(user_id,connector_id,state_token,verifier_ciphertext,redirect_uri,consented_at) VALUES($1,$2,$3,$4,$5,now()) RETURNING id").bind(user_id).bind(connector).bind(state).bind(sealed).bind(redirect).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(StartConnectionResponse {
            setup_id: Some(setup_id),
            connection_id: None,
            authorization_url: Some(authorization_url),
            status: "pending".into(),
        })
    }
    pub async fn handle_personal_callback(
        &self,
        connector: &str,
        code: Option<&str>,
        state: &str,
    ) -> Result<Uuid, FreshConnectionError> {
        if !self.personal.enabled(connector) {
            return Err(FreshConnectionError::NotConfigured(
                "Provider disabled".into(),
            ));
        }
        let setup=sqlx::query("UPDATE vox_connection_setups SET status='exchanging' WHERE connector_id=$1 AND state_token=$2 AND status='pending' AND expires_at>now() AND consented_at IS NOT NULL RETURNING *").bind(connector).bind(state).fetch_optional(&self.pool).await?.ok_or(FreshConnectionError::Unauthorized)?;
        let result = self
            .complete_personal_callback(connector, code, state, &setup)
            .await;
        if result.is_err() {
            sqlx::query("UPDATE vox_connection_setups SET status='failed',error='authorization_failed',verifier_ciphertext=NULL WHERE id=$1 AND status='exchanging'").bind(setup.get::<Uuid,_>("id")).execute(&self.pool).await?;
        }
        result
    }
    async fn complete_personal_callback(
        &self,
        connector: &str,
        code: Option<&str>,
        state: &str,
        setup: &sqlx::postgres::PgRow,
    ) -> Result<Uuid, FreshConnectionError> {
        let user_id: Uuid = setup.get("user_id");
        let setup_id: Uuid = setup.get("id");
        let redirect: String = setup.get("redirect_uri");
        let stored: Vec<u8> = setup.get("verifier_ciphertext");
        let verifier = self.cipher()?.open(state.as_bytes(), &stored)?;
        let tokens = self
            .personal
            .tokens(connector, code, &redirect, &verifier, None)
            .await?;
        let access = crate::providers::personal::StoredAccess {
            token: tokens.access_token,
        };
        let snapshot = self.personal.snapshot(connector, &access).await?;
        let aad = format!("{user_id}:{connector}");
        let sealed = self.cipher()?.seal(
            aad.as_bytes(),
            &serde_json::to_string(&access).map_err(|_| FreshConnectionError::Crypto)?,
        )?;
        let refresh = self.cipher()?.seal(
            aad.as_bytes(),
            tokens
                .refresh_token
                .as_deref()
                .ok_or(FreshConnectionError::Unauthorized)?,
        )?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user_id}:{connector}"))
            .execute(&mut *tx)
            .await?;
        let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM vox_connection_setups WHERE id=$1 AND status='exchanging' AND expires_at>now() FOR UPDATE").bind(setup_id).fetch_optional(&mut *tx).await?;
        if valid.is_none() {
            return Err(FreshConnectionError::Unauthorized);
        }
        let id:Uuid=sqlx::query_scalar("INSERT INTO vox_connections(user_id,connector_id,account_id,account_display_id,access_ciphertext,refresh_ciphertext,access_expires_at,authorization_state,sync_timeline,assistant_read,metadata,consented_at) VALUES($1,$2,$3,$4,$5,$6,$7,'authorized',true,true,$8,now()) ON CONFLICT(user_id,connector_id) DO UPDATE SET generation=gen_random_uuid(),credential_generation=gen_random_uuid(),lease_token=NULL,lease_until=NULL,account_id=EXCLUDED.account_id,account_display_id=EXCLUDED.account_display_id,access_ciphertext=EXCLUDED.access_ciphertext,refresh_ciphertext=EXCLUDED.refresh_ciphertext,access_expires_at=EXCLUDED.access_expires_at,authorization_state='authorized',sync_timeline=true,assistant_read=true,metadata=EXCLUDED.metadata,consented_at=now(),failure_code=NULL,failure_count=0,updated_at=now() RETURNING id")
            .bind(user_id).bind(connector).bind(&snapshot.account_id).bind(&snapshot.display_id).bind(sealed).bind(refresh).bind(Utc::now()+Duration::seconds(tokens.expires_in)).bind(json!({"auth_type":"oauth2","snapshot":snapshot.context})).fetch_one(&mut *tx).await?;
        self.ingestor
            .personal_activity(&mut tx, user_id, id, connector, &snapshot.activities)
            .await?;
        sqlx::query("UPDATE vox_connections SET last_synced_at=now(),next_sync_at=now()+interval '15 minutes' WHERE id=$1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE vox_connection_setups SET status='authorized',connection_id=$2,verifier_ciphertext=NULL WHERE id=$1").bind(setup_id).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }
    async fn personal_access(
        &self,
        user_id: Uuid,
        connector: &str,
        conn: &sqlx::postgres::PgRow,
        force_refresh: bool,
    ) -> Result<crate::providers::personal::StoredAccess, FreshConnectionError> {
        if conn.get::<String, _>("connector_id") != connector || !self.personal.enabled(connector) {
            return Err(FreshConnectionError::Unauthorized);
        }
        let aad = format!("{user_id}:{connector}");
        let stored: Vec<u8> = conn
            .try_get("access_ciphertext")
            .map_err(|_| FreshConnectionError::Unauthorized)?;
        let mut access: crate::providers::personal::StoredAccess =
            serde_json::from_str(&self.cipher()?.open(aad.as_bytes(), &stored)?)
                .map_err(|_| FreshConnectionError::Crypto)?;
        if force_refresh
            || conn
                .get::<Option<DateTime<Utc>>, _>("access_expires_at")
                .is_none_or(|exp| exp <= Utc::now() + Duration::seconds(60))
        {
            let stored: Vec<u8> = conn
                .try_get("refresh_ciphertext")
                .map_err(|_| FreshConnectionError::Unauthorized)?;
            let refresh = self.cipher()?.open(aad.as_bytes(), &stored)?;
            let tokens = self
                .personal
                .tokens(connector, None, "", "", Some(&refresh))
                .await?;
            access.token = tokens.access_token;
            let sealed = self.cipher()?.seal(
                aad.as_bytes(),
                &serde_json::to_string(&access).map_err(|_| FreshConnectionError::Crypto)?,
            )?;
            let refresh = tokens
                .refresh_token
                .as_deref()
                .filter(|v| !v.is_empty())
                .map(|token| self.cipher()?.seal(aad.as_bytes(), token))
                .transpose()?;
            self.store_rotated(
                conn,
                conn.get("id"),
                sealed,
                refresh,
                Utc::now() + Duration::seconds(tokens.expires_in),
            )
            .await?;
        }
        Ok(access)
    }
    async fn personal_snapshot(
        &self,
        user_id: Uuid,
        connector: &str,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<crate::providers::personal::Snapshot, FreshConnectionError> {
        let proactively_refreshed = conn
            .get::<Option<DateTime<Utc>>, _>("access_expires_at")
            .is_none_or(|exp| exp <= Utc::now() + Duration::seconds(60));
        let access = self
            .personal_access(user_id, connector, conn, false)
            .await?;
        let result = self.personal.snapshot(connector, &access).await;
        let snapshot = if !proactively_refreshed
            && matches!(result, Err(FreshConnectionError::Unauthorized))
        {
            let access = self.personal_access(user_id, connector, conn, true).await?;
            self.personal.snapshot(connector, &access).await?
        } else {
            result?
        };
        if conn.get::<Option<String>, _>("account_id").as_deref() != Some(&snapshot.account_id) {
            return Err(FreshConnectionError::Unauthorized);
        }
        Ok(snapshot)
    }
    pub async fn sync_personal(
        &self,
        user_id: Uuid,
        id: Uuid,
        connector: &str,
    ) -> Result<usize, FreshConnectionError> {
        let conn = self.claim(user_id, id).await?;
        let result = self
            .sync_personal_claimed(user_id, id, connector, &conn)
            .await;
        if let Err(ref error) = result {
            self.release_failure(id, &conn, error).await?;
        }
        result
    }
    async fn sync_personal_claimed(
        &self,
        user_id: Uuid,
        id: Uuid,
        connector: &str,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<usize, FreshConnectionError> {
        let snapshot = self.personal_snapshot(user_id, connector, conn).await?;
        let mut tx = self.pool.begin().await?;
        self.validate_commit(&mut tx, user_id, id, conn).await?;
        let count = self
            .ingestor
            .personal_activity(&mut tx, user_id, id, connector, &snapshot.activities)
            .await?;
        sqlx::query("UPDATE vox_connections SET metadata=jsonb_set(metadata,'{snapshot}',$2),last_synced_at=now(),next_sync_at=now()+interval '15 minutes',lease_token=NULL,lease_until=NULL,failure_count=0,failure_code=NULL,updated_at=now() WHERE id=$1").bind(id).bind(snapshot.context).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(count)
    }
    pub async fn read_personal(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<Value, FreshConnectionError> {
        let conn=sqlx::query("UPDATE vox_connections SET lease_token=$3,lease_until=now()+interval '15 minutes' WHERE id=$1 AND user_id=$2 AND assistant_read AND authorization_state='authorized' AND consented_at IS NOT NULL AND connector_id IN ('spotify','youtube','youtube_history') AND (lease_until IS NULL OR lease_until<=now()) RETURNING *").bind(id).bind(user_id).bind(Uuid::new_v4()).fetch_optional(&self.pool).await?.ok_or(FreshConnectionError::Unauthorized)?;
        let result = self.read_personal_claimed(user_id, id, &conn).await;
        if let Err(ref error) = result {
            self.release_failure(id, &conn, error).await?;
        }
        result
    }
    async fn read_personal_claimed(
        &self,
        user_id: Uuid,
        id: Uuid,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<Value, FreshConnectionError> {
        let connector: String = conn.get("connector_id");
        let data = if connector == "youtube_history" {
            conn.get::<Value, _>("metadata")["snapshot"].clone()
        } else {
            self.personal_snapshot(user_id, &connector, conn)
                .await?
                .context
        };
        let mut tx = self.pool.begin().await?;
        self.validate_personal_read(&mut tx, user_id, id, conn)
            .await?;
        sqlx::query("UPDATE vox_connections SET lease_token=NULL,lease_until=NULL WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(json!({"connector_id":connector,"observed_at":Utc::now(),"complete":false,"data":data}))
    }
    async fn validate_personal_read(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: Uuid,
        id: Uuid,
        conn: &sqlx::postgres::PgRow,
    ) -> Result<(), FreshConnectionError> {
        let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM vox_connections WHERE id=$1 AND user_id=$2 AND generation=$3 AND lease_token=$4 AND lease_until>now() AND assistant_read AND authorization_state='authorized' AND consented_at IS NOT NULL FOR UPDATE").bind(id).bind(user_id).bind(conn.get::<Uuid,_>("generation")).bind(conn.get::<Uuid,_>("lease_token")).fetch_optional(&mut **tx).await?;
        if valid.is_none() {
            return Err(FreshConnectionError::Unauthorized);
        }
        Ok(())
    }
    pub async fn import_youtube_history(
        &self,
        user_id: Uuid,
        history: Value,
        consent: bool,
    ) -> Result<Value, FreshConnectionError> {
        if !consent {
            return Err(FreshConnectionError::Invalid(
                "Import consent required".into(),
            ));
        }
        let (items, skipped) = crate::providers::personal::parse_youtube_history(&history)?;
        if items.is_empty() {
            return Err(FreshConnectionError::Invalid(
                "No valid YouTube watch records found".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("setup:{user_id}:youtube_history"))
            .execute(&mut *tx)
            .await?;
        let id:Uuid=sqlx::query_scalar("INSERT INTO vox_connections(user_id,connector_id,account_id,account_display_id,authorization_state,sync_timeline,assistant_read,metadata,consented_at,last_synced_at) VALUES($1,'youtube_history',$2,'Google Takeout watch history','authorized',true,true,$3,now(),now()) ON CONFLICT(user_id,connector_id) DO UPDATE SET generation=gen_random_uuid(),lease_token=NULL,lease_until=NULL,authorization_state='authorized',sync_timeline=true,assistant_read=true,metadata=EXCLUDED.metadata,consented_at=now(),last_synced_at=now(),updated_at=now() RETURNING id").bind(user_id).bind(user_id.to_string()).bind(json!({"auth_type":"import","snapshot":{"watch_history":items.iter().rev().take(100).collect::<Vec<_>>(),"source":"google_takeout","complete":false}})).fetch_one(&mut *tx).await?;
        let imported = self
            .ingestor
            .personal_activity(&mut tx, user_id, id, "youtube_history", &items)
            .await?;
        tx.commit().await?;
        Ok(json!({"imported":imported,"skipped":skipped,"complete":false,"connection_id":id}))
    }

    pub async fn run_due_syncs(&self) -> Result<(), FreshConnectionError> {
        let rows = sqlx::query(
            "SELECT id, user_id, connector_id FROM vox_connections \
             WHERE authorization_state = 'authorized' AND sync_timeline AND next_sync_at <= now() AND (lease_until IS NULL OR lease_until <= now()) AND connector_id IN ('google_calendar','playstation','swiggy','zomato','spotify','youtube') ORDER BY next_sync_at \
             LIMIT 20",
        )
        .fetch_all(&self.pool)
        .await?;

        for r in rows {
            let id: Uuid = r.get("id");
            let user_id: Uuid = r.get("user_id");
            let connector_id: String = r.get("connector_id");

            let result = match connector_id.as_str() {
                "google_calendar" => self.refresh(user_id, id).await.map(|_| ()),
                "playstation" | "swiggy" | "zomato" | "spotify" | "youtube" => {
                    self.refresh(user_id, id).await.map(|_| ())
                }
                _ => Ok(()),
            };

            if result.is_err() {
                tracing::warn!(connection_id=%id, "connection sync did not complete");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;

fn map_google_error(
    error: crate::providers::google_calendar::GoogleCalendarError,
) -> FreshConnectionError {
    match error {
        crate::providers::google_calendar::GoogleCalendarError::Unauthorized => {
            FreshConnectionError::Unauthorized
        }
        _ => FreshConnectionError::Provider("Google request failed".into()),
    }
}
fn map_psn_error(error: crate::providers::playstation::PlayStationError) -> FreshConnectionError {
    match error {
        crate::providers::playstation::PlayStationError::ReconnectRequired => {
            FreshConnectionError::Unauthorized
        }
        _ => FreshConnectionError::Provider("PSN request failed".into()),
    }
}

fn map_food_error(
    error: crate::providers::food_delivery::FoodDeliveryError,
) -> FreshConnectionError {
    match error {
        crate::providers::food_delivery::FoodDeliveryError::Unauthorized => {
            FreshConnectionError::Unauthorized
        }
        _ => FreshConnectionError::Provider(error.to_string()),
    }
}
fn food_sync_interval(orders: &[crate::providers::food_delivery::FoodDeliveryOrder]) -> i64 {
    if orders.iter().any(|o| {
        o.status.is_active()
            || o.provider_data
                .get("isActiveOrder")
                .and_then(Value::as_bool)
                == Some(true)
    }) {
        orders
            .iter()
            .filter_map(|o| o.provider_data.pointer("/tracking/pollingIntervalSeconds"))
            .filter_map(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
            })
            .max()
            .unwrap_or(60)
            .clamp(60, 86400)
    } else {
        900
    }
}
