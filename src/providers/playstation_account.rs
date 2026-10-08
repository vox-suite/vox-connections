use super::playstation::{
    PlayStationError, PlayStationGame, PlayStationService, parse_titles_from_json,
};
use crate::{
    connected_apps::crypto::CredentialCipher, connections::Connection,
    connections::ConnectionService, identity::RequestContext,
    integration_registry::IntegrationRegistry,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::collections::BTreeMap;

const AUTH: &str = "https://ca.account.sony.com/api/authz/v3/oauth";
const API: &str = "https://m.np.playstation.com";
const REDIRECT: &str = "com.scee.psxandroid.scecompcall://redirect";
const CLIENT_ID: &str = "09515159-7237-4370-9b40-3806e67c0891";
const CLIENT_AUTH: &str =
    "Basic MDk1MTUxNTktNzIzNy00MzcwLTliNDAtMzgwNmU2N2MwODkxOnVjUGprYTV0bnRCMktxc1A=";

#[derive(Clone)]
pub struct PlayStationAccounts {
    pool: PgPool,
    cipher: CredentialCipher,
    http: reqwest::Client,
    auth_origin: String,
    api_origin: String,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    refresh_token_expires_in: i64,
    id_token: Option<String>,
    scope: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GameSnapshot {
    pub total_seconds: u64,
    pub observed_at: DateTime<Utc>,
    pub last_played_at: DateTime<Utc>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObservedActivity {
    pub game: PlayStationGame,
    pub duration_seconds: u64,
    pub observation_start: DateTime<Utc>,
    pub observation_end: DateTime<Utc>,
    pub source_ref: String,
}

#[derive(Serialize)]
pub struct CaptureStatus {
    pub connection_id: uuid::Uuid,
    pub account_id: String,
    pub capture_enabled: bool,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub next_sync_at: DateTime<Utc>,
    pub failure_code: Option<String>,
    pub games: Value,
}

pub fn observed_activity(
    account_id: &str,
    previous: &BTreeMap<String, GameSnapshot>,
    games: &[PlayStationGame],
    baseline_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Vec<ObservedActivity> {
    games
        .iter()
        .filter_map(|game| {
            let new_game;
            let old = if let Some(old) = previous.get(&game.title_id) {
                old
            } else {
                let baseline = baseline_at?;
                if game
                    .first_played_at
                    .is_none_or(|first| first < baseline || first > now)
                {
                    return None;
                }
                new_game = GameSnapshot {
                    total_seconds: 0,
                    observed_at: baseline,
                    last_played_at: game.last_played_at,
                    name: game.name.clone(),
                    platform: game.platform.clone(),
                };
                &new_game
            };
            let delta = game.play_duration_seconds.checked_sub(old.total_seconds)?;
            if delta == 0 || now <= old.observed_at {
                return None;
            }
            Some(ObservedActivity {
                game: game.clone(),
                duration_seconds: delta,
                observation_start: old.observed_at,
                observation_end: now,
                source_ref: format!(
                    "{account_id}:{}:{}:{}",
                    game.title_id, old.total_seconds, game.play_duration_seconds
                ),
            })
        })
        .collect()
}

fn aad(id: uuid::Uuid, kind: &str) -> Vec<u8> {
    format!("psn:{id}:{kind}").into_bytes()
}
fn secret_error(_: crate::connected_apps::ConnectedAppError) -> PlayStationError {
    PlayStationError::NotConfigured
}

impl PlayStationAccounts {
    pub fn new(pool: PgPool, key: &str) -> Result<Self, PlayStationError> {
        Ok(Self {
            pool,
            cipher: CredentialCipher::from_hex_key(key).map_err(secret_error)?,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| PlayStationError::NotConfigured)?,
            auth_origin: AUTH.into(),
            api_origin: API.into(),
        })
    }

    async fn checked(resp: reqwest::Response) -> Result<reqwest::Response, PlayStationError> {
        match resp.status().as_u16() {
            401 | 403 => Err(PlayStationError::ReconnectRequired),
            429 => Err(PlayStationError::RateLimited(
                resp.headers()
                    .get("retry-after")
                    .and_then(|s| s.to_str().ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60)
                    .clamp(1, 86400),
            )),
            200..=299 => Ok(resp),
            _ => Err(PlayStationError::ProviderError("PSN request failed".into())),
        }
    }

    async fn exchange(&self, params: &[(&str, &str)]) -> Result<Tokens, PlayStationError> {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params.iter().copied())
            .finish();
        let response = self
            .http
            .post(format!("{}/token", self.auth_origin))
            .header("Authorization", CLIENT_AUTH)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(|_| PlayStationError::ProviderError("PSN unavailable".into()))?;
        if response.status().as_u16() == 400 {
            return Err(PlayStationError::ReconnectRequired);
        }
        let tokens: Tokens = Self::checked(response)
            .await?
            .json()
            .await
            .map_err(|_| PlayStationError::Invalid)?;
        if tokens.access_token.is_empty()
            || tokens.refresh_token.is_empty()
            || !(1..=86400).contains(&tokens.expires_in)
            || !(1..=31_536_000).contains(&tokens.refresh_token_expires_in)
            || !tokens
                .scope
                .split_whitespace()
                .any(|s| s == "psn:mobile.v2.core")
        {
            return Err(PlayStationError::Invalid);
        }
        Ok(tokens)
    }

    pub async fn link(
        &self,
        context: &RequestContext,
        npsso: &str,
        capture: bool,
    ) -> Result<Connection, PlayStationError> {
        if npsso.len() != 64
            || !npsso
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(PlayStationError::Invalid);
        }
        let response = self
            .http
            .get(format!("{}/authorize", self.auth_origin))
            .query(&[
                ("access_type", "offline"),
                ("client_id", CLIENT_ID),
                ("redirect_uri", REDIRECT),
                ("response_type", "code"),
                ("scope", "psn:mobile.v2.core psn:clientapp"),
            ])
            .header("Cookie", format!("npsso={npsso}"))
            .send()
            .await
            .map_err(|_| PlayStationError::ProviderError("PSN unavailable".into()))?;
        if !response.status().is_redirection() {
            return Err(PlayStationError::ReconnectRequired);
        }
        let redirect = response
            .headers()
            .get("location")
            .and_then(|s| s.to_str().ok())
            .and_then(|s| url::Url::parse(s).ok())
            .ok_or(PlayStationError::ReconnectRequired)?;
        if redirect.scheme() != "com.scee.psxandroid.scecompcall"
            || redirect.host_str() != Some("redirect")
        {
            return Err(PlayStationError::Invalid);
        }
        let code = redirect
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.to_string())
            .ok_or(PlayStationError::ReconnectRequired)?;
        let tokens = self
            .exchange(&[
                ("code", &code),
                ("redirect_uri", REDIRECT),
                ("grant_type", "authorization_code"),
                ("token_format", "jwt"),
            ])
            .await?;
        let account_id = account_subject(
            tokens
                .id_token
                .as_deref()
                .ok_or(PlayStationError::Invalid)?,
        )?;
        let profile = self
            .http
            .get(format!(
                "{}/api/userProfile/v1/internal/users/{}/profiles",
                self.api_origin, account_id
            ))
            .bearer_auth(&tokens.access_token)
            .send()
            .await
            .map_err(|_| PlayStationError::ProviderError("PSN unavailable".into()))?;
        let profile: Value = Self::checked(profile)
            .await?
            .json()
            .await
            .map_err(|_| PlayStationError::Invalid)?;
        if profile.get("isMe").and_then(Value::as_bool) != Some(true) {
            return Err(PlayStationError::Invalid);
        }
        let online_id = profile
            .get("onlineId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .ok_or(PlayStationError::Invalid)?;
        self.games(&tokens.access_token).await?;
        let deployment_key: String =
            sqlx::query_scalar("SELECT external_key FROM platform_deployments WHERE id=$1")
                .bind(context.subject.deployment_id.0)
                .fetch_one(&self.pool)
                .await?;
        let existing: Option<String> = sqlx::query_scalar("SELECT state FROM integration_definitions WHERE deployment_id=$1 AND external_key='playstation'")
            .bind(context.subject.deployment_id.0).fetch_optional(&self.pool).await?;
        if existing.as_deref() == Some("disabled") {
            return Err(PlayStationError::Invalid);
        }
        IntegrationRegistry::new(self.pool.clone())
            .register(PlayStationService::integration_declaration(&deployment_key))
            .await
            .map_err(|_| PlayStationError::Invalid)?;
        let mut tx = self.pool.begin().await?;
        let integration_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM integration_definitions WHERE deployment_id=$1 AND external_key='playstation' FOR UPDATE").bind(context.subject.deployment_id.0).fetch_one(&mut *tx).await?;
        if existing.is_none() {
            sqlx::query("UPDATE integration_definitions SET state='enabled' WHERE id=$1")
                .bind(integration_id)
                .execute(&mut *tx)
                .await?;
        } else {
            let enabled: bool = sqlx::query_scalar(
                "SELECT state='enabled' FROM integration_definitions WHERE id=$1",
            )
            .bind(integration_id)
            .fetch_one(&mut *tx)
            .await?;
            if !enabled {
                return Err(PlayStationError::Invalid);
            }
        }
        let id: uuid::Uuid = sqlx::query_scalar("INSERT INTO external_connections(user_context_id,integration_id,external_account_hash,account_display_id,credential_custody,authorization_state,authorized_capabilities) VALUES($1,$2,$3,$4,'platform_held','authorized',ARRAY['playstation.game_activity','playstation.recently_played']) ON CONFLICT(user_context_id,integration_id,external_account_hash) DO UPDATE SET authorization_state='authorized',authorized_capabilities=EXCLUDED.authorized_capabilities,account_display_id=EXCLUDED.account_display_id,expires_at=NULL,failure_code=NULL,revoked_at=NULL,updated_at=now() RETURNING id").bind(context.id.0).bind(integration_id).bind(Sha256::digest(account_id.as_bytes()).to_vec()).bind(online_id).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE agent_capability_grants SET state='revoked',revoked_at=now(),updated_at=now() WHERE connection_id=$1 AND state='enabled'").bind(id).execute(&mut *tx).await?;
        self.store_tokens(&mut tx, id, &tokens, Some((&account_id, capture)))
            .await?;
        tx.commit().await?;
        ConnectionService::new(self.pool.clone())
            .get(context, id)
            .await
            .map_err(|_| PlayStationError::ConnectionNotFound)
    }

    async fn store_tokens(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: uuid::Uuid,
        tokens: &Tokens,
        linked: Option<(&str, bool)>,
    ) -> Result<(), PlayStationError> {
        let access = self
            .cipher
            .seal(&aad(id, "access"), &tokens.access_token)
            .map_err(secret_error)?;
        let refresh = self
            .cipher
            .seal(&aad(id, "refresh"), &tokens.refresh_token)
            .map_err(secret_error)?;
        let now = Utc::now();
        if let Some((account_id, capture)) = linked {
            sqlx::query("INSERT INTO playstation_accounts(connection_id,account_id,access_ciphertext,refresh_ciphertext,access_expires_at,refresh_expires_at,capture_enabled) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(connection_id) DO UPDATE SET generation=gen_random_uuid(),access_ciphertext=EXCLUDED.access_ciphertext,refresh_ciphertext=EXCLUDED.refresh_ciphertext,access_expires_at=EXCLUDED.access_expires_at,refresh_expires_at=EXCLUDED.refresh_expires_at,capture_enabled=EXCLUDED.capture_enabled,snapshots='{}',last_synced_at=NULL,next_sync_at=now(),failure_code=NULL,failure_count=0,updated_at=now()")
                .bind(id).bind(account_id).bind(access).bind(refresh).bind(now+Duration::seconds(tokens.expires_in)).bind(now+Duration::seconds(tokens.refresh_token_expires_in)).bind(capture).execute(&mut **tx).await?;
        } else {
            sqlx::query("UPDATE playstation_accounts SET access_ciphertext=$2,refresh_ciphertext=$3,access_expires_at=$4,refresh_expires_at=$5,updated_at=now() WHERE connection_id=$1").bind(id).bind(access).bind(refresh).bind(now+Duration::seconds(tokens.expires_in)).bind(now+Duration::seconds(tokens.refresh_token_expires_in)).execute(&mut **tx).await?;
        }
        Ok(())
    }

    pub async fn token_for_connection(
        &self,
        id: uuid::Uuid,
        require_capture: bool,
    ) -> Result<String, PlayStationError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT c.id FROM external_connections c JOIN playstation_accounts p ON p.connection_id=c.id JOIN integration_definitions i ON i.id=c.integration_id WHERE c.id=$1 AND c.authorization_state='authorized' AND i.state='enabled' AND 'playstation.game_activity'=ANY(c.authorized_capabilities) AND (NOT $2 OR p.capture_enabled) FOR UPDATE OF c")
            .bind(id).bind(require_capture).fetch_optional(&mut *tx).await?;
        if row.is_none() {
            return Err(PlayStationError::ConnectionNotFound);
        }
        let token = self.access_token(&mut tx, id).await?;
        tx.commit().await?;
        Ok(token)
    }

    pub async fn access_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: uuid::Uuid,
    ) -> Result<String, PlayStationError> {
        let row = sqlx::query("SELECT access_ciphertext,refresh_ciphertext,access_expires_at,refresh_expires_at FROM playstation_accounts WHERE connection_id=$1 FOR UPDATE").bind(id).fetch_optional(&mut **tx).await?.ok_or(PlayStationError::ReconnectRequired)?;
        let now = Utc::now();
        if row.get::<DateTime<Utc>, _>("refresh_expires_at") <= now {
            return Err(PlayStationError::ReconnectRequired);
        }
        if row.get::<DateTime<Utc>, _>("access_expires_at") > now + Duration::seconds(60) {
            return self
                .cipher
                .open(
                    &aad(id, "access"),
                    &row.get::<Vec<u8>, _>("access_ciphertext"),
                )
                .map_err(secret_error);
        }
        let refresh = self
            .cipher
            .open(
                &aad(id, "refresh"),
                &row.get::<Vec<u8>, _>("refresh_ciphertext"),
            )
            .map_err(secret_error)?;
        let tokens = self
            .exchange(&[
                ("refresh_token", &refresh),
                ("grant_type", "refresh_token"),
                ("token_format", "jwt"),
                ("scope", "psn:mobile.v2.core psn:clientapp"),
            ])
            .await?;
        self.store_tokens(tx, id, &tokens, None).await?;
        Ok(tokens.access_token)
    }

    pub async fn games(&self, token: &str) -> Result<Vec<PlayStationGame>, PlayStationError> {
        let mut games = Vec::new();
        let mut offset = 0u64;
        for _ in 0..20 {
            let response = self
                .http
                .get(format!(
                    "{}/api/gamelist/v2/users/me/titles",
                    self.api_origin
                ))
                .query(&[
                    ("limit", "100"),
                    ("offset", &offset.to_string()),
                    ("categories", "ps5_native_game,ps4_game"),
                ])
                .bearer_auth(token)
                .send()
                .await
                .map_err(|_| PlayStationError::ProviderError("PSN unavailable".into()))?;
            let body: Value = Self::checked(response)
                .await?
                .json()
                .await
                .map_err(|_| PlayStationError::Invalid)?;
            games.extend(parse_titles_from_json(&body)?);
            match body.get("nextOffset").and_then(Value::as_u64) {
                Some(next) if next > offset => offset = next,
                None => return Ok(games),
                _ => return Err(PlayStationError::Invalid),
            }
        }
        Err(PlayStationError::ProviderError(
            "PSN history exceeds sync limit".into(),
        ))
    }

    pub async fn status(
        &self,
        context: &RequestContext,
        id: uuid::Uuid,
    ) -> Result<CaptureStatus, PlayStationError> {
        let row=sqlx::query("SELECT p.* FROM playstation_accounts p JOIN external_connections c ON c.id=p.connection_id WHERE c.id=$1 AND c.user_context_id=$2").bind(id).bind(context.id.0).fetch_optional(&self.pool).await?.ok_or(PlayStationError::ConnectionNotFound)?;
        Ok(CaptureStatus {
            connection_id: id,
            account_id: row.get("account_id"),
            capture_enabled: row.get("capture_enabled"),
            last_synced_at: row.get("last_synced_at"),
            next_sync_at: row.get("next_sync_at"),
            failure_code: row.get("failure_code"),
            games: row.get("snapshots"),
        })
    }
}

fn account_subject(token: &str) -> Result<String, PlayStationError> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let payload = token.split('.').nth(1).ok_or(PlayStationError::Invalid)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| PlayStationError::Invalid)?;
    let body: Value = serde_json::from_slice(&decoded).map_err(|_| PlayStationError::Invalid)?;
    let sub = body
        .get("sub")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_digit()))
        .ok_or(PlayStationError::Invalid)?;
    Ok(sub.into())
}
