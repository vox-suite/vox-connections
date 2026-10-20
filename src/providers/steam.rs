//! Steam OpenID identifies the account; Web API reads require public game details.
use super::personal::PersonalActivity;
use crate::accounts::FreshConnectionError as Error;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const OP: &str = "https://steamcommunity.com/openid/login";
const NS: &str = "http://specs.openid.net/auth/2.0";
#[derive(Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    key: Option<String>,
    #[cfg(test)]
    test_origin: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Game {
    pub appid: u32,
    pub name: String,
    /// None means Steam withheld playtime; it must never become zero.
    pub playtime_minutes: Option<u64>,
    pub recent_minutes: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Counter {
    pub minutes: u64,
    pub observed_at: DateTime<Utc>,
}
fn failed() -> Error {
    Error::Provider("Steam request failed".into())
}
impl Client {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            #[cfg(test)]
            test_origin: None,
            key: std::env::var("STEAM_WEB_API_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }
    fn endpoint(&self, endpoint: &str) -> String {
        #[cfg(test)]
        if let Some(origin) = &self.test_origin {
            return format!("{}{}", origin, url::Url::parse(endpoint).unwrap().path());
        }
        endpoint.to_owned()
    }
    pub fn enabled(&self) -> bool {
        self.key.is_some()
    }
    pub fn auth_url(return_to: &str) -> Result<String, Error> {
        let redirect = url::Url::parse(return_to).map_err(|_| failed())?;
        let mut url = url::Url::parse(OP).map_err(|_| failed())?;
        url.query_pairs_mut().extend_pairs([
            ("openid.ns", NS),
            ("openid.mode", "checkid_setup"),
            ("openid.return_to", return_to),
            (
                "openid.realm",
                &format!("{}/", redirect.origin().ascii_serialization()),
            ),
            (
                "openid.identity",
                "http://specs.openid.net/auth/2.0/identifier_select",
            ),
            (
                "openid.claimed_id",
                "http://specs.openid.net/auth/2.0/identifier_select",
            ),
        ]);
        Ok(url.into())
    }
    pub async fn verify(
        &self,
        params: &BTreeMap<String, String>,
        return_to: &str,
    ) -> Result<String, Error> {
        let id = validate_assertion(params, return_to, Utc::now())?;
        // Always verify at the fixed provider endpoint, never at a callback-supplied URL.
        let mut form: BTreeMap<String, String> = params
            .iter()
            .filter(|(k, _)| k.starts_with("openid."))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        form.insert("openid.mode".into(), "check_authentication".into());
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let response = self
            .http
            .post(self.endpoint(OP))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|_| failed())?;
        if !response.status().is_success() {
            return Err(failed());
        }
        let text = response.text().await.map_err(|_| failed())?;
        if !text.lines().any(|l| l.trim() == "is_valid:true") {
            return Err(Error::Unauthorized);
        }
        Ok(id)
    }
    pub async fn games(&self, steamid: &str) -> Result<Vec<Game>, Error> {
        let key = self
            .key
            .as_deref()
            .ok_or_else(|| Error::NotConfigured("Steam Web API key missing".into()))?;
        let response = self.http.get(self.endpoint("https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/"))
            .query(&[("key", key.to_owned()), ("input_json", json!({"steamid":steamid,"include_appinfo":true,"include_played_free_games":true}).to_string())])
            .send().await.map_err(|_| failed())?;
        if !response.status().is_success() {
            return Err(failed());
        }
        parse_games(&response.json::<Value>().await.map_err(|_| failed())?)
    }
}
fn validate_assertion(
    p: &BTreeMap<String, String>,
    return_to: &str,
    now: DateTime<Utc>,
) -> Result<String, Error> {
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or_default();
    if get("openid.ns") != NS
        || get("openid.mode") != "id_res"
        || get("openid.op_endpoint") != OP
        || get("openid.return_to") != return_to
        || get("openid.identity") != get("openid.claimed_id")
    {
        return Err(Error::Unauthorized);
    }
    let signed: Vec<_> = get("openid.signed").split(',').collect();
    if [
        "op_endpoint",
        "claimed_id",
        "identity",
        "return_to",
        "response_nonce",
        "assoc_handle",
    ]
    .iter()
    .any(|k| !signed.contains(k))
    {
        return Err(Error::Unauthorized);
    }
    let nonce = get("openid.response_nonce");
    let timestamp = nonce
        .get(..20)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .ok_or(Error::Unauthorized)?;
    if now.signed_duration_since(timestamp) > Duration::minutes(10)
        || timestamp.signed_duration_since(now) > Duration::minutes(1)
        || nonce.len() <= 20
    {
        return Err(Error::Unauthorized);
    }
    let claim = get("openid.claimed_id");
    let id = claim
        .strip_prefix("https://steamcommunity.com/openid/id/")
        .or_else(|| claim.strip_prefix("http://steamcommunity.com/openid/id/"))
        .ok_or(Error::Unauthorized)?;
    if id.len() != 17 || !id.bytes().all(|c| c.is_ascii_digit()) || id.parse::<u64>().is_err() {
        return Err(Error::Unauthorized);
    }
    Ok(id.to_owned())
}
pub fn parse_games(value: &Value) -> Result<Vec<Game>, Error> {
    let response = &value["response"];
    let count = response["game_count"].as_u64().ok_or_else(|| {
        Error::Provider(
            "Steam game details are private or unavailable; make Game details public".into(),
        )
    })?;
    let rows = match response["games"].as_array() {
        Some(rows) => rows.as_slice(),
        None if count == 0 => &[],
        _ => return Err(failed()),
    };
    if rows.len() as u64 != count {
        return Err(failed());
    }
    let mut seen = std::collections::BTreeSet::new();
    rows.iter()
        .map(|r| {
            let appid = r["appid"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v > 0)
                .ok_or_else(failed)?;
            if !seen.insert(appid) {
                return Err(failed());
            }
            Ok(Game {
                appid,
                name: r["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Steam game {appid}")),
                playtime_minutes: r["playtime_forever"].as_u64(),
                recent_minutes: r["playtime_2weeks"].as_u64(),
            })
        })
        .collect()
}
/// Persist high-water counters with the events in one transaction. New/hidden games
/// establish a baseline instead of importing their lifetime totals as new playtime.
pub fn observe(
    account: &str,
    previous: &BTreeMap<u32, Counter>,
    games: &[Game],
    now: DateTime<Utc>,
) -> (BTreeMap<u32, Counter>, Vec<PersonalActivity>) {
    let mut next = previous.clone();
    let mut activities = Vec::new();
    for game in games {
        let Some(minutes) = game.playtime_minutes else {
            continue;
        };
        if let Some(old) = previous.get(&game.appid) {
            if minutes < old.minutes || now <= old.observed_at {
                continue;
            }
            if let Some(seconds) = minutes
                .checked_sub(old.minutes)
                .and_then(|v| v.checked_mul(60))
                .filter(|v| *v > 0)
            {
                activities.push(PersonalActivity { source_id: format!("{account}:{}:{}:{minutes}",game.appid,old.minutes), title: game.name.clone(), occurred_at: now, ended_at: None, provider_data: json!({"action":"playtime_observed","game":game.name,"platform":"Steam","title_id":game.appid.to_string(),"duration_seconds":seconds,"observation_start":old.observed_at,"observation_end":now,"timing":"observed_counter_delta","session_times_known":false}) });
            }
        }
        next.insert(
            game.appid,
            Counter {
                minutes,
                observed_at: now,
            },
        );
    }
    (next, activities)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_is_not_empty_and_missing_playtime_is_not_zero() {
        assert!(parse_games(&json!({"response":{}})).is_err());
        assert!(
            parse_games(&json!({"response":{"game_count":0}}))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            parse_games(&json!({"response":{"game_count":1,"games":[{"appid":10,"name":"Game"}]}}))
                .unwrap()[0]
                .playtime_minutes,
            None
        );
        assert!(parse_games(&json!({"response":{"game_count":2,"games":[]}})).is_err());
    }
    #[test]
    fn observes_only_new_minutes_preserves_high_water_and_deduplicates() {
        let now = Utc::now();
        let mut games = vec![Game {
            appid: 10,
            name: "Game".into(),
            playtime_minutes: Some(100),
            recent_minutes: None,
        }];
        let (base, events) = observe("account", &BTreeMap::new(), &games, now);
        assert!(events.is_empty());
        games[0].playtime_minutes = Some(130);
        let (next, events) = observe("account", &base, &games, now + Duration::minutes(40));
        assert_eq!(events[0].provider_data["duration_seconds"], 1800);
        assert_eq!(events[0].ended_at, None);
        assert!(
            observe("account", &next, &games, now + Duration::minutes(45))
                .1
                .is_empty()
        );
        games[0].playtime_minutes = Some(90);
        assert_eq!(
            observe("account", &next, &games, now + Duration::minutes(50)).0[&10].minutes,
            130
        );
    }
    #[test]
    fn assertion_requires_expected_return_signed_identity_and_fresh_nonce() {
        let now = Utc::now();
        let ret = "https://vox.example/callback?state=secret";
        let mut p: BTreeMap<String, String> = [
            ("openid.ns", NS.to_string()),
            ("openid.mode", "id_res".into()),
            ("openid.op_endpoint", OP.into()),
            ("openid.return_to", ret.into()),
            (
                "openid.identity",
                "https://steamcommunity.com/openid/id/76561198000000001".into(),
            ),
            (
                "openid.claimed_id",
                "https://steamcommunity.com/openid/id/76561198000000001".into(),
            ),
            (
                "openid.signed",
                "op_endpoint,claimed_id,identity,return_to,response_nonce,assoc_handle".into(),
            ),
            (
                "openid.response_nonce",
                format!("{}xyz", now.format("%Y-%m-%dT%H:%M:%SZ")),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect();
        assert!(validate_assertion(&p, ret, now).is_ok());
        assert!(validate_assertion(&p, "https://evil.example", now).is_err());
        assert!(validate_assertion(&p, ret, now + Duration::minutes(11)).is_err());
        p.insert("openid.signed".into(), "return_to".into());
        assert!(validate_assertion(&p, ret, now).is_err());
    }
    #[test]
    fn missing_new_games_and_clock_reversal_never_invent_activity() {
        let now = Utc::now();
        let game = Game {
            appid: 1,
            name: "Old game".into(),
            playtime_minutes: Some(200),
            recent_minutes: None,
        };
        let (base, _) = observe("a", &BTreeMap::new(), &[game.clone()], now);
        let hidden = Game {
            playtime_minutes: None,
            ..game.clone()
        };
        assert_eq!(
            observe("a", &base, &[hidden], now + Duration::minutes(10)).0[&1].minutes,
            200
        );
        let increased = Game {
            playtime_minutes: Some(220),
            ..game.clone()
        };
        assert!(
            observe("a", &base, &[increased], now - Duration::minutes(1))
                .1
                .is_empty()
        );
        let new_game = Game { appid: 2, ..game };
        assert!(
            observe("a", &base, &[new_game], now + Duration::minutes(10))
                .1
                .is_empty()
        );
    }
    #[tokio::test]
    async fn owned_games_uses_server_key_and_service_parameters() {
        use axum::{Json, Router, extract::Query, routing::get};
        async fn handler(Query(q): Query<BTreeMap<String, String>>) -> Json<Value> {
            assert_eq!(q.get("key").unwrap(), "test-secret");
            let input: Value = serde_json::from_str(q.get("input_json").unwrap()).unwrap();
            assert_eq!(input["steamid"], "76561198000000001");
            assert_eq!(input["include_appinfo"], true);
            Json(
                json!({"response":{"game_count":1,"games":[{"appid":10,"name":"Game","playtime_forever":120}]}}),
            )
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/IPlayerService/GetOwnedGames/v1/", get(handler)),
            )
            .await
            .unwrap()
        });
        let client = Client {
            http: reqwest::Client::new(),
            key: Some("test-secret".into()),
            test_origin: Some(origin),
        };
        assert_eq!(
            client.games("76561198000000001").await.unwrap()[0].playtime_minutes,
            Some(120)
        );
        server.abort();
    }
    #[tokio::test]
    async fn openid_requires_provider_verification_and_uses_check_authentication() {
        use axum::{Router, routing::post};
        let now = Utc::now();
        let ret = "https://vox.example/callback?state=secret";
        let params: BTreeMap<String, String> = [
            ("openid.ns", NS.to_string()),
            ("openid.mode", "id_res".into()),
            ("openid.op_endpoint", OP.into()),
            ("openid.return_to", ret.into()),
            (
                "openid.identity",
                "https://steamcommunity.com/openid/id/76561198000000001".into(),
            ),
            (
                "openid.claimed_id",
                "https://steamcommunity.com/openid/id/76561198000000001".into(),
            ),
            (
                "openid.signed",
                "op_endpoint,claimed_id,identity,return_to,response_nonce,assoc_handle".into(),
            ),
            (
                "openid.response_nonce",
                format!("{}xyz", now.format("%Y-%m-%dT%H:%M:%SZ")),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect();
        for valid in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(
                    listener,
                    Router::new().route(
                        "/openid/login",
                        post(move |body: String| async move {
                            let form: BTreeMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
                                .into_owned()
                                .collect();
                            assert_eq!(form.get("openid.mode").unwrap(), "check_authentication");
                            assert!(!form.contains_key("state"));
                            format!("ns:{NS}\nis_valid:{valid}\n")
                        }),
                    ),
                )
                .await
                .unwrap()
            });
            let client = Client {
                http: reqwest::Client::new(),
                key: None,
                test_origin: Some(origin),
            };
            assert_eq!(client.verify(&params, ret).await.is_ok(), valid);
            server.abort();
        }
    }
}
