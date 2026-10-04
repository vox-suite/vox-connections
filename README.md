# Vox Connections

A curated Rust library for Google Calendar and PlayStation account access. Core authenticates callers and owns scheduling, ingestion, spans and realtime notifications. Connections owns encrypted credentials, OAuth/PKCE, verified Sony linking, per-account preferences, token rotation, sync leases and checkpoints.

Configure `VOX_CREDENTIAL_KEY` as 64 hexadecimal characters. Missing configuration disables connectors; malformed configured keys fail startup. Google also requires `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, and `VOX_CORE_API_URL`. Register `{VOX_CORE_API_URL}/v1/connectors/google/callback` as the Google OAuth callback. No arbitrary MCP endpoint, package installation, connector daemon or Google MCP adapter is shipped.

Native clients use authenticated Core `/v1/me/connectors` and `/v1/me/connections` APIs. Linking requires consent to timeline synchronization and assistant reads. Either use can be paused independently. Disconnect deletes credentials and retains imported spans. Calendar fields are provider-owned; local notes and collections remain editable. Gaming imports represent observed lifetime-counter increases with unknown session times.

Steam, Valorant, Amazon shopper data and Zomato tracking are outside this release. Release requires isolated database tests, native builds and real account linking/sync on both providers. Passing local checks alone does not validate a deployment or provider account.
