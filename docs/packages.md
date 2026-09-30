# Connector package authoring and installation

A supported remote MCP connector needs one `vox-package.json` and optional `skills/<key>/SKILL.md`. Core and Web read the reviewed package dynamically; no provider-specific code change is needed. Installation never grants a connection, tool, or action approval.

```sh
cargo run --bin vox -- package init ./my-integration
# Edit the real endpoint, operator, auth/custody profile, tools, input schemas,
# effects, recipients, supported regions, and any pinned skill digests.
cargo run --bin vox -- package check ./my-integration
cargo run --bin vox -- package test ./my-integration --sandbox
```

`package check` validates the full package and referenced local skills and prints the canonical digest. `package test` uses a local/test MCP endpoint and checks negotiated protocol, reported tool names and exact input schemas. Set `VOX_TEST_ACCESS_TOKEN` if the test server requires a credential. It does **not** certify provider effects, OAuth, production availability or consequential outcomes.

The deployment operator independently reviews provider behavior and records `review.json`. The review must bind the exact package version and digest printed by `package check`, the negotiated protocol, verified live inventory, behavior certification, and the SHA-256 digest of a separately retained test report. For example (replace every value with real evidence):

```json
{
  "schema_version": 1,
  "package_version": 1,
  "package_digest": "<64 lowercase hex characters from package check>",
  "protocol_version": "2025-11-25",
  "live_inventory_verified": true,
  "behavior_certified": true,
  "read_effects_verified": true,
  "evidence": {
    "reviewer": "<independent operator>",
    "report_digest": "<64 lowercase hex characters of the retained report>"
  }
}
```

These fields are an operator attestation; Vox cannot infer real-world effects from MCP annotations or a self-reported tool result. Do not set a passing flag without the corresponding independent report and live evidence. With `VOX_OPERATOR_TOKEN` in the environment, `cargo run --bin vox -- package publish ./my-integration https://core.example <deployment-uuid>` publishes an immutable version through Core's operator route. Publication rejects changed bytes for an existing version and preserves withdrawals. Bundled skill references must already identify published curated, active skill versions with matching digests; publish those through the operator skill route before the package. Material package version changes conflict with existing installations until an explicit update/reconsent/reconnect journey clears old authority.

A host lists `POST /v1/connector-packages/list` and presents the exact version/digest, assistant, capabilities and pinned guidance for consent. It sends one signed `POST /v1/connector-packages/setup` request:

```json
{
  "host_context": {"host_user_id": "<authenticated host user>", "organization_external_key": null},
  "setup": {
    "external_key": "<reviewed package key>", "version": 1, "digest": "<reviewed digest>",
    "redirect_uri": "<registered host callback>",
    "consent": {
      "agent_external_key": "<owned assistant key>", "agent_instruction_version": 1,
      "capability_external_keys": ["<chosen reviewed tool>"], "enable_bundled_skills": true
    }
  }
}
```

Use `consent: null` for an account-only installation. Personal Assistant may be visibly preselected, but the host must present the choices before the user confirms. Consequential capabilities must not be silently preselected. The reply has `setup_id`, `extension_id`, `external_key`, `state`, and optional `authorization_url`. `authorize` means navigate to the provider. The authenticated callback sends state/code/optional issuer to `POST /v1/connector-packages/setup/callback`; the server binds it to the durable consent. `complete` records that this setup was applied, not a permanent promise of current access. `needs_review` preserves the linked account and applies no new authority; start a fresh setup after reviewing the changed state. Manual MCP authorization without package consent returns `account_linked` and a null setup ID; no assistant access is inferred.

Setup expires after 20 minutes. Package withdrawal, changed assistant instruction version, changed declared/observed schema, revoked grants, disabled guidance and unavailable policy invalidate pending enablement. All chosen grants and guidance enablements commit together. A verified OAuth-session marker commits with account credentials; callback retry can recover after a crash without reusing the provider code. Retrying a completed setup never restores subsequently revoked access. Sign-in cancellation grants nothing. If a process dies after consuming a code but before verified credentials commit, start provider sign-in again; a consumed code is not proof of connectivity.

The lower-level install/authorize/connect-public routes remain useful for independently added MCP servers and hosts that manage each explicit step. Installation alone carries no agent authority. MCP tool dispatch rechecks current package, connection, declared schema, reported schema, and grant. Consequential dispatch additionally requires the authenticated exact approval and a durable execution. A provider response alone is an unknown outcome until independently verified.

For isolated database verification apply `examples/independent_host_schema.sql`, `schema/connectors.sql`, `schema/packages.sql`, `schema/skill-content.sql`, `schema/public-mcp.sql`, `schema/setup.sql`, and `schema/discovery.sql` in that order, then run `TEST_DATABASE_URL=postgres://... cargo test --test postgres_integration -- --ignored` and `cargo test --test setup_integration -- --ignored`.


## Dynamic discovery

`CapabilityDiscovery::search(scope, agent, query, offset)` searches PostgreSQL GIN indexes over compact reviewed tool metadata and immutable skill-version metadata. It returns at most ten current permitted results and a next offset; it never contacts a provider or returns schemas, credentials or instruction bodies. Package publication/withdrawal, current owned-agent/template policy, grants, observed schema matching and pinned skill enablement are checked at query time. An expiring account with a refresh token may appear as `requires_refresh`; discovery is not proof that refresh or execution will succeed.

The index derives from the existing manifest and skill versions; connector authors add no new registration file or provider-specific code. `ConnectedAppsService::tool_for_agent` checks the exact grant before refreshing only that connection, then loads the single reviewed schema with current authority checks. The full inventory route remains available for host account management. Core's model-facing library uses indexed search and targeted loading/proposals. Large-installation latency and retrieval quality still require retained benchmark evidence.
