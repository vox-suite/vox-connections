# Vox Connections

Reusable Rust connector platform for Vox Core and independent host applications. The crate owns integration declarations and discovery, account connection records, capability grants, remote extension packages, protocol adapters, MCP account OAuth and tool discovery, declarative skill packages, provider clients, and a conformance fixture. It does not import `vox-core` or a host UI.

Contains:
- `connections` — OAuth/authorization connection lifecycle (initiate, callback, list, disconnect).
- `capability_grants` — per-agent capability grant/revoke and effective-grant lookup.
- `integration_registry` — capability declarations and per-deployment discovery.
- `providers::{amazon, playstation, uber, zomato}` — provider clients and workers built on top of the above:
  - `amazon` — Creators API catalog discovery and labelled purchase handoff.
  - `playstation` — PSN account verification, encrypted credential refresh, bounded game-history reads and observed playtime deltas. Core owns scheduling and span ingestion.
  - `uber` — Connected read trip history and estimate options.
  - `zomato` — Restaurant search and labelled order handoff.

| Module | Responsibility |
| --- | --- |
| `integration_registry` | Versioned, protocol-neutral integration declarations and deployment discovery |
| `connections`, `capability_grants` | Context-owned `external_connections` records, revocation, and scoped grants; generic provider authorization is not yet implemented |
| `packages` | Deployment-reviewed immutable manifests; atomic, digest-bound installation and withdrawal |
| `remote_extensions` | User-added remote integration manifests, lifecycle, consent, conformance status, endpoint authorization |
| `remote_extensions::adapters` | Direct and MCP transports, DNS and address checks, context minimization, integrity, response redaction, per-protocol switches |
| `connected_apps` | Provider OAuth, encrypted credentials, and MCP tool discovery |
| `skills` | Versioned declarative packages, installation, per-agent enablement and bounded loading |
| `providers` | Amazon, PlayStation, Uber, Zomato and Expedia provider clients and data contracts |
| `conformance` | Versioned fixtures and a reference implementation for cross-host behavioral checks |

Provider adapters are separate modules. Hosts enable only the integrations and capabilities they offer; installing an extension or skill grants no account access. Remote integrations are protocol-neutral declarations. MCP is one transport, and neither MCP nor a skill bypasses grants or approval policy.

## Host boundary

A host resolves and authenticates its own user context, then implements `identity::RequestScope` to supply the minimal `RequestContext` (context ID, user ID, deployment ID). Services take `sqlx::PgPool`, not Vox Core's database wrapper. `ConnectedAppsService::from_options` accepts deployment-owned credential and OAuth settings through `ConnectedAppsOptions`; it never reads Vox environment variables. The host can construct and use the services without a Vox Core dependency; `tests/host_boundary.rs` compiles an independent host and checks the shared conformance and redaction contracts.

Vox Core owns host trust, identity resolution, agent conversations, action proposals, approvals, execution, policy and audit. Its HTTP routes call this crate; signed host access to OAuth-linked tools uses governed grants and exact consequential approvals. Conversation-level selected-agent consumption remains a release gate. In particular, Core's Expedia lodging service retains proposal and execution orchestration, while the Expedia provider transport and data contracts live here. Core calls the shared provider services through its host orchestration boundary.

The [database contract](schema/README.md) includes a standalone reference schema for independent hosts. Vox Core keeps its incremental migrations. Schema compatibility must be checked when upgrading either repository.

Package onboarding: [publish, discover, install](docs/packages.md). New MCP connectors require a manifest and operator publication; Core and Web code changes are unnecessary.

Authoring guides: [MCP integration](docs/mcp-authoring.md) and [declarative skills](docs/declarative-skills.md). Their local probes and examples live in `tools/` and `examples/`.

The [architecture review (2026-09-27)](docs/architecture-review-2026-09-27.md) records the current release gaps and the target connector authoring and installation path.

## PlayStation accounts

The PlayStation provider connects a PSN account to read PS5 and PS4 game activity through a community integration. It verifies the account against Sony, exchanges the NPSSO session token without storing it, and encrypts access and refresh tokens. Game reads provide cumulative playtime and provider first/last-played timestamps; they do not provide exact session boundaries.

In Vox, Core consumes this crate directly. Core API handles linking and manual refresh, and Core Worker captures new playtime once a day. This flow does not require a separate `vox-connections-service` deployment. The standalone service endpoints below do not expose Core's PlayStation linking or span-capture routes.

Independent hosts must apply [`schema/playstation.sql`](schema/playstation.sql) after the connector schema and supply a credential encryption key when constructing `PlayStationAccounts`. Vox Core supplies its own migration and `VOX_CREDENTIAL_KEY`. Disconnecting deletes stored PlayStation credentials; linking alone creates no agent grants.

See [Core's PlayStation setup and capture contract](https://github.com/vox-suite/vox-core/blob/main/docs/playstation.md). Publish this crate first, then update Core's dependency lockfile before releasing the consuming services.

## Standalone Service & HMAC Authentication

`vox-connections` can be deployed as an independent microservice daemon (`vox-connections-service`) with zero dependency on `vox-core`.

### Endpoints

- **Health Checks** (unauthenticated):
  - `GET /health/live` — liveness probe
  - `GET /health/ready` — readiness probe (verifies database pool connectivity)
- **Connections & OAuth**:
  - `POST /v1/connections/list` — list active connections for `RequestContext`
  - `POST /v1/connections/{id}/disconnect` — disconnect integration
- **Capability Grants**:
  - `POST /v1/capability-grants` — create scoped capability grant
  - `POST /v1/capability-grants/revoke` — revoke grant
  - `POST /v1/agents/{agent_key}/effective-capability-grants` — lookup effective grants for agent
- **Packages & Setup**:
  - `POST /v1/connector-packages/publish` — publish immutable connector package
  - `POST /v1/connector-packages/list` — list available connector packages
  - `POST /v1/connector-packages/install` — install digest-bound package
  - `POST /v1/connector-packages/withdraw` — withdraw published package
  - `POST /v1/connector-packages/setup` — initiate atomic connector setup
  - `POST /v1/connector-packages/setup/callback` — complete OAuth callback
- **Remote Extensions & Skills**:
  - `POST /v1/remote-extensions/list` — list remote extension manifests
  - `POST /v1/skills/list` — list available skills

### HMAC-SHA256 Authentication Protocol

All `/v1/*` API endpoints require HMAC-SHA256 signature verification.

#### Required Headers

| Header | Description |
| --- | --- |
| `x-vox-signature` | Hex-encoded HMAC-SHA256 string |
| `x-vox-timestamp` | Integer Unix epoch seconds |
| `x-vox-nonce` | Unique random UUID or nonce string |

*(Note: `x-vox-host-signature`, `x-vox-host-timestamp`, and `x-vox-host-nonce` are supported as alternate headers).*

#### Canonical Message Format

```
vox-hmac-v1:{method}:{path}:{timestamp}:{nonce}:{body_sha256}
```

Where `{body_sha256}` is the hex-encoded SHA-256 hash of the exact request body bytes (empty body defaults to `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`).

#### Security Guarantees

1. **Constant-Time Verification**: Prevents timing attacks via `subtle::ConstantTimeEq`.
2. **Clock Skew Enforcement**: Rejects requests where `abs(now - timestamp) > max_clock_skew_seconds` (default: 300s).
3. **Replay Prevention**: Tracks consumed nonces within the clock skew window; duplicate nonces within the window are rejected with `401 Unauthorized`.
4. **Cache Poisoning Defense**: Nonces are only retained in memory *after* cryptographic signature validation succeeds.

### Client Usage

Consumers use `ConnectionsServiceClient` for strongly-typed, auto-signed communication:

```rust
use vox_connections::service::ConnectionsServiceClient;

let client = ConnectionsServiceClient::new("http://connections:3003", hmac_secret);

// Health
let healthy = client.health_ready().await?;

// Signed API calls
let connections = client.list_connections(&context).await?;
```

## Build and verify

```sh
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

Integration behavior that reads or writes the database requires an isolated PostgreSQL instance. The included tests run without a database or provider credentials. When changing the shared crate, also compile and test the consuming Vox Core revision before release.

