use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use vox_connections::service::{HmacVerifier, ServiceConfig, ServiceState, build_service_router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,vox_connections=debug".into()),
        )
        .init();

    info!("Starting Vox Connections Service...");

    let config = ServiceConfig::from_env()?;
    info!("Binding on {}", config.bind_address);

    let verifier = Arc::new(HmacVerifier::new(
        config.hmac_secret.as_bytes(),
        config.max_clock_skew_seconds,
    ));

    let state = if let Some(db_url) = config.database_url.as_deref() {
        info!("Connecting to PostgreSQL database...");
        let pool = sqlx::PgPool::connect(db_url).await?;
        info!("Connected to database successfully.");
        ServiceState::new(pool, config.connected_apps, verifier)
    } else {
        tracing::warn!("DATABASE_URL not set; running in memory/test mode without persistence.");
        ServiceState::for_test(verifier)
    };

    let router = build_service_router(state);

    let listener = TcpListener::bind(&config.bind_address).await?;
    info!(
        "Vox Connections Service listening on {}",
        config.bind_address
    );

    axum::serve(listener, router).await?;

    Ok(())
}
