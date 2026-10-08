# Database contract

Connections owns reusable integration, account, grant and skill storage. A host supplies identity and agent tables. Core uses its own forward migrations; do not apply these standalone snapshots to Core.

For a fresh independent host, apply `examples/independent_host_schema.sql`, then `connectors.sql`, `public-mcp.sql`, `skill-content.sql`, `discovery.sql`, `playstation.sql`, `packages.sql` and `setup.sql`, in that order. For curated providers also apply `accounts.sql` and `account-authority.sql`. The independent-host integration tests exercise these public contracts.

For an existing platform host, retain the platform tables and apply missing additive feature schemas once, using a migration ledger. For an existing curated-only host, install the platform schemas first, then apply `account-scope-upgrade.sql` and `account-authority.sql`; do not replay `accounts.sql`. Upgrade steps preserve account IDs and encrypted bytes. The original user/provider ciphertext associated data remains unchanged. User-only accounts map to a context only when exactly one context owns that user; ambiguous accounts require explicit authenticated reassociation. No upgrade or link creates agent grants.

Hosts must verify schema compatibility before upgrading. The library performs no schema migration or timeline writes itself. Implement `TimelineIngestor` and commit history alongside the account checkpoint. See Core’s recovery migrations and migration regression tests for upgrades from both the old platform and retired curated schema.
