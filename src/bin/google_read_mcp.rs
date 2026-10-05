//! Stateless Google REST fallback. Terminate public HTTPS at the deployment proxy.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address =
        std::env::var("VOX_GOOGLE_READ_BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:3004".into());
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(
        listener,
        vox_connections::google_reads::GoogleReadAdapter::new().router(),
    )
    .await?;
    Ok(())
}
