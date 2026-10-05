# Spotify and YouTube

These connectors use the existing Core authenticated connection APIs. No provider token reaches the desktop client. Account access and refresh tokens are encrypted at rest and bound to user plus connector; setup verifiers are bound to the single-use OAuth state. Connection generation and lease fences protect relinking, preferences, reads, refreshes, and disconnects.

## Configuration

Set `VOX_CREDENTIAL_KEY` to the existing 64-character hexadecimal encryption key and `VOX_CORE_API_URL` to the public HTTPS Core callback base URL. Google uses `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`. Provider-specific variables loaded by this crate are:

- `SPOTIFY_CLIENT_ID`: registered Spotify OAuth application. Spotify uses PKCE; no Spotify client secret is required.
- YouTube uses the constructor's existing Google OAuth client ID and secret. Enable YouTube Data API v3 and configure consent/verification for `youtube.readonly`.

Register exact HTTPS redirect URLs:

- `{VOX_CORE_API_URL}/v1/connectors/spotify/callback`
- `{VOX_CORE_API_URL}/v1/connectors/youtube/callback`

Descriptors fail closed when encryption, callback base URL, or the required credentials are missing. Successful OAuth must produce a refresh token and pass a real resource read before persisting the connection.

## Supported records

Spotify imports the latest 50 provider playback records, keyed by verified account, track, and `played_at`. Track duration is metadata, never an invented playback end. Playlists are read as a current snapshot. Synchronization runs on the existing worker every 15 minutes; these records are explicitly a limited provider window, not a complete historical export.

YouTube API synchronization reads up to 200 playlists and subscriptions, then up to 20 playlists plus the liked-videos playlist with at most 200 entries each. A playlist item's `snippet.publishedAt` represents the playlist addition/like, never a watch time or video upload time. Subscriptions remain a snapshot. Responses explicitly report incomplete coverage and no API watch history.

Real watch-history imports use the independent `youtube_history` connection and require an explicit import consent. Upload the JSON array from an English Google Takeout `watch-history.json` export. Accepted records require `products` containing `YouTube`, a `Watched ` title, an HTTPS YouTube `/watch?v=` URL with a valid video ID, and the actual RFC3339 `time`. Search, like, deleted, malformed, missing-time, and future records are skipped. At most 20,000 records are accepted per request; Core caps request bodies and the desktop additionally bounds selected files. Record identities are stable per user, video and timestamp. Imports do not replace the OAuth YouTube connection. The result includes `imported`, `skipped`, `complete:false`, and `connection_id`; only the most recent 100 accepted records are retained for connection reads.


## Service contracts

`TimelineIngestor::personal_activity(tx, user_id, connection_id, connector, items)` runs in the connection transaction and returns the number ingested. Core owns span/event writes. `PersonalActivity` contains stable `source_id`, `title`, real `occurred_at`, optional real `ended_at`, and `provider_data`.

`handle_personal_callback(connector, code: Option<&str>, state)` returns the authorized connection UUID. `refresh(user_id, connection_id)` dispatches manual provider refreshes. `read_personal(user_id, connection_id)` returns `{connector_id, observed_at, complete:false, data}`. `assistant_read` delegates these providers to the same owned, consent-checked read.

`import_youtube_history(user_id, history: Value, consent)` returns the counts above.

## Verification and external prerequisites

The crate has mock provider contract tests and disposable PostgreSQL lifecycle tests covering OAuth state replay, account verification, per-user encryption, rotation, ownership, pause/disconnect, import consent/deduplication. These checks do not prove a production provider account link or approved application access. Real Spotify application/account eligibility, Google consent publication/verification remain necessary for live use.

Official references:

- [Spotify Authorization Code with PKCE](https://developer.spotify.com/documentation/web-api/tutorials/code-pkce-flow)
- [Spotify recently played](https://developer.spotify.com/documentation/web-api/reference/get-recently-played)
- [YouTube playlist items](https://developers.google.com/youtube/v3/docs/playlistItems/list)
- [YouTube channels and account identity](https://developers.google.com/youtube/v3/docs/channels/list)
- [Google OAuth web-server authorization](https://developers.google.com/identity/protocols/oauth2/web-server)
