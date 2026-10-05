/**
 * PlayStation 5 Provider Integration and Activity Sync Worker.
 *
 * Extracts PlayStation 5 and PlayStation Network gaming activity
 * (what game was played, when it was played, duration, platform)
 * and records it into the user's Timeline as Spans.
 */
use crate::{
    capability_grants::{CapabilityGrantError, CapabilityGrantService},
    connections::{AuthorizationState, ConnectionError, ConnectionService},
    identity::RequestContext,
    integration_registry::{
        CapabilityDeclaration, CapabilityEffect, IntegrationProtocol, RegisterIntegrationRequest,
    },
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub const PLAYSTATION_INTEGRATION_KEY: &str = "playstation";
pub const PLAYSTATION_CAPABILITY_RECENTLY_PLAYED: &str = "playstation.recently_played";
pub const PLAYSTATION_CAPABILITY_GAME_ACTIVITY: &str = "playstation.game_activity";
pub const PLAYSTATION_CAPABILITY_USER_TITLES: &str = "playstation.user_titles";

pub const PLAYSTATION_CAPABILITY_RECENTLY_PLAYED_SHORT: &str = "recently_played";
pub const PLAYSTATION_CAPABILITY_GAME_ACTIVITY_SHORT: &str = "game_activity";
pub const PLAYSTATION_CAPABILITY_USER_TITLES_SHORT: &str = "user_titles";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlayStationGame {
    pub title_id: String,
    pub name: String,
    pub platform: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_played_at: Option<DateTime<Utc>>,
    pub last_played_at: Option<DateTime<Utc>>,
    pub play_duration_seconds: u64,
    pub play_count: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum PlayStationError {
    #[error("connection not found")]
    ConnectionNotFound,
    #[error("connection is not authorized for integration playstation")]
    InvalidIntegration,
    #[error("access token or npsso expired; reconnection required")]
    ReconnectRequired,
    #[error("capability {0} is not granted to agent {1}")]
    UnauthorizedCapability(String, String),
    #[error("rate limited by PlayStation Network API; retry after {0} seconds")]
    RateLimited(u64),
    #[error("provider error: {0}")]
    ProviderError(String),
    #[error("worker sync error: {0}")]
    WorkerError(String),
    #[error("PlayStation credentials are not configured")]
    NotConfigured,
    #[error("invalid PlayStation request or response")]
    Invalid,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlayStationRecentActivityResponse {
    pub games: Vec<PlayStationGame>,
    pub count: usize,
    pub freshness_seconds: u32,
}

#[async_trait::async_trait]
pub trait PlayStationProviderClient: Send + Sync {
    async fn fetch_recently_played(
        &self,
        access_token: &str,
        limit: usize,
    ) -> Result<Vec<PlayStationGame>, PlayStationError>;

    async fn fetch_user_titles(
        &self,
        access_token: &str,
        limit: usize,
    ) -> Result<Vec<PlayStationGame>, PlayStationError>;
}

pub struct DefaultPlayStationProviderClient {
    base_url: String,
    http: reqwest::Client,
}

impl DefaultPlayStationProviderClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("PSN HTTP client"),
        }
    }
}

#[async_trait::async_trait]
impl PlayStationProviderClient for DefaultPlayStationProviderClient {
    async fn fetch_recently_played(
        &self,
        access_token: &str,
        limit: usize,
    ) -> Result<Vec<PlayStationGame>, PlayStationError> {
        let effective_limit = if limit == 0 || limit > 100 { 20 } else { limit };
        let url = format!(
            "{}/api/gamelist/v2/users/me/titles?limit={}&categories=ps5_native_game,ps4_game",
            self.base_url, effective_limit
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| PlayStationError::ProviderError(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = resp
                .headers()
                .get("Retry-After")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);
            return Err(PlayStationError::RateLimited(retry_after));
        }

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(PlayStationError::ReconnectRequired);
        }

        if !resp.status().is_success() {
            return Err(PlayStationError::ProviderError(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| PlayStationError::ProviderError(e.to_string()))?;

        parse_titles_from_json(&body)
    }

    async fn fetch_user_titles(
        &self,
        access_token: &str,
        limit: usize,
    ) -> Result<Vec<PlayStationGame>, PlayStationError> {
        self.fetch_recently_played(access_token, limit).await
    }
}

pub struct MockPlayStationProviderClient {
    pub games: std::sync::Mutex<Vec<PlayStationGame>>,
    pub fail_with_rate_limit: std::sync::atomic::AtomicBool,
    pub fail_with_unauthorized: std::sync::atomic::AtomicBool,
}

impl MockPlayStationProviderClient {
    pub fn new(games: Vec<PlayStationGame>) -> Self {
        Self {
            games: std::sync::Mutex::new(games),
            fail_with_rate_limit: std::sync::atomic::AtomicBool::new(false),
            fail_with_unauthorized: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_default_ps5_games() -> Self {
        let now = Utc::now();
        let games = vec![
            PlayStationGame {
                title_id: "PPSA01876_00".into(),
                name: "Elden Ring".into(),
                platform: "PS5".into(),
                category: "ps5_native_game".into(),
                image_url: Some(
                    "https://image.api.playstation.com/vulcan/ap/rnd/elden_ring.png".into(),
                ),
                first_played_at: Some(now - Duration::days(30)),
                last_played_at: Some(now - Duration::hours(2)),
                play_duration_seconds: 14400, // 4 hours
                play_count: 18,
            },
            PlayStationGame {
                title_id: "PPSA01521_00".into(),
                name: "Demon's Souls".into(),
                platform: "PS5".into(),
                category: "ps5_native_game".into(),
                image_url: Some(
                    "https://image.api.playstation.com/vulcan/ap/rnd/demons_souls.png".into(),
                ),
                first_played_at: Some(now - Duration::days(60)),
                last_played_at: Some(now - Duration::days(1)),
                play_duration_seconds: 7200, // 2 hours
                play_count: 8,
            },
            PlayStationGame {
                title_id: "PPSA01325_00".into(),
                name: "Astro's Playroom".into(),
                platform: "PS5".into(),
                category: "ps5_native_game".into(),
                image_url: Some(
                    "https://image.api.playstation.com/vulcan/ap/rnd/astros.png".into(),
                ),
                first_played_at: Some(now - Duration::days(90)),
                last_played_at: Some(now - Duration::days(3)),
                play_duration_seconds: 5400,
                play_count: 5,
            },
        ];
        Self::new(games)
    }
}

#[async_trait::async_trait]
impl PlayStationProviderClient for MockPlayStationProviderClient {
    async fn fetch_recently_played(
        &self,
        _access_token: &str,
        limit: usize,
    ) -> Result<Vec<PlayStationGame>, PlayStationError> {
        if self
            .fail_with_rate_limit
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(PlayStationError::RateLimited(30));
        }
        if self
            .fail_with_unauthorized
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(PlayStationError::ReconnectRequired);
        }

        let all = self.games.lock().unwrap();
        let effective_limit = if limit == 0 { 20 } else { limit };
        Ok(all.iter().take(effective_limit).cloned().collect())
    }

    async fn fetch_user_titles(
        &self,
        access_token: &str,
        limit: usize,
    ) -> Result<Vec<PlayStationGame>, PlayStationError> {
        self.fetch_recently_played(access_token, limit).await
    }
}

/// PlayStation Connected Read Service for capability checks and registry declarations.
#[derive(Clone)]
pub struct PlayStationService {
    _db: sqlx::PgPool,
    connections: ConnectionService,
    grants: CapabilityGrantService,
    client: Arc<dyn PlayStationProviderClient>,
    accounts: Option<super::playstation_account::PlayStationAccounts>,
}

impl PlayStationService {
    pub fn new(
        _db: sqlx::PgPool,
        connections: ConnectionService,
        grants: CapabilityGrantService,
        client: Arc<dyn PlayStationProviderClient>,
    ) -> Self {
        Self {
            _db,
            connections,
            grants,
            client,
            accounts: None,
        }
    }

    pub fn with_accounts(
        mut self,
        accounts: super::playstation_account::PlayStationAccounts,
    ) -> Self {
        self.accounts = Some(accounts);
        self
    }

    /// Canonical integration declaration for PlayStation matching the integration registry.
    pub fn integration_declaration(deployment_external_key: &str) -> RegisterIntegrationRequest {
        RegisterIntegrationRequest {
            deployment_external_key: deployment_external_key.into(),
            external_key: PLAYSTATION_INTEGRATION_KEY.into(),
            protocol: IntegrationProtocol::Direct,
            display_name: "PlayStation Network".into(),
            declaration_version: 1,
            capabilities: vec![
                CapabilityDeclaration {
                    external_key: PLAYSTATION_CAPABILITY_RECENTLY_PLAYED_SHORT.into(),
                    effect: CapabilityEffect::Read,
                    access_needs: vec!["recently_played".into()],
                    data_recipients: vec!["m.np.playstation.com".into()],
                    regions: vec![
                        "US".into(),
                        "GB".into(),
                        "CA".into(),
                        "JP".into(),
                        "EU".into(),
                        "IN".into(),
                    ],
                    failure_modes: vec!["reconnect_required".into(), "rate_limited".into()],
                    optional_guarantees: json!({
                        "freshness_seconds": 300,
                        "platforms": ["PS5", "PS4"],
                        "capability_level": "L2_connected_read",
                    }),
                },
                CapabilityDeclaration {
                    external_key: PLAYSTATION_CAPABILITY_GAME_ACTIVITY_SHORT.into(),
                    effect: CapabilityEffect::Read,
                    access_needs: vec!["game_activity".into()],
                    data_recipients: vec!["m.np.playstation.com".into()],
                    regions: vec![
                        "US".into(),
                        "GB".into(),
                        "CA".into(),
                        "JP".into(),
                        "EU".into(),
                        "IN".into(),
                    ],
                    failure_modes: vec!["reconnect_required".into(), "rate_limited".into()],
                    optional_guarantees: json!({
                        "freshness_seconds": 300,
                        "includes_playtime": true,
                        "includes_timestamps": true,
                        "capability_level": "L2_connected_read",
                    }),
                },
            ],
        }
    }

    /// Read recent PlayStation activity for an authorized user and granted agent.
    pub async fn read_recent_activity(
        &self,
        context: &RequestContext,
        agent_external_key: &str,
        connection_id: Uuid,
        limit: usize,
    ) -> Result<PlayStationRecentActivityResponse, PlayStationError> {
        let connection =
            self.connections
                .get(context, connection_id)
                .await
                .map_err(|e| match e {
                    ConnectionError::NotFound => PlayStationError::ConnectionNotFound,
                    ConnectionError::Database(err) => PlayStationError::Database(err),
                    _ => PlayStationError::ConnectionNotFound,
                })?;

        if connection.integration_external_key != PLAYSTATION_INTEGRATION_KEY {
            return Err(PlayStationError::InvalidIntegration);
        }

        if connection.authorization_state != AuthorizationState::Authorized {
            return Err(PlayStationError::ReconnectRequired);
        }

        if connection.expires_at.is_some_and(|exp| exp <= Utc::now()) {
            return Err(PlayStationError::ReconnectRequired);
        }

        let effective_grants = self
            .grants
            .effective_for_agent(context, agent_external_key)
            .await
            .map_err(|e| match e {
                CapabilityGrantError::Database(err) => PlayStationError::Database(err),
                _ => PlayStationError::UnauthorizedCapability(
                    PLAYSTATION_CAPABILITY_GAME_ACTIVITY.into(),
                    agent_external_key.into(),
                ),
            })?;

        let has_grant = effective_grants.iter().any(|g| {
            g.connection_id == connection_id
                && (g.capability_external_key == PLAYSTATION_CAPABILITY_GAME_ACTIVITY
                    || g.capability_external_key == PLAYSTATION_CAPABILITY_RECENTLY_PLAYED)
        });

        if !has_grant {
            return Err(PlayStationError::UnauthorizedCapability(
                PLAYSTATION_CAPABILITY_GAME_ACTIVITY.into(),
                agent_external_key.into(),
            ));
        }

        let accounts = self
            .accounts
            .as_ref()
            .ok_or(PlayStationError::NotConfigured)?;
        let mut tx = self._db.begin().await?;
        sqlx::query("SELECT id FROM external_connections WHERE id=$1 AND user_context_id=$2 AND authorization_state='authorized' FOR UPDATE").bind(connection_id).bind(context.id.0).fetch_optional(&mut *tx).await?.ok_or(PlayStationError::ReconnectRequired)?;
        let token = accounts.access_token(&mut tx, connection_id).await?;
        let games = self.client.fetch_recently_played(&token, limit).await?;
        tx.commit().await?;
        let count = games.len();

        Ok(PlayStationRecentActivityResponse {
            games,
            count,
            freshness_seconds: 300,
        })
    }
}

pub fn parse_titles_from_json(
    body: &serde_json::Value,
) -> Result<Vec<PlayStationGame>, PlayStationError> {
    let titles_array = if let Some(titles) = body.get("titles").and_then(|t| t.as_array()) {
        titles
    } else if let Some(arr) = body.as_array() {
        arr
    } else {
        return Err(PlayStationError::Invalid);
    };

    let mut result = Vec::new();

    for item in titles_array {
        let title_id = item
            .get("titleId")
            .or_else(|| item.get("npTitleId"))
            .or_else(|| item.get("conceptId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let name = item
            .get("name")
            .or_else(|| item.get("titleName"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if title_id.is_empty() || name.is_empty() {
            return Err(PlayStationError::Invalid);
        }

        let platform = item
            .get("platform")
            .or_else(|| item.get("category"))
            .and_then(|v| v.as_str())
            .map(|s| {
                if s.to_lowercase().contains("ps5") {
                    "PS5".to_string()
                } else if s.to_lowercase().contains("ps4") {
                    "PS4".to_string()
                } else {
                    s.to_string()
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        let category = item
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let image_url = item
            .get("imageUrl")
            .or_else(|| item.get("localizedImageUrl"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let first_played_at = item
            .get("firstPlayedDateTime")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let last_played_at = item
            .get("lastPlayedDateTime")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let Some(duration) = item.get("playDuration").filter(|value| !value.is_null()) else {
            continue;
        };
        let play_duration_seconds =
            checked_play_duration(duration).ok_or(PlayStationError::Invalid)?;

        let play_count = item.get("playCount").and_then(|v| v.as_u64()).unwrap_or(1) as u32;

        result.push(PlayStationGame {
            title_id,
            name,
            platform,
            category,
            image_url,
            first_played_at,
            last_played_at,
            play_duration_seconds,
            play_count,
        });
    }

    Ok(result)
}

/// Helper function to parse ISO 8601 duration strings (e.g. "PT2H15M30S") or numeric seconds.
fn checked_play_duration(val: &serde_json::Value) -> Option<u64> {
    if let Some(n) = val.as_u64() {
        return Some(n);
    }
    let s = val.as_str()?;
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    let time = s.strip_prefix("PT")?;
    if time.is_empty() {
        return None;
    }
    let mut total = 0u64;
    let mut number = String::new();
    let mut last = 0;
    for c in time.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let (order, multiplier) = match c {
            'H' => (1, 3600),
            'M' => (2, 60),
            'S' => (3, 1),
            _ => return None,
        };
        if order <= last {
            return None;
        }
        let n = number.parse::<u64>().ok()?;
        total = total.checked_add(n.checked_mul(multiplier)?)?;
        last = order;
        number.clear();
    }
    if !number.is_empty() || last == 0 {
        return None;
    }
    Some(total)
}
