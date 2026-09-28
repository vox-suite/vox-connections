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
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectorPackage {
    pub version: i32,
    pub digest: String,
    pub manifest: InstallExtensionRequest,
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
        let bytes = serde_json::to_vec(&request.manifest).map_err(|_| PackageError::Invalid)?;
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
        sqlx::query("INSERT INTO connector_packages (deployment_id,external_key,version,digest,manifest,review) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
            .bind(request.deployment_id).bind(&request.manifest.external_key).bind(request.version)
            .bind(&digest).bind(&manifest).bind(&request.review).execute(&mut *tx).await?;
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
        })
    }

    /// Latest enabled version of each package, bounded for host discovery.
    pub async fn list(
        &self,
        context: &impl RequestScope,
    ) -> Result<Vec<ConnectorPackage>, PackageError> {
        let rows = sqlx::query("SELECT DISTINCT ON (external_key) version,digest,manifest FROM connector_packages WHERE deployment_id=$1 AND enabled ORDER BY external_key,version DESC LIMIT 100")
            .bind(context.request_context().subject.deployment_id.0).fetch_all(&self.db).await?;
        rows.into_iter()
            .map(|row| {
                Ok(ConnectorPackage {
                    version: row.get("version"),
                    digest: row.get("digest"),
                    manifest: serde_json::from_value(row.get("manifest"))
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
        let row = sqlx::query("SELECT manifest,digest,review FROM connector_packages WHERE deployment_id=$1 AND external_key=$2 AND version=$3 AND enabled FOR SHARE")
            .bind(scope.subject.deployment_id.0).bind(key).bind(version).fetch_optional(&mut *tx).await?
            .ok_or(PackageError::Unavailable)?;
        if row.get::<String, _>("digest") != digest {
            return Err(PackageError::Conflict);
        }
        let manifest: InstallExtensionRequest =
            serde_json::from_value(row.get("manifest")).map_err(|_| PackageError::Invalid)?;
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
        // Binding and extension state commit together using one pool connection.
        sqlx::query("INSERT INTO connector_package_installations(extension_id,deployment_id,external_key,version) VALUES ($1,$2,$3,$4) ON CONFLICT (extension_id) DO UPDATE SET version=EXCLUDED.version,installed_at=now()")
            .bind(extension.id).bind(scope.subject.deployment_id.0).bind(key).bind(version).execute(&mut *tx).await?;
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
        tx.commit().await?;
        Ok(())
    }
}
