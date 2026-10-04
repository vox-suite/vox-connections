use super::playstation::PlayStationGame;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GameSnapshot {
    pub total_seconds: u64,
    pub observed_at: DateTime<Utc>,
    pub last_played_at: Option<DateTime<Utc>>,
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

pub fn checkpoint(
    previous: &BTreeMap<String, GameSnapshot>,
    games: &[PlayStationGame],
    now: DateTime<Utc>,
) -> BTreeMap<String, GameSnapshot> {
    let mut snapshots = previous.clone();
    for game in games {
        snapshots.insert(
            game.title_id.clone(),
            GameSnapshot {
                total_seconds: previous
                    .get(&game.title_id)
                    .map(|old| old.total_seconds.max(game.play_duration_seconds))
                    .unwrap_or(game.play_duration_seconds),
                observed_at: previous
                    .get(&game.title_id)
                    .filter(|old| game.play_duration_seconds < old.total_seconds)
                    .map(|old| old.observed_at)
                    .unwrap_or(now),
                last_played_at: game.last_played_at,
                name: game.name.clone(),
                platform: game.platform.clone(),
            },
        );
    }
    snapshots
}
#[cfg(test)]
mod tests {
    use super::*;
    fn game(total: u64) -> PlayStationGame {
        PlayStationGame {
            title_id: "game".into(),
            name: "Game".into(),
            platform: "PS5".into(),
            category: "gaming".into(),
            image_url: None,
            first_played_at: None,
            last_played_at: Some(Utc::now()),
            play_duration_seconds: total,
            play_count: 1,
        }
    }
    #[test]
    fn lowered_counters_keep_high_water_and_duplicate_sync_is_empty() {
        let now = Utc::now();
        let baseline = checkpoint(&BTreeMap::new(), &[game(100)], now);
        let lowered = checkpoint(&baseline, &[game(80)], now + chrono::Duration::hours(1));
        assert_eq!(lowered["game"].total_seconds, 100);
        assert!(
            observed_activity(
                "account",
                &lowered,
                &[game(100)],
                Some(now),
                now + chrono::Duration::hours(2)
            )
            .is_empty()
        );
        let delta = observed_activity(
            "account",
            &lowered,
            &[game(110)],
            Some(now),
            now + chrono::Duration::hours(2),
        );
        assert_eq!(delta[0].duration_seconds, 10);
        assert_eq!(delta[0].observation_start, now);
        let current = checkpoint(&lowered, &[game(110)], now + chrono::Duration::hours(2));
        assert!(
            observed_activity(
                "account",
                &current,
                &[game(110)],
                Some(now),
                now + chrono::Duration::hours(3)
            )
            .is_empty()
        );
    }
}
