//! Restored services must preserve grants, minimization and labelled handoffs.
use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use std::sync::{Arc, atomic::Ordering};
use uuid::Uuid;
use vox_connections::{
    capability_grants::{CapabilityGrantService, CreateGrantRequest},
    connections::ConnectionService,
    identity::{DeploymentId, RequestContext, RequestSubject, UserContextId, UserId},
    integration_registry::{
        IntegrationRegistry, RegisterIntegrationRequest, SetIntegrationEnabledRequest,
    },
    providers::{amazon::*, expedia::*, uber::*, zomato::*},
};

async fn connection(
    pool: &PgPool,
    context: &RequestContext,
    declaration: RegisterIntegrationRequest,
) -> Uuid {
    let provider = declaration.external_key.clone();
    let deployment = declaration.deployment_external_key.clone();
    let caps: Vec<String> = declaration
        .capabilities
        .iter()
        .map(|c| format!("{provider}.{}", c.external_key))
        .collect();
    let registry = IntegrationRegistry::new(pool.clone());
    registry.register(declaration).await.unwrap();
    registry
        .set_enabled(SetIntegrationEnabledRequest {
            deployment_external_key: deployment,
            external_key: provider.clone(),
            enabled: true,
        })
        .await
        .unwrap();
    sqlx::query_scalar("INSERT INTO external_connections(user_context_id,integration_id,external_account_hash,credential_custody,authorization_state,authorized_capabilities) SELECT $1,id,$2,'platform_held','authorized',$3 FROM integration_definitions WHERE deployment_id=$4 AND external_key=$5 RETURNING id")
        .bind(context.id.0).bind(vec![8u8;32]).bind(caps).bind(context.subject.deployment_id.0).bind(provider).fetch_one(pool).await.unwrap()
}
fn grant(connection_id: Uuid, cap: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        agent_external_key: "reader".into(),
        connection_id,
        capability_external_key: cap.into(),
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL with independent host schema"]
async fn restored_provider_services_require_grants_and_label_handoffs() {
    let pool = PgPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let key = format!("providers-{}", Uuid::new_v4());
    let dep: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_deployments(external_key) VALUES($1) RETURNING id",
    )
    .bind(&key)
    .fetch_one(&pool)
    .await
    .unwrap();
    let user: Uuid = sqlx::query_scalar("INSERT INTO users DEFAULT VALUES RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO user_contexts(deployment_id,user_id) VALUES($1,$2) RETURNING id",
    )
    .bind(dep)
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let context = RequestContext {
        id: UserContextId(ctx),
        user_id: UserId(user),
        subject: RequestSubject {
            deployment_id: DeploymentId(dep),
        },
    };
    let agent:Uuid=sqlx::query_scalar("INSERT INTO agent_definitions(deployment_id,owner_user_context_id,external_key,requested_capability_categories) VALUES($1,$2,'reader',ARRAY['*']) RETURNING id").bind(dep).bind(ctx).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO deployment_agent_selections VALUES($1,$2)")
        .bind(dep)
        .bind(agent)
        .execute(&pool)
        .await
        .unwrap();
    let connections = ConnectionService::new(pool.clone());
    let grants = CapabilityGrantService::new(pool.clone());
    let amazon_id = connection(
        &pool,
        &context,
        AmazonService::integration_declaration(&key),
    )
    .await;
    let amazon = AmazonService::new(
        pool.clone(),
        connections.clone(),
        grants.clone(),
        Arc::new(MockAmazonProviderClient::new(vec![])),
    );
    assert!(
        amazon
            .search_catalog(&context, "reader", amazon_id, "book", "IN")
            .await
            .is_err()
    );
    grants
        .grant(&context, grant(amazon_id, AMAZON_CAPABILITY_CATALOG_SEARCH))
        .await
        .unwrap();
    assert!(
        amazon
            .search_catalog(&context, "reader", amazon_id, "book", "IN")
            .await
            .unwrap()
            .items
            .is_empty()
    );
    grants
        .grant(
            &context,
            grant(amazon_id, AMAZON_CAPABILITY_PURCHASE_HANDOFF),
        )
        .await
        .unwrap();
    let handoff = amazon
        .create_purchase_handoff(
            &context,
            "reader",
            amazon_id,
            AmazonHandoffRequest {
                asin: "B001".into(),
                locale: "IN".into(),
                quantity: 1,
                partner_tag: None,
            },
        )
        .await
        .unwrap();
    assert!(!handoff.completed);
    assert_eq!(handoff.status, "handoff_created");
    assert!(
        amazon
            .execute_purchase(&context, "reader", amazon_id, "B001")
            .await
            .is_err()
    );
    grants
        .revoke(&context, grant(amazon_id, AMAZON_CAPABILITY_CATALOG_SEARCH))
        .await
        .unwrap();
    assert!(
        amazon
            .search_catalog(&context, "reader", amazon_id, "book", "IN")
            .await
            .is_err()
    );

    let zomato_id = connection(
        &pool,
        &context,
        ZomatoService::integration_declaration(&key),
    )
    .await;
    let zomato = ZomatoService::new(
        pool.clone(),
        connections.clone(),
        grants.clone(),
        Arc::new(MockZomatoProviderClient::new(vec![])),
    );
    assert!(
        zomato
            .search_restaurants(&context, "reader", zomato_id, "food", "city")
            .await
            .is_err()
    );
    grants
        .grant(
            &context,
            grant(zomato_id, ZOMATO_CAPABILITY_RESTAURANT_SEARCH),
        )
        .await
        .unwrap();
    assert!(
        zomato
            .search_restaurants(&context, "reader", zomato_id, "food", "city")
            .await
            .unwrap()
            .restaurants
            .is_empty()
    );
    grants
        .grant(&context, grant(zomato_id, ZOMATO_CAPABILITY_ORDER_HANDOFF))
        .await
        .unwrap();
    let handoff = zomato
        .create_order_handoff(
            &context,
            "reader",
            zomato_id,
            ZomatoHandoffRequest {
                res_id: Some("restaurant".into()),
                order_id: None,
                handoff_type: ZomatoHandoffType::CartAndCheckout,
            },
        )
        .await
        .unwrap();
    assert!(!handoff.completed);
    assert_eq!(handoff.action, "cart_and_checkout");
    assert!(
        zomato
            .execute_order(&context, "reader", zomato_id, json!([]))
            .await
            .is_err()
    );

    let uber_id = connection(
        &pool,
        &context,
        UberConnectedReadService::integration_declaration(&key),
    )
    .await;
    let client = Arc::new(MockUberProviderClient::new(vec![UberRawTrip {
        trip_id: "trip".into(),
        request_time: Utc::now(),
        status: "completed".into(),
        distance_miles: 2.0,
        start_city: Some("Private city".into()),
        pickup_latitude: Some(12.0),
        pickup_longitude: Some(13.0),
        dropoff_latitude: None,
        dropoff_longitude: None,
        payment_method_id: Some("payment-secret".into()),
        internal_rider_token: Some("rider-secret".into()),
    }]));
    let uber =
        UberConnectedReadService::new(pool.clone(), connections, grants.clone(), client.clone());
    assert!(
        uber.read_history(
            &context,
            "reader",
            uber_id,
            UBER_CAPABILITY_HISTORY_LITE,
            0,
            20
        )
        .await
        .is_err()
    );
    grants
        .grant(&context, grant(uber_id, UBER_CAPABILITY_HISTORY_LITE))
        .await
        .unwrap();
    let result = uber
        .read_history(
            &context,
            "reader",
            uber_id,
            UBER_CAPABILITY_HISTORY_LITE,
            0,
            20,
        )
        .await
        .unwrap();
    assert_eq!(result.trips.len(), 1);
    assert!(result.trips[0].city.is_none());
    let serialized = serde_json::to_string(&result).unwrap();
    for secret in [
        "payment-secret",
        "rider-secret",
        "Private city",
        "pickup_latitude",
    ] {
        assert!(!serialized.contains(secret));
    }
    client.fail_with_unauthorized.store(true, Ordering::SeqCst);
    assert!(matches!(
        uber.read_history(
            &context,
            "reader",
            uber_id,
            UBER_CAPABILITY_HISTORY_LITE,
            0,
            20
        )
        .await,
        Err(UberReadError::ReconnectRequired)
    ));
    assert!(
        uber.execute_ride_request(&context, "reader", uber_id, "fare")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn restored_expedia_transport_reconciles_idempotent_bookings_and_errors() {
    let client = MockExpediaProviderClient::new();
    let request = ExpediaRawBookingRequest {
        affiliate_reference_id: "request-1".into(),
        property_id: "property".into(),
        room_type_id: "room".into(),
        rate_plan_id: "rate".into(),
        checkin_date: "2026-11-01".into(),
        checkout_date: "2026-11-02".into(),
        primary_guest_name: "Fixture".into(),
        primary_guest_email: "fixture@example.invalid".into(),
        total_price_amount_minor: 10000,
        price_currency: "INR".into(),
    };
    let first = client.create_booking(&request).await.unwrap();
    let repeated = client.create_booking(&request).await.unwrap();
    assert_eq!(first.itinerary_id, repeated.itinerary_id);
    assert_eq!(
        client
            .retrieve_booking(&request.affiliate_reference_id)
            .await
            .unwrap()
            .unwrap()
            .itinerary_id,
        first.itinerary_id
    );
    client.fail_with_rate_limit.store(true, Ordering::SeqCst);
    assert!(matches!(
        client.create_booking(&request).await,
        Err(ExpediaLodgingError::RateLimited(_))
    ));
    client.fail_with_rate_limit.store(false, Ordering::SeqCst);
    client.fail_with_timeout.store(true, Ordering::SeqCst);
    assert!(matches!(
        client.create_booking(&request).await,
        Err(ExpediaLodgingError::Timeout)
    ));
}
