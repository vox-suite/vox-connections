# Account schema

`connectors.sql` is the standalone curated account schema, requiring a host-owned `users(id UUID)` table. Core uses forward migrations. The library never creates timeline spans directly: the host implements `TimelineIngestor` and commits its source events and spans in the account checkpoint transaction.
