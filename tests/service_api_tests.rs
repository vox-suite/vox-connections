use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;
use uuid::Uuid;
use vox_connections::{
    identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId},
    service::{
        ServiceState,
        auth::{HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP, HmacSigner, HmacVerifier},
        build_service_router,
    },
};

fn sample_context() -> RequestContext {
    RequestContext {
        id: UserContextId(Uuid::new_v4()),
        user_id: UserId(Uuid::new_v4()),
        subject: RequestSubject {
            deployment_id: DeploymentId(Uuid::new_v4()),
        },
    }
}

fn test_secret() -> &'static str {
    "test-secret-key-1234567890-connections-service"
}

#[tokio::test]
async fn test_health_endpoints_do_not_require_hmac() {
    let verifier = Arc::new(HmacVerifier::new(test_secret().as_bytes(), 300));
    let state = ServiceState::for_test(verifier);
    let app = build_service_router(state);

    // /health/live
    let req = Request::builder()
        .uri("/health/live")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // /health/ready
    let req = Request::builder()
        .uri("/health/ready")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_endpoints_reject_missing_hmac() {
    let verifier = Arc::new(HmacVerifier::new(test_secret().as_bytes(), 300));
    let state = ServiceState::for_test(verifier);
    let app = build_service_router(state);

    let ctx = sample_context();
    let body = serde_json::to_vec(&json!({ "context": ctx })).unwrap();

    let req = Request::builder()
        .uri("/v1/connections/list")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_endpoints_reject_invalid_hmac_signature() {
    let verifier = Arc::new(HmacVerifier::new(test_secret().as_bytes(), 300));
    let state = ServiceState::for_test(verifier);
    let app = build_service_router(state);

    let ctx = sample_context();
    let body = serde_json::to_vec(&json!({ "context": ctx })).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let nonce = Uuid::new_v4().to_string();

    let req = Request::builder()
        .uri("/v1/connections/list")
        .method("POST")
        .header("content-type", "application/json")
        .header(HEADER_SIGNATURE, "invalid-deadbeef-signature-0000000000")
        .header(HEADER_TIMESTAMP, now.to_string())
        .header(HEADER_NONCE, nonce)
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_endpoints_reject_expired_hmac_timestamp() {
    let verifier = Arc::new(HmacVerifier::new(test_secret().as_bytes(), 300));
    let state = ServiceState::for_test(verifier);
    let app = build_service_router(state);

    let ctx = sample_context();
    let body = serde_json::to_vec(&json!({ "context": ctx })).unwrap();
    // 400 seconds in the past (> 300s skew limit)
    let old_time = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64)
        - 400;
    let nonce = Uuid::new_v4().to_string();

    let signature = HmacSigner::sign(
        test_secret().as_bytes(),
        "POST",
        "/v1/connections/list",
        &body,
        old_time,
        &nonce,
    )
    .unwrap();

    let req = Request::builder()
        .uri("/v1/connections/list")
        .method("POST")
        .header("content-type", "application/json")
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_TIMESTAMP, old_time.to_string())
        .header(HEADER_NONCE, nonce)
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_endpoints_reject_replayed_nonce() {
    let verifier = Arc::new(HmacVerifier::new(test_secret().as_bytes(), 300));
    let state = ServiceState::for_test(verifier);
    let app = build_service_router(state);

    let ctx = sample_context();
    let body = serde_json::to_vec(&json!({ "context": ctx })).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let nonce = Uuid::new_v4().to_string();

    let signature = HmacSigner::sign(
        test_secret().as_bytes(),
        "POST",
        "/v1/connections/list",
        &body,
        now,
        &nonce,
    )
    .unwrap();

    // First request with valid signature passes HMAC verification (returns 503 because test state has no DB)
    let req1 = Request::builder()
        .uri("/v1/connections/list")
        .method("POST")
        .header("content-type", "application/json")
        .header(HEADER_SIGNATURE, &signature)
        .header(HEADER_TIMESTAMP, now.to_string())
        .header(HEADER_NONCE, &nonce)
        .body(Body::from(body.clone()))
        .unwrap();

    let resp1 = app.clone().oneshot(req1).await.unwrap();
    // 503 Service Unavailable means HMAC authentication passed and reached the service handler
    assert_eq!(resp1.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Second request with SAME nonce must be rejected with 401 Unauthorized by HMAC middleware
    let req2 = Request::builder()
        .uri("/v1/connections/list")
        .method("POST")
        .header("content-type", "application/json")
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_TIMESTAMP, now.to_string())
        .header(HEADER_NONCE, nonce)
        .body(Body::from(body))
        .unwrap();

    let resp2 = app.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_endpoints_reject_tampered_payload() {
    let verifier = Arc::new(HmacVerifier::new(test_secret().as_bytes(), 300));
    let state = ServiceState::for_test(verifier);
    let app = build_service_router(state);

    let ctx = sample_context();
    let original_body = serde_json::to_vec(&json!({ "context": ctx })).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let nonce = Uuid::new_v4().to_string();

    // Signature generated for original_body
    let signature = HmacSigner::sign(
        test_secret().as_bytes(),
        "POST",
        "/v1/connections/list",
        &original_body,
        now,
        &nonce,
    )
    .unwrap();

    // Body tampered with an attacker-modified payload
    let tampered_body = serde_json::to_vec(&json!({ "context": ctx, "injected": true })).unwrap();

    let req = Request::builder()
        .uri("/v1/connections/list")
        .method("POST")
        .header("content-type", "application/json")
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_TIMESTAMP, now.to_string())
        .header(HEADER_NONCE, nonce)
        .body(Body::from(tampered_body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_connections_service_client_e2e() {
    use vox_connections::service::ConnectionsServiceClient;

    let verifier = Arc::new(HmacVerifier::new(test_secret().as_bytes(), 300));
    let state = ServiceState::for_test(verifier);
    let app = build_service_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = ConnectionsServiceClient::new(format!("http://{addr}"), test_secret());

    // 1. Test health checks
    let live = client.health_live().await.unwrap();
    assert!(live);

    let ready = client.health_ready().await.unwrap();
    assert!(ready);

    // 2. Test signed endpoint call with valid HMAC secret:
    // Reaches route handler (returns 503 Service Unavailable because ServiceState::for_test has no DB pool)
    let ctx = sample_context();
    let err = client.list_connections(&ctx).await.unwrap_err();
    match err {
        vox_connections::service::ClientError::ServerError { status, .. } => {
            assert_eq!(
                status, 503,
                "Valid HMAC reached handler which returned 503 (no db pool)"
            );
        }
        other => panic!("expected ServerError 503, got {:?}", other),
    }

    // 3. Test signed endpoint call with invalid HMAC secret:
    // Blocked by HMAC middleware before reaching route handler (returns 401 Unauthorized)
    let bad_client =
        ConnectionsServiceClient::new(format!("http://{addr}"), "wrong-secret-key-000000000000");
    let err = bad_client.list_connections(&ctx).await.unwrap_err();
    match err {
        vox_connections::service::ClientError::ServerError { status, .. } => {
            assert_eq!(status, 401, "Invalid HMAC blocked with 401 Unauthorized");
        }
        other => panic!("expected ServerError 401, got {:?}", other),
    }

    server_handle.abort();
}
