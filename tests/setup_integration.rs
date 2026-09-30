//! Exercises the setup interface against the independent host schema.
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use vox_connections::{
    capability_grants::{CapabilityGrantService, CreateGrantRequest},
    connected_apps::{ConnectedAppsOptions, ConnectedAppsService},
    identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId},
    packages::{PackageMetadata, PackageRegistry, PinnedSkill, PublishPackage, package_bytes},
    remote_extensions::InstallExtensionRequest,
    setup::{ConnectorSetup, SetupConsent, SetupError, SetupRequest, SetupState},
    skills::{PublishSkillRequest, SkillService},
};

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL with the independent schema"]
async fn setup_is_atomic_scoped_and_callback_recovery_does_not_regrant() {
    let pool = PgPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let deployment_key = format!("setup-{}", Uuid::new_v4());
    let deployment: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_deployments(external_key) VALUES($1) RETURNING id",
    )
    .bind(&deployment_key)
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
    let other: Uuid = sqlx::query_scalar(
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
        id: UserContextId(other),
        ..scope
    };
    let agent: Uuid = sqlx::query_scalar("INSERT INTO agent_definitions(deployment_id,owner_user_context_id,external_key,requested_capability_categories) VALUES($1,$2,'general',ARRAY['*']) RETURNING id").bind(deployment).bind(context).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO deployment_agent_selections(deployment_id,agent_definition_id) VALUES($1,$2)",
    )
    .bind(deployment)
    .bind(agent)
    .execute(&pool)
    .await
    .unwrap();
    let skills = SkillService::new(pool.clone());
    let skill = skills
        .publish_curated(
            &deployment_key,
            PublishSkillRequest {
                external_key: "fixture-guidance".into(),
                title: "Fixture guidance".into(),
                summary: "Read fixture data".into(),
                instructions: "Use the fixture's reviewed read tool.".into(),
                requested_capabilities: vec![],
                resources: json!({}),
            },
        )
        .await
        .unwrap();
    let digest: String = sqlx::query_scalar(
        "SELECT digest FROM skill_package_versions WHERE skill_id=$1 AND version=1",
    )
    .bind(skill)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut metadata = PackageMetadata::oauth();
    metadata.skills.push(PinnedSkill {
        external_key: "fixture-guidance".into(),
        version: 1,
        digest,
    });
    let manifest: InstallExtensionRequest =
        serde_json::from_str(include_str!("../examples/mcp/read-only-manifest.json")).unwrap();
    let digest = hex::encode(Sha256::digest(package_bytes(&manifest, &metadata).unwrap()));
    let registry = PackageRegistry::new(pool.clone());
    registry.publish(PublishPackage {
        deployment_id: deployment, version:1, manifest:manifest.clone(),metadata,
        review: json!({"schema_version":1,"package_digest":digest,"package_version":1,"protocol_version":"2025-11-25","live_inventory_verified":true,"behavior_certified":true,"read_effects_verified":true,"evidence":{"reviewer":"test","report_digest":"f".repeat(64)}}),
    }).await.unwrap();
    let extension = registry
        .install(&scope, &manifest.external_key, 1, &digest)
        .await
        .unwrap();
    sqlx::query("INSERT INTO remote_extension_credentials(extension_id,issuer,token_endpoint,client_id,resource,access_token_ciphertext,tools) VALUES($1,'https://example.com','https://example.com/token','fixture','https://mcp.example.com/mcp',$2,$3)")
        .bind(extension.id).bind(vec![1_u8]).bind(json!([{"name":"echo.read","inputSchema":{"type":"object"}}])).execute(&pool).await.unwrap();
    let connection: Uuid = sqlx::query_scalar("INSERT INTO external_connections(user_context_id,remote_extension_id,external_account_hash,credential_custody,authorization_state,authorized_capabilities) VALUES($1,$2,$3,'platform_held','authorized',ARRAY['echo.read']) RETURNING id")
        .bind(context).bind(extension.id).bind(vec![0_u8;32]).fetch_one(&pool).await.unwrap();
    let apps = ConnectedAppsService::from_options(pool.clone(), ConnectedAppsOptions::default());
    let setup = ConnectorSetup::new(pool.clone());
    let consent = || SetupConsent {
        agent_external_key: "general".into(),
        agent_instruction_version: 1,
        capability_external_keys: vec!["echo.read".into()],
        enable_bundled_skills: true,
    };
    let request = |consent| SetupRequest {
        external_key: manifest.external_key.clone(),
        version: 1,
        digest: digest.clone(),
        redirect_uri: "https://host.example/callback".into(),
        consent,
    };
    assert!(matches!(
        setup.start(&foreign, &apps, request(Some(consent()))).await,
        Err(SetupError::ReviewRequired)
    ));
    let first = setup
        .start(&scope, &apps, request(Some(consent())))
        .await
        .unwrap();
    assert_eq!(first.state, SetupState::Complete);
    let enabled: bool = sqlx::query_scalar("SELECT enabled FROM skill_agent_enablements WHERE user_context_id=$1 AND skill_id=$2 AND agent_definition_id=$3").bind(context).bind(skill).bind(agent).fetch_one(&pool).await.unwrap();
    assert!(enabled);
    let grants = CapabilityGrantService::new(pool.clone());
    assert_eq!(
        grants
            .effective_for_agent(&scope, "general")
            .await
            .unwrap()
            .len(),
        1
    );
    grants
        .revoke(
            &scope,
            CreateGrantRequest {
                agent_external_key: "general".into(),
                connection_id: connection,
                capability_external_key: "echo.read".into(),
            },
        )
        .await
        .unwrap();
    // Simulate the crash window after account verification committed and before
    // setup completion. Both concurrent retries must finish exactly once.
    let state = format!("setup-state-{}", Uuid::new_v4());
    let session: Uuid = sqlx::query_scalar("INSERT INTO mcp_authorization_sessions(extension_id,user_context_id,state_hash,code_verifier_ciphertext,issuer,token_endpoint,client_id,redirect_uri,resource,endpoint_url,extension_version,expires_at,consumed_at,completed_at) VALUES($1,$2,$3,$4,'https://example.com','https://example.com/token','fixture','https://host.example/callback',$5,$5,1,now()+interval '10 minutes',now(),now()) RETURNING id")
        .bind(extension.id).bind(context).bind(hex::encode(Sha256::digest(state.as_bytes()))).bind(vec![1_u8]).bind(&manifest.endpoint_url).fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE connector_setups SET state='pending',created_at=now(),completed_at=NULL,authorization_session_id=$1 WHERE id=$2").bind(session).bind(first.setup_id).execute(&pool).await.unwrap();
    let (a, b) = tokio::join!(
        setup.complete(
            &scope,
            &apps,
            &state,
            "used-code",
            Some("https://example.com")
        ),
        setup.complete(&scope, &apps, &state, "used-code", None)
    );
    assert_eq!(a.unwrap().state, SetupState::Complete);
    assert_eq!(b.unwrap().state, SetupState::Complete);
    assert!(
        setup
            .complete(&foreign, &apps, &state, "used-code", None)
            .await
            .is_err()
    );
    assert!(
        setup
            .complete(
                &scope,
                &apps,
                &state,
                "used-code",
                Some("https://attacker.example")
            )
            .await
            .is_err()
    );
    grants
        .revoke(
            &scope,
            CreateGrantRequest {
                agent_external_key: "general".into(),
                connection_id: connection,
                capability_external_key: "echo.read".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        setup
            .complete(&scope, &apps, &state, "used-code", None)
            .await
            .unwrap()
            .state,
        SetupState::Complete
    );
    assert!(
        grants
            .effective_for_agent(&scope, "general")
            .await
            .unwrap()
            .is_empty()
    );
    // A revoked grant cannot be resurrected by a still-pending OAuth tab.
    sqlx::query("UPDATE connector_setups SET state='pending',completed_at=NULL WHERE id=$1")
        .bind(first.setup_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        setup
            .complete(&scope, &apps, &state, "used-code", None)
            .await
            .unwrap()
            .state,
        SetupState::NeedsReview
    );
    // Fault after skill enablement but before the grant commits: no partial
    // permission or guidance changes may survive, and callback retry remains safe.
    skills
        .set_agent_enabled(&scope, "general", skill, false)
        .await
        .unwrap();
    sqlx::query("UPDATE connector_setups SET state='pending',created_at=now(),completed_at=NULL WHERE id=$1")
        .bind(first.setup_id).execute(&pool).await.unwrap();
    let hook = format!("setup_fault_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE FUNCTION {hook}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.user_context_id='{context}'::uuid THEN RAISE EXCEPTION 'injected grant failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER {hook} BEFORE INSERT ON agent_capability_grants FOR EACH ROW EXECUTE FUNCTION {hook}();"))
        .execute(&pool).await.unwrap();
    assert!(matches!(
        setup
            .complete(&scope, &apps, &state, "used-code", None)
            .await,
        Err(SetupError::Database(_))
    ));
    let enabled: bool = sqlx::query_scalar("SELECT enabled FROM skill_agent_enablements WHERE user_context_id=$1 AND skill_id=$2 AND agent_definition_id=$3")
        .bind(context).bind(skill).bind(agent).fetch_one(&pool).await.unwrap();
    assert!(!enabled);
    assert!(
        grants
            .effective_for_agent(&scope, "general")
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::raw_sql(&format!(
        "DROP TRIGGER {hook} ON agent_capability_grants; DROP FUNCTION {hook}();"
    ))
    .execute(&pool)
    .await
    .unwrap();
    // A pinned skill invalidated during sign-in blocks all new enablement.
    sqlx::query("UPDATE skill_packages SET state='removed' WHERE id=$1")
        .bind(skill)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE connector_setups SET state='pending',created_at=now(),completed_at=NULL WHERE id=$1")
        .bind(first.setup_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        setup
            .complete(&scope, &apps, &state, "used-code", None)
            .await
            .unwrap()
            .state,
        SetupState::NeedsReview
    );
    assert!(
        grants
            .effective_for_agent(&scope, "general")
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE skill_packages SET state='active' WHERE id=$1")
        .bind(skill)
        .execute(&pool)
        .await
        .unwrap();
    // An agent edit between sign-in and completion invalidates old consent.
    sqlx::query("UPDATE agent_definitions SET instruction_version=2 WHERE id=$1")
        .bind(agent)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE connector_setups SET state='pending',created_at=now(),completed_at=NULL WHERE id=$1")
        .bind(first.setup_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        setup
            .complete(&scope, &apps, &state, "used-code", None)
            .await
            .unwrap()
            .state,
        SetupState::NeedsReview
    );
    assert!(
        grants
            .effective_for_agent(&scope, "general")
            .await
            .unwrap()
            .is_empty()
    );
    // Withdrawal must be able to detach the setup while completion waits for
    // the package lock. The opposite order deadlocks through the session FK.
    sqlx::query("UPDATE connector_setups SET state='pending',completed_at=NULL WHERE id=$1")
        .bind(first.setup_id)
        .execute(&pool)
        .await
        .unwrap();
    let mut withdrawal = pool.begin().await.unwrap();
    sqlx::query(
        "UPDATE connector_packages SET enabled=false WHERE deployment_id=$1 AND external_key=$2",
    )
    .bind(deployment)
    .bind(&manifest.external_key)
    .execute(&mut *withdrawal)
    .await
    .unwrap();
    let callback = {
        let setup = setup.clone();
        let apps = apps.clone();
        let state = state.clone();
        tokio::spawn(async move {
            setup
                .complete(&scope, &apps, &state, "used-code", None)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%SELECT metadata FROM connector_packages%')")
                .fetch_one(&pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }).await.expect("callback reaches package lock");
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("DELETE FROM mcp_authorization_sessions WHERE id=$1")
            .bind(session)
            .execute(&mut *withdrawal),
    )
    .await
    .expect("withdrawal does not wait on a setup lock")
    .unwrap();
    withdrawal.commit().await.unwrap();
    assert_eq!(
        callback.await.unwrap().unwrap().state,
        SetupState::NeedsReview
    );
    // The normal withdrawal path revokes all account authority and keeps audit.
    registry
        .withdraw(deployment, &manifest.external_key, 1)
        .await
        .unwrap();
    let session: Option<Uuid> =
        sqlx::query_scalar("SELECT authorization_session_id FROM connector_setups WHERE id=$1")
            .bind(first.setup_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(session.is_none());
}
