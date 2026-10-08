//! Expedia Rapid provider types and transport, reusable by any host.
use crate::integration_registry::{
    CapabilityDeclaration, CapabilityEffect, IntegrationProtocol, RegisterIntegrationRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Mutex;
use uuid::Uuid;

pub const EXPEDIA_INTEGRATION_KEY: &str = "expedia";
pub const EXPEDIA_CAPABILITY_LODGING_SEARCH: &str = "expedia.lodging_search";
pub const EXPEDIA_CAPABILITY_LODGING_BOOK: &str = "expedia.lodging_book";
pub const EXPEDIA_CAPABILITY_LODGING_MANAGE: &str = "expedia.lodging_manage";

pub const EXPEDIA_CAPABILITY_LODGING_SEARCH_SHORT: &str = "lodging_search";
pub const EXPEDIA_CAPABILITY_LODGING_BOOK_SHORT: &str = "lodging_book";
pub const EXPEDIA_CAPABILITY_LODGING_MANAGE_SHORT: &str = "lodging_manage";
/// Canonical integration declaration conforming to the E02 feasibility record.
pub fn integration_declaration(deployment_external_key: &str) -> RegisterIntegrationRequest {
    RegisterIntegrationRequest {
        deployment_external_key: deployment_external_key.into(),
        external_key: EXPEDIA_INTEGRATION_KEY.into(),
        protocol: IntegrationProtocol::Direct,
        display_name: "Expedia Rapid".into(),
        declaration_version: 1,
        capabilities: vec![
            CapabilityDeclaration {
                external_key: EXPEDIA_CAPABILITY_LODGING_SEARCH_SHORT.into(),
                effect: CapabilityEffect::Read,
                access_needs: vec!["search".into()],
                data_recipients: vec!["api.expediagroup.com".into()],
                regions: vec![
                    "US".into(),
                    "GB".into(),
                    "CA".into(),
                    "AU".into(),
                    "IN".into(),
                ],
                failure_modes: vec!["rate_limited".into()],
                optional_guarantees: json!({
                    "freshness_seconds": 60,
                    "capability_level": "L1_catalog_read",
                }),
            },
            CapabilityDeclaration {
                external_key: EXPEDIA_CAPABILITY_LODGING_BOOK_SHORT.into(),
                effect: CapabilityEffect::Write,
                access_needs: vec!["book".into()],
                data_recipients: vec!["api.expediagroup.com".into()],
                regions: vec![
                    "US".into(),
                    "GB".into(),
                    "CA".into(),
                    "AU".into(),
                    "IN".into(),
                ],
                failure_modes: vec![
                    "price_change".into(),
                    "inventory_unavailable".into(),
                    "rate_limited".into(),
                    "provider_authentication_required".into(),
                    "unknown_outcome".into(),
                ],
                optional_guarantees: json!({
                    "capability_level": "L3_consequential_write",
                    "idempotency_supported": true,
                    "reconciliation_supported": true,
                    "requires_platform_approval": true,
                }),
            },
            CapabilityDeclaration {
                external_key: EXPEDIA_CAPABILITY_LODGING_MANAGE_SHORT.into(),
                effect: CapabilityEffect::Write,
                access_needs: vec!["manage".into()],
                data_recipients: vec!["api.expediagroup.com".into()],
                regions: vec![
                    "US".into(),
                    "GB".into(),
                    "CA".into(),
                    "AU".into(),
                    "IN".into(),
                ],
                failure_modes: vec!["rate_limited".into(), "cancellation_penalty".into()],
                optional_guarantees: json!({
                    "capability_level": "L3_consequential_write",
                    "cancellation_supported": true,
                    "reconciliation_supported": true,
                }),
            },
        ],
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExpediaRawBookingRequest {
    pub affiliate_reference_id: String,
    pub property_id: String,
    pub room_type_id: String,
    pub rate_plan_id: String,
    pub checkin_date: String,
    pub checkout_date: String,
    pub primary_guest_name: String,
    pub primary_guest_email: String,
    pub total_price_amount_minor: i64,
    pub price_currency: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExpediaRawBookingResponse {
    pub itinerary_id: String,
    pub confirmation_reference: String,
    pub status: String,
    pub challenge_url: Option<String>,
    pub challenge_token: Option<String>,
    pub affiliate_reference_id: String,
    pub total_price_amount_minor: i64,
    pub price_currency: String,
    pub cancellation_penalty_minor: Option<i64>,
    pub refund_amount_minor: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ExpediaBookingOutcome {
    Succeeded {
        itinerary_id: String,
        confirmation_reference: String,
        booking_status: String,
        total_price_amount_minor: i64,
        price_currency: String,
    },
    AwaitingProviderAuthentication {
        challenge_url: String,
        challenge_token: String,
        affiliate_reference_id: String,
    },
    Failed {
        error_code: String,
        message: String,
    },
    Cancelled {
        itinerary_id: String,
        cancellation_reference: String,
        refund_amount_minor: i64,
        penalty_amount_minor: i64,
        currency: String,
    },
    Reconciling {
        affiliate_reference_id: String,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExpediaCancellationResult {
    pub itinerary_id: String,
    pub cancellation_reference: String,
    pub refund_amount_minor: i64,
    pub penalty_amount_minor: i64,
    pub currency: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExpediaLodgingError {
    #[error("connection not found")]
    ConnectionNotFound,
    #[error("connection is not authorized for integration expedia")]
    InvalidIntegration,
    #[error("connection expired or revoked; reconnection required")]
    ReconnectRequired,
    #[error("capability {0} is not granted to agent {1}")]
    UnauthorizedCapability(String, String),
    #[error("proposal request invalid: {0}")]
    InvalidProposal(String),
    #[error("proposal not found")]
    ProposalNotFound,
    #[error("proposal has expired")]
    ProposalExpired,
    #[error("proposal has not been approved or details hash mismatch")]
    NotApproved,
    #[error("execution error: {0}")]
    ExecutionFailed(String),
    #[error("rate limited by provider; retry after {0} seconds")]
    RateLimited(u64),
    #[error("provider error: {0}")]
    ProviderError(String),
    #[error("network timeout during provider execution")]
    Timeout,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[async_trait::async_trait]
pub trait ExpediaProviderClient: Send + Sync {
    async fn create_booking(
        &self,
        request: &ExpediaRawBookingRequest,
    ) -> Result<ExpediaRawBookingResponse, ExpediaLodgingError>;

    async fn retrieve_booking(
        &self,
        affiliate_reference_id: &str,
    ) -> Result<Option<ExpediaRawBookingResponse>, ExpediaLodgingError>;

    async fn cancel_booking(
        &self,
        itinerary_id: &str,
        reason: &str,
    ) -> Result<ExpediaRawBookingResponse, ExpediaLodgingError>;
}

pub struct DefaultExpediaProviderClient {
    base_url: String,
    http: reqwest::Client,
}

impl DefaultExpediaProviderClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl ExpediaProviderClient for DefaultExpediaProviderClient {
    async fn create_booking(
        &self,
        request: &ExpediaRawBookingRequest,
    ) -> Result<ExpediaRawBookingResponse, ExpediaLodgingError> {
        let url = format!("{}/v3/lodging/bookings", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ExpediaLodgingError::Timeout
                } else {
                    ExpediaLodgingError::ProviderError(e.to_string())
                }
            })?;

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);
            return Err(ExpediaLodgingError::RateLimited(retry));
        }

        if !resp.status().is_success() {
            return Err(ExpediaLodgingError::ProviderError(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        resp.json::<ExpediaRawBookingResponse>()
            .await
            .map_err(|e| ExpediaLodgingError::ProviderError(e.to_string()))
    }

    async fn retrieve_booking(
        &self,
        affiliate_reference_id: &str,
    ) -> Result<Option<ExpediaRawBookingResponse>, ExpediaLodgingError> {
        let url = format!(
            "{}/v3/lodging/bookings?affiliate_reference_id={}",
            self.base_url, affiliate_reference_id
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExpediaLodgingError::ProviderError(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(ExpediaLodgingError::ProviderError(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        resp.json::<ExpediaRawBookingResponse>()
            .await
            .map(Some)
            .map_err(|e| ExpediaLodgingError::ProviderError(e.to_string()))
    }

    async fn cancel_booking(
        &self,
        itinerary_id: &str,
        reason: &str,
    ) -> Result<ExpediaRawBookingResponse, ExpediaLodgingError> {
        let url = format!(
            "{}/v3/lodging/bookings/{}/cancel",
            self.base_url, itinerary_id
        );
        let resp = self
            .http
            .post(&url)
            .json(&json!({ "reason": reason }))
            .send()
            .await
            .map_err(|e| ExpediaLodgingError::ProviderError(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ExpediaLodgingError::ProviderError(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        resp.json::<ExpediaRawBookingResponse>()
            .await
            .map_err(|e| ExpediaLodgingError::ProviderError(e.to_string()))
    }
}

pub struct MockExpediaProviderClient {
    pub bookings: Mutex<std::collections::HashMap<String, ExpediaRawBookingResponse>>,
    pub fail_with_rate_limit: std::sync::atomic::AtomicBool,
    pub fail_with_timeout: std::sync::atomic::AtomicBool,
    pub require_3ds: std::sync::atomic::AtomicBool,
}

impl MockExpediaProviderClient {
    pub fn new() -> Self {
        Self {
            bookings: Mutex::new(std::collections::HashMap::new()),
            fail_with_rate_limit: std::sync::atomic::AtomicBool::new(false),
            fail_with_timeout: std::sync::atomic::AtomicBool::new(false),
            require_3ds: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Default for MockExpediaProviderClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ExpediaProviderClient for MockExpediaProviderClient {
    async fn create_booking(
        &self,
        request: &ExpediaRawBookingRequest,
    ) -> Result<ExpediaRawBookingResponse, ExpediaLodgingError> {
        if self
            .fail_with_rate_limit
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(ExpediaLodgingError::RateLimited(45));
        }
        if self
            .fail_with_timeout
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(ExpediaLodgingError::Timeout);
        }

        let mut store = self.bookings.lock().unwrap();
        if let Some(existing) = store.get(&request.affiliate_reference_id) {
            return Ok(existing.clone());
        }

        let is_3ds = self.require_3ds.load(std::sync::atomic::Ordering::SeqCst);
        let resp = if is_3ds {
            ExpediaRawBookingResponse {
                itinerary_id: format!("itin-{}", Uuid::new_v4()),
                confirmation_reference: format!("CONF-{}", Uuid::new_v4()),
                status: "pending_authentication".into(),
                challenge_url: Some("https://pay.expedia.com/3ds-challenge/v1".into()),
                challenge_token: Some("token_3ds_challenge_xyz".into()),
                affiliate_reference_id: request.affiliate_reference_id.clone(),
                total_price_amount_minor: request.total_price_amount_minor,
                price_currency: request.price_currency.clone(),
                cancellation_penalty_minor: None,
                refund_amount_minor: None,
            }
        } else {
            ExpediaRawBookingResponse {
                itinerary_id: format!("itin-{}", Uuid::new_v4()),
                confirmation_reference: format!("CONF-{}", Uuid::new_v4()),
                status: "booked".into(),
                challenge_url: None,
                challenge_token: None,
                affiliate_reference_id: request.affiliate_reference_id.clone(),
                total_price_amount_minor: request.total_price_amount_minor,
                price_currency: request.price_currency.clone(),
                cancellation_penalty_minor: None,
                refund_amount_minor: None,
            }
        };

        store.insert(request.affiliate_reference_id.clone(), resp.clone());
        Ok(resp)
    }

    async fn retrieve_booking(
        &self,
        affiliate_reference_id: &str,
    ) -> Result<Option<ExpediaRawBookingResponse>, ExpediaLodgingError> {
        let store = self.bookings.lock().unwrap();
        Ok(store.get(affiliate_reference_id).cloned())
    }

    async fn cancel_booking(
        &self,
        itinerary_id: &str,
        _reason: &str,
    ) -> Result<ExpediaRawBookingResponse, ExpediaLodgingError> {
        let mut store = self.bookings.lock().unwrap();
        for resp in store.values_mut() {
            if resp.itinerary_id == itinerary_id {
                resp.status = "cancelled".into();
                resp.cancellation_penalty_minor = Some(5_000); // $50 penalty
                resp.refund_amount_minor = Some(resp.total_price_amount_minor - 5_000);
                return Ok(resp.clone());
            }
        }
        Err(ExpediaLodgingError::ProviderError(
            "itinerary not found".into(),
        ))
    }
}
