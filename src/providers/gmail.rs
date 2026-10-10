use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
pub const USERINFO_EMAIL_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email";
pub const USERINFO_PROFILE_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.profile";

#[derive(Debug, thiserror::Error)]
pub enum GmailError {
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
pub struct GmailTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GmailProfile {
    pub email_address: String,
    pub history_id: u64,
    pub messages_total: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GmailWatchResponse {
    pub history_id: u64,
    pub expiration: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GmailAttachmentMeta {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GmailMessageItem {
    pub id: String,
    pub thread_id: String,
    pub history_id: u64,
    pub internal_date: i64,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub snippet: Option<String>,
    pub body_text: Option<String>,
    pub attachments: Vec<GmailAttachmentMeta>,
}

pub fn build_auth_url(client_id: &str, redirect_uri: &str, state: &str, verifier: &str) -> String {
    let scopes = format!("{GMAIL_SCOPE} {USERINFO_EMAIL_SCOPE} {USERINFO_PROFILE_SCOPE}");
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

#[derive(Clone)]
pub struct GmailClient {
    http: reqwest::Client,
    token_url: String,
    profile_url: String,
    messages_url: String,
    history_url: String,
    watch_url: String,
    stop_url: String,
    revoke_url: String,
}

impl Default for GmailClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GmailClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            profile_url: "https://gmail.googleapis.com/gmail/v1/users/me/profile".into(),
            messages_url: "https://gmail.googleapis.com/gmail/v1/users/me/messages".into(),
            history_url: "https://gmail.googleapis.com/gmail/v1/users/me/history".into(),
            watch_url: "https://gmail.googleapis.com/gmail/v1/users/me/watch".into(),
            stop_url: "https://gmail.googleapis.com/gmail/v1/users/me/stop".into(),
            revoke_url: "https://oauth2.googleapis.com/revoke".into(),
        }
    }

    pub async fn exchange_code(
        &self,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> Result<GmailTokens, GmailError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", client_id)
            .append_pair("client_secret", client_secret)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("grant_type", "authorization_code")
            .append_pair("code", code)
            .append_pair("code_verifier", verifier)
            .finish();

        let res = self
            .http
            .post(&self.token_url)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await?;

        if !res.status().is_success() {
            let _ = res.bytes().await;
            return Err(GmailError::Auth("Google token exchange failed".into()));
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
            scope: Option<String>,
        }

        let parsed: TokenResp = res
            .json()
            .await
            .map_err(|e| GmailError::Invalid(e.to_string()))?;

        if parsed
            .scope
            .as_deref()
            .is_some_and(|scope| !scope.split_whitespace().any(|s| s == GMAIL_SCOPE))
        {
            return Err(GmailError::Unauthorized);
        }

        if !(1..=86400).contains(&parsed.expires_in.unwrap_or(3600)) || parsed.access_token.is_empty() {
            return Err(GmailError::Invalid("Invalid token response".into()));
        }

        Ok(GmailTokens {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token,
            expires_in: parsed.expires_in.unwrap_or(3600),
            scope: parsed.scope,
        })
    }

    pub async fn refresh_access_token(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> Result<GmailTokens, GmailError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", client_id)
            .append_pair("client_secret", client_secret)
            .append_pair("refresh_token", refresh_token)
            .append_pair("grant_type", "refresh_token")
            .finish();

        let res = self
            .http
            .post(&self.token_url)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await?;

        if matches!(res.status().as_u16(), 400 | 401 | 403) {
            return Err(GmailError::Unauthorized);
        }
        if !res.status().is_success() {
            let _ = res.bytes().await;
            return Err(GmailError::Auth("Google token refresh failed".into()));
        }

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            expires_in: Option<i64>,
            scope: Option<String>,
        }

        let parsed: TokenResp = res
            .json()
            .await
            .map_err(|e| GmailError::Invalid(e.to_string()))?;

        if parsed.access_token.is_empty() {
            return Err(GmailError::Invalid("Invalid refreshed token".into()));
        }

        Ok(GmailTokens {
            access_token: parsed.access_token,
            refresh_token: Some(refresh_token.to_string()),
            expires_in: parsed.expires_in.unwrap_or(3600),
            scope: parsed.scope,
        })
    }

    pub async fn revoke_token(&self, token: &str) -> Result<(), GmailError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("token", token)
            .finish();

        let _ = self
            .http
            .post(&self.revoke_url)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await;
        Ok(())
    }

    pub async fn get_profile(&self, access_token: &str) -> Result<GmailProfile, GmailError> {
        let res = self
            .http
            .get(&self.profile_url)
            .bearer_auth(access_token)
            .send()
            .await?;

        if matches!(res.status().as_u16(), 401 | 403) {
            return Err(GmailError::Unauthorized);
        }
        if !res.status().is_success() {
            return Err(GmailError::Auth("Profile request failed".into()));
        }

        #[derive(Deserialize)]
        struct RawProfile {
            #[serde(rename = "emailAddress")]
            email_address: String,
            #[serde(rename = "historyId")]
            history_id: Value,
            #[serde(rename = "messagesTotal")]
            messages_total: Option<u64>,
        }

        let raw: RawProfile = res
            .json()
            .await
            .map_err(|e| GmailError::Invalid(e.to_string()))?;

        let hid = match raw.history_id {
            Value::Number(n) => n.as_u64().unwrap_or(0),
            Value::String(s) => s.parse::<u64>().unwrap_or(0),
            _ => 0,
        };

        Ok(GmailProfile {
            email_address: raw.email_address,
            history_id: hid,
            messages_total: raw.messages_total,
        })
    }

    pub async fn setup_watch(
        &self,
        access_token: &str,
        topic_name: &str,
    ) -> Result<GmailWatchResponse, GmailError> {
        let payload = serde_json::json!({
            "topicName": topic_name,
            "labelIds": ["INBOX"]
        });

        let res = self
            .http
            .post(&self.watch_url)
            .bearer_auth(access_token)
            .json(&payload)
            .send()
            .await?;

        if matches!(res.status().as_u16(), 401 | 403) {
            return Err(GmailError::Unauthorized);
        }
        if !res.status().is_success() {
            return Err(GmailError::Auth("Gmail watch setup failed".into()));
        }

        #[derive(Deserialize)]
        struct RawWatch {
            #[serde(rename = "historyId")]
            history_id: Value,
            expiration: Value,
        }

        let raw: RawWatch = res
            .json()
            .await
            .map_err(|e| GmailError::Invalid(e.to_string()))?;

        let hid = match raw.history_id {
            Value::Number(n) => n.as_u64().unwrap_or(0),
            Value::String(s) => s.parse::<u64>().unwrap_or(0),
            _ => 0,
        };
        let exp = match raw.expiration {
            Value::Number(n) => n.as_i64().unwrap_or(0),
            Value::String(s) => s.parse::<i64>().unwrap_or(0),
            _ => 0,
        };

        Ok(GmailWatchResponse {
            history_id: hid,
            expiration: exp,
        })
    }

    pub async fn stop_watch(&self, access_token: &str) -> Result<(), GmailError> {
        let _ = self
            .http
            .post(&self.stop_url)
            .bearer_auth(access_token)
            .send()
            .await;
        Ok(())
    }

    pub async fn list_history(
        &self,
        access_token: &str,
        start_history_id: u64,
    ) -> Result<(Vec<String>, u64), GmailError> {
        let mut page_token: Option<String> = None;
        let mut message_ids = Vec::new();
        let mut final_history_id = start_history_id;

        loop {
            let mut url = format!(
                "{}?startHistoryId={}&historyTypes=messageAdded",
                self.history_url, start_history_id
            );
            if let Some(ref pt) = page_token {
                url.push_str("&pageToken=");
                url.push_str(pt);
            }

            let res = self
                .http
                .get(&url)
                .bearer_auth(access_token)
                .send()
                .await?;

            if matches!(res.status().as_u16(), 401 | 403) {
                return Err(GmailError::Unauthorized);
            }
            if res.status().as_u16() == 404 {
                return Err(GmailError::Invalid("History ID expired".into()));
            }
            if !res.status().is_success() {
                return Err(GmailError::Auth("History request failed".into()));
            }

            let raw: Value = res
                .json()
                .await
                .map_err(|e| GmailError::Invalid(e.to_string()))?;

            if let Some(hid) = raw.get("historyId") {
                let parsed_hid = match hid {
                    Value::Number(n) => n.as_u64().unwrap_or(final_history_id),
                    Value::String(s) => s.parse::<u64>().unwrap_or(final_history_id),
                    _ => final_history_id,
                };
                if parsed_hid > final_history_id {
                    final_history_id = parsed_hid;
                }
            }

            if let Some(history_records) = raw.get("history").and_then(Value::as_array) {
                for record in history_records {
                    if let Some(added) = record.get("messagesAdded").and_then(Value::as_array) {
                        for item in added {
                            if let Some(msg_id) = item.pointer("/message/id").and_then(Value::as_str) {
                                message_ids.push(msg_id.to_string());
                            }
                        }
                    }
                }
            }

            match raw.get("nextPageToken").and_then(Value::as_str) {
                Some(tok) if !tok.is_empty() => {
                    page_token = Some(tok.to_string());
                }
                _ => break,
            }
        }

        message_ids.sort();
        message_ids.dedup();

        Ok((message_ids, final_history_id))
    }

    pub async fn list_messages_after(
        &self,
        access_token: &str,
        after_timestamp_secs: i64,
    ) -> Result<Vec<String>, GmailError> {
        let mut page_token: Option<String> = None;
        let mut ids = Vec::new();

        loop {
            let mut url = format!(
                "{}?q=after:{}",
                self.messages_url, after_timestamp_secs
            );
            if let Some(ref pt) = page_token {
                url.push_str("&pageToken=");
                url.push_str(pt);
            }

            let res = self
                .http
                .get(&url)
                .bearer_auth(access_token)
                .send()
                .await?;

            if matches!(res.status().as_u16(), 401 | 403) {
                return Err(GmailError::Unauthorized);
            }
            if !res.status().is_success() {
                return Err(GmailError::Auth("List messages failed".into()));
            }

            let raw: Value = res
                .json()
                .await
                .map_err(|e| GmailError::Invalid(e.to_string()))?;

            if let Some(messages) = raw.get("messages").and_then(Value::as_array) {
                for m in messages {
                    if let Some(id) = m.get("id").and_then(Value::as_str) {
                        ids.push(id.to_string());
                    }
                }
            }

            match raw.get("nextPageToken").and_then(Value::as_str) {
                Some(tok) if !tok.is_empty() => {
                    page_token = Some(tok.to_string());
                }
                _ => break,
            }
        }

        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    pub async fn get_message(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<GmailMessageItem, GmailError> {
        let url = format!("{}/{}?format=full", self.messages_url, message_id);

        let res = self
            .http
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?;

        if matches!(res.status().as_u16(), 401 | 403) {
            return Err(GmailError::Unauthorized);
        }
        if !res.status().is_success() {
            return Err(GmailError::Auth("Message request failed".into()));
        }

        let raw: Value = res
            .json()
            .await
            .map_err(|e| GmailError::Invalid(e.to_string()))?;

        let id = raw.get("id").and_then(Value::as_str).unwrap_or(message_id).to_string();
        let thread_id = raw.get("threadId").and_then(Value::as_str).unwrap_or("").to_string();
        let history_id = match raw.get("historyId") {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
            Some(Value::String(s)) => s.parse::<u64>().unwrap_or(0),
            _ => 0,
        };
        let internal_date = raw
            .get("internalDate")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let snippet = raw.get("snippet").and_then(Value::as_str).map(|s| s.to_string());

        let mut from = None;
        let mut to = None;
        let mut subject = None;
        let mut date = None;

        if let Some(headers) = raw.pointer("/payload/headers").and_then(Value::as_array) {
            for h in headers {
                let name = h.get("name").and_then(Value::as_str).unwrap_or("");
                let value = h.get("value").and_then(Value::as_str).unwrap_or("");
                if name.eq_ignore_ascii_case("from") {
                    from = Some(value.to_string());
                } else if name.eq_ignore_ascii_case("to") {
                    to = Some(value.to_string());
                } else if name.eq_ignore_ascii_case("subject") {
                    subject = Some(value.to_string());
                } else if name.eq_ignore_ascii_case("date") {
                    date = Some(value.to_string());
                }
            }
        }

        let mut body_text = None;
        let mut attachments = Vec::new();

        if let Some(payload) = raw.get("payload") {
            extract_parts(payload, &mut body_text, &mut attachments);
        }

        Ok(GmailMessageItem {
            id,
            thread_id,
            history_id,
            internal_date,
            from,
            to,
            subject,
            date,
            snippet,
            body_text,
            attachments,
        })
    }

    pub async fn get_attachment(
        &self,
        access_token: &str,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<u8>, GmailError> {
        let url = format!(
            "{}/{}/attachments/{}",
            self.messages_url, message_id, attachment_id
        );

        let res = self
            .http
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?;

        if matches!(res.status().as_u16(), 401 | 403) {
            return Err(GmailError::Unauthorized);
        }
        if !res.status().is_success() {
            return Err(GmailError::Auth("Attachment request failed".into()));
        }

        let raw: Value = res
            .json()
            .await
            .map_err(|e| GmailError::Invalid(e.to_string()))?;

        let data_b64 = raw
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| GmailError::Invalid("Missing attachment data".into()))?;

        let sanitized = data_b64.replace('-', "+").replace('_', "/");
        let padded = match sanitized.len() % 4 {
            2 => format!("{sanitized}=="),
            3 => format!("{sanitized}="),
            _ => sanitized,
        };

        base64::engine::general_purpose::STANDARD
            .decode(padded.as_bytes())
            .map_err(|e| GmailError::Invalid(format!("Base64 decode failed: {e}")))
    }
}

fn extract_parts(
    part: &Value,
    body_text: &mut Option<String>,
    attachments: &mut Vec<GmailAttachmentMeta>,
) {
    let filename = part.get("filename").and_then(Value::as_str).unwrap_or("");
    let mime_type = part.get("mimeType").and_then(Value::as_str).unwrap_or("");

    if let Some(body) = part.get("body") {
        if let Some(att_id) = body.get("attachmentId").and_then(Value::as_str) {
            let size = body.get("size").and_then(Value::as_u64).unwrap_or(0);
            attachments.push(GmailAttachmentMeta {
                attachment_id: att_id.to_string(),
                filename: filename.to_string(),
                mime_type: mime_type.to_string(),
                size_bytes: size,
            });
        } else if let Some(data) = body.get("data").and_then(Value::as_str) {
            if mime_type == "text/plain" && body_text.is_none() {
                let sanitized = data.replace('-', "+").replace('_', "/");
                let padded = match sanitized.len() % 4 {
                    2 => format!("{sanitized}=="),
                    3 => format!("{sanitized}="),
                    _ => sanitized,
                };
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(padded.as_bytes()) {
                    *body_text = Some(String::from_utf8_lossy(&bytes).to_string());
                }
            }
        }
    }

    if let Some(subparts) = part.get("parts").and_then(Value::as_array) {
        for sub in subparts {
            extract_parts(sub, body_text, attachments);
        }
    }
}
