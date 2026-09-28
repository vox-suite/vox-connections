# Publish and install a connector

The same Rust `PackageRegistry` powers independent hosts and Core's HTTP API.
Adding an MCP connector needs a manifest, review evidence and publication. No
Core router, provider implementation or Web catalog entry is required for a
server that already implements the supported MCP/OAuth contracts.

## Publisher and operator

1. Copy `examples/mcp/read-only-manifest.json`. Declare the real endpoint,
   operator, tool names, effects, data recipients and access needs.
2. Run `cargo run --bin vox-connector-check -- path/to/manifest.json`.
   The command validates the declaration and prints its SHA-256 digest; it
   does not prove the provider's behavior.
3. Independently review the provider implementation, declared effects and
   recipients. The authenticated deployment operator publishes:

```http
POST /v1/connector-packages/publish
Authorization: Bearer <Core operator token>
Content-Type: application/json
```

```json
{
  "deployment_id": "<deployment UUID>",
  "version": 1,
  "manifest": "<the manifest JSON object, not a string>",
  "review": {
    "read_effects_verified": true,
    "evidence": {"reviewer": "<operator identity>", "report": "<review evidence reference>"}
  }
}
```

Replace `manifest` with the complete object. The published result returns
`version`, `digest` and `manifest`. The digest covers the canonical typed
manifest bytes. Publication rejects a different declaration for an existing
(deployment, external key, version), including withdrawn versions. Identical
publication is idempotent and preserves the original review and withdrawal.
Neither a developer's local check nor an OAuth callback can attest conformance.

Only a nonempty, exclusively read declaration with explicit operator effect
verification and evidence can inherit reviewed read conformance at install.
Consequential declarations remain pending and disabled until the operator's
behavioral conformance and enablement process completes. Do not claim that a
server-supplied tool annotation proves an effect or a provider outcome.
OAuth providers requiring a preregistered client still need deployment-owned
client configuration and an approved canonical callback URI; secrets are never
part of a package.

## Consuming host

The Web catalog consumes the same routes as any signed, registered host:

- `POST /v1/connector-packages/list` with `host_context` returns the latest
  enabled version of up to 100 packages in that context's deployment.
- `POST /v1/connector-packages/install` with `host_context`, `external_key`,
  `version` and `digest` installs exactly the declaration the user selected.
  A manifest-only connector appears automatically, using initials when no
  optional Web brand asset exists.
- One Connect click installs and starts provider OAuth. Returning from OAuth
  links the account. The user then chooses agent access and capabilities;
  installation and account linking never create an agent grant.
- Discover effective grants and enabled skills for the selected agent, load
  skill content through the signed skill-loading route, and invoke granted
  reads or proposal-bound consequential executions through the existing
  [Core connection contract](https://github.com/vox-suite/vox-core/blob/main/docs/connections.md).
  Skills cannot provide credentials or action authority.

Installation serializes retries across processes with a PostgreSQL transaction
lock scoped to the user context. Extension creation, reviewed read activation
and package binding commit together using one pool connection. A changed
existing declaration conflicts; the install route never silently replaces an
endpoint or expands old grants. Material upgrades currently require the
explicit extension update/reconsent/reconnect/regrant journey.

## Withdrawal and limits

An authenticated operator calls `POST /v1/connector-packages/withdraw` with
`deployment_id`, `external_key` and `version`. Withdrawal hides the package,
blocks new installs, disables bound extensions, clears platform credentials
and pending OAuth sessions, and revokes their accounts and agent grants in one
transaction. Declarations and evidence remain readable. The MCP dispatch path
also rejects withdrawn or mismatched package declarations. In-flight provider
requests cannot be undone by deleting local credentials; provider-side
revocation and consequential outcome reconciliation remain separate gates.

This package contains a reviewed integration declaration. Optional bundled
skill references, automatic material upgrades, conversation-level agent
consumption and independent provider certification are not implemented by
this publish/install API. Standalone skill authoring, pinning, per-agent
loading and disablement remain available through `SkillService`.

## Verification

Apply the independent-host schema, `schema/connectors.sql` and
`schema/packages.sql` to an isolated database, then run:

```sh
TEST_DATABASE_URL=postgres://... cargo test --test postgres_integration -- --ignored
```

The test covers immutable publication, digest mismatch, deployment isolation,
concurrent idempotent installation with a one-connection pool, no implicit
grants, and withdrawal with credential/account revocation. Web browser tests
cover a connector with no compiled-in brand, one-click install, mocked OAuth
return and the agent-access handoff; they are not live-provider certification.
