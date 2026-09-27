//! Run against a disposable database after applying the matching connector schema.
use serde_json::json;
use uuid::Uuid;
use vox_connections::{
    identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId},
    integration_registry::{IntegrationRegistry, SetIntegrationEnabledRequest},
    providers::expedia::integration_declaration,
    remote_extensions::{
        ExtensionCapability, ExtensionEffect, ExtensionOperator, ExtensionProtocol,
        InstallExtensionRequest, RemoteExtensionService,
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

    let extensions = RemoteExtensionService::new(pool);
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
}
