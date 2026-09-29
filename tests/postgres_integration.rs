//! Run against a disposable database after applying the matching connector schema.
use serde_json::json;
use uuid::Uuid;
use vox_connections::{
    capability_grants::{CapabilityGrantService, CreateGrantRequest},
    connected_apps::{ConnectedAppError, ConnectedAppsOptions, ConnectedAppsService},
    connections::{AuthorizationState, ConnectionService},
    identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId},
    integration_registry::{IntegrationRegistry, SetIntegrationEnabledRequest},
    providers::expedia::integration_declaration,
    remote_extensions::{
        ExtensionCapability, ExtensionEffect, ExtensionOperator, ExtensionProtocol,
        InstallExtensionRequest, RemoteExtensionService, UpdateExtensionRequest,
    },
    skills::{PublishSkillRequest, SkillService},
};

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and the matching database schema"]
async fn independently_registered_host_can_install_skill_and_extension() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to isolated database");
    let suffix = Uuid::new_v4();
    let deployment_key = format!("connector-test-{suffix}");
    let deployment_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO platform_deployments (external_key) VALUES ($1) RETURNING id",
    )
    .bind(&deployment_key)
    .fetch_one(&pool)
    .await
    .expect("deployment");
    let user_id = sqlx::query_scalar::<_, Uuid>("INSERT INTO users DEFAULT VALUES RETURNING id")
        .fetch_one(&pool)
        .await
        .expect("user");
    let core_host_schema: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name='user_contexts' AND column_name='host_app_id')",
    ).fetch_one(&pool).await.expect("inspect host schema");
    let context_id = if core_host_schema {
        let app_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO host_apps (deployment_id, external_key) VALUES ($1, 'independent-host') RETURNING id",
        ).bind(deployment_id).fetch_one(&pool).await.expect("host app");
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO user_contexts (deployment_id, host_app_id, host_user_id, user_id) VALUES ($1, $2, $3, $4) RETURNING id",
        ).bind(deployment_id).bind(app_id).bind(format!("user-{suffix}")).bind(user_id)
            .fetch_one(&pool).await.expect("context")
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO user_contexts (deployment_id, user_id) VALUES ($1, $2) RETURNING id",
        )
        .bind(deployment_id)
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("context")
    };
    let context = RequestContext {
        id: UserContextId(context_id),
        user_id: UserId(user_id),
        subject: RequestSubject {
            deployment_id: DeploymentId(deployment_id),
        },
    };

    let registry = IntegrationRegistry::new(pool.clone());
    registry
        .register(integration_declaration(&deployment_key))
        .await
        .expect("register provider");
    registry
        .set_enabled(SetIntegrationEnabledRequest {
            deployment_external_key: deployment_key.clone(),
            external_key: "expedia".into(),
            enabled: true,
        })
        .await
        .expect("enable provider");
    assert_eq!(
        registry
            .discover(&deployment_key)
            .await
            .expect("discover provider")
            .len(),
        3
    );

    let skills = SkillService::new(pool.clone());
    let listing = skills
        .publish_private(
            &context,
            PublishSkillRequest {
                external_key: "daily-summary".into(),
                title: "Daily summary".into(),
                summary: "Summarize today's activity".into(),
                instructions: "Summarize the user's selected activity.".into(),
                requested_capabilities: vec![],
                resources: json!({}),
            },
        )
        .await
        .expect("publish skill");
    assert!(
        skills
            .list(&context)
            .await
            .expect("list skills")
            .iter()
            .any(|item| item.id == listing.id)
    );

    let extensions = RemoteExtensionService::new(pool.clone());
    let packages = vox_connections::packages::PackageRegistry::new(pool.clone());
    let manifest: InstallExtensionRequest =
        serde_json::from_str(include_str!("../examples/mcp/read-only-manifest.json"))
            .expect("package manifest");
    let package_request = vox_connections::packages::PublishPackage {
        metadata: vox_connections::packages::PackageMetadata::oauth(),
        deployment_id,
        version: 1,
        manifest: manifest.clone(),
        review: json!({"read_effects_verified":true,"evidence":{"reviewer":"test-operator"}}),
    };
    let package = packages
        .publish(package_request.clone())
        .await
        .expect("publish package");
    packages
        .publish(package_request.clone())
        .await
        .expect("same immutable package is idempotent");
    let mut changed = package_request;
    changed.manifest.endpoint_url = "https://different.example/mcp".into();
    assert!(matches!(
        packages.publish(changed).await,
        Err(vox_connections::packages::PackageError::Conflict)
    ));
    assert_eq!(packages.list(&context).await.expect("catalog").len(), 1);
    assert!(matches!(
        packages
            .install(&context, &manifest.external_key, 1, &"0".repeat(64))
            .await,
        Err(vox_connections::packages::PackageError::Conflict)
    ));
    // A one-connection pool must work: install must never acquire another
    // connection while holding its transaction or advisory lock.
    let single_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("single connection pool");
    let single_packages = vox_connections::packages::PackageRegistry::new(single_pool.clone());
    let other_deployment = RequestContext {
        subject: RequestSubject {
            deployment_id: DeploymentId(Uuid::new_v4()),
        },
        ..context
    };
    assert!(
        packages
            .list(&other_deployment)
            .await
            .expect("other deployment catalog")
            .is_empty()
    );
    assert!(matches!(
        packages
            .install(
                &other_deployment,
                &manifest.external_key,
                1,
                &package.digest
            )
            .await,
        Err(vox_connections::packages::PackageError::Unavailable)
    ));
    let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(
            single_packages.install(&context, &manifest.external_key, 1, &package.digest),
            packages.install(&context, &manifest.external_key, 1, &package.digest)
        )
    })
    .await
    .expect("concurrent installs must complete without a lock cycle");
    single_pool.close().await;
    let first = first.expect("first click");
    assert_eq!(first.id, second.expect("concurrent click").id);
    assert_eq!(
        first.lifecycle_state,
        vox_connections::remote_extensions::LifecycleState::Active
    );
    assert!(
        CapabilityGrantService::new(pool.clone())
            .effective_for_agent(&context, "general")
            .await
            .expect("no implicit grants")
            .is_empty()
    );
    // Two packages sharing one bundled skill can be withdrawn concurrently.
    // The final association removal must disable the non-independent skill.
    let bundled_request = PublishSkillRequest {
        external_key: "shared-bundle".into(),
        title: "Shared bundle".into(),
        summary: "Shared guidance".into(),
        instructions: "Summarize supplied facts.".into(),
        requested_capabilities: vec![],
        resources: json!({}),
    };
    let bundled_id = skills
        .publish_curated(&deployment_key, bundled_request.clone())
        .await
        .expect("publish bundled skill");
    let bundled_digest = vox_connections::skills::content_digest(&bundled_request).unwrap();
    let mut bundle_packages = Vec::new();
    for n in 1..=2 {
        let mut bundled_manifest = manifest.clone();
        bundled_manifest.external_key = format!("bundle-app-{n}");
        let mut metadata = vox_connections::packages::PackageMetadata::oauth();
        metadata
            .skills
            .push(vox_connections::packages::PinnedSkill {
                external_key: "shared-bundle".into(),
                version: 1,
                digest: bundled_digest.clone(),
            });
        let package = packages
            .publish(vox_connections::packages::PublishPackage {
                deployment_id,
                version: 1,
                manifest: bundled_manifest.clone(),
                metadata,
                review: json!({"reviewer":"test-operator"}),
            })
            .await
            .expect("publish bundle package");
        let installed = packages
            .install(&context, &bundled_manifest.external_key, 1, &package.digest)
            .await
            .expect("install bundle package");
        bundle_packages.push((bundled_manifest.external_key, installed.id));
    }
    let enabled: bool = sqlx::query_scalar(
        "SELECT enabled FROM skill_installations WHERE user_context_id=$1 AND skill_id=$2",
    )
    .bind(context_id)
    .bind(bundled_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(enabled);
    let withdrawal = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(
            packages.withdraw(deployment_id, &bundle_packages[0].0, 1),
            packages.withdraw(deployment_id, &bundle_packages[1].0, 1)
        )
    })
    .await
    .expect("concurrent withdrawal completes");
    withdrawal.0.expect("first withdrawal");
    withdrawal.1.expect("second withdrawal");
    let enabled: bool = sqlx::query_scalar(
        "SELECT enabled FROM skill_installations WHERE user_context_id=$1 AND skill_id=$2",
    )
    .bind(context_id)
    .bind(bundled_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!enabled, "orphaned bundle must be disabled");
    let associations:i64=sqlx::query_scalar("SELECT count(*) FROM connector_skill_installations WHERE skill_id=$1 AND user_context_id=$2").bind(bundled_id).bind(context_id).fetch_one(&pool).await.unwrap();
    assert_eq!(associations, 0);
    sqlx::query("INSERT INTO remote_extension_credentials (extension_id,issuer,token_endpoint,client_id,resource,access_token_ciphertext) VALUES ($1,'https://example.com','https://example.com/token','client','https://example.com/mcp',$2)")
        .bind(first.id).bind(vec![1_u8,2,3]).execute(&pool).await.expect("package credential");
    let package_connection: Uuid = sqlx::query_scalar("INSERT INTO external_connections (user_context_id,remote_extension_id,external_account_hash,credential_custody,authorization_state,authorized_capabilities) VALUES ($1,$2,$3,'platform_held','authorized',ARRAY['echo.read']) RETURNING id")
        .bind(context_id).bind(first.id).bind(vec![9_u8;32]).fetch_one(&pool).await.expect("package account");
    let account_listing = ConnectionService::new(pool.clone());
    let connected_apps =
        ConnectedAppsService::from_options(pool.clone(), ConnectedAppsOptions::default());
    assert_eq!(
        account_listing
            .get(&context, package_connection)
            .await
            .expect("current account")
            .authorization_state,
        AuthorizationState::Authorized
    );
    sqlx::query("UPDATE remote_extension_credentials SET expires_at=now()-interval '1 second' WHERE extension_id=$1")
        .bind(first.id).execute(&pool).await.expect("expire credential");
    let expired = account_listing
        .get(&context, package_connection)
        .await
        .expect("expired account");
    assert_eq!(expired.authorization_state, AuthorizationState::Expired);
    assert!(expired.expires_at.is_some());
    assert!(
        connected_apps
            .connections(&context)
            .await
            .expect("linked account status")
            .is_empty()
    );
    // A provider can rotate both tokens. Refresh must restore account
    // availability without replacing a grant or requiring a second login.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("refresh server");
    let port = listener.local_addr().unwrap().port();
    let refresh_server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().expect("refresh request");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = vec![0_u8; 4096];
        let size = stream.read(&mut request).expect("read refresh request");
        assert!(String::from_utf8_lossy(&request[..size]).contains("POST /token"));
        let body = r#"{"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":3600,"token_type":"Bearer"}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("send rotated tokens");
    });
    let cipher =
        vox_connections::connected_apps::crypto::CredentialCipher::from_hex_key(&"00".repeat(32))
            .unwrap();
    let mut access_aad = first.id.as_bytes().to_vec();
    access_aad.extend_from_slice(b"access");
    let mut refresh_aad = first.id.as_bytes().to_vec();
    refresh_aad.extend_from_slice(b"refresh");
    sqlx::query("UPDATE remote_extension_credentials SET token_endpoint=$2,access_token_ciphertext=$3,refresh_token_ciphertext=$4 WHERE extension_id=$1")
        .bind(first.id)
        .bind(format!("http://127.0.0.1:{port}/token"))
        .bind(cipher.seal(&access_aad, "expired-access").unwrap())
        .bind(cipher.seal(&refresh_aad, "first-refresh").unwrap())
        .execute(&pool).await.expect("set expiring provider tokens");
    let refreshing_apps = ConnectedAppsService::from_options(pool.clone(), ConnectedAppsOptions {
        credential_key: Some("00".repeat(32)),
        oauth_clients: Some(r#"{"mcp.example.com":{"client_id":"client","client_secret":"fixture-secret","send_resource":false}}"#.into()),
        ..ConnectedAppsOptions::default()
    }).with_local_endpoints_for_testing();
    assert_eq!(
        refreshing_apps
            .connections(&context)
            .await
            .expect("refresh account")
            .len(),
        1
    );
    refresh_server.join().expect("refresh server completed");
    let rotated: (Vec<u8>, Vec<u8>) = sqlx::query_as("SELECT access_token_ciphertext,refresh_token_ciphertext FROM remote_extension_credentials WHERE extension_id=$1")
        .bind(first.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        cipher.open(&access_aad, &rotated.0).unwrap(),
        "rotated-access"
    );
    assert_eq!(
        cipher.open(&refresh_aad, &rotated.1).unwrap(),
        "rotated-refresh"
    );
    sqlx::query("UPDATE remote_extension_credentials SET expires_at=now()+interval '1 hour' WHERE extension_id=$1")
        .bind(first.id).execute(&pool).await.expect("renew credential fixture");
    assert_eq!(
        account_listing
            .get(&context, package_connection)
            .await
            .expect("renewed account")
            .authorization_state,
        AuthorizationState::Authorized
    );
    assert_eq!(
        connected_apps
            .connections(&context)
            .await
            .expect("current linked status")
            .len(),
        1
    );
    packages
        .withdraw(deployment_id, &manifest.external_key, 1)
        .await
        .expect("withdraw");
    assert!(
        packages
            .list(&context)
            .await
            .expect("withdrawn catalog")
            .is_empty()
    );
    assert!(matches!(
        packages
            .install(&context, &manifest.external_key, 1, &package.digest)
            .await,
        Err(vox_connections::packages::PackageError::Unavailable)
    ));
    assert_eq!(
        extensions
            .get(&context, first.id)
            .await
            .expect("withdrawn installation")
            .lifecycle_state,
        vox_connections::remote_extensions::LifecycleState::Disabled
    );
    let credentials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM remote_extension_credentials WHERE extension_id=$1",
    )
    .bind(first.id)
    .fetch_one(&pool)
    .await
    .expect("withdraw credential count");
    assert_eq!(credentials, 0);
    let state: String =
        sqlx::query_scalar("SELECT authorization_state FROM external_connections WHERE id=$1")
            .bind(package_connection)
            .fetch_one(&pool)
            .await
            .expect("withdraw account state");
    assert_eq!(state, "revoked");
    assert!(
        extensions
            .set_operator_enabled(&context, first.id, true)
            .await
            .is_err(),
        "operator reenablement must not revive a withdrawn package"
    );
    // Even an out-of-band lifecycle change cannot bypass the package's trust
    // boundary at OAuth start/callback or the generic endpoint authorization seam.
    sqlx::query(
        "UPDATE remote_extensions SET lifecycle_state='active',operator_enabled=true WHERE id=$1",
    )
    .bind(first.id)
    .execute(&pool)
    .await
    .expect("force stale active fixture");
    assert!(
        extensions
            .authorize_call(&context, first.id, &manifest.capabilities[0].external_key)
            .await
            .is_err()
    );
    let package_apps = ConnectedAppsService::from_options(
        pool.clone(),
        ConnectedAppsOptions {
            credential_key: Some("00".repeat(32)),
            redirect_uris: vec!["https://host.example/callback".into()],
            ..ConnectedAppsOptions::default()
        },
    );
    assert!(matches!(
        package_apps
            .begin(&context, first.id, "https://host.example/callback")
            .await,
        Err(ConnectedAppError::Invalid)
    ));
    let withdrawn_state = format!("withdrawn-oauth-{suffix}");
    sqlx::query("INSERT INTO mcp_authorization_sessions (extension_id,user_context_id,state_hash,code_verifier_ciphertext,issuer,token_endpoint,client_id,redirect_uri,resource,endpoint_url,extension_version,expires_at) VALUES ($1,$2,$3,$4,'https://example.com','https://example.com/token','client','https://host.example/callback',$5,$5,1,now() + interval '10 minutes')")
        .bind(first.id).bind(context_id).bind(vox_connections::connected_apps::crypto::sha256_hex(&withdrawn_state))
        .bind(vec![4_u8,5,6]).bind(&manifest.endpoint_url).execute(&pool).await.expect("stale withdrawn session");
    assert!(matches!(
        package_apps
            .complete(&context, &withdrawn_state, "code")
            .await,
        Err(ConnectedAppError::Expired)
    ));
    packages
        .withdraw(deployment_id, &manifest.external_key, 1)
        .await
        .expect("idempotent withdrawal clears stale state");

    let mut write_manifest = manifest.clone();
    write_manifest.external_key = "reviewed-write-fixture".into();
    for capability in &mut write_manifest.capabilities {
        capability.effect = ExtensionEffect::Write;
        capability.consequential = true;
    }
    let write_package = packages
        .publish(vox_connections::packages::PublishPackage {
            metadata: vox_connections::packages::PackageMetadata::oauth(),
            deployment_id,
            version: 1,
            manifest: write_manifest.clone(),
            // Even this read-attestation must never activate a consequential package.
            review: json!({"read_effects_verified":true,"evidence":{"reviewer":"test-operator"}}),
        })
        .await
        .expect("publish write package");
    let write_extension = packages
        .install(
            &context,
            &write_manifest.external_key,
            1,
            &write_package.digest,
        )
        .await
        .expect("install pending write");
    assert!(!write_extension.operator_enabled);
    assert_eq!(
        write_extension.conformance_status,
        vox_connections::remote_extensions::ConformanceStatus::Pending
    );
    assert_eq!(
        write_extension.lifecycle_state,
        vox_connections::remote_extensions::LifecycleState::Installed
    );
    packages
        .withdraw(deployment_id, &write_manifest.external_key, 1)
        .await
        .expect("withdraw write fixture");
    let installed = extensions
        .install(
            &context,
            InstallExtensionRequest {
                external_key: "independent-weather".into(),
                display_name: "Independent weather".into(),
                protocol: ExtensionProtocol::Mcp,
                endpoint_url: "https://example.com/mcp".into(),
                operator: ExtensionOperator {
                    operator_id: "independent-operator".into(),
                    operator_name: "Independent Operator".into(),
                    support_email: None,
                    terms_url: None,
                },
                capabilities: vec![ExtensionCapability {
                    input_schema: json!({"type":"object"}),
                    supported_regions: vec![],
                    external_key: "weather.read".into(),
                    display_name: "Read weather".into(),
                    effect: ExtensionEffect::Read,
                    consequential: false,
                    data_recipients: vec!["Independent Operator".into()],
                    access_needs: vec!["city".into()],
                    optional_guarantees: json!({}),
                }],
            },
        )
        .await
        .expect("install extension");
    assert!(!installed.operator_enabled);
    assert!(
        extensions
            .list(&context)
            .await
            .expect("list extensions")
            .iter()
            .any(|item| item.id == installed.id)
    );

    sqlx::query("INSERT INTO remote_extension_credentials (extension_id,issuer,token_endpoint,client_id,resource,access_token_ciphertext) VALUES ($1,'https://example.com','https://example.com/token','client','https://example.com/mcp',$2)")
        .bind(installed.id)
        .bind(vec![1_u8, 2, 3])
        .execute(&pool)
        .await
        .expect("credential fixture");
    let connection_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO external_connections \
         (user_context_id,remote_extension_id,external_account_hash,credential_custody,authorization_state,authorized_capabilities) \
         VALUES ($1,$2,$3,'platform_held','authorized',ARRAY['weather.read']) RETURNING id",
    )
    .bind(context_id)
    .bind(installed.id)
    .bind(vec![7_u8; 32])
    .fetch_one(&pool)
    .await
    .expect("governed connection fixture");
    sqlx::query("INSERT INTO mcp_authorization_sessions (extension_id,user_context_id,state_hash,code_verifier_ciphertext,issuer,token_endpoint,client_id,redirect_uri,resource,endpoint_url,extension_version,expires_at) VALUES ($1,$2,$3,$4,'https://example.com','https://example.com/token','client','https://host.example/callback','https://example.com/mcp','https://example.com/mcp',1,now() + interval '10 minutes')")
        .bind(installed.id)
        .bind(context_id)
        .bind(format!("test-state-{suffix}"))
        .bind(vec![4_u8, 5, 6])
        .execute(&pool)
        .await
        .expect("pending OAuth fixture");
    // Core deliberately creates a separate internal user for each host context;
    // it never links people across hosts from matching account identifiers.
    // Independent hosts may reuse a user ID, so also exercise that stricter case.
    let other_user_id = if core_host_schema {
        sqlx::query_scalar::<_, Uuid>("INSERT INTO users DEFAULT VALUES RETURNING id")
            .fetch_one(&pool)
            .await
            .expect("other host user")
    } else {
        user_id
    };
    let other_context_id = if core_host_schema {
        let app_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO host_apps (deployment_id, external_key) VALUES ($1, 'other-host') RETURNING id",
        )
        .bind(deployment_id)
        .fetch_one(&pool)
        .await
        .expect("second host app");
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO user_contexts (deployment_id, host_app_id, host_user_id, user_id) VALUES ($1,$2,$3,$4) RETURNING id",
        )
        .bind(deployment_id)
        .bind(app_id)
        .bind(format!("other-user-{suffix}"))
        .bind(other_user_id)
        .fetch_one(&pool)
        .await
        .expect("second context")
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO user_contexts (deployment_id, user_id) VALUES ($1,$2) RETURNING id",
        )
        .bind(deployment_id)
        .bind(other_user_id)
        .fetch_one(&pool)
        .await
        .expect("second context")
    };
    sqlx::query("UPDATE remote_extensions SET lifecycle_state='active' WHERE id=$1")
        .bind(installed.id)
        .execute(&pool)
        .await
        .expect("activate fixture");
    let other_context = RequestContext {
        id: UserContextId(other_context_id),
        user_id: UserId(other_user_id),
        ..context
    };
    let agent_id = if core_host_schema {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO agent_definitions (deployment_id,external_key,purpose,requested_capability_categories) \
             VALUES ($1,'general','General purpose agent',ARRAY['*']) RETURNING id",
        )
        .bind(deployment_id)
        .fetch_one(&pool)
        .await
        .expect("agent")
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO agent_definitions (deployment_id,external_key,requested_capability_categories) \
             VALUES ($1,'general',ARRAY['*']) RETURNING id",
        )
        .bind(deployment_id)
        .fetch_one(&pool)
        .await
        .expect("agent")
    };
    if core_host_schema {
        let model_id: Uuid = sqlx::query_scalar("INSERT INTO agent_model_configurations(agent_definition_id,version,model_adapter,model) VALUES ($1,1,'fixture','fixture-model') RETURNING id")
            .bind(agent_id).fetch_one(&pool).await.expect("agent model configuration");
        sqlx::query("INSERT INTO deployment_agent_selections (deployment_id,agent_definition_id,model_configuration_id) VALUES ($1,$2,$3)")
            .bind(deployment_id).bind(agent_id).bind(model_id).execute(&pool).await.expect("select configured agent");
    } else {
        sqlx::query("INSERT INTO deployment_agent_selections (deployment_id,agent_definition_id) VALUES ($1,$2)")
            .bind(deployment_id).bind(agent_id).execute(&pool).await.expect("select agent");
    }
    let grants = CapabilityGrantService::new(pool.clone());
    grants
        .grant(
            &context,
            CreateGrantRequest {
                agent_external_key: "general".into(),
                connection_id,
                capability_external_key: "weather.read".into(),
            },
        )
        .await
        .expect("explicit agent grant");
    assert_eq!(
        grants
            .effective_for_agent(&context, "general")
            .await
            .expect("effective grants")
            .len(),
        1
    );
    assert!(
        grants
            .effective_for_agent(&other_context, "general")
            .await
            .expect("other context grants")
            .is_empty()
    );
    let apps = ConnectedAppsService::from_options(pool.clone(), ConnectedAppsOptions::default());
    assert_eq!(
        apps.connections(&context)
            .await
            .expect("owner connections")
            .len(),
        1
    );
    assert!(
        apps.connections(&other_context)
            .await
            .expect("other context connections")
            .is_empty()
    );
    assert!(matches!(
        apps.read_tool(
            &other_context,
            "general",
            connection_id,
            "weather.read",
            json!({})
        )
        .await,
        Err(ConnectedAppError::GrantRequired)
    ));
    let updated = extensions
        .update(
            &context,
            installed.id,
            UpdateExtensionRequest {
                endpoint_url: Some("https://replacement.example/mcp".into()),
                operator: Some(ExtensionOperator {
                    operator_id: "independent-operator".into(),
                    operator_name: "Independent Operator".into(),
                    support_email: Some("help@replacement.example".into()),
                    terms_url: Some("https://replacement.example/terms".into()),
                }),
                capabilities: None,
            },
        )
        .await
        .expect("update extension");
    assert_eq!(
        updated.operator.support_email.as_deref(),
        Some("help@replacement.example")
    );
    assert_eq!(
        updated.consent_status,
        vox_connections::remote_extensions::ConsentStatus::ConsentRequired
    );
    assert!(
        grants
            .effective_for_agent(&context, "general")
            .await
            .expect("revoked grants")
            .is_empty()
    );
    assert!(
        extensions
            .renew_consent(&other_context, installed.id, updated.current_version)
            .await
            .is_err()
    );
    assert!(
        extensions
            .renew_consent(&context, installed.id, installed.current_version)
            .await
            .is_err()
    );
    let renewed = extensions
        .renew_consent(&context, installed.id, updated.current_version)
        .await
        .expect("renew exact version consent");
    assert_eq!(
        renewed.consent_status,
        vox_connections::remote_extensions::ConsentStatus::Consented
    );
    assert!(
        grants
            .effective_for_agent(&context, "general")
            .await
            .expect("consent must not revive old grants")
            .is_empty()
    );
    for table in ["remote_extension_credentials", "mcp_authorization_sessions"] {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} WHERE extension_id=$1"
        ))
        .bind(installed.id)
        .fetch_one(&pool)
        .await
        .expect("count extension state");
        assert_eq!(count, 0, "{table} survives endpoint update");
    }
    let stale_state = format!("stale-oauth-{suffix}");
    sqlx::query("INSERT INTO mcp_authorization_sessions (extension_id,user_context_id,state_hash,code_verifier_ciphertext,issuer,token_endpoint,client_id,redirect_uri,resource,endpoint_url,extension_version,expires_at) VALUES ($1,$2,$3,$4,'https://example.com','https://example.com/token','client','https://host.example/callback','https://example.com/mcp','https://example.com/mcp',1,now() + interval '10 minutes')")
        .bind(installed.id)
        .bind(context_id)
        .bind(vox_connections::connected_apps::crypto::sha256_hex(&stale_state))
        .bind(vec![4_u8, 5, 6])
        .execute(&pool)
        .await
        .expect("stale OAuth fixture");
    let configured_apps = ConnectedAppsService::from_options(
        pool.clone(),
        ConnectedAppsOptions {
            credential_key: Some("00".repeat(32)),
            ..ConnectedAppsOptions::default()
        },
    );
    let issuer_state = format!("wrong-issuer-{suffix}");
    sqlx::query("INSERT INTO mcp_authorization_sessions (extension_id,user_context_id,state_hash,code_verifier_ciphertext,issuer,token_endpoint,client_id,redirect_uri,resource,endpoint_url,extension_version,expires_at) VALUES ($1,$2,$3,$4,'https://correct.example','https://correct.example/token','client','https://host.example/callback','https://replacement.example/mcp','https://replacement.example/mcp',$5,now() + interval '10 minutes')")
        .bind(installed.id)
        .bind(context_id)
        .bind(vox_connections::connected_apps::crypto::sha256_hex(&issuer_state))
        .bind(vec![4_u8, 5, 6])
        .bind(updated.current_version)
        .execute(&pool)
        .await
        .expect("issuer-bound OAuth fixture");
    assert!(matches!(
        configured_apps
            .complete_with_issuer(
                &context,
                &issuer_state,
                "code",
                Some("https://wrong.example")
            )
            .await,
        Err(ConnectedAppError::Unauthorized)
    ));
    assert!(matches!(
        configured_apps
            .complete(&context, &stale_state, "code")
            .await,
        Err(ConnectedAppError::Expired)
    ));
}
