use axum::{
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    capability_grants::{CapabilityGrantError, CreateGrantRequest},
    connections::ConnectionError,
    identity::RequestContext,
    integration_registry::{RegisterIntegrationRequest, SetIntegrationEnabledRequest},
    packages::{PackageError, PublishPackage},
    remote_extensions::{InstallExtensionRequest, RemoteExtensionError, UpdateExtensionRequest},
    service::{
        auth::hmac_auth_middleware,
        state::ServiceState,
    },
    setup::{SetupError, SetupRequest},
    skills::{PublishSkillRequest, SkillError},
};

// Common request wrappers with caller context
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextRequest {
    pub context: RequestContext,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextDiscoveryRequest {
    pub context: RequestContext,
    pub agent_external_key: Option<String>,
    pub region: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateGrantBody {
    pub context: RequestContext,
    #[serde(flatten)]
    pub grant: CreateGrantRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PackageInstallBody {
    pub context: RequestContext,
    pub external_key: String,
    pub version: i32,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WithdrawPackageBody {
    pub deployment_id: Uuid,
    pub external_key: String,
    pub version: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetupStartBody {
    pub context: RequestContext,
    #[serde(flatten)]
    pub request: SetupRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetupCallbackBody {
    pub context: RequestContext,
    pub state: String,
    pub code: String,
    pub iss: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstallExtensionBody {
    pub context: RequestContext,
    #[serde(flatten)]
    pub request: InstallExtensionRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateExtensionBody {
    pub context: RequestContext,
    #[serde(flatten)]
    pub request: UpdateExtensionRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetExtensionEnabledBody {
    pub context: RequestContext,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenewConsentBody {
    pub context: RequestContext,
    pub expected_version: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QuarantineBody {
    pub context: RequestContext,
    pub version: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConformanceBody {
    pub context: RequestContext,
    pub version: i32,
    pub passed: bool,
    pub report: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthorizeAppBody {
    pub context: RequestContext,
    pub host_callback_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppCallbackBody {
    pub context: RequestContext,
    pub state: String,
    pub code: String,
    pub iss: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ToolCallBody {
    pub context: RequestContext,
    pub agent_key: String,
    pub extension_id: Uuid,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublishSkillBody {
    pub context: RequestContext,
    #[serde(flatten)]
    pub request: PublishSkillRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublishCuratedSkillBody {
    pub deployment_external_key: String,
    #[serde(flatten)]
    pub request: PublishSkillRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InstallSkillBody {
    pub context: RequestContext,
    pub version: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SetSkillEnabledBody {
    pub context: RequestContext,
    pub enabled: bool,
}

// Helper to format responses
fn reply_json<T: Serialize>(res: Result<T, impl std::fmt::Display>) -> Response {
    match res {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ----------------- Handlers -----------------

// Health
async fn live() -> Response {
    (StatusCode::OK, Json(json!({ "status": "live" }))).into_response()
}

async fn ready(State(state): State<ServiceState>) -> Response {
    if let Some(pool) = state.pool.as_ref() {
        if sqlx::query("SELECT 1").execute(pool).await.is_ok() {
            return (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response();
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "unavailable", "message": "database ping failed" })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response()
}

// Connections
async fn list_connections(
    State(state): State<ServiceState>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.connections.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.list(&r.context).await {
        Ok(records) => (StatusCode::OK, Json(records)).into_response(),
        Err(ConnectionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn disconnect_connection(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.connections.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.disconnect(&r.context, id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(ConnectionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(ConnectionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// Capability Grants
async fn create_grant(
    State(state): State<ServiceState>,
    Json(r): Json<CreateGrantBody>,
) -> Response {
    let Some(svc) = state.capability_grants.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.grant(&r.context, r.grant).await {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(CapabilityGrantError::Invalid) => StatusCode::BAD_REQUEST.into_response(),
        Err(CapabilityGrantError::Unavailable) => StatusCode::FORBIDDEN.into_response(),
        Err(CapabilityGrantError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn revoke_grant(
    State(state): State<ServiceState>,
    Json(r): Json<CreateGrantBody>,
) -> Response {
    let Some(svc) = state.capability_grants.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.revoke(&r.context, r.grant).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(CapabilityGrantError::Invalid) => StatusCode::BAD_REQUEST.into_response(),
        Err(CapabilityGrantError::Unavailable) => StatusCode::NOT_FOUND.into_response(),
        Err(CapabilityGrantError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn effective_grants(
    State(state): State<ServiceState>,
    Path(agent_key): Path<String>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.capability_grants.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.effective_for_agent(&r.context, &agent_key).await {
        Ok(grants) => (StatusCode::OK, Json(grants)).into_response(),
        Err(CapabilityGrantError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// Integrations
async fn register_integration(
    State(state): State<ServiceState>,
    Json(decl): Json<RegisterIntegrationRequest>,
) -> Response {
    let Some(svc) = state.integrations.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    reply_json(svc.register(decl).await)
}

async fn set_integration_enabled(
    State(state): State<ServiceState>,
    Json(req): Json<SetIntegrationEnabledRequest>,
) -> Response {
    let Some(svc) = state.integrations.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    reply_json(svc.set_enabled(req).await)
}

async fn discover_deployment_capabilities(
    State(state): State<ServiceState>,
    Path(external_key): Path<String>,
) -> Response {
    let Some(svc) = state.integrations.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    reply_json(svc.discover(&external_key).await)
}

async fn list_integration_versions(
    State(state): State<ServiceState>,
    Path((external_key, integration_key)): Path<(String, String)>,
) -> Response {
    let Some(svc) = state.integrations.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    reply_json(svc.versions(&external_key, &integration_key).await)
}

async fn discover_context_capabilities(
    State(state): State<ServiceState>,
    Json(r): Json<ContextDiscoveryRequest>,
) -> Response {
    let Some(svc) = state.integrations.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    reply_json(
        svc.discover_for_context(
            &r.context,
            r.agent_external_key.as_deref(),
            r.region.as_deref(),
        )
        .await,
    )
}

// Packages
async fn publish_package(
    State(state): State<ServiceState>,
    Json(req): Json<PublishPackage>,
) -> Response {
    let Some(svc) = state.packages.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.publish(req).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(PackageError::Conflict) => StatusCode::CONFLICT.into_response(),
        Err(PackageError::Unavailable) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn list_packages(
    State(state): State<ServiceState>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.packages.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.list(&r.context).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn install_package(
    State(state): State<ServiceState>,
    Json(r): Json<PackageInstallBody>,
) -> Response {
    let Some(svc) = state.packages.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.install(&r.context, &r.external_key, r.version, &r.digest).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(PackageError::Conflict) => StatusCode::CONFLICT.into_response(),
        Err(PackageError::Unavailable) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn withdraw_package(
    State(state): State<ServiceState>,
    Json(r): Json<WithdrawPackageBody>,
) -> Response {
    let Some(svc) = state.packages.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.withdraw(r.deployment_id, &r.external_key, r.version).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(PackageError::Unavailable) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// Setup
async fn setup_start(
    State(state): State<ServiceState>,
    Json(r): Json<SetupStartBody>,
) -> Response {
    let (Some(svc), Some(apps)) = (state.setup.as_ref(), state.connected_apps.as_ref()) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.start(&r.context, apps, r.request).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(SetupError::Unavailable) => StatusCode::NOT_FOUND.into_response(),
        Err(SetupError::ReviewRequired) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn setup_callback(
    State(state): State<ServiceState>,
    Json(r): Json<SetupCallbackBody>,
) -> Response {
    let (Some(svc), Some(apps)) = (state.setup.as_ref(), state.connected_apps.as_ref()) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.complete(&r.context, apps, &r.state, &r.code, r.iss.as_deref()).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(SetupError::Unavailable) => StatusCode::NOT_FOUND.into_response(),
        Err(SetupError::ReviewRequired) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// Remote Extensions
async fn install_extension(
    State(state): State<ServiceState>,
    Json(r): Json<InstallExtensionBody>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.install(&r.context, r.request).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn list_extensions(
    State(state): State<ServiceState>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.list(&r.context).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn get_extension(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.get(&r.context, id).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn update_extension(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<UpdateExtensionBody>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.update(&r.context, id, r.request).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn remove_extension(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.remove(&r.context, id).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn set_extension_enabled(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<SetExtensionEnabledBody>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.set_operator_enabled(&r.context, id, r.enabled).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn record_extension_conformance(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ConformanceBody>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.record_conformance(&r.context, id, r.version, r.passed, r.report).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn renew_extension_consent(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<RenewConsentBody>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.renew_consent(&r.context, id, r.expected_version).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn quarantine_extension(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<QuarantineBody>,
) -> Response {
    let Some(svc) = state.extensions.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.quarantine(&r.context, id, r.version).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(RemoteExtensionError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RemoteExtensionError::Database(_)) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// Connected Apps
async fn authorize_app(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<AuthorizeAppBody>,
) -> Response {
    let Some(svc) = state.connected_apps.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.begin(&r.context, id, &r.host_callback_url).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn app_callback(
    State(state): State<ServiceState>,
    Json(r): Json<AppCallbackBody>,
) -> Response {
    let Some(svc) = state.connected_apps.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.complete_with_issuer(&r.context, &r.state, &r.code, r.iss.as_deref()).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn app_status(
    State(state): State<ServiceState>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.connected_apps.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.connections(&r.context).await {
        Ok(connected) => (
            StatusCode::OK,
            Json(json!({
                "configured_hosts": svc.configured_hosts(),
                "connected": connected,
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn connect_public_app(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.connected_apps.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.connect_public(&r.context, id).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn read_tool(
    State(state): State<ServiceState>,
    Json(r): Json<ToolCallBody>,
) -> Response {
    let Some(svc) = state.connected_apps.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.read_tool(
        &r.context,
        &r.agent_key,
        r.extension_id,
        &r.tool_name,
        r.arguments,
    ).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn execute_tool(
    State(state): State<ServiceState>,
    Json(r): Json<ToolCallBody>,
) -> Response {
    let Some(svc) = state.connected_apps.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.approved_tool(
        &r.context,
        &r.agent_key,
        r.extension_id,
        &r.tool_name,
        r.arguments,
    ).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// Skills
async fn publish_private_skill(
    State(state): State<ServiceState>,
    Json(r): Json<PublishSkillBody>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.publish_private(&r.context, r.request).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(SkillError::Conflict) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn publish_curated_skill(
    State(state): State<ServiceState>,
    Json(r): Json<PublishCuratedSkillBody>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.publish_curated(&r.deployment_external_key, r.request).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(SkillError::Conflict) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn list_skills(
    State(state): State<ServiceState>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.list(&r.context).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn get_skill_version(
    State(state): State<ServiceState>,
    Path((id, version)): Path<(Uuid, i32)>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.version(&r.context, id, version).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(SkillError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn install_skill(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<InstallSkillBody>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.install(&r.context, id, r.version).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(SkillError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(SkillError::Conflict) => StatusCode::CONFLICT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn disable_skill(
    State(state): State<ServiceState>,
    Path(id): Path<Uuid>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.disable(&r.context, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(SkillError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn effective_skills(
    State(state): State<ServiceState>,
    Path(agent_key): Path<String>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.effective(&r.context, &agent_key).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn load_skill_for_agent(
    State(state): State<ServiceState>,
    Path((agent_key, skill_id)): Path<(String, Uuid)>,
    Json(r): Json<ContextRequest>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.load_for_agent(&r.context, &agent_key, skill_id).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(SkillError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn set_skill_agent_enabled(
    State(state): State<ServiceState>,
    Path((agent_key, skill_id)): Path<(String, Uuid)>,
    Json(r): Json<SetSkillEnabledBody>,
) -> Response {
    let Some(svc) = state.skills.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match svc.set_agent_enabled(&r.context, &agent_key, skill_id, r.enabled).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(SkillError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// ----------------- Router Assembly -----------------

pub fn build_service_router(state: ServiceState) -> Router {
    let verifier = state.verifier.clone();

    // Unauthenticated health endpoints
    let health_routes = Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready));

    // Protected API routes requiring HMAC authentication
    let api_routes = Router::new()
        // Connections
        .route("/v1/connections/list", post(list_connections))
        .route("/v1/connections/{id}/disconnect", post(disconnect_connection))
        // Capability Grants
        .route("/v1/capability-grants", post(create_grant))
        .route("/v1/capability-grants/revoke", post(revoke_grant))
        .route(
            "/v1/agents/{external_key}/effective-capability-grants",
            post(effective_grants),
        )
        // Integrations
        .route("/v1/integrations", post(register_integration))
        .route("/v1/integrations/enabled", post(set_integration_enabled))
        .route(
            "/v1/deployments/{external_key}/capabilities",
            post(discover_deployment_capabilities),
        )
        .route(
            "/v1/deployments/{external_key}/integrations/{integration_key}/versions",
            post(list_integration_versions),
        )
        .route("/v1/capabilities/discover", post(discover_context_capabilities))
        // Connector Packages
        .route("/v1/connector-packages/publish", post(publish_package))
        .route("/v1/connector-packages/list", post(list_packages))
        .route("/v1/connector-packages/install", post(install_package))
        .route("/v1/connector-packages/withdraw", post(withdraw_package))
        .route("/v1/connector-packages/setup", post(setup_start))
        .route("/v1/connector-packages/setup/callback", post(setup_callback))
        // Remote Extensions
        .route("/v1/remote-extensions", post(install_extension))
        .route("/v1/remote-extensions/list", post(list_extensions))
        .route("/v1/remote-extensions/{id}", post(get_extension))
        .route("/v1/remote-extensions/{id}/update", post(update_extension))
        .route("/v1/remote-extensions/{id}/remove", post(remove_extension))
        .route("/v1/remote-extensions/{id}/enable", post(set_extension_enabled))
        .route("/v1/remote-extensions/{id}/conformance", post(record_extension_conformance))
        .route("/v1/remote-extensions/{id}/renew-consent", post(renew_extension_consent))
        .route("/v1/remote-extensions/{id}/quarantine", post(quarantine_extension))
        // Connected Apps
        .route("/v1/remote-extensions/{id}/authorize", post(authorize_app))
        .route("/v1/connected-apps/callback", post(app_callback))
        .route("/v1/connected-apps/status", post(app_status))
        .route("/v1/remote-extensions/{id}/connect-public", post(connect_public_app))
        .route("/v1/connected-apps/read", post(read_tool))
        .route("/v1/connected-apps/execute", post(execute_tool))
        // Skills
        .route("/v1/skills/private", post(publish_private_skill))
        .route("/v1/skills/curated", post(publish_curated_skill))
        .route("/v1/skills/list", post(list_skills))
        .route("/v1/skills/{id}/versions/{version}", post(get_skill_version))
        .route("/v1/skills/{id}/install", post(install_skill))
        .route("/v1/skills/{id}/disable", post(disable_skill))
        .route("/v1/agents/{agent_key}/effective-skills", post(effective_skills))
        .route("/v1/agents/{agent_key}/skills/{skill_id}/load", post(load_skill_for_agent))
        .route("/v1/agents/{agent_key}/skills/{skill_id}/enable", post(set_skill_agent_enabled))
        // Enforce HMAC authentication on all API routes
        .layer(middleware::from_fn_with_state(
            verifier,
            hmac_auth_middleware,
        ));

    Router::new()
        .merge(health_routes)
        .merge(api_routes)
        .with_state(state)
}
