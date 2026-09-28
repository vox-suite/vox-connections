//! Deployment-reviewed immutable packages; installation never grants agent access.
use crate::{
    identity::RequestScope,
    remote_extensions::{
        InstallExtensionRequest, RemoteExtension, RemoteExtensionError, RemoteExtensionService,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishPackage {
    pub deployment_id: Uuid,
    pub version: i32,
    pub manifest: InstallExtensionRequest,
    pub review: Value,
    pub metadata: PackageMetadata,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageMetadata {
    pub schema_version: u32,
    pub protocol_version: String,
    pub auth_mode: PackageAuthMode,
    pub credential_custody: String,
    pub skills: Vec<PinnedSkill>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PackageAuthMode {
    Oauth,
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinnedSkill {
    pub external_key: String,
    pub version: i32,
    pub digest: String,
}

impl PackageMetadata {
    pub fn oauth() -> Self {
        Self {
            schema_version: 1,
            protocol_version: "2025-11-25".into(),
            auth_mode: PackageAuthMode::Oauth,
            credential_custody: "platform_held".into(),
            skills: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), PackageError> {
        if self.schema_version != 1
            || !matches!(self.protocol_version.as_str(), "2025-11-25" | "2026-07-28")
            || !matches!(
                self.credential_custody.as_str(),
                "platform_held" | "external_operator" | "none"
            )
            || (self.auth_mode == PackageAuthMode::None && self.credential_custody != "none")
            || (self.auth_mode == PackageAuthMode::Oauth
                && self.credential_custody != "platform_held")
            || self.skills.len() > 16
        {
            return Err(PackageError::Invalid);
        }
        let mut keys = std::collections::HashSet::new();
        for skill in &self.skills {
            if skill.version < 1
                || skill.external_key.is_empty()
                || skill.external_key.len() > 64
                || !skill
                    .external_key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || skill.external_key.starts_with('-')
                || skill.external_key.ends_with('-')
                || skill.external_key.contains("--")
                || skill.digest.len() != 64
                || !skill
                    .digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || !keys.insert(&skill.external_key)
            {
                return Err(PackageError::Invalid);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectorPackage {
    pub version: i32,
    pub digest: String,
    pub manifest: InstallExtensionRequest,
    pub metadata: PackageMetadata,
}

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("invalid connector package")]
    Invalid,
    #[error("connector package is unavailable")]
    Unavailable,
    #[error("package version or installed declaration conflicts")]
    Conflict,
    #[error(transparent)]
    Extension(#[from] RemoteExtensionError),
    #[error("connector package storage unavailable")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct PackageRegistry {
    db: PgPool,
}

impl PackageRegistry {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Operator-only publication. Re-publishing identical bytes is safe;
    /// changing a published version is prohibited, even after withdrawal.
    pub async fn publish(&self, request: PublishPackage) -> Result<ConnectorPackage, PackageError> {
        RemoteExtensionService::validate_install_request(&request.manifest, false)?;
        request.metadata.validate()?;
        if request.version < 1
            || request.manifest.external_key != request.manifest.external_key.trim()
            || request.manifest.display_name != request.manifest.display_name.trim()
            || request.manifest.endpoint_url != request.manifest.endpoint_url.trim()
            || !request.review.as_object().is_some_and(|v| !v.is_empty())
        {
            return Err(PackageError::Invalid);
        }
        let manifest =
            serde_json::to_value(&request.manifest).map_err(|_| PackageError::Invalid)?;
        let bytes = package_bytes(&request.manifest, &request.metadata)?;
        if bytes.len() > 256 * 1024
            || serde_json::to_vec(&request.review)
                .map_err(|_| PackageError::Invalid)?
                .len()
                > 256 * 1024
        {
            return Err(PackageError::Invalid);
        }
        let digest = hex::encode(Sha256::digest(&bytes));
        let mut tx = self.db.begin().await?;
        sqlx::query("INSERT INTO connector_packages (deployment_id,external_key,version,digest,manifest,review,metadata) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
            .bind(request.deployment_id).bind(&request.manifest.external_key).bind(request.version)
            .bind(&digest).bind(&manifest).bind(&request.review).bind(serde_json::to_value(&request.metadata).map_err(|_| PackageError::Invalid)?).execute(&mut *tx).await?;
        let stored: String = sqlx::query_scalar("SELECT digest FROM connector_packages WHERE deployment_id=$1 AND external_key=$2 AND version=$3")
            .bind(request.deployment_id).bind(&request.manifest.external_key).bind(request.version)
            .fetch_one(&mut *tx).await?;
        if stored != digest {
            return Err(PackageError::Conflict);
        }
        tx.commit().await?;
        Ok(ConnectorPackage {
            version: request.version,
            digest,
            manifest: request.manifest,
            metadata: request.metadata,
        })
    }

    /// Latest enabled version of each package, bounded for host discovery.
    pub async fn list(
        &self,
        context: &impl RequestScope,
    ) -> Result<Vec<ConnectorPackage>, PackageError> {
        let rows = sqlx::query("SELECT DISTINCT ON (external_key) version,digest,manifest,metadata FROM connector_packages WHERE deployment_id=$1 AND enabled ORDER BY external_key,version DESC LIMIT 100")
            .bind(context.request_context().subject.deployment_id.0).fetch_all(&self.db).await?;
        rows.into_iter()
            .map(|row| {
                Ok(ConnectorPackage {
                    version: row.get("version"),
                    digest: row.get("digest"),
                    manifest: serde_json::from_value(row.get("manifest"))
                        .map_err(|_| PackageError::Invalid)?,
                    metadata: serde_json::from_value(row.get("metadata"))
                        .map_err(|_| PackageError::Invalid)?,
                })
            })
            .collect()
    }

    /// Install exactly the digest the user reviewed. Concurrent clicks across
    /// processes serialize per context. Existing grants are never expanded.
    pub async fn install(
        &self,
        context: &impl RequestScope,
        key: &str,
        version: i32,
        digest: &str,
    ) -> Result<RemoteExtension, PackageError> {
        if version < 1
            || key.is_empty()
            || key.len() > 255
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(PackageError::Invalid);
        }
        let scope = context.request_context();
        let mut tx = self.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("package-install:{}", scope.id.0))
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query("SELECT manifest,digest,review,metadata FROM connector_packages WHERE deployment_id=$1 AND external_key=$2 AND version=$3 AND enabled FOR SHARE")
            .bind(scope.subject.deployment_id.0).bind(key).bind(version).fetch_optional(&mut *tx).await?
            .ok_or(PackageError::Unavailable)?;
        if row.get::<String, _>("digest") != digest {
            return Err(PackageError::Conflict);
        }
        let manifest: InstallExtensionRequest =
            serde_json::from_value(row.get("manifest")).map_err(|_| PackageError::Invalid)?;
        let metadata: PackageMetadata =
            serde_json::from_value(row.get("metadata")).map_err(|_| PackageError::Invalid)?;
        metadata.validate()?;
        let extensions = RemoteExtensionService::new(self.db.clone());
        let existing_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM remote_extensions WHERE user_context_id=$1 AND external_key=$2 AND lifecycle_state <> 'removed' FOR UPDATE")
            .bind(scope.id.0).bind(key).fetch_optional(&mut *tx).await?;
        let mut extension = match existing_id {
            Some(id) => extensions.get_in_transaction(context, id, &mut tx).await?,
            None => {
                let id = extensions
                    .install_in_transaction(context, manifest.clone(), &mut tx)
                    .await?;
                extensions.get_in_transaction(context, id, &mut tx).await?
            }
        };
        if extension.endpoint_url != manifest.endpoint_url
            || extension.protocol != manifest.protocol
            || extension.operator != manifest.operator
            || extension.capabilities != manifest.capabilities
        {
            return Err(PackageError::Conflict);
        }
        // A normal install cannot silently rebind an existing installation to
        // changed auth, custody, protocol or bundled guidance. Updates have a
        // separate explicit consent path and clear account authority.
        let previous_metadata: Option<Value> = sqlx::query_scalar("SELECT p.metadata FROM connector_package_installations i JOIN connector_packages p USING(deployment_id,external_key,version) WHERE i.extension_id=$1")
            .bind(extension.id).fetch_optional(&mut *tx).await?;
        if previous_metadata.is_some_and(|previous| {
            previous != serde_json::to_value(&metadata).unwrap_or(Value::Null)
        }) {
            return Err(PackageError::Conflict);
        }
        let mut ordered_skills = metadata.skills.iter().collect::<Vec<_>>();
        ordered_skills.sort_by_key(|skill| &skill.external_key);
        for pinned in ordered_skills {
            let skill: Option<(Uuid, Option<String>)> = sqlx::query_as(
                "SELECT s.id,v.digest FROM skill_packages s JOIN skill_package_versions v ON v.skill_id=s.id
                 WHERE s.deployment_id=$1 AND s.external_key=$2 AND s.owner_user_context_id IS NULL AND s.state='active' AND v.version=$3 FOR SHARE OF s"
            ).bind(scope.subject.deployment_id.0).bind(&pinned.external_key).bind(pinned.version).fetch_optional(&mut *tx).await?;
            let (skill_id, stored_digest) = skill.ok_or(PackageError::Unavailable)?;
            if stored_digest.as_deref() != Some(&pinned.digest) {
                return Err(PackageError::Conflict);
            }
            let existing: Option<i32> = sqlx::query_scalar("SELECT installed_version FROM skill_installations WHERE user_context_id=$1 AND skill_id=$2 FOR UPDATE").bind(scope.id.0).bind(skill_id).fetch_optional(&mut *tx).await?;
            if existing.is_some_and(|v| v != pinned.version) {
                return Err(PackageError::Conflict);
            }
            sqlx::query("INSERT INTO skill_installations (user_context_id,skill_id,installed_version,independently_installed) VALUES ($1,$2,$3,false) ON CONFLICT DO NOTHING").bind(scope.id.0).bind(skill_id).bind(pinned.version).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO connector_skill_installations(extension_id,skill_id,user_context_id,version) VALUES ($1,$2,$3,$4) ON CONFLICT(extension_id,skill_id) DO UPDATE SET version=EXCLUDED.version").bind(extension.id).bind(skill_id).bind(scope.id.0).bind(pinned.version).execute(&mut *tx).await?;
        }
        // Binding and extension state commit together using one pool connection.
        sqlx::query("INSERT INTO connector_package_installations(extension_id,deployment_id,external_key,version) VALUES ($1,$2,$3,$4) ON CONFLICT (extension_id) DO UPDATE SET version=EXCLUDED.version,installed_at=now()")
            .bind(extension.id).bind(scope.subject.deployment_id.0).bind(key).bind(version).execute(&mut *tx).await?;
        // Only an operator's explicit effect-verification evidence can carry
        // read conformance across installations. Writes require their own
        // behavioral conformance and remain disabled after package install.
        let review: Value = row.get("review");
        if !manifest.capabilities.is_empty()
            && manifest
                .capabilities
                .iter()
                .all(|cap| !cap.effect.is_consequential())
            && review.get("read_effects_verified").and_then(Value::as_bool) == Some(true)
            && review
                .get("evidence")
                .and_then(Value::as_object)
                .is_some_and(|e| !e.is_empty())
            && extension.lifecycle_state == crate::remote_extensions::LifecycleState::Installed
            && extension.consent_status == crate::remote_extensions::ConsentStatus::Consented
            && extension.conformance_status == crate::remote_extensions::ConformanceStatus::Pending
        {
            extensions
                .record_conformance_in_transaction(
                    context,
                    extension.id,
                    extension.current_version,
                    true,
                    review,
                    &mut tx,
                )
                .await?;
            extensions
                .set_operator_enabled_in_transaction(context, extension.id, true, &mut tx)
                .await?;
            extension = extensions
                .get_in_transaction(context, extension.id, &mut tx)
                .await?;
        }
        tx.commit().await?;
        Ok(extension)
    }

    /// Withdrawal stops new installs and use without erasing historical evidence.
    pub async fn withdraw(
        &self,
        deployment: Uuid,
        key: &str,
        version: i32,
    ) -> Result<(), PackageError> {
        let mut tx = self.db.begin().await?;
        let changed = sqlx::query("UPDATE connector_packages SET enabled=false WHERE deployment_id=$1 AND external_key=$2 AND version=$3")
            .bind(deployment).bind(key).bind(version).execute(&mut *tx).await?.rows_affected();
        if changed == 0 {
            return Err(PackageError::Unavailable);
        }
        let ids = sqlx::query_scalar::<_, Uuid>("UPDATE remote_extensions SET lifecycle_state='disabled',operator_enabled=false,updated_at=now() WHERE id IN (SELECT extension_id FROM connector_package_installations WHERE deployment_id=$1 AND external_key=$2 AND version=$3) AND lifecycle_state <> 'removed' RETURNING id")
            .bind(deployment).bind(key).bind(version).fetch_all(&mut *tx).await?;
        sqlx::query("DELETE FROM mcp_authorization_sessions WHERE extension_id=ANY($1)")
            .bind(&ids)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM remote_extension_credentials WHERE extension_id=ANY($1)")
            .bind(&ids)
            .execute(&mut *tx)
            .await?;
        crate::remote_extensions::revoke_extensions_authority(&mut tx, &ids).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT extension_id FROM connector_package_installations WHERE deployment_id=$1 AND external_key=$2 AND version=$3").bind(deployment).bind(key).bind(version).fetch_all(&mut *tx).await?;
        for id in ids {
            detach_package_skills(&mut tx, id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

/// One trust check for OAuth, activation and both invocation paths. Manually
/// registered extensions have no package binding and retain their own policy.
pub(crate) async fn installed_package_available<
    'e,
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
>(
    executor: E,
    extension_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM connector_package_installations pi \
         JOIN connector_packages p ON p.deployment_id=pi.deployment_id \
           AND p.external_key=pi.external_key AND p.version=pi.version \
         JOIN remote_extensions e ON e.id=pi.extension_id \
         LEFT JOIN remote_extension_versions v ON v.extension_id=e.id AND v.version=e.current_version \
         WHERE pi.extension_id=$1 AND (NOT p.enabled \
           OR p.manifest->>'external_key' IS DISTINCT FROM e.external_key \
           OR p.manifest->>'endpoint_url' IS DISTINCT FROM e.endpoint_url \
           OR p.manifest->>'protocol' IS DISTINCT FROM e.protocol \
           OR p.manifest->'capabilities' IS DISTINCT FROM v.capabilities \
           OR p.manifest->'operator' IS DISTINCT FROM jsonb_strip_nulls(jsonb_build_object(\
             'operator_id',e.operator_id,'operator_name',e.operator_name,\
             'support_email',e.support_email,'terms_url',e.terms_url))))",
    ).bind(extension_id).fetch_one(executor).await
}

/// Canonical bytes used by author tooling and immutable publication.
pub fn package_bytes(
    manifest: &InstallExtensionRequest,
    metadata: &PackageMetadata,
) -> Result<Vec<u8>, PackageError> {
    serde_json::to_vec(&serde_json::json!({"manifest":manifest,"metadata":metadata}))
        .map_err(|_| PackageError::Invalid)
}

pub(crate) async fn detach_package_skills(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    extension_id: Uuid,
) -> Result<(), sqlx::Error> {
    // Serialize shared dependency cleanup before any association is removed.
    // All package operations lock installations in UUID order.
    sqlx::query("SELECT i.skill_id FROM skill_installations i JOIN connector_skill_installations b ON b.user_context_id=i.user_context_id AND b.skill_id=i.skill_id WHERE b.extension_id=$1 ORDER BY i.user_context_id,i.skill_id FOR UPDATE OF i")
        .bind(extension_id).fetch_all(&mut **tx).await?;
    let rows: Vec<(Uuid,Uuid)> = sqlx::query_as("DELETE FROM connector_skill_installations WHERE extension_id=$1 RETURNING user_context_id,skill_id").bind(extension_id).fetch_all(&mut **tx).await?;
    for (context, skill) in rows {
        sqlx::query("UPDATE skill_installations i SET enabled=false,updated_at=now() WHERE i.user_context_id=$1 AND i.skill_id=$2 AND NOT i.independently_installed AND NOT EXISTS (SELECT 1 FROM connector_skill_installations b WHERE b.user_context_id=i.user_context_id AND b.skill_id=i.skill_id)").bind(context).bind(skill).execute(&mut **tx).await?;
    }
    Ok(())
}
