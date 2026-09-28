//! Run against a disposable database after applying the matching connector schema.
use serde_json::json;
use uuid::Uuid;
use vox_connections::{
    capability_grants::{CapabilityGrantService, CreateGrantRequest},
    connected_apps::{ConnectedAppError, ConnectedAppsOptions, ConnectedAppsService},
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
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("second context")
    } else {
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO user_contexts (deployment_id, user_id) VALUES ($1,$2) RETURNING id",
        )
        .bind(deployment_id)
        .bind(user_id)
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
    sqlx::query("INSERT INTO deployment_agent_selections (deployment_id,agent_definition_id) VALUES ($1,$2)")
        .bind(deployment_id)
        .bind(agent_id)
        .execute(&pool)
        .await
        .expect("select agent");
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
