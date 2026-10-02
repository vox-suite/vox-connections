# Activepieces Calendar runtime evaluation

Status: **local credential-free spike; do not deploy or publish as a Vox integration**.
Date: 2026-10-02. Owner: Vox Connections; intended W11 evidence under the shared personal-assistant plan.

## Decision

Keep the existing narrow Google REST/MCP adapter as the first delivery path. Do not add Activepieces' full workflow server to Vox's production topology for Calendar/Drive. Reusing selected MIT pieces may become useful for providers with substantially more bespoke actions, but it still needs a sandboxed executor, pinned reviewed schemas/effects, deadline/response limits and a single existing Vox custody/approval boundary. It does not remove Google OAuth registration or live certification.

The actual published Calendar piece executed successfully locally. It reduced provider request-building work for a create event but did **not** simplify the complete safe connector: Vox still needs auth, account selection, current grants, proposals, single dispatch, outcome reconciliation, pagination and limits. Adopting the whole server adds a second connection/tenant/workflow model and operating surface.

## Reproduce

Prerequisite: Node.js compatible with Nock 14 (tested Node 24). No Google account, token, browser or server required.

```sh
cd experiments/activepieces-calendar
npm ci --ignore-scripts
npm test
```

The only dependencies are pinned published `@activepieces/piece-google-calendar` **0.12.0** and dev-only Nock **14.0.11**. `package-lock.json` binds registry integrity. `adapter.cjs` invokes the real bundled action functions, not a rewritten imitation. Nock disables outbound networking and intercepts Google HTTPS with synthetic responses and a fake non-credential token.

The upstream checkout and downloaded bundle used for inspection are not vendored. The experiment contains only a small adapter, tests, package manifest and lockfile; the published piece is installed from npm.

### Retained local results

Seven tests passed (0 failures):

1. Real piece reads the selected Calendar with date range and recurrence expansion; `nextPageToken` becomes an explicit `incomplete` flag, never a false total count.
2. Foreign context/agent/account, revoked access, version drift and unreviewed fields dispatch nothing.
3. Real piece creates exactly the reviewed title/times/recipient and `sendUpdates=none`; guest permissions are false and no Meet link is generated. Reusing the proposal is denied.
4. Unapproved proposal, changed recipient and missing write authorization dispatch nothing.
5. A synthetic 503 after write dispatch yields `unknown`. The same proposal cannot dispatch again; one POST was observed.
6. Inherited property names such as `constructor` reject before authority is claimed.
7. Caller mutation while the claim is paused cannot change the snapshotted approved recipient, title or notification policy sent to Google.

The authority port in the test is an **in-memory synthetic fixture**, not the production Vox authentication or durable approval implementation. It demonstrates the adapter seam and required invocation binding; it proves neither distributed replay safety nor authenticated user decisions. No provider side effect occurred. A mock 503 is not live unknown-outcome reconciliation evidence.

The whole local suite took about 0.35 seconds including npm/node startup. This is mock CPU execution timing, **not provider latency, throughput or a production capacity envelope**. The downloaded published bundle extraction occupies approximately 996 KiB. Full-service container RAM/CPU/throughput was not measured because no Activepieces deployment was created.

## Evaluated contract

Only two reviewed actions are exposed: `list_events` → `google_calendar_list_events` (read), `create_event` → `google_calendar_create_event` (write). Dynamic action names, custom API calls, service-account credentials, arbitrary URLs and upstream property default expansion are unavailable. Calendar, zoned start/end times and notification policy are explicit; no guessed time/end/account default is accepted.

Production would supply the authority port from **existing Vox Connections/Core**. Its claim input binds user context, executing agent, connection, capability, pinned piece version and exact argument digest. For writes it must additionally verify the authenticated proposal decision and durable single-dispatch claim; agent-provided approval flags cannot satisfy it. The port would provide only the current short-lived access token after current grant/service authorization checks. Runtime code must never receive refresh tokens, OAuth client secrets or host signing credentials. An exception after write dispatch is outcome unknown; approval cannot be blindly retried.

This local wrapper is intentionally **not production-ready**: it executes trusted piece code in process, calls `_actions` (an internal shape), lacks hard transport cancellation/response streaming limits, durable claims and provider reconciliation. A separate bounded subprocess with restricted egress would be required before untrusted/general pieces could run. `Promise.race` alone cannot stop an in-flight side effect and would be an inadequate substitute.

## Findings against current primary source

Source inspected at Activepieces commit `e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3`; package 0.12.0 separately inspected/executed. Main branch and published bundles are separate artifacts: no claim they are byte-for-byte the same build.

| Concern | Evidence | Consequence for Vox |
| --- | --- | --- |
| Licensing | Root LICENSE makes source outside `packages/ee/` and `packages/server/api/src/app/ee` MIT, preserves third-party licenses | A Calendar piece is outside EE; retain upstream and bundled third-party notices. Enterprise embedding/access features cannot be assumed MIT or necessary. This is source classification, not legal sign-off. |
| Footprint | Official Compose 0.92.0 defines app, worker with 5 replicas, PostgreSQL/pgvector and Redis | Full runtime is substantially more operational surface than two direct adapter capabilities. Resource sizing is unmeasured. |
| OAuth compatibility | Calendar auth includes `calendar.events`, `calendar.readonly`, `email`, PKCE and optional service account | Default piece auth overrequests write authorization for a read-only install. Vox must retain granular scopes and its own OAuth flow; disable service-account/domain-wide delegation path in ordinary catalog. |
| Credential custody | AP persists encrypted connection values; cloud OAuth refresh sends to `secrets.activepieces.com/refresh`; own-client refresh uses provider token endpoint | Do not silently duplicate secrets or select broker custody. Cloud mode requires explicit operator/custody consent; local piece execution can use a short-lived token only. |
| Tenant/account isolation | `getOne` filters `platformId`, `projectIds`, `externalId`; refresh uses distributed lock and project lookup | AP project mapping is not Vox's host/deployment/user context or agent grants. A separate explicit context mapping and cross-tenant proof would be required. No AP live tenant security claim is made. |
| Rotating refresh | AP has distributed lock around refresh/persistence | Reusing AP's refresh while Vox owns refresh would create two authorities. Keep one custody/refresh path; no duplicate refresh tokens. |
| Inventory/schema stability | Piece 0.12.0 supports minimum runtime 0.88.2; read/write props differ and include dynamic calendar dropdowns | Pin package integrity plus reviewed static Vox schemas/effects. Dynamic dropdown evaluation must not fetch account data on metadata discovery. Internal `_actions` access is a compatibility risk. |
| Read completeness | `runGetEvents` sends one list request and exposes no pageToken/maxResults property | Current piece cannot answer arbitrary complete calendar counts through this action without extra pagination adaptation. The spike honestly signals incomplete. |
| Deadline/response bounds | Read action passes neither timeout nor explicit response-size cap; common fetch client aborts only for supplied timeout | Existing Vox bounded direct adapter is preferable. A process deadline must prevent runaway execution and preserve unknown write outcomes. |
| Write replay | Create action says `idempotent:false`; request does not supply a deterministic event ID | Never use workflow retry on an uncertain create. Provider GET reconciliation requires deterministic reference strategy or user-confirmed recovery; unsupported here. |
| Engine retries | `runWithExponentialBackoff` retries any failed step when enabled, bounded by configured attempts | It is not proposal/effect-aware and cannot substitute for Vox execution policy. Workflow retry must be disabled for this non-idempotent write. |

## Source links

- [License](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/LICENSE)
- [Official Compose](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/docker-compose.yml)
- [Calendar OAuth](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/pieces/community/google-calendar/src/lib/auth.ts)
- [List implementation](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/pieces/community/google-calendar/src/lib/actions/get-events.ts)
- [Create implementation](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/pieces/community/google-calendar/src/lib/actions/create-event.ts)
- [Create effect metadata](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/pieces/community/google-calendar/src/lib/actions/ai-create-event.ts)
- [Engine retries](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/server/engine/src/lib/helper/error-handling.ts)
- [Custody encryption](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/server/api/src/app/helper/encryption.ts)
- [Cloud refresh custody](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/server/api/src/app/app-connection/app-connection-service/oauth2/services/cloud-oauth2-service.ts)
- [Connection isolation](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/server/api/src/app/app-connection/app-connection-service/app-connection-service.ts)
- [Refresh locking](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/server/api/src/app/app-connection/app-connection-service/app-connection.handler.ts)
- [HTTP deadline behavior](https://github.com/activepieces/activepieces/blob/e3e1cbebb6dafbf2bb0da35effc6dfd58b8c13e3/packages/pieces/common/src/lib/http/core/fetch-http-client.ts)

## Still required for W11

Real Google OAuth app/scopes, test-account read/revoke/refresh lifecycle, controlled write and lost-response reconciliation, current Vox governance integration and live evidence. This spike supports a runtime decision, not publication, universal provider support or W11 completion.
