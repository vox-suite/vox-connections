# Recovery verification

Audited Connections upstream main `99458ac` plus Maps `f61edd9`, and Core main `ba13964`. The [inventory](recovery-inventory.md) includes every changed file, public declarations, fields, trait contracts and re-exports. Regenerate it with `python3 tools/recovery_inventory.py --core /path/to/vox-core` with both repositories’ upstream histories available.

## Evidence

Verification used disposable PostgreSQL 18 with pgvector for Core, independent-host schemas for library fixtures, and local mock providers. No production database or provider credentials were used.

| Check | Result |
| --- | --- |
| Formatting and Clippy, all library targets | Pass, warnings denied |
| Library, binaries, public host-boundary imports and HMAC service tests | Pass |
| Curated account lifecycle suite | 22 passed: replay, rotation, disconnect/relink, leases, generations, paused preferences, explicit grants, two same-user contexts and standalone upgrade |
| Independent-host package/grant/skill installation, consent and Google MCP reads | 3 passed |
| Bounded metadata, schema drift and revocation | 1 passed |
| Restored Amazon/Uber/Zomato services and Expedia transport | 2 passed: grant denial/revocation, minimized Uber fields, labelled incomplete handoffs, idempotent reconciliation and errors |
| Core ingestion | 11 passed, including stable IDs on repeated sync, separate same-user context imports and observed PlayStation range markers |
| Core migration upgrades | 5 passed through SQLx: old platform credentials/grants preserved, already-retired access stays revoked, ambiguous ownership preserved, current curated credentials/history IDs/notes preserved without duplication |
| Core ownership, memory, worker fences, approval/cancellation and specialist delegation | Pass with separate disposable fixture databases |

## Upgrade behavior

Fresh standalone hosts install both platform and account schemas in [documented order](../schema/README.md). Existing standalone curated hosts use additive scope upgrade and authority projection; do not replay credential-expiration statements. Core keeps historical migration checksums and inserts an additive earlier safeguard before the retirement migration. Migration tests verify that the production SQLx runner accepts it even on an already-retired database. Migration writers must be stopped during this upgrade.

Already-deleted OAuth/package/credential rows cannot be reconstructed. Recovery does not reauthorize those connections or grants and does not replay approvals. Before-retirement upgrades preserve the original encrypted rows and authority rather than erase and reconstruct them. Account projection creates a dedicated `curated_<provider>` declaration alongside earlier provider capabilities; linking creates no grant. Both old and new crypto paths share the implementation and decrypt each other’s ciphertext without changing user/provider associated data.

## Remaining release gates

- https://github.com/vox-suite/vox-web/issues/27: native and hosted UIs must show explicit selected-agent grants and authenticated account reassociation. A read preference is insufficient authority.
- https://github.com/vox-suite/vox-core/issues/121: coordinated Core dependency pin and verified migration/consumer delivery.
- Real provider linking, callback allowlisting, provider configuration and production-host verification remain deployment validation. Swiggy is default enabled when configured; Zomato OAuth stays opt-in.

The latest synthetic evenly-spaced PlayStation sessions are intentionally superseded by the approved observed-range model. Retained first/last markers carry the labelled range; cumulative counter increases and estimated placement near a provider last-played timestamp are distinct. No sync deletes existing first/last history or asserts uninterrupted gaming.

## Latest reconciliation and measured performance

Retain immediate PSN first sync and sync-failure logging, consented Maps import, new Core finance records, goals and span day endpoints. HMAC verification uses the configured host key; a caller-supplied key cannot authorize requests. Proven explicit identity linking preserves contexts, connection IDs, grants and original credential encryption ownership through token rotation. Matching verified email alone does not merge authority.

Core's native list/detail/day/chart reads isolate the exact first-party context and honor disabled account preferences. Restored generic connection history remains readable to its owner. Agent timeline/chart/goal reads additionally check selected-agent grants, including a fresh check after model generation. Combined spending is retained for owner charts and withheld from agents until its execution can bind contributor grants. Expedia's legacy cancellation interface fails closed until it can carry an exact penalty-bound approval.

Local PostgreSQL18/pgvector fixture: six charts, 1,000 rows: cold5 queries/30.36ms, warm1 query/p95 1.16ms; 100,000 rows: cold5 queries/2275.82ms, warm1 query/p95 0.82ms, forced refresh5 queries/2140.63ms. Account catalogue authorization batches scopes once and effective grants once per context per pass. These are disposable local fixture measurements, not production latency guarantees.
