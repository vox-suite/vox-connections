//! Local OAuth -> reviewed package -> selected-agent grant -> REST read -> revoke.
//! Apply the independent host schema and connector schemas before this ignored test.
use axum::{
    Json, Router,
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use vox_connections::{
    capability_grants::{CapabilityGrantService, CreateGrantRequest},
    connected_apps::{ConnectedAppError, ConnectedAppsOptions, ConnectedAppsService},
    connections::ConnectionService,
    google_reads::{CALENDAR_SCOPE, CALENDAR_TOOL, DRIVE_SCOPE, DRIVE_TOOL, GoogleReadAdapter},
    identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId},
    packages::{PackageMetadata, PackageRegistry, PublishPackage, package_bytes},
    setup::{ConnectorSetup, SetupConsent, SetupRequest, SetupState},
};
#[derive(Clone, Default)]
struct Provider {
    origin: Arc<Mutex<String>>,
    token_forms: Arc<Mutex<Vec<String>>>,
    reads: Arc<Mutex<usize>>,
}
async fn metadata(State(p): State<Provider>) -> Json<Value> {
    let origin = p.origin.lock().unwrap().clone();
    Json(
        json!({"issuer":origin,"authorization_endpoint":format!("{origin}/auth"),"token_endpoint":format!("{origin}/token"),"token_endpoint_auth_methods_supported":["client_secret_post"]}),
    )
}
async fn token(State(p): State<Provider>, body: String) -> Json<Value> {
    p.token_forms.lock().unwrap().push(body.clone());
    let form =
        url::form_urlencoded::parse(body.as_bytes()).collect::<std::collections::HashMap<_, _>>();
    assert_eq!(form["client_secret"], "fixture-client-secret");
    assert!(!form.contains_key("resource"));
    let service = if form
        .get("code")
        .or_else(|| form.get("refresh_token"))
        .unwrap()
        .contains("calendar")
    {
        "calendar"
    } else {
        "drive"
    };
    if form["grant_type"] == "authorization_code" {
        assert!(!form["code_verifier"].is_empty());
        assert_eq!(form["redirect_uri"], "https://host.example/callback");
    }
    Json(
        json!({"access_token":format!("fixture-{service}-access"),"refresh_token":format!("fixture-{service}-refresh"),"expires_in":3600,"token_type":"Bearer","scope":if service=="calendar" {CALENDAR_SCOPE} else {DRIVE_SCOPE}}),
    )
}
async fn read(
    State(p): State<Provider>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let calendar = uri.path().contains("calendar/");
    let expected = if calendar {
        "Bearer fixture-calendar-access"
    } else {
        "Bearer fixture-drive-access"
    };
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some(expected) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    *p.reads.lock().unwrap() += 1;
    Json(if calendar {
        json!({"items":[{"id":"event","summary":"Fixture meeting"}],"timeZone":"UTC"})
    } else {
        json!({"files":[{"id":"file","name":"Fixture plan"}]})
    })
    .into_response()
}
async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (origin, task)
}
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL with independent connector schema"]
async fn oauth_package_reads_and_revocation_use_the_public_governed_services() {
    let pool = sqlx::PgPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let deployment: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_deployments(external_key) VALUES($1) RETURNING id",
    )
    .bind(format!("google-{}", Uuid::new_v4()))
    .fetch_one(&pool)
    .await
    .unwrap();
    let user: Uuid = sqlx::query_scalar("INSERT INTO users DEFAULT VALUES RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO user_contexts(user_id,deployment_id) VALUES($1,$2) RETURNING id",
    )
    .bind(user)
    .bind(deployment)
    .fetch_one(&pool)
    .await
    .unwrap();
    let foreign: Uuid = sqlx::query_scalar(
        "INSERT INTO user_contexts(user_id,deployment_id) VALUES($1,$2) RETURNING id",
    )
    .bind(user)
    .bind(deployment)
    .fetch_one(&pool)
    .await
    .unwrap();
    let scope = RequestContext {
        id: UserContextId(context),
        user_id: UserId(user),
        subject: RequestSubject {
            deployment_id: DeploymentId(deployment),
        },
    };
    let foreign = RequestContext {
        id: UserContextId(foreign),
        ..scope
    };
    for name in ["assistant", "specialist"] {
        let agent:Uuid=sqlx::query_scalar("INSERT INTO agent_definitions(deployment_id,owner_user_context_id,external_key,requested_capability_categories) VALUES($1,$2,$3,ARRAY['*']) RETURNING id").bind(deployment).bind(context).bind(name).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO deployment_agent_selections(deployment_id,agent_definition_id) VALUES($1,$2)").bind(deployment).bind(agent).execute(&pool).await.unwrap();
    }
    let provider = Provider::default();
    let (origin, provider_task) = serve(
        Router::new()
            .route("/.well-known/oauth-authorization-server", get(metadata))
            .route("/token", post(token))
            .route("/calendar/v3/calendars/primary/events", get(read))
            .route("/drive/v3/files", get(read))
            .with_state(provider.clone()),
    )
    .await;
    *provider.origin.lock().unwrap() = origin.clone();
    let (adapter, adapter_task) = serve(
        GoogleReadAdapter::with_local_upstream_for_testing(&origin)
            .unwrap()
            .router(),
    )
    .await;
    let clients = json!({format!("{adapter}/calendar/mcp"):{"issuer":origin,"client_id":"calendar-client","client_secret":"fixture-client-secret","token_endpoint_auth_method":"client_secret_post","scopes":[CALENDAR_SCOPE],"send_resource":false,"authorize_params":{"access_type":"offline"}},format!("{adapter}/drive/mcp"):{"issuer":origin,"client_id":"drive-client","client_secret":"fixture-client-secret","token_endpoint_auth_method":"client_secret_post","scopes":[DRIVE_SCOPE],"send_resource":false,"authorize_params":{"access_type":"offline"}}});
    let apps = ConnectedAppsService::from_options(
        pool.clone(),
        ConnectedAppsOptions {
            credential_key: Some("a".repeat(64)),
            redirect_uris: vec!["https://host.example/callback".into()],
            oauth_clients: Some(clients.to_string()),
        },
    )
    .with_local_endpoints_for_testing();
    let registry = PackageRegistry::new(pool.clone());
    let setup = ConnectorSetup::new(pool.clone());
    for (calendar, text, name, expected_scope) in [
        (
            true,
            include_str!("../examples/packages/google-calendar-reader/vox-package.json"),
            CALENDAR_TOOL,
            CALENDAR_SCOPE,
        ),
        (
            false,
            include_str!("../examples/packages/google-drive-metadata-reader/vox-package.json"),
            DRIVE_TOOL,
            DRIVE_SCOPE,
        ),
    ] {
        let package: Value = serde_json::from_str(text).unwrap();
        let mut manifest: vox_connections::remote_extensions::InstallExtensionRequest =
            serde_json::from_value(package["manifest"].clone()).unwrap();
        let metadata: PackageMetadata =
            serde_json::from_value(package["metadata"].clone()).unwrap();
        let digest = hex::encode(Sha256::digest(package_bytes(&manifest, &metadata).unwrap()));
        let review = json!({"schema_version":1,"package_digest":digest,"package_version":1,"protocol_version":"2025-11-25","live_inventory_verified":true,"behavior_certified":true,"read_effects_verified":true,"evidence":{"reviewer":"isolated fixture only","report_digest":"f".repeat(64)}});
        registry
            .publish(PublishPackage {
                deployment_id: deployment,
                version: 1,
                manifest: manifest.clone(),
                metadata,
                review,
            })
            .await
            .unwrap();
        let extension = registry
            .install(&scope, &manifest.external_key, 1, &digest)
            .await
            .unwrap();
        // Only fixture transport is rebound to loopback; production publication rejects local URLs.
        manifest.endpoint_url = format!(
            "{adapter}/{}/mcp",
            if calendar { "calendar" } else { "drive" }
        );
        sqlx::query("UPDATE remote_extensions SET endpoint_url=$2 WHERE id=$1")
            .bind(extension.id)
            .bind(&manifest.endpoint_url)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE connector_packages SET manifest=$3 WHERE deployment_id=$1 AND external_key=$2",
        )
        .bind(deployment)
        .bind(&manifest.external_key)
        .bind(serde_json::to_value(&manifest).unwrap())
        .execute(&pool)
        .await
        .unwrap();
        let start = setup
            .start(
                &scope,
                &apps,
                SetupRequest {
                    external_key: manifest.external_key.clone(),
                    version: 1,
                    digest: digest.clone(),
                    redirect_uri: "https://host.example/callback".into(),
                    consent: Some(SetupConsent {
                        agent_external_key: "assistant".into(),
                        agent_instruction_version: 1,
                        capability_external_keys: vec![name.into()],
                        enable_bundled_skills: false,
                    }),
                },
            )
            .await
            .unwrap();
        assert_eq!(start.state, SetupState::Authorize);
        let auth = url::Url::parse(start.authorization_url.as_ref().unwrap()).unwrap();
        let params = auth
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(params["scope"], expected_scope);
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["access_type"], "offline");
        assert!(!params.contains_key("resource"));
        let code = if calendar {
            "calendar-code"
        } else {
            "drive-code"
        };
        let complete = setup
            .complete(&scope, &apps, &params["state"], code, Some(&origin))
            .await
            .unwrap();
        assert_eq!(complete.state, SetupState::Complete);
        let connection: Uuid =
            sqlx::query_scalar("SELECT id FROM external_connections WHERE remote_extension_id=$1")
                .bind(extension.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let (access,refresh):(Vec<u8>,Vec<u8>)=sqlx::query_as("SELECT access_token_ciphertext,refresh_token_ciphertext FROM remote_extension_credentials WHERE extension_id=$1").bind(extension.id).fetch_one(&pool).await.unwrap();
        assert!(!String::from_utf8_lossy(&access).contains("fixture-"));
        assert!(!String::from_utf8_lossy(&refresh).contains("fixture-"));
        let arguments = if calendar {
            json!({"time_min":"2026-10-01T00:00:00Z","time_max":"2026-10-02T00:00:00Z"})
        } else {
            json!({"name_contains":"plan"})
        };
        for (actor, agent) in [(&foreign, "assistant"), (&scope, "specialist")] {
            assert!(matches!(
                apps.read_tool(actor, agent, connection, name, arguments.clone())
                    .await,
                Err(ConnectedAppError::GrantRequired)
            ));
        }
        sqlx::query("UPDATE remote_extension_credentials SET expires_at=now()+interval '30 seconds' WHERE extension_id=$1").bind(extension.id).execute(&pool).await.unwrap();
        let result = apps
            .read_tool(&scope, "assistant", connection, name, arguments.clone())
            .await
            .unwrap();
        assert_eq!(result["structuredContent"]["complete"], true);
        assert_eq!(
            provider.token_forms.lock().unwrap().len(),
            if calendar { 2 } else { 4 }
        );
        CapabilityGrantService::new(pool.clone())
            .revoke(
                &scope,
                CreateGrantRequest {
                    agent_external_key: "assistant".into(),
                    connection_id: connection,
                    capability_external_key: name.into(),
                },
            )
            .await
            .unwrap();
        let prior = *provider.reads.lock().unwrap();
        assert!(matches!(
            apps.read_tool(&scope, "assistant", connection, name, arguments.clone())
                .await,
            Err(ConnectedAppError::GrantRequired)
        ));
        assert_eq!(*provider.reads.lock().unwrap(), prior);
        let replay = setup
            .complete(&scope, &apps, &params["state"], code, Some(&origin))
            .await
            .unwrap();
        assert_eq!(replay.state, SetupState::Complete);
        assert!(
            apps.read_tool(&scope, "assistant", connection, name, arguments)
                .await
                .is_err()
        );
        ConnectionService::new(pool.clone())
            .disconnect(&scope, connection)
            .await
            .unwrap();
        assert!(
            apps.tool_for_agent(&scope, "assistant", connection, name)
                .await
                .is_err()
        );
    }
    adapter_task.abort();
    provider_task.abort();
}
