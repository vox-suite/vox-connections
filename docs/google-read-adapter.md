# Google Calendar and Drive read candidates

The `vox-google-read-mcp` binary implements two actual Google REST reads through the same reviewed package, OAuth, connection, selected-agent grant and MCP invocation contracts. No Core or Web provider registration is required. These candidate implementations are covered by local mocks; they are not published packages or live-certified Google connections.

- `/calendar/mcp` serves `google_calendar.list_events`: primary-calendar events for an explicit RFC3339 range of at most 31 days, with at most 100 events per requested page. It reads event ID, summary, start/end and status. It does not read other calendars, attendees or descriptions.
- `/drive/mcp` serves `google_drive.list_files`: nontrashed file metadata in the user's corpus, optionally filtered by a literal name substring, with at most 100 files per page. It returns ID, name, MIME type, modification time and browser link. It does not download file content or write files.

Page tokens remain explicit. `complete: false` and `incomplete_search` prevent a single page from being presented as a full inventory or exact total. Follow the next token using the same read arguments before claiming completeness.

## Operator setup

Run `cargo run --bin vox-google-read-mcp`; it binds to `127.0.0.1:3004` by default. `VOX_GOOGLE_READ_BIND_ADDRESS` changes only the listener. Terminate public HTTPS at your deployment proxy and route the two MCP paths to this process. The production upstream is fixed to `https://www.googleapis.com`; requests cannot select another URL or method. There is no production upstream override.

Enable the Calendar and Drive APIs in your Google Cloud project and register a Web OAuth client with the actual host callback URI. Configure `VOX_MCP_OAUTH_CLIENTS` in the Connections credential store using **exact endpoint keys** so scopes remain separate on the same adapter host. Exact endpoint entries take precedence over the existing host-key mapping. Replace all placeholders with your real deployment values:

```json
{
  "https://your-adapter.example/calendar/mcp": {
    "issuer": "https://accounts.google.com",
    "client_id": "<registered Google client ID>",
    "client_secret": "<protected Google client secret>",
    "token_endpoint_auth_method": "client_secret_post",
    "scopes": ["https://www.googleapis.com/auth/calendar.events.readonly"],
    "send_resource": false,
    "authorize_params": {"access_type": "offline", "prompt": "consent"}
  },
  "https://your-adapter.example/drive/mcp": {
    "issuer": "https://accounts.google.com",
    "client_id": "<registered Google client ID>",
    "client_secret": "<protected Google client secret>",
    "token_endpoint_auth_method": "client_secret_post",
    "scopes": ["https://www.googleapis.com/auth/drive.metadata.readonly"],
    "send_resource": false,
    "authorize_params": {"access_type": "offline", "prompt": "consent"}
  }
}
```

Google's [web-server OAuth contract](https://developers.google.com/identity/protocols/oauth2/web-server) describes offline access, registered redirect URIs, code exchange, scope responses and refresh. Google may combine previously granted authorization for an API project; Vox's explicit capability grants still remain per connection and agent. Tokens remain encrypted in Connections, with the existing state, PKCE, callback recovery and refresh serialization. The adapter receives each access token only for the request and forwards it to Google; it stores no credentials and returns no token or provider error body. Its operator must be the deployment's reviewed credential processor, disclosed by the package. Do not point these platform-held-token packages at an unreviewed third party.

The [Calendar events scope](https://developers.google.com/workspace/calendar/api/auth) permits event reads. [Drive metadata scope](https://developers.google.com/workspace/drive/api/guides/file-metadata) is restricted and excludes file content. Google verification, any applicable assessment, consent configuration, registered callbacks and live account tests remain external release gates. Do not widen Drive to full file access to avoid verification.

`tools/list` verifies account access using a minimal Google GET returning one ID before exposing the schema. This rejects revoked tokens and reduced scopes during connection completion and before dispatch. A normal invocation therefore performs one small access check plus the requested read. The adapter does not cache responses, retry rate limits or switch accounts. Upstream calls have a 15-second deadline, no redirects and a 2 MiB response ceiling. Connections applies its shared 30-second MCP negotiation/inventory/dispatch deadline. All provider errors are sanitized; 401, 403 and 429 remain distinct failures.

Update the two templates under `examples/packages/google-calendar-reader` and `examples/packages/google-drive-metadata-reader` with your HTTPS endpoint and actual operator/custody disclosures before running the package author workflow. Neither template contains passing `review.json` or `report.json`. Schema compatibility is tested against the actual adapter definitions. Review, live evidence, retained report integrity and immutable publication remain required.

## Provider-maintained MCP option

As checked on 2026-10-01, Google's [Calendar MCP](https://developers.google.com/workspace/calendar/api/guides/configure-mcp-server) and [Drive MCP](https://developers.google.com/workspace/drive/api/guides/configure-mcp-server) are in Developer Preview and require Preview Program membership plus enabled MCP services. Prefer those provider-maintained endpoints when the deployment is eligible and their live schemas/effects have been reviewed. Their documented availability does not certify Vox compatibility or make them generally available to ordinary users. This narrow REST fallback supports regular OAuth/API projects without relying on Preview enrollment.

## Reusable-runtime spike: Activepieces

The current [Activepieces license](https://github.com/activepieces/activepieces/blob/main/LICENSE) licenses ordinary code under MIT while excluding enterprise directories. Its [MCP documentation](https://www.activepieces.com/docs/mcp/overview) exposes project-scoped operations, discovers actions through a broad tool surface and retains connection credentials inside Activepieces. That creates another credential processor and a project-to-Vox-user-context isolation boundary that must be proven, rather than inheriting Vox agent grants.

For these two bounded reads, the fixed REST fallback has no additional database, queue, workflow engine or credential store. Defer runtime adoption until an isolated trial proves exact action/schema pinning, per-user project isolation, revocation, minimized effects and the intended self-hosted license coverage. This document is a source-based comparison, not a running Activepieces trial: controlled writes, external custody isolation, restart behavior, memory/CPU footprint and operational load remain unmeasured. Advertised connector counts are not certified Vox coverage.

## Verification

`cargo test --lib google_reads` uses mock Google REST and real local MCP sessions. `TEST_DATABASE_URL=postgres://... cargo test --test google_read_integration -- --ignored` runs OAuth setup, PKCE, exact scopes, encrypted credentials, refresh, foreign-context/specialist denial, granted reads, revocation, callback retry and disconnection against an isolated independent host schema. CI applies the standalone schemas and runs this database regression. These fixtures prove implementation behavior; they do not certify Google availability or live consent.
