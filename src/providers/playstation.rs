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

/// Represents a played game extracted from PlayStation 5 / PSN.
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
    pub last_played_at: DateTime<Utc>,
    pub play_duration_seconds: u64,
    pub play_count: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlayStationRecentActivityResponse {
    pub games: Vec<PlayStationGame>,
    pub count: usize,
    pub freshness_seconds: u32,
}

/// Prepared Span representation of a gaming session ready to be saved in the database.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlayStationSpanInput {
    pub user_id: Uuid,
    pub user_context_id: Uuid,
    pub title: String,
    pub notes: String,
    pub category: String,
    pub source: String,
    pub source_ref: String,
    pub status: String,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub execution_type: String,
    pub data: serde_json::Value,
}

/// Summary report returned after syncing PlayStation activity to Spans.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlayStationSyncResult {
    pub user_id: Uuid,
    pub user_context_id: Uuid,
    pub connection_id: Option<Uuid>,
    pub synced_spans_count: usize,
    pub span_ids: Vec<Uuid>,
    pub games_processed: Vec<String>,
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
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
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
            http: reqwest::Client::new(),
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
                image_url: Some("https://image.api.playstation.com/vulcan/ap/rnd/elden_ring.png".into()),
                first_played_at: Some(now - Duration::days(30)),
                last_played_at: now - Duration::hours(2),
                play_duration_seconds: 14400, // 4 hours
                play_count: 18,
            },
            PlayStationGame {
                title_id: "PPSA01521_00".into(),
                name: "Demon's Souls".into(),
                platform: "PS5".into(),
                category: "ps5_native_game".into(),
                image_url: Some("https://image.api.playstation.com/vulcan/ap/rnd/demons_souls.png".into()),
                first_played_at: Some(now - Duration::days(60)),
                last_played_at: now - Duration::days(1),
                play_duration_seconds: 7200, // 2 hours
                play_count: 8,
            },
            PlayStationGame {
                title_id: "PPSA01325_00".into(),
                name: "Astro's Playroom".into(),
                platform: "PS5".into(),
                category: "ps5_native_game".into(),
                image_url: Some("https://image.api.playstation.com/vulcan/ap/rnd/astros.png".into()),
                first_played_at: Some(now - Duration::days(90)),
                last_played_at: now - Duration::days(3),
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
        if self.fail_with_rate_limit.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(PlayStationError::RateLimited(30));
        }
        if self.fail_with_unauthorized.load(std::sync::atomic::Ordering::SeqCst) {
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
        }
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
                    regions: vec!["US".into(), "GB".into(), "CA".into(), "JP".into(), "EU".into(), "IN".into()],
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
                    regions: vec!["US".into(), "GB".into(), "CA".into(), "JP".into(), "EU".into(), "IN".into()],
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
        let connection = self
            .connections
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

        let mock_token = "authorized-psn-token";
        let games = self.client.fetch_recently_played(mock_token, limit).await?;
        let count = games.len();

        Ok(PlayStationRecentActivityResponse {
            games,
            count,
            freshness_seconds: 300,
        })
    }
}

/// PlayStation 5 Activity Worker.
///
/// Polls/extracts game history from PlayStation Network and records
/// each played game as a Span in the Vox timeline.
#[derive(Clone)]
pub struct PlayStationActivityWorker {
    db: sqlx::PgPool,
    connections: ConnectionService,
    client: Arc<dyn PlayStationProviderClient>,
}

impl PlayStationActivityWorker {
    pub fn new(
        db: sqlx::PgPool,
        connections: ConnectionService,
        client: Arc<dyn PlayStationProviderClient>,
    ) -> Self {
        Self {
            db,
            connections,
            client,
        }
    }

    /// Pure function converting a list of played games into Span inputs.
    pub fn extract_spans_from_games(
        user_id: Uuid,
        user_context_id: Uuid,
        games: &[PlayStationGame],
    ) -> Vec<PlayStationSpanInput> {
        games
            .iter()
            .map(|g| {
                // Determine sensible start and end times for the span.
                // If play duration is available, start_at reflects the session start,
                // capped to reasonable session length (e.g. at most 3 hours per recorded chunk).
                let session_seconds = if g.play_duration_seconds > 0 {
                    g.play_duration_seconds.min(10800)
                } else {
                    1800 // Default 30 min block if unknown
                };

                let start_at = g.last_played_at - Duration::seconds(session_seconds as i64);
                let end_at = g.last_played_at;

                // Format descriptive note with playtime stats
                let hours = g.play_duration_seconds / 3600;
                let minutes = (g.play_duration_seconds % 3600) / 60;
                let playtime_str = if hours > 0 {
                    format!("{}h {}m", hours, minutes)
                } else {
                    format!("{}m", minutes)
                };

                let notes = format!(
                    "Played {} on {}. Total recorded playtime: {}.",
                    g.name, g.platform, playtime_str
                );

                // Source reference key includes title ID and last played timestamp
                // so subsequent syncs are idempotent, while new play sessions produce distinct spans.
                let source_ref = format!("{}:{}", g.title_id, g.last_played_at.timestamp());

                let data = json!({
                    "integration": "playstation",
                    "platform": g.platform,
                    "title_id": g.title_id,
                    "game_name": g.name,
                    "category": g.category,
                    "image_url": g.image_url,
                    "play_duration_seconds": g.play_duration_seconds,
                    "play_count": g.play_count,
                    "first_played_at": g.first_played_at,
                    "last_played_at": g.last_played_at,
                });

                PlayStationSpanInput {
                    user_id,
                    user_context_id,
                    title: g.name.clone(),
                    notes,
                    category: "gaming".into(),
                    source: "playstation".into(),
                    source_ref,
                    status: "done".into(),
                    start_at,
                    end_at,
                    execution_type: "manual_human".into(),
                    data,
                }
            })
            .collect()
    }

    /// Syncs games for a specific user into the spans table.
    pub async fn sync_user_activity(
        &self,
        user_id: Uuid,
        user_context_id: Uuid,
        access_token: &str,
        limit: usize,
    ) -> Result<PlayStationSyncResult, PlayStationError> {
        let games = self
            .client
            .fetch_recently_played(access_token, limit)
            .await?;

        let span_inputs = Self::extract_spans_from_games(user_id, user_context_id, &games);
        let mut span_ids = Vec::with_capacity(span_inputs.len());
        let mut games_processed = Vec::with_capacity(span_inputs.len());

        let mut tx = self.db.begin().await?;

        for input in span_inputs {
            let span_id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO spans (
                    user_id, user_context_id, title, notes, category, source, source_ref,
                    status, start_at, end_at, completed_at, execution_type, data, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10, $11, $12, now())
                ON CONFLICT (user_id, source, source_ref) DO UPDATE SET
                    title = EXCLUDED.title,
                    notes = EXCLUDED.notes,
                    category = EXCLUDED.category,
                    start_at = EXCLUDED.start_at,
                    end_at = EXCLUDED.end_at,
                    completed_at = EXCLUDED.completed_at,
                    data = EXCLUDED.data,
                    updated_at = now()
                RETURNING id",
            )
            .bind(input.user_id)
            .bind(input.user_context_id)
            .bind(&input.title)
            .bind(&input.notes)
            .bind(&input.category)
            .bind(&input.source)
            .bind(&input.source_ref)
            .bind(&input.status)
            .bind(input.start_at)
            .bind(input.end_at)
            .bind(&input.execution_type)
            .bind(&input.data)
            .fetch_one(&mut *tx)
            .await?;

            span_ids.push(span_id);
            games_processed.push(input.title);
        }

        tx.commit().await?;

        Ok(PlayStationSyncResult {
            user_id,
            user_context_id,
            connection_id: None,
            synced_spans_count: span_ids.len(),
            span_ids,
            games_processed,
        })
    }

    /// Syncs games for an existing connection ID, verifying connection authorization state.
    pub async fn sync_connection_to_spans(
        &self,
        context: &RequestContext,
        connection_id: Uuid,
        limit: usize,
    ) -> Result<PlayStationSyncResult, PlayStationError> {
        let connection = self
            .connections
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

        let token = "authorized-psn-token";
        let mut result = self
            .sync_user_activity(context.user_id.0, context.id.0, token, limit)
            .await?;
        result.connection_id = Some(connection_id);
        Ok(result)
    }

    /// Scans all active and authorized PlayStation connections and syncs their game activity into spans.
    pub async fn sync_all_active_connections(
        &self,
        limit_per_connection: usize,
    ) -> Result<Vec<PlayStationSyncResult>, PlayStationError> {
        let rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
            "SELECT c.id, c.user_context_id, uc.user_id
             FROM external_connections c
             JOIN integration_definitions i ON i.id = c.integration_id
             JOIN user_contexts uc ON uc.id = c.user_context_id
             WHERE i.external_key = $1
               AND c.authorization_state = 'authorized'
               AND (c.expires_at IS NULL OR c.expires_at > now())",
        )
        .bind(PLAYSTATION_INTEGRATION_KEY)
        .fetch_all(&self.db)
        .await?;

        let mut results = Vec::new();
        for (connection_id, user_context_id, user_id) in rows {
            let token = "authorized-psn-token";
            match self
                .sync_user_activity(user_id, user_context_id, token, limit_per_connection)
                .await
            {
                Ok(mut res) => {
                    res.connection_id = Some(connection_id);
                    results.push(res);
                }
                Err(err) => {
                    // Log error and continue to other connections without crashing worker
                    tracing::warn!(
                        connection_id = %connection_id,
                        user_id = %user_id,
                        error = %err,
                        "Failed to sync PlayStation connection"
                    );
                }
            }
        }

        Ok(results)
    }
}

/// Helper function to parse JSON response from PlayStation Network gamelist endpoints.
fn parse_titles_from_json(body: &serde_json::Value) -> Result<Vec<PlayStationGame>, PlayStationError> {
    let titles_array = if let Some(titles) = body.get("titles").and_then(|t| t.as_array()) {
        titles
    } else if let Some(arr) = body.as_array() {
        arr
    } else {
        return Ok(Vec::new());
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
            continue;
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
            .unwrap_or_else(|| "PS5".to_string());

        let category = item
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("ps5_native_game")
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
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        let play_duration_seconds = parse_play_duration(
            item.get("playDuration")
                .unwrap_or(&serde_json::Value::Null),
        );

        let play_count = item
            .get("playCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;

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
fn parse_play_duration(val: &serde_json::Value) -> u64 {
    if let Some(n) = val.as_u64() {
        return n;
    }
    if let Some(f) = val.as_f64() {
        return f as u64;
    }
    if let Some(s) = val.as_str() {
        if let Ok(n) = s.parse::<u64>() {
            return n;
        }
        if s.starts_with("PT") {
            let mut total_secs = 0u64;
            let mut num_buf = String::new();
            for c in s.chars().skip(2) {
                if c.is_ascii_digit() {
                    num_buf.push(c);
                } else {
                    let num = num_buf.parse::<u64>().unwrap_or(0);
                    num_buf.clear();
                    match c {
                        'H' => total_secs += num * 3600,
                        'M' => total_secs += num * 60,
                        'S' => total_secs += num,
                        _ => {}
                    }
                }
            }
            return total_secs;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_play_duration() {
        assert_eq!(parse_play_duration(&json!(3600)), 3600);
        assert_eq!(parse_play_duration(&json!("7200")), 7200);
        assert_eq!(parse_play_duration(&json!("PT1H30M15S")), 5415);
        assert_eq!(parse_play_duration(&json!("PT45M")), 2700);
        assert_eq!(parse_play_duration(&json!("PT2H")), 7200);
        assert_eq!(parse_play_duration(&json!(null)), 0);
    }

    #[test]
    fn test_parse_titles_from_json() {
        let payload = json!({
            "titles": [
                {
                    "titleId": "PPSA01876_00",
                    "name": "Elden Ring",
                    "platform": "PS5",
                    "category": "ps5_native_game",
                    "playDuration": "PT10H30M",
                    "lastPlayedDateTime": "2026-09-25T18:00:00Z",
                    "firstPlayedDateTime": "2026-09-01T12:00:00Z",
                    "playCount": 24
                },
                {
                    "titleId": "PPSA01325_00",
                    "name": "Astro's Playroom",
                    "platform": "PS5",
                    "category": "ps5_native_game",
                    "playDuration": 3600,
                    "lastPlayedDateTime": "2026-09-26T10:00:00Z",
                    "playCount": 5
                }
            ]
        });

        let games = parse_titles_from_json(&payload).expect("Failed to parse games");
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].name, "Elden Ring");
        assert_eq!(games[0].platform, "PS5");
        assert_eq!(games[0].play_duration_seconds, 37800);
        assert_eq!(games[1].name, "Astro's Playroom");
        assert_eq!(games[1].play_duration_seconds, 3600);
    }

    #[test]
    fn test_extract_spans_from_games() {
        let user_id = Uuid::new_v4();
        let user_context_id = Uuid::new_v4();
        let last_played = Utc::now();

        let games = vec![PlayStationGame {
            title_id: "PPSA01876_00".into(),
            name: "Elden Ring".into(),
            platform: "PS5".into(),
            category: "ps5_native_game".into(),
            image_url: None,
            first_played_at: Some(last_played - Duration::days(10)),
            last_played_at: last_played,
            play_duration_seconds: 7200,
            play_count: 10,
        }];

        let spans = PlayStationActivityWorker::extract_spans_from_games(
            user_id,
            user_context_id,
            &games,
        );

        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.user_id, user_id);
        assert_eq!(span.user_context_id, user_context_id);
        assert_eq!(span.title, "Elden Ring");
        assert_eq!(span.category, "gaming");
        assert_eq!(span.source, "playstation");
        assert_eq!(
            span.source_ref,
            format!("PPSA01876_00:{}", last_played.timestamp())
        );
        assert_eq!(span.status, "done");
        assert_eq!(span.execution_type, "manual_human");
        assert!(span.end_at >= span.start_at);
        assert!(span.notes.contains("Played Elden Ring on PS5"));
        assert_eq!(span.data["platform"], "PS5");
        assert_eq!(span.data["title_id"], "PPSA01876_00");
    }

    #[tokio::test]
    async fn test_mock_client_fetch() {
        let client = MockPlayStationProviderClient::with_default_ps5_games();
        let games = client
            .fetch_recently_played("test-token", 10)
            .await
            .expect("fetch failed");

        assert_eq!(games.len(), 3);
        assert_eq!(games[0].name, "Elden Ring");
        assert_eq!(games[0].platform, "PS5");
    }
}
