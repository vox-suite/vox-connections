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

A host lists `POST /v1/connector-packages/list`, installs the selected version/digest with `POST /v1/connector-packages/install`, then follows the declared auth mode: public MCP uses `connect-public`, OAuth uses a signed authorization callback. The user grants selected capabilities to a selected agent. MCP tool dispatch rechecks current package, connection, declared schema, reported schema, and grant. Consequential dispatch additionally requires the authenticated exact approval and a durable execution. A provider response alone is an unknown outcome until independently verified.

For isolated database verification apply `examples/independent_host_schema.sql`, `schema/connectors.sql`, `schema/packages.sql`, `schema/skill-content.sql`, and `schema/public-mcp.sql` in that order, then run `TEST_DATABASE_URL=postgres://... cargo test --test postgres_integration -- --ignored`.
