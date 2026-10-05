-- Preserve account IDs and encrypted bytes. Ambiguous legacy ownership stays inaccessible.
ALTER TABLE vox_connections ADD COLUMN user_context_id UUID REFERENCES user_contexts(id);
ALTER TABLE vox_connection_setups ADD COLUMN user_context_id UUID REFERENCES user_contexts(id);
UPDATE vox_connections c SET user_context_id=(SELECT min(id::text)::uuid FROM user_contexts u WHERE u.user_id=c.user_id)
WHERE (SELECT count(*) FROM user_contexts u WHERE u.user_id=c.user_id)=1;
UPDATE vox_connection_setups s SET user_context_id=(SELECT min(id::text)::uuid FROM user_contexts u WHERE u.user_id=s.user_id)
WHERE (SELECT count(*) FROM user_contexts u WHERE u.user_id=s.user_id)=1;
UPDATE vox_connections SET failure_code='scope_reassociation_required',lease_token=NULL,lease_until=NULL WHERE user_context_id IS NULL;
ALTER TABLE vox_connections DROP CONSTRAINT vox_connections_user_connector_unique;
ALTER TABLE vox_connections ADD CONSTRAINT vox_connections_context_connector_unique UNIQUE(user_context_id,connector_id);
ALTER TABLE vox_connections ADD CONSTRAINT vox_connections_context_owner FOREIGN KEY(user_context_id,user_id) REFERENCES user_contexts(id,user_id);
ALTER TABLE vox_connection_setups ADD CONSTRAINT vox_setups_context_owner FOREIGN KEY(user_context_id,user_id) REFERENCES user_contexts(id,user_id);
