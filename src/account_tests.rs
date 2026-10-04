use super::*;
use crate::providers::{google_calendar::GoogleCalendarEvent, observations::ObservedActivity};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;
struct Ingestor;
#[async_trait::async_trait]
impl TimelineIngestor for Ingestor {
    async fn calendar(
        &self,
        _: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _: Uuid,
        _: Uuid,
        _: &str,
        _: &[GoogleCalendarEvent],
        _: DateTime<Utc>,
        _: DateTime<Utc>,
    ) -> Result<usize, FreshConnectionError> {
        Ok(0)
    }
    async fn gaming(
        &self,
        _: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _: Uuid,
        _: Uuid,
        _: &[ObservedActivity],
    ) -> Result<usize, FreshConnectionError> {
        Ok(0)
    }
}
async fn fixture() -> (FreshConnectionsService, Uuid, Uuid) {
    let url = std::env::var("CONNECTIONS_TEST_DATABASE_URL").expect("disposable database required");
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("connections_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .after_connect(move |conn, _| {
            let schema = schema.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path TO {schema}"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE users(id UUID PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../schema/connectors.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users VALUES($1)")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let id=sqlx::query_scalar("INSERT INTO vox_connections(user_id,connector_id,consented_at) VALUES($1,'google_calendar',now()) RETURNING id").bind(user).fetch_one(&pool).await.unwrap();
    (
        FreshConnectionsService::new(
            pool,
            Some(&"ab".repeat(32)),
            std::sync::Arc::new(Ingestor),
            Some("client".into()),
            Some("secret".into()),
            Some("https://core.example".into()),
        )
        .unwrap(),
        user,
        id,
    )
}
#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn lease_serializes_and_pause_disconnect_fence_commits() {
    let (svc, user, id) = fixture().await;
    assert!(svc.claim(Uuid::new_v4(), id).await.is_err());
    let claim = svc.claim(user, id).await.unwrap();
    assert!(svc.claim(user, id).await.is_err());
    let mut tx = svc.pool.begin().await.unwrap();
    assert!(svc.validate_commit(&mut tx, user, id, &claim).await.is_ok());
    tx.rollback().await.unwrap();
    svc.update_preferences(
        user,
        id,
        PreferencesRequest {
            sync_timeline: Some(false),
            assistant_read: None,
        },
    )
    .await
    .unwrap();
    let mut tx = svc.pool.begin().await.unwrap();
    assert!(
        svc.validate_commit(&mut tx, user, id, &claim)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    svc.update_preferences(
        user,
        id,
        PreferencesRequest {
            sync_timeline: Some(true),
            assistant_read: None,
        },
    )
    .await
    .unwrap();
    assert!(svc.claim(user, id).await.is_err());
    svc.release_failure(id, &claim, &FreshConnectionError::Unauthorized)
        .await
        .unwrap();
    let claim = svc.claim(user, id).await.unwrap();
    svc.disconnect(user, id).await.unwrap();
    let mut tx = svc.pool.begin().await.unwrap();
    assert!(
        svc.validate_commit(&mut tx, user, id, &claim)
            .await
            .is_err()
    );
}
#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn setup_is_owned_cancelled_and_expired_without_provider_calls() {
    let (svc, user, _) = fixture().await;
    assert!(
        svc.start(
            user,
            StartConnectionRequest {
                connector_id: "google_calendar".into(),
                npsso: None,
                consent: false
            }
        )
        .await
        .is_err()
    );
    let setup = svc
        .start(
            user,
            StartConnectionRequest {
                connector_id: "google_calendar".into(),
                npsso: None,
                consent: true,
            },
        )
        .await
        .unwrap();
    let id = setup.setup_id.unwrap();
    assert!(svc.get_setup_status(Uuid::new_v4(), id).await.is_err());
    svc.cancel_setup(user, id).await.unwrap();
    let state: String =
        sqlx::query_scalar("SELECT state_token FROM vox_connection_setups WHERE id=$1")
            .bind(id)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    assert!(svc.handle_google_callback("unused", &state).await.is_err());
    assert_eq!(
        svc.get_setup_status(user, id).await.unwrap().status,
        "cancelled"
    );
    let setup = svc
        .start(
            user,
            StartConnectionRequest {
                connector_id: "google_calendar".into(),
                npsso: None,
                consent: true,
            },
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE vox_connection_setups SET expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(setup.setup_id)
    .execute(&svc.pool)
    .await
    .unwrap();
    assert_eq!(
        svc.get_setup_status(user, setup.setup_id.unwrap())
            .await
            .unwrap()
            .status,
        "expired"
    );
}
#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn rotation_is_fenced_and_worker_recovers_expired_leases() {
    let (svc, user, id) = fixture().await;
    let old = svc.claim(user, id).await.unwrap();
    sqlx::query("UPDATE vox_connections SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(id)
        .execute(&svc.pool)
        .await
        .unwrap();
    let next = svc.claim(user, id).await.unwrap();
    assert!(
        svc.store_rotated(&old, id, vec![1], Some(vec![2]), Utc::now())
            .await
            .is_err()
    );
    svc.store_rotated(&next, id, vec![3], Some(vec![4]), Utc::now())
        .await
        .unwrap();
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT refresh_ciphertext FROM vox_connections WHERE id=$1")
            .bind(id)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    assert_eq!(stored, vec![4]);
    svc.release_failure(id, &old, &FreshConnectionError::Unauthorized)
        .await
        .unwrap();
    assert!(svc.claim(user, id).await.is_err());
    svc.update_preferences(
        user,
        id,
        PreferencesRequest {
            sync_timeline: None,
            assistant_read: Some(false),
        },
    )
    .await
    .unwrap();
    assert!(
        svc.assistant_read(user, "google_calendar", 10)
            .await
            .is_err()
    );
    let account = svc.list_connections(user).await.unwrap();
    assert!(!account[0].assistant_read);
    assert_ne!(
        old.get::<Uuid, _>("lease_token"),
        next.get::<Uuid, _>("lease_token")
    );
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn google_link_is_single_use_denial_and_scope_reduction_fail_closed() {
    use axum::{
        Json, Router,
        extract::State,
        routing::{get, post},
    };
    async fn token(State(reduced): State<bool>) -> Json<serde_json::Value> {
        Json(
            serde_json::json!({"access_token":"access","refresh_token":"refresh","expires_in":3600,"scope":if reduced{"profile"}else{crate::providers::google_calendar::CALENDAR_SCOPE}}),
        )
    }
    async fn profile() -> Json<serde_json::Value> {
        Json(serde_json::json!({"sub":"verified-google-user","email":"fixture@example.com"}))
    }
    async fn events() -> Json<serde_json::Value> {
        Json(serde_json::json!({"items":[],"timeZone":"Asia/Kolkata"}))
    }
    for reduced in [false, true] {
        let (mut svc, user, _) = fixture().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/token", post(token))
                    .route("/profile", get(profile))
                    .route("/events", get(events))
                    .with_state(reduced),
            )
            .await
            .unwrap()
        });
        svc.google.test_origin(&origin);
        let setup = svc
            .start(
                user,
                StartConnectionRequest {
                    connector_id: "google_calendar".into(),
                    npsso: None,
                    consent: true,
                },
            )
            .await
            .unwrap();
        let id = setup.setup_id.unwrap();
        let url = url::Url::parse(setup.authorization_url.as_deref().unwrap()).unwrap();
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .to_string();
        let result = svc.handle_google_callback("code", &state).await;
        assert_eq!(result.is_ok(), !reduced);
        assert!(svc.handle_google_callback("code", &state).await.is_err());
        assert_eq!(
            svc.get_setup_status(user, id).await.unwrap().status,
            if reduced { "failed" } else { "authorized" }
        );
        if !reduced {
            let read = svc
                .assistant_read(user, "google_calendar", 10)
                .await
                .unwrap();
            assert_eq!(read["complete"], true);
            svc.update_preferences(
                user,
                result.unwrap(),
                PreferencesRequest {
                    sync_timeline: None,
                    assistant_read: Some(false),
                },
            )
            .await
            .unwrap();
            assert!(
                svc.assistant_read(user, "google_calendar", 10)
                    .await
                    .is_err()
            );
        }
        let setup = svc
            .start(
                user,
                StartConnectionRequest {
                    connector_id: "google_calendar".into(),
                    npsso: None,
                    consent: true,
                },
            )
            .await
            .unwrap();
        let id = setup.setup_id.unwrap();
        let url = url::Url::parse(setup.authorization_url.as_deref().unwrap()).unwrap();
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .to_string();
        svc.fail_google_setup(&state).await.unwrap();
        assert!(svc.handle_google_callback("code", &state).await.is_err());
        assert_eq!(
            svc.get_setup_status(user, id).await.unwrap().status,
            "failed"
        );
        server.abort();
    }
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn provider_results_are_discarded_after_pause_or_disconnect() {
    use axum::{Json, Router, extract::State, routing::get};
    use std::sync::Arc;
    struct Gate {
        reached: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    async fn events(State(gate): State<Arc<Gate>>) -> Json<serde_json::Value> {
        gate.reached.notify_one();
        gate.release.notified().await;
        Json(serde_json::json!({"items":[],"timeZone":"UTC"}))
    }
    for disconnect in [false, true] {
        let (mut svc, user, id) = fixture().await;
        let aad = format!("{user}:google_calendar");
        let sealed = svc
            .cipher()
            .unwrap()
            .seal(aad.as_bytes(), "access")
            .unwrap();
        sqlx::query("UPDATE vox_connections SET account_id='account',access_ciphertext=$1,access_expires_at=now()+interval '1 hour' WHERE id=$2").bind(sealed).bind(id).execute(&svc.pool).await.unwrap();
        let gate = Arc::new(Gate {
            reached: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let fixture_gate = gate.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/events", get(events))
                    .with_state(fixture_gate),
            )
            .await
            .unwrap()
        });
        svc.google.test_origin(&origin);
        let background = svc.clone();
        let sync = tokio::spawn(async move { background.refresh(user, id).await });
        gate.reached.notified().await;
        if disconnect {
            svc.disconnect(user, id).await.unwrap();
        } else {
            svc.update_preferences(
                user,
                id,
                PreferencesRequest {
                    sync_timeline: Some(false),
                    assistant_read: None,
                },
            )
            .await
            .unwrap();
        }
        gate.release.notify_one();
        assert!(sync.await.unwrap().is_err());
        let accounts = svc.list_connections(user).await.unwrap();
        if !disconnect {
            assert!(accounts[0].last_synced_at.is_none());
            assert!(!accounts[0].sync_timeline);
        } else {
            assert!(accounts.is_empty());
        }
        server.abort();
    }
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn provider_rotation_is_saved_even_when_the_following_fetch_fails() {
    use axum::{
        Json, Router,
        http::StatusCode,
        routing::{get, post},
    };
    async fn token() -> Json<serde_json::Value> {
        Json(
            serde_json::json!({"access_token":"new-access","refresh_token":"rotated-refresh","expires_in":3600,"scope":crate::providers::google_calendar::CALENDAR_SCOPE}),
        )
    }
    let (mut svc, user, id) = fixture().await;
    let aad = format!("{user}:google_calendar");
    let access = svc
        .cipher()
        .unwrap()
        .seal(aad.as_bytes(), "old-access")
        .unwrap();
    let refresh = svc
        .cipher()
        .unwrap()
        .seal(aad.as_bytes(), "old-refresh")
        .unwrap();
    sqlx::query("UPDATE vox_connections SET account_id='account',access_ciphertext=$1,refresh_ciphertext=$2,access_expires_at=now()-interval '1 second' WHERE id=$3").bind(access).bind(refresh).bind(id).execute(&svc.pool).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/token", post(token))
                .route("/events", get(|| async { StatusCode::SERVICE_UNAVAILABLE })),
        )
        .await
        .unwrap()
    });
    svc.google.test_origin(&origin);
    assert!(svc.refresh(user, id).await.is_err());
    let ciphertext: Vec<u8> =
        sqlx::query_scalar("SELECT refresh_ciphertext FROM vox_connections WHERE id=$1")
            .bind(id)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    assert_eq!(
        svc.cipher()
            .unwrap()
            .open(aad.as_bytes(), &ciphertext)
            .unwrap(),
        "rotated-refresh"
    );
    assert!(
        svc.list_connections(user).await.unwrap()[0]
            .last_synced_at
            .is_none()
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn playstation_resume_preserves_high_water_and_requires_a_new_baseline() {
    let (svc, user, id) = fixture().await;
    sqlx::query("UPDATE vox_connections SET connector_id='playstation',metadata=$2 WHERE id=$1").bind(id).bind(serde_json::json!({"snapshots":{"game":{"total_seconds":100,"observed_at":Utc::now(),"last_played_at":null,"name":"Game","platform":"PS5"}}})).execute(&svc.pool).await.unwrap();
    svc.update_preferences(
        user,
        id,
        PreferencesRequest {
            sync_timeline: Some(false),
            assistant_read: None,
        },
    )
    .await
    .unwrap();
    svc.update_preferences(
        user,
        id,
        PreferencesRequest {
            sync_timeline: Some(true),
            assistant_read: None,
        },
    )
    .await
    .unwrap();
    let metadata: serde_json::Value =
        sqlx::query_scalar("SELECT metadata FROM vox_connections WHERE id=$1")
            .bind(id)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    assert_eq!(metadata["snapshots"]["game"]["total_seconds"], 100);
    assert_eq!(metadata["baseline_required"], true);
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn disconnect_invalidates_pending_reconnect_setup() {
    let (svc, user, id) = fixture().await;
    let setup = svc
        .start(
            user,
            StartConnectionRequest {
                connector_id: "google_calendar".into(),
                npsso: None,
                consent: true,
            },
        )
        .await
        .unwrap();
    let state = url::Url::parse(setup.authorization_url.as_deref().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .to_string();
    svc.disconnect(user, id).await.unwrap();
    assert_eq!(
        svc.get_setup_status(user, setup.setup_id.unwrap())
            .await
            .unwrap()
            .status,
        "cancelled"
    );
    assert!(svc.handle_google_callback("code", &state).await.is_err());
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn preference_change_retains_rotating_credentials_but_discards_sync() {
    use axum::{
        Json, Router,
        extract::State,
        routing::{get, post},
    };
    use std::sync::Arc;
    struct Gate {
        reached: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    async fn token(State(gate): State<Arc<Gate>>) -> Json<serde_json::Value> {
        gate.reached.notify_one();
        gate.release.notified().await;
        Json(
            serde_json::json!({"access_token":"new-access","refresh_token":"rotated-refresh","expires_in":3600,"scope":crate::providers::google_calendar::CALENDAR_SCOPE}),
        )
    }
    let (mut svc, user, id) = fixture().await;
    let aad = format!("{user}:google_calendar");
    let sealed = svc
        .cipher()
        .unwrap()
        .seal(aad.as_bytes(), "old-refresh")
        .unwrap();
    sqlx::query("UPDATE vox_connections SET account_id='account',access_ciphertext=$1,refresh_ciphertext=$1,access_expires_at=now()-interval '1 second' WHERE id=$2").bind(sealed).bind(id).execute(&svc.pool).await.unwrap();
    let gate = Arc::new(Gate {
        reached: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let fixture_gate = gate.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/token", post(token))
                .route(
                    "/events",
                    get(|| async { Json(serde_json::json!({"items":[]})) }),
                )
                .with_state(fixture_gate),
        )
        .await
        .unwrap()
    });
    svc.google.test_origin(&origin);
    let background = svc.clone();
    let sync = tokio::spawn(async move { background.refresh(user, id).await });
    gate.reached.notified().await;
    svc.update_preferences(
        user,
        id,
        PreferencesRequest {
            sync_timeline: Some(false),
            assistant_read: None,
        },
    )
    .await
    .unwrap();
    gate.release.notify_one();
    assert!(sync.await.unwrap().is_err());
    let row = sqlx::query(
        "SELECT refresh_ciphertext,lease_token,last_synced_at FROM vox_connections WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&svc.pool)
    .await
    .unwrap();
    assert_eq!(
        svc.cipher()
            .unwrap()
            .open(aad.as_bytes(), &row.get::<Vec<u8>, _>("refresh_ciphertext"))
            .unwrap(),
        "rotated-refresh"
    );
    assert!(row.get::<Option<Uuid>, _>("lease_token").is_none());
    assert!(
        row.get::<Option<DateTime<Utc>>, _>("last_synced_at")
            .is_none()
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn transient_google_assistant_refresh_keeps_authorization_retryable() {
    use axum::{Router, http::StatusCode, routing::post};
    let (mut svc, user, id) = fixture().await;
    let aad = format!("{user}:google_calendar");
    let sealed = svc
        .cipher()
        .unwrap()
        .seal(aad.as_bytes(), "refresh")
        .unwrap();
    sqlx::query("UPDATE vox_connections SET account_id='account',access_ciphertext=$1,refresh_ciphertext=$1,access_expires_at=now()-interval '1 second' WHERE id=$2").bind(sealed).bind(id).execute(&svc.pool).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/token", post(|| async { StatusCode::SERVICE_UNAVAILABLE })),
        )
        .await
        .unwrap()
    });
    svc.google.test_origin(&origin);
    assert!(matches!(
        svc.assistant_read(user, "google_calendar", 10).await,
        Err(FreshConnectionError::Provider(_))
    ));
    assert_eq!(
        svc.list_connections(user).await.unwrap()[0].authorization_state,
        "authorized"
    );
    server.abort();
}

#[test]
fn transient_sony_errors_do_not_require_reauthorization() {
    use crate::providers::playstation::PlayStationError;
    for error in [
        PlayStationError::ProviderError("unavailable".into()),
        PlayStationError::RateLimited(60),
    ] {
        assert!(matches!(
            map_psn_error(error),
            FreshConnectionError::Provider(_)
        ));
    }
    assert!(matches!(
        map_psn_error(PlayStationError::ReconnectRequired),
        FreshConnectionError::Unauthorized
    ));
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn disconnect_fences_inflight_verified_sony_link() {
    use axum::{
        Json, Router,
        extract::State,
        http::{StatusCode, header},
        routing::{get, post},
    };
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::sync::Arc;
    struct Gate {
        reached: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    async fn authorize() -> impl axum::response::IntoResponse {
        (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                "com.scee.psxandroid.scecompcall://redirect?code=fixture",
            )],
        )
    }
    async fn token() -> Json<serde_json::Value> {
        Json(
            serde_json::json!({"access_token":"access","refresh_token":"refresh","expires_in":3600,"refresh_token_expires_in":86400,"id_token":format!("header.{}.signature",URL_SAFE_NO_PAD.encode(br#"{"sub":"12345"}"#)),"scope":"psn:mobile.v2.core"}),
        )
    }
    async fn games(State(gate): State<Arc<Gate>>) -> Json<serde_json::Value> {
        gate.reached.notify_one();
        gate.release.notified().await;
        Json(serde_json::json!({"titles":[]}))
    }
    let (mut svc, user, id) = fixture().await;
    sqlx::query("UPDATE vox_connections SET connector_id='playstation' WHERE id=$1")
        .bind(id)
        .execute(&svc.pool)
        .await
        .unwrap();
    let gate = Arc::new(Gate {
        reached: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let fixture_gate = gate.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/authorize", get(authorize))
                .route("/token", post(token))
                .route(
                    "/api/userProfile/v1/internal/users/12345/profiles",
                    get(|| async {
                        Json(serde_json::json!({"isMe":true,"onlineId":"VerifiedPlayer"}))
                    }),
                )
                .route("/api/gamelist/v2/users/me/titles", get(games))
                .with_state(fixture_gate),
        )
        .await
        .unwrap()
    });
    svc.psn.test_origin(&origin);
    let background = svc.clone();
    let link = tokio::spawn(async move {
        background
            .start(
                user,
                StartConnectionRequest {
                    connector_id: "playstation".into(),
                    npsso: Some("a".repeat(64)),
                    consent: true,
                },
            )
            .await
    });
    gate.reached.notified().await;
    svc.disconnect(user, id).await.unwrap();
    gate.release.notify_one();
    gate.reached.notified().await;
    gate.release.notify_one();
    assert!(link.await.unwrap().is_err());
    assert!(svc.list_connections(user).await.unwrap().is_empty());
    server.abort();
}
