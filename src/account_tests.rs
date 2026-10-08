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
    sqlx::raw_sql(
        &include_str!("../examples/independent_host_schema.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/connectors.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/packages.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/skill-content.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/public-mcp.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/setup.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/discovery.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/accounts.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(
        &include_str!("../schema/account-authority.sql")
            .replace("public.", "")
            .replace(
                "SELECT pg_catalog.set_config('search_path', '', false);",
                "",
            ),
    )
    .execute(&pool)
    .await
    .unwrap();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users VALUES($1)")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let deployment: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_deployments(external_key) VALUES($1) RETURNING id",
    )
    .bind(format!("test-{user}"))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO user_contexts(id,deployment_id,user_id) VALUES($1,$2,$1)")
        .bind(user)
        .bind(deployment)
        .execute(&pool)
        .await
        .unwrap();
    let id=sqlx::query_scalar("INSERT INTO vox_connections(user_id,user_context_id,connector_id,consented_at) VALUES($1,$1,'google_calendar',now()) RETURNING id").bind(user).fetch_one(&pool).await.unwrap();
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
    svc.update_preferences_in_context(
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
    svc.update_preferences_in_context(
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
    svc.disconnect_in_context(user, id).await.unwrap();
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
        svc.start_in_context(
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
        .start_in_context(
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
    assert!(
        svc.get_setup_status_in_context(Uuid::new_v4(), id)
            .await
            .is_err()
    );
    svc.cancel_setup_in_context(user, id).await.unwrap();
    let state: String =
        sqlx::query_scalar("SELECT state_token FROM vox_connection_setups WHERE id=$1")
            .bind(id)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    assert!(svc.handle_google_callback("unused", &state).await.is_err());
    assert_eq!(
        svc.get_setup_status_in_context(user, id)
            .await
            .unwrap()
            .status,
        "cancelled"
    );
    let setup = svc
        .start_in_context(
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
        svc.get_setup_status_in_context(user, setup.setup_id.unwrap())
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
    svc.update_preferences_in_context(
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
        svc.assistant_read_in_context(user, "google_calendar", 10)
            .await
            .is_err()
    );
    let account = svc.list_connections_in_context(user).await.unwrap();
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
            .start_in_context(
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
            svc.get_setup_status_in_context(user, id)
                .await
                .unwrap()
                .status,
            if reduced { "failed" } else { "authorized" }
        );
        if !reduced {
            let read = svc
                .assistant_read_in_context(user, "google_calendar", 10)
                .await
                .unwrap();
            assert_eq!(read["complete"], true);
            svc.update_preferences_in_context(
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
                svc.assistant_read_in_context(user, "google_calendar", 10)
                    .await
                    .is_err()
            );
        }
        let setup = svc
            .start_in_context(
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
            svc.get_setup_status_in_context(user, id)
                .await
                .unwrap()
                .status,
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
        let sync = tokio::spawn(async move { background.refresh_in_context(user, id).await });
        gate.reached.notified().await;
        if disconnect {
            svc.disconnect_in_context(user, id).await.unwrap();
        } else {
            svc.update_preferences_in_context(
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
        let accounts = svc.list_connections_in_context(user).await.unwrap();
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
    assert!(svc.refresh_in_context(user, id).await.is_err());
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
        svc.list_connections_in_context(user).await.unwrap()[0]
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
    svc.update_preferences_in_context(
        user,
        id,
        PreferencesRequest {
            sync_timeline: Some(false),
            assistant_read: None,
        },
    )
    .await
    .unwrap();
    svc.update_preferences_in_context(
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
        .start_in_context(
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
    svc.disconnect_in_context(user, id).await.unwrap();
    assert_eq!(
        svc.get_setup_status_in_context(user, setup.setup_id.unwrap())
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
    let sync = tokio::spawn(async move { background.refresh_in_context(user, id).await });
    gate.reached.notified().await;
    svc.update_preferences_in_context(
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
        svc.assistant_read_in_context(user, "google_calendar", 10)
            .await,
        Err(FreshConnectionError::Provider(_))
    ));
    assert_eq!(
        svc.list_connections_in_context(user).await.unwrap()[0].authorization_state,
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
            .start_in_context(
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
    svc.disconnect_in_context(user, id).await.unwrap();
    gate.release.notify_one();
    gate.reached.notified().await;
    gate.release.notify_one();
    assert!(link.await.unwrap().is_err());
    assert!(
        svc.list_connections_in_context(user)
            .await
            .unwrap()
            .is_empty()
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn food_expiry_without_refresh_requires_reconnect_and_releases_lease() {
    let (mut svc, user, id) = fixture().await;
    svc.swiggy.client.enabled = true;
    let aad = format!("{user}:swiggy");
    let access = svc
        .cipher()
        .unwrap()
        .seal(aad.as_bytes(), "expired-access")
        .unwrap();
    sqlx::query("UPDATE vox_connections SET connector_id='swiggy',access_ciphertext=$2,access_expires_at=now()-interval '1 minute',metadata='{\"auth_type\":\"oauth2\",\"client_id\":\"registered-client\"}' WHERE id=$1")
        .bind(id).bind(access).execute(&svc.pool).await.unwrap();
    assert!(matches!(
        svc.refresh_in_context(user, id).await,
        Err(FreshConnectionError::Unauthorized)
    ));
    let row = sqlx::query(
        "SELECT authorization_state,failure_code,lease_token FROM vox_connections WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&svc.pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("authorization_state"), "expired");
    assert_eq!(row.get::<String, _>("failure_code"), "reconnect_required");
    assert_eq!(row.get::<Option<Uuid>, _>("lease_token"), None);
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn food_oauth_links_only_after_verified_reads_and_rejects_callback_replay() {
    use axum::http::{HeaderMap, StatusCode};
    use axum::{Json, Router, routing::post};
    let app=Router::new()
        .route("/register",post(|Json(v):Json<Value>| async move {
            assert_eq!(v["token_endpoint_auth_method"],"none");
            Json(json!({"client_id":"registered-client"}))
        }))
        .route("/token",post(|Json(v):Json<Value>| async move {
            assert_eq!(v["grant_type"],"authorization_code");
            assert_eq!(v["client_id"],"registered-client");
            assert!(!v["code_verifier"].as_str().unwrap().is_empty());
            Json(json!({"access_token":"verified-access","refresh_token":"issued-refresh","expires_in":3600,"token_type":"Bearer"}))
        }))
        .route("/mcp",post(|headers:HeaderMap,Json(v):Json<Value>|async move {
            assert_eq!(headers.get("Authorization").unwrap(),"Bearer verified-access");
            let result=match v["method"].as_str().unwrap() {
                "initialize"=>json!({"protocolVersion":"2025-03-26"}),
                "notifications/initialized"=>return(StatusCode::ACCEPTED,Json(Value::Null)),
                "tools/call"=>{
                    assert_eq!(v["params"]["name"],"get_addresses");
                    json!({"structuredContent":{"success":true,"data":{"addresses":[],"pagination":{"hasMore":false}}}})
                }, _=>panic!("Unexpected MCP method"),
            };
            (StatusCode::OK,Json(json!({"jsonrpc":"2.0","id":v["id"],"result":result})))
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut svc, user, _) = fixture().await;
    svc.swiggy.client.enabled = true;
    svc.swiggy.client.auth_base = base.clone();
    svc.swiggy.client.endpoint = format!("{base}/mcp");
    let setup = svc
        .start_in_context(
            user,
            StartConnectionRequest {
                connector_id: "swiggy".into(),
                npsso: None,
                consent: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(setup.status, "pending");
    assert_eq!(setup.connection_id, None);
    let url = url::Url::parse(&setup.authorization_url.unwrap()).unwrap();
    let args: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(args.get("code_challenge_method").unwrap(), "S256");
    assert_eq!(
        args.get("redirect_uri").unwrap(),
        "https://core.example/v1/connectors/swiggy/callback"
    );
    let state = args.get("state").unwrap();
    let id = svc
        .handle_food_callback("swiggy", Some("single-use-code"), state)
        .await
        .unwrap();
    assert!(
        svc.handle_food_callback("swiggy", Some("single-use-code"), state)
            .await
            .is_err()
    );
    let row=sqlx::query("SELECT access_ciphertext,refresh_ciphertext,last_synced_at FROM vox_connections WHERE id=$1").bind(id).fetch_one(&svc.pool).await.unwrap();
    let aad = format!("{user}:swiggy");
    let bytes: Vec<u8> = row.get("refresh_ciphertext");
    assert_ne!(bytes, b"issued-refresh".to_vec());
    assert_eq!(
        svc.cipher().unwrap().open(aad.as_bytes(), &bytes).unwrap(),
        "issued-refresh"
    );
    assert!(
        row.get::<Option<DateTime<Utc>>, _>("last_synced_at")
            .is_some()
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn food_rotation_survives_subsequent_read_failure_and_sync_is_retryable() {
    use axum::{Json, Router, http::StatusCode, routing::post};
    let app=Router::new().route("/token",post(|Json(v):Json<Value>| async move {
        assert_eq!(v["grant_type"],"refresh_token");assert_eq!(v["refresh_token"],"old-refresh");
        Json(json!({"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":3600,"token_type":"Bearer"}))
    })).route("/mcp",post(||async{StatusCode::SERVICE_UNAVAILABLE}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut svc, user, id) = fixture().await;
    svc.swiggy.client.enabled = true;
    svc.swiggy.client.auth_base = base.clone();
    svc.swiggy.client.endpoint = format!("{base}/mcp");
    let aad = format!("{user}:swiggy");
    let cipher = svc.cipher().unwrap();
    sqlx::query("UPDATE vox_connections SET connector_id='swiggy',access_ciphertext=$2,refresh_ciphertext=$3,access_expires_at=now()-interval '1 minute',metadata='{\"auth_type\":\"oauth2\",\"client_id\":\"registered-client\"}' WHERE id=$1")
        .bind(id).bind(cipher.seal(aad.as_bytes(),"old-access").unwrap()).bind(cipher.seal(aad.as_bytes(),"old-refresh").unwrap()).execute(&svc.pool).await.unwrap();
    assert!(matches!(
        svc.refresh_in_context(user, id).await,
        Err(FreshConnectionError::Provider(_))
    ));
    let row=sqlx::query("SELECT access_ciphertext,refresh_ciphertext,lease_token,failure_code,authorization_state FROM vox_connections WHERE id=$1").bind(id).fetch_one(&svc.pool).await.unwrap();
    assert_eq!(
        cipher
            .open(aad.as_bytes(), &row.get::<Vec<u8>, _>("access_ciphertext"))
            .unwrap(),
        "rotated-access"
    );
    assert_eq!(
        cipher
            .open(aad.as_bytes(), &row.get::<Vec<u8>, _>("refresh_ciphertext"))
            .unwrap(),
        "rotated-refresh"
    );
    assert_eq!(row.get::<Option<Uuid>, _>("lease_token"), None);
    assert_eq!(row.get::<String, _>("authorization_state"), "authorized");
    assert_eq!(row.get::<String, _>("failure_code"), "sync_failed");
    server.abort();
}

struct PersonalIngestor;
#[async_trait::async_trait]
impl TimelineIngestor for PersonalIngestor {
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
    async fn personal_activity(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _: Uuid,
        _: Uuid,
        connector: &str,
        items: &[crate::providers::personal::PersonalActivity],
    ) -> Result<usize, FreshConnectionError> {
        let mut count = 0;
        for item in items {
            count+=sqlx::query("INSERT INTO test_personal_items(connector,source_id,title) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(connector).bind(&item.source_id).bind(&item.title).execute(&mut **tx).await?.rows_affected() as usize;
        }
        Ok(count)
    }
}
async fn personal_fixture() -> (
    FreshConnectionsService,
    Uuid,
    tokio::task::JoinHandle<()>,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let (mut svc, user, _) = fixture().await;
    sqlx::query("CREATE TABLE test_personal_items(connector text,source_id text,title text,PRIMARY KEY(connector,source_id))").execute(&svc.pool).await.unwrap();
    svc.ingestor = std::sync::Arc::new(PersonalIngestor);
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    let router=axum::Router::new().fallback(move |request:axum::extract::Request|{let counter=counter.clone();async move {
        let path=request.uri().path().to_owned();
        let _method=request.method().clone();
        let response=match path.as_str(){
            "/api/token"|"/token"=>{let body=axum::body::to_bytes(request.into_body(),8192).await.unwrap();let fields=url::form_urlencoded::parse(&body).into_owned().collect::<std::collections::BTreeMap<_,_>>();assert!(fields.contains_key("client_id"));let n=counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst);if fields.get("grant_type").is_some_and(|v|v=="refresh_token"){assert_eq!(fields.get("refresh_token").unwrap(),"refresh-initial");}else{assert_eq!(fields.get("code").unwrap(),"accepted-code");if path=="/api/token"{assert!(fields.contains_key("code_verifier"));}}json!({"access_token":format!("access-{n}"),"refresh_token":if n==0 {"refresh-initial"}else{"refresh-rotated"},"expires_in":3600})},
            "/v1/me"=>json!({"id":"verified-account","display_name":"Listener"}),
                "/youtube/v3/channels" => json!({"items":[{"id":"channel-owner","snippet":{"title":"YouTube channel"},"contentDetails":{"relatedPlaylists":{"likes":"liked-playlist"}}}]}),
                "/youtube/v3/playlists" => json!({"items":[{"id":"playlist-one","snippet":{"title":"Saved playlist"}}]}),
                "/youtube/v3/subscriptions" => json!({"items":[{"id":"subscription","snippet":{"title":"Subscribed channel","publishedAt":"2026-01-01T00:00:00Z"}}]}),
                "/youtube/v3/playlistItems" => json!({"items":[{"id":"playlist-item","snippet":{"title":"Video","publishedAt":"2026-01-02T00:00:00Z","resourceId":{"videoId":"abcdefghijk"}},"contentDetails":{"videoPublishedAt":"2020-01-01T00:00:00Z"}}]}),

            "/v1/me/player/recently-played"=>json!({"items":[{"played_at":"2026-01-01T12:00:00Z","track":{"id":"track","name":"Song","duration_ms":300000}}]}),
            _=>panic!("Unexpected personal provider endpoint {path}")
        }; axum::Json(response)
    }});
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    svc.personal.spotify_id = Some("spotify-client".into());
    svc.personal.test_origin = Some(origin);
    (svc, user, task, hits)
}
#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn personal_spotify_oauth_encrypted_single_use_refresh_owned_and_deduped() {
    let (svc, user, server, hits) = personal_fixture().await;
    let start = svc
        .start_in_context(
            user,
            StartConnectionRequest {
                connector_id: "spotify".into(),
                consent: true,
                npsso: None,
            },
        )
        .await
        .unwrap();
    let auth = url::Url::parse(start.authorization_url.as_ref().unwrap()).unwrap();
    let state = auth
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    assert!(
        auth.query_pairs()
            .any(|(k, v)| k == "code_challenge_method" && v == "S256")
    );
    let id = svc
        .handle_personal_callback("spotify", Some("accepted-code"), &state)
        .await
        .unwrap();
    assert!(
        svc.handle_personal_callback("spotify", Some("accepted-code"), &state)
            .await
            .is_err()
    );
    let row = sqlx::query("SELECT * FROM vox_connections WHERE id=$1")
        .bind(id)
        .fetch_one(&svc.pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("account_id"), "verified-account");
    let sealed: Vec<u8> = row.get("access_ciphertext");
    assert!(!String::from_utf8_lossy(&sealed).contains("access-0"));
    assert!(
        svc.cipher()
            .unwrap()
            .open(format!("{}:spotify", Uuid::new_v4()).as_bytes(), &sealed)
            .is_err()
    );
    sqlx::query(
        "UPDATE vox_connections SET access_expires_at=now()-interval '1 minute' WHERE id=$1",
    )
    .bind(id)
    .execute(&svc.pool)
    .await
    .unwrap();
    assert_eq!(
        svc.refresh_in_context(user, id)
            .await
            .unwrap()
            .spans_created,
        0
    );
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    let row = sqlx::query("SELECT refresh_ciphertext FROM vox_connections WHERE id=$1")
        .bind(id)
        .fetch_one(&svc.pool)
        .await
        .unwrap();
    assert_eq!(
        svc.cipher()
            .unwrap()
            .open(
                format!("{user}:spotify").as_bytes(),
                &row.get::<Vec<u8>, _>("refresh_ciphertext")
            )
            .unwrap(),
        "refresh-rotated"
    );
    assert!(
        svc.read_personal_in_context(Uuid::new_v4(), id)
            .await
            .is_err()
    );
    let read = svc.read_personal_in_context(user, id).await.unwrap();
    assert_eq!(read["data"]["recently_played"][0]["track"]["name"], "Song");
    svc.update_preferences_in_context(
        user,
        id,
        PreferencesRequest {
            sync_timeline: None,
            assistant_read: Some(false),
        },
    )
    .await
    .unwrap();
    assert!(svc.read_personal_in_context(user, id).await.is_err());
    svc.disconnect_in_context(user, id).await.unwrap();
    assert!(svc.refresh_in_context(user, id).await.is_err());
    server.abort();
}
#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn personal_history_import_requires_consent_dedupes_and_is_separate_from_oauth() {
    let (svc, user, server, _) = personal_fixture().await;
    let history = json!([{"products":["YouTube"],"title":"Watched Real video","titleUrl":"https://www.youtube.com/watch?v=abcdefghijk","time":"2026-01-01T12:00:00Z"},{"title":"Searched"}]);
    assert!(
        svc.import_youtube_history_in_context(user, history.clone(), false)
            .await
            .is_err()
    );
    let result = svc
        .import_youtube_history_in_context(user, history.clone(), true)
        .await
        .unwrap();
    assert_eq!(result["imported"], 1);
    assert_eq!(result["skipped"], 1);
    assert_eq!(result["complete"], false);
    assert_eq!(
        svc.import_youtube_history_in_context(user, history, true)
            .await
            .unwrap()["imported"],
        0
    );
    let id: Uuid = serde_json::from_value(result["connection_id"].clone()).unwrap();
    let read = svc.read_personal_in_context(user, id).await.unwrap();
    assert_eq!(read["connector_id"], "youtube_history");
    assert_eq!(
        read["data"]["watch_history"][0]["provider_data"]["action"],
        "watch"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM vox_connections WHERE user_id=$1 AND connector_id='youtube'"
        )
        .bind(user)
        .fetch_one(&svc.pool)
        .await
        .unwrap(),
        0
    );
    server.abort();
}
#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn personal_youtube_oauth_reads_only_playlists_likes_and_subscription_snapshots() {
    let (svc, user, server, _) = personal_fixture().await;
    let start = svc
        .start_in_context(
            user,
            StartConnectionRequest {
                connector_id: "youtube".into(),
                consent: true,
                npsso: None,
            },
        )
        .await
        .unwrap();
    let auth = url::Url::parse(start.authorization_url.as_ref().unwrap()).unwrap();
    let state = auth
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    assert!(
        auth.query_pairs()
            .any(|(k, v)| k == "scope" && v == "https://www.googleapis.com/auth/youtube.readonly")
    );
    let id = svc
        .handle_personal_callback("youtube", Some("accepted-code"), &state)
        .await
        .unwrap();
    let read = svc.read_personal_in_context(user, id).await.unwrap();
    assert_eq!(read["data"]["watch_history_available"], false);
    assert_eq!(read["data"]["subscriptions_are_snapshot"], true);
    let actions = read["data"]["playlist_items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["action"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(actions, vec!["playlist_addition", "like"]);
    assert_eq!(
        svc.refresh_in_context(user, id)
            .await
            .unwrap()
            .spans_created,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM test_personal_items WHERE connector='youtube'"
        )
        .fetch_one(&svc.pool)
        .await
        .unwrap(),
        2
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn curated_accounts_are_host_scoped_and_agent_grants_are_explicit() {
    use crate::capability_grants::{CapabilityGrantService, CreateGrantRequest};
    use crate::identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId};
    let (svc, user, _) = fixture().await;
    let deployment: Uuid =
        sqlx::query_scalar("SELECT deployment_id FROM user_contexts WHERE id=$1")
            .bind(user)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    let other: Uuid = sqlx::query_scalar(
        "INSERT INTO user_contexts(deployment_id,user_id) VALUES($1,$2) RETURNING id",
    )
    .bind(deployment)
    .bind(user)
    .fetch_one(&svc.pool)
    .await
    .unwrap();
    let context = RequestContext {
        id: UserContextId(user),
        user_id: UserId(user),
        subject: RequestSubject {
            deployment_id: DeploymentId(deployment),
        },
    };
    let other_context = RequestContext {
        id: UserContextId(other),
        ..context
    };
    let history = json!([{"title":"Watched Evidence","products":["YouTube"],"titleUrl":"https://www.youtube.com/watch?v=dQw4w9WgXcQ","time":"2026-10-01T08:00:00Z"}]);
    let import = svc
        .import_youtube_history(&context, history.clone(), true)
        .await
        .unwrap();
    let id = Uuid::parse_str(import["connection_id"].as_str().unwrap()).unwrap();
    assert!(
        svc.list_connections(&other_context)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(svc.disconnect(&other_context, id).await.is_err());
    let second = svc
        .import_youtube_history(&other_context, history, true)
        .await
        .unwrap();
    assert_ne!(import["connection_id"], second["connection_id"]);
    assert_eq!(svc.list_connections(&other_context).await.unwrap().len(), 1);
    assert!(
        svc.assistant_read(&context, "assistant", "youtube_history", 25)
            .await
            .is_err()
    );
    let agent:Uuid=sqlx::query_scalar("INSERT INTO agent_definitions(deployment_id,external_key,owner_user_context_id,requested_capability_categories) VALUES($1,'assistant',$2,ARRAY['*']) RETURNING id").bind(deployment).bind(user).fetch_one(&svc.pool).await.unwrap();
    sqlx::query("INSERT INTO deployment_agent_selections VALUES($1,$2)")
        .bind(deployment)
        .bind(agent)
        .execute(&svc.pool)
        .await
        .unwrap();
    let grants = CapabilityGrantService::new(svc.pool.clone());
    let _grant = grants
        .grant(
            &context,
            CreateGrantRequest {
                agent_external_key: "assistant".into(),
                connection_id: id,
                capability_external_key: "curated_youtube_history.read".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        svc.assistant_read(&context, "assistant", "youtube_history", 25)
            .await
            .is_ok()
    );
    assert!(
        svc.assistant_read(&other_context, "assistant", "youtube_history", 25)
            .await
            .is_err()
    );
    svc.update_preferences(
        &context,
        id,
        PreferencesRequest {
            sync_timeline: None,
            assistant_read: Some(false),
        },
    )
    .await
    .unwrap();
    assert!(
        svc.assistant_read(&context, "assistant", "youtube_history", 25)
            .await
            .is_err()
    );
    svc.update_preferences(
        &context,
        id,
        PreferencesRequest {
            sync_timeline: None,
            assistant_read: Some(true),
        },
    )
    .await
    .unwrap();
    assert!(
        svc.assistant_read(&context, "assistant", "youtube_history", 25)
            .await
            .is_ok()
    );
    grants
        .revoke(
            &context,
            CreateGrantRequest {
                agent_external_key: "assistant".into(),
                connection_id: id,
                capability_external_key: "curated_youtube_history.read".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        svc.assistant_read(&context, "assistant", "youtube_history", 25)
            .await
            .is_err()
    );
    let forged = RequestContext {
        user_id: UserId(Uuid::new_v4()),
        ..context
    };
    assert!(svc.list_connections(&forged).await.is_err());
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn ambiguous_legacy_accounts_require_explicit_owned_reassociation() {
    use crate::identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId};
    let (svc, user, _) = fixture().await;
    let deployment: Uuid =
        sqlx::query_scalar("SELECT deployment_id FROM user_contexts WHERE id=$1")
            .bind(user)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    let context = RequestContext {
        id: UserContextId(user),
        user_id: UserId(user),
        subject: RequestSubject {
            deployment_id: DeploymentId(deployment),
        },
    };
    let cipher = svc
        .cipher()
        .unwrap()
        .seal(format!("{user}:spotify").as_bytes(), "encrypted-original")
        .unwrap();
    let id:Uuid=sqlx::query_scalar("INSERT INTO vox_connections(user_id,connector_id,access_ciphertext,consented_at,failure_code) VALUES($1,'spotify',$2,now(),'scope_reassociation_required') RETURNING id").bind(user).bind(&cipher).fetch_one(&svc.pool).await.unwrap();
    assert!(
        !svc.list_connections(&context)
            .await
            .unwrap()
            .iter()
            .any(|c| c.id == id)
    );
    svc.reassociate(&context, id).await.unwrap();
    assert!(
        svc.list_connections(&context)
            .await
            .unwrap()
            .iter()
            .any(|c| c.id == id)
    );
    assert_eq!(
        sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT access_ciphertext FROM vox_connections WHERE id=$1"
        )
        .bind(id)
        .fetch_one(&svc.pool)
        .await
        .unwrap(),
        cipher
    );
    assert!(svc.reassociate(&context, id).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM agent_capability_grants WHERE connection_id=$1"
        )
        .bind(id)
        .fetch_one(&svc.pool)
        .await
        .unwrap(),
        0
    );
}

#[test]
fn crypto_compatibility_adapter_preserves_ciphertext_and_associated_data() {
    let old =
        crate::connected_apps::crypto::CredentialCipher::from_hex_key(&"ab".repeat(32)).unwrap();
    let current = crate::crypto::CredentialCipher::from_hex_key(&"ab".repeat(32)).unwrap();
    let aad = b"user:provider";
    let legacy = old.seal(aad, "protected").unwrap();
    assert_eq!(current.open(aad, &legacy).unwrap(), "protected");
    let fresh = current.seal(aad, "protected").unwrap();
    assert_eq!(old.open(aad, &fresh).unwrap(), "protected");
    assert!(old.open(b"other:provider", &fresh).is_err());
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn standalone_curated_upgrade_preserves_ids_credentials_and_does_not_grant_access() {
    let (svc, user, id) = fixture().await;
    // Model the curated-only schema, keeping its original constraint name.
    sqlx::raw_sql("DROP FUNCTION project_curated_connection() CASCADE; DROP FUNCTION revoke_curated_credentials() CASCADE; ALTER TABLE vox_connections DROP COLUMN user_context_id CASCADE; ALTER TABLE vox_connections DROP COLUMN credential_user_id; ALTER TABLE vox_connection_setups DROP COLUMN user_context_id CASCADE; ALTER TABLE vox_connections ADD CONSTRAINT vox_connections_user_connector_unique UNIQUE(user_id,connector_id);")
        .execute(&svc.pool).await.unwrap();
    let cipher = svc
        .cipher()
        .unwrap()
        .seal(
            format!("{user}:google_calendar").as_bytes(),
            "retained-native-token",
        )
        .unwrap();
    sqlx::query("UPDATE vox_connections SET access_ciphertext=$1 WHERE id=$2")
        .bind(&cipher)
        .bind(id)
        .execute(&svc.pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../schema/account-scope-upgrade.sql"))
        .execute(&svc.pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../schema/account-authority.sql"))
        .execute(&svc.pool)
        .await
        .unwrap();
    let row =
        sqlx::query("SELECT id,user_context_id,access_ciphertext FROM vox_connections WHERE id=$1")
            .bind(id)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    assert_eq!(row.get::<Uuid, _>("id"), id);
    assert_eq!(row.get::<Uuid, _>("user_context_id"), user);
    assert_eq!(row.get::<Vec<u8>, _>("access_ciphertext"), cipher);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM agent_capability_grants WHERE connection_id=$1"
        )
        .bind(id)
        .fetch_one(&svc.pool)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn credential_provenance_survives_owner_change_and_rotation() {
    let (svc, context, _) = fixture().await;
    let original = svc
        .credential_aad(context, "google_calendar")
        .await
        .unwrap();
    sqlx::query("UPDATE vox_connections SET credential_user_id=user_id WHERE user_context_id=$1")
        .bind(context)
        .execute(&svc.pool)
        .await
        .unwrap();
    let new_owner: Uuid = sqlx::query_scalar("INSERT INTO users DEFAULT VALUES RETURNING id")
        .fetch_one(&svc.pool)
        .await
        .unwrap();
    // Mirror the production merger's deferred composite ownership checks.
    sqlx::raw_sql("ALTER TABLE vox_connections ALTER CONSTRAINT vox_connections_context_owner DEFERRABLE; ALTER TABLE vox_connection_setups ALTER CONSTRAINT vox_setups_context_owner DEFERRABLE;").execute(&svc.pool).await.unwrap();
    let mut tx = svc.pool.begin().await.unwrap();
    sqlx::query("SET CONSTRAINTS ALL DEFERRED")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE user_contexts SET user_id=$2 WHERE id=$1")
        .bind(context)
        .bind(new_owner)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE vox_connections SET user_id=$2,credential_generation=gen_random_uuid() WHERE user_context_id=$1").bind(context).bind(new_owner).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        svc.credential_aad(context, "google_calendar")
            .await
            .unwrap(),
        original
    );
    let encrypted = svc
        .cipher()
        .unwrap()
        .seal(original.as_bytes(), "rotated-token")
        .unwrap();
    assert_eq!(
        svc.cipher()
            .unwrap()
            .open(
                svc.credential_aad(context, "google_calendar")
                    .await
                    .unwrap()
                    .as_bytes(),
                &encrypted
            )
            .unwrap(),
        "rotated-token"
    );
}

#[tokio::test]
#[ignore = "requires disposable CONNECTIONS_TEST_DATABASE_URL"]
async fn maps_import_is_scoped_consented_and_requires_agent_grants() {
    use crate::identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId};
    let (svc, user, _) = fixture().await;
    let deployment: Uuid =
        sqlx::query_scalar("SELECT deployment_id FROM user_contexts WHERE id=$1")
            .bind(user)
            .fetch_one(&svc.pool)
            .await
            .unwrap();
    let other: Uuid = sqlx::query_scalar(
        "INSERT INTO user_contexts(deployment_id,user_id) VALUES($1,$2) RETURNING id",
    )
    .bind(deployment)
    .bind(user)
    .fetch_one(&svc.pool)
    .await
    .unwrap();
    let context = RequestContext {
        id: UserContextId(user),
        user_id: UserId(user),
        subject: RequestSubject {
            deployment_id: DeploymentId(deployment),
        },
    };
    let other_context = RequestContext {
        id: UserContextId(other),
        ..context
    };
    let history = json!([{"startTime":"2026-01-01T08:00:00Z","endTime":"2026-01-01T09:00:00Z","visit":{"topCandidate":{"placeId":"place","semanticType":"HOME","placeLocation":{"latLng":"geo:12.9,77.6"}}}}]);
    assert!(
        svc.import_maps_timeline(&context, history.clone(), false)
            .await
            .is_err()
    );
    let first = svc
        .import_maps_timeline(&context, history.clone(), true)
        .await
        .unwrap();
    let id = Uuid::parse_str(first["connection_id"].as_str().unwrap()).unwrap();
    assert!(svc.read_personal_account(&other_context, id).await.is_err());
    assert!(
        svc.assistant_read(&context, "assistant", "maps_timeline", 25)
            .await
            .is_err()
    );
    let second = svc
        .import_maps_timeline(&other_context, history, true)
        .await
        .unwrap();
    assert_ne!(first["connection_id"], second["connection_id"]);
}
