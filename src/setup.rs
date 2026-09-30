//! One setup journey: reviewed installation, verified account, explicit agent consent.
//! Provider calls happen outside transactions; authority commits atomically afterwards.
use crate::{
    connected_apps::{ConnectedAppError, ConnectedAppsService},
    identity::RequestScope,
    packages::{PackageAuthMode, PackageError, PackageMetadata, PackageRegistry},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupConsent {
    pub agent_external_key: String,
    pub agent_instruction_version: i32,
    pub capability_external_keys: Vec<String>,
    pub enable_bundled_skills: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    pub external_key: String,
    pub version: i32,
    pub digest: String,
    pub redirect_uri: String,
    pub consent: Option<SetupConsent>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    Authorize,
    Complete,
    NeedsReview,
    AccountLinked,
}

#[derive(Debug, Serialize)]
pub struct SetupResult {
    pub setup_id: Option<Uuid>,
    pub extension_id: Uuid,
    pub external_key: String,
    pub state: SetupState,
    pub authorization_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("invalid setup consent")]
    Invalid,
    #[error("setup is unavailable or expired")]
    Unavailable,
    #[error("setup requires fresh consent")]
    ReviewRequired,
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error(transparent)]
    Account(#[from] ConnectedAppError),
    #[error("setup storage unavailable")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct ConnectorSetup {
    db: PgPool,
}

impl ConnectorSetup {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn start(
        &self,
        scope: &impl RequestScope,
        apps: &ConnectedAppsService,
        request: SetupRequest,
    ) -> Result<SetupResult, SetupError> {
        if let Some(consent) = &request.consent {
            validate_consent(consent)?;
            let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_definitions a JOIN deployment_agent_selections s ON s.agent_definition_id=a.id AND s.deployment_id=a.deployment_id WHERE a.owner_user_context_id=$1 AND a.deployment_id=$2 AND a.external_key=$3 AND a.state='enabled' AND a.instruction_version=$4)")
                .bind(scope.request_context().id.0).bind(scope.request_context().subject.deployment_id.0)
                .bind(&consent.agent_external_key).bind(consent.agent_instruction_version).fetch_one(&self.db).await?;
            if !valid {
                return Err(SetupError::ReviewRequired);
            }
        }
        let extension = PackageRegistry::new(self.db.clone())
            .install(
                scope,
                &request.external_key,
                request.version,
                &request.digest,
            )
            .await?;
        if request.consent.as_ref().is_some_and(|consent| {
            consent.capability_external_keys.iter().any(|key| {
                !extension
                    .capabilities
                    .iter()
                    .any(|cap| &cap.external_key == key)
            })
        }) {
            return Err(SetupError::Invalid);
        }
        let metadata: serde_json::Value = sqlx::query_scalar("SELECT metadata FROM connector_packages WHERE deployment_id=$1 AND external_key=$2 AND version=$3 AND enabled AND digest=$4")
            .bind(scope.request_context().subject.deployment_id.0).bind(&request.external_key)
            .bind(request.version).bind(&request.digest).fetch_optional(&self.db).await?.ok_or(SetupError::ReviewRequired)?;
        let metadata: PackageMetadata =
            serde_json::from_value(metadata).map_err(|_| SetupError::Invalid)?;
        let consent = request
            .consent
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| SetupError::Invalid)?;
        let id: Uuid = sqlx::query_scalar("INSERT INTO connector_setups(user_context_id,extension_id,extension_version,deployment_id,package_key,package_version,package_digest,consent) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id")
            .bind(scope.request_context().id.0).bind(extension.id).bind(extension.current_version)
            .bind(scope.request_context().subject.deployment_id.0).bind(&request.external_key)
            .bind(request.version).bind(&request.digest).bind(consent).fetch_one(&self.db).await?;
        let linked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM external_connections x JOIN remote_extension_credentials c ON c.extension_id=x.remote_extension_id WHERE x.user_context_id=$1 AND x.remote_extension_id=$2 AND x.authorization_state='authorized' AND (x.expires_at IS NULL OR x.expires_at>now()) AND (c.expires_at IS NULL OR c.expires_at>now()))")
            .bind(scope.request_context().id.0).bind(extension.id).fetch_one(&self.db).await?;
        if linked {
            return self.finish(scope, id).await;
        }
        if metadata.auth_mode == PackageAuthMode::None {
            apps.connect_public(scope, extension.id).await?;
            return self.finish(scope, id).await;
        }
        let start = apps
            .begin(scope, extension.id, &request.redirect_uri)
            .await?;
        let changed = sqlx::query("UPDATE connector_setups SET authorization_session_id=$1 WHERE id=$2 AND user_context_id=$3 AND state='pending' AND expires_at>now()")
            .bind(start.authorization_session_id).bind(id).bind(scope.request_context().id.0).execute(&self.db).await?.rows_affected();
        if changed != 1 {
            return Err(SetupError::Unavailable);
        }
        Ok(SetupResult {
            setup_id: Some(id),
            extension_id: extension.id,
            external_key: extension.external_key,
            state: SetupState::Authorize,
            authorization_url: Some(start.authorization_url),
        })
    }

    /// Retrying a completed callback recovers the outcome, never repeats grants.
    pub async fn complete(
        &self,
        scope: &impl RequestScope,
        apps: &ConnectedAppsService,
        state: &str,
        code: &str,
        issuer: Option<&str>,
    ) -> Result<SetupResult, SetupError> {
        if state.is_empty() || state.len() > 1024 || code.is_empty() || code.len() > 8192 {
            return Err(SetupError::Invalid);
        }
        let hash = hex::encode(Sha256::digest(state.as_bytes()));
        let row = sqlx::query("SELECT i.id,s.completed_at,s.issuer FROM connector_setups i JOIN mcp_authorization_sessions s ON s.id=i.authorization_session_id WHERE s.state_hash=$1 AND i.user_context_id=$2 AND s.user_context_id=$2 AND i.extension_id=s.extension_id AND i.deployment_id=$3")
            .bind(hash).bind(scope.request_context().id.0).bind(scope.request_context().subject.deployment_id.0)
            .fetch_optional(&self.db).await?;
        let Some(row) = row else {
            // Individually added MCP servers use the same callback but have no
            // package consent. Link the account only; never infer agent grants.
            let extension = apps
                .complete_with_issuer(scope, state, code, issuer)
                .await?;
            return Ok(SetupResult {
                setup_id: None,
                extension_id: extension.id,
                external_key: extension.external_key,
                state: SetupState::AccountLinked,
                authorization_url: None,
            });
        };
        if issuer.is_some_and(|issuer| issuer != row.get::<String, _>("issuer")) {
            return Err(SetupError::Unavailable);
        }
        let completed: Option<chrono::DateTime<chrono::Utc>> = row.get("completed_at");
        if completed.is_none() {
            apps.complete_with_issuer(scope, state, code, issuer)
                .await?;
        }
        self.finish(scope, row.get("id")).await
    }

    async fn finish(&self, scope: &impl RequestScope, id: Uuid) -> Result<SetupResult, SetupError> {
        let outcome = self.apply_consent(scope, id).await;
        if !matches!(outcome, Err(SetupError::ReviewRequired)) {
            return outcome;
        }
        // Validation failed before any authority committed. Preserve the linked
        // account and a truthful recoverable outcome instead of an OAuth error.
        let row = sqlx::query("UPDATE connector_setups SET state='needs_review' WHERE id=$1 AND user_context_id=$2 AND deployment_id=$3 AND state='pending' RETURNING extension_id,package_key")
            .bind(id).bind(scope.request_context().id.0).bind(scope.request_context().subject.deployment_id.0)
            .fetch_optional(&self.db).await?;
        if let Some(row) = row {
            return Ok(result(id, &row, SetupState::NeedsReview));
        }
        // Another callback may have completed the same intent while we retried.
        self.apply_consent(scope, id).await
    }

    async fn apply_consent(
        &self,
        scope: &impl RequestScope,
        id: Uuid,
    ) -> Result<SetupResult, SetupError> {
        let context = scope.request_context();
        let mut tx = self.db.begin().await?;
        // Owned-agent edits/archive use the same context lock. Lock ordering is
        // context, package, setup, extension, agent, connection, sorted skills.
        // Withdrawal locks the package before deleting OAuth sessions (which
        // detach setup records), so locking setup first would invert that order.
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_contexts WHERE id=$1 AND user_id=$2 AND deployment_id=$3 FOR UPDATE)")
            .bind(context.id.0).bind(context.user_id.0).bind(context.subject.deployment_id.0).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(SetupError::Unavailable);
        }
        let intent = sqlx::query("SELECT *,expires_at>now() AS valid FROM connector_setups WHERE id=$1 AND user_context_id=$2 AND deployment_id=$3")
            .bind(id).bind(context.id.0).bind(context.subject.deployment_id.0).fetch_optional(&mut *tx).await?.ok_or(SetupError::Unavailable)?;
        match intent.get::<String, _>("state").as_str() {
            "complete" => return Ok(result(id, &intent, SetupState::Complete)),
            "needs_review" => return Ok(result(id, &intent, SetupState::NeedsReview)),
            _ => {}
        }
        if !intent.get::<bool, _>("valid") {
            return Err(SetupError::ReviewRequired);
        }
        let package = sqlx::query("SELECT metadata FROM connector_packages WHERE deployment_id=$1 AND external_key=$2 AND version=$3 AND digest=$4 AND enabled FOR SHARE")
            .bind(context.subject.deployment_id.0).bind(intent.get::<String,_>("package_key"))
            .bind(intent.get::<i32,_>("package_version")).bind(intent.get::<String,_>("package_digest"))
            .fetch_optional(&mut *tx).await?.ok_or(SetupError::ReviewRequired)?;
        let intent = sqlx::query("SELECT *,expires_at>now() AS valid FROM connector_setups WHERE id=$1 AND user_context_id=$2 AND deployment_id=$3 FOR UPDATE")
            .bind(id).bind(context.id.0).bind(context.subject.deployment_id.0).fetch_optional(&mut *tx).await?.ok_or(SetupError::Unavailable)?;
        match intent.get::<String, _>("state").as_str() {
            "complete" => return Ok(result(id, &intent, SetupState::Complete)),
            "needs_review" => return Ok(result(id, &intent, SetupState::NeedsReview)),
            _ => {}
        }
        if !intent.get::<bool, _>("valid") {
            return Err(SetupError::ReviewRequired);
        }
        let metadata: PackageMetadata = serde_json::from_value(package.get("metadata"))
            .map_err(|_| SetupError::ReviewRequired)?;
        let extension: Uuid = intent.get("extension_id");
        let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM remote_extensions e JOIN connector_package_installations i ON i.extension_id=e.id JOIN remote_extension_versions v ON v.extension_id=e.id AND v.version=e.current_version WHERE e.id=$1 AND e.user_context_id=$2 AND e.current_version=$3 AND i.deployment_id=$4 AND i.external_key=$5 AND i.version=$6 AND e.lifecycle_state='active' AND e.operator_enabled AND e.consent_status='consented' AND e.conformance_status='passed' AND v.conformance_status='passed' FOR SHARE OF e,i,v)")
            .bind(extension).bind(context.id.0).bind(intent.get::<i32,_>("extension_version"))
            .bind(context.subject.deployment_id.0).bind(intent.get::<String,_>("package_key")).bind(intent.get::<i32,_>("package_version"))
            .fetch_one(&mut *tx).await?;
        // Account-only setups do not claim agent readiness or require execution
        // conformance. Any chosen agent requires the reviewed active version.
        let consent: Option<serde_json::Value> = intent.get("consent");
        if consent.is_some() && !active {
            return Err(SetupError::ReviewRequired);
        }
        let connection = sqlx::query("SELECT x.id,x.authorized_capabilities,c.tools FROM external_connections x JOIN remote_extension_credentials c ON c.extension_id=x.remote_extension_id WHERE x.remote_extension_id=$1 AND x.user_context_id=$2 AND x.authorization_state='authorized' AND (x.expires_at IS NULL OR x.expires_at>now()) AND (c.expires_at IS NULL OR c.expires_at>now()) FOR SHARE OF x,c")
            .bind(extension).bind(context.id.0).fetch_optional(&mut *tx).await?.ok_or(SetupError::ReviewRequired)?;
        if let Some(consent) = consent {
            let consent: SetupConsent =
                serde_json::from_value(consent).map_err(|_| SetupError::ReviewRequired)?;
            let agent = sqlx::query("SELECT a.id,a.template_id,a.requested_capability_categories FROM agent_definitions a JOIN deployment_agent_selections s ON s.agent_definition_id=a.id AND s.deployment_id=a.deployment_id WHERE a.owner_user_context_id=$1 AND a.deployment_id=$2 AND a.external_key=$3 AND a.instruction_version=$4 AND a.state='enabled' FOR SHARE OF a,s")
                .bind(context.id.0).bind(context.subject.deployment_id.0).bind(&consent.agent_external_key)
                .bind(consent.agent_instruction_version).fetch_optional(&mut *tx).await?.ok_or(SetupError::ReviewRequired)?;
            let agent_id: Uuid = agent.get("id");
            let categories: Vec<String> = agent.get("requested_capability_categories");
            let template: Option<Uuid> = agent.get("template_id");
            let template_categories: Vec<String> = if let Some(template) = template {
                sqlx::query_scalar("SELECT requested_capability_categories FROM agent_definitions WHERE id=$1 AND deployment_id=$2 AND state='enabled' FOR SHARE")
                    .bind(template).bind(context.subject.deployment_id.0).fetch_optional(&mut *tx).await?.ok_or(SetupError::ReviewRequired)?
            } else {
                categories.clone()
            };
            let authorized: Vec<String> = connection.get("authorized_capabilities");
            // A revocation made after this consent must win over an old OAuth
            // tab. Lock existing grants so concurrent revoke also wins if it
            // arrives while completion is applying the user's choices.
            let revoked = sqlx::query("SELECT state='revoked' AND updated_at>$5 AS revoked_since_consent FROM agent_capability_grants WHERE user_context_id=$1 AND agent_definition_id=$2 AND connection_id=$3 AND capability_external_key=ANY($4) ORDER BY capability_external_key FOR UPDATE")
                .bind(context.id.0).bind(agent_id).bind(connection.get::<Uuid,_>("id"))
                .bind(&consent.capability_external_keys).bind(intent.get::<chrono::DateTime<chrono::Utc>,_>("created_at"))
                .fetch_all(&mut *tx).await?;
            if revoked
                .iter()
                .any(|row| row.get::<bool, _>("revoked_since_consent"))
            {
                return Err(SetupError::ReviewRequired);
            }
            // Recheck observed schema as well as the account capability list.
            let declared: serde_json::Value = sqlx::query_scalar("SELECT capabilities FROM remote_extension_versions WHERE extension_id=$1 AND version=$2")
                .bind(extension).bind(intent.get::<i32,_>("extension_version")).fetch_one(&mut *tx).await?;
            let tools: serde_json::Value = connection.get("tools");
            for key in &consent.capability_external_keys {
                let allowed =
                    |categories: &[String]| categories.iter().any(|c| c == "*" || c == key);
                let matching_schema = declared.as_array().is_some_and(|caps| {
                    caps.iter().any(|cap| {
                        cap["external_key"].as_str() == Some(key)
                            && tools.as_array().is_some_and(|tools| {
                                tools.iter().any(|tool| {
                                    tool["name"].as_str() == Some(key)
                                        && tool["inputSchema"] == cap["input_schema"]
                                })
                            })
                    })
                });
                if !authorized.contains(key)
                    || !allowed(&categories)
                    || !allowed(&template_categories)
                    || !matching_schema
                {
                    return Err(SetupError::ReviewRequired);
                }
            }
            let mut skills = metadata.skills;
            skills.sort_by(|a, b| a.external_key.cmp(&b.external_key));
            if consent.enable_bundled_skills {
                for pinned in skills {
                    let skill: Option<Uuid> = sqlx::query_scalar("SELECT s.id FROM skill_packages s JOIN skill_package_versions v ON v.skill_id=s.id JOIN skill_installations i ON i.skill_id=s.id AND i.user_context_id=$1 JOIN connector_skill_installations b ON b.skill_id=s.id AND b.extension_id=$2 AND b.user_context_id=$1 WHERE s.deployment_id=$3 AND s.owner_user_context_id IS NULL AND s.external_key=$4 AND s.state='active' AND v.version=$5 AND v.digest=$6 AND i.installed_version=v.version AND i.enabled AND b.version=v.version FOR SHARE OF s,v,i,b")
                        .bind(context.id.0).bind(extension).bind(context.subject.deployment_id.0).bind(pinned.external_key)
                        .bind(pinned.version).bind(pinned.digest).fetch_optional(&mut *tx).await?;
                    let skill = skill.ok_or(SetupError::ReviewRequired)?;
                    let disabled_since_consent: Option<bool> = sqlx::query_scalar("SELECT NOT enabled AND updated_at>$4 FROM skill_agent_enablements WHERE user_context_id=$1 AND skill_id=$2 AND agent_definition_id=$3 FOR UPDATE")
                        .bind(context.id.0).bind(skill).bind(agent_id).bind(intent.get::<chrono::DateTime<chrono::Utc>,_>("created_at"))
                        .fetch_optional(&mut *tx).await?;
                    if disabled_since_consent == Some(true) {
                        return Err(SetupError::ReviewRequired);
                    }
                    sqlx::query("INSERT INTO skill_agent_enablements(user_context_id,skill_id,agent_definition_id,enabled) VALUES($1,$2,$3,true) ON CONFLICT(user_context_id,skill_id,agent_definition_id) DO UPDATE SET enabled=true,updated_at=now()")
                        .bind(context.id.0).bind(skill).bind(agent_id).execute(&mut *tx).await?;
                }
            }
            for key in consent.capability_external_keys {
                sqlx::query("INSERT INTO agent_capability_grants(user_context_id,agent_definition_id,connection_id,capability_external_key) VALUES($1,$2,$3,$4) ON CONFLICT(user_context_id,agent_definition_id,connection_id,capability_external_key) DO UPDATE SET state='enabled',revoked_at=NULL,updated_at=now()")
                    .bind(context.id.0).bind(agent_id).bind(connection.get::<Uuid,_>("id")).bind(key).execute(&mut *tx).await?;
            }
        }
        sqlx::query("UPDATE connector_setups SET state='complete',completed_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result(id, &intent, SetupState::Complete))
    }
}

fn result(id: Uuid, row: &sqlx::postgres::PgRow, state: SetupState) -> SetupResult {
    SetupResult {
        setup_id: Some(id),
        extension_id: row.get("extension_id"),
        external_key: row.get("package_key"),
        state,
        authorization_url: None,
    }
}

fn validate_consent(consent: &SetupConsent) -> Result<(), SetupError> {
    let mut keys = std::collections::HashSet::new();
    if consent.agent_external_key.is_empty()
        || consent.agent_external_key.len() > 255
        || consent.agent_instruction_version < 1
        || consent.capability_external_keys.len() > 64
        || consent
            .capability_external_keys
            .iter()
            .any(|key| key.is_empty() || key.len() > 511 || !keys.insert(key))
    {
        return Err(SetupError::Invalid);
    }
    Ok(())
}
