use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.readonly";
pub const USERINFO_EMAIL_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email";
pub const USERINFO_PROFILE_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.profile";

#[derive(Debug, thiserror::Error)]
pub enum GoogleCalendarError {
    #[error("network or request error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("google auth error: {0}")]
    Auth(String),
    #[error("token expired or unauthorized")]
    Unauthorized,
    #[error("invalid response: {0}")]
    Invalid(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoogleTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoogleProfile {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoogleCalendarEvent {
    pub id: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub time_zone: Option<String>,
    pub status: String,
    pub recurring_event_id: Option<String>,
    pub original_start: Option<Value>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub has_time: bool,
}

pub fn build_auth_url(client_id: &str, redirect_uri: &str, state: &str, verifier: &str) -> String {
    let scopes = format!("{CALENDAR_SCOPE} {USERINFO_EMAIL_SCOPE} {USERINFO_PROFILE_SCOPE}");
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &scopes)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", state)
        .append_pair("code_challenge", &crate::crypto::pkce_challenge(verifier))
        .append_pair("code_challenge_method", "S256")
        .finish();
    format!("https://accounts.google.com/o/oauth2/v2/auth?{query}")
}

async fn exchange_code_at(
    http: &reqwest::Client,
    endpoint: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<GoogleTokens, GoogleCalendarError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("code_verifier", verifier)
        .finish();

    let res = http
        .post(endpoint)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?;

    if !res.status().is_success() {
        let _ = res.bytes().await;
        return Err(GoogleCalendarError::Auth("Google request failed".into()));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
        scope: Option<String>,
    }

    let parsed: TokenResponse = res
        .json()
        .await
        .map_err(|e| GoogleCalendarError::Invalid(e.to_string()))?;

    if parsed
        .scope
        .as_deref()
        .is_some_and(|scope| !scope.split_whitespace().any(|s| s == CALENDAR_SCOPE))
    {
        return Err(GoogleCalendarError::Unauthorized);
    }
    if !(1..=86400).contains(&parsed.expires_in.unwrap_or(3600)) || parsed.access_token.is_empty() {
        return Err(GoogleCalendarError::Invalid(
            "Invalid token response".into(),
        ));
    }
    Ok(GoogleTokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_in: parsed.expires_in.unwrap_or(3600),
        scope: parsed.scope,
    })
}

async fn refresh_access_token_at(
    http: &reqwest::Client,
    endpoint: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<GoogleTokens, GoogleCalendarError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret)
        .append_pair("refresh_token", refresh_token)
        .append_pair("grant_type", "refresh_token")
        .finish();

    let res = http
        .post(endpoint)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?;

    if matches!(res.status().as_u16(), 400 | 401 | 403) {
        return Err(GoogleCalendarError::Unauthorized);
    }
    if !res.status().is_success() {
        let _ = res.bytes().await;
        return Err(GoogleCalendarError::Auth("Google request failed".into()));
    }

    #[derive(Deserialize)]
    struct RefreshResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
        scope: Option<String>,
    }

    let parsed: RefreshResponse = res
        .json()
        .await
        .map_err(|e| GoogleCalendarError::Invalid(e.to_string()))?;

    if parsed
        .scope
        .as_deref()
        .is_some_and(|scope| !scope.split_whitespace().any(|s| s == CALENDAR_SCOPE))
    {
        return Err(GoogleCalendarError::Unauthorized);
    }
    if !(1..=86400).contains(&parsed.expires_in.unwrap_or(3600)) || parsed.access_token.is_empty() {
        return Err(GoogleCalendarError::Invalid(
            "Invalid token response".into(),
        ));
    }
    Ok(GoogleTokens {
        access_token: parsed.access_token,
        refresh_token: parsed
            .refresh_token
            .or_else(|| Some(refresh_token.to_string())),
        expires_in: parsed.expires_in.unwrap_or(3600),
        scope: parsed.scope,
    })
}

async fn fetch_user_profile_at(
    http: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
) -> Result<GoogleProfile, GoogleCalendarError> {
    let res = http.get(endpoint).bearer_auth(access_token).send().await?;

    if matches!(res.status().as_u16(), 401 | 403) {
        return Err(GoogleCalendarError::Unauthorized);
    }
    if !res.status().is_success() {
        let _ = res.bytes().await;
        return Err(GoogleCalendarError::Auth("Google request failed".into()));
    }

    #[derive(Deserialize)]
    struct UserInfo {
        sub: String,
        email: Option<String>,
        name: Option<String>,
    }

    let parsed: UserInfo = res
        .json()
        .await
        .map_err(|e| GoogleCalendarError::Invalid(e.to_string()))?;

    if parsed.sub.is_empty() {
        return Err(GoogleCalendarError::Invalid(
            "Account identity is missing".into(),
        ));
    }
    Ok(GoogleProfile {
        id: parsed.sub,
        email: parsed.email,
        name: parsed.name,
    })
}

fn parse_event_time(val: Option<&Value>, calendar_zone: &str) -> Option<(DateTime<Utc>, bool)> {
    let obj = val?.as_object()?;
    if let Some(dt_str) = obj.get("dateTime").and_then(Value::as_str)
        && let Ok(dt) = DateTime::parse_from_rfc3339(dt_str)
    {
        return Some((dt.with_timezone(&Utc), false));
    }
    if let Some(date_str) = obj.get("date").and_then(Value::as_str)
        && let Ok(naive) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
    {
        let naive_dt = naive.and_hms_opt(0, 0, 0)?;
        let zone: chrono_tz::Tz = obj
            .get("timeZone")
            .and_then(Value::as_str)
            .unwrap_or(calendar_zone)
            .parse()
            .ok()?;
        return Some((
            zone.from_local_datetime(&naive_dt)
                .earliest()?
                .with_timezone(&Utc),
            true,
        ));
    }
    None
}

pub async fn fetch_events(
    http: &reqwest::Client,
    access_token: &str,
    time_min: DateTime<Utc>,
    time_max: DateTime<Utc>,
) -> Result<Vec<GoogleCalendarEvent>, GoogleCalendarError> {
    fetch_events_at(
        http,
        "https://www.googleapis.com/calendar/v3/calendars/primary/events",
        access_token,
        time_min,
        time_max,
    )
    .await
}
async fn fetch_events_at(
    http: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
    time_min: DateTime<Utc>,
    time_max: DateTime<Utc>,
) -> Result<Vec<GoogleCalendarEvent>, GoogleCalendarError> {
    let mut all_events = Vec::new();
    let mut page_token: Option<String> = None;

    for _ in 0..1000 {
        let mut req = http
            .get(endpoint)
            .bearer_auth(access_token)
            .query(&[
                ("timeMin", time_min.to_rfc3339()),
                ("timeMax", time_max.to_rfc3339()),
                ("singleEvents", "true".into()),
                ("orderBy", "startTime".into()),
                ("showDeleted", "true".into()),
                ("maxResults", "100".into()),
                (
                    "fields",
                    "nextPageToken,timeZone,items(id,summary,description,location,start,end,status,recurringEventId,originalStartTime)".into(),
                ),
            ]);

        if let Some(ref pt) = page_token {
            req = req.query(&[("pageToken", pt)]);
        }

        let res = req.send().await?;
        if matches!(res.status().as_u16(), 401 | 403) {
            return Err(GoogleCalendarError::Unauthorized);
        }
        if !res.status().is_success() {
            let _ = res.bytes().await;
            return Err(GoogleCalendarError::Auth("Google request failed".into()));
        }

        let body: Value = res
            .json()
            .await
            .map_err(|e| GoogleCalendarError::Invalid(e.to_string()))?;

        let time_zone = body
            .get("timeZone")
            .and_then(Value::as_str)
            .map(String::from);

        if let Some(items) = body.get("items").and_then(Value::as_array) {
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let summary = item
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("(Untitled Event)")
                    .to_string();
                let description = item
                    .get("description")
                    .and_then(Value::as_str)
                    .map(String::from);
                let location = item
                    .get("location")
                    .and_then(Value::as_str)
                    .map(String::from);
                let status = item
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("confirmed")
                    .to_string();
                let recurring_event_id = item
                    .get("recurringEventId")
                    .and_then(Value::as_str)
                    .map(String::from);

                let start_parsed =
                    parse_event_time(item.get("start"), time_zone.as_deref().unwrap_or("UTC"));
                let end_parsed =
                    parse_event_time(item.get("end"), time_zone.as_deref().unwrap_or("UTC"));

                let (start, all_day) = match start_parsed {
                    Some(s) => s,
                    None if status == "cancelled" => (time_min, false),
                    None => {
                        return Err(GoogleCalendarError::Invalid("Event time is missing".into()));
                    }
                };
                let end = match end_parsed {
                    Some((e, _)) => e,
                    None if status == "cancelled" => start,
                    None => {
                        return Err(GoogleCalendarError::Invalid("Event end is missing".into()));
                    }
                };

                all_events.push(GoogleCalendarEvent {
                    id: id.to_string(),
                    summary,
                    description,
                    location,
                    start,
                    end,
                    all_day,
                    time_zone: time_zone.clone(),
                    status,
                    recurring_event_id,
                    original_start: item.get("originalStartTime").cloned(),
                    start_date: item
                        .pointer("/start/date")
                        .and_then(Value::as_str)
                        .map(String::from),
                    has_time: start_parsed.is_some(),
                    end_date: item
                        .pointer("/end/date")
                        .and_then(Value::as_str)
                        .map(String::from),
                });
            }
        }

        page_token = body
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(String::from);

        if page_token.is_none() {
            return Ok(all_events);
        }
    }

    Err(GoogleCalendarError::Invalid(
        "Calendar pagination exceeds limit; reconciliation aborted".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_day_uses_calendar_zone_at_dst_boundary() {
        let date = serde_json::json!({"date":"2026-03-08"});
        let (start, all_day) = parse_event_time(Some(&date), "America/New_York").unwrap();
        assert!(all_day);
        assert_eq!(start.to_rfc3339(), "2026-03-08T05:00:00+00:00");
        let end = serde_json::json!({"date":"2026-03-09"});
        assert_eq!(
            (parse_event_time(Some(&end), "America/New_York").unwrap().0 - start).num_hours(),
            23
        );
    }
    #[test]
    fn authorization_binds_pkce_and_callback() {
        let url = url::Url::parse(&build_auth_url(
            "client",
            "https://core.example/callback",
            "state",
            "verifier",
        ))
        .unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(
            query.get("redirect_uri").unwrap(),
            "https://core.example/callback"
        );
    }
}

#[cfg(test)]
mod pagination_tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Query, State},
        http::StatusCode,
        routing::get,
    };
    use std::collections::HashMap;
    async fn page(
        State(fail): State<bool>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Result<Json<Value>, StatusCode> {
        let page: usize = query
            .get("pageToken")
            .map(|v| v.parse().unwrap())
            .unwrap_or(0);
        if fail && page == 5 {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let items:Vec<Value>=(0..150).map(|i|serde_json::json!({"id":format!("event-{}",page*150+i),"summary":"Recurring appointment","start":{"date":"2026-03-08"},"end":{"date":"2026-03-09"},"recurringEventId":"series","originalStartTime":{"date":"2026-03-08"}})).collect();
        let mut body = serde_json::json!({"timeZone":"America/New_York","items":items});
        if page < 10 {
            body["nextPageToken"] = Value::String((page + 1).to_string());
        }
        Ok(Json(body))
    }
    async fn server(fail: bool) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/events", get(page)).with_state(fail),
            )
            .await
            .unwrap()
        });
        (format!("http://{address}/events"), handle)
    }
    #[tokio::test]
    async fn all_pages_above_one_thousand_are_fetched_with_recurrence_and_dates() {
        let (url, handle) = server(false).await;
        let events = fetch_events_at(
            &reqwest::Client::new(),
            &url,
            "test",
            Utc::now(),
            Utc::now() + chrono::Duration::days(1),
        )
        .await
        .unwrap();
        handle.abort();
        assert_eq!(events.len(), 1650);
        assert_eq!(events.last().unwrap().id, "event-1649");
        assert_eq!(events[0].end - events[0].start, chrono::Duration::hours(23));
        assert_eq!(events[0].start_date.as_deref(), Some("2026-03-08"));
        assert_eq!(events[0].recurring_event_id.as_deref(), Some("series"));
        assert!(events[0].original_start.is_some());
    }
    #[tokio::test]
    async fn partial_page_failure_returns_no_reconcilable_result() {
        let (url, handle) = server(true).await;
        assert!(
            fetch_events_at(
                &reqwest::Client::new(),
                &url,
                "test",
                Utc::now(),
                Utc::now() + chrono::Duration::days(1)
            )
            .await
            .is_err()
        );
        handle.abort();
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    token_url: String,
    profile_url: String,
    events_url: String,
}
impl Client {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            token_url: "https://oauth2.googleapis.com/token".into(),
            profile_url: "https://www.googleapis.com/oauth2/v3/userinfo".into(),
            events_url: "https://www.googleapis.com/calendar/v3/calendars/primary/events".into(),
        }
    }
    pub async fn exchange(
        &self,
        client: &str,
        secret: &str,
        redirect: &str,
        code: &str,
        verifier: &str,
    ) -> Result<GoogleTokens, GoogleCalendarError> {
        exchange_code_at(
            &self.http,
            &self.token_url,
            client,
            secret,
            redirect,
            code,
            verifier,
        )
        .await
    }
    pub async fn refresh(
        &self,
        client: &str,
        secret: &str,
        refresh: &str,
    ) -> Result<GoogleTokens, GoogleCalendarError> {
        refresh_access_token_at(&self.http, &self.token_url, client, secret, refresh).await
    }
    pub async fn profile(&self, access: &str) -> Result<GoogleProfile, GoogleCalendarError> {
        fetch_user_profile_at(&self.http, &self.profile_url, access).await
    }
    pub async fn events(
        &self,
        access: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<GoogleCalendarEvent>, GoogleCalendarError> {
        fetch_events_at(&self.http, &self.events_url, access, start, end).await
    }
    #[cfg(test)]
    pub(crate) fn test_origin(&mut self, origin: &str) {
        self.token_url = format!("{origin}/token");
        self.profile_url = format!("{origin}/profile");
        self.events_url = format!("{origin}/events");
    }
}
