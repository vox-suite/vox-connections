use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
