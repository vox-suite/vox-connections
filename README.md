# Vox Connections

Reusable Rust connector platform for Vox Core and independent host applications. The crate owns integration declarations and discovery, account connection records, capability grants, remote extension packages, protocol adapters, connected-app OAuth and MCP sessions, declarative skill packages, provider clients, and a conformance fixture. It does not import `vox-core` or a host UI.

Contains:
- `connections` — OAuth/authorization connection lifecycle (initiate, callback, list, disconnect).
- `capability_grants` — per-agent capability grant/revoke and effective-grant lookup.
- `integration_registry` — capability declarations and per-deployment discovery.
- `providers::{amazon, playstation, uber, zomato}` — provider clients and workers built on top of the above:
  - `amazon` — Creators API catalog discovery and labelled purchase handoff.
  - `playstation` — PlayStation 5 / PSN activity extraction and timeline sync worker that records gaming sessions as Spans.
  - `uber` — Connected read trip history and estimate options.
  - `zomato` — Restaurant search and labelled order handoff.

| Module | Responsibility |
| --- | --- |
| `integration_registry` | Versioned, protocol-neutral integration declarations and deployment discovery |
| `connections`, `capability_grants` | Connection lifecycle and scoped capability grants |
| `remote_extensions` | User-added remote integration manifests, lifecycle, consent, conformance status, endpoint authorization |
| `remote_extensions::adapters` | Direct and MCP transports, DNS and address checks, context minimization, integrity, response redaction, per-protocol switches |
| `connected_apps` | Provider OAuth, encrypted credentials, MCP sessions, tool inventory, classification and selection |
| `skills` | Versioned declarative packages, installation, per-agent enablement and bounded loading |
| `providers` | Amazon, Uber, Zomato and Expedia provider clients and data contracts |
| `conformance` | Versioned fixtures and a reference implementation for cross-host behavioral checks |

Provider adapters are separate modules. Hosts enable only the integrations and capabilities they offer; installing an extension or skill grants no account access. Remote integrations are protocol-neutral declarations. MCP is one transport, and neither MCP nor a skill bypasses grants or approval policy.

## Host boundary

A host resolves and authenticates its own user context, then implements `identity::RequestScope` to supply the minimal `RequestContext` (context ID, user ID, deployment ID). Services take `sqlx::PgPool`, not Vox Core's database wrapper. `ConnectedAppsService::from_options` accepts deployment-owned credential and OAuth settings through `ConnectedAppsOptions`; it never reads Vox environment variables. The host can construct and use the services without a Vox Core dependency; `tests/host_boundary.rs` compiles an independent host and checks the shared conformance and redaction contracts.

Vox Core owns host trust, identity resolution, agent conversations, action proposals, approvals, execution, policy and audit. Its HTTP routes and conversation tools call this crate. In particular, Core's Expedia lodging service retains proposal and execution orchestration, while the Expedia provider transport and data contracts live here. Core's thin re-exports preserve existing Rust API paths while its callers migrate.

The [database contract](schema/README.md) includes a standalone reference schema for independent hosts. Vox Core keeps its incremental migrations. Schema compatibility must be checked when upgrading either repository.

Authoring guides: [MCP integration](docs/mcp-authoring.md) and [declarative skills](docs/declarative-skills.md). Their local probes and examples live in `tools/` and `examples/`.

## Build and verify

```sh
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

Integration behavior that reads or writes the database requires an isolated PostgreSQL instance. The included tests run without a database or provider credentials. When changing the shared crate, also compile and test the consuming Vox Core revision before release.
