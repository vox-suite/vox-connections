WITH actor AS MATERIALIZED (
    SELECT a.id,a.template_id,a.requested_capability_categories
    FROM agent_definitions a
    JOIN deployment_agent_selections s ON s.agent_definition_id=a.id AND s.deployment_id=a.deployment_id
    WHERE a.owner_user_context_id=$1 AND a.deployment_id=$2 AND a.external_key=$3 AND a.state='enabled'
      AND (a.template_id IS NULL OR EXISTS(SELECT 1 FROM agent_definitions t WHERE t.id=a.template_id AND t.state='enabled'))
), candidates AS (
    SELECT ts_rank_cd(m.search_document,to_tsquery('simple',$4)) AS rank,
           m.external_key AS name,x.id AS identity,
           jsonb_build_object('kind','tool','connection_id',x.id,'name',m.external_key,
             'description',left(m.display_name,512),'effect',m.effect,
             'approval_required',m.consequential OR m.effect IN ('write','mixed'),
             'extension_version',m.version,'requires_refresh',COALESCE(c.expires_at<=now(),false)) AS metadata
    FROM connector_tool_metadata m
    JOIN remote_extensions e ON e.id=m.extension_id AND e.current_version=m.version AND e.user_context_id=$1
    JOIN remote_extension_versions v ON v.extension_id=e.id AND v.version=m.version
    JOIN external_connections x ON x.remote_extension_id=e.id AND x.user_context_id=$1
    JOIN remote_extension_credentials c ON c.extension_id=e.id
    JOIN agent_capability_grants g ON g.connection_id=x.id AND g.user_context_id=$1 AND g.capability_external_key=m.external_key AND g.state='enabled'
    JOIN actor a ON a.id=g.agent_definition_id
    LEFT JOIN connector_package_installations pi ON pi.extension_id=e.id
    LEFT JOIN connector_packages p ON p.deployment_id=pi.deployment_id AND p.external_key=pi.external_key AND p.version=pi.version
    WHERE m.search_document @@ to_tsquery('simple',$4)
      AND e.lifecycle_state='active' AND e.consent_status='consented' AND e.operator_enabled AND e.conformance_status='passed' AND v.conformance_status='passed'
      AND x.authorization_state='authorized' AND (x.expires_at IS NULL OR x.expires_at>now())
      AND (c.expires_at IS NULL OR c.expires_at>now() OR c.refresh_token_ciphertext IS NOT NULL)
      AND (m.external_key=ANY(a.requested_capability_categories) OR '*'=ANY(a.requested_capability_categories))
      AND (a.template_id IS NULL OR EXISTS(SELECT 1 FROM agent_definitions t WHERE t.id=a.template_id AND t.state='enabled' AND (m.external_key=ANY(t.requested_capability_categories) OR '*'=ANY(t.requested_capability_categories))))
      AND m.external_key=ANY(x.authorized_capabilities)
      AND (pi.extension_id IS NULL OR (p.deployment_id=$2 AND p.enabled AND p.manifest->>'external_key'=e.external_key
        AND p.manifest->>'endpoint_url'=e.endpoint_url AND p.manifest->>'protocol'=e.protocol
        AND p.manifest->'capabilities'=v.capabilities AND p.manifest->'operator'=jsonb_strip_nulls(jsonb_build_object(
          'operator_id',e.operator_id,'operator_name',e.operator_name,'support_email',e.support_email,'terms_url',e.terms_url))))
      AND EXISTS(SELECT 1 FROM jsonb_array_elements(v.capabilities) cap WHERE cap->>'external_key'=m.external_key
        AND EXISTS(SELECT 1 FROM jsonb_array_elements(c.tools) tool WHERE tool->>'name'=m.external_key AND tool->'inputSchema'=cap->'input_schema'))
    UNION ALL
    SELECT ts_rank_cd(v.search_document,to_tsquery('simple',$4)) AS rank,
           s.external_key AS name,s.id AS identity,
           jsonb_build_object('kind','skill','skill_id',s.id,'name',s.external_key,
             'title',left(v.title,255),'description',left(v.summary,512),'version',v.version,'digest',v.digest) AS metadata
    FROM skill_package_versions v
    JOIN skill_packages s ON s.id=v.skill_id AND s.deployment_id=$2 AND s.state='active'
    JOIN skill_installations i ON i.skill_id=s.id AND i.user_context_id=$1 AND i.installed_version=v.version AND i.enabled
    JOIN skill_agent_enablements se ON se.skill_id=s.id AND se.user_context_id=$1 AND se.enabled
    JOIN actor a ON a.id=se.agent_definition_id
    WHERE v.search_document @@ to_tsquery('simple',$4) AND v.digest IS NOT NULL
      AND (s.owner_user_context_id IS NULL OR s.owner_user_context_id=$1)
)
SELECT metadata FROM candidates ORDER BY rank DESC,name,identity,metadata->>'kind' LIMIT 11 OFFSET $5
