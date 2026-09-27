# vox-connections

Shared Rust crate for external-provider integration plumbing, extracted out of vox-core so vox-core and vox-bridge can both depend on it (and stay clean of provider-specific code) instead of each reimplementing it.

Contains:
- `connections` — OAuth/authorization connection lifecycle (initiate, callback, list, disconnect).
- `capability_grants` — per-agent capability grant/revoke and effective-grant lookup.
- `integration_registry` — capability declarations and per-deployment discovery.
- `providers::{amazon, uber, zomato}` — the three provider clients built on top of the above.

Not included: `expedia`, which stays in vox-core because it also depends on vox-core's `approvals`/`execution`/`execution_policy` (consequential-write approval flow) — pulling those out too would need a trait-based gateway abstraction, which wasn't part of this pass. Move it here later if that's wanted.

## Identity boundary

This crate never resolves caller identity itself. It takes `identity::RequestContext`, a minimal 3-field struct (`user_context_id`, `user_id`, `deployment_id`). The host application (vox-core) converts its own resolved identity type at the call boundary — see `ResolvedUserContext::request_context()` in vox-core's `src/identity/mod.rs`.

## Database

Services here take a plain `sqlx::PgPool`, not vox-core's `Db` wrapper (which also owns migration-running via `sqlx::migrate!()`, and can't move without creating a circular dependency). Callers pass `db.pool().clone()`.
