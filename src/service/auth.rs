use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;

type HmacSha256 = Hmac<Sha256>;

pub const HEADER_SIGNATURE: &str = "x-vox-signature";
pub const HEADER_TIMESTAMP: &str = "x-vox-timestamp";
pub const HEADER_NONCE: &str = "x-vox-nonce";
pub const HEADER_KEY_ID: &str = "x-vox-key-id";

// Host assertion headers for backward/cross-compatibility
pub const HEADER_HOST_SIGNATURE: &str = "x-vox-host-signature";
pub const HEADER_HOST_TIMESTAMP: &str = "x-vox-host-timestamp";
pub const HEADER_HOST_NONCE: &str = "x-vox-host-nonce";
pub const HEADER_HOST_SECRET: &str = "x-vox-host-secret";
pub const HEADER_HOST_CREDENTIAL: &str = "x-vox-host-credential";

#[derive(Debug, thiserror::Error)]
pub enum HmacAuthError {
    #[error("missing required auth header: {0}")]
    MissingHeader(&'static str),
    #[error("invalid header format for: {0}")]
    InvalidHeaderFormat(&'static str),
    #[error("request timestamp expired: diff={diff_secs}s, max_skew={max_skew_secs}s")]
    TimestampExpired { diff_secs: i64, max_skew_secs: i64 },
    #[error("request timestamp is too far in future: diff={diff_secs}s, max_skew={max_skew_secs}s")]
    TimestampInFuture { diff_secs: i64, max_skew_secs: i64 },
    #[error("nonce has already been used (replay detected): {0}")]
    NonceReplayed(String),
    #[error("HMAC signature verification failed")]
    InvalidSignature,
    #[error("internal crypto error: {0}")]
    CryptoError(String),
}

pub struct HmacSigner;

impl HmacSigner {
    pub fn compute_body_hash(body: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(body);
        hex::encode(hasher.finalize())
    }

    pub fn canonical_message(
        method: &str,
        path: &str,
        timestamp: i64,
        nonce: &str,
        body_hash: &str,
    ) -> String {
        format!(
            "vox-hmac-v1:{}:{}:{}:{}:{}",
            method.to_ascii_uppercase(),
            path,
            timestamp,
            nonce,
            body_hash
        )
    }

    pub fn sign(
        secret: &[u8],
        method: &str,
        path: &str,
        body: &[u8],
        timestamp: i64,
        nonce: &str,
    ) -> Result<String, String> {
        let body_hash = Self::compute_body_hash(body);
        let canonical = Self::canonical_message(method, path, timestamp, nonce, &body_hash);

        let mut mac = HmacSha256::new_from_slice(secret)
            .map_err(|e| format!("Failed to create HMAC signer: {e}"))?;
        mac.update(canonical.as_bytes());
        Ok(hex::encode(mac.finalize().into_bytes()))
    }
}

#[derive(Clone)]
pub struct HmacVerifier {
    secret: Arc<[u8]>,
    max_clock_skew_seconds: i64,
    seen_nonces: Arc<RwLock<HashMap<String, i64>>>,
}

impl HmacVerifier {
    pub fn new(secret: impl AsRef<[u8]>, max_clock_skew_seconds: i64) -> Self {
        Self {
            secret: Arc::from(secret.as_ref()),
            max_clock_skew_seconds,
            seen_nonces: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn max_clock_skew_seconds(&self) -> i64 {
        self.max_clock_skew_seconds
    }

    pub async fn verify(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        headers: &HeaderMap,
        now: i64,
    ) -> Result<(), HmacAuthError> {
        // 1. Check for standard X-Vox-Signature headers
        if let Some(sig_val) = headers.get(HEADER_SIGNATURE) {
            let signature = sig_val
                .to_str()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_SIGNATURE))?;

            let ts_val = headers
                .get(HEADER_TIMESTAMP)
                .ok_or(HmacAuthError::MissingHeader(HEADER_TIMESTAMP))?
                .to_str()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_TIMESTAMP))?;
            let timestamp = ts_val
                .parse::<i64>()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_TIMESTAMP))?;

            let nonce_val = headers
                .get(HEADER_NONCE)
                .ok_or(HmacAuthError::MissingHeader(HEADER_NONCE))?
                .to_str()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_NONCE))?;

            return self
                .verify_signature(method, path, body, timestamp, nonce_val, signature, now)
                .await;
        }

        // 2. Fallback to X-Vox-Host-Signature headers if present
        if let Some(sig_val) = headers.get(HEADER_HOST_SIGNATURE) {
            let signature = sig_val
                .to_str()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_HOST_SIGNATURE))?;

            let ts_val = headers
                .get(HEADER_HOST_TIMESTAMP)
                .ok_or(HmacAuthError::MissingHeader(HEADER_HOST_TIMESTAMP))?
                .to_str()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_HOST_TIMESTAMP))?;
            let timestamp = ts_val
                .parse::<i64>()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_HOST_TIMESTAMP))?;

            let nonce_val = headers
                .get(HEADER_HOST_NONCE)
                .ok_or(HmacAuthError::MissingHeader(HEADER_HOST_NONCE))?
                .to_str()
                .map_err(|_| HmacAuthError::InvalidHeaderFormat(HEADER_HOST_NONCE))?;

            // If a host secret was supplied in headers, verify against that or the configured secret
            let secret_to_use: &[u8] = if let Some(sec) = headers.get(HEADER_HOST_SECRET) {
                sec.as_bytes()
            } else {
                &self.secret
            };

            return self
                .verify_signature_with_secret(
                    secret_to_use,
                    method,
                    path,
                    body,
                    timestamp,
                    nonce_val,
                    signature,
                    now,
                )
                .await;
        }

        Err(HmacAuthError::MissingHeader(HEADER_SIGNATURE))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn verify_signature(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        timestamp: i64,
        nonce: &str,
        signature: &str,
        now: i64,
    ) -> Result<(), HmacAuthError> {
        self.verify_signature_with_secret(
            &self.secret,
            method,
            path,
            body,
            timestamp,
            nonce,
            signature,
            now,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn verify_signature_with_secret(
        &self,
        secret: &[u8],
        method: &str,
        path: &str,
        body: &[u8],
        timestamp: i64,
        nonce: &str,
        signature: &str,
        now: i64,
    ) -> Result<(), HmacAuthError> {
        // Clock skew check
        let skew = now - timestamp;
        if skew > self.max_clock_skew_seconds {
            return Err(HmacAuthError::TimestampExpired {
                diff_secs: skew,
                max_skew_secs: self.max_clock_skew_seconds,
            });
        }
        if skew < -self.max_clock_skew_seconds {
            return Err(HmacAuthError::TimestampInFuture {
                diff_secs: -skew,
                max_skew_secs: self.max_clock_skew_seconds,
            });
        }

        // Nonce replay check: verify that this nonce has not been seen before
        {
            let nonces = self.seen_nonces.read().await;
            if nonces.contains_key(nonce) {
                return Err(HmacAuthError::NonceReplayed(nonce.to_string()));
            }
        }

        // Compute expected HMAC
        let body_hash = HmacSigner::compute_body_hash(body);
        let canonical = HmacSigner::canonical_message(method, path, timestamp, nonce, &body_hash);

        let mut mac = HmacSha256::new_from_slice(secret)
            .map_err(|e| HmacAuthError::CryptoError(e.to_string()))?;
        mac.update(canonical.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());

        // Constant-time compare
        if !bool::from(signature.as_bytes().ct_eq(expected.as_bytes())) {
            return Err(HmacAuthError::InvalidSignature);
        }

        // Signature is valid: record the nonce into seen_nonces to prevent replay
        {
            let mut nonces = self.seen_nonces.write().await;
            // Prune expired nonces periodically
            if nonces.len() > 1000 {
                nonces.retain(|_, expiry| *expiry > now);
            }
            if nonces.contains_key(nonce) {
                return Err(HmacAuthError::NonceReplayed(nonce.to_string()));
            }
            let expiry = timestamp + self.max_clock_skew_seconds;
            nonces.insert(nonce.to_string(), expiry);
        }

        Ok(())
    }
}

#[allow(clippy::result_large_err)]
pub async fn hmac_auth_middleware(
    State(verifier): State<Arc<HmacVerifier>>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    let (parts, body) = req.into_parts();
    let method = parts.method.as_str();
    let path = parts.uri.path();

    let bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({
                    "error": "payload_too_large",
                    "message": "Request payload exceeded size limit"
                })),
            )
                .into_response());
        }
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if let Err(err) = verifier
        .verify(method, path, &bytes, &parts.headers, now)
        .await
    {
        tracing::warn!(%method, %path, %err, "HMAC authentication failed");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized",
                "message": err.to_string()
            })),
        )
            .into_response());
    }

    let req = Request::from_parts(parts, axum::body::Body::from(bytes));
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hmac_signing_and_verification() {
        let secret = b"my-super-secret-key-1234567890";
        let verifier = HmacVerifier::new(secret, 300);

        let method = "POST";
        let path = "/v1/connections/list";
        let body = b"{\"user_id\":\"123\"}";
        let now = 1700000000;
        let nonce = "nonce-1";

        let signature =
            HmacSigner::sign(secret, method, path, body, now, nonce).expect("sign failed");

        let mut headers = HeaderMap::new();
        headers.insert(HEADER_SIGNATURE, signature.parse().unwrap());
        headers.insert(HEADER_TIMESTAMP, now.to_string().parse().unwrap());
        headers.insert(HEADER_NONCE, nonce.parse().unwrap());

        // Valid request passes
        let result = verifier.verify(method, path, body, &headers, now).await;
        assert!(result.is_ok());

        // Replaying same nonce fails
        let replay = verifier.verify(method, path, body, &headers, now).await;
        assert!(matches!(replay, Err(HmacAuthError::NonceReplayed(_))));

        // Tampered path fails
        let mut headers2 = HeaderMap::new();
        headers2.insert(HEADER_SIGNATURE, signature.parse().unwrap());
        headers2.insert(HEADER_TIMESTAMP, now.to_string().parse().unwrap());
        headers2.insert(HEADER_NONCE, "nonce-2".parse().unwrap());
        let tampered_path = verifier
            .verify("POST", "/v1/other/path", body, &headers2, now)
            .await;
        assert!(matches!(
            tampered_path,
            Err(HmacAuthError::InvalidSignature)
        ));

        // Tampered body fails
        let tampered_body = verifier
            .verify(method, path, b"tampered", &headers2, now)
            .await;
        assert!(matches!(
            tampered_body,
            Err(HmacAuthError::InvalidSignature)
        ));

        // Expired timestamp fails
        let mut headers_expired = HeaderMap::new();
        let old_time = now - 400;
        let sig_old =
            HmacSigner::sign(secret, method, path, body, old_time, "nonce-expired").unwrap();
        headers_expired.insert(HEADER_SIGNATURE, sig_old.parse().unwrap());
        headers_expired.insert(HEADER_TIMESTAMP, old_time.to_string().parse().unwrap());
        headers_expired.insert(HEADER_NONCE, "nonce-expired".parse().unwrap());
        let expired = verifier
            .verify(method, path, body, &headers_expired, now)
            .await;
        assert!(matches!(
            expired,
            Err(HmacAuthError::TimestampExpired { .. })
        ));
    }
}
