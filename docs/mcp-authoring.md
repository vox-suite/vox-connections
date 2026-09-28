# Add an MCP read tool: current developer path

This is a local authoring path for the shared connector adapter's current `tools/call` subset. OAuth linking, dynamic tool discovery, context-owned connections, explicit agent grants, and a signed host read/write interface now exist. For the reviewed catalog install path, use [publish, discover, install](packages.md). This advanced endpoint-registration path still requires operator onboarding: authors must supply reviewed tool declarations, an agent must request the capability (or `*`) before a user can grant it, and operator conformance is not self-service. See the [Vox Core connection contract](https://github.com/vox-suite/vox-core/blob/main/docs/connections.md).

## Run a deterministic local fixture

In two terminals from `vox-connections`:

```sh
python3 examples/mcp/minimal_server.py
```

```sh
python3 tools/mcp_probe.py http://127.0.0.1:8765/mcp --tool echo.read --arguments '{"text":"hello"}' --local-test
```

The probe checks the JSON-RPC response id, `2.0` envelope, and MCP `content` or `structuredContent` result. It sends `2026-07-28` MCP headers and a bounded request. It is a protocol smoke test, not an operator conformance certificate or a security audit. The OAuth client first probes `2026-07-28` and falls back to a 2025 session only on explicit unsupported-method/version responses. Production traffic requires public HTTPS, a fresh DNS check with address pinning, no redirects, and bounded responses.

## Declare a remote integration

Start with [the read-only manifest](../examples/mcp/read-only-manifest.json),
then run the same declaration validation used by installation:

```sh
cargo run --quiet --bin vox-connector-check -- examples/mcp/read-only-manifest.json
```

The command prints a stable SHA-256 digest for the canonical JSON declaration.
It checks identity, endpoint shape, distinct capability keys, effects, and
recipient disclosures without contacting the provider. CI can run it before
publishing a manifest. It is not an operator review or proof that the server
actually implements the declared tools.

The registered host calls `POST /v1/remote-extensions` with a fresh host assertion and a body like:

```json
{
  "host_context": {"host_user_id": "my-user-123", "organization_external_key": null},
  "extension": {
    "external_key": "my-read-server",
    "display_name": "My read server",
    "protocol": "mcp",
    "endpoint_url": "https://mcp.example.com/mcp",
    "operator": {
      "operator_id": "my-company",
      "operator_name": "My Company",
      "support_email": "support@example.com",
      "terms_url": "https://example.com/terms"
    },
    "capabilities": [{
      "external_key": "echo.read",
      "display_name": "Read echoed text",
      "effect": "read",
      "consequential": false,
      "data_recipients": ["My Company"],
      "access_needs": ["text"]
    }]
  }
}
```

Installation creates a declaration in the caller's user context. It does not connect an external account, enable the operator, grant an agent access, or approve a write. A Core operator must independently attest conformance and enable the extension. Hosts cannot attest their own conformance or operator status. Use the operator-reviewed package publication path to make verified read declarations available in the deployment catalog. Independent behavioral conformance and production-provider validation remain release gates.
