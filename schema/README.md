# Database contract

`connectors.sql` is a reference snapshot of the connector tables used by the current services. It was exported from Vox Core's migrated PostgreSQL schema. It lets another host use `vox-connections` without the Vox Core crate or migrations. Apply it **only to a new database**, after creating host-owned `users`, `platform_deployments`, `user_contexts`, `agent_definitions`, and `deployment_agent_selections` tables with the columns and keys shown in [`examples/independent_host_schema.sql`](../examples/independent_host_schema.sql).

```sh
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f examples/independent_host_schema.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema/connectors.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema/packages.sql
```

A host must authenticate users, scope context IDs, restrict its database role to trusted server processes, and enforce its own user access policy. The reference schema does not install host-specific row-level security policies. Installation does not grant a capability or approve an action. The runtime still requires the host's authority layer to mediate consequential execution.

Vox Core uses its existing incremental migrations. Do not apply this snapshot to a Vox Core database. When the connector schema changes, update the snapshot and verify it both in an independent-host database and with the consuming Vox Core revision.

`packages.sql` adds the immutable deployment catalog and installation bindings. Existing independent-host databases built from the previous snapshot apply this addition once. Vox Core applies its matching incremental package migration.

`playstation.sql` adds encrypted PSN credentials and activity checkpoints. Apply it after `connectors.sql`, including on existing independent-host databases before upgrading the connection service. Core applies its matching incremental migration.
