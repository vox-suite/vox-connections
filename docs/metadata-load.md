# Metadata selection load fixture

This fixture covers the Connections part of W13 and the 20-connection/40-skill
selection scenario. It exercises public Rust service APIs against the standalone
PostgreSQL schema. It does not run an agent, count actual calendar events, test a
provider, or establish production capacity.

## What passes

Each of 20 independent user contexts owns 20 connections and enables 40 curated
skills for its assistant. The database contains 400 connection/extension pairs,
40 shared skill versions and 800 context-specific skill installations.

- Calendar search returns exactly one relevant compact capability summary. Its
  connection belongs to the requesting context. Unrelated guidance stays out of
  the result. Each response is at most ten summaries and 32 KiB.
- No input schema, credential, skill instructions or resource bodies appear in
  discovery results. Loading the selected tool adds exactly its schema; it adds
  no skill bodies.
- A broad inventory query traverses six pages, finding all 60 tool/skill summaries
  without duplicates. Search is a current permission snapshot, not a persistent
  inventory cursor across mutations.
- A second enabled, selected owned agent in the first context has no grants or
  skill enablements. Both Calendar and broad inventory searches are empty;
  schema/read and the other assistant's skill body are denied. This proves
  isolation between real configured agents rather than only unknown-actor denial.
- Foreign-context schema loads and reads fail. Revoking the selected grant
  removes it from search and prevents subsequent load/read through the public
  services. Another context's permission remains intact.
- Disabling a skill prevents its body from loading in that context; another
  context's installation remains loadable.

Connection/version/credential metadata is inserted as fixture data, not published
as a real certified provider package. Grants and skills use their normal service
APIs. Provider endpoints use the reserved `.invalid` domain, credentials are
unusable ciphertext, and tokens are unexpired with no refresh token. The measured
path loads metadata/schema only. Denied reads fail with `GrantRequired` before
credential use or invocation. There are zero provider, OAuth refresh or model
calls in this fixture.

## Retained local baseline

[Raw report](metadata-load-baseline.json) was measured on 2026-10-01 against
Connections base `ae4294d9` with this test added. Machine: Apple M5 Pro, 15 CPU
cores, 24 GB RAM, macOS 26.6.2, rustc 1.98.1; PostgreSQL 18.6 Homebrew through
local TCP. The test used a debug build, four Tokio worker threads and a pool of
20 database connections on a shared workstation.

Fixture creation and one search/schema warmup per context were excluded. Twenty
contexts then independently ran 25 sequential search-and-load pairs concurrently:
500 searches and 500 schema loads. Each operation's latency includes its awaited
database calls and pool acquisition. The batch time also includes response checks.
Percentiles use sorted observations and nearest-rank p95; no latency threshold
is asserted by the test.

| Operation | Samples | p50 | p95 | Maximum |
| --- | ---: | ---: | ---: | ---: |
| Calendar metadata search | 500 | 2.275 ms | 6.400 ms | 21.368 ms |
| Selected schema load | 500 | 4.510 ms | 6.589 ms | 9.739 ms |

Every calendar summary response was 443 bytes. The measured batch completed in
0.203239 seconds: 4,920 combined service operations/second over that short batch.
This rate is a descriptive sample, not sustained throughput or a capacity target.
An earlier fresh-database run had search p95 7.768 ms and schema-load p95
10.641 ms, illustrating local scheduling variance. No runtime optimization was
made from these observations. A separate reviewer reran the fixture on a fresh
database on 2026-10-02: [raw repeat report](metadata-load-review-baseline.json)
records search p95 8.778 ms and schema-load p95 8.935 ms.

The tested envelope is 20 simultaneous contexts with these metadata cardinalities,
warm data and no external latency. Cold caches, larger catalogues, heterogeneous
queries, slow providers, refresh contention, model selection behavior, HTTP host
loads and deployment resource limits require separate measurements. Production
latency/throughput targets remain unset until that deployment evidence exists.

## Repeat on a fresh database

Requires PostgreSQL 18+, a trusted local test role, Rust, and the normal project
build dependencies. Use a dedicated database; do not apply standalone snapshots
to an existing Core database. Set `PGPASSWORD` through your normal local test
configuration if required. The following creates a new database without deleting
any existing one:

```sh
load_db="vox_metadata_$(date +%s)"
createdb -h 127.0.0.1 -p "${PGPORT:-5432}" -U postgres "$load_db"
export TEST_DATABASE_URL="postgres://postgres@127.0.0.1:${PGPORT:-5432}/$load_db"
for schema_file in examples/independent_host_schema.sql schema/connectors.sql \
  schema/packages.sql schema/skill-content.sql schema/public-mcp.sql \
  schema/setup.sql schema/discovery.sql; do
  psql "$TEST_DATABASE_URL" -v ON_ERROR_STOP=1 -f "$schema_file" || exit 1
done
export VOX_LOAD_ENVIRONMENT="Record CPU, RAM, OS, Rust version and deployment conditions"
export VOX_LOAD_REVISION="$(git rev-parse HEAD)"
export VOX_LOAD_REPORT="/tmp/vox-metadata-baseline.json"
cargo test --test metadata_load -- --ignored --nocapture
```

The test prints `METADATA_LOAD_REPORT` and optionally writes the raw JSON to
`VOX_LOAD_REPORT`. Inspect assertion failures before interpreting timing. CI runs
the same correctness fixture after applying the independent schemas; variable CI
latencies are not compared with the retained machine's values.
