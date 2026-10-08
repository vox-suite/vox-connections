use crate::{
    capability_grants::CapabilityGrantService,
    connected_apps::{ConnectedAppsOptions, ConnectedAppsService},
    connections::ConnectionService,
    integration_registry::IntegrationRegistry,
    packages::PackageRegistry,
    remote_extensions::RemoteExtensionService,
    service::auth::HmacVerifier,
    setup::ConnectorSetup,
    skills::SkillService,
};
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct ServiceState {
    pub pool: Option<PgPool>,
    pub connections: Option<Arc<ConnectionService>>,
    pub capability_grants: Option<Arc<CapabilityGrantService>>,
    pub integrations: Option<Arc<IntegrationRegistry>>,
    pub packages: Option<Arc<PackageRegistry>>,
    pub skills: Option<Arc<SkillService>>,
    pub extensions: Option<Arc<RemoteExtensionService>>,
    pub connected_apps: Option<Arc<ConnectedAppsService>>,
    pub setup: Option<Arc<ConnectorSetup>>,
    pub verifier: Arc<HmacVerifier>,
}

impl ServiceState {
    pub fn new(
        pool: PgPool,
        connected_apps_options: ConnectedAppsOptions,
        verifier: Arc<HmacVerifier>,
    ) -> Self {
        let connections = Arc::new(ConnectionService::new(pool.clone()));
        let capability_grants = Arc::new(CapabilityGrantService::new(pool.clone()));
        let integrations = Arc::new(IntegrationRegistry::new(pool.clone()));
        let packages = Arc::new(PackageRegistry::new(pool.clone()));
        let skills = Arc::new(SkillService::new(pool.clone()));
        let extensions = Arc::new(RemoteExtensionService::new(pool.clone()));
        let connected_apps = Arc::new(ConnectedAppsService::from_options(
            pool.clone(),
            connected_apps_options.clone(),
        ));
        let setup = Arc::new(ConnectorSetup::new(pool.clone()));

        Self {
            pool: Some(pool),
            connections: Some(connections),
            capability_grants: Some(capability_grants),
            integrations: Some(integrations),
            packages: Some(packages),
            skills: Some(skills),
            extensions: Some(extensions),
            connected_apps: Some(connected_apps),
            setup: Some(setup),
            verifier,
        }
    }

    pub fn for_test(verifier: Arc<HmacVerifier>) -> Self {
        Self {
            pool: None,
            connections: None,
            capability_grants: None,
            integrations: None,
            packages: None,
            skills: None,
            extensions: None,
            connected_apps: None,
            setup: None,
            verifier,
        }
    }
}
