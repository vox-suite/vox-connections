use crate::connected_apps::ConnectedAppsOptions;
use std::env;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required configuration: {0}")]
    Missing(String),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Clone, Debug)]
pub struct ServiceConfig {
    pub bind_address: String,
    pub database_url: Option<String>,
    pub hmac_secret: String,
    pub max_clock_skew_seconds: i64,
    pub connected_apps: ConnectedAppsOptions,
}

impl ServiceConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind_address =
            env::var("VOX_CONNECTIONS_BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3003".to_string());

        let database_url = env::var("DATABASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let hmac_secret = env::var("VOX_CONNECTIONS_HMAC_SECRET")
            .or_else(|_| env::var("VOX_HMAC_SECRET"))
            .unwrap_or_else(|_| {
                // In development / testing, fallback if not explicitly provided
                let dev_secret = "vox-connections-dev-secret-change-in-production";
                tracing::warn!(
                    "VOX_CONNECTIONS_HMAC_SECRET not set; using default development HMAC secret."
                );
                dev_secret.to_string()
            });

        let max_clock_skew_seconds = env::var("VOX_CONNECTIONS_MAX_CLOCK_SKEW_SECS")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(300);

        let credential_key = env::var("VOX_CREDENTIAL_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let redirect_uris = env::var("VOX_MCP_OAUTH_REDIRECT_URIS")
            .map(|uris| uris.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();
        let oauth_clients = env::var("VOX_MCP_OAUTH_CLIENTS")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let connected_apps = ConnectedAppsOptions {
            credential_key,
            redirect_uris,
            oauth_clients,
        };

        Ok(Self {
            bind_address,
            database_url,
            hmac_secret,
            max_clock_skew_seconds,
            connected_apps,
        })
    }

    pub fn for_test(hmac_secret: impl Into<String>) -> Self {
        Self {
            bind_address: "127.0.0.1:0".to_string(),
            database_url: None,
            hmac_secret: hmac_secret.into(),
            max_clock_skew_seconds: 300,
            connected_apps: ConnectedAppsOptions::default(),
        }
    }
}
