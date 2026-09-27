# Add an MCP read tool: current developer path

This is a local authoring path for the shared connector adapter's current `tools/call` subset. It is **not** a complete connect-and-use quickstart: production provider authorization, dynamic tool discovery, agent registration, and self-service operator conformance are still missing. See the [Vox Core connection API](https://github.com/vox-suite/vox-core/blob/main/docs/connections.md).

## Run a deterministic local fixture

In two terminals from `vox-connections`:

```sh
python3 examples/mcp/minimal_server.py
```

```sh
python3 tools/mcp_probe.py http://127.0.0.1:8765/mcp --tool echo.read --arguments '{"text":"hello"}' --local-test
```

The probe checks the JSON-RPC response id, `2.0` envelope, and MCP `content` or `structuredContent` result. It sends the currently pinned `2026-07-28` MCP headers and a bounded request. It is a protocol smoke test, not an operator conformance certificate or a security audit. The production connector adapter only dispatches to public HTTPS addresses, does a fresh DNS check, pins the permitted address, refuses redirects, and limits the request and response to 2 MiB.

## Declare a remote integration

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

Installation creates a declaration in the caller's user context. It does not connect an external account, enable the operator, grant an agent access, or approve a write. A Core operator must independently attest conformance and enable the extension. Hosts cannot attest their own conformance or operator status. There is currently no public self-service process to obtain that attestation; production onboarding remains blocked until the release gates in the integration plan are implemented.
