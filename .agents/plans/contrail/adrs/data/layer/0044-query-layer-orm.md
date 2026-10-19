---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0044: Query layer / ORM and dialect abstraction

## Context and Problem Statement

Plane's data access is the Django ORM with heavy queryset composition (annotations, `Subquery`, `Exists`, prefetch, grouped listing in `app/views/issue/base.py`), `ChangeTrackerMixin`, soft-delete managers, and partial indexes **[S]**. In the Rust backend (ADR-0006) we need a query layer that (a) compiles for `wasm32-unknown-unknown` (Workers) **and** native (self-hosted), (b) targets the chosen DB (ADR-0040) and possibly two dialects (Postgres for self-hosted, D1/SQLite), (c) sits **behind repository ports** (ADR-0045) so no ORM type leaks into core.

## Decision Drivers

* Bundle size / cold start on Workers (64 MiB limit, 1 s startup); must compile for `wasm32-unknown-unknown` — no libpq or other native C dependencies on the Worker target.
* Dialect abstraction vs. escape hatches for raw SQL (grouped listing, FTS, partial indexes).
* Type safety from schema (ADR-0042).
* Works with Hyperdrive + a wasm-capable Postgres driver (`tokio-postgres`/rust-postgres is the Rust driver Cloudflare lists), the D1 binding via workers-rs, and (desk review only) DO SQLite.
* Testability against both adapters (ADR-0121).

## Considered Options

* ~~Option 1: Drizzle ORM~~ — superseded (TypeScript)
* ~~Option 2: Kysely (type-safe query builder) with dialect plug-ins~~ — superseded (TypeScript)
* Option 3: Hand-written SQL in repository implementations (thin query helper, e.g. tagged-template + codegen types)
* ~~Option 4: Prisma (with driver adapters)~~ — superseded (TypeScript)
* **Option 5 (Rust): `sqlx`** — compile-time-checked SQL, Postgres/SQLite; wasm32 support must be verified (likely blocked by native/socket assumptions)
* **Option 6 (Rust): SeaORM** — async ORM over sqlx; heavier; the community `better-auth-rs` already uses it (spike 0003)
* **Option 7 (Rust): `diesel`** — mature query builder; sync-first; async/wasm story to verify
* **Option 8 (Rust): `tokio-postgres` (+ D1 binding) with a thin hand-written query builder / SQL helpers** — smallest, most likely to build on wasm32; most code to own

## Decision Outcome

Decision pending — see Q-0044 (**no decision**; waits for spike 0002). The earlier TypeScript hypothesis (Kysely/Drizzle) is **superseded** by the Rust backend. The Rust shortlist (Options 5–8) is judged on one gate first: *does it compile and run in a Worker (`wasm32`) against the chosen engine?* Option 3/8-style hand SQL remains the fallback for the hottest queries (grouped listing, FTS, analytics) regardless of the choice.

Regardless of choice: **no ORM types cross the repository boundary**; repositories return plain domain records; dialect differences are confined to the adapter package.

### Consequences

* Good: portable core; hot paths optimizable; swap library later at repository granularity.
* Bad: no automatic relation loading (explicit joins/batching); we re-implement soft-delete scoping and change tracking (move to use-case layer, ADR-0043/0046).
* Neutral: a per-dialect SQL snippet library for functions that differ (JSON, array agg, FTS).

### Confirmation

Spike S3 (ADR-0040) implements grouped listing in the shortlisted library; bundle-size budget check in CI (e.g. API Worker ≤ budget from ADR-0100); contract tests pass on both dialect adapters.

## Pros and Cons of the Options

### Option 1: Drizzle (superseded — TypeScript)

* Good: strong TS DX, migrations tooling, Workers/D1 support.
* Bad: schema defined per dialect; complex dynamic queries (grouped listing) get awkward; API churn risk.

### Option 2: Kysely (superseded — TypeScript)

* Good: thin, composable, dialect-neutral builder; types via codegen/introspection; works with pg, D1, DO SQLite via dialects.
* Bad: no migrations/relations built in; dynamic typing of group-by results needs care.

### Option 3: Hand SQL

* Good: total control and performance; trivially portable per adapter.
* Bad: no compile-time safety unless codegen; duplicated SQL per dialect; review burden.

### Option 4: Prisma (superseded — TypeScript)

* Good: mature DX and tooling.
* Bad: heavy on Workers; driver-adapter maturity; limited raw/partial-index/advisory support; weak fit for two dialects.

## Update — interview round 19 (2026-10-18)

Owner chose to **wait for the DB spike** before deciding tooling (Drizzle/Kysely, migration tool, repository/transaction ports). Leading option from the interview (since superseded): Drizzle with repositories and a unit-of-work port.

## Update — interview round 21 (2026-10-18)

The leading option from round 19 (Drizzle) is **moot** — the backend is Rust. Candidate Rust data layers (sqlx, SeaORM, diesel, tokio-postgres + query builder) must run on `wasm32` against the chosen engine: evaluated in spike 0002. Decision still waits for the DB spike.

## Update — review fixes, round 25 (2026-10-18)

Rust candidates (sqlx, SeaORM, diesel, `tokio-postgres` + thin helpers) are now listed as Options 5–8 alongside the superseded TypeScript options; the body no longer reads as a Kysely decision. Spike 0002 step 6 decides which compile and run on `wasm32`. The ADR stays `proposed`.

## Update — spike results, round 26 (2026-10-18)

Spike 0001 evidence (`[tested]` unless noted): **`tokio-postgres` 0.7.18 with feature `js` builds for wasm32 and runs over `worker::Socket` via `connect_raw`**, so one SQL crate (`thrust-adapters-postgres`, written at `tokio_postgres::Client` level behind a `Connector` trait: a native pool on the server, a Hyperdrive socket on the Worker) serves both targets, including transactions, `ON CONFLICT` and a partial unique index. **`sqlx` (Postgres) does not build for `wasm32-unknown-unknown`:** without a runtime feature the `home` crate fails (`cannot find function home_dir_inner`); with `runtime-tokio`, `mio` rejects the target (`probes/sqlx-wasm`). It remains usable native-only behind the port. SeaORM and diesel-async on top of `tokio-postgres` were **not tried** (unverified). To check in Stage 2: `Client::query(&str, …)` may cost two round trips per statement unless prepared statements are cached per connection, which would matter over Hyperdrive. The decision on the query layer still waits for spike 0002; the ADR stays `proposed`. Bundle size and startup budgets (Q-0044.1): 1.58 MiB and 17.7 ms locally against limits of 64 MiB and 1 s; the Cloudflare cold start is a Stage 2 measurement.

## Open Questions

* Q-0044 — which library?
* Q-0042 — schema source/migrations interplay.
* Q-0040 — dialect set to support.
* Q-0044.1 — Bundle-size/cold-start budget for the API Worker (needed to compare options).

## More Information

[[ADR-0040]], [[ADR-0042]], [[ADR-0045]]; Plane listing: `apps/api/plane/app/views/issue/base.py`.
