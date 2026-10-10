# Vox Connections architecture review — 2026-09-27

## Verdict

The package has useful foundations but is **not yet a production-ready, one-click connector platform**. The governed integration path models declarations, user-owned connections, grants, conformance and mediated execution. OAuth completion now creates a governed connection, and signed host routes can invoke a selected agent's granted read tool or dispatch a consequential tool after an exact approval. The unsafe conversation tool injection has been deleted. Installing a remote extension or skill is separate from connecting an account and enabling it for an agent. The Web catalog now consumes immutable deployment-reviewed packages with declared capabilities. One Connect action installs the exact selected digest and begins provider OAuth; selected-agent grants remain explicit. Conversation consumption and independent provider certification remain release gates.

This review covers the checked-out `vox-connections`, its consuming `vox-core` and `vox-web` code, the shared context/PRD, and the standalone host fixture. It includes local Rust tests and one lifecycle test against a disposable PostgreSQL instance; it is not a live provider, penetration, or deployment audit.

## Current path through the box

| Stage | Existing implementation | Assessment |
| --- | --- | --- |
| Definition | `integration_registry` stores deployment declarations; `remote_extensions` stores per-context manifests; `connected_apps` stores discovered MCP tools; `skills` stores declarative guidance. | Three overlapping capability catalogs have different lifecycle and trust rules. |
| Install | `RemoteExtensionService::install` records an untrusted remote endpoint; skills pin an exact reviewed version. Web discovers reviewed packages through Core and retains a URL-based pending MCP form. | Atomic digest-bound installation is available; optional bundled skills and automatic material upgrades remain incomplete. |
| Connect | `ConnectedAppsService::begin/complete` performs MCP OAuth, stores encrypted tokens, and creates an `external_connections` record. The generic initiation, callback, and host-asserted authorization stubs were removed. | Authorization joins the governed connection/grant model, but discovered names are usable only when they match reviewed declarations. |
| Agent exposure | `CapabilityGrantService::effective_for_agent` checks context, selected agent, connection, authorization, and declaration. A signed host route invokes only granted declared reads. The conversation request has no selected agent, so it injects no MCP tools. | Host isolation is enforced; product-level agent selection and skill loading remain incomplete. |
| Execution | Core consumes exact user approvals into durable executions before dispatching consequential MCP calls. Ambiguous outcomes remain reconciling. The former direct model-call and inferred-effect path has been deleted. | The approval path is wired, but independent conformance, provider confirmation and reconciliation, and MCP version compatibility remain release gates. |
| Removal/update | Extension versions preserve declaration history. This review changed update/removal to clear stored tokens and OAuth sessions transactionally. | Historical snapshots remain, but in-flight callbacks and provider-side revocation still need verification. |

## Findings, ordered by release risk

### P0 — Agent tool use needs an explicit selected agent and grant

OAuth account status and connection grants are scoped to the authenticated user context. The signed host read route requires a selected agent and checks its effective grant at call time. A PostgreSQL test creates two contexts for the same user and verifies the other context cannot see or read the linked account. The conversation prompt still lacks a selected agent key, so it injects no MCP tools. **Remaining gate:** integrate selected-agent discovery and skill loading with conversation/task entry points. Test two agents sharing one connection and stale tool inventory against a mock server.

### P0 — Consequential action approval and provider outcome need independent proof

The former `confirm_app_action` model tool, pending-action table, heuristic effect classifier, and conversation injection have been removed. A remote MCP proposal must now name a currently declared and reported consequential tool, carry object arguments, and pass operator conformance before approval. The signed host MCP write route starts a durable execution from the exact approved proposal and dispatches only once per idempotency key; unknown outcomes remain reconciling. The approval record carries authenticated host-context evidence. A server-supplied confirmation claim never turns an MCP result into verified success. **Remaining gate:** independently validate declared effects and provider confirmation semantics; run replay, changed-argument, cross-agent, and cross-context attack tests against the full HTTP flow. MCP annotations and names remain untrusted hints.

### P0 — OAuth and endpoint version binding needs a race-proof proof

Before this review, `RemoteExtensionService::update` changed `endpoint_url` while retaining `remote_extension_credentials`, so the old token could be sent to a new endpoint. Updates and removal now clear credentials and pending OAuth sessions. OAuth sessions now persist the initiating endpoint and version; callback completion rejects a mismatch, and credential storage rechecks the current extension under a row lock. The Web consent review displays the current immutable version's endpoint, operator, recipients, access needs, and capabilities; renewing that exact version never restores old grants or credentials. The database test covers stale callbacks after update, stale consent versions, and cross-context denial. **Remaining gate:** stress-test actual concurrent update/removal versus callback and governed calls. Attempt provider-side revocation where supported, with honest disclosure when unavailable.

### P0 — Conformance is currently too easy to self-attest

`ConnectedAppsService::complete` now records OAuth credentials and discovered tools without marking conformance passed or enabling execution. Core's conformance and enablement endpoints require its operator token. The disconnected direct-tool refresh path has been removed; the future governed path must revalidate inventory changes before execution. **Remaining gate:** deliver an independent behavioral conformance workflow for declared effects, recipient disclosure, write safety, idempotency, cancellation, and unknown outcomes. The current sandbox is not an operator certificate.

### P1 — The developer interface is fragmented

A developer has to understand deployment integration declarations, per-user remote manifests, OAuth client configuration, dynamic MCP tool inventory, skill packages, and host-specific catalog entries. A small `vox-connector-check` command now validates the install manifest locally with the same shape checks as the server and emits its canonical digest. A deployment-scoped `PackageRegistry` now provides immutable publication, bounded discovery, atomic digest-bound installation and withdrawal through shared Rust and signed Core interfaces. New MCP manifests appear in Web without Core or Web edits. Independent behavioral conformance and optional bundled skills remain incomplete. The MCP guide remains a local authoring path, not a connect-and-use quickstart. **Target:** one versioned connector manifest with identity/operator, endpoint/protocol, auth mode, declared capabilities/effects/input schemas/recipients, optional skill references, and immutable content digest. Keep deployment policy and user grants as separate records. Extend the check command to connect in a local sandbox, compare discovered tools with declarations, and run conformance fixtures. Installing the package should be one user action; OAuth, scope review, agent selection/grant, and action approval remain explicit steps in that same journey.

### P1 — Skill and MCP consumption are incomplete as a platform interface

Skills have good version pinning and per-agent enablement in `SkillService`, but the production conversation path does not call `effective` or `load_for_agent`. The Core-only MCP tool module has been deleted; independent hosts still need a governed agent-facing interface. MCP resources and prompts have no equivalent governed discovery/loading path. **Target:** expose a small agent-facing interface such as `discover(context, agent)`, `load_skill(context, agent, skill_id)`, and `invoke(context, agent, capability, arguments, approval_reference)`; keep protocol sessions, auth, grants, policy, and audit behind it. Add prompts/resources only when there is a concrete use case, with the same trust and size limits.

### P1 — MCP version negotiation and advanced protocol behavior

The OAuth client now probes `server/discover` for the 2026-07-28 stateless protocol and falls back to a 2025 initialized session only when the server explicitly reports an unsupported method or version. A local mock test exercises both wire paths. The separate declared remote adapter still speaks only 2026-07-28. Interactive `input_required` results are rejected instead of being treated as completed calls. **Remaining gate:** converge the adapters, test more server implementations and malformed responses, and implement a host-mediated continuation only when its approval semantics are specified. See the [MCP protocol-version guidance](https://ts.sdk.modelcontextprotocol.io/v2/protocol-versions).

### P1 — Operational and package trust surface is unfinished

The Web catalog now reads the operator catalog; static entries contain optional branding only and the old frontend allowlist has been deleted. Publication pins the declaration digest and preserves its operator evidence. Withdrawals transactionally disable bound extensions, clear credentials/sessions and revoke accounts/grants. Cryptographic provenance, staged rollout, health/degraded state, richer tenant policy and automatic material upgrades remain incomplete. `integration_registry::register` disables a new version and revokes existing grants, which is conservative, but it also expires all authorized connections even for metadata-only changes. **Target:** promote immutable versions through draft → validated → enabled; compare material declaration changes and request renewed consent only where needed; make availability and reasons visible to hosts. Keep credentials and grant state out of portable package contents.

## Minimal target architecture

```text
Publisher: manifest + optional declarative skill → check → versioned catalog
Deployment: review/allow version and capabilities → expose install option
User: install → connect/authorize account → choose agent and capabilities
Agent: discover effective capabilities and enabled skills for this context
Call: resolve pinned version → enforce connection + grant + policy + approval
      → protocol adapter → normalized result + audit evidence
Update: diff → validate → reconsent/reauthorize when material → promote
Remove: stop new calls → clear custody and pending work → retain evidence
```

The external seam should be small and protocol-neutral. MCP, direct provider APIs, and future transports are adapters behind it. A package describes capability; a connection identifies one user's account; a grant selects which agent may use it; an approval authorizes one exact external change. Installation creates none of the latter three. A skill is guidance attached to the package or installed separately and cannot carry credentials or execution authority.

## Completion criteria

1. A developer adds a sample read-only MCP connector from a manifest and local sandbox without changing Core identity/grant/approval code or Web catalog code.
2. A user installs it through one catalog action, completes provider authorization, reviews actual account/scopes, and explicitly enables one capability for one agent.
3. A second agent and a second host context cannot discover or invoke that capability. Revocation takes effect at call time.
4. A consequential tool creates an exact Core proposal and cannot execute until an authenticated, proposal-bound decision is recorded. Duplicate, changed, timed-out, and unknown outcomes are reconciled safely.
5. Skills requested by the package are installed at a reviewed version, enabled per agent, loaded on demand, and cannot bypass grants.
6. Upgrade, endpoint swap, quarantine, and removal tests show that old credentials, sessions, grants, and approvals cannot gain new authority; history remains readable.
7. The connector conformance suite runs against an isolated PostgreSQL database and a mock MCP server, including two host contexts, two agents, OAuth races, SSRF, tool-list drift, and provider timeouts.

## Changes made during the review and cleanup

- Extension update/removal clears OAuth sessions and encrypted credentials in the same transaction; OAuth state binds to the initiating endpoint/version and token storage rechecks under a row lock.
- OAuth completion records connectivity and the reported tool inventory without self-attesting conformance or enabling agent use. Account status remains scoped to the host context.
- Deleted Core's unused connected-app model tools, model-inferred confirmation flow, selection heuristic, effect classifier, direct MCP tool caller, and pending-action table. A forward Core migration drops the table from existing development databases; the independent-host reference schema no longer creates it.
- Removed the generic connection initiation/callback stubs, host-asserted authorization method, and their unused session table. Removed Web's PlayStation flow, which could record an authorized account and fabricated activity without provider verification.
- Consolidated Core's active connection, execution, quota, and status references on `external_connections`; a forward migration drops the old `connections` table. The independent-host reference schema now has the same single connection table.
- Web displays the reported tools without claiming their effects are verified, and labels OAuth success as account linking rather than agent readiness.

The selected-agent grant and authenticated approval seams now exist for signed host calls. Immutable package publication/discovery/installation now exists. Full conversation consumption, optional bundled skills, independent conformance, provider reconciliation and protocol compatibility remain open.

## Verification of this change set

- `vox-connections`: Rust tests and Clippy passed; the independent-host lifecycle test passed against a disposable PostgreSQL database.
- `vox-core`: Rust tests and Clippy passed; its full migration chain applied to a disposable PostgreSQL database, including the OAuth binding and legacy-table removal migrations.
- `vox-web`: TypeScript, 100 unit tests, lint, and production build passed. The one database-backed OTP test was skipped by its existing test condition. There is no plugin-specific Playwright test in the repository.

These checks do not establish provider interoperability or production approval safety. No live OAuth provider or remote MCP server was exercised.

## Package catalog follow-up — 2026-09-28

Package installations use the same extension lifecycle code in one database
transaction, avoiding nested pool acquisition and partial install state. The
single-connection concurrent retry test caught and prevented a lock-order
regression. Web's manifest-only browser fixture exercises dynamic discovery,
installation, OAuth return and the explicit agent-access handoff. The linked
account dock also derives from installed extensions instead of static brands.
See [package onboarding](packages.md) for the implemented contract and limits.
