use super::playstation::{
    PlayStationError, PlayStationGameHistory, parse_history_titles_from_json,
};
use serde::Deserialize;
use serde_json::Value;
const AUTH: &str = "https://ca.account.sony.com/api/authz/v3/oauth";
const API: &str = "https://m.np.playstation.com";
const REDIRECT: &str = "com.scee.psxandroid.scecompcall://redirect";
const CLIENT_ID: &str = "09515159-7237-4370-9b40-3806e67c0891";
const CLIENT_AUTH: &str =
    "Basic MDk1MTUxNTktNzIzNy00MzcwLTliNDAtMzgwNmU2N2MwODkxOnVjUGprYTV0bnRCMktxc1A=";

#[derive(Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub refresh_token_expires_in: i64,
    pub id_token: Option<String>,
    pub scope: String,
}

pub struct VerifiedAccount {
    pub account_id: String,
    pub online_id: String,
    pub tokens: Tokens,
    pub games: Vec<PlayStationGameHistory>,
}
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    auth_origin: String,
    api_origin: String,
}
impl Client {
    pub fn new() -> Result<Self, PlayStationError> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| PlayStationError::NotConfigured)?,
            auth_origin: AUTH.into(),
            api_origin: API.into(),
        })
    }
    #[cfg(test)]
    pub(crate) fn test_origin(&mut self, origin: &str) {
        self.auth_origin = origin.into();
        self.api_origin = origin.into();
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

    pub async fn link(&self, npsso: &str) -> Result<VerifiedAccount, PlayStationError> {
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
        let games = self.games(&tokens.access_token).await?;
        Ok(VerifiedAccount {
            account_id,
            online_id: online_id.to_string(),
            tokens,
            games,
        })
    }
    pub async fn refresh(&self, refresh: &str) -> Result<Tokens, PlayStationError> {
        self.exchange(&[
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
            ("token_format", "jwt"),
            ("scope", "psn:mobile.v2.core psn:clientapp"),
        ])
        .await
    }
    pub async fn games(
        &self,
        token: &str,
    ) -> Result<Vec<PlayStationGameHistory>, PlayStationError> {
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
            games.extend(parse_history_titles_from_json(&body)?);
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{StatusCode, header},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    async fn authorize() -> impl IntoResponse {
        (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                "com.scee.psxandroid.scecompcall://redirect?code=verified-code",
            )],
        )
    }
    async fn token() -> Json<Value> {
        Json(
            serde_json::json!({"access_token":"access","refresh_token":"refresh","expires_in":3600,"refresh_token_expires_in":86400,"id_token":format!("header.{}.signature",URL_SAFE_NO_PAD.encode(br#"{"sub":"12345"}"#)),"scope":"psn:mobile.v2.core"}),
        )
    }
    async fn profile(State(mode): State<u8>) -> Json<Value> {
        Json(serde_json::json!({"isMe":mode!=1,"onlineId":"VerifiedPlayer"}))
    }
    async fn games(State(mode): State<u8>) -> Response {
        if mode == 2 {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        } else {
            Json(serde_json::json!({"titles":[]})).into_response()
        }
    }
    async fn fixture(mode: u8) -> (Client, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/authorize", get(authorize))
                    .route("/token", post(token))
                    .route(
                        "/api/userProfile/v1/internal/users/12345/profiles",
                        get(profile),
                    )
                    .route("/api/gamelist/v2/users/me/titles", get(games))
                    .with_state(mode),
            )
            .await
            .unwrap()
        });
        let mut client = Client::new().unwrap();
        client.auth_origin = origin.clone();
        client.api_origin = origin;
        (client, handle)
    }
    #[tokio::test]
    async fn verified_link_uses_custom_redirect_without_following_it() {
        let (client, handle) = fixture(0).await;
        let account = client.link(&"a".repeat(64)).await.unwrap();
        handle.abort();
        assert_eq!(account.account_id, "12345");
        assert_eq!(account.online_id, "VerifiedPlayer");
    }
    #[tokio::test]
    async fn rejects_unverified_identity_and_failed_baseline() {
        for mode in [1, 2] {
            let (client, handle) = fixture(mode).await;
            assert!(client.link(&"a".repeat(64)).await.is_err());
            handle.abort();
        }
    }
}
