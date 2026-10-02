//! Deterministic 20-connection/40-skill selection and local concurrency baseline.
//! Uses a fresh independent-host database; no live providers or model calls.
use futures_util::future::join_all;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Instant;
use uuid::Uuid;
use vox_connections::{
    capability_grants::{CapabilityGrantService, CreateGrantRequest},
    connected_apps::{ConnectedAppError, ConnectedAppsOptions, ConnectedAppsService},
    connections::ConnectionService,
    discovery::CapabilityDiscovery,
    identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId},
    skills::{PublishSkillRequest, SkillService},
};
const CONTEXTS: usize = 20;
const CONNECTIONS: usize = 20;
const SKILLS: usize = 40;
const ROUNDS: usize = 25;
const CALENDAR: &str = "fixture.calendar.count";
const SCHEMA_CANARY: &str = "SCHEMA_ONLY_CANARY";
const BODY_CANARY: &str = "SKILL_BODY_ONLY_CANARY";

async fn seed(pool: &PgPool) -> Vec<(RequestContext, Uuid)> {
    let deployment_key = format!("metadata-load-{}", Uuid::new_v4());
    let deployment: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_deployments(external_key) VALUES($1) RETURNING id",
    )
    .bind(&deployment_key)
    .fetch_one(pool)
    .await
    .unwrap();
    let skills = SkillService::new(pool.clone());
    let mut skill_ids = Vec::new();
    for i in 0..SKILLS {
        skill_ids.push(
            skills
                .publish_curated(
                    &deployment_key,
                    PublishSkillRequest {
                        external_key: format!("fixture-writing-{i:02}"),
                        title: format!("Fixture writing guide {i:02}"),
                        summary: "Writing guidance unrelated to scheduling".into(),
                        instructions: format!(
                            "{BODY_CANARY}\n{}",
                            "Bounded irrelevant guidance. ".repeat(200)
                        ),
                        requested_capabilities: vec![],
                        resources: json!({"notes":"resource body canary"}),
                    },
                )
                .await
                .unwrap(),
        );
    }
    let mut contexts = Vec::new();
    for _ in 0..CONTEXTS {
        let user: Uuid = sqlx::query_scalar("INSERT INTO users DEFAULT VALUES RETURNING id")
            .fetch_one(pool)
            .await
            .unwrap();
        let context: Uuid = sqlx::query_scalar(
            "INSERT INTO user_contexts(user_id,deployment_id) VALUES($1,$2) RETURNING id",
        )
        .bind(user)
        .bind(deployment)
        .fetch_one(pool)
        .await
        .unwrap();
        let agent: Uuid = sqlx::query_scalar("INSERT INTO agent_definitions(deployment_id,owner_user_context_id,external_key,requested_capability_categories) VALUES($1,$2,'assistant',ARRAY['*']) RETURNING id")
            .bind(deployment).bind(context).fetch_one(pool).await.unwrap();
        sqlx::query("INSERT INTO deployment_agent_selections(deployment_id,agent_definition_id) VALUES($1,$2)")
            .bind(deployment).bind(agent).execute(pool).await.unwrap();
        let scope = RequestContext {
            id: UserContextId(context),
            user_id: UserId(user),
            subject: RequestSubject {
                deployment_id: DeploymentId(deployment),
            },
        };
        for skill in &skill_ids {
            skills
                .install_for_agent(&scope, *skill, 1, Some("assistant"))
                .await
                .unwrap();
        }
        let mut calendar = Uuid::nil();
        for i in 0..CONNECTIONS {
            let key = if i == 0 {
                CALENDAR.to_string()
            } else {
                format!("fixture.mail.read_{i:02}")
            };
            let schema = json!({"type":"object","properties":{"day":{"type":"string","description":SCHEMA_CANARY}},"additionalProperties":false});
            let capability = json!({"external_key":key,"display_name":if i==0 {"Fixture calendar count".to_string()} else {format!("Fixture mail read {i:02}")},"effect":"read","consequential":false,"input_schema":schema,"data_recipients":["Fixture"],"access_needs":[],"supported_regions":[],"optional_guarantees":{}});
            let extension:Uuid=sqlx::query_scalar("INSERT INTO remote_extensions(user_context_id,external_key,display_name,protocol,endpoint_url,operator_id,operator_name,conformance_status,operator_enabled,lifecycle_state,consent_status) VALUES($1,$2,$2,'mcp','https://provider.invalid/mcp','fixture','Fixture','passed',true,'active','consented') RETURNING id")
                .bind(context).bind(format!("fixture-{i:02}")).fetch_one(pool).await.unwrap();
            sqlx::query("INSERT INTO remote_extension_versions(extension_id,version,endpoint_url,operator_id,operator_name,capabilities,conformance_status) VALUES($1,1,'https://provider.invalid/mcp','fixture','Fixture',$2,'passed')")
                .bind(extension).bind(json!([capability])).execute(pool).await.unwrap();
            // Unexpired, no refresh token: schema selection cannot contact a
            // provider or decrypt this deliberately unusable credential.
            sqlx::query("INSERT INTO remote_extension_credentials(extension_id,issuer,token_endpoint,client_id,resource,access_token_ciphertext,tools,expires_at) VALUES($1,'https://provider.invalid','https://provider.invalid/token','fixture','https://provider.invalid/mcp',$2,$3,now()+interval '1 day')")
                .bind(extension).bind(vec![0u8]).bind(json!([{"name":key,"inputSchema":schema}])).execute(pool).await.unwrap();
            let connection:Uuid=sqlx::query_scalar("INSERT INTO external_connections(user_context_id,remote_extension_id,external_account_hash,credential_custody,authorization_state,authorized_capabilities) VALUES($1,$2,$3,'platform_held','authorized',ARRAY[$4]) RETURNING id")
                .bind(context).bind(extension).bind(vec![i as u8;32]).bind(&key).fetch_one(pool).await.unwrap();
            CapabilityGrantService::new(pool.clone())
                .grant(
                    &scope,
                    CreateGrantRequest {
                        agent_external_key: "assistant".into(),
                        connection_id: connection,
                        capability_external_key: key,
                    },
                )
                .await
                .unwrap();
            if i == 0 {
                calendar = connection;
            }
        }
        contexts.push((scope, calendar));
    }
    // A real selected owned agent with the same declared categories has no
    // inherited connection grants or skill enablements from the assistant.
    let denied_scope = contexts[0].0;
    let denied_agent: Uuid = sqlx::query_scalar("INSERT INTO agent_definitions(deployment_id,owner_user_context_id,external_key,requested_capability_categories) VALUES($1,$2,'ungranted-agent',ARRAY['*']) RETURNING id")
        .bind(deployment).bind(denied_scope.id.0).fetch_one(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO deployment_agent_selections(deployment_id,agent_definition_id) VALUES($1,$2)",
    )
    .bind(deployment)
    .bind(denied_agent)
    .execute(pool)
    .await
    .unwrap();
    contexts
}
fn metadata_only(response: &Value) {
    let serialized = response.to_string();
    assert!(serialized.len() <= 32 * 1024);
    for forbidden in [
        SCHEMA_CANARY,
        BODY_CANARY,
        "input_schema",
        "inputSchema",
        "instructions",
        "resources",
        "access_token",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "metadata leaked {forbidden}"
        );
    }
    assert!(response["results"].as_array().unwrap().len() <= 10);
}
fn stats(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    json!({"samples":samples.len(),"p50_ms":samples[(samples.len()-1)/2],"p95_ms":samples[(samples.len()*95).div_ceil(100)-1],"max_ms":samples.last().unwrap()})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a fresh independent-host PostgreSQL database"]
async fn metadata_selection_is_bounded_isolated_revocable_and_measured() {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&std::env::var("TEST_DATABASE_URL").expect("fresh independent database"))
        .await
        .unwrap();
    let contexts = seed(&pool).await;
    let discovery = CapabilityDiscovery::new(pool.clone());
    let apps = ConnectedAppsService::from_options(
        pool.clone(),
        ConnectedAppsOptions {
            credential_key: None,
            redirect_uris: vec![],
            oauth_clients: None,
        },
    );
    let skills = SkillService::new(pool.clone());
    for (scope, calendar) in &contexts {
        assert_eq!(
            ConnectionService::new(pool.clone())
                .list(scope)
                .await
                .unwrap()
                .len(),
            CONNECTIONS
        );
        assert_eq!(
            skills.effective(scope, "assistant").await.unwrap().len(),
            SKILLS
        );
        let result = discovery
            .search(scope, "assistant", "calendar", 0)
            .await
            .unwrap();
        metadata_only(&result);
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        assert_eq!(result["results"][0]["name"], CALENDAR);
        assert_eq!(result["results"][0]["connection_id"], calendar.to_string());
        assert!(result["next_offset"].is_null());
        let tool = apps
            .tool_for_agent(scope, "assistant", *calendar, CALENDAR)
            .await
            .unwrap();
        assert!(tool.to_string().contains(SCHEMA_CANARY));
        assert!(!tool.to_string().contains(BODY_CANARY));
    }
    // Inventory is paged, stable and complete across both metadata kinds.
    let scope = contexts[0].0;
    let mut offset = 0;
    let mut identities = std::collections::HashSet::new();
    let mut pages = 0;
    loop {
        let page = discovery
            .search(&scope, "assistant", "fixture", offset)
            .await
            .unwrap();
        metadata_only(&page);
        for item in page["results"].as_array().unwrap() {
            assert!(identities.insert((item["kind"].to_string(), item["name"].to_string())));
        }
        pages += 1;
        match page["next_offset"].as_u64() {
            Some(next) => offset = next as usize,
            None => break,
        }
    }
    assert_eq!(pages, 6);
    assert_eq!(identities.len(), CONNECTIONS + SKILLS);
    assert!(
        apps.tool_for_agent(&contexts[1].0, "assistant", contexts[0].1, CALENDAR)
            .await
            .is_err()
    );
    for query in ["calendar", "fixture"] {
        let denied = discovery
            .search(&scope, "ungranted-agent", query, 0)
            .await
            .unwrap();
        metadata_only(&denied);
        assert!(denied["results"].as_array().unwrap().is_empty());
        assert!(denied["next_offset"].is_null());
    }
    assert!(matches!(
        apps.tool_for_agent(&scope, "ungranted-agent", contexts[0].1, CALENDAR)
            .await,
        Err(ConnectedAppError::GrantRequired)
    ));
    assert!(matches!(
        apps.read_tool(
            &scope,
            "ungranted-agent",
            contexts[0].1,
            CALENDAR,
            json!({})
        )
        .await,
        Err(ConnectedAppError::GrantRequired)
    ));
    assert!(
        skills
            .effective(&scope, "ungranted-agent")
            .await
            .unwrap()
            .is_empty()
    );
    let assistant_skill = skills.effective(&scope, "assistant").await.unwrap()[0].id;
    assert!(matches!(
        skills
            .load_for_agent(&scope, "ungranted-agent", assistant_skill)
            .await,
        Err(vox_connections::skills::SkillError::NotFound)
    ));
    // Warmup above is excluded; each context independently performs 25 search
    // and selected-schema operations. No latency assertion is a product SLA.
    let started = Instant::now();
    let results = join_all(contexts.iter().map(|(scope, calendar)| {
        let discovery = discovery.clone();
        let apps = apps.clone();
        async move {
            let mut search_times = Vec::new();
            let mut load_times = Vec::new();
            let mut bytes = Vec::new();
            for _ in 0..ROUNDS {
                let started = Instant::now();
                let response = discovery
                    .search(scope, "assistant", "calendar", 0)
                    .await
                    .unwrap();
                search_times.push(started.elapsed().as_secs_f64() * 1000.0);
                metadata_only(&response);
                assert_eq!(
                    response["results"][0]["connection_id"],
                    calendar.to_string()
                );
                bytes.push(response.to_string().len());
                let started = Instant::now();
                let loaded = apps
                    .tool_for_agent(scope, "assistant", *calendar, CALENDAR)
                    .await
                    .unwrap();
                load_times.push(started.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(loaded["connection_id"], calendar.to_string());
            }
            (search_times, load_times, bytes)
        }
    }))
    .await;
    let elapsed = started.elapsed().as_secs_f64();
    let mut search_times = Vec::new();
    let mut load_times = Vec::new();
    let mut response_bytes = Vec::new();
    for (search, load, bytes) in results {
        search_times.extend(search);
        load_times.extend(load);
        response_bytes.extend(bytes);
    }
    // Revocation is checked through the same APIs after discovery, not a cached
    // permission assumption. Another context's selected capability survives.
    CapabilityGrantService::new(pool.clone())
        .revoke(
            &scope,
            CreateGrantRequest {
                agent_external_key: "assistant".into(),
                connection_id: contexts[0].1,
                capability_external_key: CALENDAR.into(),
            },
        )
        .await
        .unwrap();
    assert!(
        discovery
            .search(&scope, "assistant", "calendar", 0)
            .await
            .unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        apps.tool_for_agent(&scope, "assistant", contexts[0].1, CALENDAR)
            .await
            .is_err()
    );
    assert!(
        apps.tool_for_agent(&contexts[1].0, "assistant", contexts[1].1, CALENDAR)
            .await
            .is_ok()
    );
    assert!(matches!(
        apps.read_tool(&scope, "assistant", contexts[0].1, CALENDAR, json!({}))
            .await,
        Err(ConnectedAppError::GrantRequired)
    ));
    assert!(matches!(
        apps.read_tool(
            &contexts[1].0,
            "assistant",
            contexts[0].1,
            CALENDAR,
            json!({})
        )
        .await,
        Err(ConnectedAppError::GrantRequired)
    ));
    let skill = skills.effective(&scope, "assistant").await.unwrap()[0].id;
    skills.disable(&scope, skill).await.unwrap();
    assert!(
        skills
            .load_for_agent(&scope, "assistant", skill)
            .await
            .is_err()
    );
    assert!(matches!(
        skills
            .load_for_agent(&scope, "ungranted-agent", skill)
            .await,
        Err(vox_connections::skills::SkillError::NotFound)
    ));
    assert!(
        skills
            .load_for_agent(&contexts[1].0, "assistant", skill)
            .await
            .is_ok()
    );
    let version: String = sqlx::query_scalar("SHOW server_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    let report = json!({"measured_at_utc":chrono::Utc::now().to_rfc3339(),"environment":std::env::var("VOX_LOAD_ENVIRONMENT").ok(),"implementation_base":std::env::var("VOX_LOAD_REVISION").ok(),"debug_build":cfg!(debug_assertions),"fixture":{"contexts":CONTEXTS,"connections_per_context":CONNECTIONS,"skills_per_context":SKILLS,"rounds_per_context":ROUNDS,"concurrent_contexts":CONTEXTS,"pool_connections":20,"runtime_threads":4,"provider_calls":0,"schema_loads_per_selection":1,"inventory_pages":pages},"postgres_version":version,"elapsed_seconds":elapsed,"operations_per_second":(CONTEXTS*ROUNDS*2) as f64/elapsed,"search":stats(search_times),"selected_schema_load":stats(load_times),"search_response_bytes":{"min":response_bytes.iter().min(),"max":response_bytes.iter().max()},"claim":"Local warm metadata baseline; excludes model/provider/network/refresh latency and is not a production SLA"});
    println!("METADATA_LOAD_REPORT={report}");
    if let Ok(path) = std::env::var("VOX_LOAD_REPORT") {
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
}
