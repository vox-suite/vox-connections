use crate::accounts::FreshConnectionError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalActivity {
    pub source_id: String,
    pub title: String,
    pub occurred_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub provider_data: Value,
}

#[derive(Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    pub spotify_id: Option<String>,
    google_id: Option<String>,
    google_secret: Option<String>,
    #[cfg(test)]
    pub test_origin: Option<String>,
}
#[derive(Deserialize)]
pub(crate) struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub scope: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct StoredAccess {
    pub token: String,
}
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub account_id: String,
    pub display_id: Option<String>,
    pub activities: Vec<PersonalActivity>,
    pub context: Value,
}
fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
impl Client {
    pub fn new(
        http: reqwest::Client,
        google_id: Option<String>,
        google_secret: Option<String>,
    ) -> Self {
        Self {
            http,
            spotify_id: env("SPOTIFY_CLIENT_ID"),
            google_id,
            google_secret,
            #[cfg(test)]
            test_origin: None,
        }
    }
    fn endpoint(&self, url: &str) -> String {
        #[cfg(test)]
        if let Some(origin) = &self.test_origin {
            let parsed = url::Url::parse(url).unwrap();
            return format!(
                "{origin}{}{}",
                parsed.path(),
                parsed.query().map(|q| format!("?{q}")).unwrap_or_default()
            );
        }
        url.to_owned()
    }
    pub fn enabled(&self, connector: &str) -> bool {
        match connector {
            "spotify" => self.spotify_id.is_some(),
            "youtube" => {
                self.google_id
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty())
                    && self
                        .google_secret
                        .as_ref()
                        .is_some_and(|v| !v.trim().is_empty())
            }
            _ => false,
        }
    }
    fn client_id(&self, connector: &str) -> Result<&str, FreshConnectionError> {
        match connector {
            "spotify" => self.spotify_id.as_deref(),
            "youtube" => self.google_id.as_deref(),
            _ => None,
        }
        .ok_or_else(|| FreshConnectionError::NotConfigured("Provider credentials missing".into()))
    }
    pub fn auth_url(
        &self,
        connector: &str,
        redirect: &str,
        state: &str,
        verifier: &str,
    ) -> Result<String, FreshConnectionError> {
        if !self.enabled(connector) {
            return Err(FreshConnectionError::NotConfigured(
                "Provider credentials missing".into(),
            ));
        }
        let mut url = url::Url::parse(match connector {
            "spotify" => "https://accounts.spotify.com/authorize",
            "youtube" => "https://accounts.google.com/o/oauth2/v2/auth",
            _ => return Err(FreshConnectionError::Invalid("Unknown provider".into())),
        })
        .unwrap();
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", self.client_id(connector)?)
            .append_pair("redirect_uri", redirect)
            .append_pair("response_type", "code")
            .append_pair("state", state);
        match connector {
            "spotify" => {
                query
                    .append_pair("scope", "user-read-recently-played user-read-private playlist-read-private playlist-read-collaborative")
                    .append_pair("code_challenge_method", "S256")
                    .append_pair("code_challenge", &crate::crypto::pkce_challenge(verifier));
            }
            "youtube" => {
                query
                    .append_pair("scope", "https://www.googleapis.com/auth/youtube.readonly")
                    .append_pair("access_type", "offline")
                    .append_pair("prompt", "consent")
                    .append_pair("code_challenge_method", "S256")
                    .append_pair("code_challenge", &crate::crypto::pkce_challenge(verifier));
            }
            _ => return Err(FreshConnectionError::Invalid("Unknown provider".into())),
        }
        drop(query);
        Ok(url.to_string())
    }
    async fn response(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<Value, FreshConnectionError> {
        let mut response = request
            .send()
            .await
            .map_err(|_| FreshConnectionError::Provider("Provider request failed".into()))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(FreshConnectionError::Unauthorized);
        }
        const MAX_RESPONSE: usize = 8 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
        {
            return Err(FreshConnectionError::Provider(
                "Provider response too large".into(),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| FreshConnectionError::Provider("Provider response failed".into()))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
                return Err(FreshConnectionError::Provider(
                    "Provider response too large".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| FreshConnectionError::Provider("Invalid provider response".into()))?;
        if !status.is_success() {
            if matches!(
                value.get("error").and_then(Value::as_str),
                Some("invalid_grant" | "invalid_token")
            ) {
                return Err(FreshConnectionError::Unauthorized);
            }
            return Err(FreshConnectionError::Provider(
                "Provider request rejected".into(),
            ));
        }
        Ok(value)
    }
    pub async fn tokens(
        &self,
        connector: &str,
        code: Option<&str>,
        redirect: &str,
        verifier: &str,
        refresh: Option<&str>,
    ) -> Result<Tokens, FreshConnectionError> {
        if !self.enabled(connector) {
            return Err(FreshConnectionError::NotConfigured(
                "Provider disabled".into(),
            ));
        }
        let endpoint = match connector {
            "spotify" => "https://accounts.spotify.com/api/token",
            "youtube" => "https://oauth2.googleapis.com/token",
            _ => return Err(FreshConnectionError::Invalid("Unknown provider".into())),
        };
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("client_id", self.client_id(connector)?);
            if let Some(refresh) = refresh {
                form.append_pair("grant_type", "refresh_token")
                    .append_pair("refresh_token", refresh);
            } else {
                form.append_pair("grant_type", "authorization_code")
                    .append_pair("code", code.ok_or(FreshConnectionError::Unauthorized)?)
                    .append_pair("redirect_uri", redirect);
                form.append_pair("code_verifier", verifier);
            }
            if connector == "youtube" {
                form.append_pair("client_secret", self.google_secret.as_deref().unwrap());
            }
            form.finish()
        };
        let request = self
            .http
            .post(self.endpoint(endpoint))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body);
        let value = self.response(request).await?;
        let tokens: Tokens =
            serde_json::from_value(value).map_err(|_| FreshConnectionError::Unauthorized)?;
        if tokens.access_token.is_empty()
            || tokens.expires_in <= 0
            || tokens.expires_in > 366 * 86400
            || (refresh.is_none() && tokens.refresh_token.as_ref().is_none_or(|v| v.is_empty()))
        {
            return Err(FreshConnectionError::Unauthorized);
        }
        if let Some(scope) = &tokens.scope {
            let required = match connector {
                "spotify" => "user-read-recently-played",
                "youtube" => "https://www.googleapis.com/auth/youtube.readonly",
                _ => "",
            };
            if !required.is_empty() && !scope.split_whitespace().any(|s| s == required) {
                return Err(FreshConnectionError::Unauthorized);
            }
        }
        Ok(tokens)
    }
    async fn get(&self, url: &str, token: &str) -> Result<Value, FreshConnectionError> {
        self.response(self.http.get(self.endpoint(url)).bearer_auth(token))
            .await
    }
    pub async fn snapshot(
        &self,
        connector: &str,
        access: &StoredAccess,
    ) -> Result<Snapshot, FreshConnectionError> {
        match connector {
            "spotify" => {
                let profile = self
                    .get("https://api.spotify.com/v1/me", &access.token)
                    .await?;
                let account = profile["id"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .ok_or(FreshConnectionError::Unauthorized)?
                    .to_owned();
                let data = self
                    .get(
                        "https://api.spotify.com/v1/me/player/recently-played?limit=50",
                        &access.token,
                    )
                    .await?;
                let playlists = self
                    .get(
                        "https://api.spotify.com/v1/me/playlists?limit=50",
                        &access.token,
                    )
                    .await?;
                let activities = parse_spotify(&account, &data)?;
                Ok(Snapshot {
                    account_id: account,
                    display_id: profile["display_name"].as_str().map(str::to_owned),
                    activities,
                    context: json!({"recently_played":data["items"],"playlists":playlists["items"],"complete":false,"history_window":"latest_50_provider_records"}),
                })
            }
            "youtube" => {
                let channels=self.get("https://www.googleapis.com/youtube/v3/channels?part=id,snippet,contentDetails&mine=true",&access.token).await?;
                let channel = channels["items"]
                    .as_array()
                    .and_then(|v| v.first())
                    .ok_or_else(|| {
                        FreshConnectionError::Provider("YouTube channel required".into())
                    })?;
                let account = channel["id"]
                    .as_str()
                    .ok_or(FreshConnectionError::Unauthorized)?
                    .to_owned();
                let playlists = self
                    .pages(
                        "playlists",
                        &[("part", "snippet,contentDetails"), ("mine", "true")],
                        &access.token,
                        4,
                    )
                    .await?;
                let subscriptions = self
                    .pages(
                        "subscriptions",
                        &[("part", "snippet"), ("mine", "true")],
                        &access.token,
                        4,
                    )
                    .await?;
                let mut activities = Vec::new();
                let mut playlist_records = Vec::new();
                let mut ids = playlists
                    .iter()
                    .filter_map(|p| {
                        p["id"]
                            .as_str()
                            .map(|id| (id.to_owned(), "playlist_addition"))
                    })
                    .take(20)
                    .collect::<Vec<_>>();
                if let Some(id) = channel
                    .pointer("/contentDetails/relatedPlaylists/likes")
                    .and_then(Value::as_str)
                {
                    ids.push((id.to_owned(), "like"));
                }
                for (id, kind) in ids {
                    let items = self
                        .pages(
                            "playlistItems",
                            &[("part", "snippet,contentDetails"), ("playlistId", &id)],
                            &access.token,
                            4,
                        )
                        .await?;
                    activities.extend(parse_youtube_playlist(&account, &id, kind, &items));
                    playlist_records.push(json!({"playlist_id":id,"action":kind,"items":items}));
                }
                Ok(Snapshot {
                    account_id: account,
                    display_id: channel
                        .pointer("/snippet/title")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    activities,
                    context: json!({"playlists":playlists,"playlist_items":playlist_records,"subscriptions":subscriptions,"complete":false,"watch_history_available":false,"subscriptions_are_snapshot":true}),
                })
            }
            _ => Err(FreshConnectionError::Invalid("Unknown provider".into())),
        }
    }
    async fn pages(
        &self,
        path: &str,
        params: &[(&str, &str)],
        token: &str,
        max_pages: usize,
    ) -> Result<Vec<Value>, FreshConnectionError> {
        let mut items = Vec::new();
        let mut next = String::new();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..max_pages {
            let mut url =
                url::Url::parse(&format!("https://www.googleapis.com/youtube/v3/{path}")).unwrap();
            url.query_pairs_mut()
                .extend_pairs(params.iter().copied())
                .append_pair("maxResults", "50");
            if !next.is_empty() {
                url.query_pairs_mut().append_pair("pageToken", &next);
            }
            let page = self.get(url.as_str(), token).await?;
            items.extend(
                page["items"]
                    .as_array()
                    .ok_or_else(|| {
                        FreshConnectionError::Provider("Invalid YouTube response".into())
                    })?
                    .iter()
                    .cloned(),
            );
            let Some(page_token) = page["nextPageToken"].as_str() else {
                break;
            };
            if !seen.insert(page_token.to_owned()) {
                return Err(FreshConnectionError::Provider(
                    "Invalid YouTube pagination".into(),
                ));
            }
            next = page_token.to_owned();
        }
        Ok(items)
    }
}
pub fn parse_spotify(
    account: &str,
    value: &Value,
) -> Result<Vec<PersonalActivity>, FreshConnectionError> {
    let items = value["items"]
        .as_array()
        .ok_or_else(|| FreshConnectionError::Provider("Invalid Spotify response".into()))?;
    Ok(items.iter().filter_map(|item|{let track=item.get("track")?;let id=track["id"].as_str()?;let time=DateTime::parse_from_rfc3339(item["played_at"].as_str()?).ok()?.with_timezone(&Utc);Some(PersonalActivity{source_id:format!("{account}:{id}:{}",time.to_rfc3339()),title:track["name"].as_str().unwrap_or("Music").to_owned(),occurred_at:time,ended_at:None,provider_data:json!({"action":"listen","account_id":account,"track_id":id,"artists":track["artists"],"album":track["album"],"context":item["context"],"reported_track_duration_ms":track["duration_ms"],"playback_end_known":false})})}).collect())
}
pub fn parse_youtube_playlist(
    account: &str,
    playlist: &str,
    kind: &str,
    items: &[Value],
) -> Vec<PersonalActivity> {
    items.iter().filter_map(|item|{let id=item["id"].as_str()?;let snippet=item.get("snippet")?;let video=snippet.pointer("/resourceId/videoId").and_then(Value::as_str)?;let time=DateTime::parse_from_rfc3339(snippet["publishedAt"].as_str()?).ok()?.with_timezone(&Utc);Some(PersonalActivity{source_id:format!("{account}:{playlist}:{id}"),title:snippet["title"].as_str().unwrap_or("YouTube playlist activity").to_owned(),occurred_at:time,ended_at:None,provider_data:json!({"action":kind,"account_id":account,"playlist_id":playlist,"video_id":video,"watch_event":false})})}).collect()
}
pub fn parse_youtube_history(
    history: &Value,
) -> Result<(Vec<PersonalActivity>, usize), FreshConnectionError> {
    let rows = history.as_array().ok_or_else(|| {
        FreshConnectionError::Invalid("Expected Google Takeout JSON watch history array".into())
    })?;
    if rows.len() > 20000 {
        return Err(FreshConnectionError::Invalid(
            "Import at most 20000 records at a time".into(),
        ));
    }
    let mut items = Vec::new();
    let mut skipped = 0;
    for row in rows {
        let parsed = (|| {
            let products = row["products"].as_array()?;
            if !products.iter().any(|p| p.as_str() == Some("YouTube")) {
                return None;
            }
            let raw = row["titleUrl"].as_str()?;
            let url = url::Url::parse(raw).ok()?;
            if url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some("www.youtube.com" | "youtube.com" | "m.youtube.com")
                )
                || url.path() != "/watch"
            {
                return None;
            }
            let video = url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned();
            if video.len() != 11
                || !video
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return None;
            }
            let time = DateTime::parse_from_rfc3339(row["time"].as_str()?)
                .ok()?
                .with_timezone(&Utc);
            if time > Utc::now() {
                return None;
            }
            let title = row["title"].as_str()?.trim();
            if !title.starts_with("Watched ") || title.len() > 2000 {
                return None;
            }
            Some(PersonalActivity {
                source_id: format!("takeout:{video}:{}", time.to_rfc3339()),
                title: title.strip_prefix("Watched ").unwrap_or(title).to_owned(),
                occurred_at: time,
                ended_at: None,
                provider_data: json!({"action":"watch","video_id":video,"source":"google_takeout","watch_event":true,"duration_known":false}),
            })
        })();
        match parsed {
            Some(item) => items.push(item),
            None => skipped += 1,
        }
    }
    Ok((items, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_uses_played_at_without_fabricated_end() {
        let data = json!({"items":[{"played_at":"2026-01-01T12:00:00Z","track":{"id":"track","name":"Song","duration_ms":600000}},{"track":{"id":"missing"}}]});
        let items = parse_spotify("account", &data).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ended_at, None);
        assert!(items[0].source_id.starts_with("account:track:"));
        assert!(parse_spotify("other", &data).unwrap()[0].source_id != items[0].source_id);
    }
    #[test]
    fn playlist_action_is_not_video_upload_or_watch() {
        let items = parse_youtube_playlist(
            "account",
            "playlist",
            "like",
            &[
                json!({"id":"item","snippet":{"publishedAt":"2026-01-02T00:00:00Z","title":"Video","resourceId":{"videoId":"vid"}},"contentDetails":{"videoPublishedAt":"2020-01-01T00:00:00Z"}}),
            ],
        );
        assert_eq!(
            items[0].occurred_at.to_rfc3339(),
            "2026-01-02T00:00:00+00:00"
        );
        assert_eq!(items[0].provider_data["watch_event"], false);
    }
    #[test]
    fn takeout_rejects_search_redirect_missing_time_and_wrong_product() {
        let valid = json!({"products":["YouTube"],"title":"Watched Video","titleUrl":"https://www.youtube.com/watch?v=abcdefghijk","time":"2026-01-01T12:00:00Z"});
        let (items,skipped)=parse_youtube_history(&json!([valid,{"products":["YouTube"],"title":"Search","titleUrl":"https://www.youtube.com/results?search_query=hi","time":"2026-01-01T12:00:00Z"},{"products":["Other"],"title":"Watched Video","titleUrl":"https://www.youtube.com/watch?v=abcdefghijk","time":"2026-01-01T12:00:00Z"}])).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(skipped, 2);
        assert_eq!(items[0].ended_at, None);
    }
    #[tokio::test]
    async fn invalid_grant_is_reconnect_required_and_body_is_not_exposed() {
        let app = axum::Router::new().fallback(|| async {
            (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(
                    json!({"error":"invalid_grant","error_description":"private-provider-detail"}),
                ),
            )
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut client = Client::new(reqwest::Client::new(), None, None);
        client.spotify_id = Some("client".into());
        client.test_origin = Some(origin);
        let result = client
            .tokens("spotify", None, "", "", Some("refresh"))
            .await;
        assert!(matches!(result, Err(FreshConnectionError::Unauthorized)));
        server.abort();
    }
    #[tokio::test]
    async fn oversized_provider_response_is_rejected() {
        let app = axum::Router::new().fallback(|| async { "x".repeat(8 * 1024 * 1024 + 1) });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut client = Client::new(reqwest::Client::new(), None, None);
        client.test_origin = Some(origin);
        assert!(
            matches!(client.get("https://api.spotify.com/v1/me","token").await,Err(FreshConnectionError::Provider(message)) if message=="Provider response too large")
        );
        server.abort();
    }
}
