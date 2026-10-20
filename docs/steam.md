# Steam playtime connector

Set `STEAM_WEB_API_KEY` on each Core API/worker/assistant process that constructs the connections service, along with the existing credential encryption key and `CORE_API_URL`. Without a key the catalogue shows Steam as unavailable. Obtain a user Web API key from https://steamcommunity.com/dev/apikey. Never put this server key in a client bundle or a connection record.

The browser signs in at Steam OpenID and returns to `CORE_API_URL/v1/connectors/steam/callback`. Setup state is expiring and single-use. The server checks the signed identity, expected return URL, nonce timestamp and provider endpoint, and verifies the assertion directly with Steam. Cancelling or superseding setup prevents its callback from committing.

Game details and playtime must be public. OpenID proves account ownership but does not grant access to private game data. An unavailable/private library is an error, not an empty library; previous counters are preserved. The API does not provide exact play session boundaries.

Initial connection establishes a baseline. Every 15 minutes, the existing sync worker compares per-game lifetime minutes with the persisted high-water counters. Positive deltas produce `gaming` timeline entries with game, platform, duration_seconds, observation_start/end, timing=observed_counter_delta and session_times_known=false. The displayed timestamp is the observation time, not a claimed session start. Newly discovered games establish their own baseline. Decreased counters never create negative activity or double-count a later recovery. Pausing then resuming establishes a new baseline and excludes activity during the pause. Offline play may appear later when Steam updates its counters.

Timeline ingestion and checkpoints commit atomically under the normal generation/lease fences. A failed transaction can be retried without duplicated playtime. Restart recovery uses database checkpoints; no local cache is required. Assistant reads enforce their own independent permission, do not advance sync checkpoints, and report whether the library response was truncated.

Existing historical lifetime playtime cannot be reconstructed as dated sessions. The first version does not import achievements or friends.

Core uses a pinned Git dependency on vox-connections. For local verification use:

```
cargo check --config 'patch."https://github.com/vox-suite/vox-connections.git".vox-connections.path="../vox-connections"'
```

For release, publish the connections commit and update Core's dependency revision/lockfile before deploying both API and worker. Local builds do not prove a real Steam sign-in or production sync.

Sources: https://partner.steamgames.com/doc/features/auth#website and https://partner.steamgames.com/doc/webapi/IPlayerService
