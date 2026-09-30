use crate::identity::RequestContext;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

const MAX_CAPABILITY_KEY_BYTES: usize = 511;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateGrantRequest {
    pub agent_external_key: String,
    pub connection_id: Uuid,
    pub capability_external_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityGrant {
    pub id: Uuid,
    pub agent_external_key: String,
    pub connection_id: Uuid,
    pub capability_external_key: String,
}

#[derive(Clone)]
pub struct CapabilityGrantService {
    db: sqlx::PgPool,
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityGrantError {
    #[error("capability grant request is invalid")]
    Invalid,
    #[error("agent, connection, or capability is unavailable")]
    Unavailable,
    #[error("capability grant storage is unavailable")]
    Database(#[from] sqlx::Error),
}

impl CapabilityGrantService {
    pub fn new(db: sqlx::PgPool) -> Self {
        Self { db }
    }

    pub async fn grant(
        &self,
        context: &RequestContext,
        request: CreateGrantRequest,
    ) -> Result<CapabilityGrant, CapabilityGrantError> {
        let agent_key = key(&request.agent_external_key)?;
        let capability_key = key(&request.capability_external_key)?;
        let mut tx = self.db.begin().await?;
        let row = sqlx::query(
            "SELECT a.id, a.external_key FROM agent_definitions a \
             JOIN deployment_agent_selections s ON s.agent_definition_id = a.id \
             WHERE a.deployment_id = $1 AND a.external_key = $2 AND a.state = 'enabled' AND (a.template_id IS NULL OR EXISTS (SELECT 1 FROM agent_definitions template WHERE template.id=a.template_id AND template.state='enabled')) AND a.owner_user_context_id = $3",
        )
        .bind(context.subject.deployment_id.0)
        .bind(&agent_key)
        .bind(context.id.0)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(CapabilityGrantError::Unavailable)?;
        let agent_id: Uuid = row.try_get("id")?;
        let declared_for_agent = sqlx::query_scalar::<_, bool>(
            "SELECT ($3 = ANY(a.requested_capability_categories) OR '*' = ANY(a.requested_capability_categories)) AND (a.template_id IS NULL OR EXISTS (SELECT 1 FROM agent_definitions t WHERE t.id=a.template_id AND t.state='enabled' AND ($3=ANY(t.requested_capability_categories) OR '*'=ANY(t.requested_capability_categories)))) FROM agent_definitions a \
             WHERE a.id = $1 AND a.deployment_id = $2",
        )
        .bind(agent_id)
        .bind(context.subject.deployment_id.0)
        .bind(&capability_key)
        .fetch_one(&mut *tx)
        .await?;
        if !declared_for_agent {
            return Err(CapabilityGrantError::Unavailable);
        }
        // Lock the connection and integration while creating the grant. A
        // concurrent declaration upgrade must either wait and revoke this
        // grant, or finish first so this check sees the disabled version.
        let connection_is_authorized = sqlx::query_scalar::<_, Uuid>(
            "SELECT x.id FROM external_connections x \
             WHERE x.id = $1 AND x.user_context_id = $2 \
             AND x.authorization_state = 'authorized' \
             AND (x.expires_at IS NULL OR x.expires_at > now()) \
             AND $4 = ANY(x.authorized_capabilities) \
             AND (EXISTS (SELECT 1 FROM integration_definitions i \
                  JOIN integration_capability_declarations c ON c.integration_id=i.id \
                  WHERE i.id=x.integration_id AND i.deployment_id=$3 AND i.state='enabled' \
                  AND concat(i.external_key,'.',c.external_key)=$4) \
               OR EXISTS (SELECT 1 FROM remote_extensions e \
                  JOIN remote_extension_versions v ON v.extension_id=e.id AND v.version=e.current_version \
                  WHERE e.id=x.remote_extension_id AND e.user_context_id=x.user_context_id \
                  AND e.lifecycle_state='active' AND e.consent_status='consented' \
                  AND EXISTS (SELECT 1 FROM jsonb_array_elements(v.capabilities) cap \
                              WHERE cap->>'external_key'=$4))) \
             FOR SHARE OF x",
        )
        .bind(request.connection_id)
        .bind(context.id.0)
        .bind(context.subject.deployment_id.0)
        .bind(&capability_key)
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
        if !connection_is_authorized {
            return Err(CapabilityGrantError::Unavailable);
        }
        let id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO agent_capability_grants \
             (user_context_id, agent_definition_id, connection_id, capability_external_key) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (user_context_id, agent_definition_id, connection_id, capability_external_key) \
             DO UPDATE SET state = 'enabled', revoked_at = NULL, updated_at = now() RETURNING id",
        )
        .bind(context.id.0)
        .bind(agent_id)
        .bind(request.connection_id)
        .bind(&capability_key)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CapabilityGrant {
            id,
            agent_external_key: row.try_get("external_key")?,
            connection_id: request.connection_id,
            capability_external_key: capability_key,
        })
    }

    pub async fn effective_for_agent(
        &self,
        context: &RequestContext,
        agent_external_key: &str,
    ) -> Result<Vec<CapabilityGrant>, CapabilityGrantError> {
        let agent_key = key(agent_external_key)?;
        let rows = sqlx::query(
            "SELECT g.id, a.external_key, g.connection_id, g.capability_external_key \
             FROM agent_capability_grants g \
             JOIN agent_definitions a ON a.id = g.agent_definition_id \
             JOIN deployment_agent_selections s ON s.agent_definition_id = a.id \
             JOIN external_connections x ON x.id = g.connection_id \
             WHERE g.user_context_id = $1 AND a.deployment_id = $2 AND a.external_key = $3 \
             AND a.state = 'enabled' AND (a.template_id IS NULL OR EXISTS (SELECT 1 FROM agent_definitions template WHERE template.id=a.template_id AND template.state='enabled' AND (g.capability_external_key=ANY(template.requested_capability_categories) OR '*'=ANY(template.requested_capability_categories)))) AND a.owner_user_context_id = $1 AND g.state = 'enabled' \
             AND x.user_context_id = $1 AND x.authorization_state = 'authorized' \
             AND (x.expires_at IS NULL OR x.expires_at > now()) \
             AND (g.capability_external_key = ANY(a.requested_capability_categories) \
                  OR '*' = ANY(a.requested_capability_categories)) \
             AND g.capability_external_key = ANY(x.authorized_capabilities) \
             AND (EXISTS (SELECT 1 FROM integration_definitions i \
                  JOIN integration_capability_declarations c ON c.integration_id=i.id \
                  WHERE i.id=x.integration_id AND i.deployment_id=$2 AND i.state='enabled' \
                  AND concat(i.external_key,'.',c.external_key)=g.capability_external_key) \
               OR EXISTS (SELECT 1 FROM remote_extensions e \
                  JOIN remote_extension_versions v ON v.extension_id=e.id AND v.version=e.current_version \
                  WHERE e.id=x.remote_extension_id AND e.user_context_id=x.user_context_id \
                  AND e.lifecycle_state='active' AND e.consent_status='consented' \
                  AND EXISTS (SELECT 1 FROM jsonb_array_elements(v.capabilities) cap \
                              WHERE cap->>'external_key'=g.capability_external_key))) \
             ORDER BY g.capability_external_key",
        )
        .bind(context.id.0)
        .bind(context.subject.deployment_id.0)
        .bind(agent_key)
        .fetch_all(&self.db)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(CapabilityGrant {
                    id: row.try_get("id")?,
                    agent_external_key: row.try_get("external_key")?,
                    connection_id: row.try_get("connection_id")?,
                    capability_external_key: row.try_get("capability_external_key")?,
                })
            })
            .collect()
    }

    pub async fn revoke(
        &self,
        context: &RequestContext,
        request: CreateGrantRequest,
    ) -> Result<(), CapabilityGrantError> {
        let agent_key = key(&request.agent_external_key)?;
        let capability_key = key(&request.capability_external_key)?;
        let changed = sqlx::query(
            "UPDATE agent_capability_grants g SET state = 'revoked', revoked_at = now(), updated_at = now() \
             FROM agent_definitions a WHERE g.agent_definition_id = a.id \
             AND g.user_context_id = $1 AND a.deployment_id = $2 AND a.external_key = $3 \
             AND g.connection_id = $4 AND g.capability_external_key = $5 AND g.state = 'enabled'",
        )
        .bind(context.id.0)
        .bind(context.subject.deployment_id.0)
        .bind(agent_key)
        .bind(request.connection_id)
        .bind(capability_key)
        .execute(&self.db)
        .await?
        .rows_affected();
        if changed == 0 {
            Err(CapabilityGrantError::Unavailable)
        } else {
            Ok(())
        }
    }
}

fn key(value: &str) -> Result<String, CapabilityGrantError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_CAPABILITY_KEY_BYTES {
        Err(CapabilityGrantError::Invalid)
    } else {
        Ok(value.to_owned())
    }
}
