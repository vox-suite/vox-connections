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

Use [the package author workflow](packages.md) for installable connectors. The package declares exact tool names and JSON input schemas, effect and recipient disclosures, auth mode, protocol, and optional bundled skills. The local `mcp_probe.py` above is a diagnostic smoke test only. Platform operator review, user authorization and agent grants remain separate from protocol discovery.

## Configure a provider OAuth client

For a provider without dynamic registration, the host supplies an endpoint-host map in `ConnectedAppsOptions.oauth_clients` (Core uses `VOX_MCP_OAUTH_CLIENTS`). Keep this JSON in the deployment secret store, separate from the package. A confidential client defaults to `client_secret_basic`; set `token_endpoint_auth_method` to `client_secret_post` when the provider requires credentials in the token form. The setting applies to code exchange and refresh. Unknown methods and a secret method without a secret are rejected. Public clients omit both secret and method.

GitHub Apps use fine-grained app permissions, not OAuth scopes. Leave `scopes` empty and configure Post authentication, following [GitHub's token exchange contract](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app). Provider-advertised supported scopes never become requested authority automatically. Linking remains separate from package conformance and selected-agent access.
