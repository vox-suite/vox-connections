# Swiggy and Zomato connections

The food integrations use official MCP endpoints with per-user OAuth authorization. Browser cookies and demo credentials are no longer accepted as food-account authorization.

## Provider setup

Keep both integrations disabled until provider access is approved and the exact callback URL is allowlisted. Set `SWIGGY_MCP_ENABLED=true` and/or `ZOMATO_MCP_ENABLED=true` in both Core API and Worker environments after approval. `VOX_CREDENTIAL_KEY` for credential encryption and `VOX_CORE_API_URL` must also be configured.

Callback URLs:

- `<VOX_CORE_API_URL>/v1/connectors/swiggy/callback`
- `<VOX_CORE_API_URL>/v1/connectors/zomato/callback`

Vox dynamically registers a public OAuth client, generates a unique state and S256 PKCE verifier, and encrypts setup credentials. The callback exchanges the authorization code and performs a real read before saving an authorized connection. Setup expires after 15 minutes and callbacks cannot be replayed. Restarted or cancelled setup cannot commit an in-flight authorization result.

## Token lifecycle

Access tokens, refresh tokens, and expiry are stored per user using the existing authenticated encryption. Issued refresh tokens are exchanged automatically when access expiry is within 60 seconds, or once after an authorization failure. Rotated credentials are persisted under the existing connection lease and credential-generation fence before order reads; a later provider read failure does not discard the rotation. Network/provider failures retain authorization and release the lease for retry. Missing refresh tokens or revoked authorization require reconnecting in the browser.

Swiggy's official authentication guide explicitly says refresh-token issuance is not implemented in v1.0, even though its metadata advertises the grant. Therefore automatic background renewal cannot currently be promised for Swiggy. Reauthorization may reuse the provider's longer-lived browser session, but Vox does not automate browser consent or OTP entry. Zomato metadata advertises the refresh grant; actual issuance and rotation still need a real approved-account test.

## Real data and sync

Swiggy reads `get_addresses` with pagination, then `get_food_orders` for saved addresses and `track_food_order` for active orders. These are read-only calls. No cart, payment, ordering, or cancellation tool is exposed by this integration. Returned order IDs are deduplicated without inventing sessions or events.

Zomato discovers its authenticated tool catalogue and accepts only a supported history tool (`get_orders`, `get_order_history`, or `get_recent_orders`) with a read-only annotation and no required arguments. Its real tool name, input contract, and response schema have not yet been verified with developer access. If the catalogue does not expose that contract, linking fails explicitly rather than guessing a tool or reporting successful synchronization.

Malformed history, non-JSON history, and provider errors are errors. A valid empty order array stays empty. No order, coordinate, amount, ETA, address, item, delivery status, or timestamp is fabricated. Original provider fields are retained for assistant reads. Unrecognized statuses and timestamps remain unknown; timeline spans are created only when an actual timestamp and recognized status are available. The assistant does not claim that the provider's recent-order list is complete historical coverage.

Active orders are scheduled every 60 seconds, with longer explicit polling-interval hints honored when available. Inactive accounts are checked every 15 minutes. Core Worker runs the existing scheduler. Manual refresh and assistant reads use the same credential lifecycle. Existing cookie-linked connections must reconnect through OAuth.

Delivery maps require actual provider restaurant, destination, and rider coordinates. Swiggy Food's documented tracking schema does not provide these coordinates, so rider map tracking is not advertised. Map changes are published only after a database commit. Worker-originated map delivery across the API/Worker boundary has not been validated.

## Local verification and release boundary

Verified locally: 31 connection tests passed with all database tests enabled; the full Core test command passed (17 standard tests, with 13 database tests normally skipped); all six connection-ingestion database tests passed separately. Seven unrelated Core database tests were not run. The desktop production build passed, lint had zero errors and one existing warning in `use-mobile.ts`, and the new SVG was rendered and inspected.

Connection tests include isolated PostgreSQL tests for verified OAuth linking, encrypted refresh-token rotation, expired authorization, callback replay, and lease release. HTTP tests cover real structured responses, empty history, malformed history, and MCP JSON/SSE framing. Core tests cover timestamp preservation and skipping unknown history.

Core currently pins a published `vox-connections` Git revision. Test the local changes using a Cargo patch override; publish the connection changes and update Core's pin as part of an authorized release. Local tests do not validate a deployed provider integration.

Before enabling real accounts, verify provider access, exact allowlisted callback URLs, real browser authorization, the authenticated tool schema, provider-issued token expiry/rotation, scheduled worker sync, and account disconnect. No real account login, deployment, or live order sync was performed for this change.

## Sources checked on 2026-10-05

- [Swiggy authentication](https://mcp.swiggy.com/builders/docs/start/authenticate/)
- [Swiggy delegated authorization](https://mcp.swiggy.com/builders/docs/start/enterprise/delegated-auth/)
- [Swiggy food history schema](https://mcp.swiggy.com/builders/docs/reference/food/get_food_orders/)
- [Swiggy food tracking schema](https://mcp.swiggy.com/builders/docs/reference/food/track_food_order/)
- [Swiggy address pagination](https://mcp.swiggy.com/builders/docs/reference/food/get_addresses/)
- [Zomato developer access and callback restrictions](https://github.com/Zomato/mcp-server-manifest)
- Public authorization-server metadata was read directly from both official MCP hosts.

The Zomato SVG uses the Simple Icons vector paths, with a flat brand-red rounded tile and a centered white wordmark. Source: [Simple Icons Zomato](https://github.com/simple-icons/simple-icons/blob/develop/icons/zomato.svg).
