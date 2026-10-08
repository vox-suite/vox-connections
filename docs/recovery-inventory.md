# Recovery inventory

Baseline: `6259e0e95b4db8a3da91d553c3b321b102d4aec9`; audited tips: Connections main `99458ac` plus Maps `f61edd9`; Core main `ba13964`.

This inventory records the original five commits and seven subsequently published Connections commits, plus every corresponding Core change. It records every changed file per commit, exact public Rust declaration signatures and re-exports before/after, and recovery dispositions. Public declaration records include declarations inside modules; module visibility is recorded in the same inventory. Git additions/deletions measure diff lines, not independent features.

The author’s apparent intent was a curated native account/timeline replacement followed by provider expansion. There are no explicit revert commits in these twelve commits. Zomato was replaced with OAuth history, not restored with its earlier search/handoff service.

## Per-commit file changes

### 61073aa — Complete curated account connections and retire MCP runtime

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `.github/workflows/ci.yml` | rewritten | 2 / 14 | Both platform and account suites restored |
| `Cargo.toml` | rewritten | 3 / 21 | Combined: old public surface plus new provider/account behavior |
| `Dockerfile` | deleted | 0 / 23 | Restored from baseline |
| `README.md` | rewritten | 4 / 129 | Combined: old public surface plus new provider/account behavior |
| `defaults/skills/compare-options/SKILL.md` | deleted | 0 / 14 | Restored from baseline |
| `defaults/skills/create-a-skill/SKILL.md` | deleted | 0 / 14 | Restored from baseline |
| `defaults/skills/meeting-prep/SKILL.md` | deleted | 0 / 14 | Restored from baseline |
| `defaults/skills/plan-my-day/SKILL.md` | deleted | 0 / 14 | Restored from baseline |
| `defaults/skills/summarize-actions/SKILL.md` | deleted | 0 / 14 | Restored from baseline |
| `defaults/skills/writing-assistant/SKILL.md` | deleted | 0 / 14 | Restored from baseline |
| `docs/architecture-review-2026-09-27.md` | deleted | 0 / 106 | Restored from baseline |
| `docs/declarative-skills.md` | deleted | 0 / 9 | Restored from baseline |
| `docs/google-read-adapter.md` | deleted | 0 / 59 | Restored from baseline |
| `docs/mcp-authoring.md` | deleted | 0 / 27 | Restored from baseline |
| `docs/metadata-load-baseline.json` | deleted | 0 / 38 | Restored from baseline |
| `docs/metadata-load-review-baseline.json` | deleted | 0 / 38 | Restored from baseline |
| `docs/metadata-load.md` | deleted | 0 / 102 | Restored from baseline |
| `docs/packages.md` | deleted | 0 / 66 | Restored from baseline |
| `examples/independent_host_schema.sql` | deleted | 0 / 31 | Restored from baseline |
| `examples/mcp/minimal_server.py` | deleted | 0 / 52 | Restored from baseline |
| `examples/mcp/read-only-manifest.json` | deleted | 0 / 31 | Restored from baseline |
| `examples/packages/github-repository-reader/README.md` | deleted | 0 / 15 | Restored from baseline |
| `examples/packages/github-repository-reader/schema-source.json` | deleted | 0 / 11 | Restored from baseline |
| `examples/packages/github-repository-reader/skills/github-repository-orientation/SKILL.md` | deleted | 0 / 15 | Restored from baseline |
| `examples/packages/github-repository-reader/vox-package.json` | deleted | 0 / 209 | Restored from baseline |
| `examples/packages/google-calendar-reader/README.md` | deleted | 0 / 3 | Restored from baseline |
| `examples/packages/google-calendar-reader/vox-package.json` | deleted | 0 / 67 | Restored from baseline |
| `examples/packages/google-drive-metadata-reader/README.md` | deleted | 0 / 3 | Restored from baseline |
| `examples/packages/google-drive-metadata-reader/vox-package.json` | deleted | 0 / 58 | Restored from baseline |
| `examples/skills/meeting-prep.json` | deleted | 0 / 10 | Restored from baseline |
| `schema/README.md` | rewritten | 2 / 16 | Combined: old public surface plus new provider/account behavior |
| `schema/connectors.sql` | rewritten | 51 / 774 | Restored platform snapshot; curated schema separated into accounts.sql |
| `schema/discovery.sql` | deleted | 0 / 42 | Restored from baseline |
| `schema/packages.sql` | deleted | 0 / 23 | Restored from baseline |
| `schema/playstation.sql` | deleted | 0 / 18 | Restored from baseline |
| `schema/public-mcp.sql` | deleted | 0 / 6 | Restored from baseline |
| `schema/setup.sql` | deleted | 0 / 20 | Restored from baseline |
| `schema/skill-content.sql` | deleted | 0 / 17 | Restored from baseline |
| `src/account_tests.rs` | newly added | 790 / 0 | Retained with current behavior |
| `src/accounts.rs` | newly added | 1167 / 0 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |
| `src/bin/google_read_mcp.rs` | deleted | 0 / 13 | Restored from baseline |
| `src/bin/server.rs` | deleted | 0 / 46 | Restored from baseline |
| `src/bin/vox.rs` | deleted | 0 / 259 | Restored from baseline |
| `src/capability_grants.rs` | deleted | 0 / 241 | Restored from baseline |
| `src/conformance/fixtures/v1.json` | deleted | 0 / 185 | Restored from baseline |
| `src/conformance/mod.rs` | deleted | 0 / 781 | Restored from baseline |
| `src/connected_apps/mcp.rs` | deleted | 0 / 875 | Restored from baseline |
| `src/connected_apps/mod.rs` | deleted | 0 / 1145 | Restored from baseline |
| `src/connected_apps/oauth.rs` | deleted | 0 / 589 | Restored from baseline |
| `src/connections.rs` | deleted | 0 / 233 | Restored from baseline |
| `src/crypto.rs` ← `src/connected_apps/crypto.rs` | moved and rewritten | 99 / 0 | Shared implementation retained; old connected_apps::crypto adapter restored |
| `src/defaults.rs` | deleted | 0 / 38 | Restored from baseline |
| `src/discovery.rs` | deleted | 0 / 60 | Restored from baseline |
| `src/discovery.sql` | deleted | 0 / 49 | Restored from baseline |
| `src/google_reads.rs` | deleted | 0 / 637 | Restored from baseline |
| `src/identity.rs` | deleted | 0 / 43 | Restored from baseline |
| `src/integration_registry.rs` | deleted | 0 / 435 | Restored from baseline |
| `src/lib.rs` | rewritten | 2 / 16 | Combined: old public surface plus new provider/account behavior |
| `src/packages.rs` | deleted | 0 / 526 | Restored from baseline |
| `src/providers/amazon.rs` | deleted | 0 / 542 | Restored from baseline |
| `src/providers/expedia.rs` | deleted | 0 / 438 | Restored from baseline |
| `src/providers/google_calendar.rs` | newly added | 588 / 0 | Retained with current behavior |
| `src/providers/mod.rs` | rewritten | 3 / 40 | Combined: old public surface plus new provider/account behavior |
| `src/providers/observations.rs` | newly added | 153 / 0 | Retained with current behavior |
| `src/providers/playstation.rs` | rewritten | 4 / 460 | Combined: old public surface plus new provider/account behavior |
| `src/providers/playstation_account.rs` | deleted | 0 / 467 | Restored from baseline |
| `src/providers/psn.rs` | newly added | 322 / 0 | Retained with current behavior |
| `src/providers/uber.rs` | deleted | 0 / 739 | Restored from baseline |
| `src/providers/zomato.rs` | deleted | 0 / 490 | Combined: old public surface plus new provider/account behavior |
| `src/remote_extensions/adapters/direct.rs` | deleted | 0 / 244 | Restored from baseline |
| `src/remote_extensions/adapters/integrity.rs` | deleted | 0 / 121 | Restored from baseline |
| `src/remote_extensions/adapters/mcp.rs` | deleted | 0 / 333 | Restored from baseline |
| `src/remote_extensions/adapters/mod.rs` | deleted | 0 / 362 | Restored from baseline |
| `src/remote_extensions/adapters/privacy.rs` | deleted | 0 / 54 | Restored from baseline |
| `src/remote_extensions/adapters/transport.rs` | deleted | 0 / 89 | Restored from baseline |
| `src/remote_extensions/mod.rs` | deleted | 0 / 1296 | Restored from baseline |
| `src/service/auth.rs` | deleted | 0 / 402 | Restored from baseline |
| `src/service/client.rs` | deleted | 0 / 311 | Restored from baseline |
| `src/service/config.rs` | deleted | 0 / 80 | Restored from baseline |
| `src/service/mod.rs` | deleted | 0 / 11 | Restored from baseline |
| `src/service/routes.rs` | deleted | 0 / 1052 | Restored from baseline |
| `src/service/state.rs` | deleted | 0 / 75 | Restored from baseline |
| `src/setup.rs` | deleted | 0 / 374 | Restored from baseline |
| `src/skill_format.rs` | deleted | 0 / 164 | Restored from baseline |
| `src/skills.rs` | deleted | 0 / 596 | Restored from baseline |
| `tests/google_read_integration.rs` | deleted | 0 / 328 | Restored from baseline |
| `tests/host_boundary.rs` | deleted | 0 / 71 | Restored from baseline |
| `tests/metadata_load.rs` | deleted | 0 / 397 | Restored from baseline |
| `tests/postgres_integration.rs` | deleted | 0 / 881 | Restored from baseline |
| `tests/service_api_tests.rs` | deleted | 0 / 297 | Restored from baseline |
| `tests/setup_integration.rs` | deleted | 0 / 467 | Restored from baseline |
| `tools/mcp_probe.py` | deleted | 0 / 67 | Restored from baseline |

### da37dcb — feat: record PlayStation games from first to last played as timeline spans

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | rewritten | 13 / 0 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |

### e759fbe — Add food delivery and personal integrations, connection account updates

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `README.md` | rewritten | 4 / 0 | Combined: old public surface plus new provider/account behavior |
| `docs/food-connections.md` | newly added | 54 / 0 | Retained with current behavior |
| `docs/personal-integrations.md` | newly added | 46 / 0 | Retained with current behavior |
| `src/account_tests.rs` | rewritten | 416 / 0 | Retained with current behavior |
| `src/accounts.rs` | rewritten | 773 / 70 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |
| `src/providers/food_delivery.rs` | newly added | 648 / 0 | Retained with current behavior |
| `src/providers/food_oauth.rs` | newly added | 145 / 0 | Retained with current behavior |
| `src/providers/mod.rs` | rewritten | 7 / 0 | Combined: old public surface plus new provider/account behavior |
| `src/providers/personal.rs` | newly added | 560 / 0 | Retained with current behavior |

### 9bbf2d4 — Split Swiggy and Zomato into separate provider modules

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | moved/refactored | 9 / 37 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |
| `src/providers/food_delivery.rs` | moved/refactored | 55 / 215 | Retained with current behavior |
| `src/providers/food_oauth.rs` | moved/refactored | 1 / 1 | Retained with current behavior |
| `src/providers/mod.rs` | moved/refactored | 2 / 0 | Combined: old public surface plus new provider/account behavior |
| `src/providers/swiggy.rs` | newly added | 183 / 0 | Retained with current behavior |
| `src/providers/zomato.rs` | newly added | 85 / 0 | Combined: old public surface plus new provider/account behavior |

### e4d108d — Enable Swiggy by default; remove SWIGGY_MCP_ENABLED flag

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `README.md` | rewritten | 1 / 1 | Combined: old public surface plus new provider/account behavior |
| `docs/food-connections.md` | rewritten | 1 / 1 | Retained with current behavior |
| `src/providers/food_delivery.rs` | rewritten | 2 / 2 | Retained with current behavior |
| `src/providers/swiggy.rs` | rewritten | 1 / 1 | Retained with current behavior |
| `src/providers/zomato.rs` | rewritten | 1 / 1 | Combined: old public surface plus new provider/account behavior |

### 4eafba6 — Spotify: drop playlists and extra scopes; timeline listening history only

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `docs/personal-integrations.md` | rewritten | 1 / 1 | Retained with current behavior |
| `src/account_tests.rs` | rewritten | 0 / 1 | Retained with current behavior |
| `src/providers/personal.rs` | rewritten | 2 / 8 | Retained with current behavior |

### f93cde6 — PlayStation: sync every 10 minutes; drop first/last-played history

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | rewritten | 1 / 14 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |

### 4379e21 — PlayStation: sync every 30 minutes

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | rewritten | 1 / 1 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |

### 6f43efb — PlayStation: pass game history to the ingestor again (estimated sessions)

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | rewritten | 13 / 0 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |

### be81418 — Log the error when a scheduled connection sync fails

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | rewritten | 2 / 2 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |

### 99458ac — PlayStation: first sync right after connecting

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | rewritten | 2 / 2 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |

### f61edd9 — Add Google Maps Timeline import connector (maps_timeline)

| File | Classification | Added / deleted | Recovery disposition |
| --- | --- | --- | --- |
| `src/accounts.rs` | rewritten | 43 / 8 | Retained; public ownership arguments changed to RequestScope, assistant reads require agent grant; 30-minute PSN cadence retained; history-removal intermediate superseded by final history callback |
| `src/providers/personal.rs` | rewritten | 84 / 0 | Retained with current behavior |

## Public API changes

Complete exact signatures, re-exports and source locations for every changed Rust file are in [recovery-inventory.json](recovery-inventory.json). Changed signatures appear once as a removal and once as an addition. This is an exact source declaration inventory, not a claim that each declaration was reachable through every old module export.

Recovery restores all former module paths and provider service exports. Curated APIs retain their names but require `&impl RequestScope`; `assistant_read` and `read_personal` additionally require the selected agent key. Raw context UUID entry points are crate-private. `PlayStationGame.last_played_at` remains optional to retain provider uncertainty, and old consumers handle absent values without inventing timestamps.

## Core correspondence

Core baseline: parent of `e4a9b96`; recovery includes upstream through `658387e`. Native connection APIs, worker scheduling, WiZ, map tools, notification delivery, new OAuth callback presentation and personal ingestion are retained. The table below accounts for every file changed by the corresponding retirement commit.

| Core file | Recovery disposition |
| --- | --- |
| `.github/workflows/pr-checks.yml` | Retained; not part of connector recovery |
| `Cargo.lock` | Retained; not part of connector recovery |
| `Cargo.toml` | Retained; not part of connector recovery |
| `README.md` | Retained; not part of connector recovery |
| `contracts/device-protocol.md` | Retained; not part of connector recovery |
| `contracts/openapi.json` | Retained and reconciled with restored platform where affected |
| `defaults/skills/compare-options/SKILL.md` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `defaults/skills/create-a-skill/SKILL.md` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `defaults/skills/meeting-prep/SKILL.md` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `defaults/skills/plan-my-day/SKILL.md` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `defaults/skills/summarize-actions/SKILL.md` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `defaults/skills/writing-assistant/SKILL.md` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `docs/capability-grants.md` | Restored; compatibility adjusted where necessary |
| `docs/conformance.md` | Restored; compatibility adjusted where necessary |
| `docs/connections-verification.md` | Retained and reconciled with restored platform where affected |
| `docs/connections.md` | Retained and reconciled with restored platform where affected |
| `docs/integration-discovery.md` | Restored; compatibility adjusted where necessary |
| `docs/integration-registry.md` | Restored; compatibility adjusted where necessary |
| `docs/playstation.md` | Retained; not part of connector recovery |
| `docs/tools.md` | Retained; not part of connector recovery |
| `migrations/20261004000000_fresh_connections.sql` | Historical migration preserved; additive recovery migrations undo retirement safely |
| `migrations/20261004000001_connection_lifecycle.sql` | Historical migration preserved; additive recovery migrations undo retirement safely |
| `migrations/20261004000002_retire_connector_runtime.sql` | Historical migration preserved; additive recovery migrations undo retirement safely |
| `migrations/20261004000003_local_delegation.sql` | Historical migration preserved; additive recovery migrations undo retirement safely |
| `migrations/20261004000004_credential_generation.sql` | Historical migration preserved; additive recovery migrations undo retirement safely |
| `services/api/auth.rs` | Retained; not part of connector recovery |
| `services/api/main.rs` | Retained and reconciled with restored platform where affected |
| `services/api/openapi.rs` | Retained and reconciled with restored platform where affected |
| `services/api/router.rs` | Retained; not part of connector recovery |
| `services/api/routes/connections.rs` | Retained and reconciled with restored platform where affected |
| `services/api/routes/map_scene.rs` | Retained; not part of connector recovery |
| `services/api/routes/mod.rs` | Retained; not part of connector recovery |
| `services/api/state.rs` | Retained; not part of connector recovery |
| `services/defaults/main.rs` | Retained; not part of connector recovery |
| `services/worker/runtime.rs` | Retained; not part of connector recovery |
| `src/agent_registry/owned.rs` | Retained; not part of connector recovery |
| `src/agents/conversation.rs` | Retained; not part of connector recovery |
| `src/agents/prompts.rs` | Retained; not part of connector recovery |
| `src/agents/tools/connections.rs` | Retained and reconciled with restored platform where affected |
| `src/agents/tools/library.rs` | Restored governed behavior alongside retained native APIs |
| `src/agents/tools/map_scene.rs` | Retained; not part of connector recovery |
| `src/agents/tools/mod.rs` | Retained; not part of connector recovery |
| `src/agents/tools/visits.rs` | Retained; not part of connector recovery |
| `src/application/spans.rs` | Retained; not part of connector recovery |
| `src/approval_contract.rs` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `src/approvals/mod.rs` | Restored governed behavior alongside retained native APIs |
| `src/capability_grants.rs` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `src/config.rs` | Retained and reconciled with restored platform where affected |
| `src/conformance/mod.rs` | Restored; compatibility adjusted where necessary |
| `src/connected_apps/mod.rs` | Restored; compatibility adjusted where necessary |
| `src/connection_ingestion_tests.rs` | Retained and reconciled with restored platform where affected |
| `src/defaults.rs` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `src/delegation/mod.rs` | Restored governed behavior alongside retained native APIs |
| `src/durable_tasks/runs.rs` | Restored governed behavior alongside retained native APIs |
| `src/execution/mod.rs` | Retained and reconciled with restored platform where affected |
| `src/execution_policy/mod.rs` | Retained and reconciled with restored platform where affected |
| `src/fresh_connections.rs` | Retained and reconciled with restored platform where affected |
| `src/host_trust/mod.rs` | Retained; not part of connector recovery |
| `src/http/capability_grants.rs` | Restored; compatibility adjusted where necessary |
| `src/http/connected_apps.rs` | Restored; compatibility adjusted where necessary |
| `src/http/connected_reads.rs` | Restored; compatibility adjusted where necessary |
| `src/http/connections.rs` | Restored; compatibility adjusted where necessary |
| `src/http/connector_setup.rs` | Restored; compatibility adjusted where necessary |
| `src/http/consequential_writes.rs` | Restored; compatibility adjusted where necessary |
| `src/http/context.rs` | Retained; not part of connector recovery |
| `src/http/conversations.rs` | Retained; not part of connector recovery |
| `src/http/events.rs` | Retained; not part of connector recovery |
| `src/http/handoffs.rs` | Restored; compatibility adjusted where necessary |
| `src/http/integration_registry.rs` | Restored; compatibility adjusted where necessary |
| `src/http/library.rs` | Retained; not part of connector recovery |
| `src/http/mod.rs` | Restored governed behavior alongside retained native APIs |
| `src/http/packages.rs` | Restored; compatibility adjusted where necessary |
| `src/http/playstation.rs` | Restored; compatibility adjusted where necessary |
| `src/http/remote_extensions.rs` | Restored; compatibility adjusted where necessary |
| `src/http/schedules.rs` | Retained; not part of connector recovery |
| `src/http/skills.rs` | Retained; not part of connector recovery |
| `src/identity/mod.rs` | Retained and reconciled with restored platform where affected |
| `src/identity_contract.rs` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `src/lib.rs` | Retained and reconciled with restored platform where affected |
| `src/map_scene.rs` | Retained; not part of connector recovery |
| `src/playstation.rs` | Restored; compatibility adjusted where necessary |
| `src/privacy/content.rs` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `src/privacy/mod.rs` | Retained and reconciled with restored platform where affected |
| `src/providers/expedia.rs` | Restored; compatibility adjusted where necessary |
| `src/providers/mod.rs` | Restored; compatibility adjusted where necessary |
| `src/realtime.rs` | Retained; not part of connector recovery |
| `src/remote_extensions/adapters/mod.rs` | Restored; compatibility adjusted where necessary |
| `src/remote_extensions/mod.rs` | Restored; compatibility adjusted where necessary |
| `src/skill_format.rs` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `src/skills.rs` | Shared Connections implementation; compatibility re-export, no duplicate ownership |
| `src/status/mod.rs` | Retained and reconciled with restored platform where affected |
| `src/storage/spans.rs` | Retained; not part of connector recovery |
| `src/workers/task_executor.rs` | Restored governed behavior alongside retained native APIs |
### Core 0b4ba34 — Isolate ingestion notification regression by user

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/connection_ingestion_tests.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 95eb1c8 — Include Core-owned bundled skills in release images

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Dockerfile` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Dockerfile.worker` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 7dca9f2 — Add WiZ lights agent tool, food/personal connections, bump vox-connections

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/cors.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/main.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/conversation.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/wiz.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/spans.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/connection_ingestion_tests.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/fresh_connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/map_scene.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/spans.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core 368df63 — Fix lint and agent tool-set test for WiZ tool

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/agents/conversation.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/wiz.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/fresh_connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core a81c26b — Bump vox-connections (split Swiggy/Zomato modules), use provider_label

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/fresh_connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 02bb419 — Bump vox-connections: enable Swiggy by default

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 1ea0089 — Branded OAuth callback page

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `services/api/routes/callback_page.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core ffc9010 — Bump vox-connections: Spotify timeline-only

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core b192c38 — PlayStation: drop title prefix; store game cover image on history spans

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/fresh_connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core fc0955a — Bulk-upsert personal activity (YouTube/Spotify) in batches instead of two queries per record

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/fresh_connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 9491212 — PlayStation: first/last played markers; schedule observed sessions from last-played time

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/fresh_connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 50f5aa5 — API auth: cache Google JWKS, skip signup transaction for known users

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `services/api/auth.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/identity_token.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 7c840e8 — PlayStation: sessions only; remove first/last-played markers; bump vox-connections (10-minute sync)

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/fresh_connections.rs` | rewritten | Superseded: preserve labelled observed ranges; no deletion of history or synthetic evenly-spaced sessions |

### Core 7b20d42 — Bump vox-connections: PlayStation 30-minute sync

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 658387e — PlayStation: estimated sessions from totals, spread evenly between first and last played

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/fresh_connections.rs` | rewritten | Superseded: preserve labelled observed ranges; no deletion of history or synthetic evenly-spaced sessions |

### Core e6f2f78 — Log failed connection refreshes; 409 when a sync is already running

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 7db8b59 — One user per verified email: unify accounts at sign-in

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `services/api/identity_token.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/auth.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/account_linking.rs` | newly added | Retained explicit proven-identity linking; automatic verified-email merging superseded; preserve context IDs and encryption provenance |
| `src/lib.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 4d31e58 — Bump vox-connections: PlayStation first sync on connect

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core fd78708 — Run connection refresh in a detached task so a dropped request cannot orphan the lease

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `services/api/routes/connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core fc431f8 — Replace location activity tracking with Maps Timeline import (#123)

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261006000000_remove_location_activity.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/location.rs` | deleted | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/state.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/visits.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/spans.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/consent.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/fresh_connections.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/lib.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/location_ingestion.rs` | deleted | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/spans.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core 8b550e9 — Add Pulse discovery and cached chart execution

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Dockerfile` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Dockerfile.worker` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `build.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `docs/pulse-discovery-verification.md` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `docs/pulse-evidence/suggestions-fixture.png` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `docs/superpowers/plans/2026-10-06-pulse-discovery.md` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `docs/superpowers/specs/2026-10-06-pulse-discovery-design.md` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261006000001_pulse_discovery.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261006000002_pulse_cache_coalescing.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/pulse.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/execution.rs` | newly added | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/measurements.rs` | newly added | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/mod.rs` | newly added | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/service.rs` | newly added | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/tests.rs` | newly added | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/domain/pulse.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/pulse.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 517c058 — Pulse suggestions: Gemini-only, 3 min timeout, persist until manual refresh

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core 3309294 — Fix clippy lints blocking push

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/tests.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core ff93b53 — Pulse: 8 suggestions, generate more, prompt-driven suggestions

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/tests.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 495b868 — Pulse: iterative chart composer endpoint and shiftable time windows (offset_days)

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/execution.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/tests.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 55a5c83 — Pulse: allow day/week/month series for PlayStation observed playtime increases

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core e074ae6 — Pulse: weekly/monthly PlayStation series from observed increases and estimated sessions

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core 35889db — Fix pulse_cached_aggregate call: restore $3 bind broken by window refactor

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/application/pulse/execution.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core aad7453 — cargo fmt

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/application/pulse/execution.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core a9556bc — Pulse: stat chart type with headline total; cap discovery candidates at 12

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/execution.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/charts.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/domain/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 45d8244 — Pulse: keep time ranges out of generated chart titles

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core b64aebe — Pulse: all-time windows up to 10 years (week/month buckets beyond a year)

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/execution.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core 453cdd5 — Pulse: rank category charts by value and add top_n

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/execution.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/tests.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core ec832ec — Pulse: generic numeric measurements, schema field hints, connector title/place dimensions

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 78d0f01 — Pulse goals: saving and data-driven goals with pace, projection and chat drafting

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261007000000_pulse_goals.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/service/goals.rs` | newly added | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/domain/pulse_goals.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/pulse_goals.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core e21c93b — Pulse compose: lenient model output parsing and failure logging

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/agents/chart_suggester.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core 507fe4a — SMS: keep card transaction details from OTP messages, redact the code, record as authorization attempts

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/events/handler.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/sms_ingestion/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 198b241 — SMS: recognise more bank OTP layouts, dedupe authorization attempts separately, label them in Pulse

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/events/handler.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/sms_ingestion/finance.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/sms_ingestion/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 0f85d0a — Spaces: agent proposes goals on nodes; user approval creates the Pulse goal and links it to the node

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261007000001_goal_space_node_link.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/spaces.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/space_runtime.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/space_goals.rs` | newly added | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/service/goals.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/pulse_goals.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/pulse_goals.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core cee2f29 — Money: route completed payments to structured records, structure event-agent money, add All spending measurement, clean up existing rows

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `contracts/openapi.base.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `contracts/openapi.json` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261007000002_structure_event_agent_money.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/tools/event_actions.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/execution.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/application/pulse/measurements.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/events/handler.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/jev/event_triage.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/sms_ingestion/mod.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 78ea3d8 — Fix cleanup migration: temp table must survive autocommit (psql) as well as a transaction

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `migrations/20261007000002_structure_event_agent_money.sql` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |

### Core 26f5ce1 — Spans: per-day counts and keyset-paginated day endpoints with range index

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `migrations/20261007000000_span_day_range_index.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/spans.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/spans.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/domain/spans.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/spans.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core 68349c2 — Spans: per-user revision, collection-scoped day endpoints

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `migrations/20261007000001_span_revisions.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/spans.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/domain/spans.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/spans.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |

### Core ba13964 — Port span day endpoints, revisions, chart delete, auto space titles, JSON logs, stale-event triage and finance schema seeding onto main

| File | Classification | Recovery disposition |
| --- | --- | --- |
| `Cargo.lock` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `Cargo.toml` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `examples/merge_schemas.rs` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261007000000_span_day_range_index.sql` | deleted | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261007000001_span_revisions.sql` | deleted | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261008000000_span_day_range_index.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261008000001_span_revisions.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `migrations/20261008000002_seed_finance_schemas.sql` | newly added | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/openapi.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/router.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `services/api/routes/spaces.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/agents/space_architect.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/application/pulse/service.rs` | rewritten | Retained new charts/goals/day endpoints with native context isolation and explicit agent grants; composite spending withheld from agents pending contributor-scoped execution |
| `src/domain/spaces.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/events/handler.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/storage/pulse.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |
| `src/telemetry.rs` | rewritten | Retained; subject cache checks stay scoped; shared implementations delegated to Connections where applicable |


## Data that the code restoration cannot recover

Core’s retirement migration revoked grants and external connections, expired unused decisions, dropped credential/OAuth state, packages/installations, setup records, PlayStation credential records and compact tool metadata, then renamed six surviving platform tables. Deployments which have not yet run retirement use an additive pre-retirement safeguard to preserve these rows and their existing authority. Already-retired deployments cannot recover deleted records: forward recovery restores names and empty storage, rebuilds metadata from retained declarations, and preserves surviving IDs and revoked states. It does not revive grants, approvals or deleted credentials. Packages require republication/review and affected accounts require reconnecting.

Curated ciphertext retains its original user/provider associated-data interpretation. Context ownership is enforced separately by validated scopes, composite foreign keys and context-specific queries. Ambiguous legacy accounts retain IDs and ciphertext but are excluded from all reads/scheduled sync until explicit reassociation.

## Compatibility and release gates

Host compatibility blocker: https://github.com/vox-suite/vox-web/issues/27. Native host UIs must expose explicit grants via restored authenticated platform routes. Linking does not auto-grant the Personal Assistant. The account read tool will deny access until a grant exists. Any host relying on automatic access must update its grant journey before release. Real provider account authorization and production callback allowlisting remain deployment validation gates.

## Current restored declarations

The JSON also records the current declarations for every library Rust source, including restored modules and compatibility signatures. Provider source declarations are distinct from deployment-verified access.

## Additional confidence audit

The legacy PlayStation game timestamp, provider/account game responses and parser retain the baseline required timestamp contract. Curated optional history uses `PlayStationGameHistory` and `parse_history_titles_from_json`; missing timestamps are not fabricated. Account preferences survive relink/import, and paused operations do not ingest history. The additive `TimelineIngestor::reassociate_history` hook lets each host move its own account-bound history atomically with explicit reassociation; hosts with historical storage must implement it. Core's implementation preserves span/event identity and annotations without creating agent grants.
